use std::{collections::BTreeMap, error::Error, fs, path::{Path, PathBuf}};
use crate::bounded_fs::{self, BoundedReadError};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;
const MAX_RESOURCE_BYTES: u64 = 64 * 1024 * 1024;

crate::schema::object_struct! {
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub id: String,
    pub host_api_version: u32,
    pub entry: String,
    pub source_map: String,
    pub styles: Vec<String>,
    pub assets: Vec<String>,
    pub fonts: Vec<Font>,
    pub style_system: StyleSystem,
    #[serde(deserialize_with = "unique_digests")]
    pub artifact_digests: BTreeMap<String, String>,
    pub build_id: String,
}
}

crate::schema::object_struct! {
#[serde(deny_unknown_fields)]
pub struct Font {
    pub family: String,
    pub path: String,
}
}

crate::schema::object_struct! {
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StyleSystem {
    pub name: String,
    pub profile: String,
    pub config_digest: String,
    pub native_profile_digest: String,
}
}

#[derive(Clone)]
pub struct Stylesheet {
    pub path: String,
    pub css: String,
}

pub struct Package {
    pub directory: PathBuf,
    pub manifest: Manifest,
    pub source: String,
    pub styles: Vec<Stylesheet>,
    pub style_origins: serde_json::Value,
    pub build_info: serde_json::Value,
    pub source_map_digest: String,
    source_map: sourcemap::DecodedMap,
    generated_entry: String,
    status_path: Option<PathBuf>,
    assets: BTreeMap<String, std::sync::Arc<[u8]>>,
}

impl Package {
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
        if manifest.schema_version != 2 || manifest.host_api_version != 1 {
            return Err("unsupported package schema or host API version".into());
        }
        if manifest.id.trim().is_empty() { return Err("package ID must not be empty".into()); }
        if manifest.style_system.name != "stylex" || manifest.style_system.profile != "zgui-stylex-v1"
            || !valid_digest(&manifest.style_system.config_digest) {
            return Err("unsupported style system/profile or invalid config digest".into());
        }
        if manifest.style_system.native_profile_digest != native_profile_digest() {
            return Err(format!("native style profile digest mismatch: expected {}, got {}", native_profile_digest(), manifest.style_system.native_profile_digest).into());
        }
        if !valid_digest(&manifest.build_id) { return Err("invalid package buildId".into()); }
        let mut files = BTreeMap::new();
        for name in std::iter::once(&manifest.entry).chain(std::iter::once(&manifest.source_map))
            .chain(&manifest.styles).chain(&manifest.assets)
            .map(String::as_str).chain(["style-origins.json", "build-info.json"]) {
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
        let json_resource = |name: &str| -> Result<serde_json::Value> {
            let value: serde_json::Value = serde_json::from_slice(&files[name])?;
            if !value.is_object() { return Err(format!("{name} must be a JSON object").into()); }
            Ok(value)
        };
        let style_origins = json_resource("style-origins.json")?;
        let build_info = json_resource("build-info.json")?;
        let style_identity = serde_json::to_value(&manifest.style_system)?;
        for (name, resource) in [("style-origins.json", &style_origins), ("build-info.json", &build_info)] {
            if resource.get("styleSystem") != Some(&style_identity) {
                return Err(format!("SX14: {name} styleSystem identity is missing or differs from manifest").into());
            }
        }
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
        let styles = manifest.styles.iter().map(|name| {
            Ok(Stylesheet { path: name.clone(), css: String::from_utf8(files.remove(name).ok_or("missing stylesheet")?)? })
        }).collect::<Result<Vec<_>>>()?;
        let generated_entry = directory.join(&manifest.entry).to_string_lossy().into_owned();
        let assets = manifest.assets.iter().map(|name| {
            (name.clone(), std::sync::Arc::from(files.remove(name).expect("verified declared asset")))
        }).collect();
        Ok(Self { directory, manifest, source, styles, style_origins, build_info, source_map, generated_entry, source_map_digest, assets, status_path })
    }

    /// Immutable manifest-declared bytes from the integrity-verified package generation.
    pub fn asset_bytes(&self, name: &str) -> Option<&[u8]> {
        self.assets.get(name).map(AsRef::as_ref)
    }

