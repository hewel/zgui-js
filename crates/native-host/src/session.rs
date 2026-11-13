use std::{cell::{Cell, RefCell}, collections::VecDeque, error::Error, rc::Rc, sync::Arc, time::Instant};
use js_runtime::{JsRuntime, RuntimeLimits};
use rquickjs::{Exception, Function, Persistent};
use ui_bridge::{DeferredEvent, UiBridge, registry::NodeRegistry};
use zgui::view::host::TimerId;
use zgui::platform::{SurfaceId, WakeReason, Waker};
use crate::{MountRegion, package::Package, work::{Arguments, Work}};

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;

pub(crate) struct Session {
    // Roots and bridge are declared before engine; this order is also safe on unwind.
    disposer: RefCell<Option<Persistent<Function<'static>>>>,
    pub(crate) bridge: Rc<UiBridge>,
    work: Rc<Work>,
    engine: RefCell<Option<JsRuntime>>,
    pub(crate) region: MountRegion,
    pub(crate) timer: Cell<Option<TimerId>>,
    pub(crate) frames: Cell<usize>,
    package: Option<Rc<Package>>,
    deferred: RefCell<VecDeque<DeferredEvent>>,
    fault: RefCell<Option<String>>,
    waker: Arc<dyn Waker>,
    surface: SurfaceId,
}

impl Session {
    /// Pure module preflight: bridge mutation is disabled until mount's turn.
    pub(crate) fn prepare(region: MountRegion, package: Option<Rc<Package>>, generation: u64, waker: Arc<dyn Waker>, surface: SurfaceId, executor: host_services::Executor) -> Result<Rc<Self>> {
        let mut engine = JsRuntime::new(RuntimeLimits::default())?;
        let bridge = UiBridge::new(NodeRegistry::new(region.dom.clone(), region.root, 1, generation, 100_000));
        let work = Work::new(region.host.clone(), package.clone(), generation, executor, waker.clone(), surface);
        engine.with_context(|ctx| {
            bridge.install(&ctx)?;
            work.install(&ctx)?;
            ctx.globals().set("mountRoot", bridge.root(&ctx)?)
        })?;
        if let Some(package) = &package {
            engine.load_module(&package.directory.join(&package.manifest.entry).to_string_lossy(), &package.source).map_err(|error| package.remap_diagnostic(error))?;
            engine.with_module(|ctx, module| {
                let version: rquickjs::Value = module.get("hostApiVersion")?;
                if version.as_int() != Some(1) { return Err(Exception::throw_type(&ctx, "module hostApiVersion must equal manifest version 1")); }
                let _: Function = module.get("mount")?;
                Ok(())
            }).map_err(|error| package.remap_diagnostic(error))?;
        }
        let session = Rc::new(Self { disposer: RefCell::new(None), bridge, work, engine: RefCell::new(Some(engine)), region, timer: Cell::new(None), frames: Cell::new(0), package, deferred: RefCell::new(VecDeque::new()), fault: RefCell::new(None), waker, surface });
        let weak = Rc::downgrade(&session);
        session.bridge.set_dispatcher(Rc::new(move |id, event| {
            if let Some(session) = weak.upgrade() { session.dispatch(id, event); }
        }));
        Ok(session)
    }

    pub(crate) fn mount(&self) -> Result<()> {
        self.work.mounting();
        if let Some(package) = &self.package {
            self.region.host.install_stylesheet("application", &package.css);
            let mut engine = self.engine.borrow_mut();
            let engine = engine.as_mut().ok_or("session disposed")?;
            let disposer = self.bridge.run_turn(|| engine.with_module(|ctx, module| {
                let mount: Function = module.get("mount")?;
                let disposer: Function = mount.call((self.bridge.root(&ctx)?,))?;
                Ok(Persistent::save(&ctx, disposer))
            })).map_err(|error| self.diagnostic(error))?;
            *self.disposer.borrow_mut() = Some(disposer);
            eprintln!("mounted: package={} build={} nodes={}", package.manifest.id, package.manifest.build_id, self.bridge.live_count());
            self.snapshot();
        } else {
            self.update("globalThis.textNode = __zgui.createTextNode('JavaScript mounted'); __zgui.insertNode(mountRoot, textNode, null); 'JavaScript mounted'")?;
        }
        self.work.mounted();
        Ok(())
    }

    fn wake_owner(&self) {
        self.waker.wake(WakeReason::ReactiveWork { surfaces: vec![self.surface].into_boxed_slice() });
    }

    fn dispatch(&self, id: u64, event: &mut zgui::view::EventCx<'_>) {
        if self.bridge.is_disposed() { return; }
        let Ok(mut engine) = self.engine.try_borrow_mut() else {
            let mut queue = self.deferred.borrow_mut();
            if queue.len() == 256 {
                *self.fault.borrow_mut() = Some("nested native notification queue exhausted".into());
            } else {
                queue.push_back(DeferredEvent::capture(id, event));
            }
            self.wake_owner();
            return;
        };
        let Some(engine) = engine.as_mut() else { return; };
        let result = self.bridge.run_turn(|| engine.with_context(|ctx| self.bridge.invoke_event(&ctx, id, event)));
        if let Err(error) = result { *self.fault.borrow_mut() = Some(format!("event callback {id}: {}", self.diagnostic(error))); }
        // Cancellation is already applied and revoked before owner work is scheduled.
        self.wake_owner();
    }

