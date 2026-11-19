use super::*;
use rquickjs::function::Func;
use crate::test_package::Fixture;
use std::{sync::mpsc, time::Duration};
use zgui::geom::{DevicePx, Size};
use zgui::view::{DomHandle, ElementName, HostHandle, ObservedValue, stub::StubHost};
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
        let region = MountRegion { document: backend.document_id(), root, dom, host: HostHandle::from_rc(host.clone()), fonts: Fonts::shipped_only(), root_anchor: Rc::new(Cell::new(root)) };
        let (sender, wakes) = mpsc::channel();
        Self { backend, host, region, waker: Arc::new(ChannelWaker(sender)), wakes, executor: host_services::Executor::new(2, 64).unwrap() }
    }
    fn prepare(&self, package: Rc<Package>, generation: u64) -> Result<Rc<Session>> {
        Session::prepare(self.region.clone(), SessionModule::package(package), generation, self.waker.clone(), SurfaceId::new(7), SessionPolicy::live(self.executor.clone()))
    }
    fn preview(&self, package: Rc<Package>, limits: RuntimeLimits, deadline: Rc<Cell<Instant>>) -> Result<Rc<Session>> {
        let environment = Environment::new(Arc::new(zgui::platform::VirtualClock::new()), Some(0), 1);
        let fixtures = crate::assets::fixture::FixtureRuntime::new(Default::default(), environment.clone());
        Session::prepare(self.region.clone(), SessionModule::package(package), 1, self.waker.clone(), SurfaceId::new(7), SessionPolicy::preview(limits, deadline, environment, fixtures))
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
    let stylesheet = native.host.stylesheet("application:0").expect("active app owns its stylesheet");
    for (index, prefix) in [
        "throw Error('evaluation failed');",
        "__zgui.createTextNode('forbidden');",
        "setTimeout(() => {}, 0);",
        "requestAnimationFrame(() => {});",
        "__zguiServices.readTextAsset('test.txt');",
        "__zgui.observeGeometry(mountRoot, 'borderSize', () => {});",
        "Promise.resolve().then(() => __zgui.createTextNode('forbidden'));",
        "Promise.resolve().then(() => __zgui.observeGeometry(mountRoot, 'borderSize', () => {}));",
        "Promise.resolve().then(() => setTimeout(() => {}, 0));",
        "queueMicrotask(function again() { queueMicrotask(again); });",
    ].into_iter().enumerate() {
        let candidate = Fixture::new(&format!("{prefix}\n{SIMPLE}"), "text { color: blue }");
        assert!(native.prepare(candidate.package(), index as u64 + 2).is_err());
        assert_eq!(native.text(), "active");
        assert_eq!(active.counts(), counts);
        assert_eq!(native.host.stylesheet("application:0").as_ref(), Some(&stylesheet));
    }
    active.retire();
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
    session.retire();
}

#[test]
fn retired_or_unmounted_candidates_cannot_remove_successor_styles() {
    let first = Fixture::new(SIMPLE, "text { color: red }");
    let second = Fixture::new(SIMPLE, "text { color: green }");
    let native = Native::new();
    let retired = native.prepare(first.package(), 1).unwrap();
    retired.mount().unwrap();
    retired.retire();
    let active = native.prepare(second.package(), 2).unwrap();
    active.mount().unwrap();
    let counts = active.counts();
    let stylesheet = native.host.stylesheet("application:0").expect("successor owns its stylesheet");
    assert!(active.mount().is_err());
    assert_eq!(active.counts(), counts);
    let unmounted = native.prepare(first.package(), 3).unwrap();
    drop(unmounted);
    retired.retire();
    drop(retired);
    assert_eq!(native.text(), "active");
    assert_eq!(native.host.stylesheet("application:0").as_ref(), Some(&stylesheet));
    active.retire();
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
        if let Some(expected) = &expected { assert_eq!(&counts, expected); } else { expected = Some(counts.clone()); }
        let nodes = native.backend.children(native.region.root);
        let first = session.retire();
        let second = session.retire();
        assert_eq!(first.generation, generation);
        assert_eq!(first.before, counts);
        assert_eq!(second.before, first.before);
        assert_eq!(first.counts, ResourceCounts::default());
        assert_eq!(second.counts, ResourceCounts::default());
        assert!(first.telemetry.is_some());
        assert_eq!(session.counts(), ResourceCounts::default());
        let Some(RetirementFailure::Cleanup(cleanup)) = &first.failure else {
            panic!("throwing cleanup must remain observable in retirement report");
        };
        assert_eq!(first.failure, second.failure);
        assert_eq!(cleanup.phase, "cleanup");
        assert_eq!(cleanup.message, "cleanup failed");
        assert!(cleanup.stack.as_deref().unwrap().contains("app.mjs"));
        native.backend.end_frame();
        assert!(native.backend.children(native.region.root).is_empty());
        assert_eq!(native.backend.handler_count(), 0);
        assert!(native.region.dom.is_live(native.region.root));
        for node in nodes { assert!(!native.region.dom.is_live(node)); }
    }
}

