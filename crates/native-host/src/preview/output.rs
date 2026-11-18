use super::Error;
use crate::bounded_fs::{self, BoundedReadError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs::{self, OpenOptions}, io::{Read, Write}, path::{Path, PathBuf}};

pub(crate) fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub(crate) fn json_bytes(value: &Value) -> Result<Vec<u8>, Error> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| Error::new("E_OUTPUT_IO", 8, error.to_string()))?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(super) fn relative_path(name: &str) -> Result<&Path, Error> {
    bounded_fs::relative_path(name)
        .map_err(|_| Error::new("E_OUTPUT_IO", 8, "artifact path must be canonical and relative"))
}

const OWNER_FILE: &str = ".preview-owner";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ArtifactStatus {
    Complete,
    Historical,
    Partial,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ArtifactEntry {
    pub path: String,
    pub role: String,
    pub status: ArtifactStatus,
    pub bytes: u64,
    pub digest: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactInventory<'a> {
    schema_version: u32,
    artifacts: &'a [ArtifactEntry],
    #[serde(rename = "self")]
    self_path: &'static str,
}

fn compact_original_error(cause: Value) -> Value {
    let Some(object) = cause.as_object() else { return cause; };
    let mut compacted = serde_json::Map::new();
    for key in ["code", "exitCode", "stage", "step", "actionSequence"] {
        if let Some(value) = object.get(key) {
            compacted.insert(key.to_owned(), value.clone());
        }
    }
    if let Some(message) = object.get("message").and_then(Value::as_str) {
        compacted.insert("message".to_owned(), Value::String(message.chars().take(512).collect()));
    }
    Value::Object(compacted)
}

pub(crate) fn publication_failure(command: &str, error: Error, previous: &Value) -> Value {
    let mut failed = super::failure(command, error);
    failed["originalError"] = if previous["originalError"].is_null() {
        previous["firstError"].clone()
    } else {
        previous["originalError"].clone()
    };
    if serde_json::to_vec(&failed).map_or(true, |bytes| bytes.len() + 1 > super::MAX_STDOUT_BYTES) {
        failed["originalError"] = compact_original_error(failed["originalError"].take());
        if serde_json::to_vec(&failed).map_or(true, |bytes| bytes.len() + 1 > super::MAX_STDOUT_BYTES) {
            failed["originalError"] = serde_json::json!({
                "code": failed["originalError"]["code"],
                "exitCode": failed["originalError"]["exitCode"],
            });
        }
    }
    failed
}

pub(super) fn bounded_read_error(error: BoundedReadError, limit_message: &str) -> Error {
    match error {
        BoundedReadError::TooLarge { .. } | BoundedReadError::NotRegularFile => {
            Error::new("E_OUTPUT_LIMIT", 8, limit_message)
        }
        other => io_error(other),
    }
}

pub(crate) fn enforce_stdout_limit(command: &str, result: Value) -> Value {
    let exceeds = serde_json::to_vec(&result)
        .map_or(true, |bytes| bytes.len() + 1 > super::MAX_STDOUT_BYTES);
    if exceeds {
        publication_failure(
            command,
            Error::new("E_OUTPUT_LIMIT", 8, format!("{command} summary exceeds 8 KiB")),
            &result,
        )
    } else {
        result
    }
}

pub(crate) struct Output {
    root: PathBuf,
    limit: u64,
    used: u64,
    entries: Vec<ArtifactEntry>,
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
        let owner = bounded_fs::read_contained(&root, OWNER_FILE, 16)
            .map_err(|_| Error::new("E_OUTPUT_IO", 8, "invalid output ownership record"))?;
        if owner != format!("{parent_pid}\n").as_bytes() {
            return Err(Error::new("E_OUTPUT_IO", 8, "output belongs to a different controller"));
        }
        let output = Self {
            root, limit, used: owner.len() as u64,
            entries: vec![ArtifactEntry {
                path: OWNER_FILE.into(),
                role: "run-ownership".into(),
                status: ArtifactStatus::Complete,
                bytes: owner.len() as u64,
                digest: digest(&owner),
            }],
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
                let (mut file, _) = bounded_fs::open_contained(&self.root, name, self.limit)
                    .map_err(|error| bounded_read_error(error, "run artifact byte limit exceeded"))?;
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
                let status = if temporary {
                    ArtifactStatus::Partial
                } else if name == OWNER_FILE {
                    ArtifactStatus::Complete
                } else {
                    ArtifactStatus::Historical
                };
                self.entries.push(ArtifactEntry {
                    path: name.to_owned(),
                    role: "recovered-artifact".into(),
                    status,
                    bytes: length,
                    digest: format!("sha256:{:x}", hash.finalize()),
                });
            }
        }
        self.entries.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(())
    }

    pub fn root(&self) -> &Path { &self.root }

    /// Keep the control pipe bounded independently of the number of artifacts.
    pub fn seal_worker(&mut self) -> Result<(), Error> {
        let bytes = serde_json::to_vec(&self.entries).map_err(|error| Error::new("E_OUTPUT_IO", 8, error.to_string()))?;
        self.bytes("worker-artifacts.json", "worker-inventory", &bytes)
    }

    /// Rehash completed files after the worker exits; accept its roles only when
    /// every declared identity matches the actual exclusively owned directory.
    pub fn restore_worker(&mut self) -> Result<(), Error> {
        self.recover()?;
        let mut seal = self.entries.iter().find(|entry| entry.path == "worker-artifacts.json")
            .ok_or_else(|| Error::new("E_OUTPUT_IO", 8, "worker inventory was not published"))?.clone();
        let bytes = bounded_fs::read_contained(&self.root, "worker-artifacts.json", self.limit)
            .map_err(|error| bounded_read_error(error, "run artifact byte limit exceeded"))?;
        let mut entries: Vec<ArtifactEntry> = serde_json::from_slice(&bytes)
            .map_err(|error| Error::new("E_OUTPUT_IO", 8, format!("invalid worker inventory: {error}")))?;
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        let recovered: Vec<_> = self.entries.iter().filter(|entry| entry.path != "worker-artifacts.json").collect();
        if entries.len() != recovered.len()
            || entries.iter().zip(recovered).any(|(declared, actual)| {
                declared.path != actual.path
                    || declared.digest != actual.digest
                    || declared.bytes != actual.bytes
                    || declared.status != ArtifactStatus::Complete
            })
        {
            return Err(Error::new("E_OUTPUT_IO", 8, "worker inventory does not match retained artifact identities"));
        }
        seal.role = "worker-inventory".into();
        seal.status = ArtifactStatus::Complete;
        entries.push(seal);
        self.entries = entries;
        Ok(())
    }

    pub fn json(&mut self, name: &str, role: &str, value: &Value) -> Result<(), Error> {
        self.bytes(name, role, &json_bytes(value)?)
    }

    pub fn bytes(&mut self, name: &str, role: &str, bytes: &[u8]) -> Result<(), Error> {
        self.publish(name, bytes)?;
        self.entries.push(ArtifactEntry {
            path: name.into(),
            role: role.into(),
            status: ArtifactStatus::Complete,
            bytes: bytes.len() as u64,
            digest: digest(bytes),
        });
        Ok(())
    }

    pub fn finish(&mut self, result: &Value) -> Result<(), Error> {
        let mut result_bytes = serde_json::to_vec(result)
            .map_err(|error| Error::new("E_OUTPUT_IO", 8, error.to_string()))?;
        result_bytes.push(b'\n');
        self.entries.push(ArtifactEntry {
            path: "result.json".into(),
            role: "result".into(),
            status: ArtifactStatus::Complete,
            bytes: result_bytes.len() as u64,
            digest: digest(&result_bytes),
        });
        let inventory = ArtifactInventory {
            schema_version: 1,
            artifacts: &self.entries,
            self_path: "artifacts.json",
        };
        let mut inventory_bytes = serde_json::to_vec_pretty(&inventory)
            .map_err(|error| Error::new("E_OUTPUT_IO", 8, error.to_string()))?;
        inventory_bytes.push(b'\n');
        self.reserve(inventory_bytes.len().saturating_add(result_bytes.len()))?;
        self.publish("artifacts.json", &inventory_bytes)?;
        self.publish("result.json", &result_bytes)
    }

    /// Enforces the 8 KiB stdout budget, publishes `artifacts.json` and `result.json`,
    /// and retains the earliest cause under `originalError` if finalization fails.
    pub fn finalize(&mut self, command: &str, result: Value) -> Value {
        let mut result = enforce_stdout_limit(command, result);
        if let Err(error) = self.finish(&result) {
            result = publication_failure(command, error, &result);
        }
        result
    }

    fn reserve(&self, bytes: usize) -> Result<(), Error> {
        if self.used.checked_add(bytes as u64).is_none_or(|total| total > self.limit) {
            return Err(Error::new("E_OUTPUT_LIMIT", 8, "run artifact byte limit exceeded"));
        }
        Ok(())
    }

    fn publish(&mut self, name: &str, bytes: &[u8]) -> Result<(), Error> {
        self.reserve(bytes.len())?;
        let relative = relative_path(name)?;
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

fn io_error(error: impl fmt::Display) -> Error { Error::new("E_OUTPUT_IO", 8, error.to_string()) }

use std::fmt;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
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
        let recovered = controller.entries.iter().find(|entry| entry.path == "captures/frame.rgba").unwrap();
        assert_eq!(recovered.status, ArtifactStatus::Historical);
        assert_eq!(recovered.digest, digest(&[42; 8_000]));
        assert_eq!(controller.finish(&json!({"status":"failed","exitCode":5})).unwrap_err().code, "E_OUTPUT_LIMIT");
        assert!(!root.join("result.json").exists());
        assert_eq!(fs::read(root.join("captures/frame.rgba")).unwrap(), [42; 8_000]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn seal_worker_and_restore_worker_preserve_typed_roles() {
        let root = directory();
        let mut controller = Output::create(&root, 10_000).unwrap();
        let mut worker = Output::attach_worker(&root, 10_000, std::process::id()).unwrap();
        worker.json("environment.json", "environment", &json!({"ok": true})).unwrap();
        worker.seal_worker().unwrap();
        drop(worker);
        controller.restore_worker().unwrap();
        let finalized = controller.finalize("doctor", json!({
            "schemaVersion": 1, "command": "doctor", "status": "passed", "exitCode": 0, "firstError": null
        }));
        assert_eq!(finalized["exitCode"], 0);
        let inventory: Value = serde_json::from_slice(&fs::read(root.join("artifacts.json")).unwrap()).unwrap();
        assert_eq!(inventory["schemaVersion"], 1);
        assert_eq!(inventory["self"], "artifacts.json");
        let env_entry = inventory["artifacts"].as_array().unwrap().iter()
            .find(|entry| entry["path"] == "environment.json").unwrap();
        assert_eq!(env_entry["role"], "environment");
        assert_eq!(env_entry["status"], "complete");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn finalize_enforces_stdout_limit_seals_output_and_preserves_original_error() {
        let root = directory();
        let mut output = Output::create(&root, 64 * 1024).unwrap();
        let cause = json!({"code": "E_VISUAL_DIFF", "exitCode": 4, "message": "x".repeat(super::super::MAX_STDOUT_BYTES)});
        let result = output.finalize("compare", json!({
            "schemaVersion": 1, "command": "compare", "status": "failed", "exitCode": 4, "firstError": cause
        }));
        assert_eq!(result["exitCode"], 8);
        assert_eq!(result["firstError"]["code"], "E_OUTPUT_LIMIT");
        assert_eq!(result["originalError"]["code"], "E_VISUAL_DIFF");
        assert!(serde_json::to_vec(&result).unwrap().len() < super::super::MAX_STDOUT_BYTES);
        assert!(root.join("artifacts.json").is_file());
        let saved: Value = serde_json::from_slice(&fs::read(root.join("result.json")).unwrap()).unwrap();
        assert_eq!(saved["firstError"]["code"], "E_OUTPUT_LIMIT");
        assert_eq!(saved["originalError"]["code"], "E_VISUAL_DIFF");
        assert!(fs::read(root.join("result.json")).unwrap().len() <= super::super::MAX_STDOUT_BYTES);
        fs::remove_dir_all(root).unwrap();

        let tiny = directory();
        let mut tight = Output::create(&tiny, 64).unwrap();
        let tight_result = tight.finalize("compare", json!({
            "schemaVersion": 1, "command": "compare", "status": "failed", "exitCode": 4, "firstError": cause
        }));
        assert_eq!(tight_result["exitCode"], 8);
        assert_eq!(tight_result["firstError"]["code"], "E_OUTPUT_LIMIT");
        assert_eq!(tight_result["originalError"]["code"], "E_VISUAL_DIFF");
        assert!(serde_json::to_vec(&tight_result).unwrap().len() < super::super::MAX_STDOUT_BYTES);
        fs::remove_dir_all(tiny).unwrap();
    }
}
