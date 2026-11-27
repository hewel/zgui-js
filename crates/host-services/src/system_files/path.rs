use std::sync::Arc;
use std::{
    ffi::{CString, OsStr, OsString},
    fs::{self, File},
    io,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::{OsStrExt, OsStringExt},
    },
    path::{Component, Path, PathBuf},
};

use super::EntryKind;
use crate::{
    ServiceError,
    files::{FileMode, map_io_error},
};

/// An explicitly authorized directory and its access mode.
#[derive(Clone, Debug)]
pub struct Root {
    path: Arc<Path>,
    mode: FileMode,
    user_authorized: bool,
    entry_only: Option<Arc<OsStr>>,
}

impl Root {
    /// Resolve an explicitly granted live directory.
    pub fn open(path: &Path, mode: FileMode) -> Result<Self, ServiceError> {
        let path = fs::canonicalize(path).map_err(map_io_error)?;
        Self::reopen(path, mode)
    }

    /// Reopen a previously canonical grant without accepting a symlink replacement.
    pub fn reopen(path: PathBuf, mode: FileMode) -> Result<Self, ServiceError> {
        if !path.is_absolute() || path.components().any(|c| c == Component::ParentDir) {
            return Err(ServiceError::new(
                "InvalidPath",
                "system grant root is not canonical",
            ));
        }
        pin(&path, libc::O_DIRECTORY | libc::O_PATH).map_err(path_error)?;
        Ok(Self::virtual_root(path, mode))
    }

    /// Construct authority for a canonical path verified by a virtual filesystem.
    pub(super) fn virtual_root(path: PathBuf, mode: FileMode) -> Self {
        Self {
            path: path.into(),
            mode,
            user_authorized: false,
            entry_only: None,
        }
    }

    /// Canonical root path, never sent to JS as a handle property.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Access enforced by the originating handle as well as the resolved path.
    pub fn mode(&self) -> FileMode {
        self.mode
    }

    /// Mark an entire directory authorized by user selection, directory drop, or grant restore.
    pub fn with_user_authorization(mut self) -> Self {
        debug_assert!(self.entry_only.is_none());
        self.user_authorized = true;
        self
    }

    /// Reject persistence when this handle did not receive whole-directory user authorization.
    pub fn require_user_authorization(&self) -> Result<(), ServiceError> {
        if self.is_entry_only() {
            return Err(entry_only_denied());
        }
        if !self.user_authorized {
            return Err(ServiceError::new(
                "E_CAPABILITY",
                "manifest-declared system directories cannot mint persistent user grants",
            ));
        }
        Ok(())
    }

    /// Restrict a dropped non-directory to exactly one child of this root.
    pub fn confine_to_entry(mut self, name: &OsStr) -> Self {
        self.mode = FileMode::Read;
        self.user_authorized = false;
        self.entry_only = Some(name.into());
        self
    }

    /// Whether this handle grants a child rather than its containing directory.
    pub fn is_entry_only(&self) -> bool {
        self.entry_only.is_some()
    }

    /// Encode an entry relative to this whole-directory grant.
    pub fn relative_path(&self, path: &Path) -> Option<String> {
        if self.is_entry_only() {
            return None;
        }
        let mut relative = String::new();
        for part in path.strip_prefix(self.path()).ok()?.components() {
            if !relative.is_empty() {
                relative.push('/');
            }
            push_encoded_name(&mut relative, part.as_os_str());
        }
        Some(relative)
    }
}

/// Locate the dropped name under a canonical parent, without following a final symlink.
pub fn locate_dropped_entry(path: &Path) -> Result<(PathBuf, EntryKind), ServiceError> {
    locate_drop(
        path,
        |parent| fs::canonicalize(parent).map_err(map_io_error),
        |path| {
            fs::symlink_metadata(path)
                .map(|meta| EntryKind::of(&meta))
                .map_err(map_io_error)
        },
    )
}

pub(super) fn locate_drop(
    path: &Path,
    resolve: impl FnOnce(&Path) -> Result<PathBuf, ServiceError>,
    kind: impl FnOnce(&Path) -> Result<EntryKind, ServiceError>,
) -> Result<(PathBuf, EntryKind), ServiceError> {
    if path == Path::new("/") {
        return Ok((path.to_owned(), EntryKind::Dir));
    }
    let (Some(parent), Some(name)) = (path.parent().filter(|p| p.is_absolute()), path.file_name())
    else {
        return Err(ServiceError::new(
            "InvalidPath",
            "invalid dropped file path",
        ));
    };
    let path = resolve(parent)?.join(name);
    let kind = kind(&path)?;
    Ok((path, kind))
}

/// Snapshot of a handle and the additional active roots it can traverse.
#[derive(Clone, Debug)]
pub struct Scope {
    root: Root,
    other: Arc<Vec<Root>>,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Access {
    Read,
    Write,
}

impl Scope {
    /// Start with authority from one directory handle.
    pub fn new(root: Root) -> Self {
        Self {
            root,
            other: Arc::default(),
        }
    }

    /// Reuse the same active authority snapshot for a batch of entry references.
    pub fn with_roots(root: Root, other: Arc<Vec<Root>>) -> Self {
        Self { root, other }
    }