#[test]
fn host_bytes_register_without_a_package_and_retire_with_the_session() {
    let native = Native::new();
    let bytes: Arc<[u8]> = Arc::from(&b"transport image bytes"[..]);
    let held = Arc::downgrade(&bytes);
    let session = Session::prepare(
        native.region.clone(),
        SessionModule { name: "transport-test".into(), source: SIMPLE.into(), styles: Vec::new(), package: None },
        1,
        native.waker.clone(),
        SurfaceId::new(7),
        SessionPolicy::live(native.executor.clone()),
    ).unwrap();
    assert!(session.work.register_image_bytes(Arc::clone(&bytes)).is_err());
    session.mount().unwrap();
    let uri = session.work.register_image_bytes(bytes).unwrap();
    assert!(uri.starts_with("zgui-blob://"), "{uri}");
    assert!(held.upgrade().is_some(), "registry must retain the supplied buffer");
    assert_eq!(session.counts().image_handles, 1);
    assert_eq!(session.retire().counts.image_handles, 1);
    assert!(held.upgrade().is_none(), "retirement must free the transport buffer");
    assert!(session.work.register_image_bytes(Arc::from(&b"late bytes"[..])).is_err());
}

#[test]
fn image_handles_release_during_cleanup_and_remaining_handles_are_reported_then_cleared() {
    let fixture = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount() {
            const register = __zguiServices.registerImageAsset;
            for (const args of [[], ['test.txt', undefined], ['test.txt', ...Array(1024)]]) {
                try {
                    register(...args);
                    throw Error('invalid arity was accepted');
                } catch (error) {
                    if (!(error instanceof TypeError)) throw error;
                }
            }
            const release = __zguiServices.registerImageAsset('test.txt');
            const leaked = __zguiServices.registerImageAsset('test.txt');
            if (!leaked.src.startsWith('zgui-blob://') || leaked.src === release.src) {
                throw Error('registration must return a distinct opaque reference');
            }
            return () => { release.release(); release.release(); };
        }
    "#, "");
    let native = Native::new();
    let session = native.prepare(fixture.package(), 1).unwrap();
    session.mount().unwrap();
    assert_eq!(session.counts().image_handles, 2);
    let report = session.retire();
    assert!(report.failure.is_none());
    assert_eq!(report.before.image_handles, 2);
    assert_eq!(report.counts, ResourceCounts { image_handles: 1, ..Default::default() });
    assert_eq!(session.counts(), ResourceCounts::default());
    assert_eq!(session.retire().counts, report.counts);
}

#[test]
fn releasing_a_mounted_image_handle_wakes_its_surface_once() {
    let fixture = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount() {
            const image = __zguiServices.registerImageAsset('test.txt');
            globalThis.releaseImage = image.release;
            return () => {};
        }
    "#, "");
    let native = Native::new();
    let session = native.prepare(fixture.package(), 1).unwrap();
    session.mount().unwrap();
    session
        .engine
        .borrow_mut()
        .as_mut()
        .unwrap()
        .evaluate_script("releaseImage(); releaseImage(); 'released'")
        .unwrap();
    let WakeReason::ReactiveWork { surfaces } = native.wakes.try_recv().unwrap() else {
        panic!("image release must wake the owner surface");
    };
    assert_eq!(surfaces.as_ref(), &[session.surface()]);
    assert!(native.wakes.try_recv().is_err(), "repeated release requested another frame");
    assert_eq!(session.counts().image_handles, 0);
}

#[test]
fn external_services_deny_zero_capability_packages_before_opening_any_endpoint() {
    let fixture = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount() {
            const services = __zguiServices;
            for (const call of [
                () => services.connectUnixSocket('/tmp/no-such-zgui-service.sock', {framing: 'line', onMessage() {}}),
                () => services.spawnCommand('/bin/true', []),
                () => services.onControlMessage(() => 'not authorized')
            ]) {
                try { call(); }
                catch (error) {
                    if (error.code === 'PermissionDenied') continue;
                    throw error;
                }
                throw Error('external service accepted a package without its capability');
            }
            return () => {};
        }
    "#, "");
    let native = Native::new();
    let session = native.prepare(fixture.package(), 1).expect("load zero-capability package");
    session.mount().expect("all three service calls must deny undeclared authority");
    assert_eq!(session.retire().counts, ResourceCounts::default(), "denials must not acquire resources");
}

#[test]
fn command_argv_is_bounded_snapshotted_and_passed_literally() {
    let fixture = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount(root) {
            const spawn = __zguiServices.spawnCommand;
            const expectError = (f, code) => {
                try { f(); } catch (error) { if (error.code === code) return; throw error; }
                throw Error(`expected ${code}`);
            };
            expectError(() => spawn('/usr/bin/printf', []), 'PermissionDenied');
            const tooLong = Array(65);
            Object.defineProperty(tooLong, 0, {get() { throw Error('read beyond admission'); }});
            expectError(() => spawn('printf', tooLong), 'TooLarge');
            expectError(() => spawn('printf', ['\ud800']), 'InvalidUtf8');
            const args = ['%s'];
            Object.defineProperty(args, 1, {get() {
                args.length = 65;
                return '$(printf wrong);*';
            }});
            const label = __zgui.createTextNode('pending');
            __zgui.insertNode(root, label, null);
            const request = spawn('printf', args);
            request.promise.then(({status, stdout, stderr}) =>
                __zgui.replaceText(label, `${status}:${stdout}:${stderr}`));
            return () => { request.cancel(); __zgui.clearRoot(root); };
        }
    "#, "").with_capabilities(crate::package::Capabilities { commands: vec!["printf".into()], ..Default::default() });
    let native = Native::new();
    let session = native.prepare(fixture.package(), 1).expect("load command package");
    session.mount().expect("mount command consumer");
    let deadline = Instant::now() + Duration::from_secs(5);
    while native.text() == "pending" {
        native.wakes.recv_timeout(deadline.saturating_duration_since(Instant::now())).expect("command must complete and wake the owner");
        session.checkpoint().expect("run command delivery turn");
    }
    assert_eq!(native.text(), "0:$(printf wrong);*:", "output must contain the literal argument, not shell output");
    assert_eq!(session.retire().counts, ResourceCounts::default(), "settled command must release its resources");
}

