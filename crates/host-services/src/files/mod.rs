//! Scoped local file I/O, XDG FileChooser Portal dialogs, persistent bookmarks,
//! debounced directory watching, and in-memory preview virtual filesystem.
#![forbid(unsafe_code)]

pub mod grants;
pub mod portal;
pub mod vfs;
pub mod watcher;

use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::UNIX_EPOCH,
};

use serde::{Deserialize, Serialize};

use crate::ServiceError;

pub const MAX_READ_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_WRITE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_DIR_ENTRIES: usize = 10_000;
pub const DEFAULT_WATCH_DEBOUNCE_MS: u64 = 100;
pub const MIN_WATCH_DEBOUNCE_MS: u64 = 10;
pub const MAX_WATCH_DEBOUNCE_MS: u64 = 5_000;

static TMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Access level granted to a file or directory capability handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileMode {
    Read,
    ReadWrite,
}

impl FileMode {
    #[must_use]
    pub const fn can_write(self) -> bool {
        matches!(self, Self::ReadWrite)
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::ReadWrite => "readwrite",
        }
    }
}

/// Kind of filesystem capability handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HandleKind {
    File,
    Dir,
}

/// Metadata returned by `stat(...)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStat {
    pub is_file: bool,
    pub is_dir: bool,
    pub size: u64,
    pub modified_ms: Option<u64>,
}

/// Entry returned by `readDir(...)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_file: bool,
    pub is_dir: bool,
    pub size: u64,
}

/// File-chooser filter group (`*.md`, etc.).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogFilter {
    pub name: String,
    pub extensions: Vec<String>,
}

/// Platform-neutral options for file/directory picker dialogs.
#[derive(Clone, Debug, Default)]
pub struct DialogOptions {
    pub title: Option<String>,
    pub suggested_name: Option<String>,
    pub filters: Vec<DialogFilter>,
}

/// Kind of coalesced directory watcher change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WatchEventKind {
    Create,
    Modify,
    Remove,
    Rename,
    Other,
}

impl WatchEventKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Modify => "modify",
            Self::Remove => "remove",
            Self::Rename => "rename",
            Self::Other => "other",
        }
    }
}

/// One coalesced directory change event relative to the watched `DirHandle`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WatchEvent {
    pub path: String,
    pub kind: WatchEventKind,
}

/// Resolves the live XDG documents directory (`$XDG_DOCUMENTS_DIR`, `~/.config/user-dirs.dirs`, or `$HOME/Documents`).
pub fn resolve_documents_dir() -> Result<PathBuf, ServiceError> {
    if let Ok(xdg) = env::var("XDG_DOCUMENTS_DIR")
        && !xdg.is_empty()
    {
        let p = PathBuf::from(&xdg);
        if p.is_absolute() {
            return Ok(p);
        }
    }
    let home = env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| ServiceError::new("InvalidState", "HOME is not set to an absolute path"))?;

    let config_home = env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"));

    if let Ok(text) = fs::read_to_string(config_home.join("user-dirs.dirs")) {
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("XDG_DOCUMENTS_DIR=") {
                let unquoted = rest.trim().trim_matches('"');
                if let Some(sub) = unquoted.strip_prefix("$HOME/") {
                    return Ok(home.join(sub));
                }
                let p = PathBuf::from(unquoted);
                if p.is_absolute() {
                    return Ok(p);
                }
            }
        }
    }
    Ok(home.join("Documents"))
}

/// Validates a relative path beneath a `DirHandle` (rejecting absolute paths, `..`, `.`, `\`, and NUL).
pub fn parse_relative_segments(rel: &str) -> Result<Vec<&str>, ServiceError> {
    if rel.is_empty() {
        return Ok(Vec::new());
    }
    if rel.starts_with('/') || rel.contains(['\\', '\0']) {
        return Err(ServiceError::new(
            "InvalidPath",
            format!("relative path must not be absolute or contain backslashes/NUL: {rel:?}"),
        ));
    }
    let mut parts = Vec::new();
    for part in rel.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(ServiceError::new(
                "InvalidPath",
                format!("relative path contains forbidden segment: {rel:?}"),
            ));
        }
        parts.push(part);
    }
    if !Path::new(rel)
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(ServiceError::new(
            "InvalidPath",
            format!("relative path contains non-normal components: {rel:?}"),
        ));
    }
    Ok(parts)
}

