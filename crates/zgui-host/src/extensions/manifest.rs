//! Verified extension overlay manifest, ABI compatibility, SHA-256 verification,
//! extension CSS preflight, and in-memory closed import graph digest calculation.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    sync::Arc,
};

use sha2::{Digest, Sha256};

use crate::{
    bounded_fs::{self, BoundedReadError},
    package::{Capabilities, Stylesheet},
};

pub(crate) const OVERLAY_SCHEMA_VERSION: u32 = 1;
const MAX_OVERLAY_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
const MAX_BUILD_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
const MAX_EXTENSION_MODULE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_EXTENSION_STYLE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SharedRuntimeAbi {
    pub major: u32,
    pub minor: u32,
}

impl SharedRuntimeAbi {
    pub(crate) const CURRENT: Self = Self { major: 1, minor: 0 };

    pub(crate) fn parse(raw: &str) -> Result<Self, String> {
        let (maj, min) = raw
            .split_once('.')
            .ok_or_else(|| format!("invalid shared_runtime_abi {raw:?}: expected <major>.<minor>"))?;
        if maj.is_empty()
            || min.is_empty()
            || !maj.bytes().all(|b| b.is_ascii_digit())
            || !min.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(format!("invalid shared_runtime_abi {raw:?}: expected numeric <major>.<minor>"));
        }
        let major: u32 = maj
            .parse()
            .map_err(|_| format!("invalid shared_runtime_abi major in {raw:?}"))?;
        let minor: u32 = min
            .parse()
            .map_err(|_| format!("invalid shared_runtime_abi minor in {raw:?}"))?;
        Ok(Self { major, minor })
    }

    /// Extensions compiled for the same major ABI and an equal or lower minor ABI are compatible.
    #[must_use]
    pub(crate) const fn is_compatible_with(self, extension: Self) -> bool {
        self.major == extension.major && self.minor >= extension.minor
    }
}

impl std::fmt::Display for SharedRuntimeAbi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExtensionsOverlayManifest {
    pub schema_version: u32,
    pub overlay_id: String,
    #[serde(default)]
    pub rollback_allowed_extensions: Vec<String>,
    #[serde(default)]
    pub effective_host_capabilities: Capabilities,
    #[serde(default)]
    pub shared_runtime_modules: Vec<String>,
    #[serde(default)]
    pub extensions: Vec<OverlayExtensionSpec>,
    #[serde(default)]
    pub client_metadata: Option<serde_json::Value>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OverlayExtensionSpec {
    pub id: String,
    pub version: String,
    pub shared_runtime_abi: String,
    pub content_digest: String,
    pub bundle_root: String,
    pub build_manifest: String,
    #[serde(default)]
    pub styles: Vec<String>,
    #[serde(default)]
    pub entries: Vec<OverlayExtensionEntry>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OverlayExtensionEntry {
    pub entry_id: String,
    pub kind: String,
    pub bundle: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExtensionBuildManifest {
    #[serde(default = "default_build_schema")]
    pub schema_version: u32,
    pub shared_runtime_abi: String,
    #[serde(default, alias = "artifactDigests", alias = "artifact_digests")]
    pub files: BTreeMap<String, String>,
}

fn default_build_schema() -> u32 {
    1
}

/// Best-effort rollback policy extracted from raw overlay JSON even if full verification failed.
#[derive(Clone, Debug, Default)]
pub(crate) struct FailedOverlayMeta {
    pub overlay_id: Option<String>,
    pub rollback_allowed_extensions: Vec<String>,
    pub client_metadata: Option<serde_json::Value>,
}

pub(crate) fn extract_raw_overlay_metadata(bytes: &[u8]) -> FailedOverlayMeta {
    let Ok(val) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return FailedOverlayMeta::default();
    };
    let overlay_id = val
        .get("overlay_id")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned);
    let rollback_allowed_extensions = val
        .get("rollback_allowed_extensions")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let client_metadata = val.get("client_metadata").cloned();
    FailedOverlayMeta {
        overlay_id,
        rollback_allowed_extensions,
        client_metadata,
    }
}

#[derive(Clone, Debug)]
pub(crate) struct VerifiedExtensionModule {
    pub extension_id: String,
    pub rel_path: String,
    pub source: Arc<str>,
    pub sha256: String,
    pub resolved_import_graph_digest: String,
}

#[derive(Clone, Debug)]
pub(crate) struct VerifiedExtensionEntry {
    pub entry_id: String,
    pub extension_id: String,
    pub version: String,
    pub kind: String,
    pub bundle: String,
    pub canonical_specifier: String,
}

#[derive(Clone)]
pub(crate) struct VerifiedOverlay {
    pub overlay_id: String,
    pub raw_sha256: [u8; 32],
    pub rollback_allowed_extensions: Vec<String>,
    pub effective_host_capabilities: Capabilities,
    pub shared_runtime_modules: Vec<String>,
    pub extension_ids: Vec<String>,
    pub styles: Arc<[Stylesheet]>,
    pub entries_by_id: BTreeMap<String, VerifiedExtensionEntry>,
    pub entries_by_specifier: BTreeSet<String>,
    pub modules_by_name: BTreeMap<String, VerifiedExtensionModule>,
    pub extension_module_paths: BTreeMap<String, BTreeSet<String>>,
    pub client_metadata: Option<serde_json::Value>,
}

pub(crate) fn is_valid_extension_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 128 || id.contains('\\') || id.contains('\0') {
        return false;
    }
    let mut parts = id.split('/');
    let Some(first) = parts.next() else {
        return false;
    };
    let rest: Vec<&str> = parts.collect();
    if rest.len() > 1 {
        return false;
    }
    std::iter::once(first).chain(rest).all(|seg| {
        !seg.is_empty()
            && seg != "."
            && seg != ".."
            && seg
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    })
}

