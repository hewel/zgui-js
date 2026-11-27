use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    ffi::{OsStr, OsString},
    fs::{self, DirBuilder, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use super::{
    jobs::{EntryRef, Run, alternate_name, exists, publish, rename_noreplace},
    path::{Access, Leaf, Root, Scope, denied, fd_path, pin},
    volumes::{Mount, mounts},
};
use crate::{ServiceError, files::map_io_error};

/// On-disk Freedesktop Trash configuration for the current user.
#[derive(Clone, Debug)]
pub struct Trash {
    home: PathBuf,
    uid: u32,
}

/// A recoverable item whose original path is covered by an active root.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashEntry {
    pub id: String,
    pub name: String,
    pub original_path: String,
    pub deletion_date: String,
}

struct Store {
    root: PathBuf,
    top: Option<PathBuf>,
    files: File,
    info: File,
}

struct Record {
    entry: TrashEntry,
    payload: PathBuf,
    info: PathBuf,
    original: PathBuf,
}

impl Trash {
    /// Configure the absolute XDG data directory; no I/O occurs until a job starts.
    pub fn new(data_home: PathBuf) -> Self {
        // SAFETY: geteuid takes no arguments and has no memory preconditions.
        Self {
            home: data_home.join("Trash"),
            uid: unsafe { libc::geteuid() },
        }
    }

    /// Use XDG_DATA_HOME or the standard HOME fallback, ignoring relative values.
    pub fn from_environment() -> Option<Self> {
        xdg_path("XDG_DATA_HOME", ".local/share").map(Self::new)
    }

    pub(super) fn put(
        &self,
        entries: Vec<EntryRef>,
        run: &mut Run<'_>,
    ) -> Result<(), ServiceError> {
        run.state.items_total = entries.len();
        let mounts = mounts().map_err(unavailable)?;
        for entry in entries {
            run.check()?;
            if entry.relative_path.is_empty() {
                return Err(ServiceError::new(
                    "InvalidPath",
                    "trash requires an entry beneath the granted root",
                ));
            }
            let leaf = entry.scope.leaf(&entry.relative_path, Access::Write)?;
            let parent = fs::read_link(fd_path(&leaf.parent)).map_err(map_io_error)?;
            entry.scope.authorize(&parent, Access::Write)?;
            let path = parent.join(&leaf.name);
            let source = leaf.at();
            let source_dev = fs::symlink_metadata(&source).map_err(map_io_error)?.dev();
            let store = self.store_for(&path, source_dev, &mounts)?;
            if path.starts_with(&store.root) || store.root.starts_with(&path) {
                return Err(unavailable(
                    "cannot move Trash or its ancestors into itself",
                ));
            }
            store.trash(&path, &source, &leaf.name, run)?;
            run.item_done(entry.relative_path);
        }
        Ok(())
    }

    pub(super) fn list(
        &self,
        roots: &[Root],
        run: &Run<'_>,
    ) -> Result<Vec<TrashEntry>, ServiceError> {
        Ok(self
            .inventory(roots, run)?
            .1
            .into_iter()
            .map(|record| record.entry)
            .collect())
    }

