use super::*;
use rquickjs::function::Func;
use crate::test_package::Fixture;
use std::{sync::mpsc, time::Duration};
use zgui::view::{DomHandle, ElementName, HostHandle, stub::StubHost};
use zgui_dom::Document;
use zgui_view_dom::DocumentDom;

struct ChannelWaker(mpsc::Sender<WakeReason>);
impl Waker for ChannelWaker { fn wake(&self, reason: WakeReason) { let _ = self.0.send(reason); } }
struct Native {
    backend: Rc<DocumentDom>,
    host: Rc<StubHost>,
    region: MountRegion,
    waker: Arc<dyn Waker>,
    wakes: mpsc::Receiver<WakeReason>,
    executor: host_services::Executor,
}
impl Native {
    fn new() -> Self {
        let backend = Rc::new(DocumentDom::new(Rc::new(RefCell::new(Document::new()))));
        let dom = DomHandle::from_rc(backend.clone());
        let root = dom.create_element(ElementName::new("column"));
        dom.insert(backend.root_node(), root, None);
        let host = Rc::new(StubHost::default());
        let region = MountRegion { document: backend.document_id(), root, dom, host: HostHandle::from_rc(host.clone()) };
        let (sender, wakes) = mpsc::channel();
        Self { backend, host, region, waker: Arc::new(ChannelWaker(sender)), wakes, executor: host_services::Executor::new(2, 64).unwrap() }
    }
    fn prepare(&self, package: Rc<Package>, generation: u64) -> Result<Rc<Session>> {
        Session::prepare(self.region.clone(), Some(package), generation, self.waker.clone(), SurfaceId::new(7), self.executor.clone(), SessionPolicy::Live)
    }
    fn preview(&self, package: Rc<Package>, limits: RuntimeLimits, deadline: Rc<Cell<Instant>>) -> Result<Rc<Session>> {
        let environment = crate::preview::clock::Environment::new(Arc::new(zgui::platform::VirtualClock::new()), Some(0), 1);
        let fixtures = crate::work::fixture::FixtureRuntime::new(Default::default(), environment.clone());
        Session::prepare(self.region.clone(), Some(package), 1, self.waker.clone(), SurfaceId::new(7), self.executor.clone(), SessionPolicy::Preview { limits, deadline, environment, fixtures })
    }
    fn text(&self) -> String { self.region.dom.text_content(self.region.root) }
    fn completion(&self, session: &Session) -> host_services::Completion {
        self.wakes.recv_timeout(Duration::from_secs(5)).expect("asset completion did not wake owner");
        session.work.next_completion().expect("wake must follow publication")
    }
}

const SIMPLE: &str = r#"
export const hostApiVersion = 1;
export function mount(root) {
    const label = __zgui.createTextNode('active');
    __zgui.insertNode(root, label, null);
    return () => __zgui.clearRoot(root);
}
"#;

const RESOURCES: &str = r#"
export const hostApiVersion = 1;
export function mount(root) {
    const label = __zgui.createTextNode('pending');
    const control = __zgui.createElement('control');
    __zgui.insertNode(root, label, null);
    __zgui.insertNode(root, control, null);
    __zgui.setProperty(control, 'onClick', () => {});
    const timer = setTimeout(() => {}, 60000);
    const frame = requestAnimationFrame(() => {});
    const request = __zguiServices.readTextAsset('test.txt');
    request.promise.then(value => __zgui.replaceText(label, value));
    return () => {
        clearTimeout(timer);
        cancelAnimationFrame(frame);
        request.cancel();
        __zgui.setProperty(control, 'onClick', null);
        __zgui.clearRoot(root);
        if (typeof cleanupHook === 'function') cleanupHook();
        if (globalThis.throwCleanup) throw Error('cleanup failed');
    };
}
"#;

