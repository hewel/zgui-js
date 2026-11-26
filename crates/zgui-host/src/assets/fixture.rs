//! Deterministic, owner-thread service fixtures. This module never opens a file.
use super::ContractFailure;
use crate::{bounded_fs::relative_path, environment::Environment, package::{Capabilities, Package}, runtime_capabilities::ResolvedCapabilities, runtime_paths::{self, Root}};
use host_services::{Completion, ServiceError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{cell::{OnceCell, RefCell}, collections::{BTreeMap, VecDeque}, marker::PhantomData, path::Path, rc::Rc, sync::Arc};
use crate::schema::{deserialize_object,object_struct};

mod external;
mod network;
pub(crate) use external::ExternalDelivery;
pub(crate) use network::NetworkDelivery;
use external::{CommandScript, ControlMessage, ControlProvider, ExternalJob, ExternalState, SocketScript};
use network::{NetworkFixture, NetworkJob, NetworkState};

const MAX_TRACE_RECORDS: usize = 4096;
const MAX_TRACE_BYTES: usize = 1024 * 1024;

struct ByteCount(usize);
impl std::io::Write for ByteCount {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> { self.0 += bytes.len(); Ok(bytes.len()) }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}


object_struct! {
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(crate) struct FixtureDefinition {
        pub schema_version: u32,
        pub id: String,
        pub services: Services,
        pub events: Vec<FixtureEvent>,
        /// Synthetic path bindings; never read from the preview host.
        #[serde(default, deserialize_with = "present_provider", skip_serializing_if = "Option::is_none")]
        runtime_environment: Option<RuntimeEnvironment>,
    }
}
impl Default for FixtureDefinition {
    fn default() -> Self { Self { schema_version: 1, id: "empty".into(), services: Services::default(), events: vec![], runtime_environment: None } }
}

object_struct! {
    /// Lexical runtime values used instead of host environment and directories.
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct RuntimeEnvironment {
        #[serde(default, deserialize_with = "present_provider", skip_serializing_if = "Option::is_none")]
        xdg_runtime_dir: Option<String>,
        #[serde(default, deserialize_with = "present_provider", skip_serializing_if = "Option::is_none")]
        home_dir: Option<String>,
        #[serde(default, deserialize_with = "present_provider", skip_serializing_if = "Option::is_none")]
        xdg_data_home: Option<String>,
        #[serde(default, deserialize_with = "present_provider", skip_serializing_if = "Option::is_none")]
        xdg_documents_dir: Option<String>,
        #[serde(default, deserialize_with = "present_provider", skip_serializing_if = "Option::is_none")]
        wayland_display: Option<String>,
        #[serde(default, deserialize_with = "unique_entries", skip_serializing_if = "BTreeMap::is_empty")]
        named_sockets: BTreeMap<String, String>,
    }
}

impl RuntimeEnvironment {
    fn validate(&self) -> Result<(), String> {
        for (field, path) in [
            ("xdgRuntimeDir", &self.xdg_runtime_dir),
            ("homeDir", &self.home_dir),
            ("xdgDataHome", &self.xdg_data_home),
            ("xdgDocumentsDir", &self.xdg_documents_dir),
        ] {
            if path.as_deref().is_some_and(|path| !runtime_paths::canonical_absolute(path)) {
                return Err(format!("fixture runtimeEnvironment.{field} must be canonical absolute text"));
            }
        }
        if self.wayland_display.as_deref().is_some_and(|display| !runtime_paths::safe_segment(display)) {
            return Err("fixture runtimeEnvironment.waylandDisplay must be a single safe filename component".into());
        }
        for (name, path) in &self.named_sockets {
            if !runtime_paths::socket_name(name) || !runtime_paths::canonical_absolute(path) {
                return Err(format!("fixture runtimeEnvironment.namedSockets has an invalid name or path for {name:?}"));
            }
        }
        Ok(())
    }

    fn root(&self, root: Root) -> Option<String> {
        match root {
            Root::Runtime => self.xdg_runtime_dir.clone(),
            Root::Home => self.home_dir.clone(),
            Root::Data => self.xdg_data_home.clone().or_else(|| self.home_dir.as_deref().map(runtime_paths::default_data_home)),
        }
    }
}

object_struct! {
    #[derive(Default)]
    #[serde(deny_unknown_fields)]
    pub(crate) struct Services {
        #[serde(default, deserialize_with = "present_provider", skip_serializing_if = "Option::is_none")]
        pub assets: Option<TextProvider>,
        #[serde(default, deserialize_with = "present_provider", skip_serializing_if = "Option::is_none")]
        pub images: Option<ImageProvider>,
        #[serde(default, deserialize_with = "present_provider", skip_serializing_if = "Option::is_none")]
        sockets: Option<Vec<SocketScript>>,
        #[serde(default, deserialize_with = "present_provider", skip_serializing_if = "Option::is_none")]
        commands: Option<Vec<CommandScript>>,
        #[serde(default, deserialize_with = "present_provider", skip_serializing_if = "Option::is_none")]
        control: Option<ControlProvider>,
        /// Optional one-shot response from the fixture compositor.
        #[serde(rename = "sessionLock", default, deserialize_with = "present_provider", skip_serializing_if = "Option::is_none")]
        pub(crate) session_lock: Option<SessionLockFixture>,
        /// Optional initial clipboard content seeded into the preview MemoryClipboard.
        #[serde(default, deserialize_with = "present_provider", skip_serializing_if = "Option::is_none")]
        pub(crate) clipboard: Option<ClipboardFixture>,
        /// Optional SQLite in-memory database seed statements keyed by database name.
        #[serde(default, deserialize_with = "present_provider", skip_serializing_if = "Option::is_none")]
        pub(crate) sqlite: Option<BTreeMap<String, SqliteFixture>>,
        /// Optional in-memory virtual filesystem, scripted dialogs, and seeded bookmarks.
        #[serde(default, deserialize_with = "present_provider", skip_serializing_if = "Option::is_none")]
        pub(crate) files: Option<FilesFixture>,
        /// Optional deterministic network scripts (`http`, `sse`, `websockets`).
        #[serde(default, deserialize_with = "present_provider", skip_serializing_if = "Option::is_none")]
        pub(crate) network: Option<NetworkFixture>,
    }
}

object_struct! {
    /// Scripted file/directory picker response for preview.
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(crate) struct ScriptedDialogFixture {
        pub(crate) action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub(crate) path: Option<String>,
    }
}

object_struct! {
    /// Seeded persistent bookmark entry for preview.
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(crate) struct BookmarkFixture {
        pub(crate) token: String,
        pub(crate) kind: host_services::files::HandleKind,
        pub(crate) path: String,
        pub(crate) mode: host_services::files::FileMode,
    }
}

object_struct! {
    /// In-memory virtual filesystem and dialog configuration for preview.
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(crate) struct FilesFixture {
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        pub(crate) files: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub(crate) dirs: Vec<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        pub(crate) symlinks: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub(crate) dialogs: Vec<ScriptedDialogFixture>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub(crate) bookmarks: Vec<BookmarkFixture>,
    }
}

impl FilesFixture {
    fn validate(&self) -> Result<(), String> {
        for path in self.files.keys().chain(self.dirs.iter()).chain(self.symlinks.keys()) {
            if !runtime_paths::canonical_absolute(path) {
                return Err(format!("fixture services.files path must be canonical absolute: {path:?}"));
            }
        }
        for dialog in &self.dialogs {
            if !matches!(dialog.action.as_str(), "openFile" | "saveFile" | "openDirectory") {
                return Err(format!("invalid fixture dialog action: {:?}", dialog.action));
            }
            if let Some(p) = &dialog.path && !runtime_paths::canonical_absolute(p) {
                return Err(format!("fixture dialog path must be canonical absolute: {p:?}"));
            }
        }
        for bookmark in &self.bookmarks {
            if bookmark.token.is_empty() || !runtime_paths::canonical_absolute(&bookmark.path) {
                return Err("fixture bookmark has invalid token or path".into());
            }
        }
        Ok(())
    }
}

object_struct! {
    /// Seed SQL statements executed when opening a preview in-memory SQLite database.
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(crate) struct SqliteFixture {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub(crate) statements: Vec<String>,
    }
}

impl SqliteFixture {
    fn validate(&self) -> Result<(), String> {
        if self.statements.len() > 1000 {
            return Err("fixture sqlite seed exceeds 1000 statements".into());
        }
        for sql in &self.statements {
            if sql.len() > host_services::sqlite::MAX_SQL_BYTES {
                return Err("fixture sqlite seed statement exceeds 1 MiB".into());
            }
        }
        Ok(())
    }
}

object_struct! {
    /// Initial clipboard payload seeded into preview MemoryClipboard.
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(crate) struct ClipboardFixture {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub(crate) text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub(crate) image_base64: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pub(crate) secret: bool,
    }
}

impl ClipboardFixture {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.text.is_some() && self.image_base64.is_some() {
            return Err("fixture services.clipboard cannot set both text and imageBase64".into());
        }
        if let Some(encoded) = &self.image_base64 {
            let bytes = decode_base64(encoded)?;
            if bytes.is_empty() {
                return Err("fixture services.clipboard.imageBase64 must not be empty".into());
            }
        }
        Ok(())
    }

    pub(crate) fn image_bytes(&self) -> Result<Option<Vec<u8>>, String> {
        self.image_base64.as_deref().map(decode_base64).transpose()
    }
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let cleaned: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    if !cleaned.len().is_multiple_of(4) {
        return Err("invalid base64 length in fixture".into());
    }
    let mut out = Vec::with_capacity((cleaned.len() / 4) * 3);
    let val = |b: u8| -> Result<u8, String> {
        match b {
            b'A'..=b'Z' => Ok(b - b'A'),
            b'a'..=b'z' => Ok(b - b'a' + 26),
            b'0'..=b'9' => Ok(b - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err(format!("invalid base64 byte: 0x{b:02x}")),
        }
    };
    let (chunks, _) = cleaned.as_chunks::<4>();
    for (idx, chunk) in chunks.iter().enumerate() {
        let is_last = (idx + 1) * 4 == cleaned.len();
        let a = val(chunk[0])?;
        let b = val(chunk[1])?;
        if chunk[2] == b'=' {
            if !is_last || chunk[3] != b'=' {
                return Err("invalid base64 padding in fixture".into());
            }
            out.push((a << 2) | (b >> 4));
        } else if chunk[3] == b'=' {
            if !is_last {
                return Err("invalid base64 padding in fixture".into());
            }
            let c = val(chunk[2])?;
            out.push((a << 2) | (b >> 4));
            out.push((b << 4) | (c >> 2));
        } else {
            let c = val(chunk[2])?;
            let d = val(chunk[3])?;
            out.push((a << 2) | (b >> 4));
            out.push((b << 4) | (c >> 2));
            out.push((c << 6) | d);
        }
    }
    Ok(out)
}

object_struct! {
    /// Scripted response to the next session-lock request.
    #[serde(deny_unknown_fields)]
    pub(crate) struct SessionLockFixture {
        /// Acquisition outcome reported to the application.
        pub(crate) outcome: LockOutcome,
    }
}

/// The fixture compositor's acquisition decision.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum LockOutcome {
    /// Confirm the lock.
    Locked,
    /// Refuse before confirming the lock.
    Finished,
}
fn present_provider<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<Option<T>, D::Error> { T::deserialize(deserializer).map(Some) }
object_struct! {
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(crate) struct TextProvider {
        pub provider: TextProviderVersion,
        pub latency_ms: u32,
        pub initial_state: AssetState,
    }
}
object_struct! {
    #[serde(deny_unknown_fields)]
    pub(crate) struct ImageProvider {
        #[serde(default, deserialize_with = "unique_entries", skip_serializing_if = "BTreeMap::is_empty")]
        files: BTreeMap<String, String>,
        #[serde(default, deserialize_with = "unique_entries", skip_serializing_if = "BTreeMap::is_empty")]
        blobs: BTreeMap<String, String>,
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) enum TextProviderVersion { #[serde(rename = "mock-text-assets-v1")] V1 }
object_struct! {
    #[serde(deny_unknown_fields)]
    pub(crate) struct AssetState {
        #[serde(deserialize_with = "unique_entries")]
        pub assets: BTreeMap<String, AssetBehavior>,
    }
}
fn unique_entries<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error> {
    struct Entries<T>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Entries<T> {
        type Value = BTreeMap<String, T>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("a fixture object with unique keys") }
        fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
            let mut entries = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, T>()? {
                if entries.insert(key, value).is_some() { return Err(serde::de::Error::custom("duplicate fixture object key")); }
            }
            Ok(entries)
        }
    }
    deserializer.deserialize_map(Entries(PhantomData))
}
object_struct! {
    #[serde(deny_unknown_fields)]
    pub(crate) struct AssetBehavior { pub error: Option<InjectedError> }
}
object_struct! {
    #[serde(deny_unknown_fields)]
    pub(crate) struct InjectedError { pub code: ErrorCode, pub message: String }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) enum ErrorCode { Io, TooLarge, InvalidUtf8 }
