use std::{cell::{Cell, RefCell}, collections::{HashSet, VecDeque}, error::Error, rc::Rc, sync::Arc, time::Instant};
use js_runtime::{JsRuntime, RuntimeLimits};
use rquickjs::{Exception, Function, Persistent};
use ui_bridge::{DeferredEvent, UiBridge, registry::NodeRegistry};
use zgui::view::host::TimerId;
use zgui::platform::{SurfaceId, WakeReason, Waker};
use crate::{MountRegion, package::Package, work::{Arguments, Work}};
pub(crate) use crate::work::ContractFailure;
use crate::work::ServicePolicy;

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;

/// Execution and provider policy is fixed before the application module is evaluated.
pub(crate) enum SessionPolicy {
    Live,
    Preview {
        limits: RuntimeLimits,
        deadline: Rc<Cell<Instant>>,
        environment: Rc<crate::preview::clock::Environment>,
        fixtures: Rc<crate::work::fixture::FixtureRuntime>,
    },
}

#[derive(Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResourceCounts {
    pub nodes: usize,
    pub wrappers: usize,
    pub callbacks: usize,
    pub timers: usize,
    pub frame_callbacks: usize,
    pub requests: usize,
    pub subscriptions: usize,
    pub engine_alive: bool,
}

pub(crate) struct Session {
    // Roots and bridge are declared before engine; this order is also safe on unwind.
    disposer: RefCell<Option<Persistent<Function<'static>>>>,
    pub(crate) bridge: Rc<UiBridge>,
    work: Rc<Work>,
    engine: RefCell<Option<JsRuntime>>,
    shutdown_started: Cell<bool>,
    mount_started: Cell<bool>,
    stylesheet_installed: Cell<bool>,
    pub(crate) generation: u64,
    pub(crate) region: MountRegion,
    pub(crate) timer: Cell<Option<TimerId>>,
    pub(crate) frames: Cell<usize>,
    package: Option<Rc<Package>>,
    deferred: RefCell<VecDeque<DeferredEvent>>,
    fault: RefCell<Option<Box<dyn Error>>>,
    cleanup_failure: RefCell<Option<js_runtime::Diagnostic>>,
    policy: SessionPolicy,
    checkpoint_cursor: Cell<usize>,
    waker: Arc<dyn Waker>,
    surface: SurfaceId,
}

impl Session {
    /// Pure module preflight: bridge mutation is disabled until mount's turn.
    pub(crate) fn prepare(region: MountRegion, package: Option<Rc<Package>>, generation: u64, waker: Arc<dyn Waker>, surface: SurfaceId, executor: host_services::Executor, policy: SessionPolicy) -> Result<Rc<Self>> {
        let (limits, service_policy) = match &policy {
            SessionPolicy::Live => (RuntimeLimits::default(), ServicePolicy::Live),
            SessionPolicy::Preview { limits, fixtures, .. } => (limits.clone(), ServicePolicy::Fixture(fixtures.clone())),
        };
        let mut engine = JsRuntime::new(limits)?;
        if let SessionPolicy::Preview { deadline, .. } = &policy { engine.set_enclosing_deadline(deadline.clone()); }
        let bridge = UiBridge::new(NodeRegistry::new(region.dom.clone(), region.root, 1, generation, 100_000), region.host.clone());
        let work = Work::new(region.host.clone(), package.clone(), generation, executor, waker.clone(), surface, service_policy);
        engine.with_context(|ctx| {
            if let SessionPolicy::Preview { environment, .. } = &policy { environment.install(&ctx)?; }
            bridge.install(&ctx)?;
            work.install(&ctx)?;
            ctx.globals().set("mountRoot", bridge.root(&ctx)?)
        })?;
        if let Some(package) = &package {
            let loaded = engine.load_module(&package.directory.join(&package.manifest.entry).to_string_lossy(), &package.source);
            if let Some(failure) = work.contract_failure() { return Err(failure.into()); }
            loaded.map_err(|error| package.remap_diagnostic(error))?;
            engine.with_module(|ctx, module| {
                let version: rquickjs::Value = module.get("hostApiVersion")?;
                if version.as_int() != Some(1) { return Err(Exception::throw_type(&ctx, "module hostApiVersion must equal manifest version 1")); }
                let _: Function = module.get("mount")?;
                Ok(())
            }).map_err(|error| package.remap_diagnostic(error))?;
            // Drain bootstrap continuations while every host-creation guard is
            // still closed. Otherwise a top-level Promise could defer side effects
            // until the first mounted checkpoint and bypass preflight.
            let jobs = engine.pump_jobs();
            if let Some(failure) = work.contract_failure() { return Err(failure.into()); }
            if let Some(error) = jobs.diagnostics.into_iter().next() { return Err(package.remap_diagnostic(error).into()); }
            if jobs.has_pending_jobs { return Err("preflight jobs exceeded their bounded pump".into()); }
            let rejections = drain_rejections(&mut engine, true, Some(package));
            if let Some(failure) = work.contract_failure() { return Err(failure.into()); }
            rejections?;
            if engine.is_job_pending() { return Err("preflight diagnostic accessors scheduled more jobs".into()); }
        }
        let session = Rc::new(Self { disposer: RefCell::new(None), bridge, work, engine: RefCell::new(Some(engine)), shutdown_started: Cell::new(false), mount_started: Cell::new(false), stylesheet_installed: Cell::new(false), generation, region, timer: Cell::new(None), frames: Cell::new(0), package, deferred: RefCell::new(VecDeque::new()), fault: RefCell::new(None), cleanup_failure: RefCell::new(None), policy, checkpoint_cursor: Cell::new(0), waker, surface });
        let weak = Rc::downgrade(&session);
        session.bridge.set_dispatcher(Rc::new(move |id, event| {
            if let Some(session) = weak.upgrade() { session.dispatch(id, event); }
        }));
        Ok(session)
    }

