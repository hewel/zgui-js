//! Hermetic preview configuration. Resource limits are mirrored in the JSON schemas.
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zgui::app::Fonts;

use crate::bounded_fs::{self, BoundedReadError};
use super::Error;

type Result<T, E = Error> = std::result::Result<T, E>;
const MAX_JSON_BYTES: u64 = 64 * 1024;
const MAX_FONT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_TOTAL_FONT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_LICENSE_BYTES: u64 = 64 * 1024;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_EPOCH_MS: u64 = 8_640_000_000_000_000;
const MAX_PHYSICAL_EDGE: u32 = 16_384;
pub(crate) const MAX_PHYSICAL_PIXELS: u64 = 16_777_216;

#[derive(Default)]
pub(crate) struct Overrides {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub scale: Option<f64>,
    pub theme: Option<String>,
    pub timeout_ms: Option<u64>,
    pub max_js_stack_bytes: Option<usize>,
}

pub(crate) struct LoadedProfile {
    pub profile: Profile,
    pub fonts: Fonts,
    /// Escaped ordered CSS family value; apply as `:root { font-family: VALUE; }`.
    pub font_family_css: String,
    /// Canonical reusable profile object, including all CLI overrides.
    pub resolved_json: Value,
    /// SHA-256 of a canonical envelope binding the profile and verified font evidence.
    pub digest: String,
    pub physical_width: u32,
    pub physical_height: u32,
    pub font_evidence: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Profile {
    pub schema_version: u32,
    pub id: String,
    pub surface: Surface,
    #[serde(default, skip_serializing_if = "is_false")]
    pub pure_shell: bool,
    pub theme: String,
    #[serde(default)]
    pub reduced_motion: bool,
    pub locale: String,
    pub direction: String,
    pub time_zone: String,
    pub clock: Clock,
    pub random: Random,
    pub fonts: FontConfig,
    pub assets: AssetConfig,
    pub limits: Limits,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Surface {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub refresh_rate_milli_hz: u32,
    pub opaque: bool,
    pub background: String,
}

fn is_false(value: &bool) -> bool { !value }

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "lowercase", deny_unknown_fields)]
pub(crate) enum Clock {
    Wall {},
    Virtual {
        #[serde(rename = "epochMs")]
        epoch_ms: u64,
        #[serde(rename = "startUs")]
        start_us: u64,
    },
}