impl ErrorCode { fn as_str(self) -> &'static str { match self { Self::Io => "Io", Self::TooLarge => "TooLarge", Self::InvalidUtf8 => "InvalidUtf8" } } }
impl InjectedError {
    fn service_error(&self) -> ServiceError { ServiceError { code: self.code.as_str(), message: self.message.clone() } }
    fn validate(&self) -> Result<(), String> {
        if self.message.len() > 4096 { return Err("injected error message exceeds 4096 bytes".into()); }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FixtureEvent {
    pub at_ms: u32,
    #[serde(flatten)]
    pub update: StateEvent,
}
impl<'de> Deserialize<'de> for FixtureEvent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "channel", deny_unknown_fields)]
        enum Input {
            #[serde(rename = "assets.state")]
            Assets { #[serde(rename = "atMs")] at_ms: u32, value: AssetState },
            #[serde(rename = "control.message")]
            Control { #[serde(rename = "atMs")] at_ms: u32, value: ControlMessage },
        }
        Ok(match deserialize_object::<D, Input>(deserializer)? {
            Input::Assets { at_ms, value } => Self { at_ms, update: StateEvent::Assets(value) },
            Input::Control { at_ms, value } => Self { at_ms, update: StateEvent::Control(value) },
        })
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "channel", content = "value", deny_unknown_fields)]
pub(crate) enum StateEvent {
    #[serde(rename = "assets.state")] Assets(AssetState),
    #[serde(rename = "control.message")] Control(ControlMessage),
}
impl<'de> Deserialize<'de> for StateEvent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "channel", content = "value", deny_unknown_fields)]
        enum Input {
            #[serde(rename = "assets.state")] Assets(AssetState),
            #[serde(rename = "control.message")] Control(ControlMessage),
        }
        Ok(match deserialize_object::<D, Input>(deserializer)? {
            Input::Assets(value) => Self::Assets(value),
            Input::Control(value) => Self::Control(value),
        })
    }
}

