use std::{cell::{OnceCell, RefCell}, collections::BTreeMap, error::Error, fs, path::{Path, PathBuf}, rc::Rc, sync::Arc};
use crate::bounded_fs::{self, BoundedReadError};
use crate::runtime_paths::{canonical_absolute, socket_name, template};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;
const MAX_RESOURCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CHUNKS: usize = 256;
const MAX_WORKERS: usize = 32;

crate::schema::object_struct! {
/// One sealed lazy/shared JS chunk and its paired source map.
#[derive(PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChunkEntry {
    pub path: String,
    pub source_map: String,
}
}

crate::schema::object_struct! {
/// One sealed isolated compute worker entrypoint and its paired source map.
#[derive(PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerEntry {
    pub name: String,
    pub path: String,
    pub source_map: String,
}
}

pub const HOST_API_VERSION: u32 = 1;
pub const SCHEMA_VERSION: u32 = 2;

crate::schema::object_struct! {
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub id: String,
    pub host_api_version: u32,
    pub entry: String,
    pub source_map: String,
    /// Optional compiled QuickJS bytecode format (`"qjs-bytecode-v1"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytecode_format: Option<String>,
    /// Optional exact QuickJS bytecode ABI tag matching the linked engine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quickjs_abi: Option<String>,
    /// Optional closed graph of additional JS chunks sealed into `artifactDigests`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chunks: Vec<ChunkEntry>,
    /// Optional sealed isolated compute workers declared by the package.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workers: Vec<WorkerEntry>,
    pub styles: Vec<String>,
    pub assets: Vec<String>,
    pub fonts: Vec<Font>,
    /// Optional primary window configuration sealed into the package manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowConfig>,
    /// Host authority sealed into the package identity; omitted declarations grant nothing.
    #[serde(default, skip_serializing_if = "Capabilities::is_empty")]
    pub capabilities: Capabilities,
    pub style_system: StyleSystem,
    #[serde(deserialize_with = "unique_digests")]
    pub artifact_digests: BTreeMap<String, String>,
    pub build_id: String,
}
}

crate::schema::object_struct! {
/// Clipboard capability flags; writes are allowed by default, reads require declaration or user gesture.
#[derive(Default, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ClipboardCapabilities {
    /// Allows reading the clipboard outside an active user-gesture turn.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub read: bool,
}
}

impl ClipboardCapabilities {
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        !self.read
    }
}

crate::schema::object_struct! {
/// App-scoped SQLite database declarations.
#[derive(Default, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct SqliteCapabilities {
    /// Named SQLite databases this application may open.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub databases: Vec<String>,
}
}

impl SqliteCapabilities {
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.databases.is_empty()
    }
}

crate::schema::object_struct! {
/// One pre-declared directory allowlist grant.
#[derive(PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileDirectoryGrant {
    pub path: String,
    pub mode: host_services::files::FileMode,
}
}

crate::schema::object_struct! {
/// Local directory allowlist capability declarations.
#[derive(Default, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct FilesCapabilities {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub directories: Vec<FileDirectoryGrant>,
}
}

impl FilesCapabilities {
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.directories.is_empty()
    }
}

crate::schema::object_struct! {
/// Origin-allowlisted outbound network capability declarations.
#[derive(Default, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkCapabilities {
    /// Exact normalized origins (`scheme://host[:port]`) the session may contact.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub origins: Vec<String>,
    /// Explicitly permits `http://` and `ws://` origins when listed.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub allow_insecure_http: bool,
    /// Explicitly permits loopback, link-local, and RFC1918/ULA private destinations.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub allow_private_networks: bool,
}
}

impl NetworkCapabilities {
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.origins.is_empty() && !self.allow_insecure_http && !self.allow_private_networks
    }

    pub(crate) fn to_origin_policy(&self) -> std::result::Result<host_services::network::OriginPolicy, String> {
        let mut parsed = Vec::with_capacity(self.origins.len());
        let mut seen = std::collections::BTreeSet::new();
        for entry in &self.origins {
            let origin = host_services::network::Origin::parse_manifest_entry(entry)
                .map_err(|e| format!("invalid capabilities.network.origins entry {entry:?}: {e}"))?;
            if origin.to_string() != *entry {
                return Err(format!(
                    "capabilities.network.origins entry {entry:?} must be in canonical form {:?}",
                    origin.to_string()
                ));
            }
            if !seen.insert(origin.clone()) {
                return Err(format!("duplicate capabilities.network.origins entry: {entry:?}"));
            }
            parsed.push(origin);
        }
        host_services::network::OriginPolicy::new(
            parsed,
            self.allow_insecure_http,
            self.allow_private_networks,
        )
        .map_err(|e| format!("invalid capabilities.network policy: {e}"))
    }
}

pub(crate) fn valid_file_directory_path(path: &str) -> bool {
    if canonical_absolute(path) {
        return true;
    }
    let (root, tail) = match path.split_once('/') {
        Some((root, tail)) => (root, Some(tail)),
        None => (path, None),
    };
    if !matches!(root, "$HOME" | "$XDG_DATA_HOME" | "$XDG_DOCUMENTS_DIR") {
        return false;
    }
    tail.is_none_or(|t| {
        t.split('/')
            .all(|part| crate::runtime_paths::safe_segment(part) && !part.contains('$'))
    })
}

crate::schema::object_struct! {
/// Host privileges requested by the package, all denied by default.
#[derive(Default, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Capabilities {
    /// Allows the session to create desktop shell surfaces.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub shell_surfaces: bool,
    /// Allows the session to request a compositor session lock.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub session_lock: bool,
    /// Literal or templated directory prefixes for read-only `file://` images.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub image_prefixes: Vec<String>,
    /// Literal or templated exact Unix socket paths.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unix_sockets: Vec<String>,
    /// Names of sockets that may be bound by the host at launch.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub named_sockets: Vec<String>,
    /// Exact command names the session may invoke.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
    /// Allows the session to expose a host control channel.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub control_channel: bool,
    /// Clipboard read authorization outside user gestures.
    #[serde(skip_serializing_if = "ClipboardCapabilities::is_empty")]
    pub clipboard: ClipboardCapabilities,
    /// App-scoped SQLite database declarations.
    #[serde(skip_serializing_if = "SqliteCapabilities::is_empty")]
    pub sqlite: SqliteCapabilities,
    /// Pre-declared local directory allowlist grants.
    #[serde(skip_serializing_if = "FilesCapabilities::is_empty")]
    pub files: FilesCapabilities,
    /// Origin-allowlisted outbound network declarations.
    #[serde(skip_serializing_if = "NetworkCapabilities::is_empty")]
    pub network: NetworkCapabilities,
}
}

impl Capabilities {
    /// Whether the package requests no host authority.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        !self.shell_surfaces
            && !self.session_lock
            && self.image_prefixes.is_empty()
            && self.unix_sockets.is_empty()
            && self.named_sockets.is_empty()
            && self.commands.is_empty()
            && !self.control_channel
            && self.clipboard.is_empty()
            && self.sqlite.is_empty()
            && self.files.is_empty()
            && self.network.is_empty()
    }

    fn validate(&self) -> Result<()> {
        if !self.network.is_empty() {
            self.network.to_origin_policy().map_err(Box::<dyn Error>::from)?;
        }
        let mut seen_dbs = std::collections::BTreeSet::new();
        for name in &self.sqlite.databases {
            if !host_services::sqlite::is_valid_database_name(name) || !seen_dbs.insert(name) {
                return Err(format!("invalid or duplicate capabilities.sqlite.databases entry: {name:?}").into());
            }
        }
        let mut seen_dirs = std::collections::BTreeSet::new();
        for grant in &self.files.directories {
            if !valid_file_directory_path(&grant.path) || !seen_dirs.insert(&grant.path) {
                return Err(format!("invalid or duplicate capabilities.files.directories entry: {:?}", grant.path).into());
            }
        }
        for (name, entries) in [
            ("imagePrefixes", &self.image_prefixes),
            ("unixSockets", &self.unix_sockets),
            ("namedSockets", &self.named_sockets),
            ("commands", &self.commands),
        ] {
            let mut seen = std::collections::BTreeSet::new();
            for entry in entries {
                let valid = match name {
                    "commands" => !entry.trim_matches(' ').is_empty() && !entry.chars().any(char::is_control),
                    "namedSockets" => socket_name(entry),
                    _ => canonical_absolute(entry) || template(entry).is_some(),
                };
                if !valid || !seen.insert(entry) {
                    return Err(
                        format!("invalid or duplicate capabilities.{name} entry: {entry:?}").into(),
                    );
                }
            }
        }
        Ok(())
    }
}

crate::schema::object_struct! {
/// Optional primary window dimensions and metadata declared in `package.json` (`zgui.window`).
#[derive(Default, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct WindowConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resizable: Option<bool>,
}
}

impl WindowConfig {
    fn validate(&self) -> Result<()> {
        if let Some(title) = &self.title
            && (title.is_empty()
                || title.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}') != title
                || title.chars().any(char::is_control))
        {
            return Err("manifest window.title must be a non-empty, trimmed string without control characters".into());
        }
        for (name, value) in [
            ("width", self.width),
            ("height", self.height),
            ("minWidth", self.min_width),
            ("minHeight", self.min_height),
        ] {
            if let Some(px) = value
                && !(1..=16384).contains(&px)
            {
                return Err(format!("manifest window.{name} must be between 1 and 16384").into());
            }
        }
        Ok(())
    }
}

