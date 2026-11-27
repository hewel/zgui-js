use std::{
    fs,
    os::unix::fs::symlink,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use host_services::{
    files::FileMode,
    locale::LocaleContext,
    system_files::{EntryKind, ReadOptions, Root, Scope},
};

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "zgui-system-files-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn symlink_metadata_does_not_redeem_target_authority() {
    let temp = Temp::new();
    let home = temp.0.join("home");
    let disk = temp.0.join("disk");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&disk).unwrap();
    fs::write(disk.join("note.txt"), "outside").unwrap();
    symlink("../disk", home.join("img10")).unwrap();
    symlink("absent", home.join("img2")).unwrap();
    let root = Root::open(&home, FileMode::ReadWrite).unwrap();
    let mut scope = Scope::new(root);
    let locale = LocaleContext::live();
    let first = scope
        .read_dir(
            "",
            &ReadOptions {
                page_size: 1,
                ..ReadOptions::default()
            },
            &locale,
        )
        .unwrap();
    assert_eq!(first.total_entries, 2);
    assert_eq!(first.entries[0].relative_path, "img2");
    assert!(first.entries[0].symlink_broken);
    let second = scope
        .read_dir(
            "",
            &ReadOptions {
                cursor: first.next_cursor,
                page_size: 1,
                ..ReadOptions::default()
            },
            &locale,
        )
        .unwrap();
    assert!(second.next_cursor.is_none());
    let link = scope.lstat("img10").unwrap();
    assert_eq!(link, second.entries[0]);
    assert_eq!(link.kind, EntryKind::Symlink);
    assert_eq!(link.symlink_target.as_deref(), Some("../disk"));
    assert_eq!(link.symlink_target_kind, Some(EntryKind::Dir));
    assert!(!link.symlink_broken);
    assert_eq!(
        scope.lstat("img10/note.txt").unwrap_err().code,
        "E_PATH_ESCAPE"
    );
    // Checking only the final canonical path would grant traversal through an
    // ungranted mount that links back to an otherwise accessible directory.
    symlink("../home", disk.join("return")).unwrap();
    assert_eq!(
        scope.lstat("img10/return/img2").unwrap_err().code,
        "E_PATH_ESCAPE"
    );
    scope.add_root(Root::open(&disk, FileMode::Read).unwrap());
    assert_eq!(
        scope.lstat("img10/note.txt").unwrap().mime_type,
        "text/plain"
    );
    assert!(scope.lstat("img10/return/img2").unwrap().symlink_broken);
    assert_eq!(
        scope
            .remove_permanent("img10/note.txt", false)
            .unwrap_err()
            .code,
        "E_CAPABILITY"
    );
    assert!(disk.join("note.txt").is_file());
}

#[test]
fn recursive_removal_unlinks_only_the_symlink() {
    let temp = Temp::new();
    fs::create_dir(temp.0.join("target")).unwrap();
    fs::write(temp.0.join("target/keep.txt"), "keep").unwrap();
    symlink("target", temp.0.join("shortcut")).unwrap();
    let scope = Scope::new(Root::open(&temp.0, FileMode::ReadWrite).unwrap());
    scope.remove_permanent("shortcut", true).unwrap();
    assert!(fs::symlink_metadata(temp.0.join("shortcut")).is_err());
    assert_eq!(
        fs::read_to_string(temp.0.join("target/keep.txt")).unwrap(),
        "keep"
    );
}