fn accept_service_peer(listener: &std::os::unix::net::UnixListener) -> std::os::unix::net::UnixStream {
    listener.set_nonblocking(true).expect("bound the accept wait");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((peer, _)) => return peer,
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("socket worker did not connect: {error}"),
        }
    }
}

#[test]
fn live_length_framing_routes_svg_as_an_image_and_rejects_invalid_text() {
    use std::{io::Write, os::unix::net::UnixListener};
    let fixture = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount(root) {
            const label = __zgui.createTextNode('pending');
            __zgui.insertNode(root, label, null);
            const seen = [];
            let image;
            const socket = __zguiServices.connectUnixSocket(socketPath, {
                framing: 'length',
                onMessage(text) { seen.push(`message:${text}`); },
                onBinaryImage(handle) { image = handle; seen.push(handle.src); },
                onClose(error) {
                    __zgui.replaceText(label, [error?.code, ...seen].join('|'));
                }
            });
            return () => { socket.close(); image?.release(); __zgui.clearRoot(root); };
        }
    "#, "");
    let path = fixture.package().directory.join("service.sock");
    let socket_path = path.to_str().expect("UTF-8 test socket path");
    let fixture = fixture.with_capabilities(crate::package::Capabilities {
        unix_sockets: vec![socket_path.into()],
        ..Default::default()
    });
    let native = Native::new();
    let session = native.prepare(fixture.package(), 1).expect("prepare socket app");
    let listener = UnixListener::bind(&path).expect("bind private test socket");
    session.engine.borrow_mut().as_mut().expect("prepared engine").with_context(|ctx| {
        ctx.globals().set("socketPath", socket_path)
    }).expect("supply private socket path");
    session.mount().expect("mount socket app");
    let mut peer = accept_service_peer(&listener);
    peer.set_write_timeout(Some(Duration::from_secs(5))).expect("bound peer write");
    peer.write_all(b"\x00\x00\x00\x07\x01<svg/>\x00\x00\x00\x02\x00\xff")
        .expect("send one tagged SVG image and a malformed text frame");
    let deadline = Instant::now() + Duration::from_secs(5);
    while native.text() == "pending" {
        native.wakes.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("worker must wake for socket deliveries");
        session.checkpoint().expect("dispatch socket deliveries to JS");
    }
    let text = native.text();
    let Some((error, src)) = text.split_once('|') else {
        panic!("expected protocol error plus one image handle, got {text:?}");
    };
    assert_eq!(error, "InvalidUtf8", "malformed text closes the connection");
    assert!(src.starts_with("zgui-blob://") && !src.contains('|'), "expected one image callback and no text callbacks, got {text:?}");
    assert_eq!(session.counts().image_handles, 1, "invalid text must not allocate an image handle");
    assert_eq!(session.retire().counts, ResourceCounts::default(), "disposal releases the image");
}

#[test]
fn session_samples_external_leaks_then_closes_their_endpoints() {
    use std::{io::Read, os::unix::net::UnixListener};
    let fixture = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount() {
            __zguiServices.connectUnixSocket(socketPath, { framing: 'line', onMessage() {} }).send('ready');
            __zguiServices.spawnCommand('sleep', ['60'], {stream: true});
            const first = __zguiServices.onControlMessage(() => 'first');
            first();
            __zguiServices.onControlMessage(() => 'leaked');
            return () => {};
        }
    "#, "");
    let path = fixture.package().directory.join("service.sock");
    let fixture = fixture.with_capabilities(crate::package::Capabilities {
        unix_sockets: vec![path.to_str().expect("UTF-8 test socket path").into()],
        commands: vec!["sleep".into()],
        control_channel: true,
        ..Default::default()
    });
    let native = Native::new();
    let session = native.prepare(fixture.package(), 1).expect("load package with three external capabilities");
    let listener = UnixListener::bind(&path).expect("bind private test socket");
    session.engine.borrow_mut().as_mut().expect("prepared engine").with_context(|ctx| {
        ctx.globals().set("socketPath", path.to_str().expect("UTF-8 socket path"))
    }).expect("supply private socket path");
    session.mount().expect("mount external services");
    let mut peer = accept_service_peer(&listener);
    peer.set_read_timeout(Some(Duration::from_secs(5))).expect("bound peer read");
    let mut message = [0; 6];
    peer.read_exact(&mut message).expect("drain socket send before retirement");
    assert_eq!(&message, b"ready\n", "live socket must apply line framing");
    let report = session.retire();
    assert_eq!(report.counts, ResourceCounts {
        unix_sockets: 1, child_processes: 1, control_listeners: 1, ..Default::default()
    }, "the report must retain all three leaked registrations");
    assert_eq!(peer.read(&mut message).expect("observe socket cleanup"), 0, "forced cleanup must close the peer");
    assert_eq!(session.counts(), ResourceCounts::default(), "forced cleanup must release retained resources");
}

