use crate::{ServiceError, files::map_io_error};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    ffi::OsString,
    fs, io,
    os::unix::{
        ffi::OsStringExt,
        fs::{FileTypeExt, MetadataExt},
    },
    path::{Path, PathBuf},
    time::Duration,
};
use zbus::{Connection, fdo::ManagedObjects, zvariant::ObjectPath};

const UDISKS_TIMEOUT: Duration = Duration::from_millis(50);
const UDISKS_NAME: &str = "org.freedesktop.UDisks2";
const UDISKS_DRIVE: &str = "org.freedesktop.UDisks2.Drive";
const UDISKS_BLOCK: &str = "org.freedesktop.UDisks2.Block";

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
    device: (u32, u32),
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
    let (major, minor) = std::str::from_utf8(fields.nth(2)?).ok()?.split_once(':')?;
    let device = (major.parse().ok()?, minor.parse().ok()?);
    let point = PathBuf::from(unescape(fields.nth(1)?));
    let read_only = fields.next()?.split(|&c| c == b',').any(|opt| opt == b"ro");
    fields.find(|&field| field == b"-")?;
    let fs_type = std::str::from_utf8(fields.next()?).ok()?.to_owned();
    let source = unescape(fields.next()?);
    Some(Mount {
        point,
        device,
        source,
        fs_type,
        read_only,
    })
}

fn sysfs_removable(block: &Path) -> bool {
    let Ok(block) = block.canonicalize() else {
        return false;
    };
    let device = if block.join("partition").exists() {
        block.parent().unwrap_or(&block)
    } else {
        &block
    };
    fs::read_to_string(device.join("removable")).is_ok_and(|value| value.trim() == "1")
}

fn removable_devices() -> Option<HashSet<u64>> {
    // Include connection setup and the single inventory call in the same deadline.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .ok()?;
    let objects = runtime.block_on(async {
        tokio::time::timeout(UDISKS_TIMEOUT, async {
            let bus = Connection::system().await?;
            let reply = bus
                .call_method(
                    Some(UDISKS_NAME),
                    "/org/freedesktop/UDisks2",
                    Some("org.freedesktop.DBus.ObjectManager"),
                    "GetManagedObjects",
                    &(),
                )
                .await?;
            reply.body().deserialize::<ManagedObjects>()
        })
        .await
        .ok()?
        .ok()
    })?;
    let drives: HashSet<_> = objects
        .iter()
        .filter_map(|(path, interfaces)| {
            let drive = interfaces.get(UDISKS_DRIVE)?;
            ["Removable", "MediaRemovable"]
                .iter()
                .any(|name| {
                    drive
                        .get(*name)
                        .and_then(|value| bool::try_from(value).ok())
                        == Some(true)
                })
                .then_some(path.as_str())
        })
        .collect();
    Some(
        objects
            .values()
            .filter_map(|interfaces| {
                let block = interfaces.get(UDISKS_BLOCK)?;
                let drive: &ObjectPath<'_> = block.get("Drive")?.try_into().ok()?;
                if !drives.contains(drive.as_str()) {
                    return None;
                }
                block.get("DeviceNumber")?.try_into().ok()
            })
            .collect(),
    )
}

fn device_removable(mount: &Mount, udisks: &HashSet<u64>) -> bool {
    // Some filesystems (e.g. btrfs) use a virtual mountinfo device number.
    // Prefer the block source's dev_t, falling back to mountinfo without /dev.
    let device = fs::metadata(&mount.source)
        .ok()
        .filter(|metadata| metadata.file_type().is_block_device())
        .map_or_else(
            || libc::makedev(mount.device.0, mount.device.1),
            |metadata| metadata.rdev(),
        );
    if udisks.contains(&device) {
        return true;
    }
    let block = format!(
        "/sys/dev/block/{}:{}",
        libc::major(device),
        libc::minor(device)
    );
    sysfs_removable(Path::new(&block))
        || Path::new(&mount.source)
            .file_name()
            .is_some_and(|name| sysfs_removable(&Path::new("/sys/class/block").join(name)))
}

/// Read the Linux mount namespace, excluding pseudo filesystems and kernel FUSE control.
pub fn list_volumes() -> Result<Vec<Volume>, ServiceError> {
    let mounts = mounts().map_err(map_io_error)?;
    let udisks = if mounts
        .iter()
        .any(|mount| Path::new(&mount.source).starts_with("/dev"))
    {
        removable_devices().unwrap_or_default()
    } else {
        HashSet::new()
    };
    let mut volumes: Vec<_> = mounts
        .into_iter()
        .filter_map(|mount| {
            let under_media =
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
                || mount.point == Path::new("/")
                || under_media
                    && !matches!(
                        mount.fs_type.as_str(),
                        "autofs" | "proc" | "sysfs" | "devtmpfs"
                    );
            let removable = under_media || (block && device_removable(&mount, &udisks));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sysfs_partition_inherits_removability_from_the_parent_disk() {
        let dir = std::env::temp_dir().join(format!("zgui-removable-test-{}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        let disk = dir.join("sda");
        let partition = disk.join("sda1");
        let class_entry = dir.join("class-sda1");
        fs::create_dir_all(&partition).unwrap();
        std::os::unix::fs::symlink(&partition, &class_entry).unwrap();
        fs::write(partition.join("partition"), "1\n").unwrap();
        fs::write(disk.join("removable"), "1\n").unwrap();
        assert!(sysfs_removable(&class_entry));
        assert!(sysfs_removable(&disk));
        fs::write(disk.join("removable"), "0\n").unwrap();
        assert!(!sysfs_removable(&class_entry));
        fs::remove_dir_all(dir).unwrap();
    }
}
