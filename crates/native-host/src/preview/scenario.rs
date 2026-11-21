//! Closed scenario preflight. No application code is evaluated by input loading.
use std::{collections::BTreeSet, path::Path};

use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeMap};

use crate::shell::Monitor;
use crate::bounded_fs;
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
    Capture { id: String, capture_id: String, surface: String, required_frame_time_us: Option<u64>, allowed_pending_assets: Vec<String> },
    PointerMove { id: String, location: PointerLocation, pointer: PointerDevice },
    PointerClick { id: String, location: PointerLocation, button: PointerButton, pointer: PointerDevice },
    PointerDown { id: String, location: PointerLocation, button: PointerButton, pointer: PointerDevice },
    PointerUp { id: String, location: PointerLocation, button: PointerButton, pointer: PointerDevice },
    PointerCancel { id: String, location: PointerLocation, pointer: PointerDevice },
    Wheel { id: String, location: PointerLocation, delta: WheelDelta, phase: WheelPhase },
    DragEnter { id: String, location: PointerLocation, paths: Vec<String> },
    DragMove { id: String, location: PointerLocation },
    DragLeave { id: String, surface: String },
    DragDrop { id: String, location: PointerLocation, paths: Vec<String> },
    KeyPress { id: String, surface: String, key: Key, modifiers: Vec<Modifier> },
    TextCommit { id: String, surface: String, text: String },
    Assert { id: String, target: Target, check: Check },
    ClockAdvance { id: String, by_ms: u64 },
    ServiceEmit { id: String, channel: String, value: serde_json::Value },
    AssertService { id: String, check: ServiceCallCheck },
    SurfaceResize { id: String, surface: String, width: u32, height: u32 },
    SurfaceScale { id: String, surface: String, scale: f64 },
    SurfaceColorScheme { id: String, surface: String, color_scheme: String },
    SurfaceRecreate { id: String, surface: String },
    SurfaceCloseRequest { id: String, surface: String },
    MonitorsSet { id: String, monitors: Vec<Monitor> },
    PopupDone { id: String, surface: String },
}

impl Step {
    pub fn id(&self) -> &str {
        match self {
            Self::Settle { id, .. } | Self::Capture { id, .. }
            | Self::PointerMove { id, .. } | Self::PointerClick { id, .. }
            | Self::PointerDown { id, .. } | Self::PointerUp { id, .. } | Self::PointerCancel { id, .. }
            | Self::Wheel { id, .. }
            | Self::DragEnter { id, .. } | Self::DragMove { id, .. } | Self::DragLeave { id, .. } | Self::DragDrop { id, .. }
            | Self::KeyPress { id, .. } | Self::TextCommit { id, .. }
            | Self::Assert { id, .. } | Self::AssertService { id, .. }
            | Self::ClockAdvance { id, .. } | Self::ServiceEmit { id, .. }
            | Self::SurfaceResize { id, .. } | Self::SurfaceScale { id, .. } | Self::SurfaceColorScheme { id, .. }
            | Self::SurfaceRecreate { id, .. } | Self::SurfaceCloseRequest { id, .. } | Self::MonitorsSet { id, .. } | Self::PopupDone { id, .. } => id,
        }
    }

    pub fn op(&self) -> &'static str {
        match self {
            Self::Settle { .. } => "settle",
            Self::Capture { .. } => "capture",
            Self::PointerMove { .. } => "pointer.move",
            Self::PointerClick { .. } => "pointer.click",
            Self::PointerDown { .. } => "pointer.down",
            Self::PointerUp { .. } => "pointer.up",
            Self::PointerCancel { .. } => "pointer.cancel",
            Self::Wheel { .. } => "wheel",
            Self::DragEnter { .. } => "drag.enter",
            Self::DragMove { .. } => "drag.move",
            Self::DragLeave { .. } => "drag.leave",
            Self::DragDrop { .. } => "drag.drop",
            Self::KeyPress { .. } => "key.press",
            Self::TextCommit { .. } => "text.commit",
            Self::Assert { .. } | Self::AssertService { .. } => "assert",
            Self::ClockAdvance { .. } => "clock.advance",
            Self::ServiceEmit { .. } => "service.emit",
            Self::SurfaceResize { .. } => "surface.resize",
            Self::SurfaceScale { .. } => "surface.scale",
            Self::SurfaceColorScheme { .. } => "surface.colorScheme",
            Self::SurfaceRecreate { .. } => "surface.recreate",
            Self::SurfaceCloseRequest { .. } => "surface.closeRequest",
            Self::MonitorsSet { .. } => "monitors.set",
            Self::PopupDone { .. } => "popupDone",
        }
    }
}

#[derive(Debug, Clone)]
pub enum PointerLocation {
    Target(Target),
    Position { surface: String, x: f64, y: f64 },
}

impl PointerLocation {
    pub fn surface(&self) -> &str {
        match self { Self::Target(target) => &target.surface, Self::Position { surface, .. } => surface }
    }
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
#[serde(rename_all = "lowercase")]
pub enum PointerKind { Mouse, Touch, Pen, Unknown }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PointerDevice {
    #[serde(serialize_with = "serialize_pointer_id")]
    pub id: u64,
    pub kind: PointerKind,
    pub primary: bool,
}

impl Default for PointerDevice {
    fn default() -> Self { Self { id: 0, kind: PointerKind::Mouse, primary: true } }
}

fn serialize_pointer_id<S: Serializer>(id: &u64, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&id.to_string())
}

