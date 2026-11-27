use super::*;
use crate::{assets::AssetSource, test_package::Fixture, work::{Work, WorkSession}};
use js_runtime::{JsRuntime, RuntimeLimits};
use std::{sync::Arc, time::Duration};
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

#[test]
fn fixture_runtime_environment_rejects_invalid_and_duplicate_bindings() {
    let fixture = |runtime| format!(r#"{{"schemaVersion":1,"id":"bindings","services":{{}},"events":[],"runtimeEnvironment":{runtime}}}"#);
    for runtime in [
        r#"{"xdgRuntimeDir":"relative"}"#,
        r#"{"homeDir":"/synthetic/../home"}"#,
        r#"{"xdgDataHome":"$HOME/.local/share"}"#,
        r#"{"waylandDisplay":"foo/bar"}"#,
        r#"{"waylandDisplay":".."}"#,
        r#"{"waylandDisplay":"wayland\u0000"}"#,
        r#"{"namedSockets":{"bad name":"/socket"}}"#,
        r#"{"namedSockets":{"niri":"/run/./socket"}}"#,
        r#"{"namedSockets":{"niri":"/one","niri":"/two"}}"#,
    ] {
        let result = serde_json::from_str::<FixtureDefinition>(&fixture(runtime));
        assert!(result.is_err() || result.is_ok_and(|fixture| fixture.validate().is_err()), "accepted {runtime}");
    }
    let input = fixture(r#"{"xdgRuntimeDir":"/synthetic/run","homeDir":"/synthetic/home","waylandDisplay":"wayland-2","namedSockets":{"niri":"/synthetic/niri.sock"}}"#);
    serde_json::from_str::<FixtureDefinition>(&input).expect("read valid runtime environment")
        .validate().expect("synthetic directories need not exist");
}
fn assets(latency: u32) -> Value { json!({"provider":"mock-text-assets-v1","latencyMs":latency,"initialState":{"assets":{"test.txt":{}}}}) }
impl Harness {
    fn new(definition: FixtureDefinition) -> Self {
        let package = Fixture::new("export const hostApiVersion = 1; export function mount() { return () => {}; }", "");
        let clock = Arc::new(VirtualClock::new());
        let environment = Environment::new(clock.clone(), Some(1000), 7);
        let runtime = FixtureRuntime::new(definition, environment.clone());
        let pkg = package.package();
        let capabilities = Arc::new(pkg.manifest.capabilities.clone());
        let work = Work::new(
            WorkSession { package: Some(pkg), capabilities, generation: 7, assets: AssetSource::Fixture(runtime.clone()), resolved: Default::default(), launch: Default::default() },
            HostHandle::from_rc(Rc::new(StubHost::default())), SurfaceId::new(7), Arc::new(NoWake),
        );
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
    let directory = h._package.package().directory.clone();
    std::fs::write(directory.join("test.txt"), "ambient replacement").unwrap();
    h.script("globalThis.request = __zguiServices.readTextAsset('test.txt'); request.promise.then(value => seen.push(value));");
    h.jobs();
    assert_eq!(h.script("seen.length"), "0");
    assert!(!h.work.has_deliverable_work());
    assert_eq!(h.runtime.next_deadline_us(), Some(0));
    assert!(h.runtime.release_due().unwrap());
    assert!(h.work.has_deliverable_work());
    assert_eq!(h.script("seen.length"), "0", "release_due must never enter JS");
    h.deliver();
    assert_eq!(h.script("seen.join(',')"), "completed");
    assert_eq!(h.work.counts().2, 0);
    h.runtime.assert_calls("assets", "readTextAsset", 1, None).unwrap();
    h.runtime.finalize().unwrap();
}

#[test]
fn events_precede_equal_deadline_requests_and_asset_delivery_keeps_release_order() {
    let mut h = Harness::new(definition(json!({"assets":assets(10)}), json!([
        {"atMs":10,"channel":"assets.state","value":{"assets":{"test.txt":{"error":{"code":"Io","message":"first state"}}}}},
        {"atMs":10,"channel":"assets.state","value":{"assets":{"test.txt":{}}}}
    ])));
    h.script("__zguiServices.readTextAsset('test.txt').promise.then(value => seen.push('first:' + value)); __zguiServices.readTextAsset('test.txt').promise.then(value => seen.push('second:' + value));");
    h.clock.advance(Duration::from_micros(9999));
    assert!(!h.runtime.release_due().unwrap());
    assert_eq!(h.runtime.next_deadline_us(), Some(10000));
    h.clock.advance(Duration::from_micros(1));
    h.runtime.release_due().unwrap(); h.deliver();
    assert_eq!(h.script("seen.join('|')"), "first:completed|second:completed");
    let trace = h.runtime.trace();
    let released: Vec<_> = trace["records"].as_array().unwrap().iter()
        .filter(|record| record["kind"] == "state" || record["kind"] == "response")
        .map(|record| record["kind"].as_str().unwrap()).collect();
    assert_eq!(released, ["state", "state", "response", "response"]);
    h.script("__zguiServices.readTextAsset('test.txt').promise.then(value => seen.push(value));");
    h.clock.advance(Duration::from_millis(10)); h.runtime.release_due().unwrap(); h.deliver();
    assert_eq!(h.script("seen[2]"), "completed", "same-time events retain source order");
    h.runtime.finalize().unwrap();
}

#[test]
fn cancellation_rechecks_selected_deliveries_and_cancels_future_work() {
    let mut h = Harness::new(definition(json!({"assets":assets(20)}), json!([])));
    h.script("globalThis.request = __zguiServices.readTextAsset('test.txt'); request.promise.then(value => seen.push(value), error => errors.push(error.code));");
    h.clock.advance(Duration::from_millis(20)); h.runtime.release_due().unwrap();
    let selected = h.work.next_completion().unwrap();
    h.script("request.cancel(); request.cancel();");
    h.engine.with_context(|ctx| h.work.settle(&ctx, selected)).unwrap();
    h.deliver();
    assert_eq!(h.script("seen.length + ':' + errors.join(',')"), "0:Cancelled");
    assert_eq!(h.work.counts(), (0, 0, 0));
    assert_eq!(h.runtime.next_deadline_us(), None);
    h.runtime.emit("assets.state", &json!({"assets":{"test.txt":{}}})).unwrap();
    h.deliver();
    assert_eq!(h.script("seen.length"), "0");
    h.runtime.finalize().unwrap();
}

#[test]
fn emitted_state_is_typed_and_errors_require_exact_acknowledgement() {
    let mut h = Harness::new(definition(json!({"assets":assets(0)}), json!([])));
    h.runtime.emit("assets.state", &json!({"assets":{"test.txt":{"error":{"code":"Io","message":"injected read failure"}}}})).unwrap();
    h.script("__zguiServices.readTextAsset('test.txt').promise.catch(error => errors.push(error.code));");
    h.runtime.release_due().unwrap(); h.deliver();
    assert_eq!(h.script("errors.join(',')"), "Io");
    assert_eq!(h.work.counts().2, 0);
    assert_eq!(h.runtime.finalize().unwrap_err().code, "E_FIXTURE_UNEXPECTED_ERROR");
    assert_eq!(h.runtime.assert_calls("assets", "readTextAsset", 2, Some("Io")).unwrap_err().code, "E_ASSERTION");
    assert_eq!(h.runtime.assert_calls("assets", "readTextAsset", 1, Some("TooLarge")).unwrap_err().code, "E_ASSERTION");
    h.runtime.assert_calls("assets", "readTextAsset", 1, None).unwrap();
    assert!(h.runtime.finalize().is_err(), "count-only assertions must not acknowledge injections");
    h.runtime.assert_calls("assets", "readTextAsset", 1, Some("Io")).unwrap();
    h.runtime.finalize().unwrap();
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
        (json!({"assets":assets(0)}), "__zguiServices.readTextAsset('../test.txt')", "E_FIXTURE_CONTRACT"),
        (json!({"assets":assets(0)}), "__zguiServices.readTextAsset(1)", "E_FIXTURE_CONTRACT"),
    ] {
        let mut h = Harness::new(definition(services, json!([])));
        assert_eq!(h.script(&format!("try {{ {source}; 'unexpected success' }} catch (error) {{ error.code }}")), expected);
        assert_eq!(h.work.contract_failure().unwrap().code, expected);
        assert_eq!(h.runtime.finalize().unwrap_err().code, expected);
    }
    let h = Harness::new(definition(json!({"assets":assets(0)}), json!([])));
    assert_eq!(h.runtime.emit("assets.state", &json!({"assets":{},"extra":true})).unwrap_err().code, "E_FIXTURE_CONTRACT");
    assert!(h.runtime.finalize().is_err());
}

#[test]
fn caught_blob_fixture_violations_are_sticky_and_never_allocate_a_handle() {
    for (blobs, path) in [
        (json!({}), "test.txt"),
        (json!({"undeclared.png":"test.txt"}), "undeclared.png"),
        (json!({"test.txt":"undeclared.png"}), "test.txt"),
    ] {
        let mut fixture = Harness::new(definition(json!({"images":{"blobs":blobs}}), json!([])));
        assert_eq!(fixture.script(&format!(
            "try {{ __zguiServices.registerImageAsset({path:?}); }} catch (e) {{ e.code }}"
        )), "E_FIXTURE_CONTRACT");
        assert_eq!(fixture.runtime.finalize().unwrap_err().code, "E_FIXTURE_CONTRACT");
        assert!(fixture.work.image_blobs().is_empty());
    }
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
    assert_eq!(h.work.counts(), (0, 0, 0));
    h.runtime.finalize().unwrap();
}

#[test]
fn fixture_validation_rejects_unknown_providers_fields_and_invalid_event_shapes() {
    let valid = json!({"schemaVersion":1,"id":"closed","services":{"assets":assets(0)},"events":[{"atMs":0,"channel":"assets.state","value":{"assets":{"test.txt":{}}}}]});
    let fixture: FixtureDefinition = serde_json::from_value(valid.clone()).unwrap();
    fixture.validate().unwrap();
    for changed in [
        { let mut v = valid.clone(); v["events"][0]["extra"] = json!(true); v },
        { let mut v = valid.clone(); v["events"][0]["channel"] = json!("assets.command"); v },
        { let mut v = valid.clone(); v["services"]["assets"]["socket"] = json!("/tmp/live"); v },
        { let mut v = valid.clone(); v["services"]["assets"] = Value::Null; v },
        { let mut v = valid.clone(); v["services"]["assets"]["provider"] = json!("live-assets"); v },
        { let mut v = valid.clone(); v["services"]["audio"] = json!({}); v },
    ] { assert!(serde_json::from_value::<FixtureDefinition>(changed).is_err()); }
    let mut invalid_path = fixture;
    let StateEvent::Assets(state) = &mut invalid_path.events[0].update else {
        panic!("expected an assets.state event in the validated fixture");
    };
    state.assets.insert("../test.txt".into(), AssetBehavior { error: None });
    assert!(invalid_path.validate().is_err());
}

#[test]
fn retiring_one_generation_cancels_only_its_socket_and_control_deliveries() {
    use host_services::unix_socket::{Event, Framing};

    let harness = Harness::new(definition(json!({
        "sockets": [
            {"path":"/fixture/socket", "framing":"line", "send":[], "events":[
                {"atMs":0,"kind":"message","text":"stale ready message"},
                {"atMs":10,"kind":"message","text":"stale future message"}
            ]},
            {"path":"/fixture/socket", "framing":"line", "send":[], "events":[
                {"atMs":10,"kind":"message","text":"candidate"}
            ]}
        ],
        "control": {}
    }), json!([])));
    let runtime = &harness.runtime;
    let first = runtime.registration_id().expect("allocate predecessor socket ID");
    let second = runtime.registration_id().expect("allocate candidate socket ID");
    let first_listener = runtime.registration_id().expect("allocate predecessor listener ID");
    let second_listener = runtime.registration_id().expect("allocate candidate listener ID");
    runtime.open_socket(1, first, "/fixture/socket", Framing::Line, None).expect("admit predecessor socket");
    runtime.open_socket(2, second, "/fixture/socket", Framing::Line, None).expect("admit candidate socket");
    runtime.register_control(1, first_listener).expect("register active listener");
    runtime.register_control(2, second_listener).expect("stage candidate listener");
    let inbound = json!({"args":["get state"],"reply":{"text":"state"}});
    runtime.emit("control.message", &inbound).expect("emit to active listener");
    let Some(ExternalDelivery::Control { listener, request, args }) = runtime.take_external(1) else {
        panic!("predecessor should receive control while candidate is staged");
    };
    assert_eq!(listener, first_listener, "only the oldest registered generation receives control");
    assert_eq!(args, ["get state"], "fixtures preserve exact argument boundaries");
    runtime.external_reply(request, Ok("state".into())).expect("match predecessor reply");
    runtime.release_due().expect("release current clock work");
    runtime.emit("control.message", &inbound).expect("queue control to predecessor before retirement");
    assert!(runtime.has_external(1), "predecessor has work selected for delivery");
    assert!(!runtime.has_external(2), "candidate cannot receive predecessor work");
    runtime.retire_external_generation(1);
    runtime.emit("control.message", &inbound).expect("emit to newly active candidate");
    let Some(ExternalDelivery::Control { listener, request, .. }) = runtime.take_external(2) else {
        panic!("candidate should receive control after predecessor retires");
    };
    assert_eq!(listener, second_listener, "promote the candidate without re-registering it");
    runtime.external_cancel(second_listener);
    runtime.external_reply(request, Ok("state".into())).expect("active callback can unsubscribe before replying");
    harness.clock.advance(Duration::from_millis(10));
    runtime.release_due().expect("release later work");
    assert!(runtime.take_external(1).is_none(), "neither ready nor future predecessor work survives retirement");
    assert!(matches!(runtime.take_external(2), Some(ExternalDelivery::Socket(id, Event::Message(message))) if id == second && message == "candidate"),
        "candidate receives only its scripted messages");
    runtime.assert_calls("control", "reply", 2, None).expect("assert whole-run reply history");
    runtime.finalize().expect("no unchecked replies or unmatched scripts");
}

#[test]
fn unmatched_commands_control_replies_and_missing_handlers_fail_the_fixture() {
    use host_services::command::Mode;

    let harness = Harness::new(definition(json!({
        "commands":[{"program":"fixture-command","args":["x","y"],"mode":"buffered","events":[]}]
    }), json!([])));
    let runtime = &harness.runtime;
    let id = runtime.registration_id().expect("allocate command ID");
    let error = runtime.spawn_command(1, id, "fixture-command", &["y".into(), "x".into()], Mode::Buffered)
        .expect_err("argument order is part of an exact command script");
    assert_eq!(runtime.contract_failure(), Some(error), "a rejected fixture call is sticky");

    for handler in [false, true] {
        let harness = Harness::new(definition(json!({"control": {}}), json!([])));
        let runtime = &harness.runtime;
        let inbound = json!({"args":[],"reply":{"error":"expected error"}});
        let error = if handler {
            let listener = runtime.registration_id().expect("allocate control ID");
            runtime.register_control(1, listener).expect("register handler before emission");
            runtime.emit("control.message", &inbound).expect("queue an inbound message");
            let Some(ExternalDelivery::Control { request, .. }) = runtime.take_external(1) else {
                panic!("emitted control fixture should reach its registered handler");
            };
            runtime.external_reply(request, Ok("expected error".into())).expect_err("text and error replies are not interchangeable")
        } else {
            runtime.emit("control.message", &inbound).expect_err("emission requires an active listener")
        };
        assert_eq!(runtime.contract_failure(), Some(error), "control failures remain sticky (handler={handler})");
    }
}

#[test]
fn streaming_command_fixture_events_must_be_single_lines_within_the_live_byte_limit() {
    let max_line = "é".repeat(32 * 1024);
    let oversized_line = format!("{max_line}x");
    for kind in ["stdout", "stderr"] {
        for (text, expected_error) in [
            (max_line.as_str(), None),
            ("one\ntwo", Some("LF")),
            (oversized_line.as_str(), Some("byte limit")),
        ] {
            let fixture: FixtureDefinition = serde_json::from_value(json!({
                "schemaVersion":1, "id":"stream-line", "events":[],
                "services":{"commands":[{"program":"fixture-command", "args":[],
                    "mode":"streaming", "events":[{"atMs":0, "kind":kind, "text":text}]
                }]}
            })).expect("decode streaming command fixture");
            match expected_error {
                None => fixture.validate().expect("a line of exactly 64 KiB is valid"),
                Some(message) => assert!(
                    fixture.validate().expect_err("invalid streaming line").contains(message),
                    "incorrect validation error for {kind}"
                ),
            }
        }
    }
}

#[test]
fn unsubscribing_rejects_queued_control_requests_but_preserves_the_active_reply() {
    let harness = Harness::new(definition(json!({"control":{}}), json!([])));
    let runtime = &harness.runtime;
    let listener = runtime.registration_id().expect("allocate listener ID");
    runtime.register_control(1, listener).expect("register control handler");
    runtime.emit("control.message", &json!({"args":["active"], "reply":{"text":"done"}}))
        .expect("enqueue active control request");
    let Some(ExternalDelivery::Control { request, .. }) = runtime.take_external(1) else {
        panic!("active control request was not delivered");
    };
    for arg in ["queued-one", "queued-two"] {
        runtime.emit("control.message", &json!({
            "args":[arg], "reply":{"error":"control handler unsubscribed"}
        })).expect("enqueue control request for the same handler");
    }

    runtime.external_cancel(listener);
    assert!(runtime.take_external(1).is_none(), "queued work must not run after unsubscribe");
    runtime.external_reply(request, Ok("done".into())).expect("reply from the active handler");
    runtime.assert_calls("control", "reply", 3, None).expect("count automatic and active replies");
    runtime.finalize().expect("every request received its expected reply");
}

#[test]
fn unsubscribe_rejection_must_match_the_queued_expected_reply() {
    let harness = Harness::new(definition(json!({"control":{}}), json!([])));
    let runtime = &harness.runtime;
    let listener = runtime.registration_id().expect("allocate listener ID");
    runtime.register_control(1, listener).expect("register control handler");
    runtime.emit("control.message", &json!({
        "args":[], "reply":{"text":"control handler unsubscribed"}
    })).expect("enqueue a request expecting a successful reply");

    runtime.external_cancel(listener);
    let error = runtime.finalize().expect_err("unsubscribe returns an error, not text");
    assert!(error.message.contains("received Err(\"control handler unsubscribed\")"), "{error:?}");
}

#[test]
fn explicit_and_scripted_close_reject_missing_socket_sends() {
    use host_services::unix_socket::Framing;

    for explicit in [false, true] {
        let harness = Harness::new(definition(json!({
            "sockets":[{"path":"/fixture/socket","framing":"line","send":["required"],"events":[{"atMs":0,"kind":"close"}]}]
        }), json!([])));
        let runtime = &harness.runtime;
        let id = runtime.registration_id().expect("allocate socket ID");
        runtime.open_socket(1, id, "/fixture/socket", Framing::Line, None).expect("admit socket script");
        if explicit {
            runtime.external_cancel(id);
        } else {
            runtime.release_due().expect("release close at current clock");
            assert!(runtime.take_external(1).is_none(), "an unmatched close must not be delivered");
        }
        let error = runtime.contract_failure().expect("missing expected send is sticky");
        assert_eq!(error.code, "E_FIXTURE_CONTRACT", "close checks outgoing messages (explicit={explicit})");
        assert!(error.message.contains("missing 1 expected sends"), "missing send count is diagnosable: {error}");
    }
}

#[test]
fn cancellation_before_deadline_removes_requests() {
    let mut h = Harness::new(definition(json!({"assets":assets(20)}), json!([])));
    h.script("globalThis.request = __zguiServices.readTextAsset('test.txt'); request.promise.catch(error => errors.push(error.code)); request.cancel();");
    assert_eq!(h.runtime.next_deadline_us(), None);
    h.clock.advance(Duration::from_millis(20));
    assert!(!h.runtime.release_due().unwrap());
    h.deliver();
    assert_eq!(h.script("seen.length + ':' + errors.join(',')"), "0:Cancelled");
    assert_eq!(h.work.counts(), (0, 0, 0));
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
        r#"{"schemaVersion":1,"id":"duplicate","services":{"assets":{"provider":"mock-text-assets-v1","latencyMs":0,"initialState":{"assets":{}}}},"events":[{"atMs":0,"channel":"assets.state","value":{"assets":{},"assets":{}}}]}"#,
        r#"{"schemaVersion":1,"id":"duplicate","services":{"images":{"files":{"file:///images/example.png":"test.txt","file:///images/example.png":"other.txt"}}},"events":[]}"#,
        r#"{"schemaVersion":1,"id":"duplicate","services":{"images":{"blobs":{"test.txt":"test.txt","test.txt":"other.txt"}}},"events":[]}"#,
    ] {
        assert!(serde_json::from_str::<FixtureDefinition>(input).is_err());
    }
}

#[test]
fn disposal_cancels_released_but_undelivered_errors_without_releasing_any_js_roots_late() {
    let mut provider = assets(0);
    provider["initialState"]["assets"]["test.txt"]["error"] = json!({"code":"Io","message":"cancelled response"});
    let mut h = Harness::new(definition(json!({"assets":provider}), json!([])));
    h.script("__zguiServices.readTextAsset('test.txt').promise.then(value => seen.push(value), error => errors.push(error.code));");
    h.runtime.release_due().unwrap();
    h.work.begin_stop(); h.work.finish_stop();
    h.deliver();
    assert_eq!(h.script("seen.length + errors.length"), "0");
    assert_eq!(h.work.counts(), (0, 0, 0));
    h.runtime.finalize().unwrap();
}
#[test]
fn retired_generation_preserves_fixture_ledger_events_and_new_registrations() {
    let mut h=Harness::new(definition(json!({"assets":assets(20)}),json!([
        {"atMs":10,"channel":"assets.state","value":{"assets":{"test.txt":{"error":{"code":"Io","message":"after resume"}}}}}
    ])));
    h.script("globalThis.oldRequest = __zguiServices.readTextAsset('test.txt'); oldRequest.promise.then(v => seen.push('old'), e => errors.push(e.code));");
    let old_ids: Vec<_> = h.runtime.state.borrow().calls.keys().copied().collect();
    h.work.finish_stop();
    assert_eq!(h.work.counts(),(0,0,0));
    let next_pkg = h._package.package();
    let capabilities = Arc::new(next_pkg.manifest.capabilities.clone());
    let work = Work::new(
        WorkSession { package: Some(next_pkg), capabilities, generation: 8, assets: AssetSource::Fixture(h.runtime.clone()), resolved: Default::default(), launch: Default::default() },
        HostHandle::from_rc(Rc::new(StubHost::default())), SurfaceId::new(8), Arc::new(NoWake),
    );
    h.engine.with_context(|ctx|work.install(&ctx)).unwrap();
    work.mounting(); work.mounted();
    h.work=work;
    h.script("__zguiServices.readTextAsset('test.txt').promise.then(v => seen.push('new'), e => errors.push(e.code));");
    let new_ids: Vec<_> = h.runtime.state.borrow().calls.keys().filter(|id|!old_ids.contains(id)).copied().collect();
    assert_eq!(new_ids.len(),1);
    assert!(new_ids.iter().all(|id|*id>*old_ids.last().unwrap()));
    h.clock.advance(Duration::from_millis(20));
    h.runtime.release_due().unwrap();
    h.deliver();
    assert_eq!(h.script("JSON.stringify(seen)"),r#"["new"]"#);
    assert_eq!(h.script("errors.length"),"0");
    h.runtime.assert_calls("assets","readTextAsset",2,None).unwrap();
    h.script("__zguiServices.readTextAsset('test.txt').promise.catch(e => errors.push(e.code));");
    h.clock.advance(Duration::from_millis(20)); h.runtime.release_due().unwrap(); h.deliver();
    assert_eq!(h.script("errors.join(',')"), "Io", "scheduled state survives session retirement");
    h.runtime.assert_calls("assets","readTextAsset",3,Some("Io")).unwrap();
    assert!(!h.runtime.state.borrow().disposed);
    h.work.finish_stop();
    h.runtime.dispose();
    assert!(h.runtime.state.borrow().disposed);
    assert!(h.runtime.state.borrow().scheduled.is_empty());
    assert!(h.runtime.state.borrow().deliveries.is_empty());
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
    rejects_arrays::<Services>(&["[]", "[null]"]);
    rejects_arrays::<ImageProvider>(&["[]", "[{},{}]"]);
    rejects_arrays::<TextProvider>(&[r#"["mock-text-assets-v1",0,{"assets":{}}]"#]);
    rejects_arrays::<AssetState>(&["[{}]"]);
    rejects_arrays::<AssetBehavior>(&["[]", "[null]"]);
    rejects_arrays::<InjectedError>(&[r#"["Io","failure"]"#]);
    rejects_arrays::<FixtureEvent>(&[r#"[0,"assets.state",{"assets":{}}]"#]);
    rejects_arrays::<StateEvent>(&[r#"["assets.state",{"assets":{}}]"#]);
    for input in [
        r#"{"schemaVersion":1,"id":"bad","services":[],"events":[]}"#,
        r#"{"schemaVersion":1,"id":"bad","services":{"assets":["mock-text-assets-v1",0,{"assets":{}}]},"events":[]}"#,
        r#"{"schemaVersion":1,"id":"bad","services":{"assets":{"provider":"mock-text-assets-v1","latencyMs":0,"initialState":[{}]}},"events":[]}"#,
        r#"{"schemaVersion":1,"id":"bad","services":{"assets":{"provider":"mock-text-assets-v1","latencyMs":0,"initialState":{"assets":{"test.txt":[]}}}},"events":[]}"#,
        r#"{"schemaVersion":1,"id":"bad","services":{"assets":{"provider":"mock-text-assets-v1","latencyMs":0,"initialState":{"assets":{"test.txt":{"error":["Io","failure"]}}}}},"events":[]}"#,
        r#"{"schemaVersion":1,"id":"bad","services":{"assets":{"provider":"mock-text-assets-v1","latencyMs":0,"initialState":{"assets":{}}}},"events":[{"atMs":0,"channel":"assets.state","value":[{}]}]}"#,
    ] {
        assert!(serde_json::from_str::<FixtureDefinition>(input).is_err(), "accepted nested positional object: {input}");
    }
}

#[test]
fn optional_error_nulls_match_schema_without_accepting_positional_error_objects() {
    for input in ["{}", r#"{"error":null}"#] {
        assert!(serde_json::from_str::<AssetBehavior>(input).unwrap().error.is_none());
    }
    assert!(serde_json::from_str::<Services>(r#"{"assets":null}"#).is_err());
    assert!(serde_json::from_str::<Services>(r#"{"images":null}"#).is_err());
}

#[test]
fn map_only_decoding_preserves_duplicate_field_rejection_at_nested_boundaries() {
    fn rejects<T: serde::de::DeserializeOwned>(input: &str) {
        assert!(serde_json::from_str::<T>(input).is_err(), "accepted duplicate field: {input}");
    }
    rejects::<Services>(r#"{"assets":{"provider":"mock-text-assets-v1","latencyMs":0,"initialState":{"assets":{}}},"assets":{"provider":"mock-text-assets-v1","latencyMs":0,"initialState":{"assets":{}}}}"#);
    rejects::<TextProvider>(r#"{"provider":"mock-text-assets-v1","latencyMs":0,"latencyMs":1,"initialState":{"assets":{}}}"#);
    rejects::<AssetBehavior>(r#"{"error":null,"error":null}"#);
    rejects::<InjectedError>(r#"{"code":"Io","code":"TooLarge","message":"failure"}"#);
}