    pub(crate) fn mount(&self) -> Result<()> {
        if self.shutdown_started.get() { return Err("cannot mount a retired session".into()); }
        if self.mount_started.replace(true) { return Err("session mount was already attempted".into()); }
        self.work.mounting();
        if let Some(package) = &self.package {
            self.region.host.install_stylesheet("application", &package.css);
            self.stylesheet_installed.set(true);
            let mut engine = self.engine.borrow_mut();
            let engine = engine.as_mut().ok_or("session disposed")?;
            let mounted = self.bridge.run_turn(|| engine.with_module(|ctx, module| {
                let mount: Function = module.get("mount")?;
                let disposer: Function = mount.call((self.bridge.root(&ctx)?,))?;
                Ok(Persistent::save(&ctx, disposer))
            }));
            match mounted {
                Ok(disposer) => *self.disposer.borrow_mut() = Some(disposer),
                Err(error) => {
                    self.check_contract()?;
                    return Err(self.diagnostic(error).into());
                }
            }
            self.check_contract()?;
            eprintln!("mounted: package={} build={} nodes={}", package.manifest.id, package.manifest.build_id, self.bridge.live_count());
            self.snapshot();
        } else {
            self.update("globalThis.textNode = __zgui.createTextNode('JavaScript mounted'); __zgui.insertNode(mountRoot, textNode, null); 'JavaScript mounted'")?;
        }
        self.work.mounted();
        self.check_contract()?;
        Ok(())
    }

    fn wake_owner(&self) {
        self.waker.wake(WakeReason::ReactiveWork { surfaces: vec![self.surface].into_boxed_slice() });
    }

    fn dispatch(&self, id: u64, event: &mut zgui::view::EventCx<'_>) {
        if self.shutdown_started.get() || self.bridge.is_disposed() { return; }
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
        if let Err(error) = result { *self.fault.borrow_mut() = Some(self.diagnostic(error).into()); }
        // Cancellation is already applied and revoked before owner work is scheduled.
        self.wake_owner();
    }

    pub(crate) fn checkpoint(&self) -> Result<bool> {
        let result = self.checkpoint_work();
        // A native provider guard can throw during any job/event entry. Preserve
        // its typed contract failure even when that entry returned a JS exception.
        self.check_contract()?;
        result
    }