#[test]
fn copy_streams_large_files_and_cancels_without_publishing_partial_data() {
    use host_services::system_files::{EntryRef, Job, Progress, run_job};
    use sha2::{Digest, Sha256};
    use std::{
        io::{Read, Write},
        sync::atomic::AtomicBool,
    };

    let temp = Temp::new();
    fs::create_dir(temp.0.join("dst")).unwrap();
    let mut source = fs::File::create(temp.0.join("big.bin")).unwrap();
    for index in 0..80_u8 {
        source.write_all(&vec![index; 256 * 1024]).unwrap();
    }
    let root = Scope::new(Root::open(&temp.0, FileMode::ReadWrite).unwrap());
    let make_job = || Job::Copy {
        sources: vec![EntryRef::new(root.clone(), "big.bin".into())],
        destination: EntryRef::new(root.clone(), "dst".into()),
    };
    let cancelled = AtomicBool::new(false);
    let result = run_job(
        make_job(),
        None,
        &cancelled,
        |_| panic!("unexpected conflict"),
        |progress: &Progress| {
            if progress.bytes_completed > 0 {
                assert!(progress.bytes_completed < progress.bytes_total);
                cancelled.store(true, Ordering::Release);
            }
        },
    );
    assert_eq!(result.unwrap_err().code, "Cancelled");
    assert_eq!(fs::read_dir(temp.0.join("dst")).unwrap().count(), 0);
    cancelled.store(false, Ordering::Release);
    run_job(
        make_job(),
        None,
        &cancelled,
        |_| panic!("unexpected conflict"),
        |_| {},
    )
    .unwrap();
    let digest = |path| {
        let mut file = fs::File::open(path).unwrap();
        let mut digest = Sha256::new();
        let mut chunk = [0_u8; 32768];
        loop {
            let size = file.read(&mut chunk).unwrap();
            if size == 0 {
                return digest.finalize();
            }
            digest.update(&chunk[..size]);
        }
    };
    assert_eq!(
        digest(temp.0.join("big.bin")),
        digest(temp.0.join("dst/big.bin"))
    );
}

#[test]
fn conflicting_transfer_pauses_before_any_overwrite() {
    use host_services::system_files::{ConflictAction, ConflictResolution, EntryRef, Job, run_job};
    use std::sync::{atomic::AtomicBool, mpsc::sync_channel};
    let temp = Temp::new();
    fs::create_dir(temp.0.join("src")).unwrap();
    fs::create_dir(temp.0.join("dst")).unwrap();
    fs::write(temp.0.join("src/report.txt"), "incoming").unwrap();
    let root = Scope::new(Root::open(&temp.0, FileMode::ReadWrite).unwrap());
    for (action, expected) in [
        (ConflictAction::Skip, "existing"),
        (ConflictAction::AutoRename, "existing"),
        (ConflictAction::Cancel, "existing"),
        (ConflictAction::Overwrite, "incoming"),
    ] {
        fs::write(temp.0.join("dst/report.txt"), "existing").unwrap();
        let job = Job::Copy {
            sources: vec![EntryRef::new(root.clone(), "src/report.txt".into())],
            destination: EntryRef::new(root.clone(), "dst".into()),
        };
        let (notify, paused) = sync_channel(0);
        let (resume, decision) = sync_channel(0);
        let worker = std::thread::spawn(move || {
            run_job(
                job,
                None,
                &AtomicBool::new(false),
                |conflict| {
                    notify.send(conflict).unwrap();
                    decision.recv().unwrap()
                },
                |_| {},
            )
        });
        let conflict = paused.recv().unwrap();
        assert_eq!(conflict.destination_path, "dst/report.txt");
        assert_eq!(
            fs::read_to_string(temp.0.join("dst/report.txt")).unwrap(),
            "existing"
        );
        resume
            .send(ConflictResolution {
                action,
                apply_to_all: false,
            })
            .unwrap();
        let result = worker.join().unwrap();
        if action == ConflictAction::Cancel {
            assert_eq!(result.unwrap_err().code, "Cancelled");
        } else {
            result.unwrap();
        }
        assert_eq!(
            fs::read_to_string(temp.0.join("dst/report.txt")).unwrap(),
            expected
        );
    }
    assert_eq!(
        fs::read_to_string(temp.0.join("dst/report (1).txt")).unwrap(),
        "incoming"
    );
}

