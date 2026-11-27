//! In-memory desktop filesystem used by preview. No method accesses host paths.

use std::{
    collections::{BTreeMap, VecDeque},
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::AtomicBool},
};

use super::{
    Conflict, ConflictAction, ConflictResolution, DirPage, EntryInfo, EntryKind, EntryRef, Job,
    JobResult, Progress, ReadOptions, Root, Scope, ThumbnailSize, TrashEntry, Volume,
    jobs::{Run, alternate_name, append_name},
    mime, paginate,
    path::{Access, denied, encode_name},
    trash::unavailable,
};
use crate::{ServiceError, files::FileMode, locale::LocaleContext};

/// Deterministic desktop filesystem and service data shared by preview sessions.
#[derive(Debug, Default)]
pub struct PreviewSystemFiles {
    data: Mutex<Data>,
    thumbnails: BTreeMap<(PathBuf, &'static str), CachedPng>,
    pub volumes: Vec<Volume>,
    pub trash_unavailable: bool,
    pub dialogs: Mutex<VecDeque<Option<PathBuf>>>,
}

#[derive(Debug)]
struct CachedPng {
    mtime: i64,
    bytes: Arc<[u8]>,
}

#[derive(Debug, Default)]
struct Data {
    nodes: BTreeMap<PathBuf, Node>,
    trash: BTreeMap<String, Archived>,
    next_trash: u64,
}

#[derive(Debug)]
struct Archived {
    info: TrashEntry,
    original: PathBuf,
    tree: Vec<(PathBuf, Node)>,
}

#[derive(Clone, Debug)]
struct Node {
    body: Body,
    mode: u32,
    mtime: i64,
}

#[derive(Clone, Debug)]
enum Body {
    Dir,
    File(u64),
    Symlink(PathBuf),
}

impl Body {
    fn kind(&self) -> EntryKind {
        match self {
            Self::Dir => EntryKind::Dir,
            Self::File(_) => EntryKind::File,
            Self::Symlink(_) => EntryKind::Symlink,
        }
    }

    fn size(&self) -> u64 {
        match self {
            Self::Dir => 0,
            Self::File(size) => *size,
            Self::Symlink(target) => target.as_os_str().len() as u64,
        }
    }
}

impl Node {
    fn directory() -> Self {
        Self {
            body: Body::Dir,
            mode: 0o755,
            mtime: 0,
        }
    }
}

impl PreviewSystemFiles {
    /// Create a deterministic root and its missing ancestors.
    pub fn seed_directory(&mut self, path: &Path) {
        self.seed(path, Node::directory());
    }

    /// Seed file metadata; no system operation can read contents into JavaScript.
    pub fn seed_file(&mut self, path: &Path, size: u64) {
        self.seed(
            path,
            Node {
                body: Body::File(size),
                mode: 0o644,
                mtime: 0,
            },
        );
    }

    /// Seed a symlink without resolving its target.
    pub fn seed_symlink(&mut self, path: &Path, target: PathBuf) {
        self.seed(
            path,
            Node {
                body: Body::Symlink(target),
                mode: 0o777,
                mtime: 0,
            },
        );
    }

    /// Seed optional source metadata on an existing entry.
    pub fn seed_metadata(
        &mut self,
        path: &Path,
        mode: Option<u32>,
        modified_ms: Option<i64>,
    ) -> Result<(), ServiceError> {
        let data = self
            .data
            .get_mut()
            .expect("preview system files lock poisoned");
        let node = data.nodes.get_mut(path).ok_or_else(not_found)?;
        if let Some(mode) = mode {
            node.mode = mode & 0o7777;
        }
        if let Some(mtime) = modified_ms {
            node.mtime = mtime;
        }
        Ok(())
    }

    fn seed(&mut self, path: &Path, node: Node) {
        let data = self
            .data
            .get_mut()
            .expect("preview system files lock poisoned");
        for ancestor in path.ancestors().skip(1) {
            data.nodes
                .entry(ancestor.to_owned())
                .or_insert_with(Node::directory);
        }
        data.nodes.insert(path.to_owned(), node);
    }

    /// Seed encoded PNG bytes and their virtual Thumb::MTime.
    pub fn seed_thumbnail(
        &mut self,
        path: PathBuf,
        size: ThumbnailSize,
        mtime: i64,
        bytes: Vec<u8>,
    ) {
        self.thumbnails.insert(
            (path, size.folder()),
            CachedPng {
                mtime,
                bytes: bytes.into(),
            },
        );
    }