#[test]
fn live_control_request_updates_the_tree_and_replies_from_a_bridge_turn() {
    use std::os::unix::net::UnixListener;
    let fixture = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount(root) {
            const label = __zgui.createTextNode('waiting');
            __zgui.insertNode(root, label, null);
            const unsubscribe = __zguiServices.onControlMessage(args => {
                __zgui.replaceText(label, `${args[0]}:${args[1]}`);
                return { received: args };
            });
            return () => { unsubscribe(); __zgui.clearRoot(root); };
        }
    "#, "").with_capabilities(crate::package::Capabilities { control_channel: true, ..Default::default() });
    let package = fixture.package();
    let path = package.directory.join("control.sock");
    let native = Native::new();
    let session = native.prepare(package, 1).expect("load control-capable package");
    session.mount().expect("register control handler");
    let (wake, queued) = mpsc::channel();
    let listener = host_services::control::Listener::start(
        UnixListener::bind(&path).expect("bind test control socket"),
        |frame| serde_json::from_str(frame).map_err(|error| error.to_string()),
        |response| Ok(response.unwrap_or_else(|error| error).as_bytes().to_vec()),
        move || { let _ = wake.send(()); },
    ).expect("start test control transport");
    let peer = std::thread::spawn(move || {
        let client = host_services::control::connect(&path).expect("connect to test control socket");
        host_services::control::exchange(client, br#"["hello world","--flag"]"#)
            .expect("exchange control request and structured reply")
    });
    queued.recv_timeout(Duration::from_secs(5)).expect("control transport must notify the owner");
    session.offer_control(listener.try_recv().expect("published inbound request"));
    assert_eq!(native.text(), "waiting", "inbound admission must not execute the handler");
    session.checkpoint().expect("deliver control request in a bounded Bridge turn");
    assert_eq!(native.text(), "hello world:--flag", "the handler must receive argv separately and update the tree");
    let reply = peer.join().expect("join socket peer");
    assert_eq!(reply, br#"{"received":["hello world","--flag"]}"#, "the transport must return the handler's serialized reply");
    assert_eq!(session.retire().counts, ResourceCounts::default(), "unsubscribe must release the listener registration");
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
    assert!(session.invoke("advance").is_err());
    assert!(session.bridge().is_disposed());
    session.retire();
    session.retire();
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
    first.retire();
    let second = native.prepare(fixture.package(), 2).unwrap();
    second.mount().unwrap();
    let current = native.completion(&second);
    assert_eq!(late.request_id, current.request_id);
    assert_ne!(late.generation, current.generation);
    second.bridge().run_turn(second.engine.borrow_mut().as_mut().unwrap().turn(), |_, js| js.with_context(|ctx| second.work.settle(&ctx, late))).unwrap();
    second.checkpoint().unwrap();
    assert_eq!(native.text(), "pending");
    assert_eq!(second.counts().requests, 1);
    second.bridge().run_turn(second.engine.borrow_mut().as_mut().unwrap().turn(), |_, js| js.with_context(|ctx| second.work.settle(&ctx, current))).unwrap();
    second.checkpoint().unwrap();
    assert_eq!(native.text(), "completed");
    assert_eq!(second.counts().requests, 0);
    second.retire();
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
    session.retire();
    assert_eq!(session.counts(), ResourceCounts::default());
}

#[test]
fn self_scheduling_preflight_jobs_are_budget_diagnostics_not_invariants() {
    let native = Native::new();
    let fixture = Fixture::new(&format!("queueMicrotask(function again() {{ queueMicrotask(again); }});\n{SIMPLE}"), "");
    let error = native.preview(fixture.package(),
        RuntimeLimits { max_jobs_per_pump: 1, ..RuntimeLimits::default() },
        Rc::new(Cell::new(Instant::now() + Duration::from_secs(5)))).err().expect("self-scheduling preflight must fail");
    let SessionError::Diagnostic(diagnostic) = error else { panic!("expected budget diagnostic, got {error:?}") };
    assert_eq!(diagnostic.kind, js_runtime::DiagnosticKind::BudgetExceeded);
    assert_eq!(diagnostic.phase, "preflight");
}

#[test]
fn preview_caught_fixture_violation_fails_preflight_and_mount() {
    let native = Native::new();
    for preflight in [true, false] {
        let attempt = "try { __zguiServices.readTextAsset('test.txt'); } catch (error) { if (error.code !== 'E_FIXTURE_MISSING_SERVICE') throw error; }";
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
            assert_eq!(session.counts().requests, 0);
            assert_eq!(session.contract_failure().unwrap().code, "E_FIXTURE_MISSING_SERVICE");
            assert!(session.checkpoint().is_err());
            session.retire();
            assert_eq!(session.contract_failure().unwrap().code, "E_FIXTURE_MISSING_SERVICE");
            // A retired session ignores late native binding notifications: no
            // fault or owner wake, even with a sticky contract failure.
            while native.wakes.try_recv().is_ok() {}
            session.frame_checkpoint();
            assert!(native.wakes.try_recv().is_err());
            assert_eq!(session.counts(), ResourceCounts::default());
            error
        };
        let SessionError::Contract(failure) = error else { panic!("fixture failure must retain its type") };
        assert_eq!(failure.code, "E_FIXTURE_MISSING_SERVICE");
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
                let SessionError::Diagnostic(diagnostic) = error else { panic!("rejection must stay a diagnostic") };
                assert_eq!(diagnostic.message, message);
            }
            None => { session.checkpoint().unwrap(); }
        }
        session.retire();
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
    let SessionError::Diagnostic(diagnostic) = error else { panic!("frame checkpoint must not stringify a job exception") };
    assert_eq!(diagnostic.message, "original job sentinel");
    assert_eq!(diagnostic.phase, "pending job");
    assert!(diagnostic.stack.as_deref().unwrap().contains("originalJob"));
    session.retire();
}