#[test]
fn freedesktop_trash_publishes_info_and_restores_the_original_path() {
    use host_services::system_files::{EntryRef, Job, JobResult, Trash, run_job};
    use std::sync::atomic::AtomicBool;
    let temp = Temp::new();
    let data_home = temp.0.join("data");
    let root = Scope::new(Root::open(&temp.0, FileMode::ReadWrite).unwrap());
    let trash = Trash::new(data_home.clone());
    let run = |job| {
        run_job(
            job,
            Some(&trash),
            &AtomicBool::new(false),
            |_| panic!("unexpected conflict"),
            |_| {},
        )
        .unwrap()
    };
    fs::write(temp.0.join("report #1.txt"), "recovery").unwrap();
    run(Job::Trash {
        entries: vec![EntryRef::new(root.clone(), "report #1.txt".into())],
    });
    assert!(!temp.0.join("report #1.txt").exists());
    assert_eq!(
        fs::read_to_string(data_home.join("Trash/files/report #1.txt")).unwrap(),
        "recovery"
    );
    let info_path = data_home.join("Trash/info/report #1.txt.trashinfo");
    let info = fs::read_to_string(&info_path).unwrap();
    assert!(info.starts_with("[Trash Info]\nPath=/"));
    assert!(info.contains("/report%20%231.txt\nDeletionDate="));
    assert_eq!(
        info.lines()
            .last()
            .unwrap()
            .trim_start_matches("DeletionDate=")
            .len(),
        19
    );
    let JobResult::Trash(entries) = run(Job::ListTrash {
        roots: vec![root.root().clone()],
    }) else {
        panic!("expected a Trash inventory");
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "report #1.txt");
    run(Job::RestoreFromTrash {
        ids: vec![entries[0].id.clone()],
        roots: vec![root.root().clone()],
    });
    assert!(!info_path.exists());
    assert!(!data_home.join("Trash/files/report #1.txt").exists());
    assert_eq!(
        fs::read_to_string(temp.0.join("report #1.txt")).unwrap(),
        "recovery"
    );
}

#[test]
fn freedesktop_thumbnail_matches_source_seconds() {
    use host_services::system_files::{ThumbnailSize, cached_thumbnail, thumbnail_key};
    use std::os::unix::fs::MetadataExt;

    let temp = Temp::new();
    let file = temp.0.join("picture #1.png");
    fs::write(&file, "source").unwrap();
    assert_eq!(
        thumbnail_key(std::path::Path::new("/path/to/image.png")).unwrap(),
        "88e2b304a2e2fce3c76791eaffc5996e",
    );
    let cache = temp.0.join("cache");
    let dir = cache.join("thumbnails/normal");
    fs::create_dir_all(&dir).unwrap();
    let thumbnail = dir.join(format!("{}.png", thumbnail_key(&file).unwrap()));
    let mtime = fs::metadata(&file).unwrap().mtime();
    let png = |seconds| {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut push = |kind: &[u8; 4], data: &[u8]| {
            png.extend_from_slice(&(data.len() as u32).to_be_bytes());
            png.extend_from_slice(kind);
            png.extend_from_slice(data);
            let mut crc = flate2::Crc::new();
            crc.update(kind);
            crc.update(data);
            png.extend_from_slice(&crc.sum().to_be_bytes());
        };
        push(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0]);
        push(b"tEXt", format!("Thumb::MTime\0{seconds}").as_bytes());
        push(b"IDAT", &[0x78, 0x9c, 0x63, 0x60, 0, 0, 0, 2, 0, 1]);
        push(b"IEND", &[]);
        png
    };
    let valid = png(mtime);
    fs::write(&thumbnail, &valid).unwrap();
    let scope = Scope::new(Root::open(&temp.0, FileMode::Read).unwrap());
    assert_eq!(
        cached_thumbnail(&scope, "picture #1.png", ThumbnailSize::Normal, &cache)
            .unwrap()
            .as_deref(),
        Some(valid.as_slice())
    );
    fs::write(&thumbnail, png(mtime - 1)).unwrap();
    assert!(
        cached_thumbnail(&scope, "picture #1.png", ThumbnailSize::Normal, &cache)
            .unwrap()
            .is_none()
    );
}