    pub(crate) fn checkpoint(&self) -> Result<bool> {
        if let Some(error) = self.fault.borrow_mut().take() { return Err(error.into()); }
        if self.bridge.is_disposed() { return Ok(false); }
        let Ok(mut engine) = self.engine.try_borrow_mut() else { self.wake_owner(); return Ok(false); };
        let Some(engine) = engine.as_mut() else { return Ok(false); };
        let end = Instant::now() + engine.limits().job_timeout;
        let max_jobs = engine.limits().max_jobs_per_pump;
        let mut worked = false;
        for _ in 0..32 {
            if Instant::now() >= end { break; }
            let Some(completion) = self.work.next_completion() else { break; };
            self.bridge.run_turn(|| engine.with_context(|ctx| self.work.settle(&ctx, completion))).map_err(|error| self.diagnostic(error))?;
            worked = true;
        }
        for _ in 0..32 {
            if Instant::now() >= end { break; }
            let Some(call) = self.work.next_call() else { break; };
            self.bridge.run_turn(|| engine.with_context(|ctx| {
                let callback = call.callback.restore(&ctx)?;
                match call.arguments { Arguments::Timer => callback.call::<_, ()>(()), Arguments::Frame(timestamp) => callback.call::<_, ()>((timestamp,)) }
            })).map_err(|error| self.diagnostic(error))?;
            worked = true;
        }
        for _ in 0..32 {
            if Instant::now() >= end { break; }
            let Some(event) = self.deferred.borrow_mut().pop_front() else { break; };
            self.bridge.run_turn(|| engine.with_context(|ctx| self.bridge.invoke_deferred(&ctx, event))).map_err(|error| self.diagnostic(error))?;
            worked = true;
        }
        for _ in 0..max_jobs {
            if Instant::now() >= end { break; }
            let attempted = self.bridge.run_turn(|| engine.execute_job(end.saturating_duration_since(Instant::now())))
                .map_err(|error| self.diagnostic(error))?;
            if !attempted { break; }
            worked = true;
        }
        let rejections = self.bridge.run_turn(|| Ok::<_, std::convert::Infallible>(engine.drain_rejections())).unwrap();
        worked |= !rejections.updates.is_empty();
        for rejection in rejections.updates {
            eprintln!("promise-rejection: id={} handled={} {}", rejection.id, rejection.handled, self.diagnostic(rejection.diagnostic));
        }
        if rejections.dropped_rejections != 0 { eprintln!("promise-rejection: evicted={} (tracking limit)", rejections.dropped_rejections); }
        if engine.is_job_pending() || !self.deferred.borrow().is_empty() || self.work.has_ready() || self.work.has_completions() { self.wake_owner(); }
        Ok(worked)
    }

    pub(crate) fn frame_checkpoint(&self) {
        if let Err(error) = self.checkpoint() {
            *self.fault.borrow_mut() = Some(error.to_string());
            self.wake_owner();
        }
    }

    fn diagnostic(&self, error: js_runtime::Diagnostic) -> js_runtime::Diagnostic {
        match &self.package { Some(package) => package.remap_diagnostic(error), None => error }
    }

    pub(crate) fn advance(&self, external: bool) -> Result<()> {
        if self.package.is_some() {
            let mut engine = self.engine.borrow_mut();
            let engine = engine.as_mut().ok_or("session disposed")?;
            self.bridge.run_turn(|| engine.with_module(|_, module| {
                let advance: Function = module.get("advance")?;
                advance.call::<_, ()>(())
            })).map_err(|error| self.diagnostic(error))?;
            self.snapshot();
            Ok(())
        } else if external {
            self.update("__zgui.replaceText(textNode, 'External wake updated JavaScript'); 'External wake updated JavaScript'")
        } else {
            self.update("__zgui.replaceText(textNode, 'Native timer updated JavaScript'); 'Native timer updated JavaScript'")
        }
    }

    fn update(&self, source: &str) -> Result<()> {
        let mut engine = self.engine.borrow_mut();
        let engine = engine.as_mut().ok_or("session disposed")?;
        let value = self.bridge.run_turn(|| engine.evaluate_script(source)).map_err(|error| self.diagnostic(error))?;
        if self.region.dom.text_content(self.region.root) != value { return Err("native text does not match JavaScript result".into()); }
        self.snapshot();
        Ok(())
    }

    fn snapshot(&self) {
        eprintln!("native-text: {}", self.region.dom.text_content(self.region.root));
    }

    pub(crate) fn shutdown(&self) {
        self.work.begin_stop();
        if let Some(timer) = self.timer.take() { self.region.host.cancel_timer(timer); }
        self.deferred.borrow_mut().clear();
        let disposer = self.disposer.borrow_mut().take();
        if let Some(disposer) = disposer {
            if !self.bridge.is_disposed() && let Some(engine) = self.engine.borrow_mut().as_mut() {
                let result = self.bridge.run_turn(|| engine.with_cleanup_context(|ctx| disposer.restore(&ctx)?.call::<_, ()>(() )));
                if let Err(error) = result { eprintln!("disposer error: {}", self.diagnostic(error)); }
            }
        }
        self.work.finish_stop();
        self.bridge.dispose();
        self.region.host.remove_stylesheet("application");
        if self.engine.borrow_mut().take().is_some() {
            let (timers, frames, requests) = self.work.counts();
            eprintln!("session-disposed: frames={} nodes={} wrappers={} callbacks={} timers={} frame_callbacks={} requests={}", self.frames.get(), self.bridge.live_count(), self.bridge.wrapper_count(), self.bridge.callback_count(), timers, frames, requests);
        }
    }
}
impl Drop for Session { fn drop(&mut self) { self.shutdown(); } }
