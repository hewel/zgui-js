//! Deterministic, owner-thread providers. This module never opens a file or compositor socket.
use super::ContractFailure;
use crate::{package::Package, preview::clock::Environment};
use host_services::{ServiceError, niri::{NiriSnapshot, NiriWindow, NiriWorkspace}};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{cell::RefCell, collections::{BTreeMap, BTreeSet, VecDeque}, rc::Rc, sync::Arc};
use crate::preview::schema::{deserialize_object,object_struct};

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
    }
}
impl Default for FixtureDefinition {
    fn default() -> Self { Self { schema_version: 1, id: "empty".into(), services: Services::default(), events: vec![] } }
}
object_struct! {
    #[derive(Default)]
    #[serde(deny_unknown_fields)]
    pub(crate) struct Services {
        #[serde(default, deserialize_with = "present_provider", skip_serializing_if = "Option::is_none")]
        pub assets: Option<TextProvider>,
        #[serde(default, deserialize_with = "present_provider", skip_serializing_if = "Option::is_none")]
        pub niri: Option<NiriProvider>,
    }
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
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) enum TextProviderVersion { #[serde(rename = "mock-text-assets-v1")] V1 }
object_struct! {
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(crate) struct NiriProvider {
        pub provider: NiriProviderVersion,
        pub latency_ms: u32,
        pub initial_state: Snapshot,
        pub error: Option<InjectedError>,
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) enum NiriProviderVersion { #[serde(rename = "mock-niri-v1")] V1 }
object_struct! {
    #[serde(deny_unknown_fields)]
    pub(crate) struct AssetState {
        #[serde(deserialize_with = "unique_assets")]
        pub assets: BTreeMap<String, AssetBehavior>,
    }
}
fn unique_assets<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<BTreeMap<String, AssetBehavior>, D::Error> {
    struct Assets;
    impl<'de> serde::de::Visitor<'de> for Assets {
        type Value = BTreeMap<String, AssetBehavior>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("unique fixture asset paths") }
        fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
            let mut assets = BTreeMap::new();
            while let Some((path, behavior)) = map.next_entry::<String, AssetBehavior>()? {
                if assets.insert(path, behavior).is_some() { return Err(serde::de::Error::custom("duplicate fixture asset path")); }
            }
            Ok(assets)
        }
    }
    deserializer.deserialize_map(Assets)
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
pub(crate) enum ErrorCode { Io, TooLarge, InvalidUtf8, NiriProtocol }
impl ErrorCode { fn as_str(self) -> &'static str { match self { Self::Io => "Io", Self::TooLarge => "TooLarge", Self::InvalidUtf8 => "InvalidUtf8", Self::NiriProtocol => "NiriProtocol" } } }
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
            #[serde(rename = "niri.state")]
            Niri { #[serde(rename = "atMs")] at_ms: u32, value: Snapshot },
            #[serde(rename = "niri.error")]
            NiriError { #[serde(rename = "atMs")] at_ms: u32, value: InjectedError },
        }
        Ok(match deserialize_object::<D, Input>(deserializer)? {
            Input::Assets { at_ms, value } => Self { at_ms, update: StateEvent::Assets(value) },
            Input::Niri { at_ms, value } => Self { at_ms, update: StateEvent::Niri(value) },
            Input::NiriError { at_ms, value } => Self { at_ms, update: StateEvent::NiriError(value) },
        })
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "channel", content = "value", deny_unknown_fields)]
pub(crate) enum StateEvent {
    #[serde(rename = "assets.state")] Assets(AssetState),
    #[serde(rename = "niri.state")] Niri(Snapshot),
    #[serde(rename = "niri.error")] NiriError(InjectedError),
}
impl<'de> Deserialize<'de> for StateEvent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "channel", content = "value", deny_unknown_fields)]
        enum Input {
            #[serde(rename = "assets.state")] Assets(AssetState),
            #[serde(rename = "niri.state")] Niri(Snapshot),
            #[serde(rename = "niri.error")] NiriError(InjectedError),
        }
        Ok(match deserialize_object::<D, Input>(deserializer)? {
            Input::Assets(value) => Self::Assets(value),
            Input::Niri(value) => Self::Niri(value),
            Input::NiriError(value) => Self::NiriError(value),
        })
    }
}

