use serde::{Deserialize, Serialize};
use std::{
    ffi::{CString, OsStr, OsString},
    fs::{self, DirBuilder, File, FileTimes, Metadata, OpenOptions, Permissions},
    io::{self, Read, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt, symlink},
    },
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use super::path::{Access, Leaf, Scope, encode_name, fd_path, path_error};
use super::{Root, Trash, TrashEntry, trash::unavailable};
use crate::{ServiceError, files::map_io_error};

const CHUNK_BYTES: usize = 128 * 1024;

/// One authorized entry, addressed relative to an active system directory.
#[derive(Clone, Debug)]
pub struct EntryRef {
    pub scope: Scope,
    pub relative_path: String,
}

impl EntryRef {
    /// Bind a relative path to its originating authority.
    pub fn new(scope: Scope, relative_path: String) -> Self {
        Self {
            scope,
            relative_path,
        }
    }
}

/// One cancellable file operation; copy and move destinations are directories.
#[derive(Debug)]
pub enum Job {
    Copy {
        sources: Vec<EntryRef>,
        destination: EntryRef,
    },
    Move {
        sources: Vec<EntryRef>,
        destination: EntryRef,
    },
    RemovePermanent {
        entries: Vec<EntryRef>,
        recursive: bool,
    },
    Mkdir {
        entry: EntryRef,
    },
    CreateSymlink {
        entry: EntryRef,
        target: String,
    },
    SetPermissions {
        entry: EntryRef,
        mode: u32,
    },
    Trash {
        entries: Vec<EntryRef>,
    },
    ListTrash {
        roots: Vec<Root>,
    },
    RestoreFromTrash {
        ids: Vec<String>,
        roots: Vec<Root>,
    },
}

/// Decision for an existing destination.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConflictAction {
    Overwrite,
    Skip,
    AutoRename,
    Cancel,
}

/// A conflict decision, optionally reused for the rest of this job.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConflictResolution {
    pub action: ConflictAction,
    #[serde(default)]
    pub apply_to_all: bool,
}

impl ConflictResolution {
    /// Default when no handler is installed or the job is cancelled.
    pub const CANCEL: Self = Self {
        action: ConflictAction::Cancel,
        apply_to_all: false,
    };
}

/// Relative source and destination names; never exposes ambient paths.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub source_path: String,
    pub destination_path: String,
}

/// Cumulative bytes and top-level items completed by the job.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub bytes_completed: u64,
    pub bytes_total: u64,
    pub items_completed: usize,
    pub items_total: usize,
    pub current_path: String,
}

/// A completed filesystem mutation; reserved separately from its progress reports.
#[derive(Debug)]
pub enum JobResult {
    Done,
    Trash(Vec<TrashEntry>),
}

/// Execute a POSIX job on the caller's I/O worker. The callbacks run on that worker.
pub fn run_job(
    job: Job,
    trash: Option<&Trash>,
    cancelled: &AtomicBool,
    mut conflict: impl FnMut(Conflict) -> ConflictResolution,
    mut progress: impl FnMut(&Progress),
) -> Result<JobResult, ServiceError> {
    let mut run = Run::new(cancelled, &mut conflict, &mut progress);
    run.check()?;
    let trash = || trash.ok_or_else(|| unavailable("no home Trash directory is configured"));
    match job {
        Job::Copy {
            sources,
            destination,
        } => run.transfer(sources, &destination, Transfer::Copy)?,
        Job::Move {
            sources,
            destination,
        } => run.transfer(sources, &destination, Transfer::Move)?,
        Job::Trash { entries } => trash()?.put(entries, &mut run)?,
        Job::ListTrash { roots } => return Ok(JobResult::Trash(trash()?.list(&roots, &run)?)),
        Job::RestoreFromTrash { ids, roots } => trash()?.restore(ids, &roots, &mut run)?,
        Job::RemovePermanent { entries, recursive } => {
            run.state.items_total = entries.len();
            for entry in entries {
                run.check()?;
                entry
                    .scope
                    .remove_permanent(&entry.relative_path, recursive)?;
                run.item_done(entry.relative_path);
            }
        }
        Job::Mkdir { entry } => {
            let leaf = named(&entry, Access::Write)?;
            fs::create_dir(leaf.at()).map_err(map_io_error)?;
            run.single_done(entry.relative_path);
        }
        Job::CreateSymlink { entry, target } => {
            let leaf = named(&entry, Access::Write)?;
            symlink(target, leaf.at()).map_err(map_io_error)?;
            run.single_done(entry.relative_path);
        }
        Job::SetPermissions { entry, mode } => {
            // chmod a pinned inode, never a subsequently substituted symlink.
            let (_, file) =
                entry
                    .scope
                    .follow_with(&entry.relative_path, Access::Write, libc::O_PATH)?;
            fs::set_permissions(fd_path(&file), Permissions::from_mode(mode & 0o7777))
                .map_err(map_io_error)?;
            run.single_done(entry.relative_path);
        }
    }
    Ok(JobResult::Done)
}

