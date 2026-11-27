use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    fs::{self, File, Metadata, OpenOptions},
    hash::{DefaultHasher, Hash, Hasher},
    io::{self, Read},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

use super::path::{Access, Scope, encode_name, fd_path};
use crate::{
    ServiceError,
    files::map_io_error,
    locale::{CollationSensitivity, LocaleContext},
};

/// POSIX entry type, reported without dereferencing the final symlink.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Dir,
    File,
    Symlink,
    Other,
}

impl EntryKind {
    pub(super) fn of(meta: &Metadata) -> Self {
        if meta.is_dir() {
            Self::Dir
        } else if meta.is_file() {
            Self::File
        } else if meta.is_symlink() {
            Self::Symlink
        } else {
            Self::Other
        }
    }
}

/// lstat metadata and informational symlink target status.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryInfo {
    pub name: String,
    pub relative_path: String,
    pub kind: EntryKind,
    pub size: u64,
    pub modified_ms: Option<i64>,
    pub mode: u32,
    pub is_hidden: bool,
    pub symlink_target: Option<String>,
    pub symlink_broken: bool,
    pub symlink_target_kind: Option<EntryKind>,
    pub mime_type: &'static str,
}

/// The primary directory sort key. Every sort uses the filename as a tie-breaker.
#[derive(Clone, Copy, Debug, Default, Deserialize, Hash)]
#[serde(rename_all = "camelCase")]
pub enum SortBy {
    #[default]
    Name,
    Size,
    ModifiedMs,
    Kind,
}

/// Sort direction, independent of directory grouping.
#[derive(Clone, Copy, Debug, Default, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum SortDirection {
    #[default]
    Asc,
    Desc,
}

/// Pagination, filtering, and collation for one directory query.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadOptions {
    pub cursor: Option<String>,
    pub page_size: usize,
    pub include_hidden: bool,
    pub directories_first: bool,
    pub sort_by: SortBy,
    pub sort_direction: SortDirection,
    pub numeric_collation: bool,
    pub locale: Option<String>,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            cursor: None,
            page_size: 2000,
            include_hidden: false,
            directories_first: false,
            sort_by: SortBy::Name,
            sort_direction: SortDirection::Asc,
            numeric_collation: true,
            locale: None,
        }
    }
}

/// Bounded page from a sorted directory query.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirPage {
    pub entries: Vec<EntryInfo>,
    pub next_cursor: Option<String>,
    pub total_entries: usize,
}

// Sorted rows initially carry encoded basenames. Prefix only the returned page.
pub(super) fn paginate(
    mut entries: Vec<EntryInfo>,
    directory: &Path,
    relative: &str,
    options: &ReadOptions,
    locale: &LocaleContext,
) -> Result<DirPage, ServiceError> {
    let mut key = DefaultHasher::new();
    (
        directory,
        relative,
        options.include_hidden,
        options.directories_first,
        options.sort_by,
        options.sort_direction,
        options.numeric_collation,
        &options.locale,
    )
        .hash(&mut key);
    let key = key.finish();
    let cursor_error =
        || ServiceError::new("InvalidPath", "cursor does not match this directory query");
    let offset = match &options.cursor {
        None => 0,
        Some(cursor) => {
            let (tag, offset) = cursor.split_once(':').ok_or_else(cursor_error)?;
            if u64::from_str_radix(tag, 16).ok() != Some(key) {
                return Err(cursor_error());
            }
            offset.parse::<usize>().map_err(|_| cursor_error())?
        }
    };
    if options.page_size == 0 || options.page_size > 5000 {
        return Err(ServiceError::new(
            "InvalidPath",
            "pageSize must be between 1 and 5000",
        ));
    }
    if !options.include_hidden {
        entries.retain(|info| !info.is_hidden);
    }
    let collator = locale
        .collator(
            options.locale.as_deref(),
            Some(options.numeric_collation),
            CollationSensitivity::Variant,
        )
        .map_err(|error| ServiceError::new("InvalidPath", error.to_string()))?;
    entries.sort_unstable_by(|a, b| {
        if options.directories_first {
            let group = (b.kind == EntryKind::Dir).cmp(&(a.kind == EntryKind::Dir));
            if group != Ordering::Equal {
                return group;
            }
        }
        let primary = match options.sort_by {
            SortBy::Name => Ordering::Equal,
            SortBy::Size => a.size.cmp(&b.size),
            SortBy::ModifiedMs => a.modified_ms.cmp(&b.modified_ms),
            SortBy::Kind => a.kind.cmp(&b.kind),
        };
        let order = primary
            .then_with(|| collator.compare(&a.name, &b.name))
            .then_with(|| a.relative_path.cmp(&b.relative_path));
        match options.sort_direction {
            SortDirection::Asc => order,
            SortDirection::Desc => order.reverse(),
        }
    });
    let total_entries = entries.len();
    // Long paths or link targets can make even 5,000 entries too large for one
    // bridge turn. Budget worst-case JSON escaping without serializing twice.
    const PAGE_BYTE_BUDGET: usize = 4 * 1024 * 1024;
    let mut page = Vec::with_capacity(options.page_size.min(total_entries.saturating_sub(offset)));
    let mut budget = PAGE_BYTE_BUDGET;
    for mut entry in entries.into_iter().skip(offset).take(options.page_size) {
        let strings = entry.name.len()
            + entry.relative_path.len()
            + relative.len()
            + entry.symlink_target.as_ref().map_or(0, String::len);
        let bytes = 320usize.saturating_add(strings.saturating_mul(6));
        if bytes > budget && !page.is_empty() {
            break;
        }
        budget = budget.saturating_sub(bytes);
        if !relative.is_empty() {
            entry.relative_path = format!("{relative}/{}", entry.relative_path);
        }
        page.push(entry);
    }
    let end = offset.saturating_add(page.len());
    Ok(DirPage {
        entries: page,
        next_cursor: (end < total_entries).then(|| format!("{key:016x}:{end}")),
        total_entries,
    })
}

