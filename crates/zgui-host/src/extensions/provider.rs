//! Composite verified module provider combining:
//! 1) Shared runtime bare-specifier allowlist (6 core slots + pluggable host table; shell-host excluded),
//! 2) Base shell sealed package graph (`Package`),
//! 3) Generic extension virtual namespace `zgui-ext://<ext_id>/<rel_path>` with optional `.zsqb` bytecode cache.
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

use js_runtime::{VerifiedModulePayload, VerifiedModuleProvider};

use super::manifest::{VerifiedOverlay, resolve_relative_in_extension};
use crate::bytecode_cache::{BytecodeCache, compute_cache_key};

pub(crate) const CORE_SHARED_RUNTIME_SPECIFIERS: [&str; 6] = [
    "solid-js",
    "@hewel/zgui",
    "@hewel/zgui/renderer",
    "@hewel/zgui/primitives",
    "@hewel/zgui/jsx-runtime",
    "@hewel/zgui-services",
];

pub(crate) const SHELL_HOST_SUBPATH: &str = "@hewel/zgui/shell-host";

/// Rejects base-shell privileged subpaths, Node/Bun builtins, DOM adapters, and relative/URL paths.
#[must_use]
pub(crate) fn is_forbidden_shared_specifier(specifier: &str) -> bool {
    if specifier.is_empty()
        || specifier.chars().any(char::is_whitespace)
        || specifier.starts_with('.')
        || specifier.starts_with('/')
        || specifier.contains("://")
        || specifier.contains('\\')
    {
        return true;
    }
    specifier == SHELL_HOST_SUBPATH
        || specifier.starts_with("@hewel/zgui/shell-host/")
        || specifier.starts_with("node:")
        || specifier == "bun"
        || specifier.starts_with("bun:")
        || specifier.ends_with("/bun")
        || specifier == "solid-js/web"
        || specifier.starts_with("solid-js/web/")
        || specifier == "solid-js/server"
        || specifier.starts_with("solid-js/server/")
        || specifier == "react-dom"
        || specifier.starts_with("react-dom/")
}

#[derive(Debug)]
pub(crate) struct SharedRuntimeRegistry {
    allowed_specifiers: BTreeSet<String>,
    registered_exports: BTreeMap<String, Vec<String>>,
}

impl SharedRuntimeRegistry {
    pub(crate) fn new(extra_allowed: &[String]) -> Self {
        let mut allowed_specifiers: BTreeSet<String> = CORE_SHARED_RUNTIME_SPECIFIERS
            .iter()
            .map(|&s| s.to_owned())
            .collect();
        for spec in extra_allowed {
            if !is_forbidden_shared_specifier(spec) {
                allowed_specifiers.insert(spec.clone());
            }
        }
        Self {
            allowed_specifiers,
            registered_exports: BTreeMap::new(),
        }
    }

    pub(crate) fn is_allowed(&self, specifier: &str) -> bool {
        self.allowed_specifiers.contains(specifier) && !is_forbidden_shared_specifier(specifier)
    }

    pub(crate) fn register_module_exports(
        &mut self,
        specifier: &str,
        export_names: Vec<String>,
    ) -> Result<(), String> {
        if is_forbidden_shared_specifier(specifier) {
            return Err(format!(
                "refusing to expose forbidden module specifier {specifier:?} to extensions"
            ));
        }
        self.allowed_specifiers.insert(specifier.to_owned());
        self.registered_exports.insert(specifier.to_owned(), export_names);
        Ok(())
    }

    fn build_esm_shim(&self, specifier: &str) -> Result<String, String> {
        let keys = self.registered_exports.get(specifier).ok_or_else(|| {
            format!("shared runtime module {specifier:?} was not registered by base shell")
        })?;
        let spec_json = serde_json::to_string(specifier)
            .map_err(|e| format!("failed to encode specifier: {e}"))?;
        let mut out = format!("const __m = globalThis.__zguiSharedRuntimeGet({spec_json});\n");
        for key in keys {
            if key == "default" {
                out.push_str("export default __m.default;\n");
            } else if is_valid_js_identifier(key) && key != "__m" {
                use std::fmt::Write as _;
                let _ = writeln!(out, "export const {key} = __m[{:?}];", key);
            }
        }
        Ok(out)
    }
}