fn valid_sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

fn sha256_prefixed(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let sum: [u8; 32] = Sha256::digest(bytes).into();
    let mut out = String::with_capacity(7 + 64);
    out.push_str("sha256:");
    for b in sum {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn normalize_relative_path(path: &str) -> Option<String> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') || path.contains('\0') {
        return None;
    }
    let mut parts = Vec::new();
    for seg in path.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return None;
        }
        parts.push(seg);
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

pub(crate) fn resolve_relative_in_extension(base_rel_path: &str, specifier: &str) -> Option<String> {
    if !specifier.starts_with("./") && !specifier.starts_with("../") {
        return None;
    }
    let base_dir = base_rel_path.rsplit_once('/').map_or("", |(dir, _)| dir);
    let mut parts: Vec<&str> = if base_dir.is_empty() {
        Vec::new()
    } else {
        base_dir.split('/').collect()
    };
    for seg in specifier.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => {
                if other.contains('\\') || other.contains('\0') {
                    return None;
                }
                parts.push(other);
            }
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn read_verified_file(root: &Path, rel: &str, expected_digest: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
    if rel.to_ascii_lowercase().ends_with(".qjsb") {
        return Err(format!("extension artifact {rel:?} uses forbidden .qjsb bytecode"));
    }
    if !valid_sha256_digest(expected_digest) {
        return Err(format!("invalid sha256 digest for {rel:?}: {expected_digest:?}"));
    }
    let bytes = bounded_fs::read_contained(root, rel, max_bytes).map_err(|err| match err {
        BoundedReadError::InvalidPath => format!("invalid relative extension path: {rel:?}"),
        BoundedReadError::Symlink => format!("symlink extension artifact rejected: {rel:?}"),
        BoundedReadError::NotRegularFile | BoundedReadError::TooLarge { .. } => {
            format!("invalid or oversized extension artifact: {rel:?}")
        }
        BoundedReadError::Io(io) => format!("failed reading extension artifact {rel:?}: {io}"),
    })?;
    let actual = sha256_prefixed(&bytes);
    if actual != expected_digest {
        return Err(format!(
            "extension artifact integrity mismatch for {rel:?}: expected {expected_digest}, got {actual}"
        ));
    }
    Ok(bytes)
}

/// Validates extension CSS before mounting: passes native Stylo/URL audits and blocks global/root rules.
pub(crate) fn validate_extension_stylesheet(css: &str) -> Result<(), String> {
    if let Err(reason) = zgui::runtime::image_policy::audit_stylesheet_image_urls(css) {
        return Err(format!("extension stylesheet image URL rejected: {reason}"));
    }
    zgui_style::preflight_stylesheet(css)
        .map_err(|diag| format!("extension stylesheet rejected: {diag:?}"))?;
    reject_global_selectors(css)
}

fn reject_global_selectors(css: &str) -> Result<(), String> {
    let bytes = css.as_bytes();
    let mut i = 0;
    let mut depth = 0i32;
    let mut at_keyframes = false;
    let mut header = String::new();
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        if b == b'"' || b == b'\'' {
            let quote = b;
            header.push(b as char);
            i += 1;
            while i < bytes.len() {
                let c = bytes[i];
                header.push(c as char);
                i += 1;
                if c == b'\\' && i < bytes.len() {
                    header.push(bytes[i] as char);
                    i += 1;
                } else if c == quote {
                    break;
                }
            }
            continue;
        }
        match b {
            b'{' => {
                let header_text = std::mem::take(&mut header);
                let trimmed = header_text.trim();
                if trimmed.starts_with('@') {
                    let lower = trimmed.to_ascii_lowercase();
                    if lower.starts_with("@keyframes") {
                        at_keyframes = true;
                    }
                } else if !at_keyframes {
                    check_selector_list(trimmed)?;
                }
                depth += 1;
            }
            b'}' => {
                depth = (depth - 1).max(0);
                if depth == 0 {
                    at_keyframes = false;
                }
                header.clear();
            }
            b';' if depth == 0 => {
                header.clear();
            }
            _ => {
                header.push(b as char);
            }
        }
        i += 1;
    }
    Ok(())
}

