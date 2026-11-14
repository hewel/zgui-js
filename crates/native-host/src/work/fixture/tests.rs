use super::*;
use crate::{test_package::Fixture, work::{ServicePolicy, Work}};
use js_runtime::{JsRuntime, RuntimeLimits};
use std::time::Duration;
use zgui::{platform::{SurfaceId, VirtualClock, WakeReason, Waker}, view::{HostHandle, stub::StubHost}};

struct NoWake;
impl Waker for NoWake { fn wake(&self, _: WakeReason) {} }
struct Harness {
    work: Rc<Work>,
    engine: JsRuntime,
    runtime: Rc<FixtureRuntime>,
    clock: Arc<VirtualClock>,
    _package: Fixture,
}
fn definition(services: Value, events: Value) -> FixtureDefinition {
    let fixture: FixtureDefinition = serde_json::from_value(json!({"schemaVersion":1,"id":"test","services":services,"events":events})).unwrap();
    fixture.validate().unwrap();
    fixture
}
fn niri(latency: u32) -> Value { json!({"provider":"mock-niri-v1","latencyMs":latency,"initialState":{"windows":[],"workspaces":[]}}) }
fn assets(latency: u32) -> Value { json!({"provider":"mock-text-assets-v1","latencyMs":latency,"initialState":{"assets":{"test.txt":{}}}}) }
fn snapshot(name: &str) -> Value {
    json!({"windows":[],"workspaces":[{"id":"18446744073709551615","index":1,"name":name,"output":null,"urgent":false,"active":true,"focused":true,"activeWindowId":null}]})
}
impl Harness {
    fn new(definition: FixtureDefinition) -> Self {
        let package = Fixture::new("export const hostApiVersion = 1; export function mount() { return () => {}; }", "");
        let clock = Arc::new(VirtualClock::new());
        let environment = Environment::new(clock.clone(), Some(1000), 7);
        let runtime = FixtureRuntime::new(definition, environment.clone());
        let work = Work::new(HostHandle::from_rc(Rc::new(StubHost::default())), Some(package.package()), 7,
            host_services::Executor::new(1, 8).unwrap(), Arc::new(NoWake), SurfaceId::new(7), ServicePolicy::Fixture(runtime.clone()));
        let mut engine = JsRuntime::new(RuntimeLimits::default()).unwrap();
        engine.with_context(|ctx| { environment.install(&ctx)?; work.install(&ctx)?; ctx.eval::<(), _>("globalThis.seen = []; globalThis.errors = [];") }).unwrap();
        work.mounting(); work.mounted();
        Self { work, engine, runtime, clock, _package: package }
    }
    fn script(&mut self, source: &str) -> String {
        self.engine.with_context(|ctx| ctx.eval::<rquickjs::Coerced<String>, _>(source)).unwrap().0
    }
    fn jobs(&mut self) {
        let report = self.engine.pump_jobs();
        assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
        assert!(!report.has_pending_jobs);
    }
    fn deliver(&mut self) {
        for _ in 0..2048 {
            if let Some(completion) = self.work.next_completion() {
                self.engine.with_context(|ctx| self.work.settle(&ctx, completion)).unwrap();
            } else if let Some(delivery) = self.work.next_subscription() {
                self.engine.with_context(|ctx| self.work.deliver_subscription(&ctx, delivery)).unwrap();
            } else { self.jobs(); return; }
            self.jobs();
        }
        panic!("fixture test exceeded bounded delivery count");
    }
}
impl Drop for Harness { fn drop(&mut self) { self.work.finish_stop(); } }

#[test]
fn zero_latency_never_enters_javascript_during_construction_and_uses_verified_bytes() {
    let mut h = Harness::new(definition(json!({"assets":assets(0)}), json!([])));
    // Mutating the source package after load must not change the fixture payload.
    let directory = h.work.package.as_ref().unwrap().directory.clone();
    std::fs::write(directory.join("test.txt"), "ambient replacement").unwrap();
    h.script("globalThis.request = __zguiServices.readTextAsset('test.txt'); request.promise.then(value => seen.push(value));");
    h.jobs();
    assert_eq!(h.script("seen.length"), "0");
    assert!(!h.work.has_completions());
    assert_eq!(h.runtime.next_deadline_us(), Some(0));
    assert!(h.runtime.release_due().unwrap());
    assert_eq!(h.script("seen.length"), "0", "release_due must never enter JS");
    h.deliver();
    assert_eq!(h.script("seen.join(',')"), "completed");
    assert_eq!(h.work.counts().2, 0);
    h.runtime.assert_calls("assets", "readTextAsset", 1, None).unwrap();
    h.runtime.finalize().unwrap();
}