    /// Obtain a virtual root, following a symlink only for a new explicit grant.
    pub fn open(&self, path: &Path, mode: FileMode) -> Result<Root, ServiceError> {
        let path = if path == Path::new("/") {
            path.to_owned()
        } else {
            self.data().resolve(path)?
        };
        self.data().require_dir(&path)?;
        Ok(Root::virtual_root(path, mode))
    }

    /// Restore a virtual root without widening the persisted path.
    pub fn reopen(&self, path: &Path, mode: FileMode) -> Result<Root, ServiceError> {
        let root = self.open(path, mode)?;
        if root.path() != path {
            return Err(ServiceError::new(
                "E_PATH_ESCAPE",
                "fixture system grant root changed",
            ));
        }
        Ok(root)
    }

    /// Resolve a granted virtual entry for a drag offer or external open.
    pub fn follow(&self, entry: &EntryRef, mode: FileMode) -> Result<PathBuf, ServiceError> {
        self.data().follow(
            entry,
            if mode.can_write() {
                Access::Write
            } else {
                Access::Read
            },
        )
    }

    /// Inspect the final link itself and separately report target status.
    pub fn lstat(&self, entry: &EntryRef) -> Result<EntryInfo, ServiceError> {
        let data = self.data();
        let path = if entry.relative_path.is_empty() {
            data.follow(entry, Access::Read)?
        } else {
            data.leaf(entry, Access::Read)?
        };
        data.info(&path, entry.relative_path.clone())
    }

    /// List and collate a page with the same semantics as the POSIX adapter.
    pub fn read_dir(
        &self,
        entry: &EntryRef,
        options: &ReadOptions,
        locale: &LocaleContext,
    ) -> Result<DirPage, ServiceError> {
        let (path, entries) = {
            let data = self.data();
            let path = data.follow(entry, Access::Read)?;
            data.require_dir(&path)?;
            let entries = data
                .subtree(&path)
                .filter(|(child, _)| child.parent() == Some(&path))
                .filter_map(|(child, _)| {
                    let name = child.file_name()?;
                    if !options.include_hidden && name.as_encoded_bytes().starts_with(b".") {
                        return None;
                    }
                    Some(data.info(child, encode_name(name)))
                })
                .collect::<Result<Vec<_>, _>>()?;
            (path, entries)
        };
        paginate(entries, &path, &entry.relative_path, options, locale)
    }

    /// Return bytes only for the current virtual mtime and requested size.
    pub fn thumbnail(
        &self,
        entry: &EntryRef,
        size: ThumbnailSize,
    ) -> Result<Option<Arc<[u8]>>, ServiceError> {
        let data = self.data();
        let path = data.follow(entry, Access::Read)?;
        let node = data.node(&path)?;
        if !matches!(node.body, Body::File(_)) {
            return Ok(None);
        }
        Ok(self
            .thumbnails
            .get(&(path, size.folder()))
            .and_then(|cached| {
                (cached.mtime == node.mtime.div_euclid(1000)).then(|| cached.bytes.clone())
            }))
    }

    /// Run the same job contract with deterministic in-memory outcomes.
    pub fn run_job(
        &self,
        job: Job,
        cancelled: &AtomicBool,
        mut conflict: impl FnMut(Conflict) -> ConflictResolution,
        mut progress: impl FnMut(&Progress),
    ) -> Result<JobResult, ServiceError> {
        let mut run = Run::new(cancelled, &mut conflict, &mut progress);
        run.check()?;
        match job {
            Job::Copy {
                sources,
                destination,
            } => self.transfer(sources, &destination, FileMode::Read, &mut run)?,
            Job::Move {
                sources,
                destination,
            } => self.transfer(sources, &destination, FileMode::ReadWrite, &mut run)?,
            Job::RemovePermanent { entries, recursive } => {
                run.state.items_total = entries.len();
                for entry in entries {
                    run.check()?;
                    {
                        let mut data = self.data();
                        let path = data.named(&entry)?;
                        data.node(&path)?;
                        if !recursive && data.subtree(&path).nth(1).is_some() {
                            return Err(ServiceError::new("Io", "directory is not empty"));
                        }
                        data.take_tree(&path);
                    }
                    run.item_done(entry.relative_path);
                }
            }
            Job::Mkdir { entry } => self.create(entry, Node::directory(), &mut run)?,
            Job::CreateSymlink { entry, target } => {
                if target.is_empty() || target.contains('\0') {
                    return Err(ServiceError::new("InvalidPath", "invalid symlink target"));
                }
                self.create(
                    entry,
                    Node {
                        body: Body::Symlink(target.into()),
                        mode: 0o777,
                        mtime: 0,
                    },
                    &mut run,
                )?;
            }
            Job::SetPermissions { entry, mode } => {
                {
                    let mut data = self.data();
                    let path = data.follow(&entry, Access::Write)?;
                    data.nodes.get_mut(&path).ok_or_else(not_found)?.mode = mode & 0o7777;
                }
                run.single_done(entry.relative_path);
            }
            Job::Trash { entries } => self.put_trash(entries, &mut run)?,
            Job::ListTrash { roots } => {
                self.require_trash()?;
                return Ok(JobResult::Trash(
                    self.data()
                        .trash
                        .values()
                        .filter(|item| covered(&item.original, &roots))
                        .map(|item| item.info.clone())
                        .collect(),
                ));
            }
            Job::RestoreFromTrash { ids, roots } => self.restore_trash(ids, &roots, &mut run)?,
        }
        Ok(JobResult::Done)
    }