impl Scope {
    /// Remove a named entry. Recursive removal never descends into a symlink.
    pub fn remove_permanent(&self, relative: &str, recursive: bool) -> Result<(), ServiceError> {
        if relative.is_empty() {
            return Err(root_mutation());
        }
        let leaf = self.leaf(relative, Access::Write)?;
        remove(&leaf.at(), recursive).map_err(map_io_error)
    }
}

fn root_mutation() -> ServiceError {
    ServiceError::new(
        "InvalidPath",
        "operation requires an entry beneath the granted root",
    )
}

fn named(entry: &EntryRef, access: Access) -> Result<Leaf, ServiceError> {
    if entry.relative_path.is_empty() && matches!(access, Access::Write) {
        return Err(root_mutation());
    }
    entry.scope.leaf(&entry.relative_path, access)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transfer {
    Copy,
    Move,
}

pub(super) struct Run<'a> {
    pub(super) cancelled: &'a AtomicBool,
    pub(super) conflict: &'a mut dyn FnMut(Conflict) -> ConflictResolution,
    pub(super) progress: &'a mut dyn FnMut(&Progress),
    pub(super) state: Progress,
    decision: Option<ConflictAction>,
    buffer: Vec<u8>,
}

impl<'a> Run<'a> {
    pub(super) fn new(
        cancelled: &'a AtomicBool,
        conflict: &'a mut dyn FnMut(Conflict) -> ConflictResolution,
        progress: &'a mut dyn FnMut(&Progress),
    ) -> Self {
        Self {
            cancelled,
            conflict,
            progress,
            state: Progress::default(),
            decision: None,
            buffer: Vec::new(),
        }
    }
}