    /// Root from which an operation was requested.
    pub fn root(&self) -> &Root {
        &self.root
    }

    /// Add an active session root to the traversal snapshot.
    pub fn add_root(&mut self, root: Root) {
        Arc::make_mut(&mut self.other).push(root);
    }

    pub(super) fn lexical(&self, relative: &str, access: Access) -> Result<PathBuf, ServiceError> {
        authorize_mode(self.root.mode, access)?;
        let relative = decode_relative(relative)?;
        if self
            .root
            .entry_only
            .as_deref()
            .is_some_and(|name| relative != Path::new(name))
        {
            return Err(entry_only_denied());
        }
        Ok(self.root.path.join(relative))
    }

    pub(super) fn authorize(&self, path: &Path, access: Access) -> Result<(), ServiceError> {
        let mut covered = false;
        for root in std::iter::once(&self.root).chain(self.other.iter()) {
            if !root.is_entry_only() && path.starts_with(&root.path) {
                covered = true;
                if authorize_mode(root.mode, access).is_ok() {
                    return Ok(());
                }
            }
        }
        Err(if covered {
            denied()
        } else {
            ServiceError::new(
                "E_PATH_ESCAPE",
                "symlink target is outside the active system directory grants",
            )
        })
    }

    /// Resolve an existing authorized entry, following its final symlink.
    pub fn follow(&self, relative: &str) -> Result<PathBuf, ServiceError> {
        Ok(self.follow_with(relative, Access::Read, libc::O_PATH)?.0)
    }

    /// Resolve an existing entry for an outbound drag that may commit a Move.
    pub fn follow_for_move(&self, relative: &str) -> Result<PathBuf, ServiceError> {
        Ok(self.follow_with(relative, Access::Write, libc::O_PATH)?.0)
    }

    pub(super) fn follow_with(
        &self,
        relative: &str,
        access: Access,
        flags: i32,
    ) -> Result<(PathBuf, File), ServiceError> {
        let path = self.lexical(relative, access)?;
        if self.root.is_entry_only() {
            // Opening the exact name without following any link avoids both target probes
            // and accepting a directory substituted for a dropped regular file.
            let file = pin(&path, flags).map_err(|error| {
                if error.raw_os_error() == Some(libc::ELOOP) {
                    entry_only_denied()
                } else {
                    path_error(error)
                }
            })?;
            if !file.metadata().map_err(map_io_error)?.is_file() {
                return Err(entry_only_denied());
            }
            return Ok((path, file));
        }
        let path = self.canonical_inside(&path, access)?;
        let file = pin(&path, flags).map_err(path_error)?;
        Ok((path, file))
    }

    pub(super) fn canonical_inside(
        &self,
        path: &Path,
        access: Access,
    ) -> Result<PathBuf, ServiceError> {
        self.resolve_with(path, access, |path| {
            fs::canonicalize(path).map_err(map_io_error)
        })
    }

    pub(super) fn resolve_with(
        &self,
        path: &Path,
        access: Access,
        mut resolve: impl FnMut(&Path) -> Result<PathBuf, ServiceError>,
    ) -> Result<PathBuf, ServiceError> {
        let relative = path.strip_prefix(self.root.path()).map_err(|_| {
            ServiceError::new(
                "E_PATH_ESCAPE",
                "path did not originate in the granted root",
            )
        })?;
        let mut canonical = resolve(self.root.path())?;
        if canonical != self.root.path() {
            return Err(ServiceError::new(
                "E_PATH_ESCAPE",
                "authorized directory was replaced by a symlink",
            ));
        }
        if self.root.is_entry_only() {
            // The parent is only an anchor for the name, never an authorized target.
            return if relative.as_os_str().is_empty() {
                Ok(canonical)
            } else {
                Err(entry_only_denied())
            };
        }
        self.authorize(&canonical, access)?;
        // A link can leave the granted roots and later link back to an authorized path.
        for component in relative.components() {
            canonical.push(component.as_os_str());
            canonical = resolve(&canonical)?;
            self.authorize(&canonical, access)?;
        }
        Ok(canonical)
    }

    pub(super) fn leaf(&self, relative: &str, access: Access) -> Result<Leaf, ServiceError> {
        let lexical = self.lexical(relative, access)?;
        let name = lexical
            .file_name()
            .ok_or_else(|| ServiceError::new("InvalidPath", "expected a named entry"))?;
        if relative.is_empty() {
            return Leaf::absolute(&lexical);
        }
        let parent = lexical.parent().expect("absolute named entry has a parent");
        let canonical = self.canonical_inside(parent, access)?;
        Leaf::new(&canonical, name)
    }

    pub(super) fn require_listable(&self) -> Result<(), ServiceError> {
        if self.root.is_entry_only() {
            return Err(entry_only_denied());
        }
        Ok(())
    }

