//! Pure in-memory virtual filesystem, bookmark store, scripted dialogs, and watchers for preview.
//! Zero host filesystem or D-Bus access occurs here.
#![forbid(unsafe_code)]

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};

use crate::{
    ServiceError,
    files::{
        DirEntry, FileMode, FileStat, HandleKind, MAX_DIR_ENTRIES, MAX_READ_BYTES, MAX_WRITE_BYTES,
        WatchEvent, WatchEventKind, grants::BookmarkRecord, parse_relative_segments,
    },
};

/// One node in the preview virtual filesystem.
#[derive(Clone, Debug)]
pub enum VfsNode {
    File { bytes: Vec<u8>, modified_ms: u64 },
    Dir,
    Symlink { target: String },
}

/// Scripted dialog response queued by a preview fixture.
#[derive(Clone, Debug)]
pub struct ScriptedDialog {
    pub kind: ScriptedDialogKind,
    pub path: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptedDialogKind {
    OpenFile,
    SaveFile,
    OpenDirectory,
}

struct PreviewWatcher {
    root: String,
    recursive: bool,
    pending: VecDeque<Vec<WatchEvent>>,
}

/// Deterministic in-memory filesystem + grant table + scripted dialogs for `zgui-host preview`.
pub struct PreviewFs {
    nodes: BTreeMap<String, VfsNode>,
    dialogs: VecDeque<ScriptedDialog>,
    bookmarks: Vec<BookmarkRecord>,
    watchers: BTreeMap<u64, PreviewWatcher>,
    next_token: u64,
    next_watcher_id: u64,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Default for PreviewFs {
    fn default() -> Self {
        let mut nodes = BTreeMap::new();
        nodes.insert("/".to_owned(), VfsNode::Dir);
        Self {
            nodes,
            dialogs: VecDeque::new(),
            bookmarks: Vec::new(),
            watchers: BTreeMap::new(),
            next_token: 1,
            next_watcher_id: 1,
            wake: None,
        }
    }
}

impl PreviewFs {
    pub fn set_wake(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.wake = Some(wake);
    }

    /// Seeds a virtual directory (and all ancestor directories).
    pub fn seed_dir(&mut self, canonical_path: &str) {
        let normalized = normalize_abs(canonical_path);
        ensure_ancestors(&mut self.nodes, &normalized);
        self.nodes.insert(normalized, VfsNode::Dir);
    }

    /// Seeds a virtual regular file (creating parent directories automatically).
    pub fn seed_file(&mut self, canonical_path: &str, bytes: Vec<u8>, modified_ms: u64) {
        let normalized = normalize_abs(canonical_path);
        ensure_ancestors(&mut self.nodes, &normalized);
        self.nodes
            .insert(normalized, VfsNode::File { bytes, modified_ms });
    }

    /// Seeds a synthetic symlink (used to deterministically verify symlink-escape rejection).
    pub fn seed_symlink(&mut self, canonical_path: &str, target: String) {
        let normalized = normalize_abs(canonical_path);
        ensure_ancestors(&mut self.nodes, &normalized);
        self.nodes.insert(normalized, VfsNode::Symlink { target });
    }

    pub fn push_dialog(&mut self, dialog: ScriptedDialog) {
        self.dialogs.push_back(dialog);
    }

    pub fn seed_bookmark(&mut self, record: BookmarkRecord) {
        self.bookmarks.push(record);
    }

    /// Consumes the next scripted dialog response matching `kind`.
    pub fn take_dialog(&mut self, kind: ScriptedDialogKind) -> Result<Option<String>, ServiceError> {
        let Some(next) = self.dialogs.pop_front() else {
            return Err(ServiceError::new(
                "E_FIXTURE_CONTRACT",
                "preview fixture has no scripted dialog response queued",
            ));
        };
        if next.kind != kind {
            return Err(ServiceError::new(
                "E_FIXTURE_CONTRACT",
                format!(
                    "scripted dialog mismatch: expected {:?}, got {:?}",
                    kind, next.kind
                ),
            ));
        }
        Ok(next.path.map(|p| normalize_abs(&p)))
    }

