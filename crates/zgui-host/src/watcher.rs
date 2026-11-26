use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
        Arc,
    },
    thread::{self, Thread},
    time::Duration,
};

use crate::{build_status::{BuildStatus, MAX_STATUS_BYTES, STATUS_FILE, Status, Typecheck}, package::Package};

type Publication = Result<Package, String>;
const POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DevBuildStatus {
    Clear,
    Failed { title: String, detail: String },
}

pub(crate) fn parse_build_status(bytes: &[u8]) -> Option<DevBuildStatus> {
    let status = BuildStatus::parse(bytes).ok()?;
    match status.status {
        Status::Building => None,
        Status::Failed => Some(DevBuildStatus::Failed {
            title: "Build failed".into(),
            detail: status.error?.message,
        }),
        Status::Published => match status.typecheck {
            Some(Typecheck::Failed { diagnostics }) => Some(DevBuildStatus::Failed {
                title: "Typecheck error".into(),
                detail: diagnostics,
            }),
            Some(Typecheck::Running { .. }) => None,
            Some(Typecheck::Passed { .. }) | None => Some(DevBuildStatus::Clear),
        },
    }
}

/// Validates immutable publications off the owner thread. Only `next`'s caller
/// handles the resulting package; the wake callback carries no package state.
pub(crate) struct PackageWatcher {
    results: Option<Receiver<Publication>>,
    statuses: Option<Receiver<DevBuildStatus>>,
    stop: Arc<AtomicBool>,
    thread: Thread,
}

impl PackageWatcher {
    pub(crate) fn start(
        path: PathBuf,
        initial: PathBuf,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> std::io::Result<Self> {
        let (sender, results) = mpsc::sync_channel(1);
        let (status_sender, statuses) = mpsc::sync_channel(4);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let status_path = path.parent().map(|parent| parent.join(STATUS_FILE));
        let worker = thread::Builder::new().name("package-watcher".into()).spawn(move || {
            let mut published = PublishedTarget { directory: initial, error: None };
            let mut last_status_bytes: Option<Vec<u8>> = None;
            loop {
                thread::park_timeout(POLL_INTERVAL);
                if worker_stop.load(Ordering::Acquire) { break; }
                let mut woke = false;
                if let Some(status_file) = &status_path
                    && let Ok(bytes) = crate::bounded_fs::read_input(status_file, MAX_STATUS_BYTES)
                    && last_status_bytes.as_deref() != Some(bytes.as_slice())
                {
                    let parsed = parse_build_status(&bytes);
                    last_status_bytes = Some(bytes);
                    if let Some(parsed) = parsed {
                        let _ = status_sender.try_send(parsed);
                        woke = true;
                    }
                }
                if let Some(result) = published.scan(&path) {
                    if worker_stop.load(Ordering::Acquire) { break; }
                    // A single producer preserves FIFO while this bounded send
                    // applies backpressure. The next scan resolves dist afresh,
                    // rather than retaining a possibly superseded publication.
                    if sender.send(result).is_err() { break; }
                    if worker_stop.load(Ordering::Acquire) { break; }
                    woke = true;
                }
                if woke {
                    wake();
                }
            }
        })?;
        let thread = worker.thread().clone();
        // Detach: teardown must never wait for filesystem I/O to finish.
        drop(worker);
        Ok(Self { results: Some(results), statuses: Some(statuses), stop, thread })
    }

    pub(crate) fn next(&self) -> Option<Publication> {
        self.results.as_ref()?.try_recv().ok()
    }

    pub(crate) fn next_status(&self) -> Option<DevBuildStatus> {
        let rx = self.statuses.as_ref()?;
        let mut latest = None;
        while let Ok(item) = rx.try_recv() {
            latest = Some(item);
        }
        latest
    }
}

impl Drop for PackageWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.thread.unpark();
        // Disconnect before returning, including when send is blocked on a
        // full channel. Joining here could hang the owner on filesystem I/O.
        drop(self.results.take());
        drop(self.statuses.take());
    }
}

