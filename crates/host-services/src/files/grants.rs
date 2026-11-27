//! Per-app persistent filesystem grants at `$XDG_STATE_HOME/zgui-host/grants/<appId>.json`.
#![forbid(unsafe_code)]

use std::{
    env,
    fs::{self, File},
    io::Read,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
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

pub const MAX_GRANTS_PER_APP: usize = 256;
static TOKEN_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Persistent filesystem authority, distinct from the kind of live handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GrantKind {
    WorkspaceFile,
    WorkspaceDir,
    SystemDir,
}

impl From<HandleKind> for GrantKind {
    fn from(kind: HandleKind) -> Self {
        match kind {
            HandleKind::File => Self::WorkspaceFile,
            HandleKind::Dir => Self::WorkspaceDir,
        }
    }
}

impl GrantKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WorkspaceFile => "workspace-file",
            Self::WorkspaceDir => "workspace-dir",
            Self::SystemDir => "system-dir",
        }
    }

    pub const fn workspace_kind(self) -> Option<HandleKind> {
        match self {
            Self::WorkspaceFile => Some(HandleKind::File),
            Self::WorkspaceDir => Some(HandleKind::Dir),
            Self::SystemDir => None,
        }
    }
}

/// The active package's maximum authority for persisted workspace handles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkspaceGrantCeiling {
    Disabled,
    ReadOnly,
    #[default]
    ReadWrite,
}

impl WorkspaceGrantCeiling {
    /// Check authority before materializing a handle or persisting a new grant.
    pub fn authorize(self, kind: GrantKind, mode: FileMode) -> Result<HandleKind, ServiceError> {
        match kind.workspace_kind() {
            Some(kind)
                if match self {
                    Self::ReadWrite => true,
                    Self::ReadOnly => !mode.can_write(),
                    Self::Disabled => false,
                } =>
            {
                Ok(kind)
            }
            _ => Err(ServiceError::new(
                "E_CAPABILITY",
                "grant exceeds the package workspace grant capability",
            )),
        }
    }
}

/// Serialized persistent grant record isolated per `appId`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GrantRecord {
    pub token: String,
    pub kind: GrantKind,
    pub label: String,
    #[serde(rename = "accessMode")]
    pub mode: FileMode,
    pub created_ms: u64,
    #[serde(rename = "scopeMetadata")]
    pub scope: GrantScope,
}

/// Filesystem scope persisted with a grant; not exposed in the JS inventory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantScope {
    pub path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrantHeader {
    schema_version: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyBookmark {
    token: String,
    kind: HandleKind,
    path: String,
    label: String,
    mode: FileMode,
    created_ms: u64,
}

#[derive(Deserialize)]
struct LegacyFile {
    bookmarks: Vec<LegacyBookmark>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GrantFile {
    schema_version: u32,
    grants: Vec<GrantRecord>,
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
    let bytes = match fs::read(store_path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(GrantFile {
                schema_version: 2,
                grants: Vec::new(),
            });
        }
        Err(err) => return Err(map_io_error(err)),
    };
    let corrupted = |err| ServiceError::new("Io", format!("corrupted grant store: {err}"));
    let header: GrantHeader = serde_json::from_slice(&bytes).map_err(corrupted)?;
    let file: GrantFile = match header.schema_version {
        1 => {
            let legacy: LegacyFile = serde_json::from_slice(&bytes).map_err(corrupted)?;
            GrantFile {
                schema_version: 1,
                grants: legacy
                    .bookmarks
                    .into_iter()
                    .map(|b| GrantRecord {
                        token: b.token,
                        kind: b.kind.into(),
                        scope: GrantScope { path: b.path },
                        label: b.label,
                        mode: b.mode,
                        created_ms: b.created_ms,
                    })
                    .collect(),
            }
        }
        2 => serde_json::from_slice(&bytes).map_err(corrupted)?,
        _ => {
            return Err(ServiceError::new(
                "Io",
                "unsupported grant store schema version",
            ));
        }
    };
    if file.grants.len() > MAX_GRANTS_PER_APP {
        return Err(grant_limit());
    }
    Ok(file)
}

pub(super) fn grant_limit() -> ServiceError {
    ServiceError::new(
        "E_LIMIT",
        format!("grant limit ({MAX_GRANTS_PER_APP}) reached for app"),
    )
}

fn grant_parent(store_path: &Path) -> Result<&Path, ServiceError> {
    store_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| ServiceError::new("InvalidPath", "grant store has no parent directory"))
}