impl AssetState {
    fn validate(&self) -> Result<(), String> {
        if self.assets.len() > 1024 { return Err("fixture exceeds 1024 text assets".into()); }
        for (path, behavior) in &self.assets {
            if path.is_empty() || path.contains('\\') || path.contains('\0') || path.split('/').any(|p| p.is_empty() || p == "." || p == "..") { return Err("fixture asset path must be canonical package-relative text".into()); }
            if let Some(error) = &behavior.error { error.validate()?; }
        }
        Ok(())
    }
}
impl ImageProvider {
    fn validate(&self) -> Result<(), String> {
        if self.files.keys().any(|uri| !uri.starts_with("file:///")) {
            return Err("fixture image file keys must be absolute file:// URIs".into());
        }
        if self.files.values().chain(self.blobs.keys()).chain(self.blobs.values()).any(|path| relative_path(path).is_err()) {
            return Err("fixture image assets must use canonical package-relative paths".into());
        }
        Ok(())
    }
}
impl FixtureDefinition {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 { return Err("unsupported fixture schemaVersion; expected 1".into()); }
        if self.id.trim().is_empty() || self.id.chars().count() > 128 { return Err("fixture id must contain 1..=128 characters of nonblank text".into()); }
        if self.events.len() > 4096 { return Err("fixture exceeds 4096 events".into()); }
        if let Some(runtime) = &self.runtime_environment { runtime.validate()?; }
        if let Some(provider) = &self.services.assets { provider.initial_state.validate()?; }
        if let Some(images) = &self.services.images { images.validate()?; }
        if let Some(clipboard) = &self.services.clipboard { clipboard.validate()?; }
        if let Some(sqlite) = &self.services.sqlite {
            for (name, seed) in sqlite {
                if !host_services::sqlite::is_valid_database_name(name) {
                    return Err(format!("fixture services.sqlite has invalid database name: {name:?}"));
                }
                seed.validate()?;
            }
        }
        if let Some(files) = &self.services.files { files.validate()?; }
        if let Some(network) = &self.services.network { network.validate()?; }
        self.services.validate_external()?;
        for event in &self.events { self.validate_event(&event.update)?; }
        Ok(())
    }
    fn validate_event(&self, event: &StateEvent) -> Result<(), String> {
        match event {
            StateEvent::Assets(state) if self.services.assets.is_some() => state.validate(),
            StateEvent::Control(message) if self.services.control.is_some() => message.validate(),
            _ => Err("fixture event requires its declared provider".into()),
        }
    }
}

