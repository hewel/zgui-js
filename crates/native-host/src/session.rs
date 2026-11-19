use std::{cell::{Cell, RefCell}, collections::BTreeMap, rc::{Rc, Weak}, sync::Arc, time::{Duration, Instant}};
use js_runtime::{JsRuntime, RejectionPolicy, RuntimeLimits};
use rquickjs::{Exception, Function, Persistent};
use ui_bridge::{BridgeGroup, BridgeStep, DispatchOutcome, HostPort, UiBridge, registry::NodeRegistry};
use zgui::{app::Fonts, platform::{SurfaceId, WakeReason, Waker}, runtime::{AppTimers, ImageBlobs}, view::DocumentId};
use crate::{assets::{AssetSource, ContractFailure}, environment::Environment, session_host::MountRegion, package::Package, shell::{Authority, ContractPolicy, Monitor, Shell}, work::{Arguments, Work}};

/// Typed session failure: engine diagnostics, sticky provider contracts, and
/// host invariants keep distinct categories through recovery classification.
#[derive(Debug)]
pub(crate) enum SessionError {
    Diagnostic(js_runtime::Diagnostic),
    Contract(ContractFailure),
    InvalidState(String),
    Style { code: &'static str, message: String, details: serde_json::Value },
}
impl std::fmt::Display for SessionError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Diagnostic(diagnostic) => diagnostic.fmt(out),
            Self::Contract(failure) => failure.fmt(out),
            Self::InvalidState(message) => out.write_str(message),
            Self::Style { code, message, details } => {
                write!(out, "{code}: {message}")?;
                if !details.is_null() { write!(out, " {details}")?; }
                Ok(())
            }
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
    pub styles: Vec<crate::package::Stylesheet>,
    pub package: Option<Rc<Package>>,
}
impl SessionModule {
    pub(crate) fn package(package: Rc<Package>) -> Self {
        Self {
            name: package.directory.join(&package.manifest.entry).to_string_lossy().into_owned(),
            source: package.source.clone(),
            styles: package.styles.clone(),
            package: Some(package),
        }
    }
}

impl SessionModule {
    /// The package declaration is the sole authority for dynamic shell surfaces.
    pub(crate) fn shell_authority(&self) -> Authority {
        if self.package.as_ref().is_some_and(|package| package.manifest.capabilities.shell_surfaces) {
            Authority::Allowed
        } else {
            Authority::Denied
        }
    }
}

type TeardownReporter = Rc<dyn Fn(&ResourceCounts)>;

/// Execution and provider policy is fixed before the application module is evaluated.
#[derive(Clone)]
pub(crate) struct SessionPolicy {
    limits: RuntimeLimits,
    deadline: Option<Rc<Cell<Instant>>>,
    environment: Option<Rc<Environment>>,
    strict_rejections: bool,
    assets: AssetSource,
    reduced_motion: bool,
    teardown_reporter: Option<TeardownReporter>,
}
impl SessionPolicy {
    /// Ordinary live execution: wall clock, ambient limits, logged rejections,
    /// package asset reads on the shared worker pool.
    pub(crate) fn live(executor: host_services::Executor) -> Self {
        Self { limits: RuntimeLimits::default(), deadline: None, environment: None, strict_rejections: false, assets: AssetSource::Live(executor), reduced_motion: false, teardown_reporter: None }
    }
    /// Deterministic preview execution: supplied limits, an enclosing deadline,
    /// the virtual environment, strict rejection handling and fixture assets.
    pub(crate) fn preview(limits: RuntimeLimits, deadline: Rc<Cell<Instant>>, environment: Rc<Environment>, fixtures: Rc<crate::assets::fixture::FixtureRuntime>) -> Self {
        Self { limits, deadline: Some(deadline), environment: Some(environment), strict_rejections: true, assets: AssetSource::Fixture(fixtures), reduced_motion: false, teardown_reporter: None }
    }
    pub(crate) fn with_reduced_motion(mut self, reduced_motion: bool) -> Self {
        self.reduced_motion = reduced_motion;
        self
    }