    pub(super) fn check_leaf_kind(&self, kind: EntryKind) -> Result<(), ServiceError> {
        if self.root.is_entry_only() && !matches!(kind, EntryKind::File | EntryKind::Symlink) {
            return Err(entry_only_denied());
        }
        Ok(())
    }
}

fn authorize_mode(mode: FileMode, access: Access) -> Result<(), ServiceError> {
    if matches!(access, Access::Write) && !mode.can_write() {
        Err(denied())
    } else {
        Ok(())
    }
}

pub(super) fn denied() -> ServiceError {
    ServiceError::new("E_CAPABILITY", "system directory grant is read-only")
}

pub(super) fn entry_only_denied() -> ServiceError {
    ServiceError::new(
        "E_CAPABILITY",
        "external drop grants only the named entry, not its directory or symlink target",
    )
}

pub(super) fn path_error(error: io::Error) -> ServiceError {
    if error.raw_os_error() == Some(libc::ELOOP) {
        ServiceError::new("E_PATH_ESCAPE", "authorized path changed during resolution")
    } else {
        map_io_error(error)
    }
}

/// An entry beneath a pinned directory, with no caller-controlled ancestors left to traverse.
#[derive(Debug)]
pub(super) struct Leaf {
    pub(super) parent: File,
    pub(super) name: OsString,
}

impl Leaf {
    pub(super) fn new(parent: &Path, name: &OsStr) -> Result<Self, ServiceError> {
        Ok(Self {
            parent: pin(parent, libc::O_DIRECTORY | libc::O_PATH).map_err(path_error)?,
            name: name.into(),
        })
    }

    pub(super) fn absolute(path: &Path) -> Result<Self, ServiceError> {
        let parent = path
            .parent()
            .ok_or_else(|| ServiceError::new("InvalidPath", "expected a named entry"))?;
        Self::new(
            parent,
            path.file_name()
                .expect("absolute named entry has a filename"),
        )
    }

    pub(super) fn at(&self) -> PathBuf {
        fd_path(&self.parent).join(&self.name)
    }
}

pub(super) fn fd_path(fd: &File) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", fd.as_raw_fd()))
}

// Re-open canonical paths without following replacements of any ancestor.
// This also rejects magic links and fails closed on Linux without openat2.
pub(super) fn pin(path: &Path, flags: i32) -> io::Result<File> {
    let name = CString::new(path.as_os_str().as_bytes())?;
    // SAFETY: open_how contains only integer request fields; zero is valid for all of them.
    let mut how: libc::open_how = unsafe { std::mem::zeroed() };
    how.flags = (flags | libc::O_CLOEXEC) as u64;
    how.resolve = libc::RESOLVE_NO_SYMLINKS;
    // SAFETY: a NUL-terminated path and initialized open_how live through the syscall.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            libc::AT_FDCWD,
            name.as_ptr(),
            &how,
            size_of_val(&how),
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful openat2 transfers ownership of this descriptor.
    Ok(unsafe { File::from_raw_fd(fd as i32) })
}

// Backslash escaping keeps non-UTF-8 POSIX names addressable without aliasing a
// different file whose actual name happens to contain replacement characters.
pub(super) fn encode_name(name: &OsStr) -> String {
    let mut encoded = String::with_capacity(name.len());
    push_encoded_name(&mut encoded, name);
    encoded
}

fn push_encoded_name(encoded: &mut String, name: &OsStr) {
    if let Some(name) = name.to_str() {
        for (index, part) in name.split('\\').enumerate() {
            if index != 0 {
                encoded.push_str("\\\\");
            }
            encoded.push_str(part);
        }
        return;
    }
    use std::fmt::Write as _;
    for &byte in name.as_bytes() {
        if byte.is_ascii() && byte != b'\\' {
            encoded.push(char::from(byte));
        } else {
            write!(encoded, "\\x{byte:02x}").expect("writing to String cannot fail");
        }
    }
}

pub(super) fn decode_relative(relative: &str) -> Result<PathBuf, ServiceError> {
    let invalid = || ServiceError::new("InvalidPath", "invalid system directory relative path");
    let mut path = PathBuf::new();
    if relative.is_empty() {
        return Ok(path);
    }
    for part in relative.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains('\0') {
            return Err(invalid());
        }
        if !part.contains('\\') {
            path.push(part);
            continue;
        }
        let mut bytes = Vec::with_capacity(part.len());
        let mut raw = part.as_bytes().iter().copied();
        while let Some(byte) = raw.next() {
            bytes.push(if byte != b'\\' {
                byte
            } else {
                match raw.next() {
                    Some(b'\\') => b'\\',
                    Some(b'x') => {
                        let digit = |byte| {
                            char::from(byte)
                                .to_digit(16)
                                .map(|n| n as u8)
                                .ok_or_else(invalid)
                        };
                        (digit(raw.next().ok_or_else(invalid)?)? << 4)
                            | digit(raw.next().ok_or_else(invalid)?)?
                    }
                    _ => return Err(invalid()),
                }
            });
        }
        let part = OsString::from_vec(bytes);
        if part.as_bytes().contains(&0)
            || part.as_bytes().contains(&b'/')
            || !matches!(
                Path::new(&part).components().next(),
                Some(Component::Normal(_))
            )
        {
            return Err(invalid());
        }
        path.push(part);
    }
    Ok(path)
}