// Lock the directory, not the ledger inode: each atomic rewrite replaces that inode.
fn lock_grant_directory(parent: &Path) -> Result<Option<File>, ServiceError> {
    let dir = match File::open(parent) {
        Ok(dir) => dir,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(map_io_error(err)),
    };
    dir.lock().map_err(map_io_error)?;
    Ok(Some(dir))
}

fn save_grant_file(store_path: &Path, data: &mut GrantFile) -> Result<(), ServiceError> {
    fs::set_permissions(grant_parent(store_path)?, fs::Permissions::from_mode(0o700))
        .map_err(map_io_error)?;
    data.schema_version = 2;
    let bytes =
        serde_json::to_vec_pretty(data).map_err(|err| ServiceError::new("Io", err.to_string()))?;
    atomic_write_bytes(store_path, &bytes, Some(0o600))
}

/// Persists a workspace bookmark, returning its opaque token and metadata.
pub fn persist_grant(
    store_path: &Path,
    app_id: &str,
    kind: HandleKind,
    canonical_path: &Path,
    mode: FileMode,
) -> Result<GrantRecord, ServiceError> {
    let parent = grant_parent(store_path)?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)
        .map_err(map_io_error)?;
    let _lock = lock_grant_directory(parent)?
        .ok_or_else(|| ServiceError::new("NotFound", "grant store directory disappeared"))?;
    let mut file = load_grant_file(store_path)?;
    let kind = kind.into();
    let path_str = canonical_path.to_string_lossy();
    let index = if let Some(index) = file
        .grants
        .iter()
        .position(|b| b.kind == kind && b.scope.path == path_str)
    {
        if mode.can_write() {
            file.grants[index].mode = FileMode::ReadWrite;
        }
        index
    } else {
        let index = file.grants.len();
        if index >= MAX_GRANTS_PER_APP {
            return Err(grant_limit());
        }
        file.grants.push(GrantRecord {
            token: generate_bookmark_token(app_id, &path_str),
            kind,
            scope: GrantScope {
                path: path_str.into_owned(),
            },
            label: canonical_path
                .file_name()
                .map_or_else(|| "/".to_owned(), |s| s.to_string_lossy().into_owned()),
            mode,
            created_ms: now_ms(),
        });
        index
    };
    save_grant_file(store_path, &mut file)?;
    Ok(file.grants.swap_remove(index))
}

pub(super) fn grant_not_found() -> ServiceError {
    ServiceError::new("NotFound", "bookmark token not found or revoked")
}

/// Redeems only workspace grants admitted by the current package, never a system grant.
pub fn restore_workspace_grant(
    store_path: &Path,
    token: &str,
    ceiling: WorkspaceGrantCeiling,
) -> Result<GrantRecord, ServiceError> {
    let Some(_lock) = lock_grant_directory(grant_parent(store_path)?)? else {
        return Err(grant_not_found());
    };
    let mut file = load_grant_file(store_path)?;
    let index = file
        .grants
        .iter()
        .position(|r| r.token == token)
        .ok_or_else(grant_not_found)?;
    ceiling.authorize(file.grants[index].kind, file.grants[index].mode)?;
    if file.schema_version == 1 {
        save_grant_file(store_path, &mut file)?;
    }
    Ok(file.grants.swap_remove(index))
}

/// Lists all persisted grants for this app, upgrading an existing legacy ledger.
pub fn list_grants(store_path: &Path) -> Result<Vec<GrantRecord>, ServiceError> {
    let Some(_lock) = lock_grant_directory(grant_parent(store_path)?)? else {
        return Ok(Vec::new());
    };
    let mut file = load_grant_file(store_path)?;
    if file.schema_version == 1 {
        save_grant_file(store_path, &mut file)?;
    }
    Ok(file.grants)
}

/// Revokes a grant by opaque token, returning `true` if it existed.
pub fn revoke_grant(store_path: &Path, token: &str) -> Result<bool, ServiceError> {
    revoke(store_path, token, |_| true)
}

/// Revokes a workspace bookmark without touching system grants.
pub fn revoke_workspace_grant(store_path: &Path, token: &str) -> Result<bool, ServiceError> {
    revoke(store_path, token, |kind| kind.workspace_kind().is_some())
}