    /// Report residual external leases to the live application on any retirement path.
    pub(crate) fn with_teardown_reporter(mut self, reporter: TeardownReporter) -> Self {
        self.teardown_reporter = Some(reporter);
        self
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
    #[serde(skip_serializing_if = "is_zero")]
    pub image_handles: usize,
    /// Unix socket clients still open after application cleanup.
    #[serde(skip_serializing_if = "is_zero")]
    pub unix_sockets: usize,
    /// Child process groups not yet closed and reaped.
    #[serde(skip_serializing_if = "is_zero")]
    pub child_processes: usize,
    /// Inbound control handlers still registered.
    #[serde(skip_serializing_if = "is_zero")]
    pub control_listeners: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub canvas_scenes: usize,
    /// Native auxiliary shell surfaces still registered with the platform.
    pub shell_surfaces: usize,
    pub fonts: usize,
    pub engine_alive: bool,
}

fn is_zero(value: &usize) -> bool { *value == 0 }

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

/// The bridge and native root owned together, until the surface is closed.
pub(crate) struct SessionRegion {
    pub(crate) region: MountRegion,
    pub(crate) bridge: Rc<UiBridge>,
    pub(crate) surface: SurfaceId,
    pub(crate) name: String,
    disposer: RefCell<Option<Persistent<Function<'static>>>>,
    font_validation_revision: Cell<Option<(u64, u64, u64, zgui_style::ColorScheme)>>,
}

enum Startup {
    Primary { region: MountRegion, surface: SurfaceId },
    Shell { fonts: Fonts, timers: AppTimers },
}

pub(crate) struct Session {
    // All persistent JS roots precede the engine and drain before it drops.
    disposer: RefCell<Option<Persistent<Function<'static>>>>,
    primary: Option<Rc<SessionRegion>>,
    layers: RefCell<BTreeMap<u32, Rc<SessionRegion>>>,
    shell_surfaces: Cell<usize>,
    bridges: Rc<BridgeGroup>,
    pub(crate) shell: Rc<Shell>,
    work: Rc<Work>,
    engine: RefCell<Option<JsRuntime>>,
    font_leases: RefCell<Vec<zgui::app::fonts::FontLease>>,
    pub(crate) fonts: Fonts,
    telemetry: Cell<Option<RuntimeTelemetry>>,
    shutdown_started: Cell<bool>,
    mount_started: Cell<bool>,
    stylesheet_installed: Cell<bool>,
    contract_reported: Cell<bool>,
    pub(crate) generation: u64,
    pub(crate) frames: Cell<usize>,
    module: SessionModule,
    fault: RefCell<Option<SessionError>>,
    retired: RefCell<Option<SessionRetirement>>,
    retirement_reported: Cell<bool>,
    policy: SessionPolicy,
    checkpoint_cursor: Cell<usize>,
    waker: Arc<dyn Waker>,
    notifications_after: Cell<u32>,
}

impl Session {
    /// Pure module preflight: bridge mutation is disabled until mount's turn.
    pub(crate) fn prepare(region: MountRegion, module: SessionModule, generation: u64, waker: Arc<dyn Waker>, surface: SurfaceId, policy: SessionPolicy) -> Result<Rc<Self>> {
        Self::prepare_inner(Startup::Primary { region, surface }, module, generation, waker, policy)
    }

    /// Pure preflight without a document, window or mount root.
    pub(crate) fn prepare_shell(fonts: Fonts, timers: AppTimers, module: SessionModule, generation: u64, waker: Arc<dyn Waker>, policy: SessionPolicy) -> Result<Rc<Self>> {
        if module.shell_authority() != Authority::Allowed {
            return Err(ContractFailure { code: "E_CAPABILITY", message: "pure-shell startup requires capabilities.shellSurfaces".into() }.into());
        }
        Self::prepare_inner(Startup::Shell { fonts, timers }, module, generation, waker, policy)
    }