    fn checkpoint_work(&self) -> Result<bool> {
        if let Some(error) = self.fault.borrow_mut().take() { return Err(error); }
        self.check_contract()?;
        if self.bridge.is_disposed() { return Ok(false); }
        let Ok(mut engine) = self.engine.try_borrow_mut() else { self.wake_owner(); return Ok(false); };
        let Some(engine) = engine.as_mut() else { return Ok(false); };
        let end = self.deadline(Instant::now() + engine.limits().job_timeout);
        if Instant::now() >= end {
            return Err(js_runtime::Diagnostic {
                kind: js_runtime::DiagnosticKind::BudgetExceeded, phase: "checkpoint",
                message: "enclosing deadline exceeded".into(), stack: None, generated_stack: None,
            }.into());
        }
        let max_jobs = engine.limits().max_jobs_per_pump;
        let mut worked = false;
        let mut remaining = [32, 32, 32, 32, max_jobs];
        // Interleave work classes and preserve the next class across deadlines.
        // A continuously replenished IPC queue must not starve its own cancellation job.
        while Instant::now() < end && remaining.iter().any(|&count| count != 0) {
            let phase = self.checkpoint_cursor.get();
            self.checkpoint_cursor.set((phase + 1) % remaining.len());
            if remaining[phase] == 0 { continue; }
            remaining[phase] -= 1;
            let attempted = match phase {
                0 => if let Some(completion) = self.work.next_completion() {
                    self.bridge.run_turn(|| engine.with_context(|ctx| self.work.settle(&ctx, completion))).map_err(|error| self.diagnostic(error))?;
                    true
                } else { false },
                1 => if let Some(delivery) = self.work.next_subscription() {
                    self.bridge.run_turn(|| engine.with_context(|ctx| self.work.deliver_subscription(&ctx, delivery))).map_err(|error| self.diagnostic(error))?;
                    true
                } else { false },
                2 => if let Some(call) = self.work.next_call() {
                    self.bridge.run_turn(|| engine.with_context(|ctx| {
                        let callback = call.callback.restore(&ctx)?;
                        match call.arguments { Arguments::Timer => callback.call::<_, ()>(()), Arguments::Frame(timestamp) => callback.call::<_, ()>((timestamp,)) }
                    })).map_err(|error| self.diagnostic(error))?;
                    true
                } else { false },
                3 => {
                    let event = self.deferred.borrow_mut().pop_front();
                    if let Some(event) = event {
                        self.bridge.run_turn(|| engine.with_context(|ctx| self.bridge.invoke_deferred(&ctx, event))).map_err(|error| self.diagnostic(error))?;
                        true
                    } else { false }
                },
                _ => self.bridge.run_turn(|| engine.execute_job(end.saturating_duration_since(Instant::now())))
                    .map_err(|error| self.diagnostic(error))?,
            };
            if attempted { worked = true; } else { remaining[phase] = 0; }
        }
        worked |= self.bridge.run_turn(|| drain_rejections(engine, self.is_preview(), self.package.as_deref()))?;
        self.check_contract()?;
        if engine.is_job_pending() || !self.deferred.borrow().is_empty() || self.work.has_ready() || self.work.has_completions() || self.work.has_subscriptions() { self.wake_owner(); }
        Ok(worked)
    }

    pub(crate) fn frame_checkpoint(&self) {
        if let Err(error) = self.checkpoint() {
            *self.fault.borrow_mut() = Some(error);
            self.wake_owner();
        }
    }

    fn diagnostic(&self, error: js_runtime::Diagnostic) -> js_runtime::Diagnostic {
        match &self.package { Some(package) => package.remap_diagnostic(error), None => error }
    }

    fn is_preview(&self) -> bool { matches!(&self.policy, SessionPolicy::Preview { .. }) }

    fn deadline(&self, end: Instant) -> Instant {
        match &self.policy { SessionPolicy::Live => end, SessionPolicy::Preview { deadline, .. } => end.min(deadline.get()) }
    }

    /// A provider violation is sticky: catching the JavaScript exception cannot
    /// turn an attempted live-service access into a successful preview.
    pub(crate) fn contract_failure(&self) -> Option<ContractFailure> { self.work.contract_failure() }

    fn check_contract(&self) -> Result<()> {
        match self.contract_failure() { Some(failure) => Err(failure.into()), None => Ok(()) }
    }