#[test]
fn candidate_preflight_rejects_deferred_side_effects_without_touching_active_tree() {
    let good = Fixture::new(SIMPLE, "text { color: red }");
    let native = Native::new();
    let active = native.prepare(good.package(), 1).unwrap();
    active.mount().unwrap();
    let counts = active.counts();
    let stylesheet = native.host.stylesheet("application").expect("active app owns its stylesheet");
    for (index, prefix) in [
        "throw Error('evaluation failed');",
        "__zgui.createTextNode('forbidden');",
        "setTimeout(() => {}, 0);",
        "requestAnimationFrame(() => {});",
        "__zguiServices.readTextAsset('test.txt');",
        "Promise.resolve().then(() => __zgui.createTextNode('forbidden'));",
        "Promise.resolve().then(() => setTimeout(() => {}, 0));",
        "queueMicrotask(function again() { queueMicrotask(again); });",
    ].into_iter().enumerate() {
        let candidate = Fixture::new(&format!("{prefix}\n{SIMPLE}"), "text { color: blue }");
        assert!(native.prepare(candidate.package(), index as u64 + 2).is_err());
        assert_eq!(native.text(), "active");
        assert_eq!(active.counts(), counts);
        assert_eq!(native.host.stylesheet("application").as_ref(), Some(&stylesheet));
    }
    active.shutdown();
}

