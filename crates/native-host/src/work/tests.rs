use super::*;
use js_runtime::{Diagnostic, JsRuntime, RuntimeLimits};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::PathBuf, sync::atomic::AtomicU64};
use zgui::view::stub::StubHost;

const DEADLINE: Duration = Duration::from_secs(5);
static NEXT_PACKAGE: AtomicU64 = AtomicU64::new(0);

struct Assets(PathBuf);

impl Assets {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "zgui-work-tests-{}-{}",
            std::process::id(),
            NEXT_PACKAGE.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&directory).unwrap();
        let files = BTreeMap::from([
            ("app.mjs", b"export const hostApiVersion = 1; export function mount() { return () => {}; }".as_slice()),
            ("app.mjs.map", br#"{"version":3,"sources":[],"names":[],"mappings":""}"#.as_slice()),
            ("invalid.txt", &[0xff][..]),
            ("text.txt", "owned asset: héllo\n".as_bytes()),
        ]);
        let mut digest = Sha256::new();
        for (name, bytes) in files {
            fs::write(directory.join(name), bytes).unwrap();
            digest.update(name.as_bytes());
            digest.update([0]);
            digest.update(bytes);
            digest.update([0]);
        }
        let manifest = serde_json::json!({
            "schemaVersion": 1, "id": "work-tests", "hostApiVersion": 1,
            "entry": "app.mjs", "sourceMap": "app.mjs.map", "styles": [],
            "assets": ["text.txt", "invalid.txt"],
            "buildId": format!("sha256:{:x}", digest.finalize()),
        });
        fs::write(directory.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
        Self(directory)
    }

    fn package(&self) -> Rc<Package> {
        Rc::new(Package::load(&self.0).unwrap())
    }
}

impl Drop for Assets {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

struct ChannelWaker(mpsc::Sender<WakeReason>);

impl Waker for ChannelWaker {
    fn wake(&self, reason: WakeReason) {
        let _ = self.0.send(reason);
    }
}

// StubHost only supplies registration IDs. These tests explicitly model native
// firing with Work::fire; they do not establish native timer/frame timing.
struct Session {
    work: Rc<Work>,
    engine: JsRuntime,
    host: Rc<StubHost>,
    wakes: Receiver<WakeReason>,
}

impl Session {
    fn new(package: Rc<Package>, generation: u64) -> Self {
        let host = Rc::new(StubHost::default());
        let (sender, wakes) = mpsc::channel();
        let work = Work::new(
            HostHandle::from_rc(host.clone()), Some(package), generation,
            Executor::new(1, 8).unwrap(), Arc::new(ChannelWaker(sender)), SurfaceId::new(7), ServicePolicy::Live,
        );
        let mut engine = JsRuntime::new(RuntimeLimits::default()).unwrap();
        engine.with_context(|ctx| work.install(&ctx)).unwrap();
        Self { work, engine, host, wakes }
    }

    fn script(&mut self, source: &str) -> String {
        self.engine.with_context(|ctx| ctx.eval::<rquickjs::Coerced<String>, _>(source)).unwrap().0
    }

    fn id(&mut self, name: &str) -> u64 {
        self.script(name).parse().unwrap()
    }

    fn completion(&self) -> Completion {
        let wake = self.wakes.recv_timeout(DEADLINE).expect("executor did not publish a completion");
        match wake {
            WakeReason::ReactiveWork { surfaces } => assert_eq!(&*surfaces, &[SurfaceId::new(7)]),
            other => panic!("unexpected executor wake: {other:?}"),
        }
        self.work.next_completion().expect("wake must follow completion publication")
    }

    fn settle(&mut self, completion: Completion) {
        self.engine.with_context(|ctx| self.work.settle(&ctx, completion)).unwrap();
    }

    fn invoke_next(&mut self) -> Result<(), Diagnostic> {
        let call = self.work.next_call().expect("scheduled callback is ready");
        self.engine.with_context(|ctx| {
            let callback = call.callback.restore(&ctx)?;
            match call.arguments {
                Arguments::Timer => callback.call::<_, ()>(()),
                Arguments::Frame(timestamp) => callback.call::<_, ()>((timestamp,)),
            }
        })
    }