fn check_selector_list(selector_list: &str) -> Result<(), String> {
    for raw in selector_list.split(',') {
        let selector = raw.trim();
        if selector.is_empty() {
            continue;
        }
        if selector.contains(":root") {
            return Err("extension stylesheets must not style :root".into());
        }
        // Require at least one class, attribute, or id anchor in every rule selector.
        if !selector.contains('.') && !selector.contains('[') && !selector.contains('#') {
            return Err(format!(
                "extension stylesheet selector {selector:?} must be scoped to a class, attribute, or id"
            ));
        }
    }
    Ok(())
}

/// Lexer-aware scan for static ESM `import ... from "spec"` / `import "spec"` / `export ... from "spec"`.
pub(crate) fn scan_static_esm_imports(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let tokens = tokenize_esm(source);
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Ident(kw) if kw == "import" => {
                // `import "side-effect"`
                if let Some(Token::Str(spec)) = tokens.get(i + 1) {
                    out.push(spec.clone());
                    i += 2;
                    continue;
                }
                // Skip dynamic `import(` and `import.meta`
                if matches!(tokens.get(i + 1), Some(Token::Punct('(' | '.'))) {
                    i += 1;
                    continue;
                }
                // Walk forward to `from "spec"`
                let mut j = i + 1;
                let mut brace_depth = 0usize;
                while j < tokens.len() {
                    match &tokens[j] {
                        Token::Ident(f) if f == "from" && brace_depth == 0 => {
                            if let Some(Token::Str(spec)) = tokens.get(j + 1) {
                                out.push(spec.clone());
                            }
                            i = j + 1;
                            break;
                        }
                        Token::BraceOpen => brace_depth += 1,
                        Token::BraceClose => brace_depth = brace_depth.saturating_sub(1),
                        Token::Punct(';') if brace_depth == 0 => {
                            i = j;
                            break;
                        }
                        _ => {}
                    }
                    j += 1;
                }
            }
            Token::Ident(kw) if kw == "export" => {
                let mut j = i + 1;
                while j < tokens.len() {
                    match &tokens[j] {
                        Token::Ident(f) if f == "from" => {
                            if let Some(Token::Str(spec)) = tokens.get(j + 1) {
                                out.push(spec.clone());
                            }
                            i = j + 1;
                            break;
                        }
                        Token::Punct(';') => {
                            i = j;
                            break;
                        }
                        Token::Ident(other)
                            if matches!(
                                other.as_str(),
                                "const" | "let" | "var" | "function" | "class" | "async" | "default"
                            ) =>
                        {
                            break;
                        }
                        _ => j += 1,
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

enum Token {
    Ident(String),
    Str(String),
    Punct(char),
    BraceOpen,
    BraceClose,
}

fn tokenize_esm(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        if b == b'"' || b == b'\'' {
            let quote = b;
            i += 1;
            let mut val = String::new();
            while i < bytes.len() {
                let c = bytes[i];
                i += 1;
                if c == b'\\' && i < bytes.len() {
                    val.push(bytes[i] as char);
                    i += 1;
                } else if c == quote {
                    break;
                } else {
                    val.push(c as char);
                }
            }
            tokens.push(Token::Str(val));
            continue;
        }
        if b == b'`' {
            i += 1;
            while i < bytes.len() {
                let c = bytes[i];
                i += 1;
                if c == b'\\' && i < bytes.len() {
                    i += 1;
                } else if c == b'`' {
                    break;
                }
            }
            continue;
        }
        if b.is_ascii_alphabetic() || b == b'_' || b == b'$' {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'$') {
                i += 1;
            }
            tokens.push(Token::Ident(source[start..i].to_owned()));
            continue;
        }
        match b {
            b'{' => tokens.push(Token::BraceOpen),
            b'}' => tokens.push(Token::BraceClose),
            b'(' | b'.' | b';' => tokens.push(Token::Punct(b as char)),
            _ => {}
        }
        i += 1;
    }
    tokens
}

impl VerifiedOverlay {
    pub(crate) fn load_from_path(
        path: &Path,
        host_abi: SharedRuntimeAbi,
    ) -> Result<Self, (String, FailedOverlayMeta)> {
        let meta = fs::symlink_metadata(path).map_err(|err| {
            (
                format!("cannot stat extensions overlay {}: {err}", path.display()),
                FailedOverlayMeta::default(),
            )
        })?;
        if meta.file_type().is_symlink() || !meta.file_type().is_file() {
            return Err((
                format!("extensions overlay {} must be a regular non-symlink file", path.display()),
                FailedOverlayMeta::default(),
            ));
        }
        if meta.len() > MAX_OVERLAY_MANIFEST_BYTES {
            return Err((
                format!("extensions overlay {} exceeds maximum size", path.display()),
                FailedOverlayMeta::default(),
            ));
        }
        let bytes = fs::read(path).map_err(|err| {
            (
                format!("cannot read extensions overlay {}: {err}", path.display()),
                FailedOverlayMeta::default(),
            )
        })?;
        Self::load_from_bytes(&bytes, host_abi)
    }

    pub(crate) fn load_from_bytes(
        bytes: &[u8],
        host_abi: SharedRuntimeAbi,
    ) -> Result<Self, (String, FailedOverlayMeta)> {
        let raw_meta = extract_raw_overlay_metadata(bytes);
        let parsed: ExtensionsOverlayManifest = serde_json::from_slice(bytes)
            .map_err(|err| (format!("invalid extensions overlay JSON: {err}"), raw_meta.clone()))?;
        Self::verify(parsed, bytes, host_abi).map_err(|err| (err, raw_meta))
    }

    fn verify(
        manifest: ExtensionsOverlayManifest,
        raw_bytes: &[u8],
        host_abi: SharedRuntimeAbi,
    ) -> Result<Self, String> {
        if manifest.schema_version != OVERLAY_SCHEMA_VERSION {
            return Err(format!(
                "unsupported extensions overlay schema_version: {}",
                manifest.schema_version
            ));
        }
        if manifest.overlay_id.trim().is_empty() || manifest.overlay_id.chars().any(char::is_control) {
            return Err("extensions overlay requires non-empty overlay_id".into());
        }
        manifest
            .effective_host_capabilities
            .validate()
            .map_err(|err| format!("invalid effective_host_capabilities: {err}"))?;
        for spec in &manifest.shared_runtime_modules {
            if super::provider::is_forbidden_shared_specifier(spec) {
                return Err(format!("forbidden shared runtime module specifier: {spec:?}"));
            }
        }

        let mut seen_ext_ids = BTreeSet::new();
        let mut entries_by_id = BTreeMap::new();
        let mut entries_by_specifier = BTreeSet::new();
        let mut modules_by_name = BTreeMap::new();
        let mut extension_module_paths = BTreeMap::new();
        let mut styles = Vec::new();
        let mut extension_ids = Vec::with_capacity(manifest.extensions.len());

        for ext in &manifest.extensions {
            if !is_valid_extension_id(&ext.id) {
                return Err(format!("invalid extension id: {:?}", ext.id));
            }
            if !seen_ext_ids.insert(ext.id.clone()) {
                return Err(format!("duplicate extension id in overlay: {:?}", ext.id));
            }
            extension_ids.push(ext.id.clone());

            if ext.version.trim().is_empty() {
                return Err(format!("extension {:?} missing version", ext.id));
            }
            if !valid_sha256_digest(&ext.content_digest) {
                return Err(format!(
                    "extension {:?} invalid content_digest {:?}",
                    ext.id, ext.content_digest
                ));
            }
            let ext_abi = SharedRuntimeAbi::parse(&ext.shared_runtime_abi)?;
            if !host_abi.is_compatible_with(ext_abi) {
                return Err(format!(
                    "extension {:?} requires incompatible shared_runtime_abi {ext_abi} (host {host_abi})",
                    ext.id
                ));
            }

            let root_path = Path::new(&ext.bundle_root);
            if !root_path.is_absolute() {
                return Err(format!(
                    "extension {:?} bundle_root must be absolute: {:?}",
                    ext.id, ext.bundle_root
                ));
            }
            let canonical_root = fs::canonicalize(root_path).map_err(|err| {
                format!("extension {:?} bundle_root error: {err}", ext.id)
            })?;
            let root_meta = fs::symlink_metadata(&canonical_root).map_err(|err| {
                format!("extension {:?} bundle_root stat failed: {err}", ext.id)
            })?;
            if !root_meta.file_type().is_dir() {
                return Err(format!("extension {:?} bundle_root is not a directory", ext.id));
            }

            let build_manifest_rel = normalize_relative_path(&ext.build_manifest).ok_or_else(|| {
                format!("extension {:?} invalid build_manifest path {:?}", ext.id, ext.build_manifest)
            })?;
            let build_manifest_bytes =
                bounded_fs::read_contained(&canonical_root, &build_manifest_rel, MAX_BUILD_MANIFEST_BYTES)
                    .map_err(|err| {
                        format!(
                            "extension {:?} build_manifest {build_manifest_rel:?} failed: {err:?}",
                            ext.id
                        )
                    })?;
            let build_manifest: ExtensionBuildManifest =
                serde_json::from_slice(&build_manifest_bytes).map_err(|err| {
                    format!("extension {:?} invalid build_manifest JSON: {err}", ext.id)
                })?;
            if build_manifest.schema_version != 1 {
                return Err(format!(
                    "extension {:?} unsupported build_manifest schema_version {}",
                    ext.id, build_manifest.schema_version
                ));
            }
            if build_manifest.shared_runtime_abi != ext.shared_runtime_abi {
                return Err(format!(
                    "extension {:?} build_manifest ABI {:?} does not match overlay {:?}",
                    ext.id, build_manifest.shared_runtime_abi, ext.shared_runtime_abi
                ));
            }

            let mut verified_rel_digests = BTreeMap::new();
            let mut ext_mjs_paths = BTreeSet::new();
            let mut ext_sources: BTreeMap<String, (Arc<str>, String, Vec<String>)> = BTreeMap::new();

            for (raw_rel, digest) in &build_manifest.files {
                let rel = normalize_relative_path(raw_rel).ok_or_else(|| {
                    format!("extension {:?} build_manifest invalid path {raw_rel:?}", ext.id)
                })?;
                if rel.to_ascii_lowercase().ends_with(".qjsb") {
                    return Err(format!(
                        "extension {:?} forbids .qjsb artifact in build_manifest: {rel:?}",
                        ext.id
                    ));
                }
                if !valid_sha256_digest(digest) {
                    return Err(format!(
                        "extension {:?} invalid digest for {rel:?}: {digest:?}",
                        ext.id
                    ));
                }
                if verified_rel_digests.insert(rel.clone(), digest.clone()).is_some() {
                    return Err(format!(
                        "extension {:?} duplicate normalized build_manifest path: {rel:?}",
                        ext.id
                    ));
                }
                if rel.ends_with(".mjs") {
                    let bytes = read_verified_file(
                        &canonical_root,
                        &rel,
                        digest,
                        MAX_EXTENSION_MODULE_BYTES,
                    )?;
                    let source = String::from_utf8(bytes).map_err(|_| {
                        format!("extension {:?} module {rel:?} is not valid UTF-8", ext.id)
                    })?;
                    let imports = scan_static_esm_imports(&source);
                    ext_mjs_paths.insert(rel.clone());
                    ext_sources.insert(rel, (Arc::from(source), digest.clone(), imports));
                }
            }

            for style_rel in &ext.styles {
                let rel = normalize_relative_path(style_rel).ok_or_else(|| {
                    format!("extension {:?} invalid style path {style_rel:?}", ext.id)
                })?;
                if !rel.ends_with(".css") {
                    return Err(format!("extension {:?} style {rel:?} must end with .css", ext.id));
                }
                let expected = verified_rel_digests.get(&rel).ok_or_else(|| {
                    format!("extension {:?} style {rel:?} is not registered in build_manifest", ext.id)
                })?;
                let bytes = read_verified_file(&canonical_root, &rel, expected, MAX_EXTENSION_STYLE_BYTES)?;
                let css = String::from_utf8(bytes).map_err(|_| {
                    format!("extension {:?} stylesheet {rel:?} is not valid UTF-8", ext.id)
                })?;
                validate_extension_stylesheet(&css)?;
                styles.push(Stylesheet {
                    path: Arc::from(format!("ext:{}:{rel}", ext.id)),
                    css: Arc::from(css),
                });
            }

            for entry in &ext.entries {
                if entry.entry_id.trim().is_empty() {
                    return Err(format!("extension {:?} has empty entry_id", ext.id));
                }
                let rel = normalize_relative_path(&entry.bundle).ok_or_else(|| {
                    format!("extension {:?} entry {:?} invalid bundle path", ext.id, entry.entry_id)
                })?;
                if !rel.ends_with(".mjs") || !ext_mjs_paths.contains(&rel) {
                    return Err(format!(
                        "extension {:?} entry {:?} bundle {rel:?} is not a verified .mjs module",
                        ext.id, entry.entry_id
                    ));
                }
                let canonical_specifier = format!("zgui-ext://{}/{rel}", ext.id);
                if entries_by_id.contains_key(&entry.entry_id) {
                    return Err(format!("duplicate extension entry_id in overlay: {:?}", entry.entry_id));
                }
                entries_by_specifier.insert(canonical_specifier.clone());
                entries_by_id.insert(
                    entry.entry_id.clone(),
                    VerifiedExtensionEntry {
                        entry_id: entry.entry_id.clone(),
                        extension_id: ext.id.clone(),
                        version: ext.version.clone(),
                        kind: entry.kind.clone(),
                        bundle: rel,
                        canonical_specifier,
                    },
                );
            }

            // Build deterministic transitive import-graph digest per module for 4-tuple cache keying.
            for (rel, (source, digest, _)) in &ext_sources {
                let canonical_name = format!("zgui-ext://{}/{rel}", ext.id);
                let graph_digest = compute_module_graph_digest(
                    &ext.id,
                    rel,
                    &ext.shared_runtime_abi,
                    &ext_sources,
                );
                modules_by_name.insert(
                    canonical_name,
                    VerifiedExtensionModule {
                        extension_id: ext.id.clone(),
                        rel_path: rel.clone(),
                        source: Arc::clone(source),
                        sha256: digest.clone(),
                        resolved_import_graph_digest: graph_digest,
                    },
                );
            }
            extension_module_paths.insert(ext.id.clone(), ext_mjs_paths);
        }

        Ok(Self {
            overlay_id: manifest.overlay_id,
            raw_sha256: Sha256::digest(raw_bytes).into(),
            rollback_allowed_extensions: manifest.rollback_allowed_extensions,
            effective_host_capabilities: manifest.effective_host_capabilities,
            shared_runtime_modules: manifest.shared_runtime_modules,
            extension_ids,
            styles: styles.into(),
            entries_by_id,
            entries_by_specifier,
            modules_by_name,
            extension_module_paths,
            client_metadata: manifest.client_metadata,
        })
    }
}

fn compute_module_graph_digest(
    ext_id: &str,
    root_rel: &str,
    shared_abi: &str,
    ext_sources: &BTreeMap<String, (Arc<str>, String, Vec<String>)>,
) -> String {
    let mut visited = BTreeSet::new();
    let mut stack = vec![root_rel.to_owned()];
    let mut edges = BTreeSet::new();
    while let Some(rel) = stack.pop() {
        if !visited.insert(rel.clone()) {
            continue;
        }
        let Some((_, _, imports)) = ext_sources.get(&rel) else {
            continue;
        };
        for spec in imports {
            if let Some(target_rel) = resolve_relative_in_extension(&rel, spec) {
                if let Some((_, target_digest, _)) = ext_sources.get(&target_rel) {
                    edges.insert(format!("{rel}->{spec}=zgui-ext://{ext_id}/{target_rel}@{target_digest}"));
                    stack.push(target_rel);
                } else {
                    edges.insert(format!("{rel}->{spec}=MISSING"));
                }
            } else {
                edges.insert(format!("{rel}->{spec}=shared@{shared_abi}"));
            }
        }
    }
    let mut hasher = Sha256::new();
    for edge in edges {
        hasher.update(edge.as_bytes());
        hasher.update([0]);
    }
    format!("sha256:{}", hex::encode_32(&hasher.finalize().into()))
}

mod hex {
    use std::fmt::Write as _;
    pub(super) fn encode_32(bytes: &[u8; 32]) -> String {
        let mut s = String::with_capacity(64);
        for b in bytes {
            let _ = write!(s, "{b:02x}");
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_runtime_abi_checks_upward_minor_compatibility() {
        let host_1_1 = SharedRuntimeAbi::parse("1.1").unwrap();
        assert!(host_1_1.is_compatible_with(SharedRuntimeAbi::parse("1.0").unwrap()));
        assert!(host_1_1.is_compatible_with(SharedRuntimeAbi::parse("1.1").unwrap()));
        assert!(!host_1_1.is_compatible_with(SharedRuntimeAbi::parse("1.2").unwrap()));
        assert!(!host_1_1.is_compatible_with(SharedRuntimeAbi::parse("2.0").unwrap()));
        assert!(SharedRuntimeAbi::parse("1").is_err());
        assert!(SharedRuntimeAbi::parse("1.0.0").is_err());
    }

    #[test]
    fn extension_stylesheet_rejects_root_and_unscoped_element_selectors() {
        assert!(validate_extension_stylesheet(".extCard { padding: 8px; }").is_ok());
        assert!(validate_extension_stylesheet("[data-widget] > text { color: #ffffff; }").is_ok());
        assert!(validate_extension_stylesheet("@keyframes pulse { from { opacity: 0; } to { opacity: 1; } }").is_ok());
        assert!(validate_extension_stylesheet(":root { background: #000000; }").is_err());
        assert!(validate_extension_stylesheet("text { color: #ff0000; }").is_err());
    }

    #[test]
    fn static_esm_import_scanner_extracts_static_and_skips_dynamic_and_strings() {
        let src = r#"
// import { fake } from "node:fs";
import { createSignal } from "solid-js";
import './side.mjs';
const s = 'import { a } from "fake"';
export { helper } from "./chunk.mjs";
load("dyn");
"#;
        let imports = scan_static_esm_imports(src);
        assert_eq!(imports, vec!["solid-js", "./side.mjs", "./chunk.mjs"]);
    }
}