    fn data(&self) -> std::sync::MutexGuard<'_, Data> {
        self.data
            .lock()
            .expect("preview system files lock poisoned")
    }

    fn target(
        &self,
        source: &str,
        target: &Path,
        dest: &str,
        run: &mut Run<'_>,
    ) -> Result<Option<(PathBuf, bool)>, ServiceError> {
        if !self.data().nodes.contains_key(target) {
            return Ok(Some((target.to_owned(), false)));
        }
        match run.decide(source, dest)? {
            ConflictAction::Skip => Ok(None),
            ConflictAction::Overwrite => Ok(Some((target.to_owned(), true))),
            ConflictAction::AutoRename => {
                let data = self.data();
                for index in 1.. {
                    run.check()?;
                    let name =
                        alternate_name(target.file_name().expect("target has a name"), index);
                    let path = target.with_file_name(name);
                    if !data.nodes.contains_key(&path) {
                        return Ok(Some((path, false)));
                    }
                }
                unreachable!()
            }
            ConflictAction::Cancel => unreachable!("cancel is an error"),
        }
    }

    fn transfer(
        &self,
        sources: Vec<EntryRef>,
        dest: &EntryRef,
        mode: FileMode,
        run: &mut Run<'_>,
    ) -> Result<(), ServiceError> {
        let mut entries = Vec::with_capacity(sources.len());
        run.state.items_total = sources.len();
        let dest_dir = {
            let data = self.data();
            let dest = data.follow(dest, Access::Write)?;
            data.require_dir(&dest)?;
            for entry in sources {
                run.check()?;
                let from = if mode.can_write() {
                    data.named(&entry)?
                } else {
                    data.leaf(&entry, Access::Read)?
                };
                data.node(&from)?;
                if dest.starts_with(&from) {
                    return Err(ServiceError::new(
                        "InvalidPath",
                        "cannot transfer a directory inside itself",
                    ));
                }
                let size = data
                    .subtree(&from)
                    .filter_map(|(_, node)| match node.body {
                        Body::File(size) => Some(size),
                        _ => None,
                    })
                    .fold(0, u64::saturating_add);
                run.state.bytes_total = run.state.bytes_total.saturating_add(size);
                entries.push((from, entry.relative_path, size));
            }
            dest
        };
        for (from, relative, size) in entries {
            run.check()?;
            let name = from.file_name().ok_or_else(invalid)?;
            let new = dest_dir.join(name);
            let dest_rel = append_name(&dest.relative_path, name);
            if let Some((target, overwrite)) = self.target(&relative, &new, &dest_rel, run)?
                && target != from
            {
                if from.starts_with(&target) {
                    return Err(ServiceError::new(
                        "InvalidPath",
                        "cannot replace a source ancestor",
                    ));
                }
                run.check()?;
                let mut data = self.data();
                data.node(&from)?;
                data.require_dir(&dest_dir)?;
                data.prepare_destination(&target, overwrite)?;
                let tree = if mode.can_write() {
                    data.take_tree(&from)
                } else {
                    data.clone_tree(&from)
                };
                data.place_tree(&target, tree);
            }
            run.state.bytes_completed = run.state.bytes_completed.saturating_add(size);
            run.item_done(relative);
        }
        Ok(())
    }

    fn create(&self, entry: EntryRef, node: Node, run: &mut Run<'_>) -> Result<(), ServiceError> {
        {
            let mut data = self.data();
            let path = data.named(&entry)?;
            data.prepare_destination(&path, false)?;
            data.nodes.insert(path, node);
        }
        run.single_done(entry.relative_path);
        Ok(())
    }