object_struct! {
    #[serde(deny_unknown_fields)]
    pub(crate) struct Snapshot { pub windows: Vec<Window>, pub workspaces: Vec<Workspace> }
}
object_struct! {
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(crate) struct Window {
        pub id: String,
        #[serde(deserialize_with = "required_option")] pub title: Option<String>,
        #[serde(deserialize_with = "required_option")] pub app_id: Option<String>,
        #[serde(deserialize_with = "required_option")] pub pid: Option<u32>,
        #[serde(deserialize_with = "required_option")] pub workspace_id: Option<String>,
        pub focused: bool, pub floating: bool, pub urgent: bool,
    }
}
object_struct! {
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(crate) struct Workspace {
        pub id: String, pub index: u8,
        #[serde(deserialize_with = "required_option")] pub name: Option<String>,
        #[serde(deserialize_with = "required_option")] pub output: Option<String>,
        pub urgent: bool, pub active: bool, pub focused: bool,
        #[serde(deserialize_with = "required_option")] pub active_window_id: Option<String>,
    }
}
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<Option<T>, D::Error> { Option::deserialize(deserializer) }
fn id(value: &str) -> Result<u64, String> {
    let parsed = value.parse::<u64>().map_err(|_| "niri IDs must be canonical decimal u64 strings".to_string())?;
    if parsed.to_string() != value { return Err("niri IDs must be canonical decimal u64 strings".into()); }
    Ok(parsed)
}
impl Snapshot {
    fn validate(&self) -> Result<(), String> {
        if self.windows.len() > 4096 || self.workspaces.len() > 4096 { return Err("niri fixture exceeds 4096 rows".into()); }
        let mut windows = BTreeSet::new();
        for window in &self.windows {
            if !windows.insert(id(&window.id)?) { return Err("duplicate niri window ID".into()); }
            if let Some(value) = &window.workspace_id { id(value)?; }
        }
        let mut workspaces = BTreeSet::new();
        for workspace in &self.workspaces {
            if !workspaces.insert(id(&workspace.id)?) { return Err("duplicate niri workspace ID".into()); }
            if let Some(value) = &workspace.active_window_id { id(value)?; }
        }
        Ok(())
    }
    fn native(&self) -> NiriSnapshot {
        let mut windows: Vec<_> = self.windows.iter().map(|w| Arc::new(NiriWindow {
            id: id(&w.id).expect("validated ID"), title: w.title.clone(), app_id: w.app_id.clone(), pid: w.pid,
            workspace_id: w.workspace_id.as_deref().map(|value| id(value).expect("validated ID")),
            is_focused: w.focused, is_floating: w.floating, is_urgent: w.urgent,
        })).collect();
        let mut workspaces: Vec<_> = self.workspaces.iter().map(|w| Arc::new(NiriWorkspace {
            id: id(&w.id).expect("validated ID"), idx: w.index, name: w.name.clone(), output: w.output.clone(),
            is_urgent: w.urgent, is_active: w.active, is_focused: w.focused,
            active_window_id: w.active_window_id.as_deref().map(|value| id(value).expect("validated ID")),
        })).collect();
        windows.sort_by_key(|w| w.id); workspaces.sort_by_key(|w| w.id);
        NiriSnapshot { windows, workspaces }
    }
}
impl AssetState {
    fn validate(&self) -> Result<(), String> {
        if self.assets.len() > 1024 { return Err("fixture exceeds 1024 text assets".into()); }
        for (path, behavior) in &self.assets {
            if path.is_empty() || path.contains('\\') || path.contains('\0') || path.split('/').any(|p| p.is_empty() || p == "." || p == "..") { return Err("fixture asset path must be canonical package-relative text".into()); }
            if let Some(error) = &behavior.error { error.validate()?; if matches!(error.code, ErrorCode::NiriProtocol) { return Err("NiriProtocol is not a text asset error".into()); } }
        }
        Ok(())
    }
}
impl FixtureDefinition {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 { return Err("unsupported fixture schemaVersion; expected 1".into()); }
        if self.id.trim().is_empty() || self.id.chars().count() > 128 { return Err("fixture id must contain 1..=128 characters of nonblank text".into()); }
        if self.events.len() > 4096 { return Err("fixture exceeds 4096 events".into()); }
        if let Some(provider) = &self.services.assets { provider.initial_state.validate()?; }
        if let Some(provider) = &self.services.niri { provider.initial_state.validate()?; if let Some(error) = &provider.error { error.validate()?; } }
        for event in &self.events { self.validate_event(&event.update)?; }
        Ok(())
    }
    fn validate_event(&self, event: &StateEvent) -> Result<(), String> {
        match event {
            StateEvent::Assets(state) if self.services.assets.is_some() => state.validate(),
            StateEvent::Niri(state) if self.services.niri.is_some() => state.validate(),
            StateEvent::NiriError(error) if self.services.niri.is_some() => error.validate(),
            _ => Err("fixture event requires its declared provider".into()),
        }
    }
}