crate::schema::object_struct! {
#[derive(PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Font {
    pub family: String,
    pub path: String,
}
}

crate::schema::object_struct! {
#[derive(PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StyleSystem {
    pub name: String,
    pub profile: String,
    pub config_digest: String,
}
}

#[derive(Clone)]
pub struct Stylesheet {
    pub path: Arc<str>,
    pub css: Arc<str>,
}

struct VerifiedChunk {
    relative_path: String,
    canonical_name: String,
    digest: String,
    source_map_path: String,
    source_map_digest: String,
    source_map: OnceCell<Option<sourcemap::DecodedMap>>,
}

/// Verified initial entry module payload (either UTF-8 ESM source or ABI-verified bytecode).
#[derive(Clone, Debug)]
pub(crate) enum EntryPayload {
    Source(Arc<str>),
    Bytecode(Arc<[u8]>),
}

fn logical_js_path(rel_path: &str) -> String {
    rel_path
        .strip_suffix(".qjsb")
        .map_or_else(|| rel_path.to_owned(), |stem| format!("{stem}.mjs"))
}

pub struct Package {
    pub directory: PathBuf,
    pub manifest: Manifest,
    pub entry_digest: String,
    entry_payload: RefCell<Option<EntryPayload>>,
    pub styles: Arc<[Stylesheet]>,
    pub source_map_digest: String,
    source_map: OnceCell<Option<sourcemap::DecodedMap>>,
    generated_entry: String,
    chunks: BTreeMap<String, VerifiedChunk>,
    workers: BTreeMap<String, VerifiedChunk>,
    status_path: Option<PathBuf>,
    assets: BTreeMap<String, Arc<[u8]>>,
}

impl Package {
    /// Returns true strictly when `next` preserves identical JS entry, chunks,
    /// workers, assets, fonts, window declaration, capabilities, and style system,
    /// differing only in packaged stylesheet assets.
    pub(crate) fn is_css_only_successor(&self, next: &Self) -> bool {
        let a = &self.manifest;
        let b = &next.manifest;
        if a.schema_version != b.schema_version
            || a.host_api_version != b.host_api_version
            || a.id != b.id
            || a.entry != b.entry
            || a.source_map != b.source_map
            || a.bytecode_format != b.bytecode_format
            || a.quickjs_abi != b.quickjs_abi
            || a.chunks != b.chunks
            || a.workers != b.workers
            || a.assets != b.assets
            || a.fonts != b.fonts
            || a.window != b.window
            || a.capabilities != b.capabilities
            || a.style_system != b.style_system
        {
            return false;
        }
        let same_digest = |key: &str| a.artifact_digests.get(key) == b.artifact_digests.get(key);
        if !same_digest(&a.entry) {
            return false;
        }
        for chunk in &a.chunks {
            if !same_digest(&chunk.path) {
                return false;
            }
        }
        for worker in &a.workers {
            if !same_digest(&worker.path) {
                return false;
            }
        }
        for asset in &a.assets {
            if !same_digest(asset) {
                return false;
            }
        }
        if a.styles != b.styles {
            return true;
        }
        a.styles.iter().any(|style| !same_digest(style))
    }