    fn require_trash(&self) -> Result<(), ServiceError> {
        if self.trash_unavailable {
            Err(unavailable("fixture Trash is unavailable"))
        } else {
            Ok(())
        }
    }

    fn put_trash(&self, entries: Vec<EntryRef>, run: &mut Run<'_>) -> Result<(), ServiceError> {
        self.require_trash()?;
        run.state.items_total = entries.len();
        for entry in entries {
            run.check()?;
            {
                let mut data = self.data();
                let original = data.named(&entry)?;
                data.node(&original)?;
                let tree = data.take_tree(&original);
                data.next_trash += 1;
                let id = format!("fixture-trash-{:016x}", data.next_trash);
                let name = original
                    .file_name()
                    .expect("named entry")
                    .to_string_lossy()
                    .into_owned();
                let info = TrashEntry {
                    id: id.clone(),
                    name,
                    original_path: original.to_string_lossy().into_owned(),
                    deletion_date: "1970-01-01T00:00:00".into(),
                };
                data.trash.insert(
                    id,
                    Archived {
                        info,
                        original,
                        tree,
                    },
                );
            }
            run.item_done(entry.relative_path);
        }
        Ok(())
    }

    fn restore_trash(
        &self,
        ids: Vec<String>,
        roots: &[Root],
        run: &mut Run<'_>,
    ) -> Result<(), ServiceError> {
        self.require_trash()?;
        let active = Arc::new(roots.to_vec());
        run.state.items_total = ids.len();
        for id in ids {
            run.check()?;
            let (target, info) = {
                let data = self.data();
                let archive = data
                    .trash
                    .get(&id)
                    .filter(|item| covered(&item.original, roots))
                    .ok_or_else(not_found)?;
                let root = roots
                    .iter()
                    .find(|root| {
                        root.mode().can_write() && archive.original.starts_with(root.path())
                    })
                    .ok_or_else(denied)?;
                let scope = Scope::with_roots(root.clone(), active.clone());
                let parent = scope.resolve_with(
                    archive.original.parent().expect("named entry"),
                    Access::Write,
                    |path| data.resolve(path),
                )?;
                (
                    parent.join(archive.original.file_name().expect("named entry")),
                    archive.info.clone(),
                )
            };
            if let Some((target, overwrite)) =
                self.target(&info.name, &target, &info.original_path, run)?
            {
                run.check()?;
                let mut data = self.data();
                if !data.trash.contains_key(&id) {
                    return Err(not_found());
                }
                data.require_dir(target.parent().expect("named target"))?;
                data.prepare_destination(&target, overwrite)?;
                let item = data
                    .trash
                    .remove(&id)
                    .expect("checked record under the same lock");
                data.place_tree(&target, item.tree);
            }
            run.item_done(info.original_path);
        }
        Ok(())
    }
}

impl Data {
    fn node(&self, path: &Path) -> Result<&Node, ServiceError> {
        self.nodes.get(path).ok_or_else(not_found)
    }

    fn require_dir(&self, path: &Path) -> Result<(), ServiceError> {
        if path == Path::new("/") || matches!(self.node(path)?.body, Body::Dir) {
            Ok(())
        } else {
            Err(ServiceError::new("InvalidPath", "entry is not a directory"))
        }
    }

    fn resolve(&self, path: &Path) -> Result<PathBuf, ServiceError> {
        fn parts(path: &Path) -> impl Iterator<Item = OsString> {
            path.components().map(|part| part.as_os_str().to_owned())
        }
        let mut pending: VecDeque<_> = parts(path).collect();
        let mut resolved = PathBuf::from("/");
        let mut links = 0;
        while let Some(part) = pending.pop_front() {
            match part.to_str() {
                Some("/") => resolved = PathBuf::from("/"),
                Some(".") => {}
                Some("..") => {
                    self.require_dir(&resolved)?;
                    resolved.pop();
                }
                _ => {
                    self.require_dir(&resolved)?;
                    resolved.push(part);
                    if let Body::Symlink(target) = &self.node(&resolved)?.body {
                        links += 1;
                        if links > 40 {
                            return Err(ServiceError::new("NotFound", "symlink cycle"));
                        }
                        resolved.pop();
                        for part in parts(target).collect::<Vec<_>>().into_iter().rev() {
                            pending.push_front(part);
                        }
                    }
                }
            }
        }
        Ok(resolved)
    }