pub(super) enum Delivery {
    Asset { generation: u64, id: u64, result: Result<String, ServiceError> },
    Niri { generation: u64, id: u64, result: Result<NiriSnapshot, ServiceError> },
}
enum Scheduled {
    Event(StateEvent),
    Asset { generation: u64, id: u64, result: Result<String, ServiceError>, injected: bool },
    Niri { generation: u64, id: u64 },
}
struct Subscription { generation: u64, active: bool }
struct Call { generation: u64, service: &'static str, method: &'static str, error: Option<&'static str>, error_delivered: bool, acknowledged: bool }
struct State {
    scheduled: BTreeMap<(u64, u8, u64), Scheduled>,
    deliveries: VecDeque<Delivery>,
    assets: Option<AssetState>,
    niri: Option<Snapshot>,
    niri_error: Option<InjectedError>,
    subscriptions: BTreeMap<u64, Subscription>,
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
        let scheduled = definition.events.iter().enumerate().map(|(i, event)| ((u64::from(event.at_ms) * 1000, 0, i as u64), Scheduled::Event(event.update.clone()))).collect();
        Rc::new(Self { state: RefCell::new(State {
            scheduled, deliveries: VecDeque::new(), assets: definition.services.assets.as_ref().map(|p| p.initial_state.clone()),
            niri: definition.services.niri.as_ref().map(|p| p.initial_state.clone()), niri_error: definition.services.niri.as_ref().and_then(|p| p.error.clone()),
            subscriptions: BTreeMap::new(), calls: BTreeMap::new(), next_registration_id: 1, sequence: 0, trace: vec![], trace_bytes: 0, dropped: 0, failure: invalid, disposed: false,
        }), definition, environment })
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
        state.failure.get_or_insert_with(|| error.clone());
        self.record(&mut state, "contractFailure", json!({"code":error.code,"message":error.message}));
        error
    }
    pub(crate) fn contract_failure(&self) -> Option<ContractFailure> { self.state.borrow().failure.clone() }
    pub(super) fn malformed(&self, message: impl Into<String>) -> ContractFailure { self.fail(failure("E_FIXTURE_CONTRACT", message)) }
    pub(super) fn require_assets(&self) -> Result<(), ContractFailure> {
        if self.definition.services.assets.is_none() { return Err(self.fail(failure("E_FIXTURE_MISSING_SERVICE", "preview fixture does not provide readTextAsset"))); }
        Ok(())
    }
    pub(super) fn registration_id(&self) -> Option<u64> {
        let mut state = self.state.borrow_mut();
        let id = state.next_registration_id;
        if state.disposed || id>super::MAX_SAFE_ID { return None; }
        state.next_registration_id += 1;
        Some(id)
    }
    fn admission(&self, service: &'static str, method: &'static str, request_id: u64, generation: u64) -> Result<(), ContractFailure> {
        if let Some(error) = self.contract_failure() { return Err(error); }
        let mut state = self.state.borrow_mut();
        if state.disposed { drop(state); return Err(self.malformed("fixture runtime is disposed")); }
        // Calls remain bounded independently of the lossy trace: exact assertions cannot rely on trace retention.
        if state.calls.len() >= 16384 { drop(state); return Err(self.malformed("fixture call limit reached")); }
        if state.calls.contains_key(&request_id) { drop(state); return Err(self.malformed("duplicate fixture request ID")); }
        state.calls.insert(request_id, Call { generation, service, method, error: None, error_delivered: false, acknowledged: false });
        self.record(&mut state, "request", json!({"requestId":request_id,"service":service,"method":method}));
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
        self.state.borrow_mut().scheduled.insert((deadline, 1, request_id), Scheduled::Asset { generation, id: request_id, result, injected });
        let mut state = self.state.borrow_mut();
        self.record(&mut state, "arguments", json!({"requestId":request_id,"path":path}));
        Ok(())
    }
    pub(super) fn subscribe(&self, generation: u64, request_id: u64) -> Result<(), ContractFailure> {
        self.admission("niri", "watchNiri", request_id, generation)?;
        let Some(provider) = &self.definition.services.niri else { return Err(self.fail(failure("E_FIXTURE_MISSING_SERVICE", "preview fixture does not provide watchNiri"))); };
        let deadline = self.environment.now_us().checked_add(u64::from(provider.latency_ms) * 1000).ok_or_else(|| self.malformed("fixture deadline overflow"))?;
        let mut state = self.state.borrow_mut();
        state.subscriptions.insert(request_id, Subscription { generation, active: false });
        state.scheduled.insert((deadline, 1, request_id), Scheduled::Niri { generation, id: request_id });
        self.record(&mut state, "subscription", json!({"requestId":request_id}));
        Ok(())
    }
    pub(crate) fn next_deadline_us(&self) -> Option<u64> { self.state.borrow().scheduled.first_key_value().map(|(key, _)| key.0) }
    pub(crate) fn release_due(&self) -> Result<bool, ContractFailure> {
        if let Some(error) = self.contract_failure() { return Err(error); }
        let now = self.environment.now_us();
        let mut state = self.state.borrow_mut();
        let mut worked = false;
        while state.scheduled.first_key_value().is_some_and(|(key, _)| key.0 <= now) {
            let (_, scheduled) = state.scheduled.pop_first().expect("due work");
            if state.deliveries.len() >= 1024 {
                let error = failure("E_FIXTURE_CONTRACT", "fixture delivery queue exceeds 1024 entries");
                state.failure.get_or_insert_with(|| error.clone());
                return Err(error);
            }
            worked = true;
            match scheduled {
                Scheduled::Event(event) => self.apply(&mut state, event),
                Scheduled::Asset { generation, id, result, injected } => {
                    if injected { if let Some(call) = state.calls.get_mut(&id) { call.error = result.as_ref().err().map(|e| e.code); } }
                    self.record(&mut state, "response", json!({"requestId":id,"errorCode":result.as_ref().err().map(|e| e.code)}));
                    state.deliveries.push_back(Delivery::Asset { generation, id, result });
                }
                Scheduled::Niri { generation, id } => {
                    if let Some(subscription) = state.subscriptions.get_mut(&id) { subscription.active = true; }
                    self.queue_snapshot(&mut state, generation, id);
                }
            }
        }
        if let Some(error) = &state.failure { return Err(error.clone()); }
        Ok(worked)
    }
    fn queue_snapshot(&self, state: &mut State, generation: u64, id: u64) {
        if state.deliveries.len() >= 1024 {
            state.failure.get_or_insert_with(|| failure("E_FIXTURE_CONTRACT", "fixture delivery queue exceeds 1024 entries"));
            return;
        }
        let result = if let Some(error) = &state.niri_error {
            if let Some(call) = state.calls.get_mut(&id) { call.error = Some(error.code.as_str()); }
            Err(error.service_error())
        } else { Ok(state.niri.as_ref().expect("declared niri provider").native()) };
        self.record(state, "response", json!({"requestId":id,"errorCode":result.as_ref().err().map(|e| e.code)}));
        if result.is_err() { state.subscriptions.remove(&id); }
        state.deliveries.push_back(Delivery::Niri { generation, id, result });
    }
    fn apply(&self, state: &mut State, event: StateEvent) {
        self.record(state, "state", serde_json::to_value(&event).expect("typed state"));
        let notify = !matches!(&event, StateEvent::Assets(_));
        match event {
            StateEvent::Assets(assets) => state.assets = Some(assets),
            StateEvent::Niri(snapshot) => { state.niri = Some(snapshot); state.niri_error = None; }
            StateEvent::NiriError(error) => state.niri_error = Some(error),
        }
        if notify {
            let listeners: Vec<_> = state.subscriptions.iter().filter(|(_, s)| s.active).map(|(&id, s)| (id, s.generation)).collect();
            for (id, generation) in listeners { self.queue_snapshot(state, generation, id); }
        }
    }
    pub(crate) fn emit(&self, channel: &str, value: &Value) -> Result<(), ContractFailure> {
        if let Some(error) = self.contract_failure() { return Err(error); }
        let event: StateEvent = serde_json::from_value(json!({"channel":channel,"value":value})).map_err(|e| self.malformed(format!("invalid fixture event: {e}")))?;
        self.definition.validate_event(&event).map_err(|e| self.malformed(e))?;
        let mut state = self.state.borrow_mut();
        if state.disposed { drop(state); return Err(self.malformed("fixture runtime is disposed")); }
        self.apply(&mut state, event);
        if let Some(error) = &state.failure { return Err(error.clone()); }
        Ok(())
    }
    pub(super) fn take(&self, asset: bool) -> Option<Delivery> {
        let mut state = self.state.borrow_mut();
        if state.deliveries.front().is_some_and(|d| matches!(d, Delivery::Asset { .. }) == asset) { state.deliveries.pop_front() } else { None }
    }
    pub(super) fn has_delivery(&self, asset: bool) -> bool { self.state.borrow().deliveries.front().is_some_and(|d| matches!(d, Delivery::Asset { .. }) == asset) }
    pub(super) fn delivered(&self, id: u64, error: bool) {
        if error { if let Some(call) = self.state.borrow_mut().calls.get_mut(&id) { call.error_delivered = true; } }
    }
    pub(super) fn ignored(&self, id: u64, reason: &str) {
        let mut state = self.state.borrow_mut();
        self.record(&mut state, "ignored", json!({"requestId":id,"reason":reason}));
    }
    pub(super) fn cancel(&self, id: u64) {
        let mut state = self.state.borrow_mut();
        if let Some(call) = state.calls.get_mut(&id) {
            if !call.error_delivered { call.error = None; }
        }
        state.subscriptions.remove(&id);
        state.scheduled.retain(|_, item| !matches!(item, Scheduled::Asset { id: other, .. } | Scheduled::Niri { id: other, .. } if *other == id));
        state.deliveries.retain(|item| !matches!(item, Delivery::Asset { id: other, .. } | Delivery::Niri { id: other, .. } if *other == id));
        self.record(&mut state, "cancellation", json!({"requestId":id}));
    }
    /// Retire only one session's registrations; scenario events and the exact call ledger survive.
    pub(super) fn cancel_generation(&self, generation: u64) {
        let ids: Vec<_> = self.state.borrow().calls.iter()
            .filter_map(|(&id, call)|(call.generation==generation).then_some(id)).collect();
        for id in ids { self.cancel(id); }
    }
    pub(crate) fn dispose(&self) {
        let mut state = self.state.borrow_mut();
        if state.disposed { return; }
        let scheduled = state.scheduled.len();
        let deliveries = state.deliveries.len();
        let subscriptions = state.subscriptions.len();
        for call in state.calls.values_mut() {
            if !call.error_delivered { call.error = None; }
        }
        state.disposed = true; state.scheduled.clear(); state.deliveries.clear(); state.subscriptions.clear();
        self.record(&mut state, "disposal", json!({"cancelledScheduled":scheduled,"cancelledDeliveries":deliveries,"cancelledSubscriptions":subscriptions}));
    }
    pub(crate) fn trace(&self) -> Value {
        let state = self.state.borrow();
        json!({"schemaVersion":1,"records":state.trace,"droppedRecords":state.dropped,"recordLimit":MAX_TRACE_RECORDS,"byteLimit":MAX_TRACE_BYTES})
    }
    pub(crate) fn assert_calls(&self, service: &str, method: &str, count: u64, error_code: Option<&str>) -> Result<(), ContractFailure> {
        if let Some(error) = self.contract_failure() { return Err(error); }
        if !matches!((service, method), ("assets", "readTextAsset") | ("niri", "watchNiri")) {
            return Err(failure("E_ASSERTION", format!("unknown fixture method {service}.{method}")));
        }
        let mut state = self.state.borrow_mut();
        let matches: Vec<_> = state.calls.iter().filter(|(_, call)| call.service == service && call.method == method).map(|(&id, _)| id).collect();
        if matches.len() as u64 != count || error_code.is_some_and(|code| !matches.iter().any(|id| state.calls[id].error == Some(code))) {
            return Err(failure("E_ASSERTION", format!("expected {count} {service}.{method} calls with error {error_code:?}; observed {}", matches.len())));
        }
        if let Some(code) = error_code { for id in matches { let call = state.calls.get_mut(&id).unwrap(); if call.error == Some(code) { call.acknowledged = true; } } }
        Ok(())
    }
    pub(crate) fn finalize(&self) -> Result<(), ContractFailure> {
        let state = self.state.borrow();
        if let Some(error) = &state.failure { return Err(error.clone()); }
        if let Some(call) = state.calls.values().find(|call| call.error.is_some() && !call.acknowledged) {
            return Err(failure("E_FIXTURE_UNEXPECTED_ERROR", format!("unacknowledged injected {}.{} error {}", call.service, call.method, call.error.unwrap())));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
