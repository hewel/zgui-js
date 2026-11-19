use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub(crate) enum FileAccess {
    #[default]
    Denied,
    Live(Arc<[String]>),
    Fixture(Arc<BTreeMap<String, Arc<[u8]>>>),
}

impl FileAccess {
    pub(crate) fn live(prefixes: &[String]) -> Self {
        Self::Live(
            prefixes
                .iter()
                .filter(|prefix| canonical_path(prefix))
                .cloned()
                .collect(),
        )
    }

    pub(crate) fn fixtures(
        prefixes: &[String],
        files: impl IntoIterator<Item = (String, Arc<[u8]>)>,
    ) -> Self {
        let prefixes: Vec<_> = prefixes
            .iter()
            .filter(|prefix| canonical_path(prefix))
            .collect();
        let files = files.into_iter().filter(|(uri, _)| {
            file_path(uri).is_some_and(|path| prefixes.iter().any(|prefix| beneath(&path, prefix)))
        });
        Self::Fixture(Arc::new(files.collect()))
    }

    pub(crate) fn permits(&self, uri: &str) -> bool {
        match self {
            Self::Denied => false,
            Self::Live(prefixes) => file_path(uri)
                .is_some_and(|path| prefixes.iter().any(|prefix| beneath(&path, prefix))),
            Self::Fixture(files) => files.contains_key(uri),
        }
    }

    pub(crate) fn fixture(&self, uri: &str) -> Option<Arc<[u8]>> {
        match self {
            Self::Fixture(files) => files.get(uri).cloned(),
            _ => None,
        }
    }

    /// The lexical grant is resolved again here, on the worker that opens the file.
    pub(crate) fn read_live(&self, uri: &str) -> io::Result<Vec<u8>> {
        let Self::Live(prefixes) = self else {
            return Err(denied(uri));
        };
        let path = file_path(uri).ok_or_else(|| denied(uri))?;
        // The broadest matching grant already confers access to all matching narrower grants.
        let prefix = prefixes
            .iter()
            .filter(|prefix| beneath(&path, prefix))
            .min_by_key(|prefix| prefix.len())
            .ok_or_else(|| denied(uri))?;
        let relative = path[prefix.len()..].trim_start_matches('/');
        read_beneath(prefix, relative)
    }
}

pub(crate) fn denied(uri: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("image source is not an authorized file URI: {uri}"),
    )
}

fn canonical_path(path: &str) -> bool {
    path.strip_prefix('/').is_some_and(|relative| {
        relative.is_empty()
            || (!relative.contains('\\')
                && !relative.chars().any(char::is_control)
                && relative
                    .split('/')
                    .all(|part| !part.is_empty() && part != "." && part != ".."))
    })
}

fn beneath(path: &str, prefix: &str) -> bool {
    if prefix == "/" {
        return path != "/";
    }
    path.strip_prefix(prefix)
        .is_some_and(|tail| tail.starts_with('/'))
}

fn file_path(uri: &str) -> Option<Cow<'_, str>> {
    let path = uri.strip_prefix("file://")?;
    if !path.starts_with('/')
        || !path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/-._~!$&'()*+,;=:@%".contains(&byte))
    {
        return None;
    }
    let path = if path.contains('%') {
        let mut decoded = Vec::with_capacity(path.len());
        let mut bytes = path.bytes();
        while let Some(byte) = bytes.next() {
            if byte != b'%' {
                decoded.push(byte);
                continue;
            }
            let hi = char::from(bytes.next()?).to_digit(16)?;
            let lo = char::from(bytes.next()?).to_digit(16)?;
            let byte = (hi * 16 + lo) as u8;
            if byte == b'/' {
                return None;
            }
            decoded.push(byte);
        }
        Cow::Owned(String::from_utf8(decoded).ok()?)
    } else {
        Cow::Borrowed(path)
    };
    canonical_path(&path).then_some(path)
}

#[cfg(target_os = "linux")]
fn read_beneath(prefix: &str, relative: &str) -> io::Result<Vec<u8>> {
    use std::{fs::File, io::Read, os::fd::AsRawFd};

    use rustix::fs::{self, FileType, Mode, OFlags, ResolveFlags};

    const MAX_ENCODED_BYTES: u64 = 64 * 1024 * 1024;
    let dir_flags = OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC;
    let root = fs::open("/", dir_flags, Mode::empty())?;
    let resolve = ResolveFlags::BENEATH | ResolveFlags::NO_MAGICLINKS;
    let dir = if prefix == "/" {
        root
    } else {
        fs::openat2(
            root,
            &prefix[1..],
            dir_flags,
            Mode::empty(),
            resolve | ResolveFlags::NO_SYMLINKS,
        )?
    };
    let fd = fs::openat2(
        dir,
        relative,
        OFlags::PATH | OFlags::CLOEXEC,
        Mode::empty(),
        resolve,
    )?;
    let stat = fs::fstat(&fd)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "image source is not a regular file",
        ));
    }
    let too_large = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "image file exceeds the 64 MiB encoded size limit",
        )
    };
    let size = u64::try_from(stat.st_size).map_err(|_| too_large())?;
    if size > MAX_ENCODED_BYTES {
        return Err(too_large());
    }

    // Use the already-confined inode. Reopening its original spelling would race path replacement;
    // opening for reading before fstat could block on a FIFO or trigger device side effects.
    let file = File::open(format!("/proc/self/fd/{}", fd.as_raw_fd()))?;
    let mut bytes = Vec::with_capacity(size as usize + 1);
    file.take(MAX_ENCODED_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_ENCODED_BYTES {
        return Err(too_large());
    }
    Ok(bytes)
}

#[cfg(not(target_os = "linux"))]
fn read_beneath(_prefix: &str, _relative: &str) -> io::Result<Vec<u8>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "live file image authority requires Linux",
    ))
}