    /// Strictly resolves `rel_path` beneath `root`, rejecting `..` and symlink escapes.
    pub fn resolve_scoped(
        &self,
        root: &str,
        rel_path: &str,
        allow_missing_leaf: bool,
    ) -> Result<String, ServiceError> {
        let root = normalize_abs(root);
        let segments = parse_relative_segments(rel_path)?;
        if segments.is_empty() {
            return Ok(root);
        }
        let mut current = root.clone();
        for (idx, seg) in segments.iter().enumerate() {
            let is_last = idx + 1 == segments.len();
            let candidate = join_abs(&current, seg);
            match self.nodes.get(&candidate) {
                Some(VfsNode::Symlink { target }) => {
                    let resolved = if target.starts_with('/') {
                        normalize_abs(target)
                    } else {
                        join_abs(&current, target)
                    };
                    if !is_within_root(&resolved, &root) || !self.nodes.contains_key(&resolved) {
                        return Err(ServiceError::new(
                            "E_PATH_ESCAPE",
                            format!("symlink escapes authorized directory root: {rel_path:?}"),
                        ));
                    }
                    current = resolved;
                }
                Some(_) => {
                    current = candidate;
                }
                None if is_last && allow_missing_leaf => {
                    if !is_within_root(&current, &root) {
                        return Err(ServiceError::new(
                            "E_PATH_ESCAPE",
                            format!("parent escapes root: {rel_path:?}"),
                        ));
                    }
                    return Ok(candidate);
                }
                None => {
                    return Err(ServiceError::new(
                        "NotFound",
                        format!("virtual path not found: {rel_path:?}"),
                    ));
                }
            }
        }
        if !is_within_root(&current, &root) {
            return Err(ServiceError::new(
                "E_PATH_ESCAPE",
                format!("resolved path escapes root: {rel_path:?}"),
            ));
        }
        Ok(current)
    }

    pub fn read_bytes(&self, abs_path: &str, max_bytes: usize) -> Result<Vec<u8>, ServiceError> {
        let cap = max_bytes.min(MAX_READ_BYTES);
        let path = normalize_abs(abs_path);
        match self.nodes.get(&path) {
            Some(VfsNode::File { bytes, .. }) => {
                if bytes.len() > cap {
                    return Err(ServiceError::new(
                        "TooLarge",
                        format!("file exceeds read byte limit of {cap} bytes"),
                    ));
                }
                Ok(bytes.clone())
            }
            Some(_) => Err(ServiceError::new("InvalidPath", "target is not a regular file")),
            None => Err(ServiceError::new("NotFound", format!("file not found: {path}"))),
        }
    }

    pub fn read_text(&self, abs_path: &str, max_bytes: usize) -> Result<String, ServiceError> {
        let bytes = self.read_bytes(abs_path, max_bytes)?;
        String::from_utf8(bytes)
            .map_err(|err| ServiceError::new("InvalidUtf8", err.utf8_error().to_string()))
    }

    pub fn write_bytes(
        &mut self,
        abs_path: &str,
        bytes: Vec<u8>,
        now_ms: u64,
    ) -> Result<(), ServiceError> {
        if bytes.len() > MAX_WRITE_BYTES {
            return Err(ServiceError::new(
                "TooLarge",
                format!("write payload exceeds maximum of {MAX_WRITE_BYTES} bytes"),
            ));
        }
        let path = normalize_abs(abs_path);
        let parent = parent_abs(&path);
        if !matches!(self.nodes.get(&parent), Some(VfsNode::Dir)) {
            return Err(ServiceError::new("NotFound", "parent directory does not exist"));
        }
        let existed = self.nodes.contains_key(&path);
        if matches!(self.nodes.get(&path), Some(VfsNode::Dir)) {
            return Err(ServiceError::new("InvalidPath", "cannot overwrite directory with file"));
        }
        self.nodes
            .insert(path.clone(), VfsNode::File { bytes, modified_ms: now_ms });
        self.notify_watchers(
            &path,
            if existed {
                WatchEventKind::Modify
            } else {
                WatchEventKind::Create
            },
        );
        Ok(())
    }

    pub fn stat(&self, abs_path: &str) -> Result<FileStat, ServiceError> {
        let path = normalize_abs(abs_path);
        match self.nodes.get(&path) {
            Some(VfsNode::File { bytes, modified_ms }) => Ok(FileStat {
                is_file: true,
                is_dir: false,
                size: bytes.len() as u64,
                modified_ms: Some(*modified_ms),
            }),
            Some(VfsNode::Dir) => Ok(FileStat {
                is_file: false,
                is_dir: true,
                size: 0,
                modified_ms: None,
            }),
            Some(VfsNode::Symlink { .. }) => Err(ServiceError::new("E_PATH_ESCAPE", "unresolved symlink")),
            None => Err(ServiceError::new("NotFound", format!("path not found: {path}"))),
        }
    }

