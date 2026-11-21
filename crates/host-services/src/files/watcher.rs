//! Debounced worker-thread directory watcher backed by `notify`.
#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    thread,
    time::{Duration, Instant},
};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::{
    ServiceError,
    files::{
        MAX_WATCH_DEBOUNCE_MS, MIN_WATCH_DEBOUNCE_MS, WatchEvent, WatchEventKind,
    },
};

const MAX_PENDING_WATCH_BATCHES: usize = 64;

/// Active debounced filesystem watcher rooted at a canonical directory.
///
/// Dropping or calling `stop()` immediately signals the worker thread to exit
/// and drops the OS watcher without blocking the owner thread.
pub struct DirectoryWatcher {
    stop_flag: Arc<AtomicBool>,
    stop_tx: Mutex<Option<mpsc::SyncSender<()>>>,
    events: Mutex<Receiver<Vec<WatchEvent>>>,
}

impl DirectoryWatcher {
    /// Starts watching `root_canonical` on a background thread, coalescing bursts within `debounce_ms`.
    pub fn start(
        root_canonical: PathBuf,
        recursive: bool,
        debounce_ms: u64,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, ServiceError> {
        let debounce = Duration::from_millis(
            debounce_ms.clamp(MIN_WATCH_DEBOUNCE_MS, MAX_WATCH_DEBOUNCE_MS),
        );
        let (raw_tx, raw_rx) = mpsc::channel::<notify::Result<Event>>();
        let mut watcher = RecommendedWatcher::new(
            move |res| {
                let _ = raw_tx.send(res);
            },
            notify::Config::default(),
        )
        .map_err(|err| ServiceError::new("Io", format!("cannot create directory watcher: {err}")))?;

        let mode = if recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        watcher
            .watch(&root_canonical, mode)
            .map_err(|err| ServiceError::new("Io", format!("cannot watch directory: {err}")))?;

        let stop_flag = Arc::new(AtomicBool::new(false));
        let (stop_tx, stop_rx) = mpsc::sync_channel::<()>(1);
        let (batch_tx, batch_rx) = mpsc::sync_channel::<Vec<WatchEvent>>(MAX_PENDING_WATCH_BATCHES);

        let worker_stop = stop_flag.clone();
        thread::Builder::new()
            .name("zgui-fs-watcher".to_owned())
            .spawn(move || {
                // Keep watcher owned by this thread so stopping drops the OS descriptor cleanly.
                let _owned_watcher = watcher;
                watcher_loop(
                    &root_canonical,
                    debounce,
                    raw_rx,
                    stop_rx,
                    worker_stop,
                    batch_tx,
                    wake,
                );
            })
            .map_err(|err| ServiceError::new("Io", err.to_string()))?;

        Ok(Self {
            stop_flag,
            stop_tx: Mutex::new(Some(stop_tx)),
            events: Mutex::new(batch_rx),
        })
    }

    /// Polls one coalesced batch of watch events without blocking.
    pub fn try_recv(&self) -> Option<Vec<WatchEvent>> {
        self.events.lock().ok()?.try_recv().ok()
    }

    /// Stops watching without blocking the caller.
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Release);
        if let Ok(mut guard) = self.stop_tx.lock()
            && let Some(tx) = guard.take()
        {
            let _ = tx.try_send(());
        }
    }
}

impl Drop for DirectoryWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

fn watcher_loop(
    root_canonical: &Path,
    debounce: Duration,
    raw_rx: Receiver<notify::Result<Event>>,
    stop_rx: Receiver<()>,
    stop_flag: Arc<AtomicBool>,
    batch_tx: mpsc::SyncSender<Vec<WatchEvent>>,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    let mut pending: BTreeMap<String, WatchEventKind> = BTreeMap::new();
    let mut flush_at: Option<Instant> = None;

    while !stop_flag.load(Ordering::Acquire) {
        if stop_rx.try_recv().is_ok() {
            break;
        }
        let wait = match flush_at {
            Some(deadline) => deadline
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::ZERO),
            None => Duration::from_millis(50),
        };
        match raw_rx.recv_timeout(wait) {
            Ok(Ok(event)) => {
                let Some(kind) = map_event_kind(&event.kind) else {
                    continue;
                };
                for path in event.paths {
                    if let Some(rel) = relative_to_root(root_canonical, &path) {
                        if rel.is_empty() || rel.split('/').any(|seg| seg.starts_with(".zgui-tmp-")) {
                            continue;
                        }
                        pending
                            .entry(rel)
                            .and_modify(|existing| *existing = merge_kind(*existing, kind))
                            .or_insert(kind);
                    }
                }
                if !pending.is_empty() && flush_at.is_none() {
                    flush_at = Some(Instant::now() + debounce);
                }
            }
            Ok(Err(_)) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        if let Some(deadline) = flush_at
            && Instant::now() >= deadline
        {
            flush_at = None;
            if !pending.is_empty() {
                let batch: Vec<WatchEvent> = std::mem::take(&mut pending)
                    .into_iter()
                    .map(|(path, kind)| WatchEvent { path, kind })
                    .collect();
                if batch_tx.try_send(batch).is_ok() {
                    wake();
                }
            }
        }
    }
}

fn relative_to_root(root_canonical: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root_canonical).ok()?;
    let text = rel.to_string_lossy();
    Some(text.into_owned())
}

fn map_event_kind(kind: &EventKind) -> Option<WatchEventKind> {
    match kind {
        EventKind::Access(_) => None,
        EventKind::Create(_) => Some(WatchEventKind::Create),
        EventKind::Modify(notify::event::ModifyKind::Name(_)) => Some(WatchEventKind::Rename),
        EventKind::Modify(_) => Some(WatchEventKind::Modify),
        EventKind::Remove(_) => Some(WatchEventKind::Remove),
        _ => Some(WatchEventKind::Other),
    }
}

const fn merge_kind(existing: WatchEventKind, incoming: WatchEventKind) -> WatchEventKind {
    match (existing, incoming) {
        (_, WatchEventKind::Remove) => WatchEventKind::Remove,
        (WatchEventKind::Create, WatchEventKind::Modify) => WatchEventKind::Create,
        (_, other) => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn debounced_watcher_coalesces_burst_writes() {
        let dir = std::env::temp_dir().join(format!(
            "zgui-watch-test-{}-{}",
            std::process::id(),
            crate::files::TMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let canonical = dir.canonicalize().unwrap();

        let wakes = Arc::new(AtomicBool::new(false));
        let wakes_cb = wakes.clone();
        let watcher = DirectoryWatcher::start(
            canonical.clone(),
            true,
            60,
            Arc::new(move || wakes_cb.store(true, Ordering::Release)),
        )
        .expect("start watcher");

        // Rapid burst of 5 writes to the same file.
        for i in 0..5 {
            fs::write(canonical.join("burst.md"), format!("iteration {i}")).unwrap();
        }

        let start = Instant::now();
        let mut received = Vec::new();
        while start.elapsed() < Duration::from_secs(2) {
            if let Some(batch) = watcher.try_recv() {
                received.extend(batch);
                if received.iter().any(|e| e.path == "burst.md") {
                    break;
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(wakes.load(Ordering::Acquire));
        assert!(
            received.iter().any(|e| e.path == "burst.md"),
            "expected burst.md event, got {received:?}"
        );
        // Coalesced: far fewer events than 5 writes * 2 raw inotify events.
        assert!(received.len() <= 3, "expected coalesced events, got {received:?}");

        watcher.stop();
        let _ = fs::remove_dir_all(&canonical);
    }
}
