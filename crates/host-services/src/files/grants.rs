//! Per-app persistent handle bookmark store at `$XDG_STATE_HOME/zgui-host/grants/<appId>.json`.
#![forbid(unsafe_code)]

use std::{
    env,
    fs::{self, File},
    io::Read,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    ServiceError,
    files::{FileMode, HandleKind, atomic_write_bytes, map_io_error},
    sqlite::is_safe_app_id,
};

const MAX_BOOKMARKS_PER_APP: usize = 256;
static TOKEN_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Serialized persistent grant record isolated per `appId`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkRecord {
    pub token: String,
    pub kind: HandleKind,
    pub path: String,
    pub label: String,
    pub mode: FileMode,
    pub created_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrantFile {
    schema_version: u32,
    bookmarks: Vec<BookmarkRecord>,
}

/// Resolves `$XDG_STATE_HOME/zgui-host/grants/<appId>.json`.
pub fn resolve_grant_store_path(app_id: &str) -> Result<PathBuf, ServiceError> {
    if !is_safe_app_id(app_id) {
        return Err(ServiceError::new(
            "InvalidPath",
            format!("invalid appId for grant store: {app_id:?}"),
        ));
    }
    let state_home = if let Ok(xdg) = env::var("XDG_STATE_HOME")
        && !xdg.is_empty()
        && Path::new(&xdg).is_absolute()
    {
        PathBuf::from(xdg)
    } else if let Ok(home) = env::var("HOME")
        && !home.is_empty()
        && Path::new(&home).is_absolute()
    {
        PathBuf::from(home).join(".local/state")
    } else {
        return Err(ServiceError::new(
            "InvalidState",
            "neither XDG_STATE_HOME nor HOME is set to an absolute path",
        ));
    };
    Ok(state_home
        .join("zgui-host/grants")
        .join(format!("{app_id}.json")))
}

/// Generates an opaque 32-character hex bookmark token.
pub fn generate_bookmark_token(app_id: &str, canonical_path: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(app_id.as_bytes());
    hasher.update(b":");
    hasher.update(canonical_path.as_bytes());
    hasher.update(b":");
    hasher.update(TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    let mut entropy = [0u8; 16];
    if let Ok(mut f) = File::open("/dev/urandom") {
        let _ = f.read_exact(&mut entropy);
    }
    hasher.update(entropy);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(32);
    for byte in &digest[..16] {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn load_grant_file(store_path: &Path) -> Result<GrantFile, ServiceError> {
    match fs::read(store_path) {
        Ok(bytes) => {
            let parsed: GrantFile = serde_json::from_slice(&bytes)
                .map_err(|err| ServiceError::new("Io", format!("corrupted grant store: {err}")))?;
            Ok(parsed)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(GrantFile {
            schema_version: 1,
            bookmarks: Vec::new(),
        }),
        Err(err) => Err(map_io_error(err)),
    }
}

fn save_grant_file(store_path: &Path, data: &GrantFile) -> Result<(), ServiceError> {
    if let Some(parent) = store_path.parent() {
        fs::create_dir_all(parent).map_err(map_io_error)?;
        let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
    }
    let bytes = serde_json::to_vec_pretty(data)
        .map_err(|err| ServiceError::new("Io", err.to_string()))?;
    atomic_write_bytes(store_path, &bytes, Some(0o600))
}

/// Persists a bookmark for `canonical_path` in `store_path`, returning its opaque token.
pub fn persist_grant(
    store_path: &Path,
    app_id: &str,
    kind: HandleKind,
    canonical_path: &Path,
    mode: FileMode,
) -> Result<BookmarkRecord, ServiceError> {
    let mut file = load_grant_file(store_path)?;
    let path_str = canonical_path.to_string_lossy().into_owned();
    if let Some(existing) = file
        .bookmarks
        .iter_mut()
        .find(|b| b.kind == kind && b.path == path_str)
    {
        if mode.can_write() {
            existing.mode = FileMode::ReadWrite;
        }
        let cloned = existing.clone();
        save_grant_file(store_path, &file)?;
        return Ok(cloned);
    }
    if file.bookmarks.len() >= MAX_BOOKMARKS_PER_APP {
        return Err(ServiceError::new(
            "E_LIMIT",
            format!("bookmark limit ({MAX_BOOKMARKS_PER_APP}) reached for app"),
        ));
    }
    let label = canonical_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".to_owned());
    let record = BookmarkRecord {
        token: generate_bookmark_token(app_id, &path_str),
        kind,
        path: path_str,
        label,
        mode,
        created_ms: now_ms(),
    };
    file.schema_version = 1;
    file.bookmarks.push(record.clone());
    save_grant_file(store_path, &file)?;
    Ok(record)
}

/// Restores a previously persisted bookmark by opaque token.
pub fn restore_grant(store_path: &Path, token: &str) -> Result<BookmarkRecord, ServiceError> {
    let file = load_grant_file(store_path)?;
    file.bookmarks
        .into_iter()
        .find(|b| b.token == token)
        .ok_or_else(|| ServiceError::new("NotFound", "bookmark token not found or revoked"))
}

/// Lists all persisted bookmarks for this app.
pub fn list_grants(store_path: &Path) -> Result<Vec<BookmarkRecord>, ServiceError> {
    Ok(load_grant_file(store_path)?.bookmarks)
}

/// Revokes a bookmark by opaque token, returning `true` if it existed.
pub fn revoke_grant(store_path: &Path, token: &str) -> Result<bool, ServiceError> {
    let mut file = load_grant_file(store_path)?;
    let before = file.bookmarks.len();
    file.bookmarks.retain(|b| b.token != token);
    let removed = file.bookmarks.len() != before;
    if removed {
        save_grant_file(store_path, &file)?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmark_store_isolates_apps_and_supports_roundtrip_and_revoke() {
        let dir = env::temp_dir().join(format!(
            "zgui-grants-test-{}-{}",
            std::process::id(),
            TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let app_a = dir.join("app-a.json");
        let app_b = dir.join("app-b.json");
        let target = Path::new("/tmp/workspace-demo");

        let rec = persist_grant(&app_a, "app-a", HandleKind::Dir, target, FileMode::ReadWrite)
            .expect("persist grant");
        assert_eq!(rec.token.len(), 32);
        assert_eq!(rec.label, "workspace-demo");

        // App A can restore and list it.
        let restored = restore_grant(&app_a, &rec.token).expect("restore grant");
        assert_eq!(restored, rec);
        assert_eq!(list_grants(&app_a).unwrap().len(), 1);

        // App B cannot restore App A's token.
        assert!(restore_grant(&app_b, &rec.token).is_err());

        // Revoking removes it cleanly.
        assert!(revoke_grant(&app_a, &rec.token).unwrap());
        assert!(!revoke_grant(&app_a, &rec.token).unwrap());
        assert!(restore_grant(&app_a, &rec.token).is_err());

        let _ = fs::remove_dir_all(&dir);
    }
}
