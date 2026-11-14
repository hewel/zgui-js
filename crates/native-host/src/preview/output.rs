use super::Error;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs::{self, OpenOptions}, io::{Read, Write}, path::{Component, Path, PathBuf}};

pub(crate) fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

const OWNER_FILE: &str = ".preview-owner";

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OutputState {
    entries: Vec<Value>,
    used: u64,
}

pub(crate) struct Output {
    root: PathBuf,
    limit: u64,
    used: u64,
    entries: Vec<Value>,
}

impl Output {
    pub fn create(path: &Path, limit: u64) -> Result<Self, Error> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if !metadata.is_dir() || metadata.file_type().is_symlink() => {
                return Err(Error::new("E_OUTPUT_IO", 8, "output must be a real directory"));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(path).map_err(io_error)?;
            }
            Err(error) => return Err(io_error(error)),
        }
        if fs::read_dir(path).map_err(io_error)?.next().is_some() {
            return Err(Error::new("E_OUTPUT_IO", 8, "refusing a nonempty output directory"));
        }
        let mut output = Self { root: path.canonicalize().map_err(io_error)?, limit, used: 0, entries: Vec::new() };
        // Atomic no-replace publication below makes this an exclusive run claim,
        // even if two controllers both observed the directory while it was empty.
        output.bytes(OWNER_FILE, "run-ownership", format!("{}\n", std::process::id()).as_bytes())?;
        Ok(output)
    }

    pub fn attach_worker(path: &Path, limit: u64, parent_pid: u32) -> Result<Self, Error> {
        let root = path.canonicalize().map_err(io_error)?;
        let mut files = fs::read_dir(&root).map_err(io_error)?;
        let first = files.next().transpose().map_err(io_error)?;
        if first.as_ref().is_none_or(|entry| entry.file_name() != OWNER_FILE)
            || files.next().transpose().map_err(io_error)?.is_some() {
            return Err(Error::new("E_OUTPUT_IO", 8, "worker requires its controller's fresh output directory"));
        }
        let metadata = fs::symlink_metadata(root.join(OWNER_FILE)).map_err(io_error)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 16 {
            return Err(Error::new("E_OUTPUT_IO", 8, "invalid output ownership record"));
        }
        let owner = fs::read(root.join(OWNER_FILE)).map_err(io_error)?;
        if owner != format!("{parent_pid}\n").as_bytes() {
            return Err(Error::new("E_OUTPUT_IO", 8, "output belongs to a different controller"));
        }
        let output = Self {
            root, limit, used: owner.len() as u64,
            entries: vec![json!({"path":OWNER_FILE,"role":"run-ownership","status":"complete","bytes":owner.len(),"digest":digest(&owner)})],
        };
        output.reserve(0)?;
        Ok(output)
    }

    /// Recover only this exclusively owned run after the child has stopped. Final
    /// names were published atomically; orphan temporary files remain partial.
    pub fn recover(&mut self) -> Result<(), Error> {
        self.used = 0;
        self.entries.clear();
        let mut directories = vec![(self.root.clone(), 0)];
        while let Some((directory, depth)) = directories.pop() {
            for entry in fs::read_dir(directory).map_err(io_error)? {
                let entry = entry.map_err(io_error)?;
                let kind = entry.file_type().map_err(io_error)?;
                if kind.is_dir() {
                    if depth >= 8 { return Err(Error::new("E_OUTPUT_LIMIT", 8, "artifact directory depth exceeds 8")); }
                    directories.push((entry.path(), depth + 1));
                    continue;
                }
                if !kind.is_file() { return Err(Error::new("E_OUTPUT_IO", 8, "non-regular file in owned run directory")); }
                if self.entries.len() >= 100_000 { return Err(Error::new("E_OUTPUT_LIMIT", 8, "artifact inventory exceeds 100000 files")); }
                let path = entry.path();
                let name = path.strip_prefix(&self.root).expect("owned directory traversal").to_str()
                    .ok_or_else(|| Error::new("E_OUTPUT_IO", 8, "artifact path is not UTF-8"))?;
                let mut file = fs::File::open(&path).map_err(io_error)?;
                let mut hash = Sha256::new();
                let mut length = 0_u64;
                let mut chunk = [0_u8; 8192];
                loop {
                    let count = file.read(&mut chunk).map_err(io_error)?;
                    if count == 0 { break; }
                    self.reserve(count)?;
                    self.used += count as u64;
                    length += count as u64;
                    hash.update(&chunk[..count]);
                }
                let temporary = entry.file_name().to_str().is_some_and(|name| name.starts_with(".preview-") && name.ends_with(".tmp"));
                let status = if temporary { "partial" } else if name == OWNER_FILE { "complete" } else { "historical" };
                self.entries.push(json!({"path":name,"role":"recovered-artifact","status":status,"bytes":length,"digest":format!("sha256:{:x}",hash.finalize())}));
            }
        }
        self.entries.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
        Ok(())
    }

    pub fn root(&self) -> &Path { &self.root }

    pub fn into_state(self) -> OutputState {
        OutputState { entries: self.entries, used: self.used }
    }

    pub fn adopt(&mut self, state: OutputState) -> Result<(), Error> {
        if state.used > self.limit {
            return Err(Error::new("E_OUTPUT_LIMIT", 8, "worker exceeded artifact byte limit"));
        }
        self.entries = state.entries;
        self.used = state.used;
        Ok(())
    }

    pub fn json(&mut self, name: &str, role: &str, value: &Value) -> Result<(), Error> {
        let mut bytes = serde_json::to_vec_pretty(value)
            .map_err(|error| Error::new("E_OUTPUT_IO", 8, error.to_string()))?;
        bytes.push(b'\n');
        self.bytes(name, role, &bytes)
    }

    pub fn bytes(&mut self, name: &str, role: &str, bytes: &[u8]) -> Result<(), Error> {
        self.publish(name, bytes)?;
        self.entries.push(json!({"path": name, "role": role, "status": "complete", "bytes": bytes.len(), "digest": digest(bytes)}));
        Ok(())
    }

    pub fn finish(&mut self, result: &Value) -> Result<(), Error> {
        let mut result_bytes = serde_json::to_vec(result)
            .map_err(|error| Error::new("E_OUTPUT_IO", 8, error.to_string()))?;
        result_bytes.push(b'\n');
        self.entries.push(json!({"path":"result.json", "role":"result", "status":"complete", "bytes":result_bytes.len(), "digest":digest(&result_bytes)}));
        let inventory = json!({"schemaVersion":1,"artifacts":self.entries,"self":"artifacts.json"});
        let mut inventory_bytes = serde_json::to_vec_pretty(&inventory)
            .map_err(|error| Error::new("E_OUTPUT_IO", 8, error.to_string()))?;
        inventory_bytes.push(b'\n');
        self.reserve(inventory_bytes.len().saturating_add(result_bytes.len()))?;
        self.publish("artifacts.json", &inventory_bytes)?;
        self.publish("result.json", &result_bytes)
    }

    fn reserve(&self, bytes: usize) -> Result<(), Error> {
        if self.used.checked_add(bytes as u64).is_none_or(|total| total > self.limit) {
            return Err(Error::new("E_OUTPUT_LIMIT", 8, "run artifact byte limit exceeded"));
        }
        Ok(())
    }

    fn publish(&mut self, name: &str, bytes: &[u8]) -> Result<(), Error> {
        self.reserve(bytes.len())?;
        let relative = Path::new(name);
        if name.is_empty() || name.contains('\\') || name.split('/').any(|part| part.is_empty() || part == "." || part == "..")
            || relative.components().any(|part| !matches!(part, Component::Normal(_))) {
            return Err(Error::new("E_OUTPUT_IO", 8, "artifact path must be canonical and relative"));
        }
        let path = self.root.join(relative);
        let mut parent = self.root.clone();
        if let Some(ancestors) = relative.parent() {
            for component in ancestors.components() {
                parent.push(component);
                match fs::symlink_metadata(&parent) {
                    Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
                    Ok(_) => return Err(Error::new("E_OUTPUT_IO", 8, "artifact parent is not a real directory")),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir(&parent).map_err(io_error)?,
                    Err(error) => return Err(io_error(error)),
                }
            }
        }
        if fs::symlink_metadata(&path).is_ok() {
            return Err(Error::new("E_OUTPUT_IO", 8, format!("artifact already exists: {name}")));
        }
        let temporary = parent.join(format!(".preview-{}-{}.tmp", std::process::id(), self.entries.len()));
        let mut file = OpenOptions::new().write(true).create_new(true).open(&temporary).map_err(io_error)?;
        // Linking a complete same-directory temporary is atomic and never
        // replaces an existing name. Plain rename would overwrite a racing run.
        let result = file.write_all(bytes).and_then(|()| file.sync_all())
            .and_then(|()| fs::hard_link(&temporary, &path))
            .and_then(|()| fs::remove_file(&temporary));
        if let Err(error) = result {
            let _ = fs::remove_file(&temporary);
            return Err(io_error(error));
        }
        self.used += bytes.len() as u64;
        Ok(())
    }
}