    fn prepare_inner(startup: Startup, module: SessionModule, generation: u64, waker: Arc<dyn Waker>, policy: SessionPolicy) -> Result<Rc<Self>> {
        let fonts = match &startup { Startup::Primary { region, .. } => region.fonts.clone(), Startup::Shell { fonts, .. } => fonts.clone() };
        let mut engine = JsRuntime::new(policy.limits.clone())?;
        let mut font_families = std::collections::BTreeSet::new();
        for sheet in &module.styles {
            let summary = zgui_style::preflight_stylesheet(&sheet.css)
                .map_err(|error| SessionError::Style { code: "E_STYLE_PROFILE", message: format!("stylesheet {} rejected", sheet.path),
                    details: serde_json::to_value(error).expect("native stylesheet diagnostics serialize") })?;
            font_families.extend(summary.font_families);
        }
        let mut font_leases = Vec::new();
        if let Some(package) = &module.package {
            for font in &package.manifest.fonts {
                let bytes = package.asset_data(&font.path).ok_or("verified package font is missing")?;
                let lease = fonts.register_scoped(Arc::new(bytes), &font.family)
                    .map_err(|error| SessionError::Style { code: "E_FONT", message: format!("font {} as {:?} rejected: {error:?}", font.path, font.family), details: serde_json::Value::Null })?;
                font_leases.push(lease);
            }
        }
        for family in font_families {
            if !fonts.contains_family(&family) {
                return Err(SessionError::Style { code: "E_FONT_FAMILY", message: format!("FONT_FAMILY_UNREGISTERED: {family:?} is not registered"), details: serde_json::Value::Null });
            }
        }
        if let Some(deadline) = &policy.deadline { engine.set_enclosing_deadline(deadline.clone()); }
        let session = Rc::new_cyclic(|weak: &Weak<Self>| {
            let bridges = Rc::new(BridgeGroup::new());
            let contract = if policy.strict_rejections { ContractPolicy::Sticky } else { ContractPolicy::Throw };
            let shell = Shell::new(module.shell_authority(), contract, waker.clone());
            let (primary, work) = match startup {
                Startup::Primary { region, surface } => {
                    let work = Work::new(region.host.clone(), module.package.clone(), generation, waker.clone(), surface, policy.assets.clone());
                    let region = Rc::new(Self::build_region(region, "main".into(), surface, generation, &policy, waker.clone(), weak.clone()));
                    bridges.add(&region.bridge);
                    (Some(region), work)
                }
                Startup::Shell { timers, .. } => (None, Work::new_shell(module.package.clone(), generation, waker.clone(), policy.assets.clone(), timers)),
            };
            Self {
                disposer: RefCell::new(None), primary, layers: RefCell::new(BTreeMap::new()), bridges, shell, work, engine: RefCell::new(None),
                shell_surfaces: Cell::new(0), retirement_reported: Cell::new(false),
                telemetry: Cell::new(None), shutdown_started: Cell::new(false),
                mount_started: Cell::new(false), stylesheet_installed: Cell::new(false),
                contract_reported: Cell::new(false),
                font_leases: RefCell::new(Vec::new()),
                fonts,
                generation, frames: Cell::new(0), module,
                fault: RefCell::new(None), retired: RefCell::new(None),
                policy, checkpoint_cursor: Cell::new(0), waker, notifications_after: Cell::new(0),
            }
        });
        engine.with_context(|ctx| {
            if let Some(environment) = &session.policy.environment { environment.install(&ctx)?; }
            if let Some(primary) = &session.primary {
                primary.bridge.install(&ctx)?;
                ctx.globals().set("mountRoot", primary.bridge.root(&ctx)?)?;
            }
            session.shell.install(&ctx)?;
            session.work.install(&ctx)
        })?;
        let loaded = engine.load_module(&session.module.name, &session.module.source);
        session.check_contract()?;
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
        session.check_contract()?;
        if let Some(error) = jobs.diagnostics.into_iter().next() { return Err(remap(&session.module, error)); }
        if jobs.has_pending_jobs {
            return Err(js_runtime::Diagnostic {
                kind: js_runtime::DiagnosticKind::BudgetExceeded, phase: "preflight",
                message: "preflight jobs exceeded their bounded pump".into(), stack: None, generated_stack: None,
            }.into());
        }
        let rejections = engine.drain_rejections(RejectionPolicy::Strict).map_err(|error| remap(&session.module, error));
        session.check_contract()?;
        rejections?;
        if engine.is_job_pending() {
            return Err(js_runtime::Diagnostic {
                kind: js_runtime::DiagnosticKind::BudgetExceeded, phase: "preflight",
                message: "preflight diagnostic accessors scheduled more jobs".into(), stack: None, generated_stack: None,
            }.into());
        }
        *session.engine.borrow_mut() = Some(engine);
        *session.font_leases.borrow_mut() = font_leases;
        Ok(session)
    }