    pub(crate) fn asset_data(&self, name: &str) -> Option<std::sync::Arc<[u8]>> {
        self.assets.get(name).map(std::sync::Arc::clone)
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

    pub(crate) fn image_assets(&self) -> impl Iterator<Item = (String, std::sync::Arc<[u8]>)> + '_ {
        self.assets.iter().map(|(name, bytes)| (name.clone(), std::sync::Arc::clone(bytes)))
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

fn valid_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| hex.len() == 64
        && hex.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
}

fn digest(bytes: &[u8]) -> String { format!("sha256:{:x}", Sha256::digest(bytes)) }

pub(crate) fn native_profile_digest() -> &'static str {
    static DIGEST: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        let profile: serde_json::Value = serde_json::from_str(include_str!("../../../schemas/stylex-profile.json"))
            .expect("native StyleX profile descriptor is valid JSON");
        digest(&canonical_bytes(&profile).expect("native StyleX profile descriptor serializes"))
    });
    &DIGEST
}

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
            let style_system = serde_json::json!({"name":"stylex","profile":"zgui-stylex-v1",
                "configDigest":digest(b"test"),"nativeProfileDigest":native_profile_digest()});
            let metadata = serde_json::to_vec(&serde_json::json!({"styleSystem":style_system})).unwrap();
            fs::write(self.0.join("style-origins.json"), &metadata).unwrap();
            fs::write(self.0.join("build-info.json"), &metadata).unwrap();
            let manifest = serde_json::json!({"schemaVersion":2,"id":"test","hostApiVersion":1,"entry":"app.mjs","sourceMap":"app.mjs.map","styles":[],"assets":[],"fonts":[],
                "styleSystem":style_system,"artifactDigests":{}});
            self.save(manifest);
        }
        fn save(&self, mut manifest: serde_json::Value) {
            let names: Vec<_> = ["entry", "sourceMap"].into_iter().map(|key| manifest[key].as_str().unwrap().to_owned())
                .chain(["styles", "assets"].into_iter().flat_map(|key| manifest[key].as_array().unwrap().iter().map(|value| value.as_str().unwrap().to_owned())))
                .chain(["style-origins.json".into(), "build-info.json".into()]).collect();
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
    fn rejects_mixed_metadata_identities_even_when_every_artifact_and_build_hash_is_valid() {
        let fixture = Fixture::new();
        fixture.publish(b"export {};", br#"{"version":3,"sources":[],"names":[],"mappings":""}"#);
        for resource in ["style-origins.json", "build-info.json"] {
            let original = fs::read(fixture.0.join(resource)).unwrap();
            for identity in [None, Some(serde_json::json!({"name":"stylex","profile":"other","configDigest":digest(b"other")}))] {
                let metadata = identity.map(|identity|serde_json::json!({"styleSystem":identity})).unwrap_or_else(||serde_json::json!({}));
                fs::write(fixture.0.join(resource), serde_json::to_vec(&metadata).unwrap()).unwrap();
                fixture.save(manifest(&fixture));
                assert!(Package::load(&fixture.0).err().unwrap().to_string().contains("SX14"), "accepted mixed {resource}");
            }
            fs::write(fixture.0.join(resource), &original).unwrap();
            fixture.save(manifest(&fixture));
        }
        assert!(Package::load(&fixture.0).is_ok());
    }

    #[test]
    fn acceptance_status_is_shared_by_symlink_and_resolved_generation_loads() {
        let fixture = Fixture::new();
        fixture.publish(b"export {};", br#"{"version":3,"sources":[],"names":[],"mappings":""}"#);
        let generation = fixture.0.join(".zgui-generations/candidate");
        fs::create_dir_all(&generation).unwrap();
        for name in ["app.mjs", "app.mjs.map", "style-origins.json", "build-info.json", "manifest.json"] {
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
            ("style-origins.json", serde_json::Value::Null),
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
        assert_eq!(package.styles.iter().map(|sheet|sheet.path.as_str()).collect::<Vec<_>>(), ["base.css","app.css"]);
        assert_eq!(package.styles[0].css, ".base {color:red}");
        value["styles"] = serde_json::json!(["app.css", "base.css"]);
        fixture.save(value.clone());
        let reversed = Package::load(&fixture.0).unwrap();
        assert_ne!(package.manifest.build_id, reversed.manifest.build_id);
        assert_eq!(reversed.styles[0].path, "app.css");
        value["fonts"][0]["path"] = serde_json::json!("app.css");
        fixture.save(value);
        assert!(Package::load(&fixture.0).err().unwrap().to_string().contains("undeclared font"));
    }

    #[test]
    fn rejects_closed_manifest_violations_and_confined_paths() {
        let fixture = Fixture::new();
        fixture.publish(b"export {};", br#"{"version":3,"sources":[],"names":[],"mappings":""}"#);
        let original = manifest(&fixture);
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
            { let mut value = original.clone(); value["styleSystem"]["nativeProfileDigest"] = serde_json::json!(digest(b"wrong-native-profile")); value },
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
        let data: zgui::text::FontData = std::sync::Arc::new(include_bytes!("../../../preview/fonts/NotoSans-Regular.ttf").as_slice());
        let first = fonts.register_scoped(data.clone(), "Application Sans").unwrap();
        let successor = fonts.register_scoped(data.clone(), "Application Sans").unwrap();
        drop(first);
        assert!(fonts.contains_family("Application Sans"), "retirement withdrew the successor");
        assert!(!fonts.contains_family("Application Snas"), "misspelled alias must not use fallback");
        assert!(fonts.register_scoped(std::sync::Arc::new(b"not a font"), "Application Sans").is_err());
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
        assert_eq!(Package::load(&fixture.0).unwrap().source.as_bytes(), source);
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