    /// Resolve the publication symlink once. Every read then belongs to that
    /// immutable generation, even if the builder replaces dist concurrently.
    pub fn load(path: &Path) -> Result<Self> {
        let directory = path.canonicalize()?;
        let status_path = if fs::symlink_metadata(path)?.file_type().is_symlink() {
            Some(path.parent().unwrap_or(Path::new(".")).canonicalize()?.join(".zgui-native-status.json"))
        } else if directory.parent().and_then(|parent| parent.file_name()).is_some_and(|name| name == ".zgui-generations") {
            directory.parent().and_then(|parent| parent.parent()).map(|root| root.join(".zgui-native-status.json"))
        } else { None };
        if !directory.is_dir() { return Err("bundle root is not a directory".into()); }
        let manifest: Manifest = serde_json::from_slice(&resource(&directory, "manifest.json")?)?;
        if manifest.schema_version != SCHEMA_VERSION || manifest.host_api_version != HOST_API_VERSION {
            return Err("unsupported package schema or host API version".into());
        }
        if manifest.id.is_empty()
            || manifest.id.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}') != manifest.id
            || manifest.id.chars().any(char::is_control)
        {
            return Err("package ID must be a non-empty, trimmed string without control characters".into());
        }
        if let Some(window) = &manifest.window {
            window.validate()?;
        }
        manifest.capabilities.validate()?;
        if manifest.style_system.name != "lightningcss" || manifest.style_system.profile != "lightningcss-v1"
            || !valid_digest(&manifest.style_system.config_digest) {
            return Err("unsupported style system/profile or invalid config digest".into());
        }
        let is_bytecode = match (manifest.bytecode_format.as_deref(), manifest.quickjs_abi.as_deref()) {
            (None, None) => false,
            (Some(fmt), Some(abi)) => {
                if fmt != js_runtime::BYTECODE_FORMAT {
                    return Err(format!(
                        "unsupported bytecodeFormat {fmt:?}; expected {:?}",
                        js_runtime::BYTECODE_FORMAT
                    )
                    .into());
                }
                let expected_abi = js_runtime::quickjs_abi_tag();
                if abi != expected_abi {
                    return Err(format!(
                        "incompatible quickjsAbi {abi:?}; host expects {expected_abi:?}"
                    )
                    .into());
                }
                true
            }
            _ => {
                return Err(
                    "manifest must specify both bytecodeFormat and quickjsAbi together".into(),
                );
            }
        };
        let module_ext = if is_bytecode { ".qjsb" } else { ".mjs" };
        if !valid_digest(&manifest.build_id) { return Err("invalid package buildId".into()); }
        if manifest.chunks.len() > MAX_CHUNKS {
            return Err(format!("package exceeds maximum chunk count ({MAX_CHUNKS})").into());
        }
        if manifest.workers.len() > MAX_WORKERS {
            return Err(format!("package exceeds maximum worker count ({MAX_WORKERS})").into());
        }
        let mut files = BTreeMap::new();
        let chunk_resources = manifest.chunks.iter().flat_map(|chunk| [chunk.path.as_str(), chunk.source_map.as_str()]);
        let worker_resources = manifest.workers.iter().flat_map(|worker| [worker.path.as_str(), worker.source_map.as_str()]);
        for name in std::iter::once(manifest.entry.as_str())
            .chain(std::iter::once(manifest.source_map.as_str()))
            .chain(chunk_resources)
            .chain(worker_resources)
            .chain(manifest.styles.iter().map(String::as_str))
            .chain(manifest.assets.iter().map(String::as_str))
            .chain(std::iter::once("build-info.json"))
        {
            if name == "manifest.json" || files.contains_key(name) {
                return Err(format!("duplicate or reserved resource: {name}").into());
            }
            let expected = manifest.artifact_digests.get(name).ok_or_else(|| format!("missing artifact digest: {name}"))?;
            if !valid_digest(expected) { return Err(format!("invalid artifact digest: {name}").into()); }
            let bytes = resource(&directory, name)?;
            let actual = digest(&bytes);
            if &actual != expected { return Err(format!("artifact integrity mismatch for {name}: expected {expected}, got {actual}").into()); }
            files.insert(name.to_owned(), bytes);
        }
        if files.len() != manifest.artifact_digests.len() {
            return Err("artifact digests contain undeclared resources".into());
        }
        verify_inventory(&directory, &directory, &files)?;
        let mut identity = serde_json::to_value(&manifest)?;
        identity.as_object_mut().ok_or("manifest must be an object")?.remove("buildId");
        let actual = digest(&canonical_bytes(&identity)?);
        if actual != manifest.build_id { return Err(format!("package integrity mismatch: expected {}, got {actual}", manifest.build_id).into()); }
        let mut families = std::collections::BTreeSet::new();
        let mut font_bytes = 0;
        if manifest.fonts.len() > 16 { return Err("package exceeds 16 font mappings".into()); }
        for font in &manifest.fonts {
            if font.family.trim().is_empty() || font.family.len() > 128 || font.family.chars().any(char::is_control)
                || !families.insert(font.family.to_lowercase()) || !manifest.assets.contains(&font.path) {
                return Err(format!("invalid, duplicate or undeclared font mapping: {}", font.family).into());
            }
            let bytes = &files[&font.path];
            font_bytes += bytes.len();
            if bytes.len() > 32 * 1024 * 1024 || font_bytes > 64 * 1024 * 1024 {
                return Err("package font byte budget exceeded".into());
            }
        }
        let style_identity = serde_json::to_value(&manifest.style_system)?;
        {
            let name = "build-info.json";
            let bytes = files.remove(name).ok_or_else(|| format!("missing metadata: {name}"))?;
            let value: serde_json::Value = serde_json::from_slice(&bytes)?;
            drop(bytes);
            if !value.is_object() { return Err(format!("{name} must be a JSON object").into()); }
            if value.get("styleSystem") != Some(&style_identity) {
                return Err(format!("{name} styleSystem identity is missing or differs from manifest").into());
            }
        }
        if !manifest.entry.ends_with(module_ext) {
            return Err(format!(
                "manifest entry {} must end with {module_ext}",
                manifest.entry
            )
            .into());
        }
        let entry_bytes = files.remove(&manifest.entry).ok_or("missing entry")?;
        let entry_digest = digest(&entry_bytes);
        let entry_payload = if is_bytecode {
            if entry_bytes.is_empty() {
                return Err("bytecode entry payload must be non-empty".into());
            }
            EntryPayload::Bytecode(Arc::from(entry_bytes))
        } else {
            EntryPayload::Source(Arc::from(String::from_utf8(entry_bytes)?))
        };
        let map_bytes = files.remove(&manifest.source_map).ok_or("missing source map")?;
        let source_map_digest = digest(&map_bytes);
        // The decoder defaults missing fields, accepting even `{}`; require a
        // recognized map document and validate its mappings eagerly, then drop
        // the decoded table until a diagnostic stack trace actually needs it.
        if !sourcemap::is_sourcemap_slice(&map_bytes) {
            return Err(format!("invalid source map {}: unrecognized source map document", manifest.source_map).into());
        }
        let validated_map = sourcemap::DecodedMap::from_reader(map_bytes.as_slice())
            .map_err(|error| format!("invalid source map {}: {error}", manifest.source_map))?;
        drop(validated_map);
        drop(map_bytes);
        let mut chunks = BTreeMap::new();
        for chunk in &manifest.chunks {
            if !chunk.path.ends_with(module_ext) || !chunk.source_map.ends_with(".map") {
                return Err(format!("invalid chunk declaration: {} / {}", chunk.path, chunk.source_map).into());
            }
            let chunk_bytes = files.remove(&chunk.path).ok_or_else(|| format!("missing chunk: {}", chunk.path))?;
            let chunk_digest = digest(&chunk_bytes);
            if is_bytecode {
                if chunk_bytes.is_empty() {
                    return Err(format!("bytecode chunk {} must be non-empty", chunk.path).into());
                }
            } else {
                // Validate UTF-8 eagerly, then release bytes until QuickJS lazily loads this chunk.
                let _ = std::str::from_utf8(&chunk_bytes)?;
            }
            drop(chunk_bytes);
            let chunk_map_bytes = files.remove(&chunk.source_map).ok_or_else(|| format!("missing chunk source map: {}", chunk.source_map))?;
            let chunk_map_digest = digest(&chunk_map_bytes);
            if !sourcemap::is_sourcemap_slice(&chunk_map_bytes) {
                return Err(format!("invalid source map {}: unrecognized source map document", chunk.source_map).into());
            }
            let validated = sourcemap::DecodedMap::from_reader(chunk_map_bytes.as_slice())
                .map_err(|error| format!("invalid source map {}: {error}", chunk.source_map))?;
            drop(validated);
            drop(chunk_map_bytes);
            let canonical_name = directory.join(&chunk.path).to_string_lossy().into_owned();
            chunks.insert(
                chunk.path.clone(),
                VerifiedChunk {
                    relative_path: chunk.path.clone(),
                    canonical_name,
                    digest: chunk_digest,
                    source_map_path: chunk.source_map.clone(),
                    source_map_digest: chunk_map_digest,
                    source_map: OnceCell::new(),
                },
            );
        }
        let mut workers = BTreeMap::new();
        for worker in &manifest.workers {
            if !valid_worker_name(&worker.name)
                || !worker.path.ends_with(module_ext)
                || !worker.source_map.ends_with(".map")
            {
                return Err(format!("invalid worker declaration: {} ({})", worker.name, worker.path).into());
            }
            if workers.contains_key(&worker.name) {
                return Err(format!("duplicate worker declaration: {}", worker.name).into());
            }
            let worker_bytes = files.remove(&worker.path).ok_or_else(|| format!("missing worker: {}", worker.path))?;
            let worker_digest = digest(&worker_bytes);
            if is_bytecode {
                if worker_bytes.is_empty() {
                    return Err(format!("bytecode worker {} must be non-empty", worker.path).into());
                }
            } else {
                let _ = std::str::from_utf8(&worker_bytes)?;
            }
            drop(worker_bytes);
            let worker_map_bytes = files.remove(&worker.source_map).ok_or_else(|| format!("missing worker source map: {}", worker.source_map))?;
            let worker_map_digest = digest(&worker_map_bytes);
            if !sourcemap::is_sourcemap_slice(&worker_map_bytes) {
                return Err(format!("invalid source map {}: unrecognized source map document", worker.source_map).into());
            }
            let validated = sourcemap::DecodedMap::from_reader(worker_map_bytes.as_slice())
                .map_err(|error| format!("invalid source map {}: {error}", worker.source_map))?;
            drop(validated);
            drop(worker_map_bytes);
            let canonical_name = directory.join(&worker.path).to_string_lossy().into_owned();
            workers.insert(
                worker.name.clone(),
                VerifiedChunk {
                    relative_path: worker.path.clone(),
                    canonical_name,
                    digest: worker_digest,
                    source_map_path: worker.source_map.clone(),
                    source_map_digest: worker_map_digest,
                    source_map: OnceCell::new(),
                },
            );
        }
        let styles: Arc<[Stylesheet]> = manifest.styles.iter().map(|name| {
            Ok(Stylesheet {
                path: Arc::from(name.as_str()),
                css: Arc::from(String::from_utf8(files.remove(name).ok_or("missing stylesheet")?)?),
            })
        }).collect::<Result<Vec<_>>>()?.into();
        let generated_entry = directory.join(&manifest.entry).to_string_lossy().into_owned();
        let assets = manifest.assets.iter().map(|name| {
            (name.clone(), Arc::from(files.remove(name).expect("verified declared asset")))
        }).collect();
        Ok(Self {
            directory,
            manifest,
            entry_digest,
            entry_payload: RefCell::new(Some(entry_payload)),
            styles,
            source_map_digest,
            source_map: OnceCell::new(),
            generated_entry,
            chunks,
            workers,
            status_path,
            assets,
        })
    }

    /// Canonical entry module identifier used when evaluating the bundle in QuickJS.
    #[must_use]
    pub fn generated_entry(&self) -> &str {
        &self.generated_entry
    }

    /// Returns whether this package ships ABI-verified QuickJS bytecode payloads (`.qjsb`).
    #[must_use]
    pub fn is_bytecode(&self) -> bool {
        self.manifest.bytecode_format.is_some()
    }

    /// Consume the initial entry payload buffer so steady-state sessions do not retain
    /// full JS/bytecode buffers in Rust heap after QuickJS evaluation; retries reload from the
    /// verified immutable package directory on demand.
    pub(crate) fn take_or_load_entry(&self) -> Result<EntryPayload> {
        if let Some(cached) = self.entry_payload.borrow_mut().take() {
            return Ok(cached);
        }
        self.read_verified_entry()
    }

    fn read_verified_entry(&self) -> Result<EntryPayload> {
        let bytes = resource(&self.directory, &self.manifest.entry)?;
        let actual = digest(&bytes);
        if actual != self.entry_digest {
            return Err(format!(
                "artifact integrity mismatch for {}: expected {}, got {actual}",
                self.manifest.entry, self.entry_digest
            )
            .into());
        }
        if self.is_bytecode() {
            Ok(EntryPayload::Bytecode(Arc::from(bytes)))
        } else {
            Ok(EntryPayload::Source(Arc::from(String::from_utf8(bytes)?)))
        }
    }

    /// Read and digest-verify a declared JSON metadata artifact (e.g., `build-info.json`)
    /// on demand so live desktop sessions never retain parsed trees.
    pub fn read_verified_json(&self, name: &str) -> Result<serde_json::Value> {
        let expected = self.manifest.artifact_digests.get(name)
            .ok_or_else(|| format!("undeclared package metadata resource: {name}"))?;
        let bytes = resource(&self.directory, name)?;
        let actual = digest(&bytes);
        if &actual != expected {
            return Err(format!(
                "artifact integrity mismatch for {name}: expected {expected}, got {actual}"
            ).into());
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        if !value.is_object() {
            return Err(format!("{name} must be a JSON object").into());
        }
        Ok(value)
    }

    fn decoded_source_map(&self) -> Option<&sourcemap::DecodedMap> {
        self.source_map.get_or_init(|| {
            let bytes = resource(&self.directory, &self.manifest.source_map).ok()?;
            if digest(&bytes) != self.source_map_digest || !sourcemap::is_sourcemap_slice(&bytes) {
                return None;
            }
            sourcemap::DecodedMap::from_reader(bytes.as_slice()).ok()
        }).as_ref()
    }

    fn decoded_chunk_source_map<'a>(&'a self, chunk: &'a VerifiedChunk) -> Option<&'a sourcemap::DecodedMap> {
        chunk.source_map.get_or_init(|| {
            let bytes = resource(&self.directory, &chunk.source_map_path).ok()?;
            if digest(&bytes) != chunk.source_map_digest || !sourcemap::is_sourcemap_slice(&bytes) {
                return None;
            }
            sourcemap::DecodedMap::from_reader(bytes.as_slice()).ok()
        }).as_ref()
    }

    /// Build a closed-graph QuickJS module provider backed strictly by verified
    /// package chunk digests (never reading mutable dist, cwd, or network).
    pub(crate) fn module_provider(self: &Rc<Self>) -> Rc<dyn js_runtime::VerifiedModuleProvider> {
        Rc::new(PackageModuleProvider {
            package: Rc::clone(self),
        })
    }

    fn relative_module_path<'a>(&'a self, referrer: &'a str) -> Option<&'a str> {
        if referrer == self.generated_entry
            || referrer == self.manifest.entry
            || (self.is_bytecode() && referrer == logical_js_path(&self.manifest.entry))
        {
            return Some(self.manifest.entry.as_str());
        }
        if let Some(chunk) = self.chunks.values().find(|c| {
            c.canonical_name == referrer
                || c.relative_path == referrer
                || (self.is_bytecode() && logical_js_path(&c.relative_path) == referrer)
        }) {
            return Some(chunk.relative_path.as_str());
        }
        None
    }

    fn resolve_sealed_import(&self, referrer: &str, specifier: &str) -> std::result::Result<String, String> {
        if !specifier.starts_with("./") && !specifier.starts_with("../") {
            return Err(format!("non-relative module specifier is forbidden: {specifier:?}"));
        }
        let base_rel = self
            .relative_module_path(referrer)
            .ok_or_else(|| format!("unknown module referrer: {referrer:?}"))?;
        let base_dir = base_rel.rsplit_once('/').map_or("", |(dir, _)| dir);
        let normalized = normalize_posix_relative(base_dir, specifier)
            .ok_or_else(|| format!("module specifier escapes sealed package root: {specifier:?}"))?;
        let mapped = if self.is_bytecode()
            && let Some(stem) = normalized.strip_suffix(".mjs")
        {
            format!("{stem}.qjsb")
        } else {
            normalized.clone()
        };
        if normalized == self.manifest.entry || mapped == self.manifest.entry {
            return Ok(self.generated_entry.clone());
        }
        if let Some(chunk) = self.chunks.get(&normalized).or_else(|| self.chunks.get(&mapped)) {
            return Ok(chunk.canonical_name.clone());
        }
        Err(format!("module specifier is not in the sealed chunk graph: {specifier:?} -> {normalized:?}"))
    }

    fn load_sealed_module(
        &self,
        canonical_name: &str,
    ) -> std::result::Result<js_runtime::VerifiedModulePayload, String> {
        if canonical_name == self.generated_entry {
            return match self.read_verified_entry().map_err(|err| err.to_string())? {
                EntryPayload::Source(src) => Ok(js_runtime::VerifiedModulePayload::Source(src.to_string())),
                EntryPayload::Bytecode(bytes) => {
                    Ok(js_runtime::VerifiedModulePayload::Bytecode(bytes.to_vec()))
                }
            };
        }
        let chunk = self
            .chunks
            .values()
            .find(|c| c.canonical_name == canonical_name)
            .ok_or_else(|| format!("undeclared sealed chunk: {canonical_name:?}"))?;
        let bytes = resource(&self.directory, &chunk.relative_path).map_err(|err| err.to_string())?;
        let actual = digest(&bytes);
        if actual != chunk.digest {
            return Err(format!(
                "artifact integrity mismatch for {}: expected {}, got {actual}",
                chunk.relative_path, chunk.digest
            ));
        }
        if self.is_bytecode() {
            Ok(js_runtime::VerifiedModulePayload::Bytecode(bytes))
        } else {
            String::from_utf8(bytes)
                .map(js_runtime::VerifiedModulePayload::Source)
                .map_err(|err| err.to_string())
        }
    }

    /// Reads and SHA-256-verifies a declared compute worker module (source or bytecode)
    /// from the immutable package generation on demand.
    pub(crate) fn load_worker_payload(
        &self,
        name: &str,
    ) -> std::result::Result<(String, js_runtime::VerifiedModulePayload), String> {
        let worker = self
            .workers
            .get(name)
            .ok_or_else(|| format!("undeclared compute worker: {name:?}"))?;
        let bytes = resource(&self.directory, &worker.relative_path).map_err(|err| err.to_string())?;
        let actual = digest(&bytes);
        if actual != worker.digest {
            return Err(format!(
                "artifact integrity mismatch for {}: expected {}, got {actual}",
                worker.relative_path, worker.digest
            ));
        }
        if self.is_bytecode() {
            Ok((
                logical_js_path(&worker.relative_path),
                js_runtime::VerifiedModulePayload::Bytecode(bytes),
            ))
        } else {
            let source = String::from_utf8(bytes).map_err(|err| err.to_string())?;
            Ok((
                worker.canonical_name.clone(),
                js_runtime::VerifiedModulePayload::Source(source),
            ))
        }
    }

    #[cfg(test)]
    pub(crate) fn load_worker_source(&self, name: &str) -> std::result::Result<(String, String), String> {
        let (module_id, payload) = self.load_worker_payload(name)?;
        match payload {
            js_runtime::VerifiedModulePayload::Source(source) => Ok((module_id, source)),
            js_runtime::VerifiedModulePayload::Bytecode(_) => {
                Err(format!("worker {name:?} is compiled bytecode, not UTF-8 source"))
            }
        }
    }

    /// Immutable manifest-declared bytes from the integrity-verified package generation.
    pub fn asset_bytes(&self, name: &str) -> Option<&[u8]> {
        self.assets.get(name).map(AsRef::as_ref)
    }

    pub(crate) fn asset_data(&self, name: &str) -> Option<Arc<[u8]>> {
        self.assets.get(name).map(Arc::clone)
    }

    pub(crate) fn record_acceptance(&self, record: &serde_json::Value) -> Result<()> {
        let Some(path) = &self.status_path else { return Ok(()); };
        let temporary = path.with_file_name(format!(".zgui-native-status.json.{}.tmp", std::process::id()));
        fs::write(&temporary, serde_json::to_vec(record)?)?;
        if let Err(error) = fs::rename(&temporary, path) {
            let _ = fs::remove_file(temporary);
            return Err(error.into());
        }
        Ok(())
    }

    pub(crate) fn image_assets(&self) -> impl Iterator<Item = (String, Arc<[u8]>)> + '_ {
        self.assets.iter().map(|(name, bytes)| (name.clone(), Arc::clone(bytes)))
    }

    /// Remap this package's generated entry frames to one-based original locations.
    /// Messages, frame names, and frames without a matching source token are retained.
    pub fn remap_diagnostic(&self, mut diagnostic: js_runtime::Diagnostic) -> js_runtime::Diagnostic {
        let Some(stack) = &diagnostic.stack else { return diagnostic; };
        let mut remapped = None::<String>;
        let mut offset = 0;
        for frame in stack.split_inclusive('\n') {
            // QuickJS-NG emits `    at name (file:line:column)\n`, including
            // `<anonymous>` for module evaluation. Parse from the right because
            // package paths and function names may themselves contain colons.
            let location = (|| {
                let body = frame.strip_suffix('\n').unwrap_or(frame);
                let body = body.strip_prefix("    at ")?.strip_suffix(')')?;
                let (file_line, column) = body.rsplit_once(':')?;
                let (file, line) = file_line.rsplit_once(':')?;
                let entry_logical = logical_js_path(&self.manifest.entry);
                let (matched_name, decoded_map) = if let Some(entry) = [
                    self.generated_entry.as_str(),
                    self.manifest.entry.as_str(),
                    entry_logical.as_str(),
                ]
                .into_iter()
                .find(|&entry| {
                    file.strip_suffix(entry).is_some_and(|prefix| prefix.ends_with(" ("))
                }) {
                    (entry.to_owned(), self.decoded_source_map()?)
                } else {
                    let (chunk, matched) = self.chunks.values().chain(self.workers.values()).find_map(|chunk| {
                        let logical = logical_js_path(&chunk.relative_path);
                        [
                            chunk.canonical_name.as_str(),
                            chunk.relative_path.as_str(),
                            logical.as_str(),
                        ]
                        .into_iter()
                        .find(|&candidate| file.strip_suffix(candidate).is_some_and(|prefix| prefix.ends_with(" (")))
                        .map(|matched| (chunk, matched.to_owned()))
                    })?;
                    (matched, self.decoded_chunk_source_map(chunk)?)
                };
                let start = "    at ".len() + file.len() - matched_name.len();
                let length = matched_name.len() + line.len() + column.len() + 2;
                let line = line.parse::<u32>().ok()?.checked_sub(1)?;
                let column = column.parse::<u32>().ok()?.checked_sub(1)?;
                let token = decoded_map.lookup_token(line, column)?;
                if token.get_dst_line() != line { return None; }
                let source = token.get_source()?;
                let original_line = token.get_src_line().checked_add(1)?;
                let original_column = token.get_src_col().checked_add(1)?;
                Some((start, length, source, original_line, original_column))
            })();
            if let Some((start, length, source, line, column)) = location {
                let output = remapped.get_or_insert_with(|| {
                    let mut output = String::with_capacity(stack.len());
                    output.push_str(&stack[..offset]);
                    output
                });
                output.push_str(&frame[..start]);
                write!(output, "{source}:{line}:{column}").expect("writing to a String cannot fail");
                output.push_str(&frame[start + length..]);
            } else if let Some(output) = &mut remapped {
                output.push_str(frame);
            }
            offset += frame.len();
        }
        if let Some(stack) = remapped {
            if diagnostic.generated_stack.is_none() { diagnostic.generated_stack = diagnostic.stack.take(); }
            diagnostic.stack = Some(stack);
        }
        diagnostic
    }
}