#[test]
fn preview_cleanup_retains_deadline_failure_and_releases_every_resource() {
    let fixture = Fixture::new(SIMPLE, "");
    let native = Native::new();
    let deadline = Rc::new(Cell::new(Instant::now() + Duration::from_secs(5)));
    let session = native.preview(fixture.package(), RuntimeLimits::default(), deadline.clone()).unwrap();
    session.mount().unwrap();
    deadline.set(Instant::now() - Duration::from_millis(1));
    let first = session.retire();
    let second = session.retire();
    let Some(RetirementFailure::Cleanup(diagnostic)) = &first.failure else {
        panic!("expired cleanup may not pass silently");
    };
    assert_eq!(first.failure, second.failure);
    assert_eq!(diagnostic.kind, js_runtime::DiagnosticKind::BudgetExceeded);
    assert_eq!(diagnostic.phase, "cleanup");
    assert_eq!(first.counts, ResourceCounts::default());
    assert_eq!(session.counts(), ResourceCounts::default());
}

#[test]
fn preview_uncaught_fixture_violation_retains_its_category_across_entries() {
    for statement in [
        "__zguiServices.readTextAsset('test.txt');",
        "Promise.resolve().then(() => __zguiServices.readTextAsset('test.txt'));",
    ] {
        let native = Native::new();
        let fixture = Fixture::new(&format!("export const hostApiVersion=1; export function mount() {{ {statement} return () => {{}}; }}"), "");
        let session = native.preview(fixture.package(), RuntimeLimits::default(), Rc::new(Cell::new(Instant::now()+Duration::from_secs(5)))).unwrap();
        let failure = session.mount().and_then(|()|session.checkpoint().map(|_|())).unwrap_err();
        let SessionError::Contract(contract) = failure else { panic!("fixture failure must retain its type") };
        assert_eq!(contract.code,"E_FIXTURE_MISSING_SERVICE");
        session.retire();
        assert_eq!(session.counts(),ResourceCounts::default());
    }
}

#[test]
fn rejected_mount_keeps_its_returned_disposer_for_cleanup() {
    let native = Native::new();
    let fixture = Fixture::new("export const hostApiVersion=1; export function mount() { try { __zguiServices.readTextAsset('test.txt'); } catch {} return () => recordCleanup(); }", "");
    let session = native.preview(fixture.package(), RuntimeLimits::default(), Rc::new(Cell::new(Instant::now()+Duration::from_secs(5)))).unwrap();
    let called = Rc::new(Cell::new(0));
    let count = called.clone();
    session.engine.borrow_mut().as_mut().unwrap().with_context(|ctx| {
        ctx.globals().set("recordCleanup",Function::new(ctx.clone(),move ||count.set(count.get()+1))?)
    }).unwrap();
    assert!(session.mount().is_err());
    session.retire();
    session.retire();
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
    let environment = Environment::new(clock.clone(), Some(1234), 7);
    let definition: crate::assets::fixture::FixtureDefinition = serde_json::from_value(serde_json::json!({
        "schemaVersion":1, "id":"session-service", "services": {
            "assets":{"provider":"mock-text-assets-v1","latencyMs":5,"initialState":{"assets":{"test.txt":{}}}}
        }, "events":[]
    })).unwrap();
    let fixtures = crate::assets::fixture::FixtureRuntime::new(definition, environment.clone());
    let session = Session::prepare(native.region.clone(), SessionModule::package(fixture.package()), 1, native.waker.clone(),
        SurfaceId::new(7), SessionPolicy::preview(RuntimeLimits::default(), Rc::new(Cell::new(Instant::now() + Duration::from_secs(5))),
            environment, fixtures.clone())).unwrap();
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
    session.retire();
    assert_eq!(session.counts(), ResourceCounts::default());
}

const GEOMETRY: &str = r#"
export const hostApiVersion = 1;
export function mount(root) {
    const label = __zgui.createTextNode('waiting');
    const box = __zgui.createElement('box');
    __zgui.insertNode(root, label, null);
    __zgui.insertNode(root, box, null);
    globalThis.geometryCalls = 0;
    globalThis.cancelGeometry = __zgui.observeGeometry(box, 'borderSize', size => {
        geometryCalls += 1;
        __zgui.replaceText(label, String(size.width));
    });
    if (geometryCalls !== 0) throw Error('registration invoked a callback');
    return () => { cancelGeometry(); __zgui.clearRoot(root); };
}
"#;