impl Run<'_> {
    pub(super) fn check(&self) -> Result<(), ServiceError> {
        check_cancelled(self.cancelled)
    }

    pub(super) fn item_done(&mut self, path: String) {
        self.state.items_completed += 1;
        self.state.current_path = path;
        (self.progress)(&self.state);
    }

    pub(super) fn single_done(&mut self, path: String) {
        self.state.items_total = 1;
        self.item_done(path);
    }

    pub(super) fn decide(
        &mut self,
        source_path: &str,
        destination_path: &str,
    ) -> Result<ConflictAction, ServiceError> {
        if let Some(action) = self.decision {
            return Ok(action);
        }
        let decision = (self.conflict)(Conflict {
            source_path: source_path.to_owned(),
            destination_path: destination_path.to_owned(),
        });
        self.check()?;
        if decision.action == ConflictAction::Cancel {
            return Err(cancelled());
        }
        if decision.apply_to_all {
            self.decision = Some(decision.action);
        }
        Ok(decision.action)
    }

    fn transfer(
        &mut self,
        entries: Vec<EntryRef>,
        destination: &EntryRef,
        transfer: Transfer,
    ) -> Result<(), ServiceError> {
        let (dest_path, dest_dir) = destination.scope.follow_with(
            &destination.relative_path,
            Access::Write,
            libc::O_DIRECTORY | libc::O_PATH,
        )?;
        let dest_at = fd_path(&dest_dir);
        let access = if transfer == Transfer::Move {
            Access::Write
        } else {
            Access::Read
        };
        self.state.items_total = entries.len();
        let mut sources = Vec::with_capacity(entries.len());
        for entry in entries {
            self.check()?;
            let leaf = named(&entry, access)?;
            let at = leaf.at();
            let metadata = fs::symlink_metadata(&at).map_err(map_io_error)?;
            let parent = fs::read_link(fd_path(&leaf.parent)).map_err(map_io_error)?;
            let source_path = parent.join(&leaf.name);
            if metadata.is_dir() && dest_path.starts_with(&source_path) {
                return Err(ServiceError::new(
                    "InvalidPath",
                    "cannot transfer a directory inside itself",
                ));
            }
            let size = if transfer == Transfer::Copy {
                tree_size(&at, &metadata, self.cancelled)?
            } else if metadata.is_file() {
                metadata.len()
            } else {
                0
            };
            self.state.bytes_total = self.state.bytes_total.saturating_add(size);
            sources.push((entry.relative_path, leaf, at, metadata, size, source_path));
        }
        for (relative, leaf, at, metadata, size, source_path) in sources {
            self.check()?;
            let existing = dest_at.join(&leaf.name);
            let dest_rel = append_name(&destination.relative_path, &leaf.name);
            let source = Source {
                at: &at,
                path: &source_path,
                meta: &metadata,
                relative: &relative,
            };
            let placed = self.transfer_one(source, &existing, &dest_path, &dest_rel, transfer)?;
            if !placed {
                // A skipped item or fast rename still consumes its share of the work.
                self.state.bytes_completed = self.state.bytes_completed.saturating_add(size);
            }
            self.item_done(relative);
        }
        Ok(())
    }

    pub(super) fn target(
        &mut self,
        src: &str,
        target: &Path,
        relative: &str,
    ) -> Result<Option<(PathBuf, bool)>, ServiceError> {
        if !exists(target).map_err(map_io_error)? {
            return Ok(Some((target.to_owned(), false)));
        }
        match self.decide(src, relative)? {
            ConflictAction::Skip => Ok(None),
            ConflictAction::Overwrite => Ok(Some((target.to_owned(), true))),
            ConflictAction::AutoRename => {
                let parent = target.parent().expect("target has a parent");
                let name = target.file_name().expect("target has a name");
                for index in 1.. {
                    self.check()?;
                    let path = parent.join(alternate_name(name, index));
                    if !exists(&path).map_err(map_io_error)? {
                        return Ok(Some((path, false)));
                    }
                }
                unreachable!()
            }
            ConflictAction::Cancel => unreachable!("cancel is an error"),
        }
    }

    fn transfer_one(
        &mut self,
        source: Source<'_>,
        target: &Path,
        dest_dir: &Path,
        dest_rel: &str,
        transfer: Transfer,
    ) -> Result<bool, ServiceError> {
        let Source {
            at,
            path,
            meta,
            relative,
        } = source;
        let Some((target, overwrite)) = self.target(relative, target, dest_rel)? else {
            return Ok(false);
        };
        if fs::symlink_metadata(&target).is_ok_and(|existing| same_inode(meta, &existing)) {
            return Ok(false);
        }
        if path.starts_with(dest_dir.join(target.file_name().expect("target has a name"))) {
            return Err(ServiceError::new(
                "InvalidPath",
                "cannot replace a source ancestor",
            ));
        }
        self.check()?;
        if transfer == Transfer::Move {
            if !same_inode(meta, &fs::symlink_metadata(at).map_err(map_io_error)?) {
                return Err(ServiceError::new("Io", "move source changed before rename"));
            }
            match publish(at, &target, overwrite) {
                Ok(()) => return Ok(false),
                Err(error) if error.raw_os_error() == Some(libc::EXDEV) => {
                    if meta.is_dir() {
                        let size = tree_size(at, meta, self.cancelled)?;
                        self.state.bytes_total = self.state.bytes_total.saturating_add(size);
                    }
                }
                Err(error) => return Err(map_io_error(error)),
            }
        }
        self.state.current_path = relative.to_owned();
        let parent = target.parent().expect("target has a parent");
        let stage = Stage::new(parent).map_err(map_io_error)?;
        let staged = fd_path(&stage.dir).join("entry");
        copy_tree(at, &staged, meta, self)?;
        self.check()?;
        publish(&staged, &target, overwrite).map_err(map_io_error)?;
        File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(map_io_error)?;
        if transfer == Transfer::Move {
            let current = fs::symlink_metadata(at).map_err(map_io_error)?;
            if !same_inode(meta, &current) {
                return Err(ServiceError::new(
                    "Io",
                    "move source changed before removal",
                ));
            }
            remove(at, true).map_err(map_io_error)?;
        }
        Ok(true)
    }
}