struct PackageModuleProvider {
    package: Rc<Package>,
}

impl js_runtime::VerifiedModuleProvider for PackageModuleProvider {
    fn resolve(&self, referrer: &str, specifier: &str) -> std::result::Result<String, String> {
        self.package.resolve_sealed_import(referrer, specifier)
    }

    fn load(&self, canonical_name: &str) -> std::result::Result<js_runtime::VerifiedModulePayload, String> {
        self.package.load_sealed_module(canonical_name)
    }
}

fn normalize_posix_relative(base_dir: &str, specifier: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in base_dir.split('/').chain(specifier.split('/')) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            segment => {
                if segment.contains('\\') || segment.contains('\0') {
                    return None;
                }
                parts.push(segment);
            }
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

fn valid_worker_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes[0].is_ascii_alphabetic()
        && bytes[1..].iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
}

fn valid_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| hex.len() == 64
        && hex.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
}

fn digest(bytes: &[u8]) -> String { format!("sha256:{:x}", Sha256::digest(bytes)) }

fn canonical_bytes(value: &serde_json::Value) -> Result<Vec<u8>> {
    fn append(value: &serde_json::Value, bytes: &mut Vec<u8>) -> Result<()> {
        match value {
            serde_json::Value::Object(object) => {
                let mut entries: Vec<_> = object.iter().collect();
                entries.sort_unstable_by(|(left, _), (right, _)| left.encode_utf16().cmp(right.encode_utf16()));
                bytes.push(b'{');
                for (index, (key, value)) in entries.into_iter().enumerate() {
                    if index != 0 { bytes.push(b','); }
                    serde_json::to_writer(&mut *bytes, key)?;
                    bytes.push(b':');
                    append(value, bytes)?;
                }
                bytes.push(b'}');
            }
            serde_json::Value::Array(array) => {
                bytes.push(b'[');
                for (index, value) in array.iter().enumerate() {
                    if index != 0 { bytes.push(b','); }
                    append(value, bytes)?;
                }
                bytes.push(b']');
            }
            other => serde_json::to_writer(bytes, other)?,
        }
        Ok(())
    }
    let mut bytes = Vec::new();
    append(value, &mut bytes)?;
    Ok(bytes)
}