#[test]
fn preflight_drains_pure_initialization_and_accepts_handled_rejections() {
    let fixture = Fixture::new(r#"
        export const hostApiVersion = 1;
        let initialized = false;
        Promise.reject(Error('handled')).catch(() => {});
        Promise.resolve().then(() => { initialized = true; });
        export function mount(root) {
            __zgui.insertNode(root, __zgui.createTextNode(initialized ? 'initialized' : 'too early'), null);
            return () => __zgui.clearRoot(root);
        }
    "#, "");
    let native = Native::new();
    let session = native.prepare(fixture.package(), 1).unwrap();
    session.mount().unwrap();
    assert_eq!(native.text(), "initialized");
    session.shutdown();
}

#[test]
fn retired_or_unmounted_candidates_cannot_remove_successor_styles() {
    let first = Fixture::new(SIMPLE, "text { color: red }");
    let second = Fixture::new(SIMPLE, "text { color: green }");
    let native = Native::new();
    let retired = native.prepare(first.package(), 1).unwrap();
    retired.mount().unwrap();
    retired.shutdown();
    let active = native.prepare(second.package(), 2).unwrap();
    active.mount().unwrap();
    let counts = active.counts();
    let stylesheet = native.host.stylesheet("application").expect("successor owns its stylesheet");
    assert!(active.mount().is_err());
    assert_eq!(active.counts(), counts);
    let unmounted = native.prepare(first.package(), 3).unwrap();
    drop(unmounted);
    retired.shutdown();
    drop(retired);
    assert_eq!(native.text(), "active");
    assert_eq!(native.host.stylesheet("application").as_ref(), Some(&stylesheet));
    active.shutdown();
    assert_eq!(active.counts(), ResourceCounts::default());
}

#[test]
fn repeated_sessions_release_native_and_js_resources_even_when_cleanup_throws() {
    let fixture = Fixture::new(RESOURCES, "text { color: red }");
    let package = fixture.package();
    let native = Native::new();
    let mut expected = None;
    for generation in 1..=32 {
        let session = native.prepare(package.clone(), generation).unwrap();
        session.engine.borrow_mut().as_mut().unwrap().evaluate_script("globalThis.throwCleanup = true; 'armed'").unwrap();
        session.mount().unwrap();
        let counts = session.counts();
        if let Some(expected) = &expected { assert_eq!(&counts, expected); } else { expected = Some(counts); }
        let nodes = native.backend.children(native.region.root);
        session.shutdown();
        session.shutdown();
        assert_eq!(session.counts(), ResourceCounts::default());
        let cleanup = session.take_cleanup_failure().expect("throwing cleanup must remain observable after repeated shutdown");
        assert_eq!(cleanup.phase, "cleanup");
        assert_eq!(cleanup.message, "cleanup failed");
        assert!(cleanup.stack.as_deref().unwrap().contains("app.mjs"));
        assert!(session.take_cleanup_failure().is_none());
        native.backend.end_frame();
        assert!(native.backend.children(native.region.root).is_empty());
        assert_eq!(native.backend.handler_count(), 0);
        assert!(native.region.dom.is_live(native.region.root));
        for node in nodes { assert!(!native.region.dom.is_live(node)); }
    }
}

#[test]
fn faulted_native_tree_still_runs_js_cleanup_once() {
    let fixture = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount(root) {
            __zgui.insertNode(root, __zgui.createTextNode('active'), null);
            return () => { cleanupHook(); __zgui.clearRoot(root); };
        }
        export function advance() { throw Error('event failed'); }
    "#, "");
    let native = Native::new();
    let session = native.prepare(fixture.package(), 1).unwrap();
    let cleanups = Rc::new(Cell::new(0));
    let called = cleanups.clone();
    session.engine.borrow_mut().as_mut().unwrap().with_context(|ctx| {
        ctx.globals().set("cleanupHook", Func::from(move || called.set(called.get() + 1)))
    }).unwrap();
    session.mount().unwrap();
    assert!(session.advance(false).is_err());
    assert!(session.bridge.is_disposed());
    session.shutdown();
    session.shutdown();
    assert_eq!(cleanups.get(), 1);
    assert_eq!(session.counts(), ResourceCounts::default());
}

#[test]
fn completion_retained_across_reload_cannot_settle_successor_request() {
    let fixture = Fixture::new(RESOURCES, "");
    let native = Native::new();
    let first = native.prepare(fixture.package(), 1).unwrap();
    first.mount().unwrap();
    let late = native.completion(&first);
    first.shutdown();
    let second = native.prepare(fixture.package(), 2).unwrap();
    second.mount().unwrap();
    let current = native.completion(&second);
    assert_eq!(late.request_id, current.request_id);
    assert_ne!(late.generation, current.generation);
    second.bridge.run_turn(|| second.engine.borrow_mut().as_mut().unwrap().with_context(|ctx| second.work.settle(&ctx, late))).unwrap();
    second.checkpoint().unwrap();
    assert_eq!(native.text(), "pending");
    assert_eq!(second.counts().requests, 1);
    second.bridge.run_turn(|| second.engine.borrow_mut().as_mut().unwrap().with_context(|ctx| second.work.settle(&ctx, current))).unwrap();
    second.checkpoint().unwrap();
    assert_eq!(native.text(), "completed");
    assert_eq!(second.counts().requests, 0);
    second.shutdown();
    assert_eq!(second.counts(), ResourceCounts::default());
}

#[test]
fn promise_cancellation_runs_between_ready_service_completions() {
    let fixture = Fixture::new(SIMPLE, "");
    let native = Native::new();
    let session = native.prepare(fixture.package(), 1).unwrap();
    session.mount().unwrap();
    session.engine.borrow_mut().as_mut().unwrap().with_context(|ctx| ctx.eval::<(), _>(r#"
        globalThis.resolved = 0;
        const requests = Array.from({ length: 4 }, () => __zguiServices.readTextAsset('test.txt'));
        for (const request of requests) request.promise.then(() => {
            resolved++;
            for (const other of requests) other.cancel();
        }, () => {});
    "#)).unwrap();
    // All four completions are already ready before owner scheduling starts.
    for _ in 0..4 { native.wakes.recv_timeout(Duration::from_secs(5)).unwrap(); }
    for _ in 0..16 {
        session.checkpoint().unwrap();
        if session.counts().requests == 0 && !session.engine.borrow().as_ref().unwrap().is_job_pending() { break; }
    }
    let resolved = session.engine.borrow_mut().as_mut().unwrap()
        .with_context(|ctx| ctx.eval::<u32, _>("resolved")).unwrap();
    assert_eq!(resolved, 1, "a ready completion queue starved its Promise cancellation");
    session.shutdown();
    assert_eq!(session.counts(), ResourceCounts::default());
}

#[test]
fn preview_caught_fixture_violation_fails_preflight_and_mount() {
    let native = Native::new();
    for preflight in [true, false] {
        let attempt = "try { __zguiServices.watchNiri(() => {}, () => {}); } catch (error) { if (error.code !== 'E_FIXTURE_MISSING_SERVICE') throw error; }";
        let source = if preflight {
            format!("{attempt}\n{SIMPLE}")
        } else {
            format!("export const hostApiVersion = 1; export function mount() {{ {attempt} return () => {{}}; }}")
        };
        let fixture = Fixture::new(&source, "");
        let prepared = native.preview(fixture.package(), RuntimeLimits::default(), Rc::new(Cell::new(Instant::now() + Duration::from_secs(5))));
        let error = if preflight {
            prepared.err().expect("caught preflight violation must reject candidate")
        } else {
            let session = prepared.unwrap();
            let error = session.mount().unwrap_err();
            assert_eq!(session.counts().subscriptions, 0);
            assert_eq!(session.contract_failure().unwrap().code, "E_FIXTURE_MISSING_SERVICE");
            assert!(session.checkpoint().is_err());
            session.shutdown();
            assert_eq!(session.contract_failure().unwrap().code, "E_FIXTURE_MISSING_SERVICE");
            error
        };
        let failure = error.downcast_ref::<ContractFailure>().expect("fixture failure must retain its type");
        assert_eq!(failure.code, "E_FIXTURE_MISSING_SERVICE");
        assert!(failure.message.contains("watchNiri"));
    }
}

#[test]
fn preview_rejections_reduce_handled_updates_but_fail_unhandled_and_eviction() {
    for (source, capacity, expected) in [
        ("const p = Promise.reject(Error('handled')); p.catch(() => {});", 256, None),
        ("Promise.reject(Error('unhandled sentinel'));", 256, Some("unhandled sentinel")),
        ("Promise.reject(Error('first')); Promise.reject(Error('second'));", 1, Some("rejection tracking limit exceeded")),
    ] {
        let fixture = Fixture::new(&format!("export const hostApiVersion = 1; export function mount() {{ {source} return () => {{}}; }}"), "");
        let native = Native::new();
        let session = native.preview(fixture.package(), RuntimeLimits { max_tracked_rejections: capacity, ..RuntimeLimits::default() }, Rc::new(Cell::new(Instant::now() + Duration::from_secs(5)))).unwrap();
        session.mount().unwrap();
        match expected {
            Some(message) => {
                let error = session.checkpoint().unwrap_err();
                assert_eq!(error.downcast_ref::<js_runtime::Diagnostic>().unwrap().message, message);
            }
            None => { session.checkpoint().unwrap(); }
        }
        session.shutdown();
        assert_eq!(session.counts(), ResourceCounts::default());
    }
}

#[test]
fn frame_checkpoint_retains_original_job_diagnostic() {
    let fixture = Fixture::new(SIMPLE, "");
    let native = Native::new();
    let session = native.preview(fixture.package(), RuntimeLimits::default(), Rc::new(Cell::new(Instant::now() + Duration::from_secs(5)))).unwrap();
    session.mount().unwrap();
    session.engine.borrow_mut().as_mut().unwrap().with_context(|ctx| {
        ctx.eval::<Function, _>("function originalJob() { throw Error('original job sentinel'); } originalJob")?.defer(())
    }).unwrap();
    session.frame_checkpoint();
    let error = session.checkpoint().unwrap_err();
    let diagnostic = error.downcast_ref::<js_runtime::Diagnostic>().expect("frame checkpoint must not stringify a job exception");
    assert_eq!(diagnostic.message, "original job sentinel");
    assert_eq!(diagnostic.phase, "pending job");
    assert!(diagnostic.stack.as_deref().unwrap().contains("originalJob"));
    session.shutdown();
}

#[test]
fn preview_cleanup_retains_deadline_failure_and_releases_every_resource() {
    let fixture = Fixture::new(SIMPLE, "");
    let native = Native::new();
    let deadline = Rc::new(Cell::new(Instant::now() + Duration::from_secs(5)));
    let session = native.preview(fixture.package(), RuntimeLimits::default(), deadline.clone()).unwrap();
    session.mount().unwrap();
    deadline.set(Instant::now() - Duration::from_millis(1));
    session.shutdown();
    session.shutdown();
    let diagnostic = session.take_cleanup_failure().expect("expired cleanup may not pass silently");
    assert_eq!(diagnostic.kind, js_runtime::DiagnosticKind::BudgetExceeded);
    assert_eq!(diagnostic.phase, "cleanup");
    assert_eq!(session.counts(), ResourceCounts::default());
    assert!(session.take_cleanup_failure().is_none());
}

#[test]
fn preview_uncaught_fixture_violation_retains_its_category_across_entries() {
    for statement in [
        "__zguiServices.watchNiri(() => {}, () => {});",
        "Promise.resolve().then(() => __zguiServices.watchNiri(() => {}, () => {}));",
    ] {
        let native = Native::new();
        let fixture = Fixture::new(&format!("export const hostApiVersion=1; export function mount() {{ {statement} return () => {{}}; }}"), "");
        let session = native.preview(fixture.package(), RuntimeLimits::default(), Rc::new(Cell::new(Instant::now()+Duration::from_secs(5)))).unwrap();
        let failure = session.mount().and_then(|()|session.checkpoint().map(|_|())).unwrap_err();
        assert_eq!(failure.downcast_ref::<ContractFailure>().unwrap().code,"E_FIXTURE_MISSING_SERVICE");
        session.shutdown();
        assert_eq!(session.counts(),ResourceCounts::default());
    }
}

#[test]
fn rejected_mount_keeps_its_returned_disposer_for_cleanup() {
    let native = Native::new();
    let fixture = Fixture::new("export const hostApiVersion=1; export function mount() { try { __zguiServices.watchNiri(() => {}, () => {}); } catch {} return () => recordCleanup(); }", "");
    let session = native.preview(fixture.package(), RuntimeLimits::default(), Rc::new(Cell::new(Instant::now()+Duration::from_secs(5)))).unwrap();
    let called = Rc::new(Cell::new(0));
    let count = called.clone();
    session.engine.borrow_mut().as_mut().unwrap().with_context(|ctx| {
        ctx.globals().set("recordCleanup",Function::new(ctx.clone(),move ||count.set(count.get()+1))?)
    }).unwrap();
    assert!(session.mount().is_err());
    session.shutdown();
    session.shutdown();
    assert_eq!(called.get(),1);
    assert_eq!(session.counts(),ResourceCounts::default());
}

#[test]
fn preview_installs_clock_before_module_evaluation_and_settles_fixture_through_owner_checkpoint() {
    let fixture = Fixture::new(r#"
        export const hostApiVersion = 1;
        const evaluated = Date.now() + ':' + performance.now();
        export function mount(root) {
            const label = __zgui.createTextNode(evaluated);
            __zgui.insertNode(root, label, null);
            const request = __zguiServices.readTextAsset('test.txt');
            request.promise.then(value => __zgui.replaceText(label, value + ':' + Date.now()));
            return () => { request.cancel(); __zgui.clearRoot(root); };
        }
    "#, "");
    let native = Native::new();
    let clock = Arc::new(zgui::platform::VirtualClock::new());
    clock.advance(Duration::from_millis(17));
    let environment = crate::preview::clock::Environment::new(clock.clone(), Some(1234), 7);
    let definition: crate::work::fixture::FixtureDefinition = serde_json::from_value(serde_json::json!({
        "schemaVersion":1, "id":"session-service", "services": {
            "assets":{"provider":"mock-text-assets-v1","latencyMs":5,"initialState":{"assets":{"test.txt":{}}}}
        }, "events":[]
    })).unwrap();
    let fixtures = crate::work::fixture::FixtureRuntime::new(definition, environment.clone());
    let session = Session::prepare(native.region.clone(), Some(fixture.package()), 1, native.waker.clone(),
        SurfaceId::new(7), native.executor.clone(), SessionPolicy::Preview {
            limits: RuntimeLimits::default(), deadline: Rc::new(Cell::new(Instant::now() + Duration::from_secs(5))),
            environment, fixtures: fixtures.clone(),
        }).unwrap();
    session.mount().unwrap();
    assert_eq!(native.text(), "1251:17");
    assert_eq!(fixtures.next_deadline_us(), Some(22000));
    assert!(!fixtures.release_due().unwrap());
    session.checkpoint().unwrap();
    assert_eq!(native.text(), "1251:17");
    clock.advance(Duration::from_millis(5));
    fixtures.release_due().unwrap();
    session.checkpoint().unwrap();
    assert_eq!(native.text(), "completed:1256");
    fixtures.assert_calls("assets", "readTextAsset", 1, None).unwrap();
    fixtures.finalize().unwrap();
    session.shutdown();
    assert_eq!(session.counts(), ResourceCounts::default());
    assert!(fixtures.trace()["records"].as_array().unwrap().iter().any(|record| record["kind"] == "disposal"));
}
