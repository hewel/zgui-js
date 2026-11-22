//! Bounded regular-file reading for contained artifact trees and CLI inputs.

use std::{
    error::Error,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

#[derive(Debug)]
pub(crate) enum BoundedReadError {
    InvalidPath,
    Symlink,
    NotRegularFile,
    TooLarge { limit: u64 },
    Io(io::Error),
}

impl fmt::Display for BoundedReadError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath => out.write_str("path must be canonical and relative"),
            Self::Symlink => out.write_str("symbolic links are rejected"),
            Self::NotRegularFile => out.write_str("path must refer to a regular file"),
            Self::TooLarge { limit } => write!(out, "file exceeds {limit} byte limit"),
            Self::Io(error) => error.fmt(out),
        }
    }
}

impl Error for BoundedReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for BoundedReadError {
    fn from(error: io::Error) -> Self {
        #[cfg(unix)]
        if error.raw_os_error() == Some(libc::ELOOP) {
            return Self::Symlink;
        }
        Self::Io(error)
    }
}

/// Validates that `name` is a canonical, non-empty relative path without escapes.
pub(crate) fn relative_path(name: &str) -> Result<&Path, BoundedReadError> {
    let path = Path::new(name);
    if name.is_empty()
        || name.contains('\\')
        || name.contains('\0')
        || name.split('/').any(|part| part.is_empty() || part == "." || part == "..")
        || path.components().any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(BoundedReadError::InvalidPath);
    }
    Ok(path)
}

/// Opens a regular file inside `root`, rejecting symlinks along every relative component.
pub(crate) fn open_contained(root: &Path, relative: &str, limit: u64) -> Result<(File, u64), BoundedReadError> {
    let relative = relative_path(relative)?;
    let mut resolved = PathBuf::from(root);
    let mut leaf = None;
    for component in relative.components() {
        resolved.push(component);
        let metadata = fs::symlink_metadata(&resolved)?;
        if metadata.file_type().is_symlink() {
            return Err(BoundedReadError::Symlink);
        }
        leaf = Some(metadata);
    }
    let metadata = leaf.ok_or(BoundedReadError::InvalidPath)?;
    if !metadata.is_file() {
        return Err(BoundedReadError::NotRegularFile);
    }
    if metadata.len() > limit {
        return Err(BoundedReadError::TooLarge { limit });
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    validate_open_file(options.open(&resolved)?, limit)
}

/// Reads a contained regular file whose length never exceeds `limit` bytes.
pub(crate) fn read_contained(root: &Path, relative: &str, limit: u64) -> Result<Vec<u8>, BoundedReadError> {
    let (file, declared_len) = open_contained(root, relative, limit)?;
    read_bounded_reader(file, (declared_len.min(limit)) as usize, limit)
}

/// Reads an explicit host/CLI input file, permitting symlinks while rejecting special or oversized targets.
pub(crate) fn read_input(path: &Path, limit: u64) -> Result<Vec<u8>, BoundedReadError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let (file, declared_len) = validate_open_file(options.open(path)?, limit)?;
    read_bounded_reader(file, (declared_len.min(limit)) as usize, limit)
}

fn validate_open_file(file: File, limit: u64) -> Result<(File, u64), BoundedReadError> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(BoundedReadError::NotRegularFile);
    }
    let len = metadata.len();
    if len > limit {
        return Err(BoundedReadError::TooLarge { limit });
    }
    Ok((file, len))
}

fn read_bounded_reader(mut reader: impl Read, capacity: usize, limit: u64) -> Result<Vec<u8>, BoundedReadError> {
    let mut bytes = Vec::with_capacity(capacity);
    reader.by_ref().take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(BoundedReadError::TooLarge { limit });
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "zgui-bounded-fs-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn contained_reads_reject_traversal_symlinks_non_regular_files_and_oversized_payloads() {
        let dir = TempDir::new();
        let nested = dir.0.join("sub");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("ok.txt"), b"hello").unwrap();
        fs::write(dir.0.join("big.txt"), b"0123456789").unwrap();

        assert_eq!(read_contained(&dir.0, "sub/ok.txt", 16).unwrap(), b"hello");

        for bad in ["", "../escape", "/abs", "./rel", "sub//ok.txt", "sub\\ok.txt", "a\0b"] {
            assert!(matches!(read_contained(&dir.0, bad, 16), Err(BoundedReadError::InvalidPath)), "{bad:?}");
        }

        assert!(matches!(read_contained(&dir.0, "sub", 16), Err(BoundedReadError::NotRegularFile)));
        assert!(matches!(read_contained(&dir.0, "big.txt", 5), Err(BoundedReadError::TooLarge { limit: 5 })));

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(nested.join("ok.txt"), dir.0.join("leaf-link.txt")).unwrap();
            assert!(matches!(read_contained(&dir.0, "leaf-link.txt", 16), Err(BoundedReadError::Symlink)));

            std::os::unix::fs::symlink(&nested, dir.0.join("dir-link")).unwrap();
            assert!(matches!(read_contained(&dir.0, "dir-link/ok.txt", 16), Err(BoundedReadError::Symlink)));
        }
    }

    #[test]
    fn input_reads_allow_symlinks_while_rejecting_directories_fifos_and_growth() {
        let dir = TempDir::new();
        let target = dir.0.join("real.json");
        fs::write(&target, b"{}").unwrap();
        assert!(matches!(read_input(&dir.0, 64), Err(BoundedReadError::NotRegularFile)));

        #[cfg(unix)]
        {
            let symlink = dir.0.join("alias.json");
            std::os::unix::fs::symlink(&target, &symlink).unwrap();
            assert_eq!(read_input(&symlink, 64).unwrap(), b"{}");

            let fifo = dir.0.join("pipe");
            let c_fifo = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
            // SAFETY: `c_fifo` is a valid NUL-terminated path inside an exclusively owned temporary directory.
            let status = unsafe { libc::mkfifo(c_fifo.as_ptr(), 0o600) };
            assert_eq!(status, 0);
            assert!(matches!(read_input(&fifo, 64), Err(BoundedReadError::NotRegularFile)));
            assert!(matches!(read_contained(&dir.0, "pipe", 64), Err(BoundedReadError::NotRegularFile)));
        }

        assert!(matches!(read_input(&target, 1), Err(BoundedReadError::TooLarge { limit: 1 })));
        let grew = read_bounded_reader(io::Cursor::new(b"123456"), 4, 4);
        assert!(matches!(grew, Err(BoundedReadError::TooLarge { limit: 4 })));
    }
}