#[test]
fn events_precede_equal_deadline_requests_and_cross_service_delivery_keeps_release_order() {
    let mut h = Harness::new(definition(json!({"assets":assets(10),"niri":niri(10)}), json!([
        {"atMs":10,"channel":"niri.state","value":snapshot("first")},
        {"atMs":10,"channel":"niri.state","value":snapshot("second")}
    ])));
    h.script("__zguiServices.watchNiri(value => seen.push('niri:' + value.workspaces[0].name + ':' + value.workspaces[0].id), error => errors.push(error.code)); __zguiServices.readTextAsset('test.txt').promise.then(value => seen.push(value));");
    h.clock.advance(Duration::from_micros(9999));
    assert!(!h.runtime.release_due().unwrap());
    assert_eq!(h.runtime.next_deadline_us(), Some(10000));
    h.clock.advance(Duration::from_micros(1));
    h.runtime.release_due().unwrap(); h.deliver();
    assert_eq!(h.script("seen.join('|')"), "niri:second:18446744073709551615|completed");
    let trace = h.runtime.trace();
    let released: Vec<_> = trace["records"].as_array().unwrap().iter()
        .filter(|record| record["kind"] == "state" || record["kind"] == "response")
        .map(|record| record["kind"].as_str().unwrap()).collect();
    assert_eq!(released, ["state", "state", "response", "response"]);
    h.runtime.finalize().unwrap();
}

#[test]
fn cancellation_rechecks_selected_deliveries_and_cancels_future_work() {
    let mut h = Harness::new(definition(json!({"assets":assets(20),"niri":niri(20)}), json!([])));
    h.script("globalThis.request = __zguiServices.readTextAsset('test.txt'); request.promise.then(value => seen.push(value), error => errors.push(error.code)); globalThis.subscription = __zguiServices.watchNiri(value => seen.push(value), error => errors.push(error.code));");
    h.clock.advance(Duration::from_millis(20)); h.runtime.release_due().unwrap();
    let selected = h.work.next_completion().unwrap();
    h.script("request.cancel(); request.cancel(); subscription.cancel(); subscription.cancel();");
    h.engine.with_context(|ctx| h.work.settle(&ctx, selected)).unwrap();
    h.deliver();
    assert_eq!(h.script("seen.length + ':' + errors.join(',')"), "0:Cancelled");
    assert_eq!(h.work.counts(), (0, 0, 0, 0));
    assert_eq!(h.runtime.next_deadline_us(), None);
    h.runtime.emit("niri.state", &snapshot("after cancellation")).unwrap();
    h.deliver();
    assert_eq!(h.script("seen.length"), "0");
    h.runtime.finalize().unwrap();
}

#[test]
fn emitted_state_is_typed_and_terminal_errors_require_exact_acknowledgement() {
    let mut h = Harness::new(definition(json!({"niri":niri(0)}), json!([])));
    h.script("globalThis.subscription = __zguiServices.watchNiri(value => seen.push(value.workspaces.length), error => errors.push(error.code));");
    h.runtime.release_due().unwrap(); h.deliver();
    h.runtime.emit("niri.state", &snapshot("changed")).unwrap(); h.deliver();
    assert_eq!(h.script("seen.join(',')"), "0,1");
    h.runtime.emit("niri.error", &json!({"code":"Io","message":"injected disconnect"})).unwrap(); h.deliver();
    assert_eq!(h.script("errors.join(',')"), "Io");
    assert_eq!(h.work.counts().3, 0);
    assert_eq!(h.runtime.finalize().unwrap_err().code, "E_FIXTURE_UNEXPECTED_ERROR");
    assert_eq!(h.runtime.assert_calls("niri", "watchNiri", 2, Some("Io")).unwrap_err().code, "E_ASSERTION");
    assert_eq!(h.runtime.assert_calls("niri", "watchNiri", 1, Some("TooLarge")).unwrap_err().code, "E_ASSERTION");
    h.runtime.assert_calls("niri", "watchNiri", 1, None).unwrap();
    assert!(h.runtime.finalize().is_err(), "count-only assertions must not acknowledge injections");
    h.runtime.assert_calls("niri", "watchNiri", 1, Some("Io")).unwrap();
    h.runtime.finalize().unwrap();
    h.runtime.emit("niri.state", &snapshot("no reconnect")).unwrap(); h.deliver();
    assert_eq!(h.script("seen.join(',')"), "0,1");
}