    fn build_region(region: MountRegion, name: String, surface: SurfaceId, generation: u64, policy: &SessionPolicy, waker: Arc<dyn Waker>, session: Weak<Self>) -> SessionRegion {
        let port = HostPort::live(surface, waker, session, Self::dispatch_step);
        let bridge = UiBridge::new_with_options(NodeRegistry::new(region.dom.clone(), region.root, 1, generation, 100_000),
            region.host.clone(), port, ui_bridge::BridgeOptions { strict_styles: policy.strict_rejections, reduced_motion: policy.reduced_motion, defer_stylesheet_install: true });
        SessionRegion { region, bridge, surface, name, disposer: RefCell::new(None), font_validation_revision: Cell::new(None) }
    }

    /// The primary region is absent for a pure-shell session.
    pub(crate) fn primary(&self) -> Option<&SessionRegion> { self.primary.as_deref() }

    #[cfg(test)]
    pub(crate) fn primary_region(&self) -> &MountRegion { &self.primary().expect("test session has a primary region").region }

    #[cfg(test)]
    fn bridge(&self) -> &UiBridge { &self.primary().expect("test session has a primary bridge").bridge }

    /// Find the owner of a native document without changing its generation.
    pub(crate) fn mount_region(&self, document: DocumentId) -> Option<Rc<SessionRegion>> {
        self.primary.iter().chain(self.layers.borrow().values()).find(|owned| owned.region.document == document).cloned()
    }

    /// Snapshot region references for preview inspection and native scheduling.
    pub(crate) fn mount_regions(&self) -> Vec<(String, Rc<SessionRegion>)> {
        self.primary.iter().chain(self.layers.borrow().values()).map(|owned| (owned.name.clone(), owned.clone())).collect()
    }

    /// Request presentation in every owned document after shared work runs.
    pub(crate) fn request_frames(&self, mut request: impl FnMut(DocumentId)) {
        for owned in self.primary.iter().chain(self.layers.borrow().values()) { request(owned.region.document); }
    }

    /// Deliver monitor changes in one structural turn, even with no current document.
    pub(crate) fn update_monitors(&self, monitors: &[Monitor]) -> Result<bool> {
        if !self.shell.set_monitors(monitors) || self.shutdown_started.get() { return Ok(false); }
        let mut engine = self.engine.borrow_mut();
        let engine = engine.as_mut().ok_or("session disposed")?;
        let result = self.bridges.run(engine.turn(), |js| js.with_context(|ctx| self.shell.dispatch_monitors(&ctx)));
        self.mark_reported(self.check_contract())?;
        result.map(|()| true).map_err(|error| self.diagnostic(error).into())
    }

    /// Build and mount an auxiliary document in the already-running JS engine.
    pub(crate) fn attach_layer(self: &Rc<Self>, id: u32, name: String, region: MountRegion, surface: SurfaceId) -> Result<()> {
        let attach = self.shell.attach(id).ok_or("shell surface was closed before attachment")?;
        let owned = Rc::new(Self::build_region(region, name, surface, self.generation, &self.policy, self.waker.clone(), Rc::downgrade(self)));
        self.bridges.add(&owned.bridge);
        self.layers.borrow_mut().insert(id, owned.clone());
        self.refresh_frame_host();
        self.install_sheets(&owned);
        let mut engine = self.engine.borrow_mut();
        let engine = engine.as_mut().ok_or("session disposed")?;
        let result = self.bridges.run(engine.turn(), |js| js.with_context(|ctx| {
            let callback = attach.restore(&ctx)?;
            let disposer: Function = callback.call((owned.bridge.root(&ctx)?, owned.bridge.api(&ctx)?))?;
            Ok(Persistent::save(&ctx, disposer))
        }));
        match result {
            Ok(disposer) => *owned.disposer.borrow_mut() = Some(disposer),
            Err(error) => {
                self.check_contract()?;
                return Err(self.diagnostic(error).into());
            }
        }
        self.check_contract()?;
        self.preflight_stylesheets(&owned.bridge)?;
        owned.bridge.arm_stylesheets();
        Ok(())
    }

    pub(crate) fn shell_surface_opened(&self) { self.shell_surfaces.set(self.shell_surfaces.get() + 1); }

    pub(crate) fn shell_surface_closed(&self) { self.shell_surfaces.set(self.shell_surfaces.get() - 1); }

