//! Closed scenario preflight. No application code is evaluated by input loading.
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::Read,
    path::Path,
};

use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeMap};

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

#[derive(Debug, Clone)]
pub enum Step {
    Settle { id: String, timeout_ms: Option<u64> },
    Capture { id: String, capture_id: String, required_frame_time_us: Option<u64> },
    PointerMove { id: String, location: PointerLocation },
    PointerClick { id: String, location: PointerLocation, button: PointerButton },
    KeyPress { id: String, surface: String, key: Key, modifiers: Vec<Modifier> },
    TextCommit { id: String, surface: String, text: String },
    Assert { id: String, target: Target, check: Check },
}

impl Step {
    pub fn id(&self) -> &str {
        match self {
            Self::Settle { id, .. } | Self::Capture { id, .. }
            | Self::PointerMove { id, .. } | Self::PointerClick { id, .. }
            | Self::KeyPress { id, .. } | Self::TextCommit { id, .. }
            | Self::Assert { id, .. } => id,
        }
    }

    pub fn op(&self) -> &'static str {
        match self {
            Self::Settle { .. } => "settle",
            Self::Capture { .. } => "capture",
            Self::PointerMove { .. } => "pointer.move",
            Self::PointerClick { .. } => "pointer.click",
            Self::KeyPress { .. } => "key.press",
            Self::TextCommit { .. } => "text.commit",
            Self::Assert { .. } => "assert",
        }
    }
}

#[derive(Debug, Clone)]
pub enum PointerLocation {
    Target(Target),
    Position { surface: String, x: f64, y: f64 },
}

impl Serialize for PointerLocation {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Target(target) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("target", target)?;
                map.end()
            }
            Self::Position { surface, x, y } => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("surface", surface)?;
                map.serialize_entry("position", &PositionInput { x: *x, y: *y })?;
                map.end()
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Target {
    pub surface: String,
    #[serde(flatten)]
    pub selector: Selector,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    TestId(String),
    Semantic { role: String, name: String },
}