/// Strictly resolves `rel_path` beneath `root_canonical`, blocking `..` traversal and symlink escapes.
///
/// When `allow_missing_leaf` is `true`, the final component need not exist yet, while every
/// existing ancestor (and the leaf itself if it already exists) is verified to resolve canonically
/// within `root_canonical`.
pub fn resolve_scoped_path(
    root_canonical: &Path,
    rel_path: &str,
    allow_missing_leaf: bool,
) -> Result<PathBuf, ServiceError> {
    let segments = parse_relative_segments(rel_path)?;
    if segments.is_empty() {
        return Ok(root_canonical.to_path_buf());
    }

    let mut current = root_canonical.to_path_buf();
    for (idx, seg) in segments.iter().enumerate() {
        let is_last = idx + 1 == segments.len();
        let next = current.join(seg);
        match fs::symlink_metadata(&next) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    let resolved = fs::canonicalize(&next).map_err(|_| {
                        ServiceError::new(
                            "E_PATH_ESCAPE",
                            format!("dangling or unresolvable symlink rejected: {rel_path:?}"),
                        )
                    })?;
                    if !resolved.starts_with(root_canonical) {
                        return Err(ServiceError::new(
                            "E_PATH_ESCAPE",
                            format!("symlink escapes authorized directory root: {rel_path:?}"),
                        ));
                    }
                    current = resolved;
                } else {
                    current = next;
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound && is_last && allow_missing_leaf => {
                // Verify parent stays strictly inside root_canonical before returning missing leaf.
                let parent_canonical = fs::canonicalize(&current).map_err(ServiceError::from)?;
                if !parent_canonical.starts_with(root_canonical) {
                    return Err(ServiceError::new(
                        "E_PATH_ESCAPE",
                        format!("parent directory escapes authorized root: {rel_path:?}"),
                    ));
                }
                return Ok(parent_canonical.join(seg));
            }
            Err(err) => return Err(map_io_error(err)),
        }
    }

    let canonical = fs::canonicalize(&current).map_err(map_io_error)?;
    if !canonical.starts_with(root_canonical) {
        return Err(ServiceError::new(
            "E_PATH_ESCAPE",
            format!("resolved path escapes authorized directory root: {rel_path:?}"),
        ));
    }
    Ok(canonical)
}

/// Reads up to `max_bytes` from a regular file (never trusting metadata length).
pub fn read_file_bytes(path: &Path, max_bytes: usize) -> Result<Vec<u8>, ServiceError> {
    let cap = max_bytes.min(MAX_READ_BYTES);
    let file = File::open(path).map_err(map_io_error)?;
    let meta = file.metadata().map_err(map_io_error)?;
    if !meta.is_file() {
        return Err(ServiceError::new("InvalidPath", "target is not a regular file"));
    }
    let limit = u64::try_from(cap).unwrap_or(u64::MAX).saturating_add(1);
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes).map_err(map_io_error)?;
    if bytes.len() > cap {
        return Err(ServiceError::new(
            "TooLarge",
            format!("file exceeds read byte limit of {cap} bytes"),
        ));
    }
    Ok(bytes)
}

/// Reads a bounded UTF-8 text file.
pub fn read_file_text(path: &Path, max_bytes: usize) -> Result<String, ServiceError> {
    let bytes = read_file_bytes(path, max_bytes)?;
    String::from_utf8(bytes)
        .map_err(|err| ServiceError::new("InvalidUtf8", err.utf8_error().to_string()))
}

/// Atomically writes `bytes` to `target_path` via a same-directory temporary file, `fsync`, and `rename`.
pub fn atomic_write_bytes(
    target_path: &Path,
    bytes: &[u8],
    mode_octal: Option<u32>,
) -> Result<(), ServiceError> {
    if bytes.len() > MAX_WRITE_BYTES {
        return Err(ServiceError::new(
            "TooLarge",
            format!("write payload exceeds maximum of {MAX_WRITE_BYTES} bytes"),
        ));
    }
    let parent = target_path
        .parent()
        .ok_or_else(|| ServiceError::new("InvalidPath", "target file has no parent directory"))?;
    if !parent.is_dir() {
        return Err(ServiceError::new("NotFound", "parent directory does not exist"));
    }
    let pid = std::process::id();
    let mut tmp_path = PathBuf::new();
    let mut file = None;
    for _ in 0..16 {
        let seq = TMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(".zgui-tmp-{pid}-{seq}"));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode_octal.unwrap_or(0o666))
            .open(&candidate)
        {
            Ok(opened) => {
                tmp_path = candidate;
                file = Some(opened);
                break;
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(map_io_error(err)),
        }
    }
    let mut file = file.ok_or_else(|| {
        ServiceError::new("Io", "failed to create unique temporary file for atomic write")
    })?;

    let write_res = (|| -> io::Result<()> {
        if let Some(octal) = mode_octal {
            file.set_permissions(fs::Permissions::from_mode(octal))?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    })();
    drop(file);

    if let Err(err) = write_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(map_io_error(err));
    }
    if let Err(err) = fs::rename(&tmp_path, target_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(map_io_error(err));
    }
    if let Ok(dir_handle) = File::open(parent) {
        let _ = dir_handle.sync_all();
    }
    Ok(())
}

