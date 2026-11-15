use std::{cell::{Cell, RefCell}, collections::{HashSet, VecDeque}, rc::Rc, sync::Arc, time::Instant};
use js_runtime::{JsRuntime, RuntimeLimits};
use rquickjs::{Exception, Function, Persistent};
use ui_bridge::{DeferredEvent, UiBridge, registry::NodeRegistry};
use zgui::platform::{SurfaceId, WakeReason, Waker};
use crate::{assets::{AssetSource, ContractFailure}, environment::Environment, session_host::MountRegion, package::Package, work::{Arguments, Work}};

/// Typed session failure: engine diagnostics, sticky provider contracts, and
/// host invariants keep distinct categories through recovery classification.
#[derive(Debug)]
pub(crate) enum SessionError {
    Diagnostic(js_runtime::Diagnostic),
    Contract(ContractFailure),
    InvalidState(String),
}
impl std::fmt::Display for SessionError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Diagnostic(diagnostic) => diagnostic.fmt(out),
            Self::Contract(failure) => failure.fmt(out),
            Self::InvalidState(message) => out.write_str(message),
        }
    }
}
impl std::error::Error for SessionError {}
impl From<js_runtime::Diagnostic> for SessionError {
    fn from(error: js_runtime::Diagnostic) -> Self { Self::Diagnostic(error) }
}
impl From<ContractFailure> for SessionError {
    fn from(error: ContractFailure) -> Self { Self::Contract(error) }
}
impl From<String> for SessionError {
    fn from(error: String) -> Self { Self::InvalidState(error) }
}
impl From<&str> for SessionError {
    fn from(error: &str) -> Self { Self::InvalidState(error.into()) }
}
pub(crate) type Result<T> = std::result::Result<T, SessionError>;

/// Everything one session generation loads and mounts. Package metadata is
/// optional; without it the module is the host-owned integration baseline.
#[derive(Clone)]
pub(crate) struct SessionModule {
    pub name: String,
    pub source: String,
    pub css: String,
    pub package: Option<Rc<Package>>,
}
impl SessionModule {
    pub(crate) fn package(package: Rc<Package>) -> Self {
        Self {
            name: package.directory.join(&package.manifest.entry).to_string_lossy().into_owned(),
            source: package.source.clone(),
            css: package.css.clone(),
            package: Some(package),
        }
    }
}

/// Execution and provider policy is fixed before the application module is evaluated.
#[derive(Clone)]
pub(crate) struct SessionPolicy {
    limits: RuntimeLimits,
    deadline: Option<Rc<Cell<Instant>>>,
    environment: Option<Rc<Environment>>,
    strict_rejections: bool,
    assets: AssetSource,
}
impl SessionPolicy {
    /// Ordinary live execution: wall clock, ambient limits, logged rejections,
    /// package asset reads on the shared worker pool.
    pub(crate) fn live(executor: host_services::Executor) -> Self {
        Self { limits: RuntimeLimits::default(), deadline: None, environment: None, strict_rejections: false, assets: AssetSource::Live(executor) }
    }
    /// Deterministic preview execution: supplied limits, an enclosing deadline,
    /// the virtual environment, strict rejection handling and fixture assets.
    pub(crate) fn preview(limits: RuntimeLimits, deadline: Rc<Cell<Instant>>, environment: Rc<Environment>, fixtures: Rc<crate::assets::fixture::FixtureRuntime>) -> Self {
        Self { limits, deadline: Some(deadline), environment: Some(environment), strict_rejections: true, assets: AssetSource::Fixture(fixtures) }
    }
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
    pub(crate) frames: Cell<usize>,
    module: SessionModule,
    deferred: RefCell<VecDeque<DeferredEvent>>,
    fault: RefCell<Option<SessionError>>,
    cleanup_failure: RefCell<Option<js_runtime::Diagnostic>>,
    policy: SessionPolicy,
    checkpoint_cursor: Cell<usize>,
    waker: Arc<dyn Waker>,
    surface: SurfaceId,
}

