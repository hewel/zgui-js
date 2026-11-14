//! P1 scenario preflight. No application code is evaluated by input loading.
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::Read,
    path::Path,
};

use serde::{Deserialize, Deserializer};

use super::{Error, output::digest};

const MAX_INPUT_BYTES: u64 = 1024 * 1024;

#[derive(Debug)]
pub struct LoadedScenario {
    pub scenario: Scenario,
    /// Exact input bytes, including whitespace, for the saved run inputs.
    pub original: Vec<u8>,
    pub digest: String,
}

#[derive(Debug)]
pub struct Scenario {
    pub id: String,
    pub steps: Vec<Step>,
    /// Sorted union of explicit and inferred requirements, not just the declaration.
    pub required_capabilities: Vec<String>,
}

#[derive(Debug)]
pub enum Step {
    Settle { id: String, timeout_ms: Option<u64> },
    Capture { id: String, capture_id: String, required_frame_time_us: Option<u64> },
}

impl Step {
    pub fn id(&self) -> &str {
        match self {
            Self::Settle { id, .. } | Self::Capture { id, .. } => id,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScenarioInput {
    schema_version: u32,
    id: String,
    required_capabilities: Vec<String>,
    steps: Vec<StepInput>,
}

#[derive(Deserialize)]
#[serde(tag = "op")]
enum StepInput {
    #[serde(rename = "settle")]
    Settle(SettleInput),
    #[serde(rename = "capture")]
    Capture(CaptureInput),
    // Recognize unavailable operations only to reject them as capabilities, never
    // to accept or execute their future payloads. Supported payloads stay closed.
    #[serde(rename = "clock.advance")]
    ClockAdvance,
    #[serde(rename = "pointer.move")]
    PointerMove,
    #[serde(rename = "pointer.click")]
    PointerClick,
    #[serde(rename = "pointer.drag")]
    PointerDrag,
    #[serde(rename = "wheel")]
    Wheel,
    #[serde(rename = "key.press")]
    KeyPress,
    #[serde(rename = "text.commit")]
    TextCommit,
    #[serde(rename = "service.emit")]
    ServiceEmit,
    #[serde(rename = "assert")]
    Assert,
    #[serde(rename = "ime.preedit")]
    ImePreedit,
    #[serde(rename = "ime.commit")]
    ImeCommit,
    #[serde(rename = "ime.cancel")]
    ImeCancel,
    #[serde(rename = "surface.resize")]
    SurfaceResize,
    #[serde(rename = "surface.scale")]
    SurfaceScale,
    #[serde(rename = "wait.until")]
    WaitUntil,
    #[serde(rename = "capture.sequence")]
    CaptureSequence,
    #[serde(rename = "lifecycle.reload")]
    LifecycleReload,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SettleInput {
    id: String,
    #[serde(default, deserialize_with = "present_u64")]
    timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CaptureInput {
    id: String,
    capture_id: String,
    #[serde(default, deserialize_with = "present_u64")]
    required_frame_time_us: Option<u64>,
}

// Optional means absent, not null; a present budget/timestamp must be an integer.
fn present_u64<'de, D: Deserializer<'de>>(input: D) -> Result<Option<u64>, D::Error> {
    u64::deserialize(input).map(Some)
}

pub fn load(path: &Path, available: &[&str], virtual_profile: bool) -> Result<LoadedScenario, Error> {
    let original = read_input(path)?;
    let scenario = parse(&original, available, virtual_profile)?;
    let digest = digest(&original);
    Ok(LoadedScenario { scenario, original, digest })
}

fn read_input(path: &Path) -> Result<Vec<u8>, Error> {
    let metadata = fs::metadata(path)
        .map_err(|error| invalid(format!("scenario {}: {error}", path.display())))?;
    if !metadata.is_file() || metadata.len() > MAX_INPUT_BYTES {
        return Err(invalid("scenario must be a regular file of at most 1 MiB"));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)
        .and_then(|file| file.take(MAX_INPUT_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| invalid(format!("scenario {}: {error}", path.display())))?;
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        return Err(invalid("scenario exceeds 1 MiB"));
    }
    Ok(bytes)
}

fn parse(bytes: &[u8], available: &[&str], virtual_profile: bool) -> Result<Scenario, Error> {
    let input: ScenarioInput = serde_json::from_slice(bytes)
        .map_err(|error| invalid(format!("invalid scenario: {error}")))?;
    if input.schema_version != 1 {
        return Err(invalid("unsupported scenario schemaVersion; expected 1"));
    }
    if input.id.trim().is_empty() || input.id.chars().count() > 128 {
        return Err(invalid("scenario id must contain 1..=128 characters of nonblank text"));
    }
    let mut requirements = BTreeSet::new();
    for capability in input.required_capabilities {
        if !safe_id(&capability) {
            return Err(invalid("requiredCapabilities entries must match [a-z0-9][a-z0-9._-]*"));
        }
        requirements.insert(capability);
    }
    let mut needs_virtual_time = virtual_profile;
    let mut needs_pixels = false;
    let mut steps = Vec::with_capacity(input.steps.len());
    for input_step in input.steps {
        let step = match input_step {
            StepInput::Settle(input) => {
                if input.timeout_ms == Some(0) {
                    return Err(invalid("settle timeoutMs must be a positive integer"));
                }
                Step::Settle { id: input.id, timeout_ms: input.timeout_ms }
            }
            StepInput::Capture(input) => {
                if !safe_id(&input.capture_id) {
                    return Err(invalid("captureId must match [a-z0-9][a-z0-9._-]*"));
                }
                needs_pixels = true;
                if input.required_frame_time_us.is_some() {
                    needs_virtual_time = true;
                }
                Step::Capture {
                    id: input.id, capture_id: input.capture_id,
                    required_frame_time_us: input.required_frame_time_us,
                }
            }
            unsupported => {
                let operation = match unsupported {
                    StepInput::ClockAdvance => "clock.advance",
                    StepInput::PointerMove => "pointer.move",
                    StepInput::PointerClick => "pointer.click",
                    StepInput::PointerDrag => "pointer.drag",
                    StepInput::Wheel => "wheel",
                    StepInput::KeyPress => "key.press",
                    StepInput::TextCommit => "text.commit",
                    StepInput::ServiceEmit => "service.emit",
                    StepInput::Assert => "assert",
                    StepInput::ImePreedit => "ime.preedit",
                    StepInput::ImeCommit => "ime.commit",
                    StepInput::ImeCancel => "ime.cancel",
                    StepInput::SurfaceResize => "surface.resize",
                    StepInput::SurfaceScale => "surface.scale",
                    StepInput::WaitUntil => "wait.until",
                    StepInput::CaptureSequence => "capture.sequence",
                    StepInput::LifecycleReload => "lifecycle.reload",
                    StepInput::Settle(_) | StepInput::Capture(_) => unreachable!(),
                };
                return Err(unsupported_capability(format!("operation {operation} is unavailable in P1")));
            }
        };
        steps.push(step);
    }
    let mut step_ids = BTreeSet::new();
    let mut capture_ids = BTreeSet::new();
    for step in &steps {
        if !safe_id(step.id()) || !step_ids.insert(step.id()) {
            return Err(invalid("step id must be unique and match [a-z0-9][a-z0-9._-]*"));
        }
        if let Step::Capture { capture_id, .. } = step
            && !capture_ids.insert(capture_id.as_str()) {
            return Err(invalid("captureId must be unique"));
        }
    }
    if needs_pixels && !requirements.contains("pixels") {
        requirements.insert("pixels".to_owned());
    }
    if needs_virtual_time && !requirements.contains("virtual-time") {
        requirements.insert("virtual-time".to_owned());
    }
    if !virtual_profile && requirements.contains("virtual-time") {
        return Err(unsupported_capability("virtual-time is unavailable in a wall-clock profile"));
    }
    for capability in &requirements {
        if !available.contains(&capability.as_str()) {
            return Err(unsupported_capability(format!("required capability {capability} is unavailable")));
        }
    }
    Ok(Scenario { id: input.id, steps, required_capabilities: requirements.into_iter().collect() })
}

fn safe_id(value: &str) -> bool {
    if value.len() > 128 { return false; }
    let mut bytes = value.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-'))
}

fn invalid(message: impl Into<String>) -> Error { Error::new("E_SCHEMA", 2, message) }
fn unsupported_capability(message: impl Into<String>) -> Error {
    Error::new("E_UNSUPPORTED_CAPABILITY", 2, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicU64, Ordering};

    fn declaration(steps: Value) -> Value {
        json!({"schemaVersion":1,"id":"test","requiredCapabilities":[],"steps":steps})
    }

    fn check(input: &Value, available: &[&str], virtual_profile: bool) -> Result<Scenario, Error> {
        parse(&serde_json::to_vec(input).unwrap(), available, virtual_profile)
    }

    fn schema_error(input: &Value) {
        let error = check(input, &["pixels"], false).unwrap_err();
        assert_eq!((error.code, error.exit_code), ("E_SCHEMA", 2), "{error}");
    }

    #[test]
    fn capture_infers_pixels_and_preserves_explicit_requirements() {
        let mut input = declaration(json!([{"id":"shot","op":"capture","captureId":"shot"}]));
        let error = check(&input, &[], false).unwrap_err();
        assert_eq!((error.code, error.exit_code), ("E_UNSUPPORTED_CAPABILITY", 2));
        input["requiredCapabilities"] = json!(["selectors.test-id", "pixels", "pixels"]);
        let scenario = check(&input, &["selectors.test-id", "pixels"], false).unwrap();
        assert_eq!(scenario.required_capabilities, ["pixels", "selectors.test-id"]);
        let error = check(&input, &["pixels"], false).unwrap_err();
        assert_eq!(error.code, "E_UNSUPPORTED_CAPABILITY");
        assert!(error.message.contains("selectors.test-id"));
    }

    #[test]
    fn frame_timestamp_and_virtual_profile_cannot_omit_virtual_time() {
        let input = declaration(json!([{"id":"shot","op":"capture","captureId":"shot","requiredFrameTimeUs":0}]));
        for available in [&["pixels"][..], &["pixels", "virtual-time"][..]] {
            assert_eq!(check(&input, available, false).unwrap_err().code, "E_UNSUPPORTED_CAPABILITY");
        }
        let scenario = check(&input, &["pixels", "virtual-time"], true).unwrap();
        assert_eq!(scenario.required_capabilities, ["pixels", "virtual-time"]);
        let no_steps = declaration(json!([]));
        assert_eq!(check(&no_steps, &["pixels"], true).unwrap_err().code, "E_UNSUPPORTED_CAPABILITY");
        let mut explicit = no_steps;
        explicit["requiredCapabilities"] = json!(["virtual-time"]);
        assert_eq!(check(&explicit, &["virtual-time"], false).unwrap_err().code, "E_UNSUPPORTED_CAPABILITY");
    }

    #[test]
    fn closed_supported_payloads_reject_ambiguous_fields_and_types() {
        for step in [
            json!({"id":"ready","op":"settle","timeoutMs":0}),
            json!({"id":"ready","op":"settle","timeoutMs":null}),
            json!({"id":"ready","op":"settle","timeoutMs":1.5}),
            json!({"id":"ready","op":"settle","timeoutMs":"2"}),
            json!({"id":"ready","op":"settle","timeoutMs":-1}),
            json!({"id":"ready","op":"settle","timeoutMS":2}),
            json!({"id":"shot","op":"capture","captureId":"shot","requiredFrameTimeUs":null}),
            json!({"id":"shot","op":"capture","captureId":"shot","requiredFrameTimeUs":0.5}),
            json!({"id":"shot","op":"capture","captureId":"shot","target":{}}),
            json!({"id":"shot","op":"capture","captureId":3}),
            json!({"id":"unknown","op":"capture.typo"}),
            json!({"id":"unknown","op":4}),
        ] { schema_error(&declaration(json!([step]))); }
        let mut input = declaration(json!([]));
        input["schemaVersion"] = json!(2);
        schema_error(&input);
        input["schemaVersion"] = json!(1);
        input["unknown"] = json!(true);
        schema_error(&input);
        let duplicate = br#"{"schemaVersion":1,"id":"test","requiredCapabilities":[],"steps":[{"id":"one","id":"two","op":"settle"}]}"#;
        assert_eq!(parse(duplicate, &[], false).unwrap_err().code, "E_SCHEMA");
        let duplicate_op = br#"{"schemaVersion":1,"id":"test","requiredCapabilities":[],"steps":[{"id":"one","op":"settle","op":"capture"}]}"#;
        assert_eq!(parse(duplicate_op, &[], false).unwrap_err().code, "E_SCHEMA");
    }

    #[test]
    fn path_ids_and_duplicate_output_names_are_rejected() {
        for id in ["", ".", "..", "../escape", "a/../b", "/absolute", "a\\b", "a%2fb", "Upper", "a\0b", "a b"] {
            schema_error(&declaration(json!([{"id":id,"op":"settle"}])));
            schema_error(&declaration(json!([{"id":"shot","op":"capture","captureId":id}])));
        }
        schema_error(&declaration(json!([
            {"id":"same","op":"settle"}, {"id":"same","op":"capture","captureId":"shot"}
        ])));
        schema_error(&declaration(json!([
            {"id":"one","op":"capture","captureId":"same"}, {"id":"two","op":"capture","captureId":"same"}
        ])));
        let scenario = check(&declaration(json!([
            {"id":"a..b-0_1","op":"settle","timeoutMs":1},
            {"id":"shot","op":"capture","captureId":"shot"}
        ])), &["pixels"], false).unwrap();
        assert_eq!(scenario.steps[0].id(), "a..b-0_1");
        assert!(matches!(&scenario.steps[0], Step::Settle { timeout_ms: Some(1), .. }));
    }

    #[test]
    fn future_operations_fail_preflight_even_when_not_declared() {
        for op in ["clock.advance", "pointer.move", "pointer.click", "pointer.drag", "wheel", "key.press", "text.commit", "service.emit", "assert", "ime.preedit", "ime.commit", "ime.cancel", "surface.resize", "surface.scale", "wait.until", "capture.sequence", "lifecycle.reload"] {
            let input = declaration(json!([{"id":"future","op":op,"target":{"surface":"main","testId":"counter"}}]));
            let error = check(&input, &["pixels", "virtual-time", "input.pointer"], true).unwrap_err();
            assert_eq!((error.code, error.exit_code), ("E_UNSUPPORTED_CAPABILITY", 2), "{op}: {error}");
            assert!(error.message.contains(op));
        }
    }

    struct InputFile(std::path::PathBuf);

    impl InputFile {
        fn new(bytes: &[u8]) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!("zgui-preview-scenario-{}-{}.json", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            use std::io::Write;
            let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
            file.write_all(bytes).unwrap();
            Self(path)
        }
    }

    impl Drop for InputFile {
        fn drop(&mut self) { fs::remove_file(&self.0).unwrap(); }
    }

    #[test]
    fn byte_limit_is_inclusive_and_hashes_exact_original_bytes() {
        let mut bytes = br#"{ "schemaVersion": 1, "id": "exact", "requiredCapabilities": [], "steps": [] }"#.to_vec();
        bytes.resize(MAX_INPUT_BYTES as usize, b' ');
        let file = InputFile::new(&bytes);
        let loaded = load(&file.0, &[], false).unwrap();
        assert_eq!(loaded.original, bytes);
        assert_eq!(loaded.digest, digest(&bytes));
        assert_eq!(loaded.scenario.id, "exact");
        bytes.push(b' ');
        let too_large = InputFile::new(&bytes);
        let error = load(&too_large.0, &[], false).unwrap_err();
        assert_eq!((error.code, error.exit_code), ("E_SCHEMA", 2));
        assert_eq!(load(&std::env::temp_dir(), &[], false).unwrap_err().code, "E_SCHEMA");
    }
}
