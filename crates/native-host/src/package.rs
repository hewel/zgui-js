use std::{collections::BTreeMap, error::Error, fs, path::{Component, Path, PathBuf}};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;
const MAX_RESOURCE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u32,
    pub id: String,
    pub host_api_version: u32,
    pub entry: String,
    pub source_map: String,
    pub styles: Vec<String>,
    pub assets: Vec<String>,
    pub build_id: String,
}

pub struct Package {
    pub directory: PathBuf,
    pub manifest: Manifest,
    pub source: String,
    pub css: String,
    pub source_map_digest: String,
    source_map: sourcemap::DecodedMap,
    generated_entry: String,
}

impl Package {
    /// Resolve the publication symlink once. Every read then belongs to that
    /// immutable generation, even if the builder replaces dist concurrently.
    pub fn load(path: &Path) -> Result<Self> {
        let directory = path.canonicalize()?;
        if !directory.is_dir() { return Err("bundle root is not a directory".into()); }
        let manifest: Manifest = serde_json::from_slice(&resource(&directory, "manifest.json")?)?;
        if manifest.schema_version != 1 || manifest.host_api_version != 1 {
            return Err("unsupported package schema or host API version".into());
        }
        if manifest.id.is_empty() { return Err("package ID must not be empty".into()); }
        let mut files = BTreeMap::new();
        for name in std::iter::once(&manifest.entry).chain(std::iter::once(&manifest.source_map)).chain(&manifest.styles).chain(&manifest.assets) {
            if name == "manifest.json" || name == "build-info.json" || files.contains_key(name) {
                return Err(format!("duplicate or reserved resource: {name}").into());
            }
            files.insert(name.clone(), resource(&directory, name)?);
        }
        // Integrity contract v1: sorted declared entry/map/styles/assets only;
        // manifest and build-info are intentionally excluded to avoid cycles.
        let mut digest = Sha256::new();
        for (name, bytes) in &files {
            digest.update(name.as_bytes()); digest.update([0]);
            digest.update(bytes); digest.update([0]);
        }
        let actual = format!("sha256:{:x}", digest.finalize());
        if actual != manifest.build_id { return Err(format!("package integrity mismatch: expected {}, got {actual}", manifest.build_id).into()); }
        let source = String::from_utf8(files.remove(&manifest.entry).ok_or("missing entry")?)?;
        let map_bytes = files.get(&manifest.source_map).ok_or("missing source map")?;
        let source_map_digest = format!("sha256:{:x}", Sha256::digest(map_bytes));
        // The decoder defaults missing fields, accepting even `{}`; require a
        // recognized map document before decoding and validating its mappings.
        if !sourcemap::is_sourcemap_slice(map_bytes) {
            return Err(format!("invalid source map {}: unrecognized source map document", manifest.source_map).into());
        }
        let source_map = sourcemap::DecodedMap::from_reader(map_bytes.as_slice())
            .map_err(|error| format!("invalid source map {}: {error}", manifest.source_map))?;
        let mut css = String::new();
        for name in &manifest.styles {
            css.push_str(std::str::from_utf8(files.get(name).ok_or("missing stylesheet")?)?);
            css.push('\n');
        }
        let generated_entry = directory.join(&manifest.entry).to_string_lossy().into_owned();
        Ok(Self { directory, manifest, source, css, source_map, generated_entry, source_map_digest })
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
                let entry = [self.generated_entry.as_str(), self.manifest.entry.as_str()].into_iter().find(|&entry| {
                    file.strip_suffix(entry).is_some_and(|prefix| prefix.ends_with(" ("))
                })?;
                let start = "    at ".len() + file.len() - entry.len();
                let length = entry.len() + line.len() + column.len() + 2;
                let line = line.parse::<u32>().ok()?.checked_sub(1)?;
                let column = column.parse::<u32>().ok()?.checked_sub(1)?;
                let token = self.source_map.lookup_token(line, column)?;
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

fn resource(root: &Path, name: &str) -> Result<Vec<u8>> {
    let path = Path::new(name);
    if name.is_empty() || name.contains('\\') || name.split('/').any(|part| part.is_empty() || part == "." || part == "..") || path.components().any(|part| !matches!(part, Component::Normal(_))) {
        return Err(format!("resource must be a canonical package-relative path: {name}").into());
    }
    let mut resolved = root.to_path_buf();
    for part in path.components() {
        resolved.push(part);
        if fs::symlink_metadata(&resolved)?.file_type().is_symlink() { return Err(format!("symlink resource rejected: {name}").into()); }
    }
    let metadata = fs::metadata(&resolved)?;
    if !metadata.is_file() || metadata.len() > MAX_RESOURCE_BYTES { return Err(format!("invalid or oversized resource: {name}").into()); }
    Ok(fs::read(resolved)?)
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
            let mut digest = Sha256::new();
            digest.update(b"app.mjs\0"); digest.update(source); digest.update([0]);
            digest.update(b"app.mjs.map\0"); digest.update(map); digest.update([0]);
            let manifest = serde_json::json!({"schemaVersion":1,"id":"test","hostApiVersion":1,"entry":"app.mjs","sourceMap":"app.mjs.map","styles":[],"assets":[],"buildId":format!("sha256:{:x}",digest.finalize())});
            fs::write(self.0.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
        }
    }
    impl Drop for Fixture { fn drop(&mut self) { fs::remove_dir_all(&self.0).unwrap(); } }
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
        assert_eq!(Package::load(&fixture.0).unwrap().source.as_bytes(), source);
        fs::write(fixture.0.join("app.mjs.map"), b"modified").unwrap();
        assert!(Package::load(&fixture.0).err().unwrap().to_string().contains("integrity mismatch"));
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
        let mut engine = js_runtime::JsRuntime::new(Default::default()).unwrap();
        let foreign = engine.evaluate_script(
            "function foreign() { return new Error('foreign').stack; }\nforeign();",
        ).unwrap();
        assert!(foreign.contains("    at foreign (eval_script:1:"), "{foreign}");
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
            assert_eq!(mapped.message, original.message);
            assert_eq!(mapped.kind, original.kind);
            assert_eq!(mapped.phase, original.phase);
            assert_eq!(mapped.generated_stack, original.stack);
            assert_eq!(package.remap_diagnostic(mapped.clone()), mapped);
            assert_eq!(mapped.stack.unwrap(), format!(
                "    at fail (src/Counter.tsx:42:8)\n    at runSmoke (src/Counter.tsx:52:4)\n{foreign}{unrelated}",
            ));
        }
        let plain = js_runtime::Diagnostic {
            kind: js_runtime::DiagnosticKind::Engine,
            phase: "test",
            message: "leave app.mjs:2:13 in the message".into(),
            stack: Some(foreign),
            generated_stack: None,
        };
        assert_eq!(package.remap_diagnostic(plain.clone()), plain);
        let no_stack = js_runtime::Diagnostic { stack: None, ..plain };
        assert_eq!(package.remap_diagnostic(no_stack.clone()), no_stack);
    }
}