impl Session {
    /// Pure module preflight: bridge mutation is disabled until mount's turn.
    pub(crate) fn prepare(region: MountRegion, module: SessionModule, generation: u64, waker: Arc<dyn Waker>, surface: SurfaceId, policy: SessionPolicy) -> Result<Rc<Self>> {
        let mut engine = JsRuntime::new(policy.limits.clone())?;
        if let Some(deadline) = &policy.deadline { engine.set_enclosing_deadline(deadline.clone()); }
        let bridge = UiBridge::new(NodeRegistry::new(region.dom.clone(), region.root, 1, generation, 100_000), region.host.clone());
        let work = Work::new(region.host.clone(), module.package.clone(), generation, waker.clone(), surface, policy.assets.clone());
        engine.with_context(|ctx| {
            if let Some(environment) = &policy.environment { environment.install(&ctx)?; }
            bridge.install(&ctx)?;
            work.install(&ctx)?;
            ctx.globals().set("mountRoot", bridge.root(&ctx)?)
        })?;
        let loaded = engine.load_module(&module.name, &module.source);
        if let Some(failure) = work.contract_failure() { return Err(failure.into()); }
        loaded.map_err(|error| remap(&module, error))?;
        engine.with_module(|ctx, loaded| {
            let version: rquickjs::Value = loaded.get("hostApiVersion")?;
            if version.as_int() != Some(1) { return Err(Exception::throw_type(&ctx, "module hostApiVersion must equal manifest version 1")); }
            let _: Function = loaded.get("mount")?;
            Ok(())
        }).map_err(|error| remap(&module, error))?;
        // Drain bootstrap continuations while every host-creation guard is
        // still closed. Otherwise a top-level Promise could defer side effects
        // until the first mounted checkpoint and bypass preflight.
        let jobs = engine.pump_jobs();
        if let Some(failure) = work.contract_failure() { return Err(failure.into()); }
        if let Some(error) = jobs.diagnostics.into_iter().next() { return Err(remap(&module, error)); }
        if jobs.has_pending_jobs {
            return Err(js_runtime::Diagnostic {
                kind: js_runtime::DiagnosticKind::BudgetExceeded, phase: "preflight",
                message: "preflight jobs exceeded their bounded pump".into(), stack: None, generated_stack: None,
            }.into());
        }
        let rejections = drain_rejections(&mut engine, true, module.package.as_deref());
        if let Some(failure) = work.contract_failure() { return Err(failure.into()); }
        rejections?;
        if engine.is_job_pending() {
            return Err(js_runtime::Diagnostic {
                kind: js_runtime::DiagnosticKind::BudgetExceeded, phase: "preflight",
                message: "preflight diagnostic accessors scheduled more jobs".into(), stack: None, generated_stack: None,
            }.into());
        }
        let session = Rc::new(Self { disposer: RefCell::new(None), bridge, work, engine: RefCell::new(Some(engine)), shutdown_started: Cell::new(false), mount_started: Cell::new(false), stylesheet_installed: Cell::new(false), generation, region, frames: Cell::new(0), module, deferred: RefCell::new(VecDeque::new()), fault: RefCell::new(None), cleanup_failure: RefCell::new(None), policy, checkpoint_cursor: Cell::new(0), waker, surface });
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
        if !self.module.css.is_empty() {
            self.region.host.install_stylesheet("application", &self.module.css);
            self.stylesheet_installed.set(true);
        }
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
        if let Some(package) = &self.module.package {
            eprintln!("mounted: package={} build={} nodes={}", package.manifest.id, package.manifest.build_id, self.bridge.live_count());
        }
        self.snapshot();
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
        let mut remaining = [32, 32, 32, max_jobs];
        // Interleave work classes and preserve the next class across deadlines.
        // Ready native work must not starve promise jobs that can cancel it.
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
                1 => if let Some(call) = self.work.next_call() {
                    self.bridge.run_turn(|| engine.with_context(|ctx| {
                        let callback = call.callback.restore(&ctx)?;
                        match call.arguments { Arguments::Timer => callback.call::<_, ()>(()), Arguments::Frame(timestamp) => callback.call::<_, ()>((timestamp,)) }
                    })).map_err(|error| self.diagnostic(error))?;
                    true
                } else { false },
                2 => {
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
        worked |= self.bridge.run_turn(|| drain_rejections(engine, self.policy.strict_rejections, self.module.package.as_deref()))?;
        self.check_contract()?;
        if engine.is_job_pending() || !self.deferred.borrow().is_empty() || self.work.has_ready() || self.work.has_completions() { self.wake_owner(); }
        Ok(worked)
    }

    pub(crate) fn frame_checkpoint(&self) {
        if self.shutdown_started.get() { return; }
        if let Err(error) = self.checkpoint() {
            *self.fault.borrow_mut() = Some(error);
            self.wake_owner();
        }
    }

    fn diagnostic(&self, error: js_runtime::Diagnostic) -> js_runtime::Diagnostic {
        match &self.module.package { Some(package) => package.remap_diagnostic(error), None => error }
    }

    fn deadline(&self, end: Instant) -> Instant {
        match &self.policy.deadline { Some(deadline) => end.min(deadline.get()), None => end }
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

    /// Integration harness entries ('advance', 'external') run as one bridge
    /// turn against the mounted module, with the same contract and diagnostic
    /// mapping as mount.
    pub(crate) fn invoke(&self, export: &str) -> Result<()> {
        if self.shutdown_started.get() { return Err("cannot invoke a retired session".into()); }
        let mut engine = self.engine.borrow_mut();
        let engine = engine.as_mut().ok_or("session disposed")?;
        let result = self.bridge.run_turn(|| engine.with_module(|_, module| {
            let callback: Function = module.get(export)?;
            callback.call::<_, ()>(())
        }));
        match result {
            Ok(()) => {}
            Err(error) => {
                self.check_contract()?;
                return Err(self.diagnostic(error).into());
            }
        }
        self.check_contract()?;
        self.snapshot();
        Ok(())
    }

    pub(crate) fn record_fault(&self, error: SessionError) {
        if self.shutdown_started.get() { return; }
        *self.fault.borrow_mut() = Some(error);
        self.wake_owner();
    }

    pub(crate) fn package(&self) -> Option<&Rc<Package>> { self.module.package.as_ref() }
    pub(crate) fn surface(&self) -> SurfaceId { self.surface }

    fn snapshot(&self) {
        eprintln!("native-text: {}", self.region.dom.text_content(self.region.root));
    }

    pub(crate) fn counts(&self) -> ResourceCounts {
        let (timers, frame_callbacks, requests) = self.work.counts();
        ResourceCounts {
            nodes: self.bridge.live_count(), wrappers: self.bridge.wrapper_count(),
            callbacks: self.bridge.callback_count(), timers, frame_callbacks, requests,
            engine_alive: self.engine.borrow().is_some(),
        }
    }

    pub(crate) fn shutdown(&self) {
        // An old Rc may outlive replacement. A repeated shutdown must not remove
        // the new generation's stylesheet or touch its native resources.
        if self.shutdown_started.replace(true) { return; }
        self.bridge.begin_stop();
        self.work.begin_stop();
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
                if self.policy.strict_rejections {
                    if let Err(error) = drain_rejections(engine, true, self.module.package.as_deref()) {
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

fn remap(module: &SessionModule, error: js_runtime::Diagnostic) -> SessionError {
    SessionError::Diagnostic(match &module.package {
        Some(package) => package.remap_diagnostic(error),
        None => error,
    })
}

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