impl Clock {
    pub fn is_virtual(&self) -> bool { matches!(self, Self::Virtual { .. }) }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Random {
    pub seed: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FontConfig {
    pub mode: String,
    pub manifest: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AssetConfig {
    pub mode: String,
    pub readiness: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Limits {
    pub max_run_wall_ms: u64,
    pub max_step_wall_ms: u64,
    pub max_js_entry_wall_ms: u64,
    pub max_jobs_per_pump: u32,
    pub max_pump_wall_ms: u64,
    pub max_settle_turns: u32,
    pub max_frames_per_step: u32,
    pub max_virtual_duration_ms: u64,
    pub max_output_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_js_stack_bytes: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FontManifest {
    schema_version: u32,
    faces: Vec<FontFace>,
    fallback_order: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FontFace {
    family: String,
    path: String,
    sha256: String,
    license_path: String,
}

pub(crate) fn read_profile(path: &Path, overrides: &Overrides) -> Result<(PathBuf, Profile)> {
    let profile_path = fs::canonicalize(path)
        .map_err(|error| invalid(format!("profile {}: {error}", path.display())))?;
    let bytes = bounded_fs::read_input(&profile_path, MAX_JSON_BYTES)
        .map_err(|error| invalid(format!("resource {}: {error}", profile_path.display())))?;
    let mut profile: Profile = parse_json(&bytes, "profile")?;
    // Validate the declaration as well as the override: an override must not hide an invalid file.
    validate(&profile)?;
    if let Some(width) = overrides.width { profile.surface.width = width; }
    if let Some(height) = overrides.height { profile.surface.height = height; }
    if let Some(scale) = overrides.scale { profile.surface.scale = scale; }
    if let Some(theme) = &overrides.theme { profile.theme.clone_from(theme); }
    if let Some(timeout) = overrides.timeout_ms { profile.limits.max_run_wall_ms = timeout; }
    if let Some(stack_bytes) = overrides.max_js_stack_bytes { profile.limits.max_js_stack_bytes = Some(stack_bytes); }
    validate(&profile)?;
    Ok((profile_path, profile))
}

pub(crate) fn load(path: &Path, overrides: &Overrides) -> Result<LoadedProfile> {
    let (profile_path, profile) = read_profile(path, overrides)?;
    let (physical_width, physical_height) = dimensions(&profile.surface)?;

    // The explicitly declared manifest establishes an additional resource root. Its path may
    // contain `..` (e.g. profiles/../fonts/fonts.json); faces and licenses cannot escape that root.
    let manifest_name = Path::new(&profile.fonts.manifest);
    if !valid_manifest_path(&profile.fonts.manifest) {
        return Err(invalid("fonts.manifest must be a nonempty relative path"));
    }
    let manifest_path = fs::canonicalize(profile_path.parent().expect("canonical file has parent").join(manifest_name))
        .map_err(|error| font_error(format!("font manifest {}: {error}", profile.fonts.manifest)))?;
    let manifest_bytes = bounded_fs::read_input(&manifest_path, MAX_JSON_BYTES)
        .map_err(|error| invalid(format!("resource {}: {error}", manifest_path.display())))?;
    let manifest: FontManifest = parse_json(&manifest_bytes, "font manifest")?;
    validate_manifest(&manifest)?;
    let root = manifest_path.parent().expect("canonical file has parent");
    let fonts = Fonts::shipped_only();
    let mut evidence = Vec::with_capacity(manifest.faces.len());
    let mut total_bytes = 0_u64;
    for face in &manifest.faces {
        let data = read_font_resource(root, &face.path, MAX_FONT_BYTES)?;
        total_bytes += data.len() as u64;
        if total_bytes > MAX_TOTAL_FONT_BYTES {
            return Err(font_error("declared fonts exceed the 64 MiB total font budget"));
        }
        let actual_hash = sha256(&data);
        if actual_hash != face.sha256 {
            return Err(Error::new("E_FONT_HASH", 2, format!("font {}: SHA-256 mismatch (expected {}, got {actual_hash})", face.path, face.sha256)));
        }
        let license = read_font_resource(root, &face.license_path, MAX_LICENSE_BYTES)?;
        if std::str::from_utf8(&license).map_or(true, |text| text.trim().is_empty()) {
            return Err(font_error(format!("license {} must be nonempty UTF-8 text", face.license_path)));
        }
        let byte_length = data.len();
        fonts.register(Arc::new(data), Some(&face.family))
            .map_err(|error| font_error(format!("cannot register font {} as {:?}: {error:?}", face.path, face.family)))?;
        evidence.push(json!({
            "family": face.family,
            "path": face.path,
            "sha256": actual_hash,
            "bytes": byte_length,
            "licensePath": face.license_path,
            "licenseSha256": sha256(&license),
        }));
    }
    let font_family_css = font_css(&manifest.fallback_order);
    let font_evidence = canonical(json!({
        "mode": "shipped-only", "faces": evidence, "fallbackOrder": manifest.fallback_order,
        "fontFamilyCss": font_family_css,
        "manifest": manifest,
        "physicalSurface": {
            "width": physical_width, "height": physical_height,
            "effectiveLogicalWidth": f64::from(physical_width) / profile.surface.scale,
            "effectiveLogicalHeight": f64::from(physical_height) / profile.surface.scale,
        },
        "timingDeterministic": profile.clock.is_virtual(),
    }));
    let resolved_json = canonical(serde_json::to_value(&profile).map_err(|error| invalid(error.to_string()))?);
    let digest_input = canonical(json!({"profile": resolved_json, "fontEvidence": font_evidence}));
    let digest = sha256(&serde_json::to_vec(&digest_input).map_err(|error| invalid(error.to_string()))?);
    Ok(LoadedProfile { profile, fonts, font_family_css, resolved_json, digest, physical_width, physical_height, font_evidence })
}

fn invalid(message: impl Into<String>) -> Error { Error::new("E_PROFILE", 2, message) }
fn font_error(message: impl Into<String>) -> Error { Error::new("E_FONT", 2, message) }

fn parse_json<T: serde::de::DeserializeOwned>(bytes: &[u8], label: &str) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|error| invalid(format!("invalid {label}: {error}")))
}

fn in_range(value: u64, minimum: u64, maximum: u64, name: &str) -> Result<()> {
    if !(minimum..=maximum).contains(&value) {
        return Err(invalid(format!("{name} must be in {minimum}..={maximum}")));
    }
    Ok(())
}

fn validate(profile: &Profile) -> Result<(u32, u32)> {
    if profile.schema_version != 1 { return Err(invalid("unsupported profile schemaVersion; expected 1")); }
    if profile.id.trim().is_empty() || profile.id.chars().count() > 128 { return Err(invalid("id must contain 1..=128 characters of nonblank text")); }
    if profile.surface.name != "main" { return Err(invalid("only surface main is supported")); }
    if !matches!(profile.theme.as_str(), "dark" | "light") { return Err(invalid("theme must be dark or light")); }
    if profile.locale != "en-US" || profile.time_zone != "UTC" || profile.direction != "ltr" {
        return Err(Error::new("E_PROFILE_LOCALE", 2, "only locale en-US, timeZone UTC, and direction ltr are supported"));
    }
    let background = profile.surface.background.as_bytes();
    if background.len() != 9 || background[0] != b'#' || !background[1..].iter().all(u8::is_ascii_hexdigit) {
        return Err(invalid("surface.background must be a #RRGGBBAA hexadecimal color"));
    }
    if profile.surface.opaque && !background[7..].eq_ignore_ascii_case(b"ff") {
        return Err(invalid("surface.background must be an opaque #RRGGBBff hexadecimal color"));
    }
    in_range(u64::from(profile.surface.refresh_rate_milli_hz), 1, 1_000_000, "surface.refreshRateMilliHz")?;
    if profile.fonts.mode != "shipped-only" { return Err(invalid("fonts.mode must be shipped-only")); }
    if !valid_manifest_path(&profile.fonts.manifest) { return Err(invalid("fonts.manifest must be a nonempty relative path of at most 1024 characters")); }
    if profile.assets.mode != "package-only" || profile.assets.readiness != "required-visible" {
        return Err(invalid("only package-only assets with required-visible readiness are supported"));
    }
    let limits = &profile.limits;
    in_range(limits.max_run_wall_ms, 1, 300_000, "limits.maxRunWallMs")?;
    in_range(limits.max_step_wall_ms, 1, 60_000, "limits.maxStepWallMs")?;
    in_range(limits.max_js_entry_wall_ms, 1, 1_000, "limits.maxJsEntryWallMs")?;
    in_range(u64::from(limits.max_jobs_per_pump), 1, 100_000, "limits.maxJobsPerPump")?;
    in_range(limits.max_pump_wall_ms, 1, 1_000, "limits.maxPumpWallMs")?;
    in_range(u64::from(limits.max_settle_turns), 1, 10_000, "limits.maxSettleTurns")?;
    in_range(u64::from(limits.max_frames_per_step), 1, 10_000, "limits.maxFramesPerStep")?;
    in_range(limits.max_virtual_duration_ms, 0, 3_600_000, "limits.maxVirtualDurationMs")?;
    in_range(limits.max_output_bytes, 1, 1_073_741_824, "limits.maxOutputBytes")?;
    if let Some(stack_bytes) = limits.max_js_stack_bytes {
        in_range(stack_bytes as u64, 32_768, 16_777_216, "limits.maxJsStackBytes")?;
    }
    if let Clock::Virtual { epoch_ms, start_us } = profile.clock {
        in_range(epoch_ms, 0, MAX_EPOCH_MS, "clock.epochMs")?;
        in_range(start_us, 0, MAX_SAFE_INTEGER, "clock.startUs")?;
        let end_us = start_us + limits.max_virtual_duration_ms * 1_000;
        if end_us > MAX_SAFE_INTEGER || epoch_ms + end_us.div_ceil(1_000) > MAX_EPOCH_MS {
            return Err(invalid("virtual clock plus maximum duration exceeds the exact-integer/Date range"));
        }
    }
    dimensions(&profile.surface)
}

pub(crate) fn dimensions(surface: &Surface) -> Result<(u32, u32)> {
    in_range(u64::from(surface.width), 1, 16_384, "surface.width")?;
    in_range(u64::from(surface.height), 1, 16_384, "surface.height")?;
    physical_dimensions(f64::from(surface.width), f64::from(surface.height), surface.scale)
}

pub(super) fn physical_dimensions(width: f64, height: f64, scale: f64) -> Result<(u32, u32)> {
    if !scale.is_finite() || scale <= 0.0 || scale > 8.0 {
        return Err(invalid("surface.scale must be finite and in (0, 8]"));
    }
    let physical = |logical: f64| -> Result<u32> {
        let rounded = (logical * scale + 0.5).floor();
        if !(1.0..=f64::from(MAX_PHYSICAL_EDGE)).contains(&rounded) {
            return Err(invalid("rounded physical extent must be in 1..=16384 pixels"));
        }
        Ok(rounded as u32)
    };
    let width = physical(width)?;
    let height = physical(height)?;
    if u64::from(width) * u64::from(height) > MAX_PHYSICAL_PIXELS {
        return Err(invalid("physical surface exceeds 16777216 pixels (64 MiB RGBA8)"));
    }
    Ok((width, height))
}

fn valid_manifest_path(name: &str) -> bool {
    !name.is_empty() && name.chars().count() <= 1024 && !name.contains(['\\', '\0'])
        && !Path::new(name).is_absolute()
        && name.split('/').all(|part| !part.is_empty() && part != ".")
        && Path::new(name).components().all(|part| matches!(part, Component::Normal(_) | Component::ParentDir))
}

fn valid_resource_path(name: &str) -> bool {
    valid_manifest_path(name) && Path::new(name).components().all(|part| matches!(part, Component::Normal(_)))
}

fn validate_manifest(manifest: &FontManifest) -> Result<()> {
    if manifest.schema_version != 1 { return Err(font_error("unsupported font schemaVersion; expected 1")); }
    if manifest.faces.is_empty() || manifest.faces.len() > 16 { return Err(font_error("fonts.faces must contain 1..=16 faces")); }
    let mut families = BTreeSet::new();
    for face in &manifest.faces {
        if face.family.trim().is_empty() || face.family.chars().count() > 128 || face.family.chars().any(char::is_control) {
            return Err(font_error("font family aliases must contain 1..=128 characters of nonblank text without control characters"));
        }
        if !families.insert(&face.family) { return Err(font_error(format!("duplicate font family alias {:?}", face.family))); }
        if !valid_resource_path(&face.path) || !valid_resource_path(&face.license_path) {
            return Err(font_error("font and license paths must be canonical relative paths inside the font manifest root"));
        }
        if face.sha256.len() != 64 || !face.sha256.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
            return Err(font_error(format!("font {} must have a 64-digit lowercase SHA-256", face.path)));
        }
    }
    if manifest.fallback_order.is_empty() || manifest.fallback_order.len() > 16 {
        return Err(font_error("fallbackOrder must contain 1..=16 declared aliases"));
    }
    let mut seen = BTreeSet::new();
    for family in &manifest.fallback_order {
        if !families.contains(family) || !seen.insert(family) {
            return Err(font_error(format!("fallbackOrder alias {family:?} is undeclared or repeated")));
        }
    }
    Ok(())
}

fn read_font_resource(root: &Path, name: &str, maximum: u64) -> Result<Vec<u8>> {
    bounded_fs::read_contained(root, name, maximum).map_err(|error| match error {
        BoundedReadError::InvalidPath => font_error(format!("invalid font resource path {name:?}")),
        BoundedReadError::Symlink => font_error(format!("font resource escapes its declared root: {name}")),
        BoundedReadError::NotRegularFile | BoundedReadError::TooLarge { .. } => {
            invalid(format!("resource {name} must be a regular file of at most {maximum} bytes"))
        }
        BoundedReadError::Io(io) => font_error(format!("font resource {name}: {io}")),
    })
}

fn sha256(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }

pub(super) fn canonical(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries: Vec<_> = object.into_iter().collect();
            entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
            Value::Object(entries.into_iter().map(|(key, value)| (key, canonical(value))).collect())
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonical).collect()),
        value => value,
    }
}