impl Scope {
    /// Inspect a final symlink as an entry, even if its target is dangling or ungranted.
    pub fn lstat(&self, relative: &str) -> Result<EntryInfo, ServiceError> {
        if relative.is_empty() {
            let (canonical, file) = self.follow_with(relative, Access::Read, libc::O_PATH)?;
            let name = canonical
                .file_name()
                .unwrap_or(canonical.as_os_str())
                .to_string_lossy()
                .into_owned();
            let meta = file.metadata().map_err(map_io_error)?;
            return Ok(info(name, String::new(), &meta));
        }
        let leaf = self.leaf(relative, Access::Read)?;
        let name = leaf.name.to_string_lossy().into_owned();
        let at = leaf.at();
        let mut info = inspect(&at, name, relative.to_owned())?;
        self.check_leaf_kind(info.kind)?;
        if info.kind == EntryKind::File && info.mime_type == "application/octet-stream" {
            info.mime_type = sniff(&at);
        }
        Ok(info)
    }

    /// Sort a directory on the caller's thread and return only the requested page.
    pub fn read_dir(
        &self,
        relative: &str,
        options: &ReadOptions,
        locale: &LocaleContext,
    ) -> Result<DirPage, ServiceError> {
        self.require_listable()?;
        let (path, dir) =
            self.follow_with(relative, Access::Read, libc::O_DIRECTORY | libc::O_PATH)?;
        let at = fd_path(&dir);
        let mut entries = Vec::new();
        for child in fs::read_dir(&at).map_err(map_io_error)? {
            let child = child.map_err(map_io_error)?;
            let file_name = child.file_name();
            let name = file_name.to_string_lossy().into_owned();
            if !options.include_hidden && name.starts_with('.') {
                continue;
            }
            let encoded = encode_name(&file_name);
            match inspect(&at.join(file_name), name, encoded) {
                Ok(info) => entries.push(info),
                Err(error) if error.code == "NotFound" => {} // Concurrent directory edit.
                Err(error) => return Err(error),
            }
        }
        paginate(entries, &path, relative, options, locale)
    }
}

fn info(name: String, relative_path: String, meta: &Metadata) -> EntryInfo {
    let kind = EntryKind::of(meta);
    EntryInfo {
        is_hidden: name.starts_with('.'),
        mime_type: mime(&name, kind),
        name,
        relative_path,
        kind,
        size: meta.len(),
        modified_ms: meta
            .mtime()
            .checked_mul(1000)
            .and_then(|ms| ms.checked_add(meta.mtime_nsec() / 1_000_000)),
        mode: meta.mode() & 0o7777,
        symlink_target: None,
        symlink_broken: false,
        symlink_target_kind: None,
    }
}

fn inspect(at: &Path, name: String, relative: String) -> Result<EntryInfo, ServiceError> {
    let meta = fs::symlink_metadata(at).map_err(map_io_error)?;
    let mut info = info(name, relative, &meta);
    if info.kind == EntryKind::Symlink {
        info.symlink_target = Some(
            fs::read_link(at)
                .map_err(map_io_error)?
                .to_string_lossy()
                .into_owned(),
        );
        match fs::metadata(at) {
            Ok(target) => info.symlink_target_kind = Some(EntryKind::of(&target)),
            Err(error) => {
                info.symlink_broken = matches!(
                    error.raw_os_error(),
                    Some(libc::ENOENT | libc::ELOOP | libc::ENOTDIR)
                )
            }
        }
    }
    Ok(info)
}

/// Fast MIME classification shared by listings and drag offers.
pub fn mime(name: &str, kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Dir => return "inode/directory",
        EntryKind::Symlink => return "inode/symlink",
        EntryKind::File | EntryKind::Other => {}
    }
    let ext = name.rsplit_once('.').map_or("", |(_, ext)| ext);
    [
        (&["txt", "log", "md"][..], "text/plain"),
        (&["png"], "image/png"),
        (&["jpg", "jpeg"], "image/jpeg"),
        (&["svg"], "image/svg+xml"),
        (&["gif"], "image/gif"),
        (&["webp"], "image/webp"),
        (&["pdf"], "application/pdf"),
        (&["json"], "application/json"),
        (&["zip"], "application/zip"),
        (&["mp4"], "video/mp4"),
        (&["mp3"], "audio/mpeg"),
    ]
    .into_iter()
    .find(|(extensions, _)| extensions.iter().any(|v| v.eq_ignore_ascii_case(ext)))
    .map_or("application/octet-stream", |(_, mime)| mime)
}

fn sniff(path: &Path) -> &'static str {
    let read = || -> io::Result<[u8; 12]> {
        let entry = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_NOFOLLOW)
            .open(path)?;
        if !entry.metadata()?.is_file() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let mut head = [0; 12];
        let mut data = File::open(fd_path(&entry))?.take(12);
        let mut offset = 0;
        while offset < head.len() {
            let count = data.read(&mut head[offset..])?;
            if count == 0 {
                break;
            }
            offset += count;
        }
        Ok(head)
    };
    match read().as_ref().map(|b| &b[..]) {
        Ok([137, b'P', b'N', b'G', 13, 10, 26, 10, ..]) => "image/png",
        Ok([0xff, 0xd8, 0xff, ..]) => "image/jpeg",
        Ok([b'%', b'P', b'D', b'F', b'-', ..]) => "application/pdf",
        Ok([b'P', b'K', 3, 4, ..]) => "application/zip",
        _ => "application/octet-stream",
    }
}
