use super::*;
use js_runtime::{Diagnostic, JsRuntime, RuntimeLimits};
use std::{fs, io::{BufRead, BufReader, Read, Write}, os::{fd::AsRawFd, unix::net::{UnixListener, UnixStream}}, sync::atomic::AtomicU64, time::Instant};
use zgui::view::stub::StubHost;

const DEADLINE: Duration = Duration::from_secs(5);

struct Fixture { directory: PathBuf, listener: UnixListener }
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!("zgui-niri-binding-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&directory).unwrap();
        let listener = UnixListener::bind(directory.join("socket")).unwrap();
        Self { directory, listener }
    }
    fn accept(&self) -> BufReader<UnixStream> {
        let mut fd = libc::pollfd { fd: self.listener.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: this call borrows one initialized record and a live listener FD.
        assert_eq!(unsafe { libc::poll(&mut fd, 1, 5000) }, 1, "subscription did not connect");
        let (stream, _) = self.listener.accept().unwrap();
        stream.set_read_timeout(Some(DEADLINE)).unwrap();
        stream.set_write_timeout(Some(DEADLINE)).unwrap();
        let mut peer = BufReader::new(stream);
        let mut request = String::new();
        peer.read_line(&mut request).unwrap();
        assert_eq!(request, "\"EventStream\"\n");
        send(&mut peer, r#"{"Ok":"Handled"}"#);
        peer
    }
}
impl Drop for Fixture { fn drop(&mut self) { fs::remove_dir_all(&self.directory).unwrap(); } }

fn send(peer: &mut BufReader<UnixStream>, event: &str) { writeln!(peer.get_mut(), "{event}").unwrap(); }
fn initial(peer: &mut BufReader<UnixStream>) {
    send(peer, r#"{"WorkspacesChanged":{"workspaces":[{"id":9007199254740993,"idx":1,"name":null,"output":"TEST-1","is_urgent":false,"is_active":true,"is_focused":true,"active_window_id":null}]}}"#);
    send(peer, r#"{"WindowsChanged":{"windows":[{"id":18446744073709551615,"title":null,"app_id":null,"pid":null,"workspace_id":9007199254740993,"is_focused":true,"is_floating":false,"is_urgent":false}]}}"#);
}

struct ChannelWaker(mpsc::Sender<WakeReason>);
impl Waker for ChannelWaker { fn wake(&self, reason: WakeReason) { let _ = self.0.send(reason); } }
struct Session { work: Rc<Work>, engine: JsRuntime, wakes: Receiver<WakeReason> }
impl Session {
    fn new(generation: u64) -> Self {
        let (sender, wakes) = mpsc::channel();
        let work = Work::new(HostHandle::from_rc(Rc::new(StubHost::default())), None, generation,
            Executor::new(1, 8).unwrap(), Arc::new(ChannelWaker(sender)), SurfaceId::new(7));
        let mut engine = JsRuntime::new(RuntimeLimits::default()).unwrap();
        engine.with_context(|ctx| { work.install(&ctx)?; ctx.eval::<(), _>("globalThis.seen = []; globalThis.errors = [];") }).unwrap();
        Self { work, engine, wakes }
    }
    fn subscribe(&mut self, fixture: &Fixture, snapshot: &str, error: &str) -> Result<(), Diagnostic> {
        self.engine.with_context(|ctx| {
            let on_snapshot: Function = ctx.eval(snapshot)?;
            let on_error: Function = ctx.eval(error)?;
            let subscription = self.work.subscribe_niri(&ctx, Some(fixture.directory.join("socket")), on_snapshot, on_error)?;
            ctx.globals().set("subscription", subscription)
        })
    }
    fn eval(&mut self, source: &str) -> String {
        self.engine.with_context(|ctx| ctx.eval::<rquickjs::Coerced<String>, _>(source)).unwrap().0
    }
    fn next(&self) -> NiriDelivery {
        let end = Instant::now() + DEADLINE;
        loop {
            if let Some(delivery) = self.work.next_subscription() { return delivery; }
            self.wakes.recv_timeout(end.saturating_duration_since(Instant::now())).expect("subscription did not wake its owner");
        }
    }
    fn deliver(&mut self, delivery: NiriDelivery) {
        self.engine.with_context(|ctx| self.work.deliver_subscription(&ctx, delivery)).unwrap();
    }
    fn collect(&mut self) { self.engine.with_context(|ctx| { ctx.run_gc(); Ok(()) }).unwrap(); }
    fn token(&mut self) -> Rc<()> {
        let token = Rc::new(());
        let retained = token.clone();
        self.engine.with_context(|ctx| ctx.globals().set("rootToken", Func::from(move || drop(retained.clone())))).unwrap();
        token
    }
}
impl Drop for Session { fn drop(&mut self) { self.work.finish_stop(); } }

#[test]
fn large_ids_nullable_fields_and_self_cancellation_cross_the_real_engine() {
    let fixture = Fixture::new();
    let mut session = Session::new(1);
    session.work.mounted();
    let token = session.token();
    session.subscribe(&fixture, "((retained) => snapshot => { retained(); seen.push(snapshot); subscription.cancel(); })(rootToken)", "error => errors.push(error.code)").unwrap();
    session.eval("rootToken = undefined");
    let mut peer = fixture.accept();
    initial(&mut peer);
    let delivery = session.next();
    session.deliver(delivery);
    assert_eq!(session.eval(r#"JSON.stringify([seen[0].windows[0].id, seen[0].windows[0].workspaceId, seen[0].workspaces[0].id, seen[0].windows[0].title, seen[0].windows[0].appId, seen[0].windows[0].pid, seen[0].workspaces[0].name, seen[0].workspaces[0].activeWindowId, seen[0].windows[0].focused, seen[0].workspaces[0].active])"#), r#"["18446744073709551615","9007199254740993","9007199254740993",null,null,null,null,null,true,true]"#);
    assert_eq!(session.work.counts(), (0, 0, 0, 0));
    session.collect();
    assert_eq!(Rc::strong_count(&token), 1, "self-cancellation retained the callback root");
    assert_eq!(peer.read(&mut [0]).unwrap(), 0, "cancellation did not close the transport");
}

#[test]
fn cancellation_and_disposal_discard_already_selected_deliveries() {
    for stop in [false, true] {
        let fixture = Fixture::new();
        let mut session = Session::new(1);
        session.work.mounted();
        let token = session.token();
        session.subscribe(&fixture, "((retained) => snapshot => { retained(); seen.push(snapshot); })(rootToken)", "error => errors.push(error.code)").unwrap();
        session.eval("rootToken = undefined");
        let mut peer = fixture.accept();
        initial(&mut peer);
        let selected = session.next();
        if stop { session.work.begin_stop(); } else { session.eval("subscription.cancel()"); }
        session.deliver(selected);
        session.eval("subscription.cancel(); subscription.cancel()");
        assert_eq!(session.eval("seen.length + errors.length"), "0");
        session.collect();
        assert_eq!(Rc::strong_count(&token), 1);
        assert_eq!(session.work.counts(), (0, 0, 0, 0));
    }
}

#[test]
fn stale_generation_cannot_deliver_to_a_reused_subscription_id() {
    let fixture = Fixture::new();
    let mut old = Session::new(7);
    old.work.mounted();
    old.subscribe(&fixture, "snapshot => seen.push(snapshot)", "error => errors.push(error.code)").unwrap();
    let mut first = fixture.accept();
    initial(&mut first);
    let stale = old.next();
    old.work.begin_stop();

    let mut current = Session::new(8);
    current.work.mounted();
    current.subscribe(&fixture, "snapshot => seen.push(snapshot)", "error => errors.push(error.code)").unwrap();
    let mut second = fixture.accept();
    current.deliver(stale);
    assert_eq!(current.eval("seen.length + errors.length"), "0");
    assert_eq!(current.work.counts(), (0, 0, 0, 1));
    initial(&mut second);
    let fresh = current.next();
    current.deliver(fresh);
    assert_eq!(current.eval("seen.length"), "1");
}

#[test]
fn terminal_error_releases_registration_before_calling_the_error_handler() {
    let fixture = Fixture::new();
    let mut session = Session::new(1);
    session.work.mounted();
    session.subscribe(&fixture, "snapshot => seen.push(snapshot)", "error => { errors.push([error.code, error instanceof Error]); subscription.cancel(); }").unwrap();
    let mut peer = fixture.accept();
    send(&mut peer, r#"{"WindowOpenedOrChanged":{}}"#);
    let error = session.next();
    session.deliver(error);
    assert_eq!(session.eval("JSON.stringify(errors)"), r#"[["NiriProtocol",true]]"#);
    assert_eq!(session.work.counts(), (0, 0, 0, 0));
    assert!(session.work.next_subscription().is_none());
}

#[test]
fn lifecycle_and_subscription_limit_reject_without_retaining_roots() {
    let fixture = Fixture::new();
    let mut session = Session::new(1);
    assert!(session.subscribe(&fixture, "() => {}", "() => {}").is_err());
    session.work.mounting();
    let mut peers = Vec::new();
    for _ in 0..MAX_SUBSCRIPTIONS {
        session.subscribe(&fixture, "() => {}", "() => {}").unwrap();
        peers.push(fixture.accept());
    }
    assert!(session.subscribe(&fixture, "() => {}", "() => {}").is_err());
    assert_eq!(session.work.counts(), (0, 0, 0, MAX_SUBSCRIPTIONS));
    session.work.begin_stop();
    assert!(session.subscribe(&fixture, "() => {}", "() => {}").is_err());
    assert_eq!(session.work.counts(), (0, 0, 0, 0));
    for mut peer in peers { assert_eq!(peer.read(&mut [0]).unwrap(), 0); }
}


#[test]
fn inherited_setters_cannot_intercept_subscription_or_snapshot_properties() {
    let fixture = Fixture::new();
    let mut session = Session::new(1);
    session.work.mounted();
    session.eval(r#"
        globalThis.called = 0;
        for (const key of ['id', 'windows', 'workspaces', 'cancel']) {
            Object.defineProperty(Object.prototype, key, {
                configurable: true, set() { throw Error('inherited setter: ' + key); }
            });
        }
        Object.defineProperty(Array.prototype, '0', {
            configurable: true, set() { throw Error('inherited array setter'); }
        });
    "#);
    session.subscribe(&fixture, "snapshot => { globalThis.last = snapshot; called++; subscription.cancel(); }", "() => {}").unwrap();
    let mut peer = fixture.accept();
    initial(&mut peer);
    let delivery = session.next();
    session.deliver(delivery);
    assert_eq!(session.eval("called + ':' + last.windows[0].id"), "1:18446744073709551615");
    assert_eq!(session.work.counts(), (0, 0, 0, 0));
}

#[test]
fn terminal_conversion_rechecks_cancellation_and_releases_roots_on_throw() {
    for throws in [false, true] {
        let fixture = Fixture::new();
        let mut session = Session::new(1);
        session.work.mounted();
        let token = session.token();
        session.subscribe(&fixture, "() => {}", "((retained) => error => { retained(); errors.push(error.code); })(rootToken)").unwrap();
        session.eval("rootToken = undefined");
        session.eval(if throws {
            "Object.defineProperty(Error.prototype, 'message', { configurable: true, set() { throw Error('conversion failed'); } })"
        } else {
            "Object.defineProperty(Error.prototype, 'message', { configurable: true, set() { subscription.cancel(); } })"
        });
        let mut peer = fixture.accept();
        send(&mut peer, r#"{"WindowOpenedOrChanged":{}}"#);
        let delivery = session.next();
        let result = session.engine.with_context(|ctx| session.work.deliver_subscription(&ctx, delivery));
        assert_eq!(result.is_err(), throws);
        assert_eq!(session.eval("errors.length"), "0");
        session.collect();
        assert_eq!(Rc::strong_count(&token), 1);
        assert_eq!(session.work.counts(), (0, 0, 0, 0));
    }
}

#[test]
fn cancelled_terminal_delivery_cannot_execute_error_prototype_code() {
    let fixture = Fixture::new();
    let mut session = Session::new(1);
    session.work.mounted();
    session.subscribe(&fixture, "() => {}", "error => errors.push(error.code)").unwrap();
    let mut peer = fixture.accept();
    send(&mut peer, r#"{"WindowOpenedOrChanged":{}}"#);
    let selected = session.next();
    session.eval(r#"
        subscription.cancel();
        Object.defineProperty(Error.prototype, 'message', {
            configurable: true, set() { throw Error('cancelled delivery entered JavaScript'); }
        });
    "#);
    session.deliver(selected);
    assert_eq!(session.eval("errors.length"), "0");
    assert_eq!(session.work.counts(), (0, 0, 0, 0));
}