#[test]
fn geometry_delivers_immediately_while_the_engine_is_free() {
    let fixture = Fixture::new(GEOMETRY, "");
    let native = Native::new();
    native.host.set_scale(1.75);
    let session = native.prepare(fixture.package(), 1).unwrap();
    session.mount().unwrap();
    assert_eq!(native.text(), "waiting");
    assert_eq!(session.counts().callbacks, 1);
    while native.wakes.try_recv().is_ok() {}
    session.bridge().run_turn(session.engine.borrow_mut().as_mut().unwrap().turn(), |_, js| {
        js.evaluate_script("globalThis.lateCancel = __zgui.observeGeometry(mountRoot, 'contentSize', () => {}); 'registered'")
    }).unwrap();
    assert!(native.wakes.try_recv().is_ok(), "registration did not wake the owner");
    assert!(native.wakes.try_recv().is_err(), "registration produced more than one wake");
    session.bridge().run_turn(session.engine.borrow_mut().as_mut().unwrap().turn(), |_, js| {
        js.evaluate_script("lateCancel(); 'cancelled'")
    }).unwrap();
    let box_node = native.backend.children(native.region.root)[1];
    native.backend.deliver(box_node, ObservedValue::BorderSize(Size::new(DevicePx(175.0), DevicePx(70.0))));
    assert_eq!(native.text(), "100");
    let calls = session.engine.borrow_mut().as_mut().unwrap()
        .with_context(|ctx| ctx.eval::<u32, _>("geometryCalls")).unwrap();
    assert_eq!(calls, 1);
    session.retire();
    assert_eq!(session.counts(), ResourceCounts::default());
    assert_eq!(native.backend.observation_count(), 0);
}

