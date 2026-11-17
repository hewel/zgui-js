use std::{cell::{Cell, RefCell}, rc::{Rc, Weak}, sync::Arc, time::{Duration, Instant}};
use js_runtime::{JsRuntime, RejectionPolicy, RuntimeLimits};
use rquickjs::{Exception, Function, Persistent};
use ui_bridge::{BridgeStep, DispatchOutcome, HostPort, QueueOverflow, UiBridge, registry::NodeRegistry};
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

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResourceCounts {
    pub nodes: usize,
    pub wrappers: usize,
    pub callbacks: usize,
    pub focus_traps: usize,
    pub scroll_locks: usize,
    pub timers: usize,
    pub frame_callbacks: usize,
    pub requests: usize,
    pub engine_alive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RetirementFailure {
    Contract(ContractFailure),
    Cleanup(js_runtime::Diagnostic),
}

#[derive(Clone, Debug)]
pub(crate) struct SessionRetirement {
    pub generation: u64,
    pub before: ResourceCounts,
    pub counts: ResourceCounts,
    pub failure: Option<RetirementFailure>,
    pub telemetry: Option<RuntimeTelemetry>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RuntimeTelemetry {
    pub max_entry_duration: Duration,
    pub js_heap_bytes: u64,
    pub js_stack_limit_bytes: usize,
}

impl RuntimeTelemetry {
    pub(crate) fn merge(current: Option<Self>, next: Option<Self>) -> Option<Self> {
        match (current, next) {
            (None, other) | (other, None) => other,
            (Some(a), Some(b)) => Some(Self {
                max_entry_duration: a.max_entry_duration.max(b.max_entry_duration),
                js_heap_bytes: a.js_heap_bytes.max(b.js_heap_bytes),
                js_stack_limit_bytes: b.js_stack_limit_bytes.max(a.js_stack_limit_bytes),
            }),
        }
    }
}

pub(crate) struct Session {
    // Roots and bridge are declared before engine; this order is also safe on unwind.
    disposer: RefCell<Option<Persistent<Function<'static>>>>,
    pub(crate) bridge: Rc<UiBridge>,
    work: Rc<Work>,
    engine: RefCell<Option<JsRuntime>>,
    telemetry: Cell<Option<RuntimeTelemetry>>,
    shutdown_started: Cell<bool>,
    mount_started: Cell<bool>,
    stylesheet_installed: Cell<bool>,
    contract_reported: Cell<bool>,
    pub(crate) generation: u64,
    pub(crate) region: MountRegion,
    pub(crate) frames: Cell<usize>,
    module: SessionModule,
    fault: RefCell<Option<SessionError>>,
    retired: RefCell<Option<SessionRetirement>>,
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
        let session = Rc::new_cyclic(|weak: &Weak<Self>| {
            let port = HostPort::live(surface, waker.clone(), weak.clone(), Self::dispatch_step);
            let bridge = UiBridge::new(NodeRegistry::new(region.dom.clone(), region.root, 1, generation, 100_000), region.host.clone(), port);
            let work = Work::new(region.host.clone(), module.package.clone(), generation, waker.clone(), surface, policy.assets.clone());
            Self {
                disposer: RefCell::new(None), bridge, work, engine: RefCell::new(None),
                telemetry: Cell::new(None), shutdown_started: Cell::new(false),
                mount_started: Cell::new(false), stylesheet_installed: Cell::new(false),
                contract_reported: Cell::new(false),
                generation, region, frames: Cell::new(0), module,
                fault: RefCell::new(None), retired: RefCell::new(None),
                policy, checkpoint_cursor: Cell::new(0), waker, surface,
            }
        });
        engine.with_context(|ctx| {
            if let Some(environment) = &session.policy.environment { environment.install(&ctx)?; }
            session.bridge.install(&ctx)?;
            session.work.install(&ctx)?;
            ctx.globals().set("mountRoot", session.bridge.root(&ctx)?)
        })?;
        let loaded = engine.load_module(&session.module.name, &session.module.source);
        if let Some(failure) = session.work.contract_failure() { return Err(failure.into()); }
        loaded.map_err(|error| remap(&session.module, error))?;
        engine.with_module(|ctx, loaded| {
            let version: rquickjs::Value = loaded.get("hostApiVersion")?;
            if version.as_int() != Some(1) { return Err(Exception::throw_type(&ctx, "module hostApiVersion must equal manifest version 1")); }
            let _: Function = loaded.get("mount")?;
            Ok(())
        }).map_err(|error| remap(&session.module, error))?;
        // Drain bootstrap continuations while every host-creation guard is
        // still closed. Otherwise a top-level Promise could defer side effects
        // until the first mounted checkpoint and bypass preflight.
        let jobs = engine.pump_jobs();
        if let Some(failure) = session.work.contract_failure() { return Err(failure.into()); }
        if let Some(error) = jobs.diagnostics.into_iter().next() { return Err(remap(&session.module, error)); }
        if jobs.has_pending_jobs {
            return Err(js_runtime::Diagnostic {
                kind: js_runtime::DiagnosticKind::BudgetExceeded, phase: "preflight",
                message: "preflight jobs exceeded their bounded pump".into(), stack: None, generated_stack: None,
            }.into());
        }
        let rejections = engine.drain_rejections(RejectionPolicy::Strict).map_err(|error| remap(&session.module, error));
        if let Some(failure) = session.work.contract_failure() { return Err(failure.into()); }
        rejections?;
        if engine.is_job_pending() {
            return Err(js_runtime::Diagnostic {
                kind: js_runtime::DiagnosticKind::BudgetExceeded, phase: "preflight",
                message: "preflight diagnostic accessors scheduled more jobs".into(), stack: None, generated_stack: None,
            }.into());
        }
        *session.engine.borrow_mut() = Some(engine);
        Ok(session)
    }

    pub(crate) fn mount(&self) -> Result<()> {
        let result = self.mount_inner();
        self.mark_reported(result)
    }

    fn mount_inner(&self) -> Result<()> {
        if self.shutdown_started.get() { return Err("cannot mount a retired session".into()); }
        if self.mount_started.replace(true) { return Err("session mount was already attempted".into()); }
        self.work.mounting();
        if !self.module.css.is_empty() {
            self.region.host.install_stylesheet("application", &self.module.css);
            self.stylesheet_installed.set(true);
        }
        let mut engine = self.engine.borrow_mut();
        let engine = engine.as_mut().ok_or("session disposed")?;
        let mounted = self.bridge.run_turn(engine.turn(), |turn, js| js.with_module(|ctx, module| {
            let mount: Function = module.get("mount")?;
            let disposer: Function = mount.call((turn.root(&ctx)?,))?;
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

    fn dispatch_step(&self, step: BridgeStep<'_>) -> DispatchOutcome {
        if self.shutdown_started.get() || self.bridge.is_disposed() { return DispatchOutcome::Retired; }
        let Ok(mut engine) = self.engine.try_borrow_mut() else { return DispatchOutcome::Busy; };
        let Some(engine) = engine.as_mut() else { return DispatchOutcome::Retired; };
        if let Err(error) = step.run_turn(engine.turn(), |js, cb| js.with_context(|ctx| cb.call(&ctx))) {
            let failure = self.contract_failure().map(SessionError::from)
                .unwrap_or_else(|| self.diagnostic(error).into());
            *self.fault.borrow_mut() = Some(failure);
        }
        // Cancellation is already applied and revoked before owner work is scheduled.
        self.wake_owner();
        DispatchOutcome::Executed
    }

    pub(crate) fn checkpoint(&self) -> Result<bool> {
        let result = self.checkpoint_inner();
        self.mark_reported(result)
    }

    fn checkpoint_inner(&self) -> Result<bool> {
        let result = self.checkpoint_work();
        // A native provider guard can throw during any job/event entry. Preserve
        // its typed contract failure even when that entry returned a JS exception.
        self.check_contract()?;
        result
    }

    fn checkpoint_work(&self) -> Result<bool> {
        if let Some(error) = self.fault.borrow_mut().take() { return Err(error); }
        self.check_contract()?;
        if let Err(overflow) = self.bridge.has_queued_notifications() { return Err(overflow.to_string().into()); }
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
                    self.bridge.run_turn(engine.turn(), |_, js| js.with_context(|ctx| self.work.settle(&ctx, completion)))
                        .map_err(|error| self.diagnostic(error))?;
                    true
                } else { false },
                1 => if let Some(call) = self.work.next_call() {
                    self.bridge.run_turn(engine.turn(), |_, js| js.with_context(|ctx| {
                        let callback = call.callback.restore(&ctx)?;
                        match call.arguments { Arguments::Timer => callback.call::<_, ()>(()), Arguments::Frame(timestamp) => callback.call::<_, ()>((timestamp,)) }
                    })).map_err(|error| self.diagnostic(error))?;
                    true
                } else { false },
                2 => self.bridge.flush_notification(engine.turn(), |js, cb| js.with_context(|ctx| cb.call(&ctx)))
                    .map_err(|error| self.diagnostic(error))?,
                _ => self.bridge.run_turn(engine.turn(), |_, js| js.execute_job(end.saturating_duration_since(Instant::now())))
                    .map_err(|error| self.diagnostic(error))?,
            };
            if attempted { worked = true; } else { remaining[phase] = 0; }
        }
        let policy = if self.policy.strict_rejections { RejectionPolicy::Strict } else { RejectionPolicy::Log };
        let drain = engine.drain_rejections(policy).map_err(|error| self.diagnostic(error))?;
        worked |= !drain.updates.is_empty();
        if policy == RejectionPolicy::Log {
            if drain.dropped_rejections != 0 {
                eprintln!("promise-rejection: evicted={} (tracking limit)", drain.dropped_rejections);
            }
            for rejection in drain.updates {
                let diagnostic = self.diagnostic(rejection.diagnostic);
                eprintln!("promise-rejection: id={} handled={} {diagnostic}", rejection.id, rejection.handled);
            }
        }
        self.check_contract()?;
        let queued = self.bridge.has_queued_notifications().map_err(|overflow: QueueOverflow| overflow.to_string())?;
        if engine.is_job_pending() || queued || self.work.has_ready() || self.work.has_completions() { self.wake_owner(); }
        Ok(worked)
    }

    pub(crate) fn frame_checkpoint(&self) {
        if self.shutdown_started.get() { return; }
        if let Err(error) = self.checkpoint_inner() {
            *self.fault.borrow_mut() = Some(error);
            self.wake_owner();
        }
    }

    fn mark_reported<T>(&self, result: Result<T>) -> Result<T> {
        if matches!(result, Err(SessionError::Contract(_))) {
            self.contract_reported.set(true);
        }
        result
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

    /// Integration harness entries ('advance', 'external') run as one bridge
    /// turn against the mounted module, with the same contract and diagnostic
    /// mapping as mount.
    pub(crate) fn invoke(&self, export: &str) -> Result<()> {
        let result = self.invoke_inner(export);
        self.mark_reported(result)
    }

    fn invoke_inner(&self, export: &str) -> Result<()> {
        if self.shutdown_started.get() { return Err("cannot invoke a retired session".into()); }
        let mut engine = self.engine.borrow_mut();
        let engine = engine.as_mut().ok_or("session disposed")?;
        let result = self.bridge.run_turn(engine.turn(), |_, js| js.with_module(|_, module| {
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
            callbacks: self.bridge.callback_count(), focus_traps: self.bridge.focus_trap_count(),
            scroll_locks: self.bridge.scroll_lock_count(),
            timers, frame_callbacks, requests,
            engine_alive: self.engine.borrow().is_some(),
        }
    }

    pub(crate) fn retire(&self) -> SessionRetirement {
        if let Some(existing) = self.retired.borrow().clone() {
            return existing;
        }
        // An old Rc may outlive replacement. Only the first retirement captures
        // pre-disposal counts, runs JS cleanup, and removes owned resources.
        let before = self.counts();
        self.shutdown_started.set(true);
        self.bridge.begin_stop();
        self.work.begin_stop();
        self.fault.borrow_mut().take();
        let mut cleanup_error = None;
        let disposer = self.disposer.borrow_mut().take();
        if let Some(disposer) = disposer
            && let Some(engine) = self.engine.borrow_mut().as_mut()
        {
            // A failed structural turn has already retired native handles.
            // Still run pure JS cleanup, without reopening a disposed registry.
            let result = if self.bridge.is_disposed() {
                engine.with_cleanup_context(|ctx| disposer.restore(&ctx)?.call::<_, ()>(()))
            } else {
                self.bridge.run_turn(engine.turn(), |_, js| {
                    js.with_cleanup_context(|ctx| disposer.restore(&ctx)?.call::<_, ()>(()))
                })
            };
            if let Err(error) = result {
                let error = self.diagnostic(error);
                eprintln!("disposer error: {error}");
                cleanup_error = Some(error);
            }
            if self.policy.strict_rejections
                && let Err(error) = engine.drain_rejections(RejectionPolicy::Strict)
            {
                cleanup_error.get_or_insert_with(|| self.diagnostic(error));
            }
        }
        self.work.finish_stop();
        self.bridge.dispose();
        if self.stylesheet_installed.replace(false) { self.region.host.remove_stylesheet("application"); }
        let snapshot = self.engine.borrow().as_ref().map(Self::snapshot_telemetry);
        self.telemetry.set(RuntimeTelemetry::merge(self.telemetry.get(), snapshot));
        if self.engine.borrow_mut().take().is_some() {
            eprintln!("session-disposed: generation={} frames={} resources={:?}", self.generation, self.frames.get(), self.counts());
        }
        let counts = self.counts();
        let failure = if !self.contract_reported.replace(true)
            && let Some(contract) = self.contract_failure()
        {
            Some(RetirementFailure::Contract(contract))
        } else {
            cleanup_error.map(RetirementFailure::Cleanup)
        };
        let report = SessionRetirement {
            generation: self.generation,
            before,
            counts,
            failure,
            telemetry: self.telemetry.get(),
        };
        *self.retired.borrow_mut() = Some(report.clone());
        report
    }

    fn snapshot_telemetry(engine: &JsRuntime) -> RuntimeTelemetry {
        RuntimeTelemetry {
            max_entry_duration: engine.max_entry_duration(),
            js_heap_bytes: engine.memory_usage_bytes(),
            js_stack_limit_bytes: engine.stack_limit_bytes(),
        }
    }
}
impl Drop for Session { fn drop(&mut self) { let _ = self.retire(); } }

fn remap(module: &SessionModule, error: js_runtime::Diagnostic) -> SessionError {
    SessionError::Diagnostic(match &module.package {
        Some(package) => package.remap_diagnostic(error),
        None => error,
    })
}

#[cfg(test)]
mod tests;
