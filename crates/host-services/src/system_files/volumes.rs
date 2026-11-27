use crate::{ServiceError, files::map_io_error};
use serde::{Deserialize, Serialize};
use std::{ffi::OsString, fs, io, os::unix::ffi::OsStringExt, path::PathBuf};

/// A mounted local, removable, or user-space filesystem.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Volume {
    pub mount_point: String,
    pub source: String,
    pub file_system: String,
    pub read_only: bool,
    pub removable: bool,
}

pub(super) struct Mount {
    pub(super) point: PathBuf,
    source: OsString,
    fs_type: String,
    read_only: bool,
}

pub(super) fn mounts() -> io::Result<Vec<Mount>> {
    Ok(fs::read("/proc/self/mountinfo")?
        .split(|&byte| byte == b'\n')
        .filter_map(parse_mount)
        .collect())
}

fn unescape(value: &[u8]) -> OsString {
    let mut out = Vec::with_capacity(value.len());
    let mut pos = 0;
    while pos < value.len() {
        if value[pos] == b'\\'
            && let Some(oct) = value.get(pos + 1..pos + 4)
            && matches!(oct[0], b'0'..=b'3')
            && oct[1..].iter().all(|c| matches!(c, b'0'..=b'7'))
        {
            out.push((oct[0] - b'0') * 64 + (oct[1] - b'0') * 8 + oct[2] - b'0');
            pos += 4;
        } else {
            out.push(value[pos]);
            pos += 1;
        }
    }
    OsString::from_vec(out)
}

fn parse_mount(line: &[u8]) -> Option<Mount> {
    let mut fields = line.split(|&c| c == b' ');
    let point = PathBuf::from(unescape(fields.nth(4)?));
    let read_only = fields.next()?.split(|&c| c == b',').any(|opt| opt == b"ro");
    fields.find(|&field| field == b"-")?;
    let fs_type = std::str::from_utf8(fields.next()?).ok()?.to_owned();
    let source = unescape(fields.next()?);
    Some(Mount {
        point,
        source,
        fs_type,
        read_only,
    })
}

/// Read the Linux mount namespace, excluding pseudo filesystems and kernel FUSE control.
pub fn list_volumes() -> Result<Vec<Volume>, ServiceError> {
    let mut volumes: Vec<_> = mounts()
        .map_err(map_io_error)?
        .into_iter()
        .filter_map(|mount| {
            let removable =
                mount.point.starts_with("/run/media") || mount.point.starts_with("/media");
            let fuse = mount.fs_type.starts_with("fuse.")
                || matches!(
                    mount.fs_type.as_str(),
                    "fuse" | "fuseblk" | "sshfs" | "rclone" | "gvfsd-fuse"
                );
            let block = mount.source.as_encoded_bytes().starts_with(b"/dev/");
            let network = matches!(
                mount.fs_type.as_str(),
                "nfs" | "nfs4" | "cifs" | "smb3" | "9p" | "ceph"
            );
            let visible = block
                || fuse
                || network
                || mount.point == std::path::Path::new("/")
                || removable
                    && !matches!(
                        mount.fs_type.as_str(),
                        "autofs" | "proc" | "sysfs" | "devtmpfs"
                    );
            visible.then(|| Volume {
                mount_point: mount.point.to_string_lossy().into_owned(),
                source: mount.source.to_string_lossy().into_owned(),
                file_system: mount.fs_type,
                read_only: mount.read_only,
                removable,
            })
        })
        .collect();
    volumes.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
    // mountinfo orders stacked mounts with the most recent last.
    volumes.reverse();
    volumes.dedup_by(|a, b| a.mount_point == b.mount_point);
    volumes.reverse();
    Ok(volumes)
}