    /// Dispose one auxiliary document while the native surface still exists.
    pub(crate) fn detach_layer(&self, id: u32) -> Result<()> {
        let Some(owned) = self.layers.borrow_mut().remove(&id) else { return Ok(()); };
        let disposer = owned.disposer.borrow_mut().take();
        let cleanup = self.cleanup(disposer);
        self.bridges.remove(&owned.bridge);
        let scenes = owned.bridge.canvas_scene_count();
        owned.bridge.dispose();
        self.remove_sheets(&owned);
        self.refresh_frame_host();
        if scenes != 0 { return Err(format!("E_TEARDOWN: shell surface {:?} retained {scenes} canvas scenes", owned.name).into()); }
        self.check_contract()?;
        cleanup.map_err(SessionError::from)
    }

    fn refresh_frame_host(&self) {
        if self.primary.is_none() {
            self.work.set_frame_host(self.layers.borrow().first_key_value().map(|(_, owned)| owned.region.host.clone()));
        }
    }

    pub(crate) fn mount(&self) -> Result<()> {
        let result = self.mount_inner();
        self.mark_reported(result)
    }

    fn mount_inner(&self) -> Result<()> {
        if self.shutdown_started.get() { return Err("cannot mount a retired session".into()); }
        if self.mount_started.replace(true) { return Err("session mount was already attempted".into()); }
        self.work.mounting();
        self.shell.activate();
        if let Some(primary) = self.primary() {
            self.install_sheets(primary);
            self.stylesheet_installed.set(true);
        }
        let mut engine = self.engine.borrow_mut();
        let engine = engine.as_mut().ok_or("session disposed")?;
        let mounted = self.bridges.run(engine.turn(), |js| js.with_module(|ctx, module| {
            let mount: Function = module.get("mount")?;
            let disposer: Function = if let Some(primary) = self.primary() {
                mount.call((primary.bridge.root(&ctx)?,))?
            } else {
                mount.call(())?
            };
            Ok(Persistent::save(&ctx, disposer))
        }));
        match mounted {
            Ok(disposer) => *self.disposer.borrow_mut() = Some(disposer),
            Err(error) => {
                self.check_contract()?;
                return Err(self.diagnostic(error).into());
            }
        }
        if let Some(primary) = self.primary() {
            self.preflight_stylesheets(&primary.bridge)?;
            primary.bridge.arm_stylesheets();
        }
        self.check_contract()?;
        if let Some(package) = &self.module.package {
            let nodes = self.primary().map_or(0, |primary| primary.bridge.live_count());
            eprintln!("mounted: package={} build={} nodes={nodes}", package.manifest.id, package.manifest.build_id);
        }
        self.snapshot();
        self.work.mounted();
        self.check_contract()?;
        Ok(())
    }

    fn preflight_stylesheets(&self, bridge: &UiBridge) -> Result<()> {
        for (name, css) in bridge.stylesheet_candidates() {
            zgui_style::preflight_stylesheet(&css).map_err(|error| SessionError::Style {
                code: "E_STYLE_PROFILE",
                message: format!("stylesheet {name} rejected"),
                details: serde_json::to_value(error).expect("native stylesheet diagnostics serialize"),
            })?;
        }
        Ok(())
    }

    fn wake_owner(&self) {
        let reason = self.primary().map_or(WakeReason::AppWork, |primary|
            WakeReason::ReactiveWork { surfaces: vec![primary.surface].into_boxed_slice() });
        self.waker.wake(reason);
    }