#[test]
fn asset_state_replacement_only_affects_subsequent_reads_and_can_recover() {
    let mut h = Harness::new(definition(json!({"assets":assets(10)}), json!([])));
    h.script("__zguiServices.readTextAsset('test.txt').promise.then(value => seen.push(value));");
    h.runtime.emit("assets.state", &json!({"assets":{"test.txt":{"error":{"code":"Io","message":"injected read failure"}}}})).unwrap();
    h.script("__zguiServices.readTextAsset('test.txt').promise.catch(error => errors.push(error.code));");
    h.clock.advance(Duration::from_millis(10)); h.runtime.release_due().unwrap(); h.deliver();
    assert_eq!(h.script("seen.join(',') + ':' + errors.join(',')"), "completed:Io");
    h.runtime.assert_calls("assets", "readTextAsset", 2, Some("Io")).unwrap();
    h.runtime.emit("assets.state", &json!({"assets":{"test.txt":{}}})).unwrap();
    h.script("__zguiServices.readTextAsset('test.txt').promise.then(value => seen.push(value));");
    h.clock.advance(Duration::from_millis(10)); h.runtime.release_due().unwrap(); h.deliver();
    assert_eq!(h.script("seen.length"), "2");
    h.runtime.finalize().unwrap();
}

#[test]
fn caught_missing_malformed_and_undeclared_calls_remain_sticky() {
    for (services, source, expected) in [
        (json!({}), "__zguiServices.readTextAsset('test.txt')", "E_FIXTURE_MISSING_SERVICE"),
        (json!({}), "__zguiServices.watchNiri(() => {}, () => {})", "E_FIXTURE_MISSING_SERVICE"),
        (json!({"assets":assets(0)}), "__zguiServices.readTextAsset('../test.txt')", "E_FIXTURE_CONTRACT"),
        (json!({"assets":assets(0)}), "__zguiServices.readTextAsset(1)", "E_FIXTURE_CONTRACT"),
        (json!({"niri":niri(0)}), "__zguiServices.watchNiri(() => {}, null)", "E_FIXTURE_CONTRACT"),
    ] {
        let mut h = Harness::new(definition(services, json!([])));
        assert_eq!(h.script(&format!("try {{ {source}; 'unexpected success' }} catch (error) {{ error.code }}")), expected);
        assert_eq!(h.work.contract_failure().unwrap().code, expected);
        assert_eq!(h.runtime.finalize().unwrap_err().code, expected);
    }
    let h = Harness::new(definition(json!({"niri":niri(0)}), json!([])));
    assert_eq!(h.runtime.emit("niri.state", &json!({"windows":[],"workspaces":[],"extra":true})).unwrap_err().code, "E_FIXTURE_CONTRACT");
    assert!(h.runtime.finalize().is_err());
}

#[test]
fn cancelled_injected_errors_are_not_observed_or_acknowledged() {
    let mut provider = assets(0);
    provider["initialState"]["assets"]["test.txt"]["error"] = json!({"code":"Io","message":"not delivered"});
    let mut h = Harness::new(definition(json!({"assets":provider}), json!([])));
    h.script("globalThis.request = __zguiServices.readTextAsset('test.txt'); request.promise.catch(error => errors.push(error.code));");
    h.runtime.release_due().unwrap();
    h.script("request.cancel()"); h.deliver();
    assert_eq!(h.script("errors.join(',')"), "Cancelled");
    h.runtime.finalize().unwrap();
}

#[test]
fn traces_are_lossy_but_assertions_and_disposal_are_not() {
    let h = Harness::new(definition(json!({"assets":assets(0)}), json!([])));
    for i in 1..=5000 {
        h.runtime.admission("assets", "readTextAsset", i, 7).unwrap();
        h.runtime.cancel(i);
    }
    let trace = h.runtime.trace();
    assert!(trace["records"].as_array().unwrap().len() <= MAX_TRACE_RECORDS);
    assert!(trace["droppedRecords"].as_u64().unwrap() > 0);
    assert!(serde_json::to_vec(&trace["records"]).unwrap().len() <= MAX_TRACE_BYTES + MAX_TRACE_RECORDS + 2);
    h.runtime.assert_calls("assets", "readTextAsset", 5000, None).unwrap();
    h.work.finish_stop(); h.work.finish_stop();
    assert_eq!(h.runtime.next_deadline_us(), None);
    assert_eq!(h.work.counts(), (0, 0, 0, 0));
    h.runtime.finalize().unwrap();
}