fn is_valid_js_identifier(ident: &str) -> bool {
    let mut chars = ident.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_' || first == '$') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        && !matches!(
            ident,
            "break"
                | "case"
                | "catch"
                | "class"
                | "const"
                | "continue"
                | "debugger"
                | "default"
                | "delete"
                | "do"
                | "else"
                | "enum"
                | "export"
                | "extends"
                | "false"
                | "finally"
                | "for"
                | "function"
                | "if"
                | "import"
                | "in"
                | "instanceof"
                | "new"
                | "null"
                | "return"
                | "super"
                | "switch"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "typeof"
                | "var"
                | "void"
                | "while"
                | "with"
                | "let"
                | "static"
                | "yield"
                | "await"
        )
}

fn split_verified_ext_uri<'a>(overlay: &'a VerifiedOverlay, canonical: &str) -> Option<(&'a str, String)> {
    let module = overlay.modules_by_name.get(canonical)?;
    Some((module.extension_id.as_str(), module.rel_path.clone()))
}

#[derive(Clone)]
pub(crate) struct CompositeVerifiedModuleProvider {
    base_provider: Rc<dyn VerifiedModuleProvider>,
    overlay: Option<Rc<VerifiedOverlay>>,
    shared_runtime: Rc<RefCell<SharedRuntimeRegistry>>,
    bytecode_cache: Option<Rc<BytecodeCache>>,
    approved_load_gate: Rc<Cell<bool>>,
}

impl CompositeVerifiedModuleProvider {
    pub(crate) fn new(
        base_provider: Rc<dyn VerifiedModuleProvider>,
        overlay: Option<Rc<VerifiedOverlay>>,
        shared_runtime: Rc<RefCell<SharedRuntimeRegistry>>,
        bytecode_cache: Option<Rc<BytecodeCache>>,
        approved_load_gate: Rc<Cell<bool>>,
    ) -> Self {
        Self {
            base_provider,
            overlay,
            shared_runtime,
            bytecode_cache,
            approved_load_gate,
        }
    }

    fn source_only_view(&self) -> Rc<dyn VerifiedModuleProvider> {
        Rc::new(Self {
            base_provider: Rc::clone(&self.base_provider),
            overlay: self.overlay.clone(),
            shared_runtime: Rc::clone(&self.shared_runtime),
            bytecode_cache: None,
            approved_load_gate: Rc::clone(&self.approved_load_gate),
        })
    }
}

impl VerifiedModuleProvider for CompositeVerifiedModuleProvider {
    fn resolve(&self, referrer: &str, specifier: &str) -> Result<String, String> {
        if referrer.starts_with("zgui-ext://") {
            let Some(overlay) = &self.overlay else {
                return Err("no active extension overlay".into());
            };
            let Some((ext_id, base_rel)) = split_verified_ext_uri(overlay, referrer) else {
                return Err(format!("unrecognized extension module referrer: {referrer}"));
            };
            if specifier.to_ascii_lowercase().ends_with(".qjsb") {
                return Err(format!(
                    "extension {ext_id:?} must not import .qjsb bytecode: {specifier:?}"
                ));
            }
            if specifier.starts_with("./") || specifier.starts_with("../") {
                let target_rel = resolve_relative_in_extension(&base_rel, specifier).ok_or_else(|| {
                    format!("extension {ext_id:?} relative import escapes bundle root: {specifier:?}")
                })?;
                let allowed_paths = overlay
                    .extension_module_paths
                    .get(ext_id)
                    .ok_or_else(|| format!("extension {ext_id:?} not found in overlay"))?;
                if !allowed_paths.contains(&target_rel) {
                    return Err(format!(
                        "extension {ext_id:?} relative import {specifier:?} is not declared in build_manifest"
                    ));
                }
                return Ok(format!("zgui-ext://{ext_id}/{target_rel}"));
            }
            if self.shared_runtime.borrow().is_allowed(specifier) {
                return Ok(specifier.to_owned());
            }
            return Err(format!(
                "extension {ext_id:?} is forbidden from importing {specifier:?}"
            ));
        }

        if specifier.starts_with("zgui-ext://") {
            if !self.approved_load_gate.get() {
                return Err(
                    "direct import of zgui-ext:// is forbidden; use loadApprovedPluginEntry".into(),
                );
            }
            let Some(overlay) = &self.overlay else {
                return Err("no extension overlay active".into());
            };
            if overlay.entries_by_specifier.contains(specifier) {
                return Ok(specifier.to_owned());
            }
            return Err(format!("unapproved extension entry specifier: {specifier}"));
        }

        self.base_provider.resolve(referrer, specifier)
    }