fn revoke(
    store_path: &Path,
    token: &str,
    includes: impl Fn(GrantKind) -> bool,
) -> Result<bool, ServiceError> {
    let Some(_lock) = lock_grant_directory(grant_parent(store_path)?)? else {
        return Ok(false);
    };
    let mut file = load_grant_file(store_path)?;
    let before = file.grants.len();
    file.grants
        .retain(|b| b.token != token || !includes(b.kind));
    let removed = file.grants.len() != before;
    if removed || file.schema_version == 1 {
        save_grant_file(store_path, &mut file)?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denied_legacy_restore_leaves_ledger_and_target_untouched_then_list_migrates() {
        use std::os::unix::fs::MetadataExt;

        let dir = env::temp_dir().join(format!(
            "zgui-grants-migration-{}-{}",
            std::process::id(),
            TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let ledger_dir = dir.join("grants");
        fs::create_dir_all(&ledger_dir).unwrap();
        let target = dir.join("notes.txt");
        fs::write(&target, "keep me").unwrap();
        let store = ledger_dir.join("app.json");
        let legacy = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1, "bookmarks": [
                {"token": "saved", "kind": "file", "path": target, "label": "My notes", "mode": "readwrite", "createdMs": 42},
                {"token": "folder", "kind": "dir", "path": dir, "label": "My workspace", "mode": "read", "createdMs": 43}
            ]
        })).unwrap();
        fs::write(&store, &legacy).unwrap();
        let original = File::open(&store).unwrap();

        assert_eq!(
            restore_workspace_grant(&store, "saved", WorkspaceGrantCeiling::ReadOnly)
                .unwrap_err()
                .code,
            "E_CAPABILITY"
        );
        assert_eq!(fs::read(&store).unwrap(), legacy);
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep me");

        let grants = list_grants(&store).unwrap();
        assert_eq!(grants[0].kind, GrantKind::WorkspaceFile);
        assert_eq!(grants[1].kind, GrantKind::WorkspaceDir);
        assert_eq!(grants[0].label, "My notes");
        assert_eq!(grants[0].created_ms, 42);
        assert_eq!(
            restore_workspace_grant(&store, "saved", WorkspaceGrantCeiling::ReadWrite).unwrap(),
            grants[0]
        );
        let migrated: serde_json::Value =
            serde_json::from_slice(&fs::read(&store).unwrap()).unwrap();
        assert_eq!(migrated["schemaVersion"], 2);
        assert_eq!(migrated["grants"][1]["kind"], "workspace-dir");
        assert_eq!(migrated["grants"][0]["accessMode"], "readwrite");
        assert_eq!(
            migrated["grants"][0]["scopeMetadata"]["path"],
            target.to_str().unwrap()
        );
        let metadata = fs::metadata(&store).unwrap();
        assert_ne!(original.metadata().unwrap().ino(), metadata.ino());
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        assert_eq!(
            fs::metadata(&ledger_dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(fs::read_dir(&ledger_dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

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

        let rec = persist_grant(
            &app_a,
            "app-a",
            HandleKind::Dir,
            target,
            FileMode::ReadWrite,
        )
        .expect("persist grant");
        assert_eq!(rec.token.len(), 32);
        assert_eq!(rec.label, "workspace-demo");

        // App A can restore and list it.
        let restored =
            restore_workspace_grant(&app_a, &rec.token, WorkspaceGrantCeiling::ReadWrite)
                .expect("restore grant");
        assert_eq!(restored, rec);
        assert_eq!(list_grants(&app_a).unwrap().len(), 1);

        // App B cannot restore App A's token.
        assert!(
            restore_workspace_grant(&app_b, &rec.token, WorkspaceGrantCeiling::ReadWrite).is_err()
        );

        // Revoking removes it cleanly.
        assert!(revoke_grant(&app_a, &rec.token).unwrap());
        assert!(!revoke_grant(&app_a, &rec.token).unwrap());
        assert!(
            restore_workspace_grant(&app_a, &rec.token, WorkspaceGrantCeiling::ReadWrite).is_err()
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn full_ledger_still_allows_deduplication_and_revocation() {
        let dir = env::temp_dir().join(format!(
            "zgui-grant-cap-{}-{}",
            std::process::id(),
            TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let store = dir.join("app.json");
        for i in 0..256 {
            let target = dir.join(i.to_string());
            persist_grant(&store, "app", HandleKind::File, &target, FileMode::Read).unwrap();
        }
        let extra = dir.join("overflow");
        assert_eq!(
            persist_grant(&store, "app", HandleKind::File, &extra, FileMode::Read)
                .unwrap_err()
                .code,
            "E_LIMIT"
        );
        let first = persist_grant(
            &store,
            "app",
            HandleKind::File,
            &dir.join("0"),
            FileMode::ReadWrite,
        )
        .unwrap();
        assert_eq!(list_grants(&store).unwrap().len(), 256);
        assert!(revoke_grant(&store, &first.token).unwrap());
        persist_grant(&store, "app", HandleKind::File, &extra, FileMode::Read).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
}
