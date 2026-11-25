//! Background watcher monitoring `--extensions-overlay <path>` for atomic JSON updates.
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, Thread},
    time::Duration,
};

use sha2::{Digest, Sha256};

const POLL_INTERVAL: Duration = Duration::from_millis(250);
const MAX_OVERLAY_BYTES: u64 = 4 * 1024 * 1024;

pub(crate) struct OverlayWatcher {
    updates: Option<Receiver<PathBuf>>,
    stop: Arc<AtomicBool>,
    thread: Thread,
}

impl OverlayWatcher {
    pub(crate) fn start(
        path: PathBuf,
        initial_digest: Option<[u8; 32]>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> std::io::Result<Self> {
        let (sender, updates) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::Builder::new().name("overlay-watcher".into()).spawn(move || {
            let mut last_digest = initial_digest;
            loop {
                thread::park_timeout(POLL_INTERVAL);
                if worker_stop.load(Ordering::Acquire) {
                    break;
                }
                let Ok(meta) = fs::symlink_metadata(&path) else {
                    continue;
                };
                if meta.file_type().is_symlink() || !meta.file_type().is_file() || meta.len() > MAX_OVERLAY_BYTES {
                    continue;
                }
                let Ok(bytes) = fs::read(&path) else {
                    continue;
                };
                let digest: [u8; 32] = Sha256::digest(&bytes).into();
                if last_digest == Some(digest) {
                    continue;
                }
                last_digest = Some(digest);
                if worker_stop.load(Ordering::Acquire) {
                    break;
                }
                if sender.send(path.clone()).is_err() {
                    break;
                }
                wake();
            }
        })?;
        let thread = worker.thread().clone();
        drop(worker);
        Ok(Self {
            updates: Some(updates),
            stop,
            thread,
        })
    }

    pub(crate) fn poll(&self) -> Option<PathBuf> {
        self.updates.as_ref()?.try_recv().ok()
    }
}

impl Drop for OverlayWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.thread.unpark();
        drop(self.updates.take());
    }
}
