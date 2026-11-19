//! Lexical capability paths and host directory checks shared with the control CLI.
use std::{
    fs::{self, File, OpenOptions},
    io,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

/// The only roots a portable capability may expand.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Root {
    /// The effective user's private runtime directory.
    Runtime,
    /// The user's home directory.
    Home,
    /// The user's XDG data directory.
    Data,
}

impl Root {
    /// The fixed environment key for this root.
    #[must_use]
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Runtime => "XDG_RUNTIME_DIR",
            Self::Home => "HOME",
            Self::Data => "XDG_DATA_HOME",
        }
    }
}

/// The display spellings permitted in a template segment.
pub(crate) const DISPLAY: &str = "$WAYLAND_DISPLAY";
/// The display spelling that can be embedded in a filename.
pub(crate) const EMBEDDED_DISPLAY: &str = "${WAYLAND_DISPLAY}";

/// Maximum byte length of a named socket identifier.
pub(crate) const MAX_SOCKET_NAME_BYTES: usize = 64;

/// Builds the XDG data default from a validated home path.
#[must_use]
pub(crate) fn default_data_home(home: &str) -> String {
    format!("{}/.local/share", home.trim_end_matches('/'))
}

/// Recognizes canonical literal absolute paths without touching the filesystem.
#[must_use]
pub(crate) fn canonical_absolute(path: &str) -> bool {
    path == "/"
        || path
            .strip_prefix('/')
            .is_some_and(|tail| tail.split('/').all(safe_segment))
}

/// A single non-traversing filename component.
#[must_use]
pub(crate) fn safe_segment(part: &str) -> bool {
    !matches!(part, "" | "." | "..")
        && !part.contains(['/', '\\'])
        && !part.chars().any(char::is_control)
}

/// Parses a template without resolving any host values.
#[must_use]
pub(crate) fn template(path: &str) -> Option<(Root, &str)> {
    let (root, tail) = match path.split_once('/') {
        Some((root, tail)) => (root, Some(tail)),
        None => (path, None),
    };
    let root = match root {
        "$XDG_RUNTIME_DIR" => Root::Runtime,
        "$HOME" => Root::Home,
        "$XDG_DATA_HOME" => Root::Data,
        _ => return None,
    };
    if tail.is_some_and(|tail| {
        !tail.split('/').all(|part| {
            safe_segment(part)
                && (part == DISPLAY
                    || part
                        .split(EMBEDDED_DISPLAY)
                        .all(|literal| !literal.contains('$')))
        })
    }) {
        return None;
    }
    Some((root, tail.unwrap_or_default()))
}

/// Checks the closed grammar shared by manifest names and runtime bindings.
#[must_use]
pub(crate) fn socket_name(name: &str) -> bool {
    name.len() <= MAX_SOCKET_NAME_BYTES
        && name.as_bytes().split_first().is_some_and(|(first, tail)| {
            first.is_ascii_alphabetic()
                && tail
                    .iter()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        })
}

/// Opens an existing canonical directory without following a final symlink.
///
/// # Errors
/// Fails for noncanonical paths, symlinks, or inaccessible directories.
pub(crate) fn open_canonical_dir(path: &Path) -> io::Result<File> {
    if fs::canonicalize(path)?.as_os_str() != path.as_os_str() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory must be canonical and not contain symlinks",
        ));
    }
    open_directory(path)
}

/// Opens an existing directory without following a final symlink.
///
/// # Errors
/// Fails if the path cannot be opened as a directory.
pub(crate) fn open_directory(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
}

/// Checks ownership and compares the selected mode bits with `0700`.
///
/// # Errors
/// Fails if metadata cannot be read or ownership or mode differs.
pub(crate) fn require_private(directory: &File, mode_mask: u32) -> io::Result<()> {
    let metadata = directory.metadata()?;
    if metadata.uid() != effective_uid() || metadata.mode() & mode_mask != 0o700 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "runtime directory must be owned by the effective user with mode 0700",
        ));
    }
    Ok(())
}

/// Returns the process's effective user ID.
#[must_use]
pub(crate) fn effective_uid() -> u32 {
    // SAFETY: geteuid takes no arguments and has no preconditions.
    unsafe { libc::geteuid() }
}