fn io_error(error: std::io::Error) -> Error { Error::new("E_OUTPUT_IO", 8, error.to_string()) }

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn directory() -> PathBuf {
        std::env::temp_dir().join(format!("zgui-preview-output-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)))
    }
    #[test]
    fn refuses_stale_outputs_and_never_publishes_over_budget_result() {
        let root = directory();
        let mut output = Output::create(&root, 20).unwrap();
        output.bytes("captures/small.rgba", "raw", &[1, 2, 3]).unwrap();
        assert_eq!(output.finish(&json!({"status":"passed"})).unwrap_err().code, "E_OUTPUT_LIMIT");
        assert!(!root.join("result.json").exists());
        assert!(Output::create(&root, 1000).is_err());
        assert_eq!(fs::read(root.join("captures/small.rgba")).unwrap(), [1, 2, 3]);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_paths_and_symlinked_artifact_parents() {
        let root = directory();
        let mut output = Output::create(&root, 1000).unwrap();
        for name in ["../escape", "/absolute", "captures/../escape", "captures//escape"] {
            assert_eq!(output.bytes(name, "test", b"x").unwrap_err().code, "E_OUTPUT_IO");
        }
        #[cfg(unix)] {
            std::os::unix::fs::symlink(std::env::temp_dir(), root.join("captures")).unwrap();
            assert_eq!(output.bytes("captures/escape", "test", b"x").unwrap_err().code, "E_OUTPUT_IO");
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn only_one_concurrent_controller_can_claim_a_run() {
        let root = directory();
        fs::create_dir(&root).unwrap();
        let start = std::sync::Arc::new(std::sync::Barrier::new(3));
        let threads: Vec<_> = (0..2).map(|_| {
            let root = root.clone();
            let start = start.clone();
            std::thread::spawn(move || { start.wait(); Output::create(&root, 10_000).is_ok() })
        }).collect();
        start.wait();
        let winners = threads.into_iter().map(|thread| usize::from(thread.join().unwrap())).sum::<usize>();
        assert_eq!(winners, 1);
        assert_eq!(fs::read(root.join(OWNER_FILE)).unwrap(), format!("{}\n", std::process::id()).as_bytes());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lost_worker_report_preserves_historical_files_and_budget() {
        let root = directory();
        let mut controller = Output::create(&root, 8_100).unwrap();
        let mut worker = Output::attach_worker(&root, 8_100, std::process::id()).unwrap();
        worker.bytes("captures/frame.rgba", "pixels", &[42; 8_000]).unwrap();
        drop(worker);
        controller.recover().unwrap();
        let recovered = controller.entries.iter().find(|entry| entry["path"] == "captures/frame.rgba").unwrap();
        assert_eq!(recovered["status"], "historical");
        assert_eq!(recovered["digest"], digest(&[42; 8_000]));
        assert_eq!(controller.finish(&json!({"status":"failed","exitCode":5})).unwrap_err().code, "E_OUTPUT_LIMIT");
        assert!(!root.join("result.json").exists());
        assert_eq!(fs::read(root.join("captures/frame.rgba")).unwrap(), [42; 8_000]);
        fs::remove_dir_all(root).unwrap();
    }
}