fn verify_inventory(root: &Path, directory: &Path, files: &BTreeMap<String, Vec<u8>>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            verify_inventory(root, &path, files)?;
        } else {
            let relative = path.strip_prefix(root)?.to_str().ok_or("package resource path is not UTF-8")?.to_owned();
            if !kind.is_file() || (relative != "manifest.json" && !files.contains_key(&relative)) {
                return Err(format!("undeclared or non-regular package resource: {relative}").into());
            }
        }
    }
    Ok(())
}

fn unique_digests<'de, D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<BTreeMap<String, String>, D::Error> {
    struct Digests;
    impl<'de> serde::de::Visitor<'de> for Digests {
        type Value = BTreeMap<String, String>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("unique artifact digest entries") }
        fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> std::result::Result<Self::Value, M::Error> {
            let mut digests = BTreeMap::new();
            while let Some((name, digest)) = map.next_entry::<String, String>()? {
                if digests.insert(name, digest).is_some() { return Err(serde::de::Error::custom("duplicate artifact digest")); }
            }
            Ok(digests)
        }
    }
    deserializer.deserialize_map(Digests)
}

fn resource(root: &Path, name: &str) -> Result<Vec<u8>> {
    bounded_fs::read_contained(root, name, MAX_RESOURCE_BYTES).map_err(|error| match error {
        BoundedReadError::InvalidPath => format!("resource must be a canonical package-relative path: {name}").into(),
        BoundedReadError::Symlink => format!("symlink resource rejected: {name}").into(),
        BoundedReadError::NotRegularFile | BoundedReadError::TooLarge { .. } => {
            format!("invalid or oversized resource: {name}").into()
        }
        BoundedReadError::Io(io) => io.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("zgui-package-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir(&dir).unwrap();
            Self(dir)
        }
        fn publish(&self, source: &[u8], map: &[u8]) {
            fs::write(self.0.join("app.mjs"), source).unwrap();
            fs::write(self.0.join("app.mjs.map"), map).unwrap();
            let style_system = serde_json::json!({"name":"lightningcss","profile":"lightningcss-v1",
                "configDigest":digest(b"test")});
            let metadata = serde_json::to_vec(&serde_json::json!({"styleSystem":style_system})).unwrap();
            fs::write(self.0.join("build-info.json"), &metadata).unwrap();
            let manifest = serde_json::json!({"schemaVersion":2,"id":"test","hostApiVersion":1,"entry":"app.mjs","sourceMap":"app.mjs.map","styles":[],"assets":[],"fonts":[],
                "styleSystem":style_system,"artifactDigests":{}});
            self.save(manifest);
        }
        fn save(&self, mut manifest: serde_json::Value) {
            let chunk_names = manifest.get("chunks").and_then(|v| v.as_array()).into_iter().flatten()
                .flat_map(|c| [c["path"].as_str().unwrap().to_owned(), c["sourceMap"].as_str().unwrap().to_owned()]);
            let worker_names = manifest.get("workers").and_then(|v| v.as_array()).into_iter().flatten()
                .flat_map(|w| [w["path"].as_str().unwrap().to_owned(), w["sourceMap"].as_str().unwrap().to_owned()]);
            let names: Vec<_> = ["entry", "sourceMap"].into_iter().map(|key| manifest[key].as_str().unwrap().to_owned())
                .chain(["styles", "assets"].into_iter().flat_map(|key| manifest[key].as_array().unwrap().iter().map(|value| value.as_str().unwrap().to_owned())))
                .chain(chunk_names)
                .chain(worker_names)
                .chain(std::iter::once("build-info.json".into())).collect();
            manifest["artifactDigests"] = names.into_iter().map(|name| {
                let hash = digest(&fs::read(self.0.join(&name)).unwrap()); (name, serde_json::json!(hash))
            }).collect::<serde_json::Map<_, _>>().into();
            manifest.as_object_mut().unwrap().remove("buildId");
            manifest["buildId"] = serde_json::json!(digest(&canonical_bytes(&manifest).unwrap()));
            fs::write(self.0.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
        }
    }
    impl Drop for Fixture { fn drop(&mut self) { fs::remove_dir_all(&self.0).unwrap(); } }

    #[test]
    fn capability_manifest_defaults_closed_and_binds_authority_to_build_id() {
        let fixture = Fixture::new();
        fixture.publish(b"export {};", br#"{"version":3,"sources":[],"names":[],"mappings":""}"#);
        let old = Package::load(&fixture.0).unwrap();
        assert!(old.manifest.capabilities.is_empty());
        assert!(serde_json::to_value(&old.manifest).unwrap().get("capabilities").is_none());

        let mut value = manifest(&fixture);
        value["capabilities"] = serde_json::json!({"imagePrefixes":["/var/media"],"sessionLock":true});
        fixture.save(value);
        let loaded = Package::load(&fixture.0).unwrap();
        assert_eq!(loaded.manifest.capabilities.image_prefixes, ["/var/media"]);
        assert!(loaded.manifest.capabilities.session_lock);
        assert_ne!(old.manifest.build_id, loaded.manifest.build_id);

        let mut value = manifest(&fixture);
        value["capabilities"] = serde_json::json!({
            "imagePrefixes": ["$XDG_RUNTIME_DIR/z-shell/media"],
            "unixSockets": ["$XDG_RUNTIME_DIR/z-shell/host-${WAYLAND_DISPLAY}.sock", "$HOME/$WAYLAND_DISPLAY", "/run/literal-$UNKNOWN"],
            "namedSockets": ["niri"]
        });
        fixture.save(value.clone());
        let portable = Package::load(&fixture.0).expect("portable manifest loads without host bindings");
        assert_eq!(serde_json::to_value(&portable.manifest.capabilities).expect("serializable authority"), value["capabilities"], "templates remain sealed verbatim");
        value["capabilities"]["namedSockets"] = serde_json::json!(["other"]);
        let sealed = manifest(&fixture);
        value["buildId"] = sealed["buildId"].clone();
        fs::write(fixture.0.join("manifest.json"), serde_json::to_vec(&value).expect("serialize tampered package")).expect("write tampered package");
        assert!(Package::load(&fixture.0).err().expect("tampered name rejected").to_string().contains("package integrity mismatch"), "named sockets are sealed");
        fixture.save(sealed);

        let mut tampered = manifest(&fixture);
        tampered["capabilities"]["imagePrefixes"] = serde_json::json!(["/"]);
        fs::write(fixture.0.join("manifest.json"), serde_json::to_vec(&tampered).unwrap()).unwrap();
        assert!(Package::load(&fixture.0).err().unwrap().to_string().contains("package integrity mismatch"));

        for invalid in [
            serde_json::json!({"imagePrefixes":["/var/../media"]}),
            serde_json::json!({"imagePrefixes":["/media","/media"]}),
            serde_json::json!({"unixSockets":["$HOME/../service"]}),
            serde_json::json!({"unixSockets":["$HOME/host-${OTHER}.sock"]}),
            serde_json::json!({"unixSockets":["$HOME/host-$WAYLAND_DISPLAY.sock"]}),
            serde_json::json!({"namedSockets":["niri","niri"]}),
            serde_json::json!({"namedSockets":["2socket"]}),
            serde_json::json!({"sessionLock":"true"}),
            serde_json::json!({"filesystem":true}),
            serde_json::json!([]),
        ] {
            let mut bad = manifest(&fixture);
            bad["capabilities"] = invalid.clone();
            fixture.save(bad);
            assert!(Package::load(&fixture.0).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn rejects_mixed_metadata_identities_even_when_every_artifact_and_build_hash_is_valid() {
        let fixture = Fixture::new();
        fixture.publish(b"export {};", br#"{"version":3,"sources":[],"names":[],"mappings":""}"#);
        let resource = "build-info.json";
        let original = fs::read(fixture.0.join(resource)).unwrap();
        for identity in [None, Some(serde_json::json!({"name":"lightningcss","profile":"other","configDigest":digest(b"other")}))] {
            let metadata = identity.map(|identity|serde_json::json!({"styleSystem":identity})).unwrap_or_else(||serde_json::json!({}));
            fs::write(fixture.0.join(resource), serde_json::to_vec(&metadata).unwrap()).unwrap();
            fixture.save(manifest(&fixture));
            assert!(Package::load(&fixture.0).err().unwrap().to_string().contains("styleSystem identity"), "accepted mixed {resource}");
        }
        fs::write(fixture.0.join(resource), &original).unwrap();
        fixture.save(manifest(&fixture));
        assert!(Package::load(&fixture.0).is_ok());
    }

    #[test]
    fn acceptance_status_is_shared_by_symlink_and_resolved_generation_loads() {
        let fixture = Fixture::new();
        fixture.publish(b"export {};", br#"{"version":3,"sources":[],"names":[],"mappings":""}"#);
        let generation = fixture.0.join(".zgui-generations/candidate");
        fs::create_dir_all(&generation).unwrap();
        for name in ["app.mjs", "app.mjs.map", "build-info.json", "manifest.json"] {
            fs::rename(fixture.0.join(name), generation.join(name)).unwrap();
        }
        let dist = fixture.0.join("dist");
        std::os::unix::fs::symlink(&generation, &dist).unwrap();
        let linked = Package::load(&dist).unwrap();
        linked.record_acceptance(&serde_json::json!({"status":"accepted","generation":1})).unwrap();
        let resolved = Package::load(&generation).unwrap();
        resolved.record_acceptance(&serde_json::json!({"status":"accepted","generation":2})).unwrap();
        let status: serde_json::Value = serde_json::from_slice(&fs::read(fixture.0.join(".zgui-native-status.json")).unwrap()).unwrap();
        assert_eq!(status["generation"], 2);
    }

    #[test]
    fn rejects_duplicate_artifact_digest_keys_instead_of_discarding_one() {
        let fixture = Fixture::new();
        fixture.publish(b"export {};", br#"{"version":3,"sources":[],"names":[],"mappings":""}"#);
        let value = manifest(&fixture);
        let raw = String::from_utf8(serde_json::to_vec(&value).unwrap()).unwrap();
        let duplicate = raw.replace("\"artifactDigests\":{", &format!("\"artifactDigests\":{{\"build-info.json\":{},", value["artifactDigests"]["build-info.json"]));
        fs::write(fixture.0.join("manifest.json"), duplicate).unwrap();
        assert!(Package::load(&fixture.0).err().unwrap().to_string().contains("duplicate artifact digest"));
    }

    #[test]
    fn native_acceptance_is_atomic_and_separate_from_publication_identity() {
        let generation = Fixture::new();
        generation.publish(b"export {};", br#"{"version":3,"sources":[],"names":[],"mappings":""}"#);
        let publication = Fixture::new();
        std::os::unix::fs::symlink(&generation.0, publication.0.join("dist")).unwrap();
        fs::write(publication.0.join(".zgui-build-status.json"), b"published").unwrap();
        let package = Package::load(&publication.0.join("dist")).unwrap();
        for status in ["accepted", "rejected"] {
            let record = serde_json::json!({"status":status,"candidate":{"buildId":package.manifest.build_id}});
            package.record_acceptance(&record).unwrap();
            let saved: serde_json::Value = serde_json::from_slice(&fs::read(publication.0.join(".zgui-native-status.json")).unwrap()).unwrap();
            assert_eq!(saved, record);
        }
        assert_eq!(fs::read(publication.0.join(".zgui-build-status.json")).unwrap(), b"published");
        assert!(!fs::read_dir(&publication.0).unwrap().any(|entry|entry.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
        assert_eq!(Package::load(&publication.0.join("dist")).unwrap().manifest.build_id, package.manifest.build_id);
    }

    fn manifest(fixture: &Fixture) -> serde_json::Value {
        serde_json::from_slice(&fs::read(fixture.0.join("manifest.json")).unwrap()).unwrap()
    }

    fn write_manifest(fixture: &Fixture, value: &serde_json::Value) {
        fs::write(fixture.0.join("manifest.json"), serde_json::to_vec(value).unwrap()).unwrap();
    }

    #[test]
    fn rejects_missing_wrong_and_undeclared_artifact_digests_and_metadata_tampering() {
        let fixture = Fixture::new();
        fixture.publish(b"export {};", br#"{"version":3,"sources":[],"names":[],"mappings":""}"#);
        let original = manifest(&fixture);
        for (resource, mutation) in [
            ("app.mjs", serde_json::Value::Null),
            ("app.mjs.map", serde_json::json!(digest(b"wrong"))),
            ("build-info.json", serde_json::json!(digest(b"wrong"))),
            ("undeclared", serde_json::json!(digest(b"extra"))),
        ] {
            let mut value = original.clone();
            if mutation.is_null() { value["artifactDigests"].as_object_mut().unwrap().remove(resource); }
            else { value["artifactDigests"][resource] = mutation; }
            write_manifest(&fixture, &value);
            assert!(Package::load(&fixture.0).is_err(), "accepted altered digest declaration {resource}");
        }
        let mut value = original.clone();
        value["id"] = serde_json::json!("metadata changed without resealing");
        write_manifest(&fixture, &value);
        assert!(Package::load(&fixture.0).err().unwrap().to_string().contains("package integrity mismatch"));
        write_manifest(&fixture, &original);
        fs::write(fixture.0.join("ambient.txt"), b"not declared").unwrap();
        assert!(Package::load(&fixture.0).err().unwrap().to_string().contains("undeclared"));
    }

    #[test]
    fn preserves_sheet_order_and_requires_declared_font_assets() {
        let fixture = Fixture::new();
        fixture.publish(b"export {};", br#"{"version":3,"sources":[],"names":[],"mappings":""}"#);
        fs::write(fixture.0.join("base.css"), b".base {color:red}").unwrap();
        fs::write(fixture.0.join("app.css"), b".app {color:blue}").unwrap();
        fs::write(fixture.0.join("font.ttf"), b"verified font bytes").unwrap();
        let mut value = manifest(&fixture);
        value["styles"] = serde_json::json!(["base.css", "app.css"]);
        value["assets"] = serde_json::json!(["font.ttf"]);
        value["fonts"] = serde_json::json!([{"family":"App Sans","path":"font.ttf"}]);
        fixture.save(value.clone());
        let package = Package::load(&fixture.0).unwrap();
        assert_eq!(package.styles.iter().map(|sheet| &*sheet.path).collect::<Vec<_>>(), ["base.css","app.css"]);
        assert_eq!(&*package.styles[0].css, ".base {color:red}");
        value["styles"] = serde_json::json!(["app.css", "base.css"]);
        fixture.save(value.clone());
        let reversed = Package::load(&fixture.0).unwrap();
        assert_ne!(package.manifest.build_id, reversed.manifest.build_id);
        assert_eq!(&*reversed.styles[0].path, "app.css");
        value["fonts"][0]["path"] = serde_json::json!("app.css");
        fixture.save(value);
        assert!(Package::load(&fixture.0).err().unwrap().to_string().contains("undeclared font"));
    }

    #[test]
    fn rejects_closed_manifest_violations_and_confined_paths() {
        let fixture = Fixture::new();
        fixture.publish(b"export {};", br#"{"version":3,"sources":[],"names":[],"mappings":""}"#);
        let original = manifest(&fixture);
        for id in ["", " test", "test\u{feff}", "test\nname", "test\u{009f}name"] {
            let mut value = original.clone();
            value["id"] = serde_json::json!(id);
            fixture.save(value);
            let error = Package::load(&fixture.0).err().expect("invalid package ID rejected");
            assert!(error.to_string().contains("package ID"), "{id:?}: {error}");
        }
        for path in ["../outside", "/etc/passwd", "a/../app.mjs", "a//b", "a\\\\b"] {
            let mut value = original.clone();
            value["entry"] = serde_json::json!(path);
            value["artifactDigests"][path] = serde_json::json!(digest(b"unused"));
            write_manifest(&fixture, &value);
            assert!(Package::load(&fixture.0).err().unwrap().to_string().contains("package-relative"), "{path}");
        }
        for value in [
            { let mut value = original.clone(); value["unknown"] = serde_json::json!(true); value },
            { let mut value = original.clone(); value["styleSystem"]["profile"] = serde_json::json!("browser"); value },
            { let mut value = original.clone(); value["styleSystem"]["configDigest"] = serde_json::json!("sha256:no"); value },
        ] {
            write_manifest(&fixture, &value);
            assert!(Package::load(&fixture.0).is_err());
        }
    }

    #[test]
    fn canonical_identity_matches_javascript_utf16_key_order() {
        let value = serde_json::json!({"\u{e000}":[{"z":1,"a":2}],"\u{1f600}":"astral"});
        assert_eq!(String::from_utf8(canonical_bytes(&value).unwrap()).unwrap(),
            "{\"😀\":\"astral\",\"\u{e000}\":[{\"a\":2,\"z\":1}]}");
    }

    #[test]
    fn scoped_font_leases_preserve_successors_restore_profile_fonts_and_reject_malformed_bytes() {
        let fonts = zgui::app::Fonts::shipped_only();
        let data: zgui::text::FontData = Arc::new(include_bytes!("../../../preview/fonts/NotoSans-Regular.ttf").as_slice());
        let first = fonts.register_scoped(data.clone(), "Application Sans").unwrap();
        let successor = fonts.register_scoped(data.clone(), "Application Sans").unwrap();
        drop(first);
        assert!(fonts.contains_family("Application Sans"), "retirement withdrew the successor");
        assert!(!fonts.contains_family("Application Snas"), "misspelled alias must not use fallback");
        assert!(fonts.register_scoped(Arc::new(b"not a font"), "Application Sans").is_err());
        assert!(fonts.contains_family("Application Sans"), "rejected registration damaged active font");
        drop(successor);
        assert!(!fonts.contains_family("Application Sans"), "last lease must unregister the alias");
        fonts.register(data.clone(), Some("Profile Sans")).unwrap();
        let temporary = fonts.register_scoped(data, "Profile Sans").unwrap();
        drop(temporary);
        assert!(fonts.contains_family("Profile Sans"), "profile registration must survive package cleanup");
    }
    #[test]
    fn rejects_traversal_and_symlink_escape_before_reading() {
        let fixture = Fixture::new();
        assert!(resource(&fixture.0, "../outside").is_err());
        assert!(resource(&fixture.0, "/etc/passwd").is_err());
        std::os::unix::fs::symlink("/etc", fixture.0.join("outside")).unwrap();
        assert!(resource(&fixture.0, "outside/passwd").is_err());
    }
    #[test]
    fn verifies_complete_declared_content_before_loading() {
        let fixture = Fixture::new();
        let source = b"export const hostApiVersion = 1; export function mount() { return () => {}; }";
        let map = br#"{"version":3,"sources":[],"names":[],"mappings":""}"#;
        fixture.publish(source, map);
        let package = Package::load(&fixture.0).unwrap();
        // Consuming the initial buffer releases retained JS source and reloads verified bytes on demand.
        assert!(matches!(package.take_or_load_entry().unwrap(), EntryPayload::Source(s) if s.as_bytes() == source));
        assert!(package.entry_payload.borrow().is_none(), "initial source buffer should be released after take");
        assert!(matches!(package.take_or_load_entry().unwrap(), EntryPayload::Source(s) if s.as_bytes() == source));
        assert!(package.entry_payload.borrow().is_none(), "reloaded retry source should not be retained permanently");
        assert!(package.read_verified_json("build-info.json").is_ok());
        fs::write(fixture.0.join("app.mjs"), b"tampered").unwrap();
        assert!(package.take_or_load_entry().err().unwrap().to_string().contains("integrity mismatch"));
        fs::write(fixture.0.join("app.mjs"), source).unwrap();
        fs::write(fixture.0.join("app.mjs.map"), b"modified").unwrap();
        assert!(Package::load(&fixture.0).err().unwrap().to_string().contains("integrity mismatch"));
    }

    #[test]
    fn declared_asset_access_is_verified_owned_and_excludes_other_package_files() {
        let fixture = Fixture::new();
        let source = b"export {};";
        let map = br#"{"version":3,"sources":[],"names":[],"mappings":""}"#;
        fixture.publish(source, map);
        let payload = b"verified image or text bytes";
        fs::write(fixture.0.join("asset.bin"), payload).unwrap();
        let mut manifest: serde_json::Value = serde_json::from_slice(&fs::read(fixture.0.join("manifest.json")).unwrap()).unwrap();
        manifest["assets"] = serde_json::json!(["asset.bin"]);
        fixture.save(manifest);
        let package = Package::load(&fixture.0).unwrap();
        assert_eq!(package.asset_bytes("asset.bin"), Some(payload.as_slice()));
        for name in ["ambient.bin", "app.mjs", "app.mjs.map", "manifest.json", "../asset.bin"] {
            assert_eq!(package.asset_bytes(name), None);
        }
        fs::write(fixture.0.join("asset.bin"), b"replaced after verification").unwrap();
        assert_eq!(package.asset_bytes("asset.bin"), Some(payload.as_slice()));
        let retained = package.image_assets().next().unwrap().1;
        assert_eq!(retained.as_ref(), payload);
        assert!(Package::load(&fixture.0).is_err(), "a new load still verifies the changed bytes");
    }

    #[test]
    fn rejects_malformed_integrity_verified_source_maps() {
        let fixture = Fixture::new();
        for map in [b"not JSON".as_slice(), b"{}", br#"{"version":3,"sources":["main.tsx"],"names":[],"mappings":"!"}"#] {
            fixture.publish(b"export {};", map);
            let error = Package::load(&fixture.0).err().unwrap().to_string();
            assert!(error.contains("invalid source map app.mjs.map"), "{error}");
        }
    }

    #[test]
    fn sealed_chunk_graph_verifies_digests_blocks_escapes_and_remaps_lazy_stacks() {
        let fixture = Fixture::new();
        let entry_source = b"export const hostApiVersion = 1; export function mount() { return () => {}; }";
        let empty_map = br#"{"version":3,"sources":[],"names":[],"mappings":""}"#;
        fixture.publish(entry_source, empty_map);

        fs::create_dir_all(fixture.0.join("chunks")).unwrap();
        let chunk_source = b"export function boom() { throw new Error('lazy boom'); }\n";
        let chunk_map = br#"{"version":3,"sources":["src/lazy.tsx"],"names":[],"mappings":"AAAA"}"#;
        fs::write(fixture.0.join("chunks/lazy-1.mjs"), chunk_source).unwrap();
        fs::write(fixture.0.join("chunks/lazy-1.mjs.map"), chunk_map).unwrap();

        let mut manifest: serde_json::Value = serde_json::from_slice(&fs::read(fixture.0.join("manifest.json")).unwrap()).unwrap();
        manifest["chunks"] = serde_json::json!([{
            "path": "chunks/lazy-1.mjs",
            "sourceMap": "chunks/lazy-1.mjs.map",
        }]);
        fixture.save(manifest);

        let package = Rc::new(Package::load(&fixture.0).expect("multi-chunk package loads"));
        let provider = package.module_provider();

        let resolved = provider.resolve(package.generated_entry(), "./chunks/lazy-1.mjs").expect("resolve");
        assert!(resolved.ends_with("chunks/lazy-1.mjs"));
        assert!(matches!(provider.load(&resolved).expect("load chunk"), js_runtime::VerifiedModulePayload::Source(s) if s.as_bytes() == chunk_source));

        assert!(provider.resolve(package.generated_entry(), "../outside.mjs").is_err());
        assert!(provider.resolve(package.generated_entry(), "node:fs").is_err());
        assert!(provider.resolve(package.generated_entry(), "./chunks/other.mjs").is_err());

        let remapped = package.remap_diagnostic(js_runtime::Diagnostic {
            kind: js_runtime::DiagnosticKind::Exception,
            phase: "entry",
            message: "lazy boom".into(),
            stack: Some(format!("    at boom ({resolved}:1:1)\n")),
            generated_stack: None,
        });
        assert!(remapped.stack.as_deref().unwrap_or("").contains("src/lazy.tsx:1:1"), "{:?}", remapped.stack);

        fs::write(fixture.0.join("chunks/lazy-1.mjs"), b"export const hacked = 1;").unwrap();
        assert!(provider.load(&resolved).unwrap_err().contains("integrity mismatch"));
        assert!(Package::load(&fixture.0).err().unwrap().to_string().contains("integrity mismatch"));
    }

    #[test]
    fn sealed_workers_verify_digests_and_remap_stacks() {
        let fixture = Fixture::new();
        let entry_source = b"export const hostApiVersion = 1; export function mount() { return () => {}; }";
        let empty_map = br#"{"version":3,"sources":[],"names":[],"mappings":""}"#;
        fixture.publish(entry_source, empty_map);

        fs::create_dir_all(fixture.0.join("workers")).unwrap();
        let worker_source = b"export function onMessage(x) { postMessage(x); }\n";
        let worker_map = br#"{"version":3,"sources":["src/workers/compute.ts"],"names":[],"mappings":"AAAA"}"#;
        fs::write(fixture.0.join("workers/compute.mjs"), worker_source).unwrap();
        fs::write(fixture.0.join("workers/compute.mjs.map"), worker_map).unwrap();

        let mut manifest: serde_json::Value = serde_json::from_slice(&fs::read(fixture.0.join("manifest.json")).unwrap()).unwrap();
        manifest["workers"] = serde_json::json!([{
            "name": "compute",
            "path": "workers/compute.mjs",
            "sourceMap": "workers/compute.mjs.map",
        }]);
        fixture.save(manifest);

        let package = Package::load(&fixture.0).expect("worker package loads");
        let (canonical, loaded) = package.load_worker_source("compute").expect("load worker");
        assert!(canonical.ends_with("workers/compute.mjs"));
        assert_eq!(loaded.as_bytes(), worker_source);
        assert!(package.load_worker_source("undeclared").unwrap_err().contains("undeclared"));

        let remapped = package.remap_diagnostic(js_runtime::Diagnostic {
            kind: js_runtime::DiagnosticKind::Exception,
            phase: "worker",
            message: "worker boom".into(),
            stack: Some(format!("    at onMessage ({canonical}:1:1)\n")),
            generated_stack: None,
        });
        assert!(remapped.stack.as_deref().unwrap_or("").contains("src/workers/compute.ts:1:1"), "{:?}", remapped.stack);

        fs::write(fixture.0.join("workers/compute.mjs"), b"export const tampered = true;").unwrap();
        assert!(package.load_worker_source("compute").unwrap_err().contains("integrity mismatch"));
        assert!(Package::load(&fixture.0).err().unwrap().to_string().contains("integrity mismatch"));
    }

    #[test]
    fn remaps_actual_module_stack_and_preserves_other_diagnostics() {
        let fixture = Fixture::new();
        let source = "function fail() {\n  throw new Error('module failure');\n}\nexport function runSmoke() {\n  return fail();\n}\n";
        let mut builder = sourcemap::SourceMapBuilder::new(Some("app.mjs"));
        builder.add(1, 0, 0, 0, None, None, false);
        builder.add(1, 12, 41, 7, Some("src/Counter.tsx"), None, false);
        builder.add(1, 13, 98, 98, Some("wrong-column.tsx"), None, false);
        builder.add(2, 12, 98, 98, Some("wrong-line.tsx"), None, false);
        builder.add(4, 9, 51, 3, Some("src/Counter.tsx"), None, false);
        let mut map = Vec::new();
        builder.into_sourcemap().to_writer(&mut map).unwrap();
        fixture.publish(source.as_bytes(), &map);
        let package = Package::load(&fixture.0).unwrap();
        assert!(package.source_map.get().is_none(), "source map must stay unloaded until an entry frame remaps");
        let mut engine = js_runtime::JsRuntime::new(Default::default()).unwrap();
        let foreign = engine.evaluate_script(
            "function foreign() { return new Error('foreign').stack; }\nforeign();",
        ).unwrap();
        assert!(foreign.contains("    at foreign (eval_script:1:"), "{foreign}");
        let plain = js_runtime::Diagnostic {
            kind: js_runtime::DiagnosticKind::Engine,
            phase: "test",
            message: "leave app.mjs:2:13 in the message".into(),
            stack: Some(foreign.clone()),
            generated_stack: None,
        };
        assert_eq!(package.remap_diagnostic(plain.clone()), plain);
        assert!(package.source_map.get().is_none(), "unrelated stack must not decode source map");
        let no_stack = js_runtime::Diagnostic { stack: None, ..plain.clone() };
        assert_eq!(package.remap_diagnostic(no_stack.clone()), no_stack);
        let entry = package.directory.join(&package.manifest.entry).to_string_lossy().into_owned();
        for name in [&*entry, "app.mjs"] {
            let mut diagnostic = engine.run_smoke_module(name, source).unwrap_err();
            assert_eq!(diagnostic.stack.as_deref(), Some(format!(
                "    at fail ({name}:2:13)\n    at runSmoke ({name}:5:10)\n",
            ).as_str()));
            let unrelated = concat!(
                "    at helper (foreign/app.mjs:2:13)\n",
                "    at invalid (app.mjs:0:13)\n",
                "    at unmapped (app.mjs:8:1)\n",
                "    at boundary (app.mjs:2:12)\n",
                "not a frame (app.mjs:2:13)\n",
                "    at truncated (app.mjs:2:13",
            );
            diagnostic.stack.as_mut().unwrap().push_str(&foreign);
            diagnostic.stack.as_mut().unwrap().push_str(unrelated);
            let original = diagnostic.clone();
            let mapped = package.remap_diagnostic(diagnostic);
            assert!(package.source_map.get().is_some(), "matching frame should lazily decode source map");
            assert_eq!(mapped.message, original.message);
            assert_eq!(mapped.kind, original.kind);
            assert_eq!(mapped.phase, original.phase);
            assert_eq!(mapped.generated_stack, original.stack);
            assert_eq!(package.remap_diagnostic(mapped.clone()), mapped);
            assert_eq!(mapped.stack.unwrap(), format!(
                "    at fail (src/Counter.tsx:42:8)\n    at runSmoke (src/Counter.tsx:52:4)\n{foreign}{unrelated}",
            ));
        }
        // A fresh package refuses to lazily load a source map tampered on disk after load.
        let tampered_package = Package::load(&fixture.0).unwrap();
        fs::write(fixture.0.join("app.mjs.map"), b"{}").unwrap();
        let raw = engine.run_smoke_module("app.mjs", source).unwrap_err();
        assert_eq!(tampered_package.remap_diagnostic(raw.clone()), raw);
    }

    #[test]
    fn bytecode_package_verifies_abi_digests_chunks_workers_and_remaps_stacks() {
        let fixture = Fixture::new();
        let entry_source = "export const hostApiVersion = 1;\nexport function fail() { throw new Error('bc-fail'); }\nexport function mount() { return () => {}; }\n";
        let chunk_source = "export const chunkValue = 'bc-chunk-ok';\n";
        let worker_source = "export function onMessage(x) { postMessage(x); }\n";

        let entry_bc = js_runtime::compile_module_bytecode("app.mjs", entry_source).expect("compile entry");
        let chunk_bc = js_runtime::compile_module_bytecode("chunks/lazy.mjs", chunk_source).expect("compile chunk");
        let worker_bc = js_runtime::compile_module_bytecode("workers/calc.mjs", worker_source).expect("compile worker");

        let mut builder = sourcemap::SourceMapBuilder::new(Some("app.mjs"));
        builder.add(1, 0, 19, 0, Some("src/Main.tsx"), None, false);
        builder.add(1, 25, 19, 10, Some("src/Main.tsx"), None, false);
        let mut entry_map = Vec::new();
        builder.into_sourcemap().to_writer(&mut entry_map).unwrap();
        let empty_map = br#"{"version":3,"sources":["src/lazy.ts"],"names":[],"mappings":"AAAA"}"#;

        fixture.publish(b"placeholder", &entry_map);
        let _ = fs::remove_file(fixture.0.join("app.mjs"));
        fs::write(fixture.0.join("app.qjsb"), &entry_bc).unwrap();
        fs::create_dir_all(fixture.0.join("chunks")).unwrap();
        fs::write(fixture.0.join("chunks/lazy.qjsb"), &chunk_bc).unwrap();
        fs::write(fixture.0.join("chunks/lazy.mjs.map"), empty_map).unwrap();
        fs::create_dir_all(fixture.0.join("workers")).unwrap();
        fs::write(fixture.0.join("workers/calc.qjsb"), &worker_bc).unwrap();
        fs::write(fixture.0.join("workers/calc.mjs.map"), empty_map).unwrap();

        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(fixture.0.join("manifest.json")).unwrap()).unwrap();
        manifest["entry"] = serde_json::json!("app.qjsb");
        manifest["bytecodeFormat"] = serde_json::json!(js_runtime::BYTECODE_FORMAT);
        manifest["quickjsAbi"] = serde_json::json!(js_runtime::quickjs_abi_tag());
        manifest["chunks"] = serde_json::json!([{
            "path": "chunks/lazy.qjsb",
            "sourceMap": "chunks/lazy.mjs.map",
        }]);
        manifest["workers"] = serde_json::json!([{
            "name": "calc",
            "path": "workers/calc.qjsb",
            "sourceMap": "workers/calc.mjs.map",
        }]);
        fixture.save(manifest.clone());

        let pkg = Rc::new(Package::load(&fixture.0).expect("bytecode package loads"));
        assert!(pkg.is_bytecode());
        let resolved_chunk = pkg
            .resolve_sealed_import("app.mjs", "./chunks/lazy.mjs")
            .expect("resolve logical .mjs chunk import to sealed .qjsb");
        assert!(resolved_chunk.ends_with("chunks/lazy.qjsb"));
        let loaded_chunk = pkg.load_sealed_module(&resolved_chunk).expect("load chunk bytecode");
        assert!(matches!(loaded_chunk, js_runtime::VerifiedModulePayload::Bytecode(b) if b == chunk_bc));

        let (worker_id, worker_payload) = pkg.load_worker_payload("calc").expect("load worker bytecode");
        assert_eq!(worker_id, "workers/calc.mjs");
        assert!(matches!(worker_payload, js_runtime::VerifiedModulePayload::Bytecode(b) if b == worker_bc));

        // Execute bytecode entry and confirm sidecar source map remaps stack trace cleanly.
        let mut engine = js_runtime::JsRuntime::new(Default::default()).unwrap();
        engine.set_module_provider(pkg.module_provider());
        let EntryPayload::Bytecode(loaded_entry) = pkg.take_or_load_entry().expect("entry") else {
            panic!("expected bytecode entry");
        };
        engine.load_bytecode_module(&loaded_entry).expect("eval bytecode");
        let err = engine
            .with_module(|_ctx, m| {
                let fail: rquickjs::Function = m.get("fail")?;
                fail.call::<_, ()>(())
            })
            .expect_err("fail throws");
        let remapped = pkg.remap_diagnostic(err);
        assert!(
            remapped.stack.as_deref().unwrap_or("").contains("src/Main.tsx:20:"),
            "expected remapped stack from bytecode: {:?}",
            remapped.stack
        );

        // Wrong quickjsAbi fails closed before QuickJS touches bytes.
        let mut bad_abi = manifest.clone();
        bad_abi["quickjsAbi"] = serde_json::json!("qjs-ng-incompatible-abi");
        fixture.save(bad_abi);
        let abi_err = Package::load(&fixture.0).err().unwrap().to_string();
        assert!(abi_err.contains("incompatible quickjsAbi"), "unexpected abi error: {abi_err}");

        // Restore valid manifest, then tamper .qjsb bytes -> fail closed on digest mismatch.
        fixture.save(manifest);
        let mut corrupted = entry_bc.clone();
        corrupted[0] ^= 0xff;
        fs::write(fixture.0.join("app.qjsb"), &corrupted).unwrap();
        let tamper_err = Package::load(&fixture.0).err().unwrap().to_string();
        assert!(tamper_err.contains("integrity mismatch"), "unexpected tamper error: {tamper_err}");
    }
}
