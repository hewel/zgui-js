//! Verified persistent QuickJS bytecode cache (`.zsqb`) keyed by ABI, canonical ID, source SHA-256, and import graph digest.
use std::{
    fs::{self, DirBuilder, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

use sha2::{Digest, Sha256};

pub(crate) const ZSQB_MAGIC: &[u8; 4] = b"ZSQB";
pub(crate) const ZSQB_FORMAT_VERSION: u32 = 1;
pub(crate) const DEFAULT_MAX_CACHE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CACHE_ENTRY_BYTES: u64 = 16 * 1024 * 1024;

static NEXT_TMP_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub(crate) struct BytecodeCache {
    dir: PathBuf,
    max_bytes: u64,
}

/// Computes the 4-tuple SHA-256 cache key isolating ABI, canonical ID, source bytes, and import graph.
#[must_use]
pub(crate) fn compute_cache_key(
    quickjs_abi_tag: &str,
    canonical_module_name: &str,
    module_sha256: &str,
    resolved_import_graph_digest: &str,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(quickjs_abi_tag.as_bytes());
    hasher.update([0]);
    hasher.update(canonical_module_name.as_bytes());
    hasher.update([0]);
    hasher.update(module_sha256.as_bytes());
    hasher.update([0]);
    hasher.update(resolved_import_graph_digest.as_bytes());
    hasher.finalize().into()
}

fn hex_encode(bytes: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(64);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn current_euid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and always succeeds on Linux/POSIX.
    unsafe { libc::geteuid() }
}

fn ensure_private_dir(dir: &Path) -> Result<PathBuf, String> {
    if !dir.exists() {
        let mut builder = DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder
            .create(dir)
            .map_err(|e| format!("failed to create bytecode cache dir {}: {e}", dir.display()))?;
    }
    let meta = fs::symlink_metadata(dir)
        .map_err(|e| format!("failed to stat bytecode cache dir {}: {e}", dir.display()))?;
    if !meta.file_type().is_dir() {
        return Err(format!("bytecode cache path {} is not a real directory", dir.display()));
    }
    if meta.uid() != current_euid() {
        return Err(format!("bytecode cache dir {} is not owned by current user", dir.display()));
    }
    if meta.mode() & 0o022 != 0 {
        return Err(format!(
            "bytecode cache dir {} has unsafe group/world write permissions",
            dir.display()
        ));
    }
    fs::canonicalize(dir)
        .map_err(|e| format!("failed to canonicalize bytecode cache dir {}: {e}", dir.display()))
}

impl BytecodeCache {
    pub(crate) fn open(dir: impl AsRef<Path>, max_bytes: u64) -> Result<Self, String> {
        let canonical = ensure_private_dir(dir.as_ref())?;
        Ok(Self {
            dir: canonical,
            max_bytes,
        })
    }

    pub(crate) fn entry_path(&self, key: &[u8; 32]) -> PathBuf {
        self.dir.join(format!("{}.zsqb", hex_encode(key)))
    }

    /// Reads, verifies (`ZSQB` header, ABI tag, 4-tuple key, payload SHA-256), and returns cached bytecode.
    /// Any corrupt or untrusted entry is unlinked immediately and `None` is returned for source fallback.
    pub(crate) fn get(&self, expected_abi: &str, key: &[u8; 32]) -> Option<Vec<u8>> {
        let path = self.entry_path(key);
        let mut file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
        {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
            Err(err) => {
                eprintln!("bytecode-cache-invalid: open {} failed ({err}), removing", path.display());
                let _ = fs::remove_file(&path);
                return None;
            }
        };

        let meta = match file.metadata() {
            Ok(meta) => meta,
            Err(err) => {
                eprintln!("bytecode-cache-invalid: stat {} failed ({err}), removing", path.display());
                let _ = fs::remove_file(&path);
                return None;
            }
        };
        if !meta.file_type().is_file()
            || meta.uid() != current_euid()
            || (meta.mode() & 0o077 != 0)
            || meta.len() > MAX_CACHE_ENTRY_BYTES
        {
            eprintln!("bytecode-cache-invalid: metadata check failed for {}, removing", path.display());
            let _ = fs::remove_file(&path);
            return None;
        }

        let mut bytes = Vec::with_capacity(meta.len() as usize);
        if let Err(err) = file.read_to_end(&mut bytes) {
            eprintln!("bytecode-cache-invalid: read {} failed ({err}), removing", path.display());
            let _ = fs::remove_file(&path);
            return None;
        }

        let Some(payload) = verify_zsqb_record(&bytes, expected_abi, key) else {
            eprintln!("bytecode-cache-invalid: header/digest verification failed for {}, removing", path.display());
            let _ = fs::remove_file(&path);
            return None;
        };

        let _ = file.set_modified(SystemTime::now());
        Some(payload)
    }

    /// Atomically writes a verified `ZSQB` record with mode `0600`, `fsync`, rename, and LRU eviction.
    pub(crate) fn put(&self, abi_tag: &str, key: &[u8; 32], payload: &[u8]) {
        if payload.is_empty() || payload.len() as u64 > self.max_bytes.max(MAX_CACHE_ENTRY_BYTES) {
            return;
        }
        let Ok(record) = encode_zsqb_record(abi_tag, key, payload) else {
            return;
        };
        if record.len() as u64 > MAX_CACHE_ENTRY_BYTES {
            return;
        }
        let seq = NEXT_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp_path = self.dir.join(format!(".tmp-{}-{seq}.zsqb", std::process::id()));
        let write_res = (|| -> std::io::Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp_path)?;
            file.write_all(&record)?;
            file.sync_all()?;
            Ok(())
        })();
        if let Err(err) = write_res {
            eprintln!("bytecode-cache-write-error: {}: {err}", tmp_path.display());
            let _ = fs::remove_file(&tmp_path);
            return;
        }
        let final_path = self.entry_path(key);
        if let Err(err) = fs::rename(&tmp_path, &final_path) {
            eprintln!("bytecode-cache-rename-error: {}: {err}", final_path.display());
            let _ = fs::remove_file(&tmp_path);
            return;
        }
        self.evict_lru();
    }

    fn evict_lru(&self) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut records = Vec::new();
        let mut total_bytes: u64 = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("zsqb") {
                continue;
            }
            let name = entry.file_name();
            if name.to_str().is_some_and(|n| n.starts_with('.')) {
                continue;
            }
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if !meta.file_type().is_file() {
                continue;
            }
            let len = meta.len();
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            total_bytes = total_bytes.saturating_add(len);
            records.push((modified, path, len));
        }
        if total_bytes <= self.max_bytes {
            return;
        }
        records.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        for (_, path, len) in records {
            if total_bytes <= self.max_bytes {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total_bytes = total_bytes.saturating_sub(len);
            }
        }
    }
}