#[test]
fn fixture_validation_rejects_unknown_fields_duplicate_ids_and_invalid_event_shapes() {
    let valid = json!({"schemaVersion":1,"id":"closed","services":{"niri":niri(0)},"events":[{"atMs":0,"channel":"niri.state","value":snapshot("valid")}]});
    let fixture: FixtureDefinition = serde_json::from_value(valid.clone()).unwrap();
    fixture.validate().unwrap();
    for changed in [
        { let mut v = valid.clone(); v["events"][0]["extra"] = json!(true); v },
        { let mut v = valid.clone(); v["events"][0]["channel"] = json!("niri.command"); v },
        { let mut v = valid.clone(); v["services"]["niri"]["socket"] = json!("/tmp/live"); v },
        { let mut v = valid.clone(); v["services"]["assets"] = Value::Null; v },
    ] { assert!(serde_json::from_value::<FixtureDefinition>(changed).is_err()); }
    let mut duplicate = fixture.clone();
    let StateEvent::Niri(state) = &mut duplicate.events[0].update else { unreachable!() };
    state.workspaces.push(state.workspaces[0].clone());
    assert!(duplicate.validate().is_err());
    let mut invalid_id = fixture;
    let StateEvent::Niri(state) = &mut invalid_id.events[0].update else { unreachable!() };
    state.workspaces[0].id = "18446744073709551616".into();
    assert!(invalid_id.validate().is_err());
}

#[test]
fn cancellation_before_deadline_removes_requests_and_delayed_subscriptions() {
    let mut h = Harness::new(definition(json!({"assets":assets(20),"niri":niri(20)}), json!([])));
    h.script("globalThis.request = __zguiServices.readTextAsset('test.txt'); request.promise.catch(error => errors.push(error.code)); globalThis.subscription = __zguiServices.watchNiri(value => seen.push(value), error => errors.push(error.code)); request.cancel(); subscription.cancel();");
    assert_eq!(h.runtime.next_deadline_us(), None);
    h.clock.advance(Duration::from_millis(20));
    assert!(!h.runtime.release_due().unwrap());
    h.deliver();
    assert_eq!(h.script("seen.length + ':' + errors.join(',')"), "0:Cancelled");
    assert_eq!(h.work.counts(), (0, 0, 0, 0));
    h.runtime.finalize().unwrap();
}

#[test]
fn cancellation_from_a_promise_job_retires_the_next_ready_request() {
    let mut h = Harness::new(definition(json!({"assets":assets(0)}), json!([])));
    h.script("globalThis.first = __zguiServices.readTextAsset('test.txt'); globalThis.second = __zguiServices.readTextAsset('test.txt'); first.promise.then(value => { seen.push(value); second.cancel(); }); second.promise.then(value => seen.push('unexpected'), error => errors.push(error.code));");
    h.runtime.release_due().unwrap(); h.deliver();
    assert_eq!(h.script("seen.length + ':' + errors.join(',')"), "1:Cancelled");
    assert_eq!(h.work.counts().2, 0);
    h.runtime.finalize().unwrap();
}

#[test]
fn duplicate_fixture_asset_keys_and_event_payload_fields_are_not_silently_overwritten() {
    for input in [
        r#"{"schemaVersion":1,"id":"duplicate","services":{"assets":{"provider":"mock-text-assets-v1","latencyMs":0,"initialState":{"assets":{"test.txt":{},"test.txt":{"error":{"code":"Io","message":"hidden"}}}}}},"events":[]}"#,
        r#"{"schemaVersion":1,"id":"duplicate","services":{"niri":{"provider":"mock-niri-v1","latencyMs":0,"initialState":{"windows":[],"workspaces":[]}}},"events":[{"atMs":0,"channel":"niri.state","value":{"windows":[],"windows":[],"workspaces":[]}}]}"#,
    ] {
        assert!(serde_json::from_str::<FixtureDefinition>(input).is_err());
    }
}