/// Returns file/directory metadata.
pub fn stat_path(path: &Path) -> Result<FileStat, ServiceError> {
    let meta = fs::metadata(path).map_err(map_io_error)?;
    let modified_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64);
    Ok(FileStat {
        is_file: meta.is_file(),
        is_dir: meta.is_dir(),
        size: meta.len(),
        modified_ms,
    })
}

/// Lists directory entries sorted deterministically by name, capped at `MAX_DIR_ENTRIES`.
pub fn read_dir_entries(root_canonical: &Path, dir_path: &Path) -> Result<Vec<DirEntry>, ServiceError> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir_path).map_err(map_io_error)? {
        if entries.len() >= MAX_DIR_ENTRIES {
            return Err(ServiceError::new(
                "E_LIMIT",
                format!("directory listing exceeds {MAX_DIR_ENTRIES} entries"),
            ));
        }
        let entry = entry.map_err(map_io_error)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // Skip internal atomic-write temporary files from directory listings.
        if name.starts_with(".zgui-tmp-") {
            continue;
        }
        let entry_path = entry.path();
        let sym_meta = fs::symlink_metadata(&entry_path).map_err(map_io_error)?;
        if sym_meta.file_type().is_symlink() {
            // Only expose symlinks whose canonical targets stay inside root_canonical.
            let Ok(target) = fs::canonicalize(&entry_path) else {
                continue;
            };
            if !target.starts_with(root_canonical) {
                continue;
            }
            let target_meta = fs::metadata(&target).map_err(map_io_error)?;
            entries.push(DirEntry {
                name,
                is_file: target_meta.is_file(),
                is_dir: target_meta.is_dir(),
                size: target_meta.len(),
            });
        } else {
            entries.push(DirEntry {
                name,
                is_file: sym_meta.is_file(),
                is_dir: sym_meta.is_dir(),
                size: sym_meta.len(),
            });
        }
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// Creates a directory strictly beneath `root_canonical`.
pub fn mkdir_scoped(
    root_canonical: &Path,
    rel_path: &str,
    recursive: bool,
) -> Result<(), ServiceError> {
    let segments = parse_relative_segments(rel_path)?;
    if segments.is_empty() {
        return Err(ServiceError::new(
            "InvalidPath",
            "mkdir requires a non-empty relative path",
        ));
    }
    if recursive {
        let mut current = root_canonical.to_path_buf();
        for seg in segments {
            let next = current.join(seg);
            match fs::symlink_metadata(&next) {
                Ok(meta) => {
                    if meta.file_type().is_symlink() {
                        let resolved = fs::canonicalize(&next).map_err(|_| {
                            ServiceError::new("E_PATH_ESCAPE", "symlink rejected in mkdir")
                        })?;
                        if !resolved.starts_with(root_canonical) {
                            return Err(ServiceError::new(
                                "E_PATH_ESCAPE",
                                "symlink escapes root in mkdir",
                            ));
                        }
                        current = resolved;
                    } else if meta.is_dir() {
                        current = next;
                    } else {
                        return Err(ServiceError::new(
                            "AlreadyExists",
                            "path component exists and is not a directory",
                        ));
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    fs::create_dir(&next).map_err(map_io_error)?;
                    current = next;
                }
                Err(err) => return Err(map_io_error(err)),
            }
        }
        Ok(())
    } else {
        let target = resolve_scoped_path(root_canonical, rel_path, true)?;
        fs::create_dir(target).map_err(map_io_error)
    }
}

/// Renames an entry strictly within `root_canonical`.
pub fn rename_scoped(
    root_canonical: &Path,
    from_rel: &str,
    to_rel: &str,
) -> Result<(), ServiceError> {
    if from_rel.is_empty() || to_rel.is_empty() {
        return Err(ServiceError::new(
            "InvalidPath",
            "rename requires non-empty relative source and destination paths",
        ));
    }
    let from = resolve_scoped_path(root_canonical, from_rel, false)?;
    let to = resolve_scoped_path(root_canonical, to_rel, true)?;
    if from == root_canonical || to == root_canonical {
        return Err(ServiceError::new(
            "InvalidPath",
            "cannot rename the authorized directory root itself",
        ));
    }
    fs::rename(from, to).map_err(map_io_error)
}

/// Removes a file or directory strictly within `root_canonical`.
pub fn remove_scoped(
    root_canonical: &Path,
    rel_path: &str,
    recursive: bool,
) -> Result<(), ServiceError> {
    if rel_path.is_empty() {
        return Err(ServiceError::new(
            "InvalidPath",
            "cannot remove the authorized directory root itself",
        ));
    }
    let target = resolve_scoped_path(root_canonical, rel_path, false)?;
    if target == root_canonical {
        return Err(ServiceError::new(
            "InvalidPath",
            "cannot remove the authorized directory root itself",
        ));
    }
    let meta = fs::symlink_metadata(&target).map_err(map_io_error)?;
    if meta.is_dir() {
        if recursive {
            fs::remove_dir_all(&target).map_err(map_io_error)
        } else {
            fs::remove_dir(&target).map_err(map_io_error)
        }
    } else {
        fs::remove_file(&target).map_err(map_io_error)
    }
}

pub(crate) fn map_io_error(error: io::Error) -> ServiceError {
    let code = match error.kind() {
        io::ErrorKind::NotFound => "NotFound",
        io::ErrorKind::PermissionDenied => "PermissionDenied",
        io::ErrorKind::AlreadyExists => "AlreadyExists",
        io::ErrorKind::InvalidInput => "InvalidPath",
        _ => "Io",
    };
    ServiceError::new(code, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn fresh_temp_dir(label: &str) -> PathBuf {
        let id = TMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = env::temp_dir().join(format!("zgui-files-test-{label}-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp test dir");
        dir.canonicalize().expect("canonical temp dir")
    }

    #[test]
    fn scoped_path_rejects_traversal_and_symlink_escapes() {
        let root = fresh_temp_dir("root");
        let outside = fresh_temp_dir("outside");
        let secret_file = outside.join("secret.txt");
        fs::write(&secret_file, b"top-secret").unwrap();

        // Direct traversal rejected.
        assert_eq!(
            resolve_scoped_path(&root, "../outside/secret.txt", false)
                .expect_err(".. traversal must fail")
                .code,
            "InvalidPath"
        );
        assert_eq!(
            resolve_scoped_path(&root, "/etc/passwd", false)
                .expect_err("absolute path must fail")
                .code,
            "InvalidPath"
        );

        // Symlink inside root pointing outside root rejected on read and write!
        symlink(&secret_file, root.join("escape-file")).unwrap();
        assert_eq!(
            resolve_scoped_path(&root, "escape-file", false)
                .expect_err("escaping file symlink must fail")
                .code,
            "E_PATH_ESCAPE"
        );
        assert_eq!(
            resolve_scoped_path(&root, "escape-file", true)
                .expect_err("escaping file symlink must fail even with allow_missing_leaf")
                .code,
            "E_PATH_ESCAPE"
        );

        // Symlink directory inside root pointing outside root rejected for child access!
        symlink(&outside, root.join("escape-dir")).unwrap();
        assert_eq!(
            resolve_scoped_path(&root, "escape-dir/secret.txt", false)
                .expect_err("escaping dir symlink must fail")
                .code,
            "E_PATH_ESCAPE"
        );
        assert_eq!(
            resolve_scoped_path(&root, "escape-dir/new.txt", true)
                .expect_err("write through escaping dir symlink must fail")
                .code,
            "E_PATH_ESCAPE"
        );

        // Confined relative symlink inside root is allowed.
        fs::write(root.join("note.md"), b"hello").unwrap();
        symlink(root.join("note.md"), root.join("alias.md")).unwrap();
        let resolved = resolve_scoped_path(&root, "alias.md", false).expect("confined symlink ok");
        assert_eq!(resolved, root.join("note.md"));

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn atomic_write_replaces_content_and_leaves_no_temp_files() {
        let root = fresh_temp_dir("atomic");
        let target = root.join("doc.md");

        atomic_write_bytes(&target, b"v1 content", None).expect("write v1");
        assert_eq!(read_file_text(&target, 1024).unwrap(), "v1 content");

        atomic_write_bytes(&target, b"v2 updated content", None).expect("write v2");
        assert_eq!(read_file_text(&target, 1024).unwrap(), "v2 updated content");

        // Verify no leftover .zgui-tmp-* files remain.
        let entries = read_dir_entries(&root, &root).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "doc.md");
        let raw_names: Vec<String> = fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(raw_names, vec!["doc.md"]);

        let _ = fs::remove_dir_all(&root);
    }
}