fn encode_zsqb_record(abi_tag: &str, key: &[u8; 32], payload: &[u8]) -> Result<Vec<u8>, String> {
    let abi_bytes = abi_tag.as_bytes();
    let abi_len = u16::try_from(abi_bytes.len()).map_err(|_| "ABI tag exceeds u16 length".to_owned())?;
    let payload_len =
        u32::try_from(payload.len()).map_err(|_| "bytecode payload exceeds u32 length".to_owned())?;
    let payload_sha: [u8; 32] = Sha256::digest(payload).into();

    let mut out = Vec::with_capacity(4 + 4 + 2 + abi_bytes.len() + 32 + 4 + 32 + payload.len());
    out.extend_from_slice(ZSQB_MAGIC);
    out.extend_from_slice(&ZSQB_FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&abi_len.to_le_bytes());
    out.extend_from_slice(abi_bytes);
    out.extend_from_slice(key);
    out.extend_from_slice(&payload_len.to_le_bytes());
    out.extend_from_slice(&payload_sha);
    out.extend_from_slice(payload);
    Ok(out)
}

fn verify_zsqb_record(bytes: &[u8], expected_abi: &str, expected_key: &[u8; 32]) -> Option<Vec<u8>> {
    let mut cursor = 0usize;
    let take = |len: usize, cursor: &mut usize| -> Option<&[u8]> {
        let end = cursor.checked_add(len)?;
        let slice = bytes.get(*cursor..end)?;
        *cursor = end;
        Some(slice)
    };
    if take(4, &mut cursor)? != ZSQB_MAGIC {
        return None;
    }
    let version = u32::from_le_bytes(take(4, &mut cursor)?.try_into().ok()?);
    if version != ZSQB_FORMAT_VERSION {
        return None;
    }
    let abi_len = usize::from(u16::from_le_bytes(take(2, &mut cursor)?.try_into().ok()?));
    let abi_bytes = take(abi_len, &mut cursor)?;
    if abi_bytes != expected_abi.as_bytes() {
        return None;
    }
    let key = take(32, &mut cursor)?;
    if key != expected_key {
        return None;
    }
    let payload_len = usize::try_from(u32::from_le_bytes(take(4, &mut cursor)?.try_into().ok()?)).ok()?;
    let expected_sha = take(32, &mut cursor)?;
    let payload = take(payload_len, &mut cursor)?;
    if cursor != bytes.len() || payload.is_empty() {
        return None;
    }
    let actual_sha: [u8; 32] = Sha256::digest(payload).into();
    if actual_sha.as_slice() != expected_sha {
        return None;
    }
    Some(payload.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn temp_cache_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "zgui-bcache-{label}-{}-{}",
            std::process::id(),
            NEXT_TMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn round_trip_four_tuple_key_and_corrupt_cache_unlink_fallback() {
        let dir = temp_cache_dir("roundtrip");
        let cache = BytecodeCache::open(&dir, DEFAULT_MAX_CACHE_BYTES).expect("open cache");
        let abi_string = js_runtime::quickjs_abi_tag();
        let abi = abi_string.as_str();
        let k1 = compute_cache_key(abi, "zgui-ext://ext-a/dist/a.mjs", "sha256:1111", "sha256:aaaa");
        let k2 = compute_cache_key(abi, "zgui-ext://ext-a/dist/a.mjs", "sha256:1111", "sha256:bbbb");
        assert_ne!(k1, k2, "changing resolved_import_graph_digest must change cache key");
        let k3 = compute_cache_key("different-abi", "zgui-ext://ext-a/dist/a.mjs", "sha256:1111", "sha256:aaaa");
        assert_ne!(k1, k3, "changing ABI tag must change cache key");

        let payload = b"verified-bytecode-bytes".to_vec();
        cache.put(abi, &k1, &payload);
        let entry = cache.entry_path(&k1);
        assert_eq!(fs::metadata(&entry).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(cache.get(abi, &k1).as_deref(), Some(payload.as_slice()));
        assert!(cache.get("wrong-abi", &k1).is_none());
        // Wrong ABI unlinked the mismatching file safely.
        assert!(!entry.exists());

        // Corrupt payload triggers removal + None fallback.
        cache.put(abi, &k1, &payload);
        let mut raw = fs::read(&entry).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0xff;
        fs::write(&entry, raw).unwrap();
        assert!(cache.get(abi, &k1).is_none());
        assert!(!entry.exists(), "corrupt cache entry must be unlinked");

        // Unsafe file mode triggers removal + None fallback.
        cache.put(abi, &k1, &payload);
        fs::set_permissions(&entry, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(cache.get(abi, &k1).is_none());
        assert!(!entry.exists(), "permissive mode cache entry must be rejected and unlinked");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn evicts_least_recently_used_entries_above_budget() {
        let dir = temp_cache_dir("lru");
        // Each header is ~100 bytes; 300-byte payload gives ~400 bytes/entry.
        // Budget 900 bytes retains 2 entries cleanly and evicts oldest on 3rd insert.
        let cache = BytecodeCache::open(&dir, 900).expect("open small budget cache");
        let abi = "test-abi";
        let payload = vec![0xAB; 300];
        let k1 = compute_cache_key(abi, "m1", "s1", "g1");
        let k2 = compute_cache_key(abi, "m2", "s2", "g2");
        let k3 = compute_cache_key(abi, "m3", "s3", "g3");

        cache.put(abi, &k1, &payload);
        std::thread::sleep(std::time::Duration::from_millis(20));
        cache.put(abi, &k2, &payload);
        std::thread::sleep(std::time::Duration::from_millis(20));
        // Touch k1 so k2 becomes the LRU victim.
        assert!(cache.get(abi, &k1).is_some());
        std::thread::sleep(std::time::Duration::from_millis(20));
        cache.put(abi, &k3, &payload);

        assert!(cache.entry_path(&k1).exists(), "recently accessed k1 must survive");
        assert!(cache.entry_path(&k3).exists(), "newly inserted k3 must survive");
        assert!(!cache.entry_path(&k2).exists(), "oldest unaccessed k2 must be evicted");

        let _ = fs::remove_dir_all(&dir);
    }
}