    pub(super) fn restore(
        &self,
        ids: Vec<String>,
        roots: &[Root],
        run: &mut Run<'_>,
    ) -> Result<(), ServiceError> {
        let (_stores, records) = self.inventory(roots, run)?;
        run.state.items_total = ids.len();
        for id in ids {
            run.check()?;
            let record = records
                .iter()
                .find(|record| record.entry.id == id)
                .ok_or_else(|| {
                    ServiceError::new("NotFound", "Trash item is missing or outside active grants")
                })?;
            let root = roots
                .iter()
                .find(|root| {
                    root.mode().can_write()
                        && record.original.starts_with(root.path())
                        && record.original != root.path()
                })
                .ok_or_else(denied)?;
            let mut scope = Scope::new(root.clone());
            for root in roots {
                scope.add_root(root.clone());
            }
            let parent = record
                .original
                .parent()
                .expect("authorized Trash entry has a parent");
            let canonical = scope.canonical_inside(parent, Access::Write)?;
            let name = record
                .original
                .file_name()
                .expect("authorized Trash entry has a name");
            let target = Leaf::new(&canonical, name)?;
            if let Some((target, overwrite)) = run.target(
                &record.entry.name,
                &target.at(),
                &record.entry.original_path,
            )? {
                run.check()?;
                // Trash is guaranteed to be on the original device when created. A
                // subsequently replaced mount must not delete the recoverable copy.
                publish(&record.payload, &target, overwrite).map_err(map_io_error)?;
                fs::remove_file(&record.info).map_err(map_io_error)?;
            }
            run.item_done(record.entry.original_path.clone());
        }
        Ok(())
    }

    fn store_for(&self, path: &Path, dev: u64, mounts: &[Mount]) -> Result<Store, ServiceError> {
        // Probe the nearest existing ancestor to avoid creating a home Trash on a
        // different device merely because an external volume is being used.
        let home_dev = self
            .home
            .ancestors()
            .find_map(|path| fs::metadata(path).ok().map(|meta| meta.dev()));
        if home_dev == Some(dev) {
            return self
                .home_store(true)
                .and_then(|store| store.on_device(dev))
                .map_err(unavailable);
        }
        let top = mounts
            .iter()
            .filter(|mount| path.starts_with(&mount.point))
            .max_by_key(|mount| mount.point.as_os_str().len())
            .map(|mount| mount.point.as_path())
            .filter(|top| *top != path)
            .ok_or_else(|| unavailable("mount top directory is unknown"))?;
        self.external_store(top, true)
            .and_then(|store| store.on_device(dev))
            .map_err(unavailable)
    }

    fn home_store(&self, create: bool) -> io::Result<Store> {
        let data = self
            .home
            .parent()
            .filter(|path| path.is_absolute())
            .ok_or(io::ErrorKind::InvalidInput)?;
        if create {
            fs::create_dir_all(data)?;
        }
        let root = fs::canonicalize(data)?.join("Trash");
        Store::open(&root, None, self.uid, create)
    }

    fn external_store(&self, top: &Path, create: bool) -> io::Result<Store> {
        let shared = top.join(".Trash");
        if fs::symlink_metadata(&shared)
            .is_ok_and(|meta| meta.is_dir() && meta.mode() & libc::S_ISVTX != 0)
            && let Ok(store) = Store::open(
                &shared.join(self.uid.to_string()),
                Some(top),
                self.uid,
                create,
            )
        {
            return Ok(store);
        }
        Store::open(
            &top.join(format!(".Trash-{}", self.uid)),
            Some(top),
            self.uid,
            create,
        )
    }

    fn inventory(
        &self,
        roots: &[Root],
        run: &Run<'_>,
    ) -> Result<(Vec<Store>, Vec<Record>), ServiceError> {
        let mut stores = Vec::new();
        if roots.is_empty() {
            return Ok((stores, Vec::new()));
        }
        if let Ok(store) = self.home_store(false) {
            stores.push(store);
        }
        let mut seen = BTreeSet::new();
        for mount in mounts().map_err(map_io_error)? {
            run.check()?;
            if roots.iter().any(|root| {
                mount.point.starts_with(root.path()) || root.path().starts_with(&mount.point)
            }) && seen.insert(mount.point.clone())
            {
                // Both layouts can contain recoverable files after a fallback.
                let top = &mount.point;
                let shared = top.join(".Trash");
                if fs::symlink_metadata(&shared)
                    .is_ok_and(|meta| meta.is_dir() && meta.mode() & libc::S_ISVTX != 0)
                    && let Ok(store) = Store::open(
                        &shared.join(self.uid.to_string()),
                        Some(top),
                        self.uid,
                        false,
                    )
                {
                    stores.push(store);
                }
                if let Ok(store) = Store::open(
                    &top.join(format!(".Trash-{}", self.uid)),
                    Some(top),
                    self.uid,
                    false,
                ) {
                    stores.push(store);
                }
            }
        }
        let mut records = Vec::new();
        for store in &stores {
            store.records(roots, run, &mut records)?;
        }
        records.sort_unstable_by(|a, b| {
            b.entry
                .deletion_date
                .cmp(&a.entry.deletion_date)
                .then_with(|| a.entry.id.cmp(&b.entry.id))
        });
        Ok((stores, records))
    }
}