    pub fn read_dir(&self, root: &str, dir_path: &str) -> Result<Vec<DirEntry>, ServiceError> {
        let root = normalize_abs(root);
        let dir = normalize_abs(dir_path);
        if !matches!(self.nodes.get(&dir), Some(VfsNode::Dir)) {
            return Err(ServiceError::new("NotFound", format!("directory not found: {dir}")));
        }
        let prefix = if dir == "/" {
            "/".to_owned()
        } else {
            format!("{dir}/")
        };
        let mut entries = Vec::new();
        for (path, node) in &self.nodes {
            let Some(rest) = path.strip_prefix(&prefix) else {
                continue;
            };
            if rest.is_empty() || rest.contains('/') {
                continue;
            }
            if entries.len() >= MAX_DIR_ENTRIES {
                return Err(ServiceError::new(
                    "E_LIMIT",
                    format!("directory listing exceeds {MAX_DIR_ENTRIES} entries"),
                ));
            }
            match node {
                VfsNode::File { bytes, .. } => entries.push(DirEntry {
                    name: rest.to_owned(),
                    is_file: true,
                    is_dir: false,
                    size: bytes.len() as u64,
                }),
                VfsNode::Dir => entries.push(DirEntry {
                    name: rest.to_owned(),
                    is_file: false,
                    is_dir: true,
                    size: 0,
                }),
                VfsNode::Symlink { target } => {
                    let resolved = if target.starts_with('/') {
                        normalize_abs(target)
                    } else {
                        join_abs(&dir, target)
                    };
                    if !is_within_root(&resolved, &root) {
                        continue;
                    }
                    match self.nodes.get(&resolved) {
                        Some(VfsNode::File { bytes, .. }) => entries.push(DirEntry {
                            name: rest.to_owned(),
                            is_file: true,
                            is_dir: false,
                            size: bytes.len() as u64,
                        }),
                        Some(VfsNode::Dir) => entries.push(DirEntry {
                            name: rest.to_owned(),
                            is_file: false,
                            is_dir: true,
                            size: 0,
                        }),
                        _ => {}
                    }
                }
            }
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    pub fn mkdir(&mut self, root: &str, rel_path: &str, recursive: bool) -> Result<(), ServiceError> {
        let segments = parse_relative_segments(rel_path)?;
        if segments.is_empty() {
            return Err(ServiceError::new("InvalidPath", "mkdir requires a non-empty relative path"));
        }
        let root = normalize_abs(root);
        if recursive {
            let mut cur = root.clone();
            for seg in segments {
                let next = join_abs(&cur, seg);
                match self.nodes.get(&next) {
                    Some(VfsNode::Dir) => cur = next,
                    Some(_) => {
                        return Err(ServiceError::new(
                            "AlreadyExists",
                            "path component exists and is not a directory",
                        ));
                    }
                    None => {
                        self.nodes.insert(next.clone(), VfsNode::Dir);
                        self.notify_watchers(&next, WatchEventKind::Create);
                        cur = next;
                    }
                }
            }
            Ok(())
        } else {
            let target = self.resolve_scoped(&root, rel_path, true)?;
            if self.nodes.contains_key(&target) {
                return Err(ServiceError::new("AlreadyExists", "directory already exists"));
            }
            let parent = parent_abs(&target);
            if !matches!(self.nodes.get(&parent), Some(VfsNode::Dir)) {
                return Err(ServiceError::new("NotFound", "parent directory does not exist"));
            }
            self.nodes.insert(target.clone(), VfsNode::Dir);
            self.notify_watchers(&target, WatchEventKind::Create);
            Ok(())
        }
    }

    pub fn rename(&mut self, root: &str, from_rel: &str, to_rel: &str) -> Result<(), ServiceError> {
        if from_rel.is_empty() || to_rel.is_empty() {
            return Err(ServiceError::new(
                "InvalidPath",
                "rename requires non-empty source and destination paths",
            ));
        }
        let from = self.resolve_scoped(root, from_rel, false)?;
        let to = self.resolve_scoped(root, to_rel, true)?;
        let parent = parent_abs(&to);
        if !matches!(self.nodes.get(&parent), Some(VfsNode::Dir)) {
            return Err(ServiceError::new("NotFound", "destination parent directory does not exist"));
        }
        let prefix = format!("{from}/");
        let moving: Vec<(String, VfsNode)> = self
            .nodes
            .keys()
            .filter(|k| *k == &from || k.starts_with(&prefix))
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|k| self.nodes.remove(&k).map(|v| (k, v)))
            .collect();
        for (old_key, node) in moving {
            let new_key = if old_key == from {
                to.clone()
            } else {
                format!("{to}{}", &old_key[from.len()..])
            };
            self.nodes.insert(new_key, node);
        }
        self.notify_watchers(&to, WatchEventKind::Rename);
        Ok(())
    }

    pub fn remove(&mut self, root: &str, rel_path: &str, recursive: bool) -> Result<(), ServiceError> {
        if rel_path.is_empty() {
            return Err(ServiceError::new("InvalidPath", "cannot remove directory root"));
        }
        let target = self.resolve_scoped(root, rel_path, false)?;
        let prefix = format!("{target}/");
        let has_children = self.nodes.keys().any(|k| k.starts_with(&prefix));
        if has_children && !recursive {
            return Err(ServiceError::new("Io", "directory is not empty"));
        }
        self.nodes.remove(&target);
        if recursive {
            self.nodes.retain(|k, _| !k.starts_with(&prefix));
        }
        self.notify_watchers(&target, WatchEventKind::Remove);
        Ok(())
    }

    pub fn persist_bookmark(
        &mut self,
        kind: HandleKind,
        path: &str,
        mode: FileMode,
        now_ms: u64,
    ) -> BookmarkRecord {
        let normalized = normalize_abs(path);
        if let Some(existing) = self
            .bookmarks
            .iter_mut()
            .find(|b| b.kind == kind && b.path == normalized)
        {
            if mode.can_write() {
                existing.mode = FileMode::ReadWrite;
            }
            return existing.clone();
        }
        let id = self.next_token;
        self.next_token += 1;
        let label = normalized
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("/")
            .to_owned();
        let record = BookmarkRecord {
            token: format!("preview-bookmark-{id:08x}"),
            kind,
            path: normalized,
            label,
            mode,
            created_ms: now_ms,
        };
        self.bookmarks.push(record.clone());
        record
    }

    pub fn restore_bookmark(&self, token: &str) -> Result<BookmarkRecord, ServiceError> {
        self.bookmarks
            .iter()
            .find(|b| b.token == token)
            .cloned()
            .ok_or_else(|| ServiceError::new("NotFound", "bookmark token not found or revoked"))
    }

    pub fn list_bookmarks(&self) -> Vec<BookmarkRecord> {
        self.bookmarks.clone()
    }

    pub fn revoke_bookmark(&mut self, token: &str) -> bool {
        let before = self.bookmarks.len();
        self.bookmarks.retain(|b| b.token != token);
        self.bookmarks.len() != before
    }

    pub fn add_watcher(&mut self, root: &str, recursive: bool) -> u64 {
        let id = self.next_watcher_id;
        self.next_watcher_id += 1;
        self.watchers.insert(
            id,
            PreviewWatcher {
                root: normalize_abs(root),
                recursive,
                pending: VecDeque::new(),
            },
        );
        id
    }

    pub fn remove_watcher(&mut self, id: u64) {
        self.watchers.remove(&id);
    }

    pub fn take_watcher_events(&mut self, id: u64) -> Option<Vec<WatchEvent>> {
        self.watchers.get_mut(&id)?.pending.pop_front()
    }

    fn notify_watchers(&mut self, abs_path: &str, kind: WatchEventKind) {
        let mut woke = false;
        for watcher in self.watchers.values_mut() {
            let prefix = if watcher.root == "/" {
                "/".to_owned()
            } else {
                format!("{}/", watcher.root)
            };
            let Some(rel) = abs_path.strip_prefix(&prefix) else {
                continue;
            };
            if !watcher.recursive && rel.contains('/') {
                continue;
            }
            let event = WatchEvent {
                path: rel.to_owned(),
                kind,
            };
            if let Some(last_batch) = watcher.pending.back_mut() {
                let mut set: BTreeSet<WatchEvent> = last_batch.drain(..).collect();
                set.insert(event);
                *last_batch = set.into_iter().collect();
            } else {
                watcher.pending.push_back(vec![event]);
            }
            woke = true;
        }
        if woke && let Some(wake) = &self.wake {
            wake();
        }
    }
}

fn ensure_ancestors(nodes: &mut BTreeMap<String, VfsNode>, abs_path: &str) {
    let mut cur = String::new();
    for seg in abs_path.split('/').filter(|s| !s.is_empty()) {
        if !cur.is_empty() {
            nodes.entry(cur.clone()).or_insert(VfsNode::Dir);
        }
        cur.push('/');
        cur.push_str(seg);
    }
}

pub(crate) fn normalize_abs(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_owned()
    } else if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    }
}

fn join_abs(base: &str, seg: &str) -> String {
    if base == "/" {
        format!("/{seg}")
    } else {
        format!("{base}/{seg}")
    }
}

fn parent_abs(path: &str) -> String {
    match path.rfind('/') {
        Some(0) | None => "/".to_owned(),
        Some(idx) => path[..idx].to_owned(),
    }
}

fn is_within_root(path: &str, root: &str) -> bool {
    if root == "/" {
        path.starts_with('/')
    } else {
        path == root || path.starts_with(&format!("{root}/"))
    }
}
