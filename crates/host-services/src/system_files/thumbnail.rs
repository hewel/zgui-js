use serde::{Deserialize, Serialize};
use std::{
    fmt::Write as _,
    fs::{File, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

use super::{
    path::{Access, Scope, fd_path},
    trash::{encode_path, xdg_path},
};
use crate::{ServiceError, files::map_io_error};

const MAX_THUMBNAIL_BYTES: u64 = 8 * 1024 * 1024;

/// One of the Freedesktop cache size tiers.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThumbnailSize {
    #[default]
    Normal,
    Large,
    XLarge,
    XxLarge,
}
impl ThumbnailSize {
    pub(super) fn folder(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Large => "large",
            Self::XLarge => "x-large",
            Self::XxLarge => "xx-large",
        }
    }
}

/// Standard absolute XDG cache directory or HOME fallback.
pub fn cache_home() -> Option<PathBuf> {
    xdg_path("XDG_CACHE_HOME", ".cache")
}

/// Format an absolute local filename as a percent-encoded URI without UTF-8 loss.
pub fn file_uri(path: &Path) -> Result<String, ServiceError> {
    if !path.is_absolute() {
        return Err(ServiceError::new(
            "InvalidPath",
            "file URI requires an absolute path",
        ));
    }
    Ok(format!("file://{}", encode_path(path)))
}

/// Return the lowercase RFC 1321 digest of a canonical local file URI.
pub fn thumbnail_key(path: &Path) -> Result<String, ServiceError> {
    let mut key = String::with_capacity(32);
    for byte in md5(file_uri(path)?.as_bytes()) {
        write!(key, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(key)
}

/// Read a fresh cached PNG for the granted entry, without decoding pixels.
pub fn cached_thumbnail(
    scope: &Scope,
    relative: &str,
    size: ThumbnailSize,
    cache: &Path,
) -> Result<Option<Vec<u8>>, ServiceError> {
    let (path, source) = scope.follow_with(relative, Access::Read, libc::O_PATH)?;
    let meta = source.metadata().map_err(map_io_error)?;
    if !meta.is_file() {
        return Ok(None);
    }
    let path = cache
        .join("thumbnails")
        .join(size.folder())
        .join(format!("{}.png", thumbnail_key(&path)?));
    let load = || -> Option<Vec<u8>> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_NOFOLLOW)
            .open(path)
            .ok()?;
        let cached = file.metadata().ok()?;
        if !cached.is_file() || cached.len() > MAX_THUMBNAIL_BYTES {
            return None;
        }
        let mut png = Vec::with_capacity(cached.len() as usize);
        File::open(fd_path(&file))
            .ok()?
            .take(MAX_THUMBNAIL_BYTES + 1)
            .read_to_end(&mut png)
            .ok()?;
        if png.len() as u64 > MAX_THUMBNAIL_BYTES {
            return None;
        }
        let seconds = source.metadata().ok()?.mtime();
        (thumbnail_mtime(&png)? == seconds).then_some(png)
    };
    Ok(load())
}

/// Extract `Thumb::MTime` from well-formed PNG text without inflating pixel data.
pub fn thumbnail_mtime(png: &[u8]) -> Option<i64> {
    let mut png = png.strip_prefix(b"\x89PNG\r\n\x1a\n")?;
    if png.get(..8)? != b"\0\0\0\x0dIHDR" {
        return None;
    }
    let mut mtime = None;
    let mut saw_pixels = false;
    while let Some(header) = png.get(..8) {
        let size = u32::from_be_bytes(header[..4].try_into().ok()?) as usize;
        let chunk = png.get(8..8 + size)?;
        let crc = png.get(8 + size..12 + size)?;
        png = &png[12 + size..];
        match &header[4..] {
            b"IEND" => {
                return (size == 0 && saw_pixels && png.is_empty())
                    .then_some(mtime)
                    .flatten();
            }
            b"IDAT" => saw_pixels = true,
            typ @ (b"tEXt" | b"zTXt") => {
                let Some(value) = chunk.strip_prefix(b"Thumb::MTime\0") else {
                    continue;
                };
                let mut expected = flate2::Crc::new();
                expected.update(typ);
                expected.update(chunk);
                if expected.sum().to_be_bytes() != crc {
                    return None;
                }
                let compressed = typ == b"zTXt";
                let mut out = Vec::new();
                let bytes = if compressed {
                    let value = value.strip_prefix(&[0])?;
                    flate2::read::ZlibDecoder::new(value)
                        .take(32)
                        .read_to_end(&mut out)
                        .ok()?;
                    &out[..]
                } else {
                    value
                };
                if mtime.is_some() || bytes.len() > 20 {
                    return None;
                }
                mtime = Some(std::str::from_utf8(bytes).ok()?.parse().ok()?);
            }
            _ => {}
        }
    }
    None
}

// RFC 1321 §3.4: shifts and the integer floor of 2^32 × abs(sin(i + 1)).
fn md5(bytes: &[u8]) -> [u8; 16] {
    const S: [[u32; 4]; 4] = [
        [7, 12, 17, 22],
        [5, 9, 14, 20],
        [4, 11, 16, 23],
        [6, 10, 15, 21],
    ];
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];
    let mut state = [0x67452301_u32, 0xefcdab89, 0x98badcfe, 0x10325476];
    let mut compress = |block: &[u8]| {
        let words: [u32; 16] = std::array::from_fn(|i| {
            u32::from_le_bytes(block[i * 4..][..4].try_into().expect("fixed block"))
        });
        let [mut a, mut b, mut c, mut d] = state;
        for (i, k) in K.into_iter().enumerate() {
            let (f, w) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let next = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(k)
                    .wrapping_add(words[w])
                    .rotate_left(S[i / 16][i % 4]),
            );
            (a, b, c, d) = (d, next, b, c);
        }
        for (word, round) in state.iter_mut().zip([a, b, c, d]) {
            *word = word.wrapping_add(round);
        }
    };
    let (chunks, tail) = bytes.as_chunks::<64>();
    for block in chunks {
        compress(block);
    }
    let mut padding = [0; 128];
    padding[..tail.len()].copy_from_slice(tail);
    padding[tail.len()] = 0x80;
    let end = if tail.len() < 56 { 64 } else { 128 };
    padding[end - 8..end].copy_from_slice(&(bytes.len() as u64 * 8).to_le_bytes());
    for block in padding[..end].as_chunks::<64>().0 {
        compress(block);
    }
    let mut result = [0; 16];
    for (bytes, word) in result.as_chunks_mut::<4>().0.iter_mut().zip(state) {
        *bytes = word.to_le_bytes();
    }
    result
}