impl Store {
    fn open(root: &Path, top: Option<&Path>, uid: u32, create: bool) -> io::Result<Self> {
        let name = root.file_name().ok_or(io::ErrorKind::InvalidInput)?;
        let parent = pin(
            root.parent().ok_or(io::ErrorKind::InvalidInput)?,
            libc::O_DIRECTORY | libc::O_PATH,
        )?;
        let dir = private_dir(&parent, name, uid, create)?;
        let files = private_dir(&dir, OsStr::new("files"), uid, create)?;
        let info = private_dir(&dir, OsStr::new("info"), uid, create)?;
        Ok(Self {
            root: root.to_owned(),
            top: top.map(Path::to_owned),
            files,
            info,
        })
    }

    fn on_device(self, dev: u64) -> io::Result<Self> {
        if self.files.metadata()?.dev() == dev && self.info.metadata()?.dev() == dev {
            Ok(self)
        } else {
            Err(io::Error::from_raw_os_error(libc::EXDEV))
        }
    }

    fn trash(
        &self,
        original: &Path,
        source: &Path,
        name: &OsStr,
        run: &Run<'_>,
    ) -> Result<(), ServiceError> {
        let path = self.top.as_ref().map_or(original, |top| {
            original.strip_prefix(top).expect("mount prefix checked")
        });
        let local = jiff::Timestamp::now()
            .to_zoned_datetime(jiff::tz::TimeZone::try_system().unwrap_or(jiff::tz::TimeZone::UTC));
        let date = format!("{:.0}", local.datetime());
        let text = format!(
            "[Trash Info]\nPath={}\nDeletionDate={date}\n",
            encode_path(path)
        );
        let info = fd_path(&self.info);
        let files = fd_path(&self.files);
        for index in 0.. {
            run.check()?;
            let name = if index == 0 {
                name.to_owned()
            } else {
                alternate_name(name, index)
            };
            let payload = files.join(&name);
            if exists(&payload).map_err(unavailable)? {
                continue;
            }
            let mut info_name = name;
            info_name.push(".trashinfo");
            let metadata = info.join(info_name);
            match write_info(&info, &metadata, text.as_bytes()) {
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(unavailable(error)),
                Ok(()) => {}
            }
            let status = run
                .check()
                .and_then(|()| rename_noreplace(source, &payload).map_err(unavailable));
            if status.is_err() {
                let _ = fs::remove_file(metadata);
            }
            return status;
        }
        unreachable!()
    }