    fn load(&self, canonical_name: &str) -> Result<VerifiedModulePayload, String> {
        if canonical_name.starts_with("zgui-ext://") {
            let Some(overlay) = &self.overlay else {
                return Err("no active extension overlay".into());
            };
            let module = overlay.modules_by_name.get(canonical_name).ok_or_else(|| {
                format!("extension module {canonical_name:?} not verified in active overlay")
            })?;
            if let Some(cache) = &self.bytecode_cache {
                let abi = js_runtime::quickjs_abi_tag();
                let key = compute_cache_key(
                    &abi,
                    canonical_name,
                    &module.sha256,
                    &module.resolved_import_graph_digest,
                );
                if let Some(cached_bytecode) = cache.get(&abi, &key) {
                    return Ok(VerifiedModulePayload::Bytecode(cached_bytecode));
                }
                if let Ok(compiled) = js_runtime::compile_verified_module_bytecode(
                    canonical_name,
                    self.source_only_view(),
                ) {
                    cache.put(&abi, &key, &compiled);
                    return Ok(VerifiedModulePayload::Bytecode(compiled));
                }
            }
            return Ok(VerifiedModulePayload::Source(module.source.to_string()));
        }

        if self.shared_runtime.borrow().is_allowed(canonical_name) {
            let shim = self.shared_runtime.borrow().build_esm_shim(canonical_name)?;
            return Ok(VerifiedModulePayload::Source(shim));
        }

        self.base_provider.load(canonical_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode_cache::DEFAULT_MAX_CACHE_BYTES;
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(1);

    struct DummyBaseProvider;
    impl VerifiedModuleProvider for DummyBaseProvider {
        fn resolve(&self, _referrer: &str, specifier: &str) -> Result<String, String> {
            Err(format!("base resolve denied: {specifier}"))
        }
        fn load(&self, canonical_name: &str) -> Result<VerifiedModulePayload, String> {
            Err(format!("base load denied: {canonical_name}"))
        }
    }

    fn write_test_extension(root: &std::path::Path, entry_src: &str, chunk_src: &str, css: &str, abi: &str) -> String {
        fs::create_dir_all(root.join("dist/entries")).unwrap();
        fs::create_dir_all(root.join("dist/chunks")).unwrap();
        fs::create_dir_all(root.join("dist/styles")).unwrap();
        fs::write(root.join("dist/entries/panel.mjs"), entry_src).unwrap();
        fs::write(root.join("dist/chunks/helper.mjs"), chunk_src).unwrap();
        fs::write(root.join("dist/styles/panel.css"), css).unwrap();
        let digest = |s: &str| format!("sha256:{:x}", Sha256::digest(s.as_bytes()));
        let build_manifest = serde_json::json!({
            "schema_version": 1,
            "shared_runtime_abi": abi,
            "files": {
                "dist/entries/panel.mjs": digest(entry_src),
                "dist/chunks/helper.mjs": digest(chunk_src),
                "dist/styles/panel.css": digest(css),
            }
        });
        fs::write(root.join("dist/manifest.build.json"), serde_json::to_vec(&build_manifest).unwrap()).unwrap();
        format!("sha256:{:x}", Sha256::digest(entry_src.as_bytes()))
    }

    #[test]
    fn provider_enforces_shared_allowlist_closed_graph_and_bytecode_cache_fallback() {
        let dir = std::env::temp_dir().join(format!(
            "zgui-ext-provider-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let ext_dir = dir.join("ext-a");
        let other_dir = dir.join("ext-b");
        let cache_dir = dir.join("bcache");
        let entry_src = "import { helper } from '../chunks/helper.mjs'; export function answer() { return helper() + 1; }\n";
        let chunk_src = "export function helper() { return 40; }\n";
        let css = ".extPanel { background: #112233; }\n";
        let digest_a = write_test_extension(&ext_dir, entry_src, chunk_src, css, "1.0");
        let other_entry = "export const secret = 1;";
        let digest_b = write_test_extension(&other_dir, other_entry, "export const x = 1;", css, "1.0");

        let overlay_json = serde_json::json!({
            "schema_version": 1,
            "overlay_id": "ovl-test-1",
            "rollback_allowed_extensions": ["acme/weather"],
            "effective_host_capabilities": {},
            "shared_runtime_modules": ["@custom/plugin-sdk"],
            "extensions": [
                {
                    "id": "acme/weather",
                    "version": "1.0.0",
                    "shared_runtime_abi": "1.0",
                    "content_digest": digest_a,
                    "bundle_root": ext_dir,
                    "build_manifest": "dist/manifest.build.json",
                    "styles": ["dist/styles/panel.css"],
                    "entries": [
                        { "entry_id": "acme/weather:panel", "kind": "panel", "bundle": "dist/entries/panel.mjs" }
                    ]
                },
                {
                    "id": "acme/other",
                    "version": "1.0.0",
                    "shared_runtime_abi": "1.0",
                    "content_digest": digest_b,
                    "bundle_root": other_dir,
                    "build_manifest": "dist/manifest.build.json",
                    "styles": [],
                    "entries": []
                }
            ]
        });

        let overlay = Rc::new(
            VerifiedOverlay::load_from_bytes(
                &serde_json::to_vec(&overlay_json).unwrap(),
                super::super::manifest::SharedRuntimeAbi::parse("1.1").unwrap(),
            )
            .expect("upward compatible 1.0 on 1.1 host loads"),
        );
        let cache = Rc::new(BytecodeCache::open(&cache_dir, DEFAULT_MAX_CACHE_BYTES).unwrap());
        let shared = Rc::new(RefCell::new(SharedRuntimeRegistry::new(&overlay.shared_runtime_modules)));
        let gate = Rc::new(Cell::new(false));
        let provider = CompositeVerifiedModuleProvider::new(
            Rc::new(DummyBaseProvider),
            Some(Rc::clone(&overlay)),
            Rc::clone(&shared),
            Some(Rc::clone(&cache)),
            Rc::clone(&gate),
        );

        let entry_ref = "zgui-ext://acme/weather/dist/entries/panel.mjs";
        // Shared runtime allowed + pluggable table works; shell-host & unapproved denied.
        assert_eq!(provider.resolve(entry_ref, "solid-js").unwrap(), "solid-js");
        assert_eq!(provider.resolve(entry_ref, "@custom/plugin-sdk").unwrap(), "@custom/plugin-sdk");
        assert!(provider.resolve(entry_ref, "@hewel/zgui/shell-host").is_err());
        assert!(provider.resolve(entry_ref, "node:fs").is_err());
        assert!(provider.resolve(entry_ref, "solid-js/web").is_err());

        // Relative closed-graph allowed; unverified, escaping, cross-extension, and .qjsb denied.
        assert_eq!(
            provider.resolve(entry_ref, "../chunks/helper.mjs").unwrap(),
            "zgui-ext://acme/weather/dist/chunks/helper.mjs"
        );
        assert!(provider.resolve(entry_ref, "../undeclared.mjs").is_err());
        assert!(provider.resolve(entry_ref, "../../../../etc/passwd").is_err());
        assert!(provider.resolve(entry_ref, "zgui-ext://acme/other/dist/entries/panel.mjs").is_err());
        assert!(provider.resolve(entry_ref, "./foo.qjsb").is_err());

        // Direct zgui-ext:// import from base shell is denied unless gate is open for an approved entry.
        assert!(provider.resolve("app.mjs", entry_ref).is_err());
        gate.set(true);
        assert_eq!(provider.resolve("app.mjs", entry_ref).unwrap(), entry_ref);
        assert!(provider.resolve("app.mjs", "zgui-ext://acme/weather/dist/chunks/helper.mjs").is_err());
        gate.set(false);

        // First load compiles & caches bytecode cleanly; second load hits cache.
        let payload = provider.load(entry_ref).expect("compile and cache bytecode");
        assert!(matches!(payload, VerifiedModulePayload::Bytecode(_)));
        let module = overlay.modules_by_name.get(entry_ref).unwrap();
        let key = compute_cache_key(
            &js_runtime::quickjs_abi_tag(),
            entry_ref,
            &module.sha256,
            &module.resolved_import_graph_digest,
        );
        assert!(cache.entry_path(&key).exists());

        // Corrupt cached .zsqb: get unlinks corrupt file and next load recompiles seamlessly from source.
        fs::write(cache.entry_path(&key), b"corrupt-bytes").unwrap();
        let recovered = provider.load(entry_ref).expect("corrupt cache falls back to source recompile");
        assert!(matches!(recovered, VerifiedModulePayload::Bytecode(_)));

        let _ = fs::remove_dir_all(&dir);
    }
}