struct Source<'a> {
    at: &'a Path,
    path: &'a Path,
    meta: &'a Metadata,
    relative: &'a str,
}

fn same_inode(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev() && a.ino() == b.ino()
}

pub(super) fn append_name(base: &str, name: &OsStr) -> String {
    let name = encode_name(name);
    if base.is_empty() {
        name
    } else {
        format!("{base}/{name}")
    }
}

pub(super) fn alternate_name(name: &OsStr, index: usize) -> OsString {
    let path = Path::new(name);
    let mut result = path.file_stem().unwrap_or(name).to_os_string();
    result.push(format!(" ({index})"));
    if let Some(ext) = path.extension() {
        result.push(".");
        result.push(ext);
    }
    result
}

pub(super) fn check_cancelled(cancel: &AtomicBool) -> Result<(), ServiceError> {
    if cancel.load(Ordering::Acquire) {
        Err(cancelled())
    } else {
        Ok(())
    }
}

pub(super) fn cancelled() -> ServiceError {
    ServiceError::new("Cancelled", "file job cancelled")
}

pub(super) fn exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub(super) fn remove(path: &Path, recursive: bool) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && recursive {
        fs::remove_dir_all(path)
    } else if metadata.is_dir() {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
}

pub(super) fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    rename_flags(from, to, libc::RENAME_NOREPLACE)
}