    pub(crate) fn take_cleanup_failure(&self) -> Option<js_runtime::Diagnostic> {
        self.cleanup_failure.borrow_mut().take()
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

    pub(crate) fn record_fault(&self, error: String) {
        if self.shutdown_started.get() { return; }
        *self.fault.borrow_mut() = Some(error.into());
        self.wake_owner();
    }

    fn snapshot(&self) {
        eprintln!("native-text: {}", self.region.dom.text_content(self.region.root));
    }

    pub(crate) fn counts(&self) -> ResourceCounts {
        let (timers, frame_callbacks, requests, subscriptions) = self.work.counts();
        ResourceCounts {
            nodes: self.bridge.live_count(), wrappers: self.bridge.wrapper_count(),
            callbacks: self.bridge.callback_count(), timers, frame_callbacks, requests, subscriptions,
            engine_alive: self.engine.borrow().is_some(),
        }
    }

    pub(crate) fn shutdown(&self) {
        // An old Rc may outlive replacement. A repeated shutdown must not remove
        // the new generation's stylesheet or touch its native resources.
        if self.shutdown_started.replace(true) { return; }
        self.bridge.begin_stop();
        self.work.begin_stop();
        if let Some(timer) = self.timer.take() { self.region.host.cancel_timer(timer); }
        self.deferred.borrow_mut().clear();
        self.fault.borrow_mut().take();
        let disposer = self.disposer.borrow_mut().take();
        if let Some(disposer) = disposer {
            if let Some(engine) = self.engine.borrow_mut().as_mut() {
                let cleanup = || engine.with_cleanup_context(|ctx| disposer.restore(&ctx)?.call::<_, ()>(()));
                // A failed structural turn has already retired native handles.
                // Still run pure JS cleanup, without reopening a disposed registry.
                let result = if self.bridge.is_disposed() { cleanup() } else { self.bridge.run_turn(cleanup) };
                if let Err(error) = result {
                    let error = self.diagnostic(error);
                    eprintln!("disposer error: {error}");
                    *self.cleanup_failure.borrow_mut() = Some(error);
                }
                if self.is_preview() {
                    if let Err(error) = drain_rejections(engine, true, self.package.as_deref()) {
                        self.cleanup_failure.borrow_mut().get_or_insert(error);
                    }
                }
            }
        }
        self.work.finish_stop();
        self.bridge.dispose();
        if self.stylesheet_installed.replace(false) { self.region.host.remove_stylesheet("application"); }
        if self.engine.borrow_mut().take().is_some() {
            eprintln!("session-disposed: generation={} frames={} resources={:?}", self.generation, self.frames.get(), self.counts());
        }
    }
}
impl Drop for Session { fn drop(&mut self) { self.shutdown(); } }

/// Reduce both transitions of an identity before deciding whether it is unhandled.
/// Formatting can produce further updates, so strict policy drains to quiescence
/// within a bounded number of batches instead of losing accessor-created failures.
fn drain_rejections(engine: &mut JsRuntime, strict: bool, package: Option<&Package>) -> std::result::Result<bool, js_runtime::Diagnostic> {
    let mut worked = false;
    for _ in 0..=engine.limits().max_tracked_rejections {
        let rejections = engine.drain_rejections();
        worked |= !rejections.updates.is_empty();
        if rejections.dropped_rejections != 0 {
            if strict {
                return Err(js_runtime::Diagnostic {
                    kind: js_runtime::DiagnosticKind::BudgetExceeded, phase: "promise rejection",
                    message: "rejection tracking limit exceeded".into(), stack: None, generated_stack: None,
                });
            }
            eprintln!("promise-rejection: evicted={} (tracking limit)", rejections.dropped_rejections);
        }
        let empty = rejections.updates.is_empty();
        if !strict {
            for rejection in rejections.updates {
                let diagnostic = match package { Some(package) => package.remap_diagnostic(rejection.diagnostic), None => rejection.diagnostic };
                eprintln!("promise-rejection: id={} handled={} {diagnostic}", rejection.id, rejection.handled);
            }
            return Ok(worked);
        }
        let mut seen = HashSet::new();
        for rejection in rejections.updates.into_iter().rev() {
            if !seen.insert(rejection.id) { continue; }
            let diagnostic = match package { Some(package) => package.remap_diagnostic(rejection.diagnostic), None => rejection.diagnostic };
            if !rejection.handled { return Err(diagnostic); }
        }
        if empty { return Ok(worked); }
    }
    Err(js_runtime::Diagnostic {
        kind: js_runtime::DiagnosticKind::BudgetExceeded, phase: "promise rejection",
        message: "rejection diagnostics did not quiesce".into(), stack: None, generated_stack: None,
    })
}

#[cfg(test)]
mod tests;