    fn follow(&self, entry: &EntryRef, access: Access) -> Result<PathBuf, ServiceError> {
        let path = entry.scope.lexical(&entry.relative_path, access)?;
        entry
            .scope
            .resolve_with(&path, access, |path| self.resolve(path))
    }

    fn leaf(&self, entry: &EntryRef, access: Access) -> Result<PathBuf, ServiceError> {
        let path = entry.scope.lexical(&entry.relative_path, access)?;
        if entry.relative_path.is_empty() {
            return Ok(path);
        }
        let parent =
            entry
                .scope
                .resolve_with(path.parent().ok_or_else(invalid)?, access, |path| {
                    self.resolve(path)
                })?;
        self.require_dir(&parent)?;
        Ok(parent.join(path.file_name().ok_or_else(invalid)?))
    }

    fn named(&self, entry: &EntryRef) -> Result<PathBuf, ServiceError> {
        if entry.relative_path.is_empty() {
            return Err(invalid());
        }
        self.leaf(entry, Access::Write)
    }

    fn info(&self, path: &Path, relative_path: String) -> Result<EntryInfo, ServiceError> {
        let fallback;
        let node = if path == Path::new("/") && !self.nodes.contains_key(path) {
            fallback = Node::directory();
            &fallback
        } else {
            self.node(path)?
        };
        let name = path
            .file_name()
            .unwrap_or(OsStr::new("/"))
            .to_string_lossy()
            .into_owned();
        let kind = node.body.kind();
        let (symlink_target, symlink_broken, symlink_target_kind) =
            if let Body::Symlink(target) = &node.body {
                let status = self.resolve(path).ok().and_then(|path| {
                    if path == Path::new("/") {
                        Some(EntryKind::Dir)
                    } else {
                        self.nodes.get(&path).map(|node| node.body.kind())
                    }
                });
                (
                    Some(target.to_string_lossy().into_owned()),
                    status.is_none(),
                    status,
                )
            } else {
                (None, false, None)
            };
        Ok(EntryInfo {
            is_hidden: name.starts_with('.'),
            mime_type: mime(&name, kind),
            name,
            relative_path,
            kind,
            size: node.body.size(),
            modified_ms: Some(node.mtime),
            mode: node.mode,
            symlink_target,
            symlink_broken,
            symlink_target_kind,
        })
    }

    fn subtree<'a>(&'a self, path: &'a Path) -> impl Iterator<Item = (&'a PathBuf, &'a Node)> {
        self.nodes
            .range(path.to_owned()..)
            .take_while(move |(candidate, _)| candidate.starts_with(path))
    }

    fn clone_tree(&self, path: &Path) -> Vec<(PathBuf, Node)> {
        self.subtree(path)
            .map(|(key, node)| (suffix(key, path), node.clone()))
            .collect()
    }

    fn take_tree(&mut self, path: &Path) -> Vec<(PathBuf, Node)> {
        let keys: Vec<_> = self.subtree(path).map(|(key, _)| key.clone()).collect();
        keys.into_iter()
            .filter_map(|key| {
                self.nodes
                    .remove(&key)
                    .map(|node| (suffix(&key, path), node))
            })
            .collect()
    }

    fn place_tree(&mut self, root: &Path, tree: Vec<(PathBuf, Node)>) {
        self.nodes
            .extend(tree.into_iter().map(|(path, node)| (root.join(path), node)));
    }

    fn prepare_destination(&mut self, path: &Path, overwrite: bool) -> Result<(), ServiceError> {
        if self.nodes.contains_key(path) {
            if !overwrite {
                return Err(ServiceError::new(
                    "AlreadyExists",
                    "destination already exists",
                ));
            }
            self.take_tree(path);
        }
        Ok(())
    }
}

fn suffix(path: &Path, root: &Path) -> PathBuf {
    path.strip_prefix(root)
        .expect("subtree path lies beneath root")
        .to_owned()
}
fn covered(path: &Path, roots: &[Root]) -> bool {
    roots
        .iter()
        .any(|root| path.starts_with(root.path()) && path != root.path())
}
fn not_found() -> ServiceError {
    ServiceError::new("NotFound", "fixture system entry not found")
}
fn invalid() -> ServiceError {
    ServiceError::new(
        "InvalidPath",
        "operation requires an entry beneath the granted root",
    )
}

/// Verify that a fixture path can be seeded without changing its stipulated meaning.
pub fn valid_preview_path(path: &str) -> bool {
    path == "/"
        || path.starts_with('/')
            && path[1..]
                .split('/')
                .all(|part| !part.is_empty() && !matches!(part, "." | "..") && !part.contains('\0'))
}