struct PublishedTarget {
    directory: PathBuf,
    error: Option<String>,
}

impl PublishedTarget {
    fn scan(&mut self, path: &Path) -> Option<Publication> {
        let directory = match fs::canonicalize(path) {
            Ok(directory) => directory,
            Err(error) => {
                let error = format!("cannot resolve package publication {}: {error}", path.display());
                if self.error.as_ref() == Some(&error) { return None; }
                self.error = Some(error.clone());
                return Some(Err(error));
            }
        };
        self.error = None;
        if self.directory == directory { return None; }
        // Failed validation is also final for this immutable generation.
        // Passing the resolved directory avoids following dist a second time.
        self.directory = directory;
        Some(Package::load(&self.directory).map_err(|error| error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{os::unix::fs::symlink, sync::{atomic::AtomicU64, Mutex}};

    const WAIT: Duration = Duration::from_secs(5);
    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let directory = std::env::temp_dir().join(format!(
                "zgui-watcher-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir(&directory).unwrap();
            Self(directory)
        }

        fn generation(&self, name: &str) -> PathBuf {
            let directory = self.0.join(name);
            fs::create_dir(&directory).unwrap();
            let source = format!("export const generation = {name:?};");
            let map = br#"{"version":3,"sources":[],"names":[],"mappings":""}"#;
            fs::write(directory.join("app.mjs"), source.as_bytes()).unwrap();
            fs::write(directory.join("app.mjs.map"), map).unwrap();
            let style_system = serde_json::json!({"name":"lightningcss","profile":"lightningcss-v1",
                "configDigest":format!("sha256:{:x}", Sha256::digest(b"test"))});
            let metadata = serde_json::to_vec(&serde_json::json!({"styleSystem":style_system})).unwrap();
            fs::write(directory.join("build-info.json"), &metadata).unwrap();
            let digests: std::collections::BTreeMap<_, _> = ["app.mjs", "app.mjs.map", "build-info.json"].into_iter()
                .map(|name| (name, format!("sha256:{:x}", Sha256::digest(fs::read(directory.join(name)).unwrap())))).collect();
            let mut manifest = serde_json::json!({
                "schemaVersion": 2, "id": "watcher-test", "hostApiVersion": 1,
                "entry": "app.mjs", "sourceMap": "app.mjs.map", "styles": [], "assets": [], "fonts": [],
                "styleSystem":style_system,
                "artifactDigests":digests
            });
            manifest["buildId"] = serde_json::json!(format!("sha256:{:x}", Sha256::digest(serde_json::to_vec(&manifest).unwrap())));
            fs::write(directory.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
            directory.canonicalize().unwrap()
        }

        fn path(&self) -> PathBuf { self.0.join("dist") }

        fn publish(&self, directory: &Path) {
            let temporary = self.0.join("dist-next");
            symlink(directory, &temporary).unwrap();
            fs::rename(temporary, self.path()).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) { fs::remove_dir_all(&self.0).unwrap(); }
    }

    #[test]
    fn only_loads_changed_canonical_generations() {
        let fixture = Fixture::new();
        let initial = fixture.generation("initial");
        fixture.publish(&initial);
        let initial = Package::load(&fixture.path()).unwrap().directory;
        let mut published = PublishedTarget { directory: initial.clone(), error: None };
        assert!(published.scan(&fixture.path()).is_none());

        let second = fixture.generation("second");
        fixture.publish(&second);
        assert_eq!(published.scan(&fixture.path()).unwrap().unwrap().directory, second);
        assert!(published.scan(&fixture.path()).is_none());
        let alias = fixture.0.join("alias");
        symlink(&second, &alias).unwrap();
        fixture.publish(&alias);
        assert!(published.scan(&fixture.path()).is_none());
        fixture.publish(&initial);
        assert_eq!(published.scan(&fixture.path()).unwrap().unwrap().directory, initial);
    }

    #[test]
    fn invalid_generation_is_reported_once_then_recovers() {
        let fixture = Fixture::new();
        let initial = fixture.generation("initial");
        let mut published = PublishedTarget { directory: initial, error: None };
        let invalid = fixture.generation("invalid");
        // Complete the invalid fixture before publishing it; published
        // generations themselves remain immutable throughout the test.
        fs::write(invalid.join("app.mjs"), b"corrupted").unwrap();
        fixture.publish(&invalid);
        let error = published.scan(&fixture.path()).unwrap().err().unwrap();
        assert!(error.contains("integrity mismatch"), "{error}");
        assert!(published.scan(&fixture.path()).is_none());

        let recovered = fixture.generation("recovered");
        fixture.publish(&recovered);
        assert_eq!(published.scan(&fixture.path()).unwrap().unwrap().directory, recovered);
        assert!(published.scan(&fixture.path()).is_none());
    }

    #[test]
    fn canonicalization_errors_coalesce_until_resolution_recovers() {
        let fixture = Fixture::new();
        let initial = fixture.generation("initial");
        let mut published = PublishedTarget { directory: initial.clone(), error: None };
        assert!(published.scan(&fixture.path()).unwrap().is_err());
        assert!(published.scan(&fixture.path()).is_none());
        fixture.publish(&initial);
        assert!(published.scan(&fixture.path()).is_none());
        fs::remove_file(fixture.path()).unwrap();
        assert!(published.scan(&fixture.path()).unwrap().is_err());
        assert!(published.scan(&fixture.path()).is_none());
        let recovered = fixture.generation("recovered");
        fixture.publish(&recovered);
        assert_eq!(published.scan(&fixture.path()).unwrap().unwrap().directory, recovered);
    }

    struct WorkerEnded(mpsc::Sender<()>);
    impl Drop for WorkerEnded {
        fn drop(&mut self) { let _ = self.0.send(()); }
    }

    struct WakeGate {
        wakes: Receiver<()>,
        resume: mpsc::Sender<()>,
        ended: Receiver<()>,
    }

    impl WakeGate {
        fn start(fixture: &Fixture, initial: PathBuf) -> (PackageWatcher, Self) {
            let (wake, wakes) = mpsc::channel();
            let (resume, resumed) = mpsc::channel();
            let resumed = Mutex::new(resumed);
            let (end, ended) = mpsc::channel();
            let end = WorkerEnded(end);
            let watcher = PackageWatcher::start(fixture.path(), initial, Arc::new(move || {
                let _keep_alive = &end;
                wake.send(()).unwrap();
                resumed.lock().unwrap().recv_timeout(WAIT).unwrap();
            })).unwrap();
            (watcher, Self { wakes, resume, ended })
        }
    }

    #[test]
    fn worker_delivers_fifo_and_scans_current_publication_after_delivery() {
        let fixture = Fixture::new();
        let initial = fixture.generation("initial");
        fixture.publish(&initial);
        let (watcher, gate) = WakeGate::start(&fixture, initial);
        assert!(watcher.next().is_none());

        let first = fixture.generation("first");
        fixture.publish(&first);
        watcher.thread.unpark();
        gate.wakes.recv_timeout(WAIT).unwrap();
        // Keep the first result queued while subsequent publications replace
        // each other. The worker is held in wake, without timing-based sleeps.
        let superseded = fixture.generation("superseded");
        fixture.publish(&superseded);
        let latest = fixture.generation("latest");
        fixture.publish(&latest);
        watcher.thread.unpark();
        gate.resume.send(()).unwrap();
        assert_eq!(watcher.next().unwrap().unwrap().directory, first);
        gate.wakes.recv_timeout(WAIT).unwrap();
        assert_eq!(watcher.next().unwrap().unwrap().directory, latest);
        assert!(watcher.next().is_none());
        drop(watcher);
        gate.resume.send(()).unwrap();
        gate.ended.recv_timeout(WAIT).unwrap();
    }

    #[test]
    fn drop_cancels_a_parked_worker() {
        let fixture = Fixture::new();
        let initial = fixture.generation("initial");
        fixture.publish(&initial);
        let (watcher, gate) = WakeGate::start(&fixture, initial);
        drop(watcher);
        gate.ended.recv_timeout(WAIT).unwrap();
        assert!(gate.wakes.try_recv().is_err());
    }

    #[test]
    fn drop_releases_worker_with_unread_results() {
        let fixture = Fixture::new();
        let initial = fixture.generation("initial");
        fixture.publish(&initial);
        let (watcher, gate) = WakeGate::start(&fixture, initial);
        let first = fixture.generation("first");
        fixture.publish(&first);
        watcher.thread.unpark();
        gate.wakes.recv_timeout(WAIT).unwrap();
        let second = fixture.generation("second");
        fixture.publish(&second);
        watcher.thread.unpark();
        gate.resume.send(()).unwrap();
        // Whether scanning or already blocked sending, teardown must release
        // the worker without draining the result channel or joining it.
        drop(watcher);
        gate.ended.recv_timeout(WAIT).unwrap();
    }

    #[test]
    fn receiver_disconnection_terminates_the_worker_without_a_wake() {
        let fixture = Fixture::new();
        let initial = fixture.generation("initial");
        fixture.publish(&initial);
        let (mut watcher, gate) = WakeGate::start(&fixture, initial);
        drop(watcher.results.take());
        let next = fixture.generation("next");
        fixture.publish(&next);
        watcher.thread.unpark();
        gate.ended.recv_timeout(WAIT).unwrap();
        assert!(watcher.next().is_none());
        assert!(gate.wakes.try_recv().is_err());
    }

    #[test]
    fn parse_build_status_reports_build_failure_and_ignores_in_progress_build() {
        let mut record = serde_json::json!({
            "schemaVersion": 1, "attemptId": "00000000-1234-4321-9000-000000000000", "app": "watcher-test",
            "status": "failed", "previousDirectory": "/tmp/previous", "publicationDirectory": null, "buildId": null,
            "error": {"code": "E_BUILD", "message": "src/main.tsx:2:1 bad"},
        });
        let parse = |record: &serde_json::Value| parse_build_status(&serde_json::to_vec(record).unwrap());
        assert_eq!(parse(&record),
            Some(DevBuildStatus::Failed { title: "Build failed".into(), detail: "src/main.tsx:2:1 bad".into() }));
        record["status"] = serde_json::json!("building");
        record["error"] = serde_json::Value::Null;
        assert_eq!(parse(&record), None);
    }

    #[test]
    fn parse_build_status_uses_the_strict_publication_contract() {
        let mut record = serde_json::json!({
            "schemaVersion": 1, "attemptId": "00000000-1234-4321-9000-000000000000", "app": "watcher-test",
            "status": "published", "previousDirectory": "/tmp/previous",
            "publicationDirectory": "/tmp/current", "buildId": format!("sha256:{}", "a".repeat(64)),
            "error": null,
        });
        let parse = |record: &serde_json::Value| parse_build_status(&serde_json::to_vec(record).unwrap());
        record["typecheck"] = serde_json::json!({"status": "failed", "diagnostics": "TS2322"});
        assert_eq!(parse(&record), Some(DevBuildStatus::Failed { title: "Typecheck error".into(), detail: "TS2322".into() }));
        record["typecheck"] = serde_json::json!({"status": "running", "diagnostics": null});
        assert_eq!(parse(&record), None);
        record["typecheck"] = serde_json::json!({"status": "passed", "diagnostics": null});
        assert_eq!(parse(&record), Some(DevBuildStatus::Clear));
        record["typecheck"]["diagnostics"] = serde_json::json!("unexpected");
        assert_eq!(parse(&record), None, "an invalid typecheck must not clear an existing banner");
    }
}