    fn jobs(&mut self) {
        let pump = self.engine.pump_jobs();
        assert!(pump.diagnostics.is_empty(), "{:?}", pump.diagnostics);
        assert!(!pump.has_pending_jobs, "finite test Promise chain did not drain");
        assert!(!pump.budget_exhausted);
    }

    fn root_token(&mut self) -> Rc<()> {
        let token = Rc::new(());
        let retained = token.clone();
        self.engine.with_context(|ctx| {
            ctx.globals().set("rootToken", Func::from(move || { drop(retained.clone()); }))
        }).unwrap();
        token
    }

    fn collect(&mut self) {
        self.engine.with_context(|ctx| { ctx.run_gc(); Ok(()) }).unwrap();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Also release roots before JsRuntime if an assertion unwinds the test.
        self.work.finish_stop();
    }
}

#[test]
fn preflight_rejects_timer_frame_and_request_without_retaining_roots() {
    let assets = Assets::new();
    let mut session = Session::new(assets.package(), 1);
    let token = session.root_token();
    assert_eq!(session.script(r#"
        const rejected = [];
        for (const create of [
            () => setTimeout((token => () => token())(rootToken), 0),
            () => requestAnimationFrame((token => () => token())(rootToken)),
            () => __zguiServices.readTextAsset('text.txt'),
        ]) {
            try { create(); rejected.push('accepted'); }
            catch (error) { rejected.push(error instanceof TypeError ? 'TypeError' : 'wrong error'); }
        }
        delete globalThis.rootToken;
        rejected.join(',');
    "#), "TypeError,TypeError,TypeError");
    session.collect();
    assert_eq!(Rc::strong_count(&token), 1, "rejected callbacks retained a JS root");
    assert_eq!(session.work.counts(), (0, 0, 0));
    assert_eq!(session.host.live_timers(), 0);
    assert_eq!(session.host.pending_frames(), 0);
    assert!(session.work.next_completion().is_none());
    assert!(session.wakes.try_recv().is_err());
}

#[test]
fn first_frame_callback_cancels_later_native_fired_callback_in_ready_batch() {
    let assets = Assets::new();
    let mut session = Session::new(assets.package(), 2);
    session.work.mounted();
    session.script(r#"
        globalThis.events = [];
        globalThis.first = requestAnimationFrame(timestamp => {
            events.push(`first:${timestamp}`);
            cancelAnimationFrame(second);
            cancelAnimationFrame(second);
        });
        globalThis.second = requestAnimationFrame(() => events.push('cancelled callback ran'));
        globalThis.third = requestAnimationFrame(timestamp => events.push(`third:${timestamp}`));
        'scheduled';
    "#);
    for name in ["first", "second", "third"] {
        let id = session.id(name);
        session.work.fire(id, Arguments::Frame(12.5));
    }
    assert_eq!(session.work.counts(), (0, 3, 0));
    assert_eq!(session.script("events.length"), "0", "native firing must not enter JS");
    session.invoke_next().unwrap();
    assert_eq!(session.work.counts(), (0, 1, 0));
    session.invoke_next().unwrap();
    assert_eq!(session.script("events.join(',')"), "first:12.5,third:12.5");
    assert!(!session.work.has_ready());
    assert_eq!(session.work.counts(), (0, 0, 0));
}

#[test]
fn successful_and_throwing_one_shots_release_callback_roots() {
    let assets = Assets::new();
    let mut session = Session::new(assets.package(), 3);
    session.work.mounting();
    for (schedule, arguments, throws) in [
        ("setTimeout", Arguments::Timer, false),
        ("setTimeout", Arguments::Timer, true),
        ("requestAnimationFrame", Arguments::Frame(42.25), false),
        ("requestAnimationFrame", Arguments::Frame(42.25), true),
    ] {
        let token = session.root_token();
        session.script(&format!(r#"
            globalThis.calls = 0;
            globalThis.oneShot = {schedule}((token => () => {{
                token(); calls++;
                if ({throws}) throw new Error('one-shot failure');
            }})(rootToken));
            delete globalThis.rootToken;
            'scheduled';
        "#));
        session.collect();
        assert_eq!(Rc::strong_count(&token), 2, "scheduled callback did not retain its function closure");
        session.work.mounted();
        let id = session.id("oneShot");
        session.work.fire(id, arguments);
        assert_eq!(Rc::strong_count(&token), 2, "ready callback lost its function root");
        let result = session.invoke_next();
        if throws {
            assert!(result.unwrap_err().message.contains("one-shot failure"));
        } else {
            result.unwrap();
        }
        session.collect();
        assert_eq!(Rc::strong_count(&token), 1, "finished callback retained its JS closure");
        assert_eq!(session.script("calls"), "1");
        assert_eq!(session.work.counts(), (0, 0, 0));
        session.work.fire(id, arguments);
        assert!(session.work.next_call().is_none(), "a one-shot fired twice");
    }
}

#[test]
fn cancellation_wins_after_worker_success_but_before_owner_delivery() {
    let assets = Assets::new();
    let mut session = Session::new(assets.package(), 4);
    session.work.mounted();
    session.script(r#"
        globalThis.outcome = [];
        globalThis.request = __zguiServices.readTextAsset('text.txt');
        request.promise.then(text => outcome.push(text), error => outcome.push(error.code));
        'requested';
    "#);
    assert_eq!(session.work.counts(), (0, 0, 1));
    let completion = session.completion();
    assert_eq!(completion.result.as_ref().unwrap(), "owned asset: héllo\n");
    session.script("request.cancel(); request.cancel(); 'cancelled'");
    assert_eq!(session.work.counts(), (0, 0, 0));
    session.settle(completion);
    session.jobs();
    assert_eq!(session.script("outcome.join(',')"), "Cancelled");
    assert_eq!(session.work.counts(), (0, 0, 0));
}

#[test]
fn stale_generation_and_duplicate_completions_cannot_settle_a_new_request() {
    let assets = Assets::new();
    let mut retired = Session::new(assets.package(), 5);
    retired.work.mounted();
    retired.script("globalThis.old = __zguiServices.readTextAsset('text.txt'); 'requested'");
    let stale = retired.completion();
    retired.work.begin_stop();

    let mut session = Session::new(assets.package(), 6);
    session.work.mounted();
    session.script(r#"
        globalThis.outcome = [];
        globalThis.first = __zguiServices.readTextAsset('text.txt');
        first.promise.then(text => outcome.push(`first:${text}`));
        'requested';
    "#);
    let first = session.completion();
    assert_eq!(stale.request_id, first.request_id, "exercise ID reuse across generations");
    session.settle(stale);
    session.jobs();
    assert_eq!(session.script("outcome.length"), "0");
    assert_eq!(session.work.counts(), (0, 0, 1));
    let duplicate = Completion { generation: first.generation, request_id: first.request_id, result: Ok("duplicate".into()) };
    session.settle(first);
    session.jobs();
    assert_eq!(session.script("outcome.length"), "1");
    assert_eq!(session.work.counts(), (0, 0, 0));

    session.script(r#"
        globalThis.second = __zguiServices.readTextAsset('text.txt');
        second.promise.then(text => outcome.push(`second:${text}`));
        'requested';
    "#);
    let second = session.completion();
    assert_ne!(duplicate.request_id, second.request_id);
    session.settle(duplicate);
    session.jobs();
    assert_eq!(session.script("outcome.length"), "1");
    assert_eq!(session.work.counts(), (0, 0, 1));
    session.settle(second);
    session.jobs();
    assert_eq!(session.script("JSON.stringify(outcome)"), "[\"first:owned asset: héllo\\n\",\"second:owned asset: héllo\\n\"]");
    assert_eq!(session.work.counts(), (0, 0, 0));
}

#[test]
fn real_executor_success_and_error_settle_promises_and_release_resolver_roots() {
    let assets = Assets::new();
    let mut session = Session::new(assets.package(), 7);
    session.work.mounted();
    session.script(r#"
        globalThis.outcome = [];
        globalThis.success = __zguiServices.readTextAsset('text.txt');
        globalThis.failure = __zguiServices.readTextAsset('invalid.txt');
        success.promise.then(text => outcome.push(['success', text]));
        failure.promise.then(() => outcome.push(['unexpected success']), error => outcome.push(['error', error.code, error instanceof Error]));
        'requested';
    "#);
    assert_eq!(session.work.counts(), (0, 0, 2));
    let success = session.completion();
    assert!(success.result.is_ok());
    session.settle(success);
    assert_eq!(session.work.counts(), (0, 0, 1));
    let failure = session.completion();
    assert_eq!(failure.result.as_ref().unwrap_err().code, "InvalidUtf8");
    session.settle(failure);
    assert_eq!(session.work.counts(), (0, 0, 0));
    session.jobs();
    assert_eq!(session.script("JSON.stringify(outcome)"), "[[\"success\",\"owned asset: héllo\\n\"],[\"error\",\"InvalidUtf8\",true]]");
    session.script("success.cancel(); failure.cancel(); 'already settled'");
    session.jobs();
    assert_eq!(session.script("outcome.length"), "2");
}

#[test]
fn stopping_rejects_new_work_but_existing_cancellation_is_idempotent() {
    let assets = Assets::new();
    let mut session = Session::new(assets.package(), 8);
    session.work.mounted();
    session.script(r#"
        globalThis.timer = setTimeout(() => { throw Error('stopped timer ran'); }, 0);
        globalThis.frame = requestAnimationFrame(() => { throw Error('stopped frame ran'); });
        globalThis.request = __zguiServices.readTextAsset('text.txt');
        'scheduled';
    "#);
    let completion = session.completion();
    let cancellation = session.work.requests.borrow().get(&completion.request_id).unwrap().cancelled.clone();
    session.work.begin_stop();
    assert!(cancellation.load(Ordering::Acquire));
    assert_eq!(session.script(r#"
        const rejected = [];
        for (const create of [
            () => setTimeout(() => {}, 0),
            () => requestAnimationFrame(() => {}),
            () => __zguiServices.readTextAsset('text.txt'),
        ]) {
            try { create(); rejected.push('accepted'); }
            catch (error) { rejected.push(error instanceof TypeError ? 'TypeError' : 'wrong error'); }
        }
        clearTimeout(timer); clearTimeout(timer);
        cancelAnimationFrame(frame); cancelAnimationFrame(frame);
        request.cancel(); request.cancel();
        rejected.join(',');
    "#), "TypeError,TypeError,TypeError");
    assert_eq!(session.work.counts(), (0, 0, 0));
    assert_eq!(session.host.live_timers(), 0);
    assert_eq!(session.host.pending_frames(), 0);
    session.settle(completion);
    session.work.finish_stop();
    session.work.finish_stop();
    session.script("clearTimeout(timer); cancelAnimationFrame(frame); request.cancel(); 'disposed cancellations'");
    assert_eq!(session.work.counts(), (0, 0, 0));
}

#[test]
fn console_formatting_never_calls_object_getters_or_coercion_hooks() {
    let assets = Assets::new();
    let mut session = Session::new(assets.package(), 9);
    assert_eq!(session.script(r#"
        let hits = 0;
        const hostile = Object.defineProperties({}, {
            toString: { get() { hits++; throw Error('toString getter invoked'); } },
            valueOf: { get() { hits++; throw Error('valueOf getter invoked'); } },
            [Symbol.toPrimitive]: { get() { hits++; throw Error('coercion getter invoked'); } },
            [Symbol.toStringTag]: { get() { hits++; throw Error('tag getter invoked'); } },
            arbitrary: { get() { hits++; throw Error('arbitrary getter invoked'); } },
        });
        const proxy = new Proxy(hostile, {
            get() { hits++; throw Error('proxy get invoked'); },
            ownKeys() { hits++; throw Error('proxy ownKeys invoked'); },
        });
        const error = new Error('safe');
        Object.defineProperties(error, {
            message: { get() { hits++; throw Error('message getter invoked'); } },
            stack: { get() { hits++; throw Error('stack getter invoked'); } },
        });
        const fn = () => {};
        Object.defineProperty(fn, 'toString', { get() { hits++; throw Error('function getter invoked'); } });
        const revoked = Proxy.revocable({}, {});
        revoked.revoke();
        for (const level of ['log', 'info', 'warn', 'error', 'debug']) {
            console[level](hostile, proxy, error, fn, revoked.proxy, [hostile], Symbol('safe'), 12n, null, undefined, true, 42, 'héllo');
            console[level]('é'.repeat(4096), hostile);
        }
        hits;
    "#), "0");
    assert_eq!(session.work.counts(), (0, 0, 0));
}