#[test]
fn copy_overwrite_cannot_destroy_a_source_ancestor() {
    use host_services::system_files::{ConflictAction, ConflictResolution, EntryRef, Job, run_job};
    use std::sync::atomic::AtomicBool;
    let temp = Temp::new();
    fs::create_dir_all(temp.0.join("same/same")).unwrap();
    fs::write(temp.0.join("same/keep"), "original sibling").unwrap();
    fs::write(temp.0.join("same/same/inside"), "copy source").unwrap();
    let scope = Scope::new(Root::open(&temp.0, FileMode::ReadWrite).unwrap());
    let error = run_job(
        Job::Copy {
            sources: vec![EntryRef::new(scope.clone(), "same/same".into())],
            destination: EntryRef::new(scope, String::new()),
        },
        None,
        &AtomicBool::new(false),
        |_| ConflictResolution {
            action: ConflictAction::Overwrite,
            apply_to_all: false,
        },
        |_| {},
    )
    .unwrap_err();
    assert_eq!(error.code, "InvalidPath");
    assert_eq!(
        fs::read_to_string(temp.0.join("same/keep")).unwrap(),
        "original sibling"
    );
    assert_eq!(
        fs::read_to_string(temp.0.join("same/same/inside")).unwrap(),
        "copy source"
    );
}

#[test]
fn unsafe_home_trash_is_unavailable_without_deleting_source() {
    use host_services::system_files::{EntryRef, Job, Trash, run_job};
    use std::sync::atomic::AtomicBool;
    let temp = Temp::new();
    let data_home = temp.0.join("data");
    fs::create_dir(&data_home).unwrap();
    fs::create_dir(temp.0.join("other")).unwrap();
    symlink("../other", data_home.join("Trash")).unwrap();
    fs::write(temp.0.join("keep"), "original").unwrap();
    let root = Scope::new(Root::open(&temp.0, FileMode::ReadWrite).unwrap());
    let error = run_job(
        Job::Trash {
            entries: vec![EntryRef::new(root, "keep".into())],
        },
        Some(&Trash::new(data_home)),
        &AtomicBool::new(false),
        |_| panic!("unexpected conflict"),
        |_| {},
    )
    .unwrap_err();
    assert_eq!(error.code, "E_TRASH_UNAVAILABLE");
    assert_eq!(fs::read_to_string(temp.0.join("keep")).unwrap(), "original");
    assert_eq!(fs::read_dir(temp.0.join("other")).unwrap().count(), 0);
}

#[test]
fn cross_device_move_streams_and_preserves_mode_and_mtime() {
    use host_services::system_files::{EntryRef, Job, run_job};
    use std::{
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::Path,
        sync::atomic::AtomicBool,
    };

    let from = Temp::new();
    let shm = Path::new("/dev/shm");
    let source_device = fs::metadata(&from.0).unwrap().dev();
    if !fs::metadata(shm).is_ok_and(|meta| meta.dev() != source_device) {
        return; // No second filesystem in this test environment.
    }
    let dest = shm.join(from.0.file_name().unwrap());
    match fs::create_dir(&dest) {
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem
            ) =>
        {
            return;
        }
        result => result.unwrap(),
    }
    let dest = Temp(dest);
    let path = from.0.join("payload");
    fs::write(&path, b"cross device").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let before = fs::metadata(&path).unwrap();
    let src = Scope::new(Root::open(&from.0, FileMode::ReadWrite).unwrap());
    let dst = Scope::new(Root::open(&dest.0, FileMode::ReadWrite).unwrap());
    let mut transferred = false;
    run_job(
        Job::Move {
            sources: vec![EntryRef::new(src, "payload".into())],
            destination: EntryRef::new(dst, String::new()),
        },
        None,
        &AtomicBool::new(false),
        |_| panic!("unexpected conflict"),
        |progress| {
            transferred |= progress.items_completed == 0 && progress.bytes_completed > 0;
        },
    )
    .unwrap();
    assert!(transferred, "cross-device move copied before committing");
    assert!(!path.exists());
    let target = dest.0.join("payload");
    assert_eq!(fs::read(&target).unwrap(), b"cross device");
    let after = fs::metadata(target).unwrap();
    assert_eq!(after.mode() & 0o7777, 0o640);
    assert_eq!(
        (after.mtime(), after.mtime_nsec()),
        (before.mtime(), before.mtime_nsec())
    );
}