fn rename_flags(from: &Path, to: &Path, flags: libc::c_uint) -> io::Result<()> {
    let from = CString::new(from.as_os_str().as_bytes())?;
    let to = CString::new(to.as_os_str().as_bytes())?;
    // SAFETY: both paths are valid NUL-terminated strings for the duration of renameat2.
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            flags,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(super) fn publish(from: &Path, to: &Path, overwrite: bool) -> io::Result<()> {
    if !overwrite {
        return rename_noreplace(from, to);
    }
    match fs::rename(from, to) {
        Ok(()) => return Ok(()),
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(libc::ENOTEMPTY | libc::EEXIST | libc::EISDIR | libc::ENOTDIR)
            ) => {}
        Err(error) => return Err(error),
    }
    // Exchange handles non-empty directories and type conflicts without deleting
    // the old destination until the replacement has been published.
    match rename_flags(from, to, libc::RENAME_EXCHANGE) {
        Ok(()) => remove(from, true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => rename_noreplace(from, to),
        Err(error) => Err(error),
    }
}

struct Stage {
    path: PathBuf,
    dir: File,
}

impl Stage {
    fn new(parent: &Path) -> io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        loop {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(".zgui-transfer-{}-{id}", std::process::id()));
            match DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => {
                    let dir = OpenOptions::new()
                        .read(true)
                        .custom_flags(libc::O_PATH | libc::O_NOFOLLOW | libc::O_DIRECTORY)
                        .open(&path)?;
                    // SAFETY: geteuid takes no arguments and has no memory preconditions.
                    if dir.metadata()?.uid() != unsafe { libc::geteuid() } {
                        return Err(io::ErrorKind::PermissionDenied.into());
                    }
                    return Ok(Self { path, dir });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        if let (Ok(expected), Ok(current)) = (self.dir.metadata(), fs::symlink_metadata(&self.path))
            && same_inode(&expected, &current)
        {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn open_entry(path: &Path, expected: &Metadata) -> Result<File, ServiceError> {
    let fd = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_NOFOLLOW)
        .open(path)
        .map_err(path_error)?;
    if !same_inode(expected, &fd.metadata().map_err(map_io_error)?) {
        return Err(ServiceError::new(
            "Io",
            "entry changed during file transfer",
        ));
    }
    Ok(fd)
}

fn tree_size(path: &Path, meta: &Metadata, cancelled: &AtomicBool) -> Result<u64, ServiceError> {
    check_cancelled(cancelled)?;
    if meta.is_file() {
        return Ok(meta.len());
    }
    if !meta.is_dir() {
        return Ok(0);
    }
    let dir = open_entry(path, meta)?;
    let mut size = 0_u64;
    for child in fs::read_dir(fd_path(&dir)).map_err(map_io_error)? {
        let child = child.map_err(map_io_error)?;
        size = size.saturating_add(tree_size(
            &child.path(),
            &child.metadata().map_err(map_io_error)?,
            cancelled,
        )?);
    }
    Ok(size)
}

fn copy_tree(
    from: &Path,
    to: &Path,
    meta: &Metadata,
    run: &mut Run<'_>,
) -> Result<(), ServiceError> {
    run.check()?;
    if meta.is_symlink() {
        return symlink(fs::read_link(from).map_err(map_io_error)?, to).map_err(map_io_error);
    }
    if !meta.is_file() && !meta.is_dir() {
        return Err(ServiceError::new(
            "InvalidPath",
            "cannot copy special filesystem entries",
        ));
    }
    let fd = open_entry(from, meta)?;
    let from = fd_path(&fd);
    let output = if meta.is_dir() {
        DirBuilder::new()
            .mode(0o700)
            .create(to)
            .map_err(map_io_error)?;
        for entry in fs::read_dir(&from).map_err(map_io_error)? {
            let entry = entry.map_err(map_io_error)?;
            copy_tree(
                &entry.path(),
                &to.join(entry.file_name()),
                &entry.metadata().map_err(map_io_error)?,
                run,
            )?;
        }
        File::open(to).map_err(map_io_error)?
    } else {
        let mut source = File::open(from).map_err(map_io_error)?;
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(to)
            .map_err(map_io_error)?;
        if run.buffer.is_empty() {
            run.buffer.resize(CHUNK_BYTES, 0);
        }
        loop {
            run.check()?;
            let count = match source.read(&mut run.buffer) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                other => other.map_err(map_io_error)?,
            };
            if count == 0 {
                break;
            }
            output
                .write_all(&run.buffer[..count])
                .map_err(map_io_error)?;
            run.state.bytes_completed = run.state.bytes_completed.saturating_add(count as u64);
            (run.progress)(&run.state);
        }
        output
    };
    output
        .set_permissions(Permissions::from_mode(meta.mode() & 0o7777))
        .map_err(map_io_error)?;
    let mut times = FileTimes::new();
    if let Ok(accessed) = meta.accessed() {
        times = times.set_accessed(accessed);
    }
    if let Ok(modified) = meta.modified() {
        times = times.set_modified(modified);
    }
    output
        .set_times(times)
        .and_then(|()| output.sync_all())
        .map_err(map_io_error)
}