#[test]
fn every_shell_mutation_requires_capability_even_when_caught() {
    let operations = [
        "__zguiShell.openLayer('bar', {}, () => () => {})",
        "__zguiShell.updateLayer(1, {})",
        "__zguiShell.closeLayer(1)",
        "__zguiShell.setInputRegion(1, 'empty')",
        "__zguiShell.requestSessionLock(() => {})",
        "__zguiShell.openLock('locker', 'test-output', () => () => {})",
        "__zguiShell.closeLock(1)",
    ];
    for operation in operations {
        let fixture = Fixture::new(&format!(r#"
            export const hostApiVersion = 1;
            export function mount() {{
                try {{ {operation}; }} catch (error) {{
                    if (error.code !== 'E_CAPABILITY') throw error;
                    return () => {{}};
                }}
                throw Error('unauthorized mutation was admitted');
            }}
        "#), "");
        let native = Native::new();
        let live = native.prepare(fixture.package(), 1).expect("prepare zero-capability live package");
        live.mount().expect("live denial is a typed, catchable exception");
        assert_eq!(live.retire().counts, ResourceCounts::default(), "live denial leaked resources: {operation}");
        let deadline = Rc::new(Cell::new(Instant::now() + Duration::from_secs(5)));
        let preview = native.preview(fixture.package(), RuntimeLimits::default(), deadline)
            .expect("prepare zero-capability preview package");
        let error = preview.mount().expect_err("preview denial must survive being caught");
        assert!(matches!(error, SessionError::Contract(ContractFailure { code: "E_CAPABILITY", .. })), "wrong denial for {operation}: {error}");
        assert_eq!(preview.retire().counts, ResourceCounts::default(), "preview denial leaked resources: {operation}");
    }
}

#[test]
fn geometry_callbacks_cancel_themselves_and_prepared_siblings() {
    let fixture = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount(root) {
            const box = __zgui.createElement('box');
            __zgui.insertNode(root, box, null);
            globalThis.hits = { a: 0, b: 0 };
            globalThis.cancelA = __zgui.observeGeometry(box, 'borderSize', () => { hits.a += 1; cancelA(); cancelB(); });
            globalThis.cancelB = __zgui.observeGeometry(box, 'borderSize', () => { hits.b += 1; });
            return () => __zgui.clearRoot(root);
        }
    "#, "");
    let native = Native::new();
    let session = native.prepare(fixture.package(), 1).unwrap();
    session.mount().unwrap();
    let box_node = native.backend.children(native.region.root)[0];
    native.backend.deliver(box_node, ObservedValue::BorderSize(Size::new(DevicePx(10.0), DevicePx(10.0))));
    let hits = session.engine.borrow_mut().as_mut().unwrap()
        .with_context(|ctx| ctx.eval::<String, _>("hits.a + ':' + hits.b")).unwrap();
    assert_eq!(hits, "1:0");
    native.backend.deliver(box_node, ObservedValue::BorderSize(Size::new(DevicePx(20.0), DevicePx(20.0))));
    let hits = session.engine.borrow_mut().as_mut().unwrap()
        .with_context(|ctx| ctx.eval::<String, _>("hits.a + ':' + hits.b")).unwrap();
    assert_eq!(hits, "1:0");
    assert_eq!(native.backend.observation_count(), 0);
    assert_eq!(session.counts().callbacks, 0);
    session.retire();
    assert_eq!(session.counts(), ResourceCounts::default());
}

#[test]
fn geometry_callback_failure_faults_and_retires_every_observer() {
    let fixture = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount(root) {
            const box = __zgui.createElement('box');
            __zgui.insertNode(root, box, null);
            __zgui.observeGeometry(root, 'borderSize', () => {});
            __zgui.observeGeometry(box, 'borderSize', () => { throw Error('geometry callback failed'); });
            return () => __zgui.clearRoot(root);
        }
    "#, "");
    let native = Native::new();
    let session = native.prepare(fixture.package(), 1).unwrap();
    session.mount().unwrap();
    let box_node = native.backend.children(native.region.root)[0];
    native.backend.deliver(box_node, ObservedValue::BorderSize(Size::new(DevicePx(35.0), DevicePx(35.0))));
    assert!(session.bridge().is_disposed());
    assert_eq!(native.backend.observation_count(), 0);
    let error = session.checkpoint().unwrap_err();
    let SessionError::Diagnostic(diagnostic) = error else { panic!("geometry throw must stay a diagnostic") };
    assert!(diagnostic.message.contains("geometry callback failed"));
    session.retire();
    assert_eq!(session.counts(), ResourceCounts::default());
}

#[test]
fn geometry_defers_with_the_scale_captured_at_enqueue_time() {
    let fixture = Fixture::new(GEOMETRY, "");
    let native = Native::new();
    native.host.set_scale(1.75);
    let session = native.prepare(fixture.package(), 1).unwrap();
    session.mount().unwrap();
    while native.wakes.try_recv().is_ok() {}
    let box_node = native.backend.children(native.region.root)[1];
    {
        let _guard = session.engine.borrow_mut();
        native.backend.deliver(box_node, ObservedValue::BorderSize(Size::new(DevicePx(175.0), DevicePx(70.0))));
    }
    assert_eq!(native.text(), "waiting");
    native.host.set_scale(3.5);
    session.checkpoint().unwrap();
    assert_eq!(native.text(), "100");
    {
        let _guard = session.engine.borrow_mut();
        native.backend.deliver(box_node, ObservedValue::BorderSize(Size::new(DevicePx(350.0), DevicePx(140.0))));
    }
    session.engine.borrow_mut().as_mut().unwrap().evaluate_script("cancelGeometry(); 'cancelled'").unwrap();
    session.checkpoint().unwrap();
    assert_eq!(native.text(), "100");
    let calls = session.engine.borrow_mut().as_mut().unwrap()
        .with_context(|ctx| ctx.eval::<u32, _>("geometryCalls")).unwrap();
    assert_eq!(calls, 1);
    session.retire();
    assert_eq!(session.counts(), ResourceCounts::default());
    assert_eq!(native.backend.observation_count(), 0);
}

#[test]
fn live_unhandled_rejection_discards_the_staged_lock_command_only_while_retained() {
    let fixture = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount() {
            const lock = __zguiShell.requestSessionLock(state => {
                if (state === 'locked') globalThis.retained = true;
            });
            if (globalThis.retained) lock.unlock();
            return () => lock.unsubscribe();
        }
        export function reject() { Promise.reject(Error('live rejection')); }
    "#, "").with_capabilities(crate::package::Capabilities { session_lock: true, ..Default::default() });

    for (state, command) in [
        (SessionLockState::Locked, crate::shell::LockCommand::Unlock),
        (SessionLockState::Unlocked, crate::shell::LockCommand::Request),
    ] {
        let native = Native::new();
        let policy = SessionPolicy::live(native.executor.clone()).with_session_lock(Some(state));
        let session = Session::prepare(native.region.clone(), SessionModule::package(fixture.package()), 1, native.waker.clone(), SurfaceId::new(7), policy)
            .expect("prepare live session with lock capability and compositor state");
        session.mount().expect("mount and stage the lock command");
        assert!(session.shell.has_lock_command(), "mount did not stage {command:?} under {state:?}");
        session.invoke("reject").expect("queue a rejection without throwing synchronously");
        if state == SessionLockState::Locked {
            let error = session.checkpoint().expect_err("an unhandled rejection must fault under a retained lock");
            let SessionError::Diagnostic(diagnostic) = error else {
                panic!("expected a rejection diagnostic, got {error:?}");
            };
            assert_eq!(diagnostic.phase, "promise rejection", "checkpoint must identify the failing entry");
            assert_eq!(diagnostic.message, "live rejection", "checkpoint must preserve the rejection reason");
            assert_eq!(session.shell.take_lock_command(), None, "a fault must discard the staged unlock");
        } else {
            session.checkpoint().expect("live rejection outside a retained lock should only be logged");
            assert_eq!(session.shell.take_lock_command(), Some(command), "logged rejection must not discard the lock request");
        }
    }
}

#[test]
fn retiring_live_session_reports_disposer_rejection_under_a_retained_lock() {
    let fixture = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount() {
            return () => { Promise.reject(Error('live disposer rejected')); };
        }
    "#, "");
    let native = Native::new();
    let policy = SessionPolicy::live(native.executor.clone()).with_session_lock(Some(SessionLockState::Locked));
    let session = Session::prepare(native.region.clone(), SessionModule::package(fixture.package()), 1, native.waker.clone(), SurfaceId::new(7), policy)
        .expect("prepare live predecessor on a locked compositor");
    session.mount().expect("mount the live predecessor");
    let report = session.retire();
    let Some(RetirementFailure::Cleanup(diagnostic)) = report.failure else {
        panic!("predecessor report must retain the disposal failure, got {:?}", report.failure);
    };
    assert_eq!(diagnostic.phase, "promise rejection", "report must identify the disposer rejection");
    assert_eq!(diagnostic.message, "live disposer rejected", "retirement must preserve the rejection reason");
}

#[test]
fn unlock_teardown_checks_rejections_without_running_pending_jobs() {
    let fixture = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount() { return () => {}; }
        export function reject() {
            globalThis.jobRan = false;
            globalThis.timerDenied = false;
            const reason = Error();
            Object.defineProperty(reason, 'message', {
                get() {
                    try { setTimeout(() => {}, 0); } catch { globalThis.timerDenied = true; }
                    return 'unlock teardown rejected';
                }
            });
            Promise.resolve().then(() => { globalThis.jobRan = true; });
            Promise.reject(reason);
        }
    "#, "");
    let native = Native::new();
    let session = native.prepare(fixture.package(), 1).expect("prepare live unlock teardown fixture");
    session.mount().expect("mount the session");
    session.invoke("reject").expect("enqueue both a rejection and an unrelated job");
    let error = session.check_unlock_teardown_rejections().expect_err("a synchronous teardown rejection must veto unlock");
    let SessionError::Diagnostic(diagnostic) = error else {
        panic!("expected a rejection diagnostic, got {error:?}");
    };
    assert_eq!(diagnostic.message, "unlock teardown rejected", "unlock must preserve the original rejection");
    assert!(!session.lock_commands_ready(), "the unrelated job must remain queued");
    let state = session.engine.borrow_mut().as_mut().expect("session engine is active")
        .with_context(|ctx| ctx.eval::<String, _>("jobRan + ':' + timerDenied")).expect("read teardown effects");
    assert_eq!(state, "false:true", "teardown must not run pending jobs or let diagnostic accessors create new work");
}

#[test]
fn rejection_diagnostic_accessors_cannot_mutate_the_mount_region() {
    let fixture = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount(root) {
            __zgui.insertNode(root, __zgui.createTextNode('active'), null);
            globalThis.mutated = false;
            globalThis.threw = '';
            const reason = new Error('original');
            Object.defineProperty(reason, 'message', {
                get() {
                    try {
                        const extra = __zgui.createTextNode('forged-by-rejection');
                        __zgui.insertNode(root, extra, null);
                        globalThis.mutated = true;
                    } catch (error) {
                        globalThis.threw = String(error && error.message || error);
                    }
                    return 'accessor rejection';
                }
            });
            Promise.reject(reason);
            return () => __zgui.clearRoot(root);
        }
    "#, "");
    let native = Native::new();
    let live = native.prepare(fixture.package(), 1).unwrap();
    live.mount().unwrap();
    live.checkpoint().unwrap();
    assert_eq!(native.text(), "active");
    let state = live.engine.borrow_mut().as_mut().unwrap()
        .with_context(|ctx| ctx.eval::<String, _>("mutated + ':' + threw")).unwrap();
    assert_eq!(state, "false:native mutation outside a structural turn");
    live.retire();

    let preview = native.preview(
        fixture.package(),
        RuntimeLimits::default(),
        Rc::new(Cell::new(Instant::now() + Duration::from_secs(5))),
    ).unwrap();
    preview.mount().unwrap();
    assert!(preview.checkpoint().is_err());
    assert_eq!(native.text(), "active");
    let state = preview.engine.borrow_mut().as_mut().unwrap()
        .with_context(|ctx| ctx.eval::<String, _>("mutated + ':' + threw")).unwrap();
    assert_eq!(state, "false:native mutation outside a structural turn");
    preview.retire();
}

#[test]
fn retire_prioritizes_unreported_contract_over_cleanup_and_skips_reported_contract() {
    let native = Native::new();
    let deadline = Rc::new(Cell::new(Instant::now() + Duration::from_secs(5)));

    // 1. Unreported contract violation triggered during cleanup beats cleanup exception.
    let unreported = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount() {
            return () => {
                try { __zguiServices.readTextAsset('test.txt'); } catch {}
                throw Error('cleanup exploded');
            };
        }
    "#, "");
    let s1 = native.preview(unreported.package(), RuntimeLimits::default(), deadline.clone()).unwrap();
    s1.mount().unwrap();
    let r1 = s1.retire();
    let r1_again = s1.retire();
    assert_eq!(r1.failure, r1_again.failure);
    let Some(RetirementFailure::Contract(contract)) = &r1.failure else {
        panic!("unreported contract failure must beat cleanup failure: {:?}", r1.failure);
    };
    assert_eq!(contract.code, "E_FIXTURE_MISSING_SERVICE");

    // 2. Previously reported contract failure is not duplicated; cleanup failure surfaces.
    let reported = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount() {
            try { __zguiServices.readTextAsset('test.txt'); } catch {}
            return () => { throw Error('cleanup after contract'); };
        }
    "#, "");
    let s2 = native.preview(reported.package(), RuntimeLimits::default(), deadline).unwrap();
    assert!(matches!(s2.mount(), Err(SessionError::Contract(_))));
    let r2 = s2.retire();
    let Some(RetirementFailure::Cleanup(cleanup)) = &r2.failure else {
        panic!("cleanup failure must surface once contract failure was already reported: {:?}", r2.failure);
    };
    assert_eq!(cleanup.message, "cleanup after contract");
    assert!(cleanup.stack.as_deref().unwrap().contains("app.mjs"));
}