fn font_css(families: &[String]) -> String {
    let mut css = String::with_capacity(families.iter().map(|family| family.len() * 2 + 4).sum());
    for (index, family) in families.iter().enumerate() {
        if index != 0 { css.push_str(", "); }
        css.push('"');
        for character in family.chars() {
            // Every alias is a CSS string token, never an identifier or raw declaration.
            match character {
                '"' | '\\' => { css.push('\\'); css.push(character); }
                character if character.is_control() => {
                    use std::fmt::Write as _;
                    write!(css, "\\{:x} ", u32::from(character)).expect("writing to String");
                }
                character => css.push(character),
            }
        }
        css.push('"');
    }
    css
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn reduced_motion_is_an_explicit_boolean_profile_preference() {
        let mut value = serde_json::to_value(default_profile()).unwrap();
        value["reducedMotion"] = json!(true);
        let reduced: Profile = serde_json::from_value(value.clone()).unwrap();
        assert!(reduced.reduced_motion);
        assert!(validate(&reduced).is_ok());
        value["reducedMotion"] = json!("reduce");
        assert!(serde_json::from_value::<Profile>(value).is_err());
    }

    fn default_profile() -> Profile {
        serde_json::from_str(include_str!("../../../../preview/profiles/default.json")).unwrap()
    }

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            static SEQUENCE: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!("zgui-preview-profile-{}-{}", std::process::id(), SEQUENCE.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TempDir { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }

    #[test]
    fn clock_variants_are_closed_and_require_exact_integer_times() {
        for invalid in [
            r#"{"mode":"wall","epochMs":0}"#,
            r#"{"mode":"wall","startUs":0}"#,
            r#"{"mode":"virtual","epochMs":0}"#,
            r#"{"mode":"virtual","epochMs":0,"startUs":0,"extra":true}"#,
            r#"{"mode":"virtual","epochMs":0.5,"startUs":0}"#,
            r#"{"mode":"virtual","epochMs":0,"startUs":-1}"#,
        ] { assert!(serde_json::from_str::<Clock>(invalid).is_err(), "{invalid}"); }
        let mut profile = default_profile();
        profile.clock = Clock::Virtual { epoch_ms: 0, start_us: MAX_SAFE_INTEGER };
        assert!(validate(&profile).is_err());
        profile.limits.max_virtual_duration_ms = 0;
        assert!(validate(&profile).is_ok());
    }

    #[test]
    fn all_profile_objects_reject_unknown_fields_and_unsupported_locale() {
        let original = serde_json::to_value(default_profile()).unwrap();
        for pointer in ["", "/surface", "/clock", "/random", "/fonts", "/assets", "/limits"] {
            let mut value = original.clone();
            value.pointer_mut(pointer).unwrap().as_object_mut().unwrap().insert("unknown".into(), json!(true));
            assert!(serde_json::from_value::<Profile>(value).is_err(), "{pointer}");
        }
        let mut profile = default_profile();
        profile.locale = "ja-JP".into();
        assert_eq!(validate(&profile).unwrap_err().code, "E_PROFILE_LOCALE");
        profile.locale = "en-US".into();
        profile.time_zone = "Europe/Warsaw".into();
        assert_eq!(validate(&profile).unwrap_err().code, "E_PROFILE_LOCALE");
    }

    #[test]
    fn surface_background_alpha_follows_the_opacity_policy() {
        let mut profile = default_profile();
        for background in ["#00000000", "#11223380"] {
            profile.surface.background = background.into();
            profile.surface.opaque = false;
            validate(&profile).unwrap_or_else(|error| panic!("{background}: {error}"));
            profile.surface.opaque = true;
            assert_eq!(validate(&profile).unwrap_err().code, "E_PROFILE", "{background}");
        }
        profile.surface.background = "#112233fF".into();
        validate(&profile).unwrap();
        profile.surface.opaque = false;
        for background in ["#112233", "#112233x0", "#112233é", "011223380"] {
            profile.surface.background = background.into();
            assert_eq!(validate(&profile).unwrap_err().code, "E_PROFILE", "{background}");
        }
    }

    #[test]
    fn physical_extents_round_half_up_and_enforce_allocation_budget() {
        let mut surface = default_profile().surface;
        for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
            surface.scale = scale;
            assert_eq!(dimensions(&surface).unwrap(), ((800.0 * scale) as u32, (600.0 * scale) as u32));
        }
        surface.width = 801;
        surface.scale = 1.5;
        assert_eq!(dimensions(&surface).unwrap().0, 1202);
        surface.scale = 1.0;
        assert_eq!(dimensions(&surface).unwrap().0 % 2, 1);
        surface.width = 4096;
        surface.height = 4096;
        assert!(dimensions(&surface).is_ok());
        surface.width += 1;
        assert!(dimensions(&surface).is_err());
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY, 0.00001, 8.1] {
            surface.scale = scale;
            assert!(dimensions(&surface).is_err());
        }
    }

    #[test]
    fn overrides_and_font_order_change_canonical_evidence_digest() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../preview/profiles/wall-static.json");
        let baseline = load(&path, &Overrides::default()).unwrap();
        let changed = load(&path, &Overrides { width: Some(801), height: Some(601), scale: Some(1.0), theme: Some("light".into()), timeout_ms: Some(1234), max_js_stack_bytes: None }).unwrap();
        assert_eq!((changed.physical_width, changed.physical_height), (801, 601));
        assert_eq!(changed.resolved_json["surface"]["width"], 801);
        assert_eq!(changed.resolved_json["theme"], "light");
        assert_eq!(changed.resolved_json["limits"]["maxRunWallMs"], 1234);
        assert_ne!(baseline.digest, changed.digest);
        let envelope = canonical(json!({"profile": changed.resolved_json, "fontEvidence": changed.font_evidence}));
        assert_eq!(changed.digest, sha256(&serde_json::to_vec(&envelope).unwrap()));
        let mut reordered_evidence = changed.font_evidence.clone();
        reordered_evidence["fallbackOrder"].as_array_mut().unwrap().reverse();
        let reordered = canonical(json!({"profile": changed.resolved_json, "fontEvidence": reordered_evidence}));
        assert_ne!(changed.digest, sha256(&serde_json::to_vec(&reordered).unwrap()));
        let reparsed: Profile = serde_json::from_value(changed.resolved_json.clone()).unwrap();
        assert_eq!(reparsed.surface.width, 801);
        assert_eq!(baseline.font_evidence["timingDeterministic"], false);
        let mut manifest: FontManifest = serde_json::from_str(include_str!("../../../../preview/fonts/fonts.json")).unwrap();
        let first = font_css(&manifest.fallback_order);
        manifest.fallback_order.reverse();
        assert_ne!(first, font_css(&manifest.fallback_order));
        assert!(load(&path, &Overrides { scale: Some(f64::NAN), ..Overrides::default() }).is_err());
    }

    #[test]
    fn font_aliases_are_css_strings_and_order_is_preserved() {
        assert_eq!(font_css(&["first".into(), "x\"; color:red; \\".into()]), "\"first\", \"x\\\"; color:red; \\\\\"");
        let mut manifest: FontManifest = serde_json::from_str(include_str!("../../../../preview/fonts/fonts.json")).unwrap();
        manifest.fallback_order.push("undeclared".into());
        assert!(validate_manifest(&manifest).is_err());
        manifest.fallback_order = vec![manifest.faces[0].family.clone(); 2];
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn bounded_resources_reject_oversized_files_and_path_traversal() {
        let temp = TempDir::new();
        fs::write(temp.0.join("oversized"), b"12345").unwrap();
        assert!(bounded_fs::read_input(&temp.0.join("oversized"), 4).is_err());
        for name in ["../secret", "/secret", "./font.ttf", "a//b", "a\\b"] {
            assert!(read_font_resource(&temp.0, name, 1024).is_err(), "{name}");
        }
        #[cfg(unix)] {
            fs::create_dir(temp.0.join("root")).unwrap();
            std::os::unix::fs::symlink(temp.0.join("oversized"), temp.0.join("root/escape")).unwrap();
            assert!(read_font_resource(&temp.0.join("root"), "escape", 1024).is_err());
        }
    }

    #[test]
    fn missing_fonts_hash_mismatches_and_invalid_font_bytes_fail_explicitly() {
        let temp = TempDir::new();
        let mut profile = default_profile();
        profile.fonts.manifest = "fonts.json".into();
        fs::write(temp.0.join("profile.json"), serde_json::to_vec(&profile).unwrap()).unwrap();
        fs::write(temp.0.join("LICENSE"), b"test-only license fixture").unwrap();
        let mut manifest = json!({"schemaVersion":1,"faces":[{"family":"test","path":"font.ttf","sha256":"0".repeat(64),"licensePath":"LICENSE"}],"fallbackOrder":["test"]});
        fs::write(temp.0.join("fonts.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
        let error = load(&temp.0.join("profile.json"), &Overrides::default()).err().unwrap();
        assert_eq!(error.code, "E_FONT");
        fs::write(temp.0.join("font.ttf"), b"not a font").unwrap();
        assert_eq!(load(&temp.0.join("profile.json"), &Overrides::default()).err().unwrap().code, "E_FONT_HASH");
        manifest["faces"][0]["sha256"] = json!(sha256(b"not a font"));
        fs::write(temp.0.join("fonts.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert_eq!(load(&temp.0.join("profile.json"), &Overrides::default()).err().unwrap().code, "E_FONT");
    }
}
