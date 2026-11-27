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

use crate::{
    ServiceError,
    files::{FileMode, map_io_error},
};

/// An explicitly authorized directory and its access mode.
#[derive(Clone, Debug)]
pub struct Root {
    path: Arc<Path>,
    mode: FileMode,
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
        Ok(Self {
            path: path.into(),
            mode,
        })
    }

    /// Construct authority for a canonical path verified by a virtual filesystem.
    pub(super) fn virtual_root(path: PathBuf, mode: FileMode) -> Self {
        Self {
            path: path.into(),
            mode,
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
        Ok(self.root.path.join(decode_relative(relative)?))
    }

    pub(super) fn authorize(&self, path: &Path, access: Access) -> Result<(), ServiceError> {
        let mut covered = false;
        for root in std::iter::once(&self.root).chain(self.other.iter()) {
            if path.starts_with(&root.path) {
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
    if let Some(name) = name.to_str() {
        return name.replace('\\', "\\\\");
    }
    use std::fmt::Write as _;
    let mut encoded = String::new();
    for &byte in name.as_bytes() {
        if byte.is_ascii() && byte != b'\\' {
            encoded.push(char::from(byte));
        } else {
            write!(encoded, "\\x{byte:02x}").expect("writing to String cannot fail");
        }
    }
    encoded
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