impl Serialize for Selector {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::TestId(test_id) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("testId", test_id)?;
                map.end()
            }
            Self::Semantic { role, name } => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("role", role)?;
                map.serialize_entry("name", name)?;
                map.end()
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PointerButton { Primary, Secondary, Middle }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum Key {
    Tab, Enter, Escape, Space, ArrowUp, ArrowDown, ArrowLeft, ArrowRight,
    Home, End, Backspace, Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum Modifier { Shift, Control, Alt, Meta }

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "property", content = "equals", rename_all = "camelCase", deny_unknown_fields)]
pub enum Check {
    Exists(bool),
    Text(String),
    Role(String),
    Name(String),
    Enabled(bool),
    Focused(bool),
    Visible(bool),
    Value(String),
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
    #[serde(rename = "pointer.move")]
    PointerMove(PointerMoveInput),
    #[serde(rename = "pointer.click")]
    PointerClick(PointerClickInput),
    #[serde(rename = "key.press")]
    KeyPress(KeyPressInput),
    #[serde(rename = "text.commit")]
    TextCommit(TextCommitInput),
    #[serde(rename = "assert")]
    Assert(AssertInput),
    // Recognize unavailable operations only to reject them as capabilities, never
    // to accept or execute their future payloads. Supported payloads stay closed.
    #[serde(rename = "clock.advance")]
    ClockAdvance,
    #[serde(rename = "pointer.drag")]
    PointerDrag,
    #[serde(rename = "wheel")]
    Wheel,
    #[serde(rename = "service.emit")]
    ServiceEmit,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PointerMoveInput {
    id: String,
    #[serde(default, deserialize_with = "present")]
    target: Option<TargetInput>,
    #[serde(default, deserialize_with = "present")]
    surface: Option<String>,
    #[serde(default, deserialize_with = "present")]
    position: Option<PositionInput>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PointerClickInput {
    id: String,
    #[serde(default, deserialize_with = "present")]
    target: Option<TargetInput>,
    #[serde(default, deserialize_with = "present")]
    surface: Option<String>,
    #[serde(default, deserialize_with = "present")]
    position: Option<PositionInput>,
    button: PointerButton,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PositionInput { x: f64, y: f64 }

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TargetInput {
    surface: String,
    #[serde(default, deserialize_with = "present")]
    test_id: Option<String>,
    #[serde(default, deserialize_with = "present")]
    role: Option<String>,
    #[serde(default, deserialize_with = "present")]
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyPressInput {
    id: String,
    surface: String,
    key: Key,
    #[serde(default)]
    modifiers: Vec<Modifier>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextCommitInput { id: String, surface: String, text: String }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssertInput { id: String, target: TargetInput, check: Check }

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(input: D) -> Result<Option<T>, D::Error> {
    T::deserialize(input).map(Some)
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
            StepInput::PointerMove(input) => {
                let location = pointer_location(input.target, input.surface, input.position, &mut requirements)?;
                require(&mut requirements, "input.pointer");
                Step::PointerMove { id: input.id, location }
            }
            StepInput::PointerClick(input) => {
                let location = pointer_location(input.target, input.surface, input.position, &mut requirements)?;
                require(&mut requirements, "input.pointer");
                Step::PointerClick { id: input.id, location, button: input.button }
            }
            StepInput::KeyPress(input) => {
                validate_surface(&input.surface)?;
                require(&mut requirements, "input.keyboard");
                Step::KeyPress { id: input.id, surface: input.surface, key: input.key, modifiers: input.modifiers }
            }
            StepInput::TextCommit(input) => {
                validate_surface(&input.surface)?;
                require(&mut requirements, "input.text");
                Step::TextCommit { id: input.id, surface: input.surface, text: input.text }
            }
            StepInput::Assert(input) => {
                let target = target(input.target, &mut requirements)?;
                require(&mut requirements, "tree.inspect");
                if matches!(&input.check, Check::Role(_) | Check::Name(_)) {
                    require(&mut requirements, "selectors.semantic");
                }
                Step::Assert { id: input.id, target, check: input.check }
            }
            unsupported => {
                let operation = match unsupported {
                    StepInput::ClockAdvance => "clock.advance",
                    StepInput::PointerDrag => "pointer.drag",
                    StepInput::Wheel => "wheel",
                    StepInput::ServiceEmit => "service.emit",
                    StepInput::ImePreedit => "ime.preedit",
                    StepInput::ImeCommit => "ime.commit",
                    StepInput::ImeCancel => "ime.cancel",
                    StepInput::SurfaceResize => "surface.resize",
                    StepInput::SurfaceScale => "surface.scale",
                    StepInput::WaitUntil => "wait.until",
                    StepInput::CaptureSequence => "capture.sequence",
                    StepInput::LifecycleReload => "lifecycle.reload",
                    StepInput::Settle(_) | StepInput::Capture(_)
                    | StepInput::PointerMove(_) | StepInput::PointerClick(_)
                    | StepInput::KeyPress(_) | StepInput::TextCommit(_)
                    | StepInput::Assert(_) => unreachable!(),
                };
                return Err(unsupported_capability(format!("operation {operation} is unavailable in P2")));
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

fn require(requirements: &mut BTreeSet<String>, capability: &str) {
    if !requirements.contains(capability) {
        requirements.insert(capability.to_owned());
    }
}

fn validate_surface(surface: &str) -> Result<(), Error> {
    if surface != "main" {
        return Err(unsupported_capability("only the main logical surface is supported"));
    }
    Ok(())
}

fn target(input: TargetInput, requirements: &mut BTreeSet<String>) -> Result<Target, Error> {
    let selector = match (input.test_id, input.role, input.name) {
        (Some(test_id), None, None) => {
            require(requirements, "selectors.test-id");
            Selector::TestId(test_id)
        }
        (None, Some(role), Some(name)) => {
            require(requirements, "selectors.semantic");
            Selector::Semantic { role, name }
        }
        _ => return Err(invalid("target requires exactly testId or both role and name")),
    };
    validate_surface(&input.surface)?;
    Ok(Target { surface: input.surface, selector })
}

fn pointer_location(
    target_input: Option<TargetInput>,
    surface: Option<String>,
    position: Option<PositionInput>,
    requirements: &mut BTreeSet<String>,
) -> Result<PointerLocation, Error> {
    match (target_input, surface, position) {
        (Some(input), None, None) => Ok(PointerLocation::Target(target(input, requirements)?)),
        (None, Some(surface), Some(position)) => {
            if !position.x.is_finite() || !position.y.is_finite() {
                return Err(invalid("pointer position coordinates must be finite numbers"));
            }
            validate_surface(&surface)?;
            Ok(PointerLocation::Position { surface, x: position.x, y: position.y })
        }
        _ => Err(invalid("pointer action requires exactly target or surface plus position")),
    }
}

pub(super) fn safe_id(value: &str) -> bool {
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
        for op in ["clock.advance", "pointer.drag", "wheel", "service.emit", "ime.preedit", "ime.commit", "ime.cancel", "surface.resize", "surface.scale", "wait.until", "capture.sequence", "lifecycle.reload"] {
            let input = declaration(json!([{"id":"future","op":op,"target":{"surface":"main","testId":"counter"}}]));
            let error = check(&input, &["pixels", "virtual-time", "input.pointer"], true).unwrap_err();
            assert_eq!((error.code, error.exit_code), ("E_UNSUPPORTED_CAPABILITY", 2), "{op}: {error}");
            assert!(error.message.contains(op));
        }
    }

    #[test]
    fn interaction_payloads_preserve_native_action_data_and_trace_shapes() {
        let scenario = check(&declaration(json!([
            {"id":"move","op":"pointer.move","surface":"main","position":{"x":-12.5,"y":40}},
            {"id":"click","op":"pointer.click","target":{"surface":"main","testId":"increment"},"button":"secondary"},
            {"id":"key","op":"key.press","surface":"main","key":"ArrowLeft","modifiers":["Shift","Control"]},
            {"id":"tab","op":"key.press","surface":"main","key":"Tab"},
            {"id":"text","op":"text.commit","surface":"main","text":"你好\n"},
            {"id":"assert","op":"assert","target":{"surface":"main","role":"button","name":"Increment"},"check":{"property":"exists","equals":false}}
        ])), &["input.pointer", "input.keyboard", "input.text", "selectors.test-id", "selectors.semantic", "tree.inspect"], false).unwrap();
        assert!(matches!(&scenario.steps[0], Step::PointerMove {
            location: PointerLocation::Position { surface, x, y }, ..
        } if surface == "main" && *x == -12.5 && *y == 40.0));
        let Step::PointerClick { location, button, .. } = &scenario.steps[1] else { panic!("expected click") };
        assert_eq!(*button, PointerButton::Secondary);
        assert_eq!(serde_json::to_value(location).unwrap(), json!({"target":{"surface":"main","testId":"increment"}}));
        let Step::PointerMove { location, .. } = &scenario.steps[0] else { panic!("expected movement") };
        assert_eq!(serde_json::to_value(location).unwrap(), json!({"surface":"main","position":{"x":-12.5,"y":40.0}}));
        assert!(matches!(&scenario.steps[2], Step::KeyPress { key: Key::ArrowLeft, modifiers, .. }
            if modifiers.as_slice() == [Modifier::Shift, Modifier::Control]));
        assert!(matches!(&scenario.steps[3], Step::KeyPress { key: Key::Tab, modifiers, .. } if modifiers.is_empty()));
        assert!(matches!(&scenario.steps[4], Step::TextCommit { text, .. } if text == "你好\n"));
        let Step::Assert { target, check, .. } = &scenario.steps[5] else { panic!("expected assertion") };
        assert_eq!(*check, Check::Exists(false));
        assert_eq!(serde_json::to_value(target).unwrap(), json!({"surface":"main","role":"button","name":"Increment"}));
        assert_eq!(serde_json::to_value(check).unwrap(), json!({"property":"exists","equals":false}));
        assert_eq!(scenario.steps.iter().map(Step::op).collect::<Vec<_>>(),
            ["pointer.move", "pointer.click", "key.press", "key.press", "text.commit", "assert"]);
    }

    #[test]
    fn each_interaction_infers_all_requirements_without_declarations() {
        for (step, required) in [
            (json!({"id":"action","op":"pointer.move","surface":"main","position":{"x":0,"y":0}}), vec!["input.pointer"]),
            (json!({"id":"action","op":"pointer.click","target":{"surface":"main","testId":"button"},"button":"primary"}), vec!["input.pointer", "selectors.test-id"]),
            (json!({"id":"action","op":"pointer.move","target":{"surface":"main","role":"button","name":"Go"}}), vec!["input.pointer", "selectors.semantic"]),
            (json!({"id":"action","op":"key.press","surface":"main","key":"Enter"}), vec!["input.keyboard"]),
            (json!({"id":"action","op":"text.commit","surface":"main","text":"hello"}), vec!["input.text"]),
            (json!({"id":"action","op":"assert","target":{"surface":"main","testId":"label"},"check":{"property":"text","equals":"hello"}}), vec!["selectors.test-id", "tree.inspect"]),
            (json!({"id":"action","op":"assert","target":{"surface":"main","role":"button","name":"Go"},"check":{"property":"enabled","equals":true}}), vec!["selectors.semantic", "tree.inspect"]),
            (json!({"id":"action","op":"assert","target":{"surface":"main","testId":"button"},"check":{"property":"role","equals":"button"}}), vec!["selectors.semantic", "selectors.test-id", "tree.inspect"]),
            (json!({"id":"action","op":"assert","target":{"surface":"main","testId":"button"},"check":{"property":"name","equals":"Go"}}), vec!["selectors.semantic", "selectors.test-id", "tree.inspect"]),
        ] {
            let input = declaration(json!([step]));
            let scenario = check(&input, &required, false).unwrap();
            assert_eq!(scenario.required_capabilities, required, "{input}");
            for omitted in &required {
                let available = required.iter().copied().filter(|item| item != omitted).collect::<Vec<_>>();
                let error = check(&input, &available, false).unwrap_err();
                assert_eq!((error.code, error.exit_code), ("E_UNSUPPORTED_CAPABILITY", 2), "{input}");
                assert!(error.message.contains(*omitted), "{error}");
            }
        }
    }

    #[test]
    fn pointer_and_selector_forms_are_closed_and_mutually_exclusive() {
        for op in ["pointer.move", "pointer.click"] {
            for fields in [
                json!({}),
                json!({"target":null}),
                json!({"surface":"main"}),
                json!({"position":{"x":0,"y":0}}),
                json!({"surface":null,"position":{"x":0,"y":0}}),
                json!({"surface":"main","position":null}),
                json!({"surface":"main","position":{"x":"0","y":0}}),
                json!({"surface":"main","position":{"x":0}}),
                json!({"surface":"main","position":{"x":0,"y":null}}),
                json!({"surface":"main","position":{"x":0,"y":0,"z":0}}),
                json!({"target":{"surface":"main","testId":"button"},"surface":"main"}),
                json!({"target":{"surface":"main","testId":"button"},"position":{"x":0,"y":0}}),
                json!({"target":{"surface":"main","testId":"button"},"surface":"main","position":{"x":0,"y":0}}),
            ] {
                let mut step = fields;
                step["id"] = json!("action");
                step["op"] = json!(op);
                if op == "pointer.click" { step["button"] = json!("primary"); }
                schema_error(&declaration(json!([step])));
            }
        }
        for target in [
            json!({}),
            json!({"testId":"button"}),
            json!({"surface":"main"}),
            json!({"surface":"main","testId":null}),
            json!({"surface":"main","testId":42}),
            json!({"surface":"main","role":"button"}),
            json!({"surface":"main","name":"Go"}),
            json!({"surface":"main","role":"button","name":null}),
            json!({"surface":"main","testId":"button","role":"button","name":"Go"}),
            json!({"surface":"main","testId":"button","role":null}),
            json!({"surface":"main","role":"button","name":"Go","extra":true}),
        ] {
            schema_error(&declaration(json!([{"id":"action","op":"pointer.move","target":target}])));
            schema_error(&declaration(json!([{"id":"action","op":"assert","target":target,"check":{"property":"exists","equals":false}}])));
        }
        for coordinate in ["1e400", "-1e400", "NaN", "Infinity"] {
            let bytes = format!(r#"{{"schemaVersion":1,"id":"test","requiredCapabilities":[],"steps":[{{"id":"move","op":"pointer.move","surface":"main","position":{{"x":{coordinate},"y":0}}}}]}}"#);
            assert_eq!(parse(bytes.as_bytes(), &["input.pointer"], false).unwrap_err().code, "E_SCHEMA");
        }
    }

    #[test]
    fn native_input_payloads_reject_unknown_fields_nulls_and_unsupported_mappings() {
        for step in [
            json!({"id":"action","op":"pointer.click","target":{"surface":"main","testId":"button"}}),
            json!({"id":"action","op":"pointer.click","target":{"surface":"main","testId":"button"},"button":null}),
            json!({"id":"action","op":"pointer.click","target":{"surface":"main","testId":"button"},"button":"left"}),
            json!({"id":"action","op":"pointer.move","surface":"main","position":{"x":0,"y":0},"button":"primary"}),
            json!({"id":"action","op":"key.press","surface":"main","key":"A"}),
            json!({"id":"action","op":"key.press","surface":"main","key":"Spacebar"}),
            json!({"id":"action","op":"key.press","surface":"main","key":null}),
            json!({"id":"action","op":"key.press","surface":"main","key":"Tab","modifiers":null}),
            json!({"id":"action","op":"key.press","surface":"main","key":"Tab","modifiers":["Ctrl"]}),
            json!({"id":"action","op":"key.press","surface":"main","key":"Tab","modifiers":"Shift"}),
            json!({"id":"action","op":"key.press","surface":"main","key":"Tab","extra":true}),
            json!({"id":"action","op":"text.commit","surface":"main","text":null}),
            json!({"id":"action","op":"text.commit","surface":"main","text":42}),
            json!({"id":"action","op":"text.commit","surface":"main","text":"hello","target":{}}),
            json!({"id":"action","op":"assert","target":{"surface":"main","testId":"button"},"check":{"property":"exists","equals":true},"extra":true}),
        ] { schema_error(&declaration(json!([step]))); }
        for bytes in [
            br#"{"schemaVersion":1,"id":"test","requiredCapabilities":[],"steps":[{"id":"move","op":"pointer.move","target":{"surface":"main","testId":"a","testId":"b"}}]}"#.as_slice(),
            br#"{"schemaVersion":1,"id":"test","requiredCapabilities":[],"steps":[{"id":"move","op":"pointer.move","surface":"main","position":{"x":0,"x":1,"y":0}}]}"#.as_slice(),
        ] {
            assert_eq!(parse(bytes, &["input.pointer", "selectors.test-id"], false).unwrap_err().code, "E_SCHEMA");
        }
    }

    #[test]
    fn assertion_equality_types_are_strict_and_editor_value_is_separate() {
        for (property, expected) in [
            ("exists", json!(false)), ("text", json!("")),
            ("role", json!("button")), ("name", json!("Go")),
            ("enabled", json!(true)), ("focused", json!(false)),
            ("visible", json!(true)), ("value", json!("editor value")),
        ] {
            let step = json!({"id":"verify","op":"assert","target":{"surface":"main","testId":"editor"},"check":{"property":property,"equals":expected}});
            let scenario = check(&declaration(json!([step])), &["tree.inspect", "selectors.test-id", "selectors.semantic"], false).unwrap();
            let Step::Assert { check: predicate, .. } = &scenario.steps[0] else { panic!("expected assertion") };
            assert_eq!(serde_json::to_value(predicate).unwrap(), json!({"property":property,"equals":expected}));
            if property == "value" { assert!(matches!(predicate, Check::Value(value) if value == "editor value")); }
            for invalid_check in [
                json!({"property":property,"equals":null}),
                json!({"property":property,"equals":42}),
                json!({"property":property,"equals":if expected.is_boolean() { json!("true") } else { json!(true) }}),
                json!({"property":property}),
                json!({"property":property,"equals":expected,"extra":true}),
            ] {
                schema_error(&declaration(json!([{"id":"verify","op":"assert","target":{"surface":"main","testId":"editor"},"check":invalid_check}])));
            }
        }
        schema_error(&declaration(json!([{"id":"verify","op":"assert","target":{"surface":"main","testId":"editor"},"check":{"property":"javascript","equals":"true"}}])));
    }

    #[test]
    fn unsupported_surfaces_fail_preflight_for_every_target_and_input_form() {
        for step in [
            json!({"id":"action","op":"pointer.move","surface":"popup","position":{"x":0,"y":0}}),
            json!({"id":"action","op":"pointer.click","target":{"surface":"popup","testId":"button"},"button":"middle"}),
            json!({"id":"action","op":"key.press","surface":"popup","key":"Tab"}),
            json!({"id":"action","op":"text.commit","surface":"popup","text":""}),
            json!({"id":"action","op":"assert","target":{"surface":"popup","role":"button","name":"Go"},"check":{"property":"exists","equals":true}}),
        ] {
            let error = check(&declaration(json!([step])), &[], false).unwrap_err();
            assert_eq!((error.code, error.exit_code), ("E_UNSUPPORTED_CAPABILITY", 2));
            assert!(error.message.contains("main"));
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