    fn records(
        &self,
        roots: &[Root],
        run: &Run<'_>,
        records: &mut Vec<Record>,
    ) -> Result<(), ServiceError> {
        let files = fd_path(&self.files);
        let info = fd_path(&self.info);
        for child in fs::read_dir(info).map_err(map_io_error)? {
            run.check()?;
            let child = child.map_err(map_io_error)?;
            let filename = child.file_name();
            let Some(name) = filename.as_bytes().strip_suffix(b".trashinfo") else {
                continue;
            };
            if matches!(name, b"" | b"." | b"..") {
                continue;
            }
            let name = OsStr::from_bytes(name);
            let payload = files.join(name);
            if !exists(&payload).unwrap_or(false) {
                continue;
            }
            let info = child.path();
            let Some((original, deletion_date)) = read_info(&info, self.top.as_deref()) else {
                continue;
            };
            if !roots
                .iter()
                .any(|root| original.starts_with(root.path()) && original != root.path())
            {
                continue;
            }
            let id = format!(
                "{:x}",
                Sha256::new()
                    .chain_update(self.root.as_os_str().as_bytes())
                    .chain_update([0])
                    .chain_update(name.as_bytes())
                    .finalize()
            );
            let entry = TrashEntry {
                id,
                name: original
                    .file_name()
                    .unwrap_or(name)
                    .to_string_lossy()
                    .into_owned(),
                original_path: original.to_string_lossy().into_owned(),
                deletion_date,
            };
            records.push(Record {
                entry,
                payload,
                info,
                original,
            });
        }
        Ok(())
    }
}

fn private_dir(parent: &File, name: &OsStr, uid: u32, create: bool) -> io::Result<File> {
    let path = fd_path(parent).join(name);
    if create {
        match DirBuilder::new().mode(0o700).create(&path) {
            Err(error) if error.kind() != io::ErrorKind::AlreadyExists => return Err(error),
            _ => {}
        }
    }
    let dir = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_PATH)
        .open(path)?;
    let meta = dir.metadata()?;
    if meta.uid() != uid || meta.mode() & 0o022 != 0 {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(dir)
}

fn write_info(dir: &Path, dest: &Path, text: &[u8]) -> io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let (path, mut file) = loop {
        let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!(".zgui-trash-{}-{nonce}", std::process::id()));
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
        {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            other => break (path, other?),
        }
    };
    let result = (|| {
        file.write_all(text)?;
        file.sync_all()?;
        rename_noreplace(&path, dest)
    })();
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

fn read_info(path: &Path, top: Option<&Path>) -> Option<(PathBuf, String)> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_PATH)
        .open(path)
        .ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut text = String::new();
    File::open(fd_path(&file))
        .ok()?
        .take(16 * 1024)
        .read_to_string(&mut text)
        .ok()?;
    let mut lines = text.lines();
    if lines.next()? != "[Trash Info]" {
        return None;
    }
    let mut original = None;
    let mut date = None;
    for line in lines {
        if let Some(value) = line.strip_prefix("Path=") {
            original = decode_path(value);
        } else if let Some(value) = line.strip_prefix("DeletionDate=") {
            date = Some(value.to_owned());
        }
    }
    let original = original?;
    if original
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return None;
    }
    let original = if original.is_absolute() {
        original
    } else {
        top?.join(original)
    };
    if top.is_some_and(|top| !original.starts_with(top)) {
        return None;
    }
    Some((original, date?))
}

fn decode_path(value: &str) -> Option<PathBuf> {
    let mut bytes = Vec::with_capacity(value.len());
    let mut raw = value.bytes();
    while let Some(byte) = raw.next() {
        let byte = if byte == b'%' {
            ((char::from(raw.next()?).to_digit(16)? << 4) | char::from(raw.next()?).to_digit(16)?)
                as u8
        } else {
            byte
        };
        if byte == 0 {
            return None;
        }
        bytes.push(byte);
    }
    Some(PathBuf::from(OsString::from_vec(bytes)))
}

pub(super) fn encode_path(path: &Path) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-_.~!()*'".contains(&byte) {
            out.push(char::from(byte));
        } else {
            write!(out, "%{byte:02X}").expect("writing to String cannot fail");
        }
    }
    out
}

pub(super) fn xdg_path(key: &str, fallback: &str) -> Option<PathBuf> {
    let absolute = |key| {
        std::env::var_os(key)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    absolute(key).or_else(|| absolute("HOME").map(|home| home.join(fallback)))
}

pub(super) fn unavailable(reason: impl std::fmt::Display) -> ServiceError {
    ServiceError::new("E_TRASH_UNAVAILABLE", reason.to_string())
}