enum Scheduled {
    Event(StateEvent),
    Asset { generation: u64, id: u64, result: Result<String, ServiceError>, injected: bool },
    External { generation: u64, job: ExternalJob },
    Network { generation: u64, job: NetworkJob },
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct ScheduleKey {
    deadline_us: u64,
    order: ScheduleOrder,
}

// At equal deadlines, absolute events precede call-relative outcomes.
// Call ID and then script index preserve admission order and script order.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum ScheduleOrder {
    Event { index: usize },
    Call { id: u64, index: usize },
}

struct Call { generation: u64, service: &'static str, method: &'static str, error: Option<&'static str>, error_delivered: bool, acknowledged: bool }
struct State {
    scheduled: BTreeMap<ScheduleKey, Scheduled>,
    deliveries: VecDeque<Completion>,
    assets: Option<AssetState>,
    external: ExternalState,
    network: NetworkState,
    calls: BTreeMap<u64, Call>,
    next_registration_id: u64,
    sequence: u64,
    trace: Vec<Value>, trace_bytes: usize, dropped: u64,
    failure: Option<ContractFailure>,
    disposed: bool,
}
pub(crate) struct FixtureRuntime {
    definition: FixtureDefinition,
    environment: Rc<Environment>,
    state: RefCell<State>,
}
fn failure(code: &'static str, message: impl Into<String>) -> ContractFailure { ContractFailure { code, message: message.into() } }
impl FixtureRuntime {
    pub(crate) fn new(definition: FixtureDefinition, environment: Rc<Environment>) -> Rc<Self> {
        let invalid = definition.validate().err().map(|message| failure("E_FIXTURE_CONTRACT", message));
        let scheduled = definition.events.iter().enumerate().map(|(index, event)| {
            let key = ScheduleKey {
                deadline_us: u64::from(event.at_ms) * 1000,
                order: ScheduleOrder::Event { index },
            };
            (key, Scheduled::Event(event.update.clone()))
        }).collect();
        Rc::new(Self { state: RefCell::new(State {
            scheduled, deliveries: VecDeque::new(), assets: definition.services.assets.as_ref().map(|p| p.initial_state.clone()),
            external: ExternalState::default(),
            network: NetworkState::default(),
            calls: BTreeMap::new(), next_registration_id: 1, sequence: 0, trace: vec![], trace_bytes: 0, dropped: 0, failure: invalid, disposed: false,
        }), definition, environment })
    }

    /// Resolves the package's path grants against the immutable synthetic environment.
    ///
    /// # Errors
    /// Fails for an invalid fixture or a template with a missing binding.
    pub(crate) fn resolve_capabilities(&self, declarations: &Capabilities) -> Result<ResolvedCapabilities, ContractFailure> {
        if let Some(error) = self.contract_failure() { return Err(error); }
        let inputs = self.definition.runtime_environment.as_ref();
        let resolved = ResolvedCapabilities::resolve(
            declarations,
            |root| inputs.and_then(|inputs| inputs.root(root)).ok_or_else(|| format!("fixture does not set {}", root.key())),
            || Ok(inputs.and_then(|inputs| inputs.wayland_display.clone())),
            |name| Ok(inputs.and_then(|inputs| inputs.named_sockets.get(name)).cloned()),
        )?;
        if declarations.files.directories.is_empty() { return Ok(resolved); }
        let home = inputs.and_then(|env| env.home_dir.as_deref()).unwrap_or("/home/preview");
        let data = OnceCell::new();
        let documents = OnceCell::new();
        Ok(resolved.with_file_directories(declarations, |root| match root {
            "$HOME" => Some(Path::new(home)),
            "$XDG_DATA_HOME" => Some(inputs.and_then(|env| env.xdg_data_home.as_deref())
                .map(Path::new).unwrap_or_else(|| data.get_or_init(|| Path::new(home).join(".local/share")))),
            "$XDG_DOCUMENTS_DIR" => Some(inputs.and_then(|env| env.xdg_documents_dir.as_deref())
                .map(Path::new).unwrap_or_else(|| documents.get_or_init(|| Path::new(home).join("Documents")))),
            _ => None,
        }))
    }

    fn record(&self, state: &mut State, kind: &str, detail: Value) {
        state.sequence += 1;
        if state.trace.len() >= MAX_TRACE_RECORDS || state.trace_bytes >= MAX_TRACE_BYTES {
            state.dropped += 1;
            return;
        }
        let record = json!({"sequence":state.sequence,"timeUs":self.environment.now_us(),"kind":kind,"detail":detail});
        let mut bytes = ByteCount(0);
        serde_json::to_writer(&mut bytes, &record).expect("JSON trace");
        if state.trace.len() >= MAX_TRACE_RECORDS || state.trace_bytes + bytes.0 > MAX_TRACE_BYTES { state.dropped += 1; }
        else { state.trace_bytes += bytes.0; state.trace.push(record); }
    }
    fn fail(&self, error: ContractFailure) -> ContractFailure {
        let mut state = self.state.borrow_mut();
        self.record_failure(&mut state, error)
    }
    fn record_failure(&self, state: &mut State, error: ContractFailure) -> ContractFailure {
        state.failure.get_or_insert_with(|| error.clone());
        self.record(state, "contractFailure", json!({"code":error.code,"message":error.message}));
        error
    }
    pub(crate) fn contract_failure(&self) -> Option<ContractFailure> { self.state.borrow().failure.clone() }
    pub(crate) fn worker_now_ms(&self) -> u64 { self.environment.virtual_now_ms() }
    pub(crate) fn next_worker_seed(&self) -> u64 { self.environment.next_seed() }
    /// Retains a capability denial even when application code catches it.
    pub(crate) fn deny_capability(&self, message: &str) -> ContractFailure { self.fail(failure("E_CAPABILITY", message)) }
    pub(super) fn malformed(&self, message: impl Into<String>) -> ContractFailure { self.fail(failure("E_FIXTURE_CONTRACT", message)) }
    pub(super) fn require_assets(&self) -> Result<(), ContractFailure> {
        if self.definition.services.assets.is_none() { return Err(self.fail(failure("E_FIXTURE_MISSING_SERVICE", "preview fixture does not provide readTextAsset"))); }
        Ok(())
    }
    pub(super) fn image_files<'a>(&'a self, package: &'a Package) -> impl Iterator<Item = (String, Arc<[u8]>)> + 'a {
        // Unavailable targets are not entries in this generation's authority.
        self.definition.services.images.iter().flat_map(|images| &images.files).filter_map(|(uri, path)| {
            package.asset_data(path).map(|bytes| (uri.clone(), bytes))
        })
    }
    pub(super) fn image_blob(&self, path: &str, package: Option<&Package>) -> Result<Arc<[u8]>, ContractFailure> {
        if let Some(error) = self.contract_failure() { return Err(error); }
        let Some(package) = package.filter(|package| package.asset_bytes(path).is_some()) else {
            return Err(self.malformed(format!("image asset {path:?} is not declared in the loaded package")));
        };
        let target = self.definition.services.images.as_ref().and_then(|images| images.blobs.get(path))
            .ok_or_else(|| self.malformed(format!("fixture does not map image blob {path:?}")))?;
        package.asset_data(target)
            .ok_or_else(|| self.malformed(format!("fixture image blob target {target:?} is not declared in the loaded package")))
    }
    pub(super) fn registration_id(&self) -> Option<u64> {
        let mut state = self.state.borrow_mut();
        Self::allocate_id(&mut state)
    }
    fn allocate_id(state: &mut State) -> Option<u64> {
        let id = state.next_registration_id;
        if state.disposed || id>super::MAX_SAFE_ID { return None; }
        state.next_registration_id += 1;
        Some(id)
    }
    fn admission(&self, service: &'static str, method: &'static str, request_id: u64, generation: u64) -> Result<(), ContractFailure> {
        let mut state = self.state.borrow_mut();
        self.admit(&mut state, service, method, request_id, generation)
    }
    fn admit(&self, state: &mut State, service: &'static str, method: &'static str, request_id: u64, generation: u64) -> Result<(), ContractFailure> {
        if let Some(error) = &state.failure { return Err(error.clone()); }
        if state.disposed {
            return Err(self.record_failure(state, failure("E_FIXTURE_CONTRACT", "fixture runtime is disposed")));
        }
        // Calls remain bounded independently of the lossy trace: exact assertions cannot rely on trace retention.
        if state.calls.len() >= 16384 {
            return Err(self.record_failure(state, failure("E_FIXTURE_CONTRACT", "fixture call limit reached")));
        }
        if state.calls.contains_key(&request_id) {
            return Err(self.record_failure(state, failure("E_FIXTURE_CONTRACT", "duplicate fixture request ID")));
        }
        state.calls.insert(request_id, Call { generation, service, method, error: None, error_delivered: false, acknowledged: false });
        self.record(state, "request", json!({"requestId":request_id,"service":service,"method":method}));
        Ok(())
    }
    pub(super) fn read_asset(&self, generation: u64, request_id: u64, path: &str, package: Option<&Package>) -> Result<(), ContractFailure> {
        self.admission("assets", "readTextAsset", request_id, generation)?;
        let Some(provider) = &self.definition.services.assets else { return Err(self.fail(failure("E_FIXTURE_MISSING_SERVICE", "preview fixture does not provide readTextAsset"))); };
        let behavior = self.state.borrow().assets.as_ref().and_then(|state| state.assets.get(path)).cloned();
        let Some(behavior) = behavior else { return Err(self.malformed(format!("fixture does not declare text asset {path}"))); };
        let Some(bytes) = package.and_then(|p| p.asset_bytes(path)) else { return Err(self.malformed(format!("asset {path} is not declared in the loaded package"))); };
        let injected = behavior.error.is_some();
        let result = if let Some(error) = behavior.error { Err(error.service_error()) }
            else if bytes.len() > super::MAX_ASSET_BYTES { Err(ServiceError { code: "TooLarge", message: "text asset exceeds 1 MiB".into() }) }
            else { std::str::from_utf8(bytes).map(str::to_owned).map_err(|_| ServiceError { code: "InvalidUtf8", message: "text asset is not UTF-8".into() }) };
        let deadline = self.environment.now_us().checked_add(u64::from(provider.latency_ms) * 1000).ok_or_else(|| self.malformed("fixture deadline overflow"))?;
        let key = ScheduleKey {
            deadline_us: deadline,
            order: ScheduleOrder::Call { id: request_id, index: 0 },
        };
        self.state.borrow_mut().scheduled.insert(key, Scheduled::Asset { generation, id: request_id, result, injected });
        let mut state = self.state.borrow_mut();
        self.record(&mut state, "arguments", json!({"requestId":request_id,"path":path}));
        Ok(())
    }
    pub(crate) fn next_deadline_us(&self) -> Option<u64> { self.state.borrow().scheduled.first_key_value().map(|(key, _)| key.deadline_us) }
    pub(crate) fn release_due(&self) -> Result<bool, ContractFailure> {
        if let Some(error) = self.contract_failure() { return Err(error); }
        let now = self.environment.now_us();
        let mut state = self.state.borrow_mut();
        let mut worked = false;
        while state.scheduled.first_key_value().is_some_and(|(key, _)| key.deadline_us <= now) {
            let Some((_, scheduled)) = state.scheduled.pop_first() else { break };
            worked = true;
            match scheduled {
                Scheduled::Event(event) => self.apply(&mut state, event)?,
                Scheduled::Asset { generation, id, result, injected } => {
                    self.check_delivery_limit(&mut state)?;
                    if injected && let Some(call) = state.calls.get_mut(&id) { call.error = result.as_ref().err().map(|e| e.code); }
                    self.record(&mut state, "response", json!({"requestId":id,"errorCode":result.as_ref().err().map(|e| e.code)}));
                    state.deliveries.push_back(Completion { generation, request_id: id, result });
                }
                Scheduled::External { generation, job } => self.enqueue_external(&mut state, generation, job)?,
                Scheduled::Network { generation, job } => self.enqueue_network(&mut state, generation, job)?,
            }
        }
        if let Some(error) = &state.failure { return Err(error.clone()); }
        Ok(worked)
    }
    fn check_delivery_limit(&self, state: &mut State) -> Result<(), ContractFailure> {
        if state.deliveries.len() + state.external.delivery_count() >= 1024 {
            return Err(self.record_failure(state, failure("E_FIXTURE_CONTRACT", "fixture delivery queue exceeds 1024 entries")));
        }
        Ok(())
    }
    fn apply(&self, state: &mut State, event: StateEvent) -> Result<(), ContractFailure> {
        match event {
            StateEvent::Assets(assets) => {
                self.record(state, "state", json!({"channel":"assets.state","value":&assets}));
                state.assets = Some(assets);
                Ok(())
            }
            StateEvent::Control(message) => self.dispatch_control(state, message),
        }
    }
    pub(crate) fn emit(&self, channel: &str, value: &Value) -> Result<(), ContractFailure> {
        if let Some(error) = self.contract_failure() { return Err(error); }
        let event: StateEvent = serde_json::from_value(json!({"channel":channel,"value":value})).map_err(|e| self.malformed(format!("invalid fixture event: {e}")))?;
        self.definition.validate_event(&event).map_err(|e| self.malformed(e))?;
        let mut state = self.state.borrow_mut();
        if state.disposed { drop(state); return Err(self.malformed("fixture runtime is disposed")); }
        self.apply(&mut state, event)
    }
    pub(super) fn take(&self) -> Option<Completion> { self.state.borrow_mut().deliveries.pop_front() }
    pub(super) fn has_delivery(&self) -> bool { !self.state.borrow().deliveries.is_empty() }
    pub(super) fn delivered(&self, id: u64, error: bool) {
        if error && let Some(call) = self.state.borrow_mut().calls.get_mut(&id) { call.error_delivered = true; }
    }
    pub(super) fn ignored(&self, id: u64, reason: &str) {
        let mut state = self.state.borrow_mut();
        self.record(&mut state, "ignored", json!({"requestId":id,"reason":reason}));
    }
    pub(super) fn cancel(&self, id: u64) {
        let mut state = self.state.borrow_mut();
        if let Some(call) = state.calls.get_mut(&id) && !call.error_delivered { call.error = None; }
        state.scheduled.retain(|_, item| !matches!(item, Scheduled::Asset { id: other, .. } if *other == id));
        state.deliveries.retain(|item| item.request_id != id);
        self.record(&mut state, "cancellation", json!({"requestId":id}));
    }
    /// Retire only one session's registrations; scenario events and the exact call ledger survive.
    pub(super) fn cancel_generation(&self, generation: u64) {
        let ids: Vec<_> = self.state.borrow().calls.iter()
            .filter_map(|(&id, call)|(call.generation==generation && call.service=="assets").then_some(id)).collect();
        for id in ids { self.cancel(id); }
    }
    /// Record the headless compositor's state after retiring all JS ownership.
    pub(crate) fn record_session_lock(&self, snapshot: Value) {
        self.record(&mut self.state.borrow_mut(), "sessionLock", snapshot);
    }

    /// Record the headless MemoryClipboard state at teardown for deterministic assertions.
    pub(crate) fn record_clipboard(&self, snapshot: Value) {
        self.record(&mut self.state.borrow_mut(), "clipboard", snapshot);
    }

    /// Returns optional seed SQL statements for a preview in-memory SQLite database.
    pub(crate) fn sqlite_seed_statements(&self, name: &str) -> Vec<String> {
        self.definition
            .services
            .sqlite
            .as_ref()
            .and_then(|map| map.get(name))
            .map(|seed| seed.statements.clone())
            .unwrap_or_default()
    }

    /// Builds an isolated in-memory virtual filesystem seeded from `services.files`.
    pub(crate) fn build_preview_fs(&self) -> host_services::files::vfs::PreviewFs {
        use host_services::files::{
            grants::BookmarkRecord,
            vfs::{PreviewFs, ScriptedDialog, ScriptedDialogKind},
        };
        let mut vfs = PreviewFs::default();
        if let Some(cfg) = &self.definition.services.files {
            for dir in &cfg.dirs {
                vfs.seed_dir(dir);
            }
            for (path, text) in &cfg.files {
                vfs.seed_file(path, text.as_bytes().to_vec(), 0);
            }
            for (path, target) in &cfg.symlinks {
                vfs.seed_symlink(path, target.clone());
            }
            for dialog in &cfg.dialogs {
                let kind = match dialog.action.as_str() {
                    "openFile" => ScriptedDialogKind::OpenFile,
                    "saveFile" => ScriptedDialogKind::SaveFile,
                    _ => ScriptedDialogKind::OpenDirectory,
                };
                vfs.push_dialog(ScriptedDialog {
                    kind,
                    path: dialog.path.clone(),
                });
            }
            for bm in &cfg.bookmarks {
                let label = bm
                    .path
                    .rsplit('/')
                    .next()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("/")
                    .to_owned();
                vfs.seed_bookmark(BookmarkRecord {
                    token: bm.token.clone(),
                    kind: bm.kind,
                    path: bm.path.clone(),
                    label,
                    mode: bm.mode,
                    created_ms: 0,
                });
            }
        }
        vfs
    }

    pub(crate) fn dispose(&self) {
        let mut state = self.state.borrow_mut();
        if state.disposed { return; }
        let scheduled = state.scheduled.len();
        let deliveries = state.deliveries.len() + state.external.delivery_count() + state.network.delivery_count();
        for call in state.calls.values_mut() {
            if !call.error_delivered { call.error = None; }
        }
        state.disposed = true; state.scheduled.clear(); state.deliveries.clear();
        state.external.dispose();
        state.network.dispose();
        self.record(&mut state, "disposal", json!({"cancelledScheduled":scheduled,"cancelledDeliveries":deliveries}));
    }
    pub(crate) fn trace(&self) -> Value {
        let state = self.state.borrow();
        json!({"schemaVersion":1,"records":state.trace,"droppedRecords":state.dropped,"recordLimit":MAX_TRACE_RECORDS,"byteLimit":MAX_TRACE_BYTES})
    }
    pub(crate) fn assert_calls(&self, service: &str, method: &str, count: u64, error_code: Option<&str>) -> Result<(), ContractFailure> {
        if let Some(error) = self.contract_failure() { return Err(error); }
        if !matches!((service, method),
            ("assets", "readTextAsset") | ("sockets", "connectUnixSocket" | "send")
            | ("commands", "spawnCommand") | ("control", "onControlMessage" | "reply")
            | ("network", "httpRequest" | "openEventStream" | "connectWebSocket"))
        {
            return Err(failure("E_ASSERTION", format!("unknown fixture method {service}.{method}")));
        }
        let mut state = self.state.borrow_mut();
        let matches = || state.calls.values().filter(|call| call.service == service && call.method == method);
        let observed = matches().count();
        if u64::try_from(observed).ok() != Some(count)
            || error_code.is_some_and(|code| !matches().any(|call| call.error == Some(code)))
        {
            return Err(failure("E_ASSERTION", format!("expected {count} {service}.{method} calls with error {error_code:?}; observed {observed}")));
        }
        if let Some(code) = error_code {
            for call in state.calls.values_mut().filter(|call| call.service == service && call.method == method && call.error == Some(code)) {
                call.acknowledged = true;
            }
        }
        Ok(())
    }
    pub(crate) fn finalize(&self) -> Result<(), ContractFailure> {
        let state = self.state.borrow();
        if let Some(error) = &state.failure { return Err(error.clone()); }
        if let Some((call, code)) = state.calls.values().find_map(|call| {
            call.error.filter(|_| !call.acknowledged).map(|code| (call, code))
        }) {
            return Err(failure("E_FIXTURE_UNEXPECTED_ERROR", format!("unacknowledged injected {}.{} error {code}", call.service, call.method)));
        }
        let external = self.finalize_external(&state);
        drop(state);
        external.map_err(|error| self.fail(error))
    }
}

#[cfg(test)]
mod tests;