    fn dispatch_step(&self, step: BridgeStep<'_>) -> DispatchOutcome {
        if self.shutdown_started.get() { return DispatchOutcome::Retired; }
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

    /// Only a mounted generation with a registered live handler accepts CLI requests.
    pub(crate) fn has_control_handler(&self) -> bool { self.work.has_control_handler() }

    /// The control listener namespace authorized by this active live package.
    pub(crate) fn control_app_id(&self) -> Option<&str> {
        if self.shutdown_started.get() { return None; }
        self.module.package.as_ref().filter(|package| package.manifest.capabilities.control_channel)
            .map(|package| package.manifest.id.as_str())
    }

    /// Admit a validated live CLI request without running application code inline.
    pub(crate) fn offer_control(&self, inbound: host_services::control::Inbound) {
        self.work.offer_control(inbound);
    }

    fn checkpoint_work(&self) -> Result<bool> {
        if let Some(error) = self.fault.borrow_mut().take() { return Err(error); }
        self.check_contract()?;
        self.has_queued_notifications()?;
        if self.shutdown_started.get() { return Ok(false); }
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
        let mut remaining = [32, 32, 32, max_jobs, 32];
        // Interleave work classes and preserve the next class across deadlines.
        // Ready native work must not starve promise jobs that can cancel it.
        while Instant::now() < end && remaining.iter().any(|&count| count != 0) {
            let phase = self.checkpoint_cursor.get();
            self.checkpoint_cursor.set((phase + 1) % remaining.len());
            if remaining[phase] == 0 { continue; }
            remaining[phase] -= 1;
            let attempted = match phase {
                0 => if let Some(completion) = self.work.next_completion() {
                    self.bridges.run(engine.turn(), |js| js.with_context(|ctx| self.work.settle(&ctx, completion)))
                        .map_err(|error| self.diagnostic(error))?;
                    true
                } else { false },
                1 => if let Some(call) = self.work.next_call() {
                    self.bridges.run(engine.turn(), |js| js.with_context(|ctx| {
                        let callback = call.callback.restore(&ctx)?;
                        match call.arguments { Arguments::Timer => callback.call::<_, ()>(()), Arguments::Frame(timestamp) => callback.call::<_, ()>((timestamp,)) }
                    })).map_err(|error| self.diagnostic(error))?;
                    true
                } else { false },
                2 => if let Some((id, owned)) = self.next_notification_region()? {
                    self.notifications_after.set(id);
                    owned.bridge.flush_notification(engine.turn(), |js, cb| js.with_context(|ctx| cb.call(&ctx)))
                        .map_err(|error| self.diagnostic(error))?
                } else { false },
                4 => if let Some(event) = self.work.next_external() {
                    self.bridges.run(engine.turn(), |js| js.with_context(|ctx| self.work.settle_external(&ctx, event)))
                        .map_err(|error| self.diagnostic(error))?;
                    true
                } else { false },
                _ => self.bridges.run(engine.turn(), |js| js.execute_job(end.saturating_duration_since(Instant::now())))
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
        let queued = self.has_queued_notifications()?;
        if engine.is_job_pending() || queued || self.work.has_ready() || self.work.has_completions() || self.work.has_external() { self.wake_owner(); }
        Ok(worked)
    }

    fn has_queued_notifications(&self) -> Result<bool> {
        let mut queued = false;
        for owned in self.primary.iter().chain(self.layers.borrow().values()) {
            queued |= owned.bridge.has_queued_notifications().map_err(|error| error.to_string())?;
        }
        Ok(queued)
    }

    fn next_notification_region(&self) -> Result<Option<(u32, Rc<SessionRegion>)>> {
        let previous = self.notifications_after.get();
        let layers = self.layers.borrow();
        use std::ops::Bound::{Excluded, Unbounded};
        let primary = self.primary.iter().map(|owned| (0, owned));
        for (id, owned) in layers.range((Excluded(previous), Unbounded)).map(|(&id, owned)| (id, owned))
            .chain(primary).chain(layers.range(..=previous).map(|(&id, owned)| (id, owned)))
        {
            if owned.bridge.has_queued_notifications().map_err(|error| error.to_string())? {
                return Ok(Some((id, owned.clone())));
            }
        }
        Ok(None)
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
    pub(crate) fn contract_failure(&self) -> Option<ContractFailure> { self.shell.failure().or_else(|| self.work.contract_failure()) }

    fn check_contract(&self) -> Result<()> {
        if let Some(failure) = self.contract_failure() { return Err(failure.into()); }
        let mut failure = None;
        for diagnostic in self.primary.iter().chain(self.layers.borrow().values()).flat_map(|owned| owned.bridge.take_style_diagnostics()) {
            eprintln!("style-diagnostic: {}: {}", diagnostic.code, diagnostic.message);
            if self.policy.strict_rejections && failure.is_none() {
                failure = Some(SessionError::Style { code: "E_STYLE", message: diagnostic.message,
                    details: serde_json::json!({"code":diagnostic.code}) });
            }
        }
        match failure { Some(failure) => Err(failure), None => Ok(()) }
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
        let result = self.bridges.run(engine.turn(), |js| js.with_module(|_, module| {
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

    pub(crate) fn stylesheet_rejected(&self, name: &str, diagnostics: zgui_style::CssDiagnostics) {
        let message = diagnostics.iter().map(|diagnostic| format!("{}:{}: {}", diagnostic.location.line, diagnostic.location.column, diagnostic.message)).collect::<Vec<_>>().join("; ");
        if self.policy.strict_rejections {
            self.record_fault(SessionError::Style {
                code: "E_STYLE",
                message,
                details: serde_json::json!({"code":"STYLESHEET_PARSE_INVALID","sheet":name}),
            });
        } else {
            eprintln!("style-diagnostic: STYLESHEET_PARSE_INVALID: {name}: {message}");
        }
    }

    pub(crate) fn package(&self) -> Option<&Rc<Package>> { self.module.package.as_ref() }
    pub(crate) fn image_blobs(&self) -> &ImageBlobs { self.work.image_blobs() }
    pub(crate) fn fixture_image_files(&self) -> impl Iterator<Item = (String, Arc<[u8]>)> + '_ {
        self.policy.assets.fixture_image_files(self.module.package.as_deref())
    }
    pub(crate) fn relinquish_stylesheets(&self) { self.stylesheet_installed.set(false); }

    pub(crate) fn restore_stylesheets(&self) {
        if let Some(primary) = self.primary() { self.install_sheets(primary); }
    }

    fn install_sheets(&self, owned: &SessionRegion) {
        for (index, sheet) in self.module.styles.iter().enumerate() {
            owned.region.host.install_stylesheet(&format!("application:{index}"), &sheet.css);
        }
    }

    fn remove_sheets(&self, owned: &SessionRegion) {
        for index in 0..self.module.styles.len() {
            owned.region.host.remove_stylesheet(&format!("application:{index}"));
        }
    }

    pub(crate) fn remove_surplus_stylesheets(&self, successor: &Self) {
        if let Some(primary) = self.primary() {
            for index in successor.module.styles.len()..self.module.styles.len() {
                primary.region.host.remove_stylesheet(&format!("application:{index}"));
            }
        }
    }

    #[cfg(test)]
    fn surface(&self) -> SurfaceId { self.primary().expect("test session has a primary surface").surface }

    pub(crate) fn validate_font_families(&self, window: &zgui::runtime::Window) -> Result<()> {
        let Some(owned) = self.mount_region(window.document_id()) else { return Ok(()); };
        let revision = (window.dom().revision(), window.style_revision(), window.surface_configures(), window.color_scheme());
        if owned.font_validation_revision.get() == Some(revision) { return Ok(()); }
        let layout = window.layout().borrow();
        let mut families = std::collections::BTreeSet::new();
        for node in owned.bridge.snapshot(100_000).nodes {
            let Some(key) = zgui_view_dom::id::to_document(node.native) else { continue; };
            for box_id in layout.boxes_of(key) {
                let Some(node) = layout.get(*box_id) else { continue; };
                for family in zgui_text_style::lower::font::family(node.style.get_font()).entries() {
                    if let zgui_text_style::FamilyName::Named(name) = family { families.insert(name.as_str().to_owned()); }
                }
            }
        }
        for family in families {
            if !owned.region.fonts.contains_family(&family) {
                let message = format!("FONT_FAMILY_UNREGISTERED: {family:?} is not registered");
                if self.policy.strict_rejections { return Err(SessionError::Style { code: "E_FONT_FAMILY", message, details: serde_json::Value::Null }); }
                eprintln!("style-diagnostic: {message}");
            }
        }
        owned.font_validation_revision.set(Some(revision));
        Ok(())
    }

    fn snapshot(&self) {
        if let Some(primary) = self.primary() {
            let region = &primary.region;
            eprintln!("native-text: {}", region.dom.text_content(region.root));
        }
    }

    pub(crate) fn counts(&self) -> ResourceCounts {
        let (timers, frame_callbacks, requests) = self.work.counts();
        let external = self.work.external_counts();
        let mut counts = ResourceCounts {
            callbacks: self.shell.callback_count(), timers, frame_callbacks, requests,
            image_handles: self.image_blobs().len(),
            unix_sockets: external.sockets,
            child_processes: external.processes,
            control_listeners: external.listeners,
            shell_surfaces: self.shell_surfaces.get(),
            fonts: self.font_leases.borrow().len(),
            engine_alive: self.engine.borrow().is_some(),
            ..ResourceCounts::default()
        };
        for owned in self.primary.iter().chain(self.layers.borrow().values()) {
            let bridge = &owned.bridge;
            counts.nodes += bridge.live_count();
            counts.wrappers += bridge.wrapper_count();
            counts.callbacks += bridge.callback_count();
            counts.focus_traps += bridge.focus_trap_count();
            counts.scroll_locks += bridge.scroll_lock_count();
            counts.canvas_scenes += bridge.canvas_scene_count();
        }
        counts
    }

    fn cleanup(&self, disposer: Option<Persistent<Function<'static>>>) -> std::result::Result<(), js_runtime::Diagnostic> {
        let Some(disposer) = disposer else { return Ok(()); };
        let mut engine = self.engine.borrow_mut();
        let Some(engine) = engine.as_mut() else { return Ok(()); };
        let call = |ctx: rquickjs::Ctx<'_>| disposer.restore(&ctx)?.call::<_, ()>(());
        // Cleanup may still release process/image leases after the native bridges faulted.
        if self.primary.iter().chain(self.layers.borrow().values()).any(|owned| owned.bridge.is_disposed()) {
            engine.with_cleanup_context(call)
        } else {
            self.bridges.run(engine.turn(), |js| js.with_cleanup_context(call))
        }.map_err(|error| self.diagnostic(error))
    }

    /// Dispose JavaScript while the native mount regions are still available for cleanup.
    pub(crate) fn dispose(&self) {
        if self.retired.borrow().is_some() { return; }
        let before = self.counts();
        self.shutdown_started.set(true);
        self.bridges.begin_stop();
        self.shell.begin_stop();
        self.work.begin_stop();
        self.fault.borrow_mut().take();
        let mut cleanup_error = self.cleanup(self.disposer.borrow_mut().take()).err();
        for owned in self.layers.borrow().values() {
            if let Err(error) = self.cleanup(owned.disposer.borrow_mut().take()) {
                cleanup_error.get_or_insert(error);
            }
        }
        if self.policy.strict_rejections && let Some(engine) = self.engine.borrow_mut().as_mut()
            && let Err(error) = engine.drain_rejections(RejectionPolicy::Strict)
        {
            cleanup_error.get_or_insert_with(|| self.diagnostic(error));
        }
        if let Some(error) = &cleanup_error { eprintln!("disposer error: {error}"); }
        let external = self.work.external_counts();
        self.work.finish_stop();
        // Report remaining leases before forced disposal releases the native scenes.
        let canvas_scenes = self.counts().canvas_scenes;
        self.bridges.dispose();
        self.shell.clear();
        for owned in std::mem::take(&mut *self.layers.borrow_mut()).into_values() {
            self.bridges.remove(&owned.bridge);
            self.remove_sheets(&owned);
        }
        self.font_leases.borrow_mut().clear();
        if self.stylesheet_installed.replace(false) && let Some(primary) = self.primary() {
            self.remove_sheets(primary);
        }
        let snapshot = self.engine.borrow().as_ref().map(Self::snapshot_telemetry);
        self.telemetry.set(RuntimeTelemetry::merge(self.telemetry.get(), snapshot));
        drop(self.engine.borrow_mut().take());
        let counts = ResourceCounts {
            canvas_scenes,
            unix_sockets: external.sockets,
            child_processes: external.processes,
            control_listeners: external.listeners,
            ..self.counts()
        };
        self.image_blobs().clear();
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
        *self.retired.borrow_mut() = Some(report);
    }

    /// Report residual resources after the host has attempted native surface closure.
    pub(crate) fn retire(&self) -> SessionRetirement {
        self.dispose();
        let mut report = self.retired.borrow().as_ref().expect("disposal records retirement").clone();
        report.counts.shell_surfaces = self.shell_surfaces.get();
        if !self.retirement_reported.replace(true) {
            if report.before.engine_alive {
                eprintln!("session-disposed: generation={} frames={} resources={:?}", self.generation, self.frames.get(), report.counts);
            }
            if report.counts != ResourceCounts::default()
                && let Some(reporter) = &self.policy.teardown_reporter
            {
                reporter(&report.counts);
            }
        }
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