#[derive(Debug, Clone, Copy)]
pub enum PointerOperation { Move, Down(PointerButton), Up(PointerButton), Cancel, Click(PointerButton) }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WheelUnit { Pixels, Lines }

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WheelPhase { #[default] Discrete, Started, Moved, Ended, Momentum }

crate::schema::object_struct! {
    #[serde(deny_unknown_fields)]
    pub struct WheelDelta { pub x: f64, pub y: f64, pub unit: WheelUnit }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum Key {
    Tab, Enter, Escape, Space, ArrowUp, ArrowDown, ArrowLeft, ArrowRight,
    Home, End, PageUp, PageDown, Backspace, Delete,
    #[serde(rename = "character")]
    Character(String),
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

crate::schema::object_struct! {
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct ServiceCallCheck {
        pub service:String,
        pub method:String,
        pub count:u64,
        #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
        pub error_code:Option<String>,
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
    #[serde(rename = "pointer.move")]
    PointerMove(PointerMoveInput),
    #[serde(rename = "pointer.click")]
    PointerClick(PointerButtonInput),
    #[serde(rename = "pointer.down")]
    PointerDown(PointerButtonInput),
    #[serde(rename = "pointer.up")]
    PointerUp(PointerButtonInput),
    #[serde(rename = "pointer.cancel")]
    PointerCancel(PointerMoveInput),
    #[serde(rename = "key.press")]
    KeyPress(KeyPressInput),
    #[serde(rename = "text.commit")]
    TextCommit(TextCommitInput),
    #[serde(rename = "assert")]
    Assert(AssertInput),
    #[serde(rename = "clock.advance")]
    ClockAdvance(AdvanceInput),
    #[serde(rename = "service.emit")]
    ServiceEmit(EmitInput),
    #[serde(rename = "surface.resize")]
    SurfaceResize(ResizeInput),
    #[serde(rename = "surface.scale")]
    SurfaceScale(ScaleInput),
    #[serde(rename = "surface.colorScheme")]
    SurfaceColorScheme(ColorSchemeInput),
    #[serde(rename = "surface.recreate")]
    SurfaceRecreate(NamedSurfaceInput),
    #[serde(rename = "surface.closeRequest")]
    SurfaceCloseRequest(NamedSurfaceInput),
    #[serde(rename = "monitors.set")]
    MonitorsSet(MonitorsInput),
    #[serde(rename = "popupDone")]
    PopupDone(NamedSurfaceInput),
    #[serde(rename = "wheel")]
    Wheel(WheelInput),
    #[serde(rename = "drag.enter")]
    DragEnter(DragPathsInput),
    #[serde(rename = "drag.move")]
    DragMove(DragMoveInput),
    #[serde(rename = "drag.leave")]
    DragLeave(NamedSurfaceInput),
    #[serde(rename = "drag.drop")]
    DragDrop(DragPathsInput),
    // Later operations remain explicit capability failures, never silently ignored.
    #[serde(rename = "pointer.drag")]
    PointerDrag,
    #[serde(rename = "ime.preedit")]
    ImePreedit,
    #[serde(rename = "ime.commit")]
    ImeCommit,
    #[serde(rename = "ime.cancel")]
    ImeCancel,
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
    #[serde(default, deserialize_with = "present")]
    surface: Option<String>,
    #[serde(default, deserialize_with = "present_u64")]
    required_frame_time_us: Option<u64>,
    #[serde(default)]
    allowed_pending_assets:Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MonitorsInput {
    id: String,
    monitors: Vec<Monitor>,
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
    #[serde(default, deserialize_with = "present")]
    pointer: Option<PointerDeviceInput>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PointerButtonInput {
    id: String,
    #[serde(default, deserialize_with = "present")]
    target: Option<TargetInput>,
    #[serde(default, deserialize_with = "present")]
    surface: Option<String>,
    #[serde(default, deserialize_with = "present")]
    position: Option<PositionInput>,
    button: PointerButton,
    #[serde(default, deserialize_with = "present")]
    pointer: Option<PointerDeviceInput>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PointerDeviceInput {
    id: String,
    kind: PointerKind,
    #[serde(default = "primary_pointer")]
    primary: bool,
}

fn primary_pointer() -> bool { true }

fn pointer_device(input: Option<PointerDeviceInput>) -> Result<PointerDevice, Error> {
    let Some(input) = input else { return Ok(PointerDevice::default()); };
    let id = input.id.parse::<u64>().map_err(|_| invalid("pointer id must be a canonical decimal u64 string"))?;
    if id.to_string() != input.id { return Err(invalid("pointer id must be a canonical decimal u64 string")); }
    Ok(PointerDevice { id, kind: input.kind, primary: input.primary })
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AssertInput {
    id:String,
    #[serde(default, deserialize_with = "present")]
    target:Option<TargetInput>,
    #[serde(default, deserialize_with = "present")]
    check:Option<Check>,
    #[serde(default, deserialize_with = "present")]
    service_call:Option<ServiceCallCheck>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AdvanceInput { id:String,by_ms:u64 }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmitInput { id:String,channel:String,value:serde_json::Value }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResizeInput { id:String,surface:String,width:u32,height:u32 }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScaleInput { id:String,surface:String,scale:f64 }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ColorSchemeInput { id:String,surface:String,color_scheme:String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NamedSurfaceInput { id:String,surface:String }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WheelInput {
    id: String,
    #[serde(default, deserialize_with = "present_object")]
    target: Option<TargetInput>,
    #[serde(default, deserialize_with = "present")]
    surface: Option<String>,
    #[serde(default, deserialize_with = "present_object")]
    position: Option<PositionInput>,
    delta: WheelDelta,
    #[serde(default)]
    phase: WheelPhase,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DragPathsInput {
    id: String,
    #[serde(default, deserialize_with = "present_object")]
    target: Option<TargetInput>,
    #[serde(default, deserialize_with = "present")]
    surface: Option<String>,
    #[serde(default, deserialize_with = "present_object")]
    position: Option<PositionInput>,
    paths: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DragMoveInput {
    id: String,
    #[serde(default, deserialize_with = "present_object")]
    target: Option<TargetInput>,
    #[serde(default, deserialize_with = "present")]
    surface: Option<String>,
    #[serde(default, deserialize_with = "present_object")]
    position: Option<PositionInput>,
}

fn present_object<'de, D: Deserializer<'de>, T: Deserialize<'de>>(input: D) -> Result<Option<T>, D::Error> {
    crate::schema::deserialize_object(input).map(Some)
}

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
    bounded_fs::read_input(path, MAX_INPUT_BYTES)
        .map_err(|error| super::schema_read_error("scenario", path, error))
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
                let surface = input.surface.unwrap_or_else(|| "main".into());
                validate_surface(&surface, &mut requirements)?;
                needs_pixels = true;
                if input.required_frame_time_us.is_some() {
                    needs_virtual_time = true;
                }
                if input.allowed_pending_assets.len()>4096 || input.allowed_pending_assets.iter().any(|id|!safe_id(id))
                    || input.allowed_pending_assets.iter().collect::<BTreeSet<_>>().len()!=input.allowed_pending_assets.len() {
                    return Err(invalid("allowedPendingAssets must contain unique bounded resource IDs"));
                }
                if !input.allowed_pending_assets.is_empty() { require(&mut requirements,"assets.native-readiness"); }
                Step::Capture {
                    id: input.id, capture_id: input.capture_id, surface,
                    required_frame_time_us: input.required_frame_time_us, allowed_pending_assets:input.allowed_pending_assets,
                }
            }
            StepInput::PointerMove(input) => {
                let location = pointer_location(input.target, input.surface, input.position, &mut requirements)?;
                require(&mut requirements, "input.pointer");
                Step::PointerMove { id: input.id, location, pointer: pointer_device(input.pointer)? }
            }
            StepInput::PointerClick(input) => {
                let location = pointer_location(input.target, input.surface, input.position, &mut requirements)?;
                require(&mut requirements, "input.pointer");
                Step::PointerClick { id: input.id, location, button: input.button, pointer: pointer_device(input.pointer)? }
            }
            StepInput::PointerDown(input) => {
                let location = pointer_location(input.target, input.surface, input.position, &mut requirements)?;
                require(&mut requirements, "input.pointer");
                Step::PointerDown { id: input.id, location, button: input.button, pointer: pointer_device(input.pointer)? }
            }
            StepInput::PointerUp(input) => {
                let location = pointer_location(input.target, input.surface, input.position, &mut requirements)?;
                require(&mut requirements, "input.pointer");
                Step::PointerUp { id: input.id, location, button: input.button, pointer: pointer_device(input.pointer)? }
            }
            StepInput::PointerCancel(input) => {
                let location = pointer_location(input.target, input.surface, input.position, &mut requirements)?;
                require(&mut requirements, "input.pointer");
                Step::PointerCancel { id: input.id, location, pointer: pointer_device(input.pointer)? }
            }
            StepInput::KeyPress(input) => {
                validate_surface(&input.surface, &mut requirements)?;
                if matches!(&input.key, Key::Character(text) if text.is_empty()) {
                    return Err(invalid("character key must contain nonempty text"));
                }
                require(&mut requirements, "input.keyboard");
                Step::KeyPress { id: input.id, surface: input.surface, key: input.key, modifiers: input.modifiers }
            }
            StepInput::TextCommit(input) => {
                validate_surface(&input.surface, &mut requirements)?;
                require(&mut requirements, "input.text");
                Step::TextCommit { id: input.id, surface: input.surface, text: input.text }
            }
            StepInput::Assert(input) => {
                match(input.target,input.check,input.service_call) {
                    (Some(selected),Some(check),None)=>{
                        let target=target(selected,&mut requirements)?;
                        require(&mut requirements,"tree.inspect");
                        if matches!(&check,Check::Role(_)|Check::Name(_)) { require(&mut requirements,"selectors.semantic"); }
                        Step::Assert{id:input.id,target,check}
                    }
                    (None,None,Some(check))=>{
                        if !safe_id(&check.service) || check.method.is_empty() || check.method.len()>128
                            || check.error_code.as_ref().is_some_and(|code|code.is_empty() || code.len()>512) {
                            return Err(invalid("serviceCall names and optional errorCode must be bounded nonempty strings"));
                        }
                        require(&mut requirements,"services.fixtures");
                        Step::AssertService{id:input.id,check}
                    }
                    _=>return Err(invalid("assert requires exactly target/check or serviceCall")),
                }
            }
            StepInput::ClockAdvance(input)=>{
                needs_virtual_time=true;
                Step::ClockAdvance{id:input.id,by_ms:input.by_ms}
            }
            StepInput::ServiceEmit(input)=>{
                if !safe_id(&input.channel) || !input.value.is_object() { return Err(invalid("service.emit requires a bounded channel and typed object value")); }
                require(&mut requirements,"services.fixtures");
                Step::ServiceEmit{id:input.id,channel:input.channel,value:input.value}
            }
            StepInput::SurfaceResize(input)=>{
                validate_surface(&input.surface, &mut requirements)?;
                if !(1..=16384).contains(&input.width) || !(1..=16384).contains(&input.height) { return Err(invalid("resize dimensions must be 1..16384 logical pixels")); }
                require(&mut requirements,"surface.resize");
                Step::SurfaceResize{id:input.id,surface:input.surface,width:input.width,height:input.height}
            }
            StepInput::SurfaceScale(input)=>{
                validate_surface(&input.surface, &mut requirements)?;
                if !input.scale.is_finite() || input.scale<=0.0 || input.scale>8.0 { return Err(invalid("surface scale must be finite and in (0,8]")); }
                require(&mut requirements,"surface.scale");
                Step::SurfaceScale{id:input.id,surface:input.surface,scale:input.scale}
            }
            StepInput::SurfaceColorScheme(input)=>{
                validate_surface(&input.surface, &mut requirements)?;
                if !matches!(input.color_scheme.as_str(), "light" | "dark") { return Err(invalid("colorScheme must be light or dark")); }
                require(&mut requirements,"surface.color-scheme");
                Step::SurfaceColorScheme{id:input.id,surface:input.surface,color_scheme:input.color_scheme}
            }
            StepInput::SurfaceRecreate(input)=>{
                validate_surface(&input.surface, &mut requirements)?;
                require(&mut requirements,"surface.recreate");
                Step::SurfaceRecreate{id:input.id,surface:input.surface}
            }
            StepInput::SurfaceCloseRequest(input) => {
                validate_surface(&input.surface, &mut requirements)?;
                Step::SurfaceCloseRequest { id: input.id, surface: input.surface }
            }
            StepInput::MonitorsSet(input) => {
                validate_monitors(&input.monitors)?;
                require(&mut requirements, "surface.multiple");
                Step::MonitorsSet { id: input.id, monitors: input.monitors }
            }
            StepInput::PopupDone(input) => {
                validate_surface(&input.surface, &mut requirements)?;
                require(&mut requirements, "surface.multiple");
                Step::PopupDone { id: input.id, surface: input.surface }
            }
            StepInput::Wheel(input) => {
                let location = pointer_location(input.target, input.surface, input.position, &mut requirements)?;
                if [input.delta.x, input.delta.y].into_iter().any(|v| !v.is_finite() || !(v as f32).is_finite()) {
                    return Err(invalid("wheel delta exceeds native finite coordinate range"));
                }
                if let PointerLocation::Position { x, y, .. } = &location
                    && (!(*x as f32).is_finite() || !(*y as f32).is_finite())
                {
                    return Err(invalid("wheel position exceeds native finite coordinate range"));
                }
                require(&mut requirements, "input.wheel");
                Step::Wheel { id:input.id, location, delta:input.delta, phase:input.phase }
            }
            StepInput::DragEnter(input) => {
                let location = pointer_location(input.target, input.surface, input.position, &mut requirements)?;
                Step::DragEnter { id: input.id, location, paths: input.paths }
            }
            StepInput::DragMove(input) => {
                let location = pointer_location(input.target, input.surface, input.position, &mut requirements)?;
                Step::DragMove { id: input.id, location }
            }
            StepInput::DragLeave(input) => {
                validate_surface(&input.surface, &mut requirements)?;
                Step::DragLeave { id: input.id, surface: input.surface }
            }
            StepInput::DragDrop(input) => {
                let location = pointer_location(input.target, input.surface, input.position, &mut requirements)?;
                Step::DragDrop { id: input.id, location, paths: input.paths }
            }
            unsupported => {
                let operation = match unsupported {
                    StepInput::PointerDrag => "pointer.drag",
                    StepInput::ImePreedit => "ime.preedit",
                    StepInput::ImeCommit => "ime.commit",
                    StepInput::ImeCancel => "ime.cancel",
                    StepInput::WaitUntil => "wait.until",
                    StepInput::CaptureSequence => "capture.sequence",
                    StepInput::LifecycleReload => "lifecycle.reload",
                    StepInput::Settle(_) | StepInput::Capture(_)
                    | StepInput::PointerMove(_) | StepInput::PointerClick(_)
                    | StepInput::PointerDown(_) | StepInput::PointerUp(_) | StepInput::PointerCancel(_)
                    | StepInput::Wheel(_) | StepInput::DragEnter(_) | StepInput::DragMove(_) | StepInput::DragLeave(_) | StepInput::DragDrop(_)
                    | StepInput::MonitorsSet(_) | StepInput::PopupDone(_)
                    | StepInput::KeyPress(_) | StepInput::TextCommit(_)
                    | StepInput::Assert(_) | StepInput::ClockAdvance(_) | StepInput::ServiceEmit(_)
                    | StepInput::SurfaceResize(_) | StepInput::SurfaceScale(_) | StepInput::SurfaceRecreate(_) | StepInput::SurfaceCloseRequest(_) | StepInput::SurfaceColorScheme(_) => unreachable!(),
                };
                return Err(unsupported_capability(format!("operation {operation} is unavailable")));
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
        if let Step::Capture { capture_id, surface, .. } = step
            && !capture_ids.insert((surface.as_str(), capture_id.as_str())) {
            return Err(invalid("captureId must be unique on each surface"));
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

fn validate_surface(surface: &str, requirements: &mut BTreeSet<String>) -> Result<(), Error> {
    if !safe_surface_name(surface) {
        return Err(invalid("surface must match [a-zA-Z0-9][a-zA-Z0-9._-]* and contain at most 128 bytes"));
    }
    if surface != "main" { require(requirements, "surface.multiple"); }
    Ok(())
}

fn validate_monitors(monitors: &[Monitor]) -> Result<(), Error> {
    if monitors.len() > 32 { return Err(invalid("monitors.set accepts at most 32 outputs")); }
    let mut ids = BTreeSet::new();
    for monitor in monitors {
        let (width, height) = (monitor.width, monitor.height);
        if monitor.id.is_empty() || monitor.id.len() > 128 || !ids.insert(&monitor.id)
            || monitor.name.trim().is_empty() || monitor.name.len() > 128
            || ![monitor.x, monitor.y].into_iter().all(|value| value.abs() <= f64::from(i32::MAX))
            || !(1.0..=16384.0).contains(&width) || !(1.0..=16384.0).contains(&height)
            || monitor.logical_width.is_some_and(|value| !(1.0..=16384.0).contains(&value))
            || monitor.logical_height.is_some_and(|value| !(1.0..=16384.0).contains(&value))
            || !(monitor.scale > 0.0 && monitor.scale <= 8.0)
            || (width * height) > super::profile::MAX_PHYSICAL_PIXELS as f64
            || monitor.refresh_rate_milli_hz.is_some_and(|rate| !(1..=1_000_000).contains(&rate))
        {
            return Err(invalid("monitors.set requires unique bounded output IDs and valid name, geometry, scale and refresh rate"));
        }
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
    validate_surface(&input.surface, requirements)?;
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
            validate_surface(&surface, requirements)?;
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

pub(super) fn safe_surface_name(value: &str) -> bool {
    crate::shell::valid_name(value)
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
    fn scenarios_can_require_transparent_capture_with_run_capabilities() {
        let mut input = declaration(json!([{"id":"shot","op":"capture","captureId":"shot"}]));
        input["requiredCapabilities"] = json!(["pixels", "capture.transparent"]);
        let scenario = check(&input, super::super::RUN_CAPABILITIES, false).unwrap();
        assert_eq!(scenario.required_capabilities, ["capture.transparent", "pixels"]);
        assert_eq!(check(&input, &["pixels"], false).unwrap_err().code, "E_UNSUPPORTED_CAPABILITY");
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
        for op in ["pointer.drag", "ime.preedit", "ime.commit", "ime.cancel", "wait.until", "capture.sequence", "lifecycle.reload"] {
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
    fn wheel_payloads_infer_capability_and_reject_ambiguous_values() {
        let step = json!({"id":"spin","op":"wheel","target":{"surface":"main","testId":"list"},
            "delta":{"x":-1.5,"y":3,"unit":"lines"}});
        let input = declaration(json!([step.clone()]));
        let scenario = check(&input, &["input.wheel", "selectors.test-id"], false).unwrap();
        assert_eq!(scenario.required_capabilities, ["input.wheel", "selectors.test-id"]);
        let Step::Wheel { location, delta, phase, .. } = &scenario.steps[0] else { panic!("expected wheel") };
        assert!(matches!(location, PointerLocation::Target(_)));
        assert_eq!((delta.x, delta.y, delta.unit), (-1.5, 3.0, WheelUnit::Lines));
        assert_eq!(*phase, WheelPhase::Discrete, "a missing phase is discrete");
        let error = check(&input, &["selectors.test-id"], false).unwrap_err();
        assert_eq!((error.code, error.exit_code), ("E_UNSUPPORTED_CAPABILITY", 2));
        assert!(error.message.contains("input.wheel"), "{error}");
        for (phase_name, expected) in [
            ("discrete", WheelPhase::Discrete), ("started", WheelPhase::Started),
            ("moved", WheelPhase::Moved), ("ended", WheelPhase::Ended), ("momentum", WheelPhase::Momentum),
        ] {
            let positioned = json!({"id":"spin","op":"wheel","surface":"main","position":{"x":0,"y":60},
                "delta":{"x":0,"y":60,"unit":"pixels"},"phase":phase_name});
            let scenario = check(&declaration(json!([positioned])), &["input.wheel"], false).unwrap();
            assert_eq!(scenario.required_capabilities, ["input.wheel"]);
            let Step::Wheel { location, delta, phase, .. } = &scenario.steps[0] else { panic!("expected wheel") };
            assert!(matches!(location, PointerLocation::Position { surface, x, y } if surface == "main" && *x == 0.0 && *y == 60.0));
            assert_eq!((delta.unit, *phase), (WheelUnit::Pixels, expected));
        }
        let zero = json!({"id":"spin","op":"wheel","surface":"main","position":{"x":10,"y":10},
            "delta":{"x":0,"y":0,"unit":"pixels"}});
        assert!(check(&declaration(json!([zero])), &["input.wheel"], false).is_ok(), "zero deltas are valid gesture lifecycle events");
        for invalid_step in [
            json!({"id":"w","op":"wheel","delta":{"x":0,"y":1,"unit":"pixels"}}),
            json!({"id":"w","op":"wheel","target":{"surface":"main","testId":"a"},"position":{"x":0,"y":0},"delta":{"x":0,"y":1,"unit":"pixels"}}),
            json!({"id":"w","op":"wheel","target":null,"delta":{"x":0,"y":1,"unit":"pixels"}}),
            json!({"id":"w","op":"wheel","surface":"main","position":null,"delta":{"x":0,"y":1,"unit":"pixels"}}),
            json!({"id":"w","op":"wheel","surface":"main","position":{"x":0,"y":0},"delta":{"x":0,"y":1,"unit":"pixels"},"phase":null}),
            json!({"id":"w","op":"wheel","target":{"surface":"main","testId":"a"},"delta":null}),
            json!({"id":"w","op":"wheel","target":{"surface":"main","testId":"a"},"delta":[0,1,"pixels"]}),
            json!({"id":"w","op":"wheel","target":["main","a"],"delta":{"x":0,"y":1,"unit":"pixels"}}),
            json!({"id":"w","op":"wheel","surface":"main","position":[0,0],"delta":{"x":0,"y":1,"unit":"pixels"}}),
            json!({"id":"w","op":"wheel","target":{"surface":"main","testId":"a"}}),
            json!({"id":"w","op":"wheel","surface":"main","position":{"x":0,"y":0},"delta":{"x":0,"y":1}}),
            json!({"id":"w","op":"wheel","surface":"main","position":{"x":0,"y":0},"delta":{"y":1,"unit":"pixels"}}),
            json!({"id":"w","op":"wheel","surface":"main","position":{"x":0,"y":0},"delta":{"x":0,"unit":"pixels"}}),
            json!({"id":"w","op":"wheel","surface":"main","position":{"x":0,"y":0},"delta":{"x":0,"y":1,"unit":"cells"}}),
            json!({"id":"w","op":"wheel","surface":"main","position":{"x":0,"y":0},"delta":{"x":0,"y":1,"unit":"pixels"},"phase":"spin"}),
            json!({"id":"w","op":"wheel","surface":"main","position":{"x":0,"y":0},"delta":{"x":0,"y":1,"unit":"pixels"},"extra":true}),
            json!({"id":"w","op":"wheel","surface":"main","position":{"x":0,"y":0},"delta":{"x":0,"y":1,"unit":"pixels","z":0}}),
            json!({"id":"w","op":"wheel","surface":"main","position":{"x":0,"y":0},"delta":{"x":"0","y":1,"unit":"pixels"}}),
            json!({"id":"w","op":"wheel","surface":"main","position":{"x":0,"y":0},"delta":{"x":0,"y":"1","unit":"pixels"}}),
            json!({"id":"w","op":"wheel","surface":"main","position":{"x":1e100,"y":0},"delta":{"x":0,"y":1,"unit":"pixels"}}),
            json!({"id":"w","op":"wheel","surface":"main","position":{"x":0,"y":0},"delta":{"x":0,"y":1e100,"unit":"pixels"}}),
        ] { schema_error(&declaration(json!([invalid_step]))); }
        let duplicate = br#"{"schemaVersion":1,"id":"test","requiredCapabilities":[],"steps":[{"id":"w","op":"wheel","surface":"main","position":{"x":0,"y":0},"delta":{"x":0,"x":1,"y":0,"unit":"pixels"}}]}"#;
        assert_eq!(parse(duplicate, &["input.wheel"], false).unwrap_err().code, "E_SCHEMA");
        for popup in [
            json!({"id":"w","op":"wheel","surface":"popup","position":{"x":0,"y":0},"delta":{"x":0,"y":1,"unit":"pixels"}}),
            json!({"id":"w","op":"wheel","target":{"surface":"popup","testId":"a"},"delta":{"x":0,"y":1,"unit":"pixels"}}),
        ] {
            let error = check(&declaration(json!([popup])), &["input.wheel", "selectors.test-id"], false).unwrap_err();
            assert_eq!((error.code, error.exit_code), ("E_UNSUPPORTED_CAPABILITY", 2));
        }
    }

    #[test]
    fn pointer_and_selector_forms_are_closed_and_mutually_exclusive() {
        for op in ["pointer.move", "pointer.click", "pointer.down", "pointer.up", "pointer.cancel"] {
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
                if op != "pointer.move" && op != "pointer.cancel" { step["button"] = json!("primary"); }
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
    fn named_surface_actions_require_multi_surface_support() {
        let other = ["input.pointer", "input.keyboard", "input.text", "selectors.test-id", "selectors.semantic", "tree.inspect"];
        for step in [
            json!({"id":"action","op":"pointer.move","surface":"popup","position":{"x":0,"y":0}}),
            json!({"id":"action","op":"pointer.click","target":{"surface":"popup","testId":"button"},"button":"middle"}),
            json!({"id":"action","op":"key.press","surface":"popup","key":"Tab"}),
            json!({"id":"action","op":"text.commit","surface":"popup","text":""}),
            json!({"id":"action","op":"assert","target":{"surface":"popup","role":"button","name":"Go"},"check":{"property":"exists","equals":true}}),
            json!({"id":"action","op":"popupDone","surface":"popup"}),
        ] {
            let input = declaration(json!([step]));
            let error = check(&input, &other, false).expect_err("named input requires multiple surfaces");
            assert_eq!((error.code, error.exit_code), ("E_UNSUPPORTED_CAPABILITY", 2), "step {step}");
            assert!(error.message.contains("surface.multiple"), "step {step}: {error:?}");
            let mut available = other.to_vec();
            available.push("surface.multiple");
            assert!(check(&input, &available, false).is_ok(), "named input is supported: {step}");
        }
    }

    #[test]
    fn service_call_rejects_positional_arrays_and_duplicate_fields() {
        schema_error(&declaration(json!([{"id":"x","op":"assert","serviceCall":["assets","readTextAsset",0]}])));
        let bytes=br#"{"schemaVersion":1,"id":"test","requiredCapabilities":[],"steps":[{"id":"x","op":"assert","serviceCall":{"service":"assets","service":"assets","method":"readTextAsset","count":0}}]}"#;
        assert_eq!(parse(bytes,&["services.fixtures"],false).unwrap_err().code,"E_SCHEMA");
    }

    #[test]
    fn timeline_requires_virtual_time_and_fixture_assertions_are_exclusive() {
        let advance=declaration(json!([{"id":"advance","op":"clock.advance","byMs":0}]));
        assert_eq!(check(&advance,&["virtual-time"],false).unwrap_err().code,"E_UNSUPPORTED_CAPABILITY");
        assert!(check(&advance,&["virtual-time"],true).is_ok());
        let service=json!({"service":"assets","method":"readTextAsset","count":1,"errorCode":"Io"});
        let input=declaration(json!([{"id":"verify","op":"assert","serviceCall":service}]));
        assert_eq!(check(&input,&[],false).unwrap_err().code,"E_UNSUPPORTED_CAPABILITY");
        let parsed=check(&input,&["services.fixtures"],false).unwrap();
        assert!(matches!(&parsed.steps[0],Step::AssertService{check,..} if check.error_code.as_deref()==Some("Io")));
        for extra in [
            json!({"target":{"surface":"main","testId":"a"}}),
            json!({"check":{"property":"exists","equals":true}}),
            json!({"target":null}),json!({"check":null}),
        ] {
            let mut input=input.clone();
            input["steps"][0].as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            schema_error(&input);
        }
    }
    #[test]
    fn recreation_is_closed_main_only_and_infers_its_capability() {
        let step=json!({"id":"resume","op":"surface.recreate","surface":"main"});
        let input=declaration(json!([step]));
        let scenario=check(&input,&["surface.recreate"],false).unwrap();
        assert_eq!(scenario.required_capabilities,["surface.recreate"]);
        assert!(matches!(&scenario.steps[0],Step::SurfaceRecreate{surface,..} if surface=="main"));
        assert_eq!(check(&input,&[],false).unwrap_err().code,"E_UNSUPPORTED_CAPABILITY");
        for invalid in [
            json!({"id":"resume","op":"surface.recreate"}),
            json!({"id":"resume","op":"surface.recreate","surface":null}),
            json!({"id":"resume","op":"surface.recreate","surface":"main","generation":2}),
        ] { schema_error(&declaration(json!([invalid]))); }
        let invalid=declaration(json!([{"id":"resume","op":"surface.recreate","surface":"popup"}]));
        assert_eq!(check(&invalid,&["surface.recreate"],false).unwrap_err().code,"E_UNSUPPORTED_CAPABILITY");
    }

    #[test]
    fn timeline_surface_and_pending_resource_payloads_reject_ambiguous_values() {
        for step in [
            json!({"id":"x","op":"clock.advance","byMs":null}),
            json!({"id":"x","op":"clock.advance","byMs":-1}),
            json!({"id":"x","op":"clock.advance","byMs":1.0}),
            json!({"id":"x","op":"clock.advance","byMs":0,"sleep":1}),
            json!({"id":"x","op":"service.emit","channel":"assets.state","value":null}),
            json!({"id":"x","op":"service.emit","channel":"assets.state","value":[]}),
            json!({"id":"x","op":"surface.resize","surface":"main","width":0,"height":1}),
            json!({"id":"x","op":"surface.resize","surface":"main","width":1.0,"height":1}),
            json!({"id":"x","op":"surface.scale","surface":"main","scale":0}),
            json!({"id":"x","op":"surface.scale","surface":"main","scale":9}),
            json!({"id":"x","op":"capture","captureId":"x","allowedPendingAssets":null}),
            json!({"id":"x","op":"capture","captureId":"x","allowedPendingAssets":["image-1","image-1"]}),
            json!({"id":"x","op":"assert","serviceCall":{"service":"assets","method":"readTextAsset","count":1,"errorCode":null}}),
        ] { schema_error(&declaration(json!([step]))); }
        for step in [
            json!({"id":"x","op":"surface.resize","surface":"popup","width":10,"height":10}),
            json!({"id":"x","op":"surface.scale","surface":"popup","scale":1.5}),
        ] { assert_eq!(check(&declaration(json!([step])),&[],false).unwrap_err().code,"E_UNSUPPORTED_CAPABILITY"); }
    }

    struct InputFile(std::path::PathBuf);

    impl InputFile {
        fn new(bytes: &[u8]) -> Self {
            use std::io::Write;
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!("zgui-preview-scenario-{}-{}.json", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
            file.write_all(bytes).unwrap();
            Self(path)
        }
    }

    impl Drop for InputFile {
        fn drop(&mut self) { std::fs::remove_file(&self.0).unwrap(); }
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
    #[test]
    fn pointer_phases_preserve_identity_metadata_and_capabilities() {
        let scenario = check(&declaration(json!([
            {"id":"down","op":"pointer.down","target":{"surface":"main","testId":"slider"},"button":"primary",
             "pointer":{"id":"7","kind":"pen"}},
            {"id":"move","op":"pointer.move","surface":"main","position":{"x":-20,"y":40},
             "pointer":{"id":"7","kind":"pen","primary":true}},
            {"id":"up","op":"pointer.up","surface":"main","position":{"x":10,"y":10},"button":"secondary",
             "pointer":{"id":"9","kind":"touch","primary":false}},
            {"id":"cancel","op":"pointer.cancel","surface":"main","position":{"x":0,"y":0},
             "pointer":{"id":"7","kind":"pen"}},
            {"id":"click","op":"pointer.click","surface":"main","position":{"x":5,"y":5},"button":"primary"},
            {"id":"edge","op":"pointer.down","surface":"main","position":{"x":1,"y":1},"button":"primary",
             "pointer":{"id":"18446744073709551615","kind":"unknown","primary":false}},
            {"id":"page-up","op":"key.press","surface":"main","key":"PageUp"},
            {"id":"page-down","op":"key.press","surface":"main","key":"PageDown"}
        ])), &["input.pointer", "input.keyboard", "selectors.test-id"], false).unwrap();
        assert_eq!(scenario.steps.iter().map(Step::op).collect::<Vec<_>>(), [
            "pointer.down", "pointer.move", "pointer.up", "pointer.cancel",
            "pointer.click", "pointer.down", "key.press", "key.press"]);
        let Step::PointerDown { pointer, button, location, .. } = &scenario.steps[0] else { panic!("expected down") };
        assert_eq!((pointer.id, pointer.kind, pointer.primary, *button), (7, PointerKind::Pen, true, PointerButton::Primary));
        assert!(matches!(location, PointerLocation::Target(_)));
        assert_eq!(serde_json::to_value(pointer).unwrap(), json!({"id":"7","kind":"pen","primary":true}));
        let Step::PointerMove { pointer, .. } = &scenario.steps[1] else { panic!("expected move") };
        assert_eq!((pointer.id, pointer.kind, pointer.primary), (7, PointerKind::Pen, true));
        let Step::PointerUp { pointer, button, .. } = &scenario.steps[2] else { panic!("expected up") };
        assert_eq!((pointer.id, pointer.kind, pointer.primary, *button), (9, PointerKind::Touch, false, PointerButton::Secondary));
        let Step::PointerCancel { pointer, .. } = &scenario.steps[3] else { panic!("expected cancel") };
        assert_eq!(pointer.id, 7);
        let Step::PointerClick { pointer, .. } = &scenario.steps[4] else { panic!("expected click") };
        assert_eq!((pointer.id, pointer.kind, pointer.primary), (0, PointerKind::Mouse, true),
            "a missing pointer object is the primary mouse");
        let Step::PointerDown { pointer, .. } = &scenario.steps[5] else { panic!("expected max-id down") };
        assert_eq!(pointer.id, u64::MAX);
        assert_eq!(serde_json::to_value(pointer).unwrap(), json!({"id":"18446744073709551615","kind":"unknown","primary":false}),
            "u64::MAX keeps full precision as a decimal string");
        assert!(matches!(&scenario.steps[6], Step::KeyPress { key: Key::PageUp, .. }));
        assert!(matches!(&scenario.steps[7], Step::KeyPress { key: Key::PageDown, .. }));
        assert_eq!(scenario.required_capabilities, ["input.keyboard", "input.pointer", "selectors.test-id"]);
    }

    #[test]
    fn pointer_device_objects_reject_malformed_ids_and_fields() {
        for device in [
            json!(7),
            json!({"id":7,"kind":"mouse"}),
            json!({"id":"","kind":"mouse"}),
            json!({"id":"00","kind":"mouse"}),
            json!({"id":"+1","kind":"mouse"}),
            json!({"id":"-1","kind":"mouse"}),
            json!({"id":"18446744073709551616","kind":"mouse"}),
            json!({"id":"x","kind":"mouse"}),
            json!({"id":null,"kind":"mouse"}),
            json!({"id":"1"}),
            json!({"kind":"mouse"}),
            json!({"id":"1","kind":"trackball"}),
            json!({"id":"1","kind":null}),
            json!({"id":"1","kind":"mouse","primary":"yes"}),
            json!({"id":"1","kind":"mouse","primary":null}),
            json!({"id":"1","kind":"mouse","extra":true}),
            json!(null),
        ] {
            schema_error(&declaration(json!([
                {"id":"action","op":"pointer.down","surface":"main","position":{"x":0,"y":0},
                 "button":"primary","pointer":device}
            ])));
        }
        for step in [
            json!({"id":"a","op":"pointer.down","surface":"main","position":{"x":0,"y":0}}),
            json!({"id":"a","op":"pointer.up","surface":"main","position":{"x":0,"y":0}}),
            json!({"id":"a","op":"pointer.move","surface":"main","position":{"x":0,"y":0},"button":"primary"}),
            json!({"id":"a","op":"pointer.cancel","surface":"main","position":{"x":0,"y":0},"button":"primary"}),
        ] { schema_error(&declaration(json!([step]))); }
        for op in ["pointer.down", "pointer.up", "pointer.cancel"] {
            for fields in [
                json!({}),
                json!({"target":{"surface":"main","testId":"x"},"surface":"main"}),
                json!({"target":{"surface":"main","testId":"x"},"position":{"x":0,"y":0}}),
                json!({"surface":"main","position":{"x":"0","y":0}}),
                json!({"target":{"surface":"main","role":"button"}}),
            ] {
                let mut step = fields;
                step["id"] = json!("action");
                step["op"] = json!(op);
                if op != "pointer.cancel" { step["button"] = json!("primary"); }
                schema_error(&declaration(json!([step])));
            }
        }
        let late = declaration(json!([
            {"id":"fine","op":"pointer.move","surface":"main","position":{"x":0,"y":0}},
            {"id":"bad","op":"pointer.down","surface":"main","position":{"x":0,"y":0},
             "button":"primary","pointer":{"id":"nope","kind":"pen"}}
        ]));
        let error = check(&late, &["input.pointer"], false).unwrap_err();
        assert_eq!((error.code, error.exit_code), ("E_SCHEMA", 2), "{error}");
    }

    #[test]
    fn pointer_phases_infer_pointer_and_selector_requirements() {
        for (step, required) in [
            (json!({"id":"a","op":"pointer.down","surface":"main","position":{"x":0,"y":0},"button":"primary"}), vec!["input.pointer"]),
            (json!({"id":"a","op":"pointer.up","target":{"surface":"main","testId":"t"},"button":"middle"}), vec!["input.pointer", "selectors.test-id"]),
            (json!({"id":"a","op":"pointer.cancel","target":{"surface":"main","role":"button","name":"Go"}}), vec!["input.pointer", "selectors.semantic"]),
            (json!({"id":"a","op":"pointer.move","surface":"main","position":{"x":0,"y":0},"pointer":{"id":"7","kind":"pen"}}), vec!["input.pointer"]),
        ] {
            let input = declaration(json!([step]));
            let scenario = check(&input, &required, false).unwrap();
            assert_eq!(scenario.required_capabilities, required, "{input}");
            let error = check(&input, &required[..required.len() - 1], false).unwrap_err();
            assert_eq!((error.code, error.exit_code), ("E_UNSUPPORTED_CAPABILITY", 2), "{input}: {error}");
        }
        let drag = declaration(json!([{"id":"future","op":"pointer.drag","target":{"surface":"main","testId":"x"}}]));
        let error = check(&drag, &["pixels", "virtual-time", "input.pointer"], true).unwrap_err();
        assert_eq!((error.code, error.exit_code), ("E_UNSUPPORTED_CAPABILITY", 2));
    }

    #[test]
    fn character_keys_preserve_unicode_and_named_key_identity() {
        for text in ["é", "e\u{301}", "𐐷", "Enter", " "] {
            let input = declaration(json!([
                {"id":"char","op":"key.press","surface":"main","key":{"character":text},"modifiers":["Shift","Control"]},
                {"id":"named","op":"key.press","surface":"main","key":"Enter"}
            ]));
            let scenario = check(&input, &["input.keyboard"], false).unwrap();
            assert_eq!(scenario.required_capabilities, ["input.keyboard"]);
            let Step::KeyPress { key, modifiers, .. } = &scenario.steps[0] else { panic!("expected character press") };
            assert!(matches!(key, Key::Character(value) if value == text));
            assert_eq!(modifiers, &[Modifier::Shift, Modifier::Control]);
            assert_eq!(serde_json::to_value(key).unwrap(), json!({"character":text}));
            let Step::KeyPress { key, .. } = &scenario.steps[1] else { panic!("expected named press") };
            assert_eq!(serde_json::to_value(key).unwrap(), json!("Enter"));
            assert_eq!(check(&input, &[], false).unwrap_err().code, "E_UNSUPPORTED_CAPABILITY");
        }
    }

    #[test]
    fn character_key_payloads_are_closed_and_nonempty() {
        for key in [json!({"character":""}),json!({"character":null}),json!({"character":42}),
            json!({"character":[]}),json!({"Character":"a"}),json!({"text":"a"}),
            json!({"character":"a","extra":true}),json!("character"),json!("A")] {
            schema_error(&declaration(json!([{"id":"key","op":"key.press","surface":"main","key":key}])));
        }
        let bytes = br#"{"schemaVersion":1,"id":"test","requiredCapabilities":[],"steps":[{"id":"key","op":"key.press","surface":"main","key":{"character":"a","character":"b"}}]}"#;
        assert_eq!(parse(bytes, &["input.keyboard"], false).unwrap_err().code, "E_SCHEMA");
    }
}