#[test]
fn disposal_cancels_released_but_undelivered_errors_without_releasing_any_js_roots_late() {
    let mut provider = niri(0);
    provider["error"] = json!({"code":"Io","message":"cancelled terminal response"});
    let mut h = Harness::new(definition(json!({"niri":provider}), json!([])));
    h.script("__zguiServices.watchNiri(value => seen.push(value), error => errors.push(error.code));");
    h.runtime.release_due().unwrap();
    h.work.begin_stop(); h.work.finish_stop();
    h.deliver();
    assert_eq!(h.script("seen.length + errors.length"), "0");
    assert_eq!(h.work.counts(), (0, 0, 0, 0));
    h.runtime.finalize().unwrap();
}
#[test]
fn retired_generation_preserves_fixture_ledger_events_and_new_registrations() {
    let mut h=Harness::new(definition(json!({"assets":assets(20),"niri":niri(20)}),json!([
        {"atMs":10,"channel":"niri.state","value":snapshot("after-resume")}
    ])));
    h.script("globalThis.oldRequest = __zguiServices.readTextAsset('test.txt'); oldRequest.promise.then(v => seen.push('old'), e => errors.push(e.code)); __zguiServices.watchNiri(v => seen.push('old-subscription'), e => errors.push(e.code));");
    let old_ids: Vec<_> = h.runtime.state.borrow().calls.keys().copied().collect();
    h.work.finish_stop();
    assert_eq!(h.work.counts(),(0,0,0,0));
    let work=Work::new(HostHandle::from_rc(Rc::new(StubHost::default())),Some(h._package.package()),8,
        host_services::Executor::new(1,8).unwrap(),Arc::new(NoWake),SurfaceId::new(8),ServicePolicy::Fixture(h.runtime.clone()));
    h.engine.with_context(|ctx|work.install(&ctx)).unwrap();
    work.mounting(); work.mounted();
    h.work=work;
    h.script("__zguiServices.readTextAsset('test.txt').promise.then(v => seen.push('new'), e => errors.push(e.code)); __zguiServices.watchNiri(v => seen.push(v.workspaces[0].name), e => errors.push(e.code));");
    let new_ids: Vec<_> = h.runtime.state.borrow().calls.keys().filter(|id|!old_ids.contains(id)).copied().collect();
    assert_eq!(new_ids.len(),2);
    assert!(new_ids.iter().all(|id|*id>*old_ids.last().unwrap()));
    h.clock.advance(Duration::from_millis(20));
    h.runtime.release_due().unwrap();
    h.deliver();
    assert_eq!(h.script("JSON.stringify(seen)"),r#"["new","after-resume"]"#);
    assert_eq!(h.script("errors.length"),"0");
    h.runtime.assert_calls("assets","readTextAsset",2,None).unwrap();
    h.runtime.assert_calls("niri","watchNiri",2,None).unwrap();
    assert!(!h.runtime.state.borrow().disposed);
    h.work.finish_stop();
    h.runtime.dispose();
    assert!(h.runtime.state.borrow().disposed);
    assert!(h.runtime.state.borrow().scheduled.is_empty());
    assert!(h.runtime.state.borrow().deliveries.is_empty());
    assert!(h.runtime.state.borrow().subscriptions.is_empty());
    h.runtime.finalize().unwrap();
}

fn rejects_arrays<T: serde::de::DeserializeOwned>(inputs: &[&str]) {
    for input in inputs {
        assert!(serde_json::from_str::<T>(input).is_err(), "accepted positional object: {input}");
    }
}

#[test]
fn every_fixture_object_boundary_rejects_positional_arrays() {
    rejects_arrays::<FixtureDefinition>(&[r#"[1,"empty",{},[]]"#]);
    rejects_arrays::<Services>(&["[]", "[null,null]"]);
    rejects_arrays::<TextProvider>(&[r#"["mock-text-assets-v1",0,{"assets":{}}]"#]);
    rejects_arrays::<NiriProvider>(&[r#"["mock-niri-v1",0,{"windows":[],"workspaces":[]},null]"#]);
    rejects_arrays::<AssetState>(&["[{}]"]);
    rejects_arrays::<AssetBehavior>(&["[]", "[null]"]);
    rejects_arrays::<InjectedError>(&[r#"["Io","failure"]"#]);
    rejects_arrays::<Snapshot>(&["[[],[]]"]);
    rejects_arrays::<Window>(&[r#"["1",null,null,null,null,false,false,false]"#]);
    rejects_arrays::<Workspace>(&[r#"["1",1,null,null,false,false,false,null]"#]);
    rejects_arrays::<FixtureEvent>(&[r#"[0,"niri.state",{"windows":[],"workspaces":[]}]"#]);
    rejects_arrays::<StateEvent>(&[r#"["niri.state",{"windows":[],"workspaces":[]}]"#]);
    for input in [
        r#"{"schemaVersion":1,"id":"bad","services":[],"events":[]}"#,
        r#"{"schemaVersion":1,"id":"bad","services":{"assets":["mock-text-assets-v1",0,{"assets":{}}]},"events":[]}"#,
        r#"{"schemaVersion":1,"id":"bad","services":{"assets":{"provider":"mock-text-assets-v1","latencyMs":0,"initialState":[{}]}},"events":[]}"#,
        r#"{"schemaVersion":1,"id":"bad","services":{"assets":{"provider":"mock-text-assets-v1","latencyMs":0,"initialState":{"assets":{"test.txt":[]}}}},"events":[]}"#,
        r#"{"schemaVersion":1,"id":"bad","services":{"assets":{"provider":"mock-text-assets-v1","latencyMs":0,"initialState":{"assets":{"test.txt":{"error":["Io","failure"]}}}}},"events":[]}"#,
        r#"{"schemaVersion":1,"id":"bad","services":{"niri":{"provider":"mock-niri-v1","latencyMs":0,"initialState":{"windows":[],"workspaces":[]}}},"events":[{"atMs":0,"channel":"niri.state","value":[[],[]]}]}"#,
        r#"{"schemaVersion":1,"id":"bad","services":{"niri":{"provider":"mock-niri-v1","latencyMs":0,"initialState":{"windows":[["1",null,null,null,null,false,false,false]],"workspaces":[]}}},"events":[]}"#,
        r#"{"schemaVersion":1,"id":"bad","services":{"niri":{"provider":"mock-niri-v1","latencyMs":0,"initialState":{"windows":[],"workspaces":[["1",1,null,null,false,false,false,null]]}}},"events":[]}"#,
    ] {
        assert!(serde_json::from_str::<FixtureDefinition>(input).is_err(), "accepted nested positional object: {input}");
    }
}

#[test]
fn optional_error_nulls_match_schema_without_accepting_positional_error_objects() {
    for input in ["{}", r#"{"error":null}"#] {
        assert!(serde_json::from_str::<AssetBehavior>(input).unwrap().error.is_none());
    }
    let mut provider = niri(0);
    assert!(serde_json::from_value::<NiriProvider>(provider.clone()).unwrap().error.is_none());
    provider["error"] = Value::Null;
    assert!(serde_json::from_value::<NiriProvider>(provider.clone()).unwrap().error.is_none());
    provider["error"] = json!(["Io", "failure"]);
    assert!(serde_json::from_value::<NiriProvider>(provider).is_err());
    for input in [r#"{"assets":null}"#, r#"{"niri":null}"#] {
        assert!(serde_json::from_str::<Services>(input).is_err());
    }
}

#[test]
fn map_only_decoding_preserves_duplicate_field_rejection_at_nested_boundaries() {
    fn rejects<T: serde::de::DeserializeOwned>(input: &str) {
        assert!(serde_json::from_str::<T>(input).is_err(), "accepted duplicate field: {input}");
    }
    rejects::<Services>(r#"{"niri":{"provider":"mock-niri-v1","latencyMs":0,"initialState":{"windows":[],"workspaces":[]}},"niri":{"provider":"mock-niri-v1","latencyMs":0,"initialState":{"windows":[],"workspaces":[]}}}"#);
    rejects::<TextProvider>(r#"{"provider":"mock-text-assets-v1","latencyMs":0,"latencyMs":1,"initialState":{"assets":{}}}"#);
    rejects::<AssetBehavior>(r#"{"error":null,"error":null}"#);
    rejects::<InjectedError>(r#"{"code":"Io","code":"TooLarge","message":"failure"}"#);
    rejects::<Window>(r#"{"id":"1","id":"2","title":null,"appId":null,"pid":null,"workspaceId":null,"focused":false,"floating":false,"urgent":false}"#);
    rejects::<Workspace>(r#"{"id":"1","index":1,"name":null,"name":null,"output":null,"urgent":false,"active":false,"focused":false,"activeWindowId":null}"#);
}
