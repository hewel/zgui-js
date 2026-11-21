pub(crate) mod window_control;

use std::{cell::{Cell, RefCell}, collections::BTreeMap, rc::{Rc, Weak}, sync::Arc, time::{Duration, Instant}};
use js_runtime::{JsRuntime, RejectionPolicy, RuntimeLimits};
use rquickjs::{Exception, Function, Persistent};
use ui_bridge::{BridgeGroup, BridgeStep, DispatchOutcome, HostPort, UiBridge, registry::NodeRegistry};
use zgui::{app::Fonts, platform::{SessionLockState, Surface, SurfaceId, WakeReason, Waker}, runtime::{AppTimers, ImageBlobs}, view::DocumentId};
use crate::{assets::{AssetSource, ContractFailure, MAX_SAFE_ID}, dev_state::{DevStateScope, DevStateSnapshot, DevStateStore}, environment::Environment, session_host::MountRegion, package::Package, shell::{Authority, ContractPolicy, Monitor, Shell, ShellConfig}, work::{Arguments, Work, WorkSession}};
use crate::runtime_capabilities::ResolvedCapabilities;
use window_control::{WindowCommand, WindowControls, WindowState};

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
/// Packaged modules share a single `Rc<Package>` with zero source/style clones.
#[derive(Clone)]
pub(crate) enum SessionModule {
    Package(Rc<Package>),
    Inline {
        name: Arc<str>,
        source: Arc<str>,
        styles: Arc<[crate::package::Stylesheet]>,
    },
}

impl SessionModule {
    pub(crate) fn package(package: Rc<Package>) -> Self {
        Self::Package(package)
    }

    pub(crate) fn inline(
        name: impl Into<Arc<str>>,
        source: impl Into<Arc<str>>,
        styles: Vec<crate::package::Stylesheet>,
    ) -> Self {
        Self::Inline {
            name: name.into(),
            source: source.into(),
            styles: styles.into(),
        }
    }

    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Package(package) => package.generated_entry(),
            Self::Inline { name, .. } => name,
        }
    }

    pub(crate) fn styles(&self) -> &[crate::package::Stylesheet] {
        match self {
            Self::Package(package) => &package.styles,
            Self::Inline { styles, .. } => styles,
        }
    }

    pub(crate) fn as_package(&self) -> Option<&Rc<Package>> {
        match self {
            Self::Package(package) => Some(package),
            Self::Inline { .. } => None,
        }
    }

    fn take_or_load_source(&self) -> Result<Arc<str>> {
        match self {
            Self::Package(package) => package
                .take_or_load_source()
                .map_err(|error| SessionError::InvalidState(error.to_string())),
            Self::Inline { source, .. } => Ok(Arc::clone(source)),
        }
    }

    /// The package declaration is the sole authority for dynamic shell surfaces.
    pub(crate) fn shell_authority(&self) -> Authority {
        if self.as_package().is_some_and(|package| package.manifest.capabilities.shell_surfaces) {
            Authority::Allowed
        } else {
            Authority::Denied
        }
    }

    fn declares_session_lock(&self) -> bool {
        self.as_package().is_some_and(|package| package.manifest.capabilities.session_lock)
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
    socket_bindings: Option<Rc<BTreeMap<String, String>>>,
    reduced_motion: Option<bool>,
    teardown_reporter: Option<TeardownReporter>,
    session_lock: Option<SessionLockState>,
    dev_state: bool,
    hydrated_dev_state: Option<DevStateSnapshot>,
}
impl SessionPolicy {
    /// Ordinary live execution: wall clock, ambient limits, and package assets
    /// on the shared worker pool. Unhandled rejections fault under a retained lock.
    pub(crate) fn live(executor: host_services::Executor) -> Self {
        Self { limits: RuntimeLimits::default(), deadline: None, environment: None, strict_rejections: false, assets: AssetSource::Live(executor), socket_bindings: None, reduced_motion: None, teardown_reporter: None, session_lock: None, dev_state: false, hydrated_dev_state: None }
    }
    /// Deterministic preview execution: supplied limits, an enclosing deadline,
    /// the virtual environment, strict rejection handling and fixture assets.
    pub(crate) fn preview(limits: RuntimeLimits, deadline: Rc<Cell<Instant>>, environment: Rc<Environment>, fixtures: Rc<crate::assets::fixture::FixtureRuntime>) -> Self {
        Self { limits, deadline: Some(deadline), environment: Some(environment), strict_rejections: true, assets: AssetSource::Fixture(fixtures), socket_bindings: None, reduced_motion: Some(false), teardown_reporter: None, session_lock: None, dev_state: false, hydrated_dev_state: None }
    }
    /// Install the explicit launch bindings shared by all live candidates.
    #[must_use]
    pub(crate) fn with_socket_bindings(mut self, bindings: BTreeMap<String, String>) -> Self {
        self.socket_bindings = (!bindings.is_empty()).then(|| Rc::new(bindings));
        self
    }

    pub(crate) fn with_reduced_motion(mut self, reduced_motion: bool) -> Self {
        self.reduced_motion = Some(reduced_motion);
        self
    }

    pub(crate) fn with_stack_limit_bytes(mut self, stack_limit_bytes: usize) -> Self {
        self.limits.stack_limit_bytes = stack_limit_bytes;
        self
    }

    pub(crate) fn with_session_lock(mut self, state: Option<SessionLockState>) -> Self {
        self.session_lock = state;
        self
    }

    /// Enable the dev-only bounded plain-data reload snapshot channel.
    pub(crate) fn with_dev_state(mut self, enabled: bool) -> Self {
        self.dev_state = enabled;
        self
    }

    /// Attach a compatible plain-data snapshot captured from the settled predecessor.
    pub(crate) fn with_hydrated_dev_state(mut self, snapshot: Option<DevStateSnapshot>) -> Self {
        self.hydrated_dev_state = snapshot;
        self
    }

    /// Whether the dev-only reload snapshot channel is active.
    pub(crate) fn dev_state_enabled(&self) -> bool {
        self.dev_state
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
    /// Open SQLite database handles still retained after cleanup.
    #[serde(skip_serializing_if = "is_zero")]
    pub sqlite_databases: usize,
    /// Active SQLite transactions still open after cleanup.
    #[serde(skip_serializing_if = "is_zero")]
    pub sqlite_transactions: usize,
    /// Open local file or directory capability handles still retained after cleanup.
    #[serde(skip_serializing_if = "is_zero")]
    pub file_handles: usize,
    /// Active directory watchers still running after cleanup.
    #[serde(skip_serializing_if = "is_zero")]
    pub fs_watchers: usize,
    /// In-flight HTTP requests or open HTTP response body streams still active after cleanup.
    #[serde(skip_serializing_if = "is_zero")]
    pub http_requests: usize,
    /// Open Server-Sent Events streams still active after cleanup.
    #[serde(skip_serializing_if = "is_zero")]
    pub event_streams: usize,
    /// Open WebSocket connections still active after cleanup.
    #[serde(skip_serializing_if = "is_zero")]
    pub web_sockets: usize,
    /// Active internal drag sessions still retained after cleanup.
    #[serde(skip_serializing_if = "is_zero")]
    pub active_drags: usize,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProcessMemory {
    pub rss_bytes: u64,
    pub pss_bytes: u64,
    pub pss_anon_bytes: u64,
    pub pss_file_bytes: u64,
    pub private_dirty_bytes: u64,
}

impl ProcessMemory {
    fn merge(current: Option<Self>, next: Option<Self>) -> Option<Self> {
        match (current, next) {
            (None, other) | (other, None) => other,
            (Some(a), Some(b)) => Some(Self {
                rss_bytes: a.rss_bytes.max(b.rss_bytes),
                pss_bytes: a.pss_bytes.max(b.pss_bytes),
                pss_anon_bytes: a.pss_anon_bytes.max(b.pss_anon_bytes),
                pss_file_bytes: a.pss_file_bytes.max(b.pss_file_bytes),
                private_dirty_bytes: a.private_dirty_bytes.max(b.private_dirty_bytes),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RuntimeTelemetry {
    pub max_entry_duration: Duration,
    /// Peak QuickJS heap observed while the session was live (mount / pre-disposal).
    pub js_heap_bytes: u64,
    /// Live QuickJS heap sampled immediately before teardown disposers ran.
    pub js_heap_live_bytes: u64,
    /// Residual QuickJS heap sampled after teardown disposers and bridge cleanup.
    pub js_heap_post_dispose_bytes: u64,
    pub js_stack_limit_bytes: usize,
    /// Linux `/proc/self/smaps_rollup` process memory snapshot sampled before disposal.
    pub process_memory: Option<ProcessMemory>,
}

impl RuntimeTelemetry {
    pub(crate) fn merge(current: Option<Self>, next: Option<Self>) -> Option<Self> {
        match (current, next) {
            (None, other) | (other, None) => other,
            (Some(a), Some(b)) => Some(Self {
                max_entry_duration: a.max_entry_duration.max(b.max_entry_duration),
                js_heap_bytes: a.js_heap_bytes.max(b.js_heap_bytes),
                js_heap_live_bytes: a.js_heap_live_bytes.max(b.js_heap_live_bytes),
                js_heap_post_dispose_bytes: a.js_heap_post_dispose_bytes.max(b.js_heap_post_dispose_bytes),
                js_stack_limit_bytes: b.js_stack_limit_bytes.max(a.js_stack_limit_bytes),
                process_memory: ProcessMemory::merge(a.process_memory, b.process_memory),
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
    lock_claim: Option<std::sync::Weak<dyn Surface>>,
    font_validation_revision: Cell<Option<(u64, u64, u64, zgui_style::ColorScheme)>>,
}

impl SessionRegion {
    pub(crate) fn is_lock_surface(&self) -> bool {
        self.lock_claim.is_some()
    }
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
    shell_stylesheets: RefCell<Vec<(String, String)>>,
    shell_surfaces: Cell<usize>,
    pub(crate) bridges: Rc<BridgeGroup>,
    pub(crate) shell: Rc<Shell>,
    window_controls: Rc<WindowControls>,
    pub(crate) work: Rc<Work>,
    pub(crate) dnd: crate::dnd::DndState,
    dev_state: Option<Rc<RefCell<DevStateStore>>>,
    platform_surfaces: RefCell<std::collections::HashMap<SurfaceId, std::sync::Weak<dyn Surface>>>,
    pub(crate) engine: RefCell<Option<JsRuntime>>,
    font_leases: RefCell<Vec<zgui::app::fonts::FontLease>>,
    reported_unregistered_fonts: RefCell<std::collections::BTreeSet<String>>,
    pub(crate) fonts: Fonts,
    telemetry: Cell<Option<RuntimeTelemetry>>,
    pub(crate) shutdown_started: Cell<bool>,
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
    reduced_motion: Cell<bool>,
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
        if module.shell_authority() != Authority::Allowed && !module.declares_session_lock() {
            return Err(ContractFailure { code: "E_CAPABILITY", message: "pure-shell startup requires capabilities.shellSurfaces or capabilities.sessionLock".into() }.into());
        }
        Self::prepare_inner(Startup::Shell { fonts, timers }, module, generation, waker, policy)
    }

    fn prepare_inner(startup: Startup, module: SessionModule, generation: u64, waker: Arc<dyn Waker>, policy: SessionPolicy) -> Result<Rc<Self>> {
        if generation > MAX_SAFE_ID { return Err("session generation exhausted".into()); }
        let resolved = if let Some(package) = module.as_package() {
            let declarations = &package.manifest.capabilities;
            match &policy.assets {
                AssetSource::Live(_) => ResolvedCapabilities::live(declarations, policy.socket_bindings.as_deref().unwrap_or(&BTreeMap::new()), |key| std::env::var_os(key)),
                AssetSource::Fixture(fixture) => fixture.resolve_capabilities(declarations),
            }?
        } else {
            ResolvedCapabilities::default()
        };
        let fonts = match &startup { Startup::Primary { region, .. } => region.fonts.clone(), Startup::Shell { fonts, .. } => fonts.clone() };
        let mut engine = JsRuntime::new(policy.limits.clone())?;
        let mut font_families = std::collections::BTreeSet::new();
        for sheet in module.styles() {
            let summary = zgui_style::preflight_stylesheet(&sheet.css)
                .map_err(|error| SessionError::Style { code: "E_STYLE_PROFILE", message: format!("stylesheet {} rejected", sheet.path),
                    details: serde_json::to_value(error).expect("native stylesheet diagnostics serialize") })?;
            font_families.extend(summary.font_families);
        }
        let mut font_leases = Vec::new();
        if let Some(package) = module.as_package() {
            for font in &package.manifest.fonts {
                let bytes = package.asset_data(&font.path).ok_or("verified package font is missing")?;
                let lease = fonts.register_scoped(Arc::new(bytes), &font.family)
                    .map_err(|error| SessionError::Style { code: "E_FONT", message: format!("font {} as {:?} rejected: {error:?}", font.path, font.family), details: serde_json::Value::Null })?;
                font_leases.push(lease);
            }
        }
        for family in font_families {
            if !is_css_generic_font_family(&family) && !fonts.contains_family(&family) {
                return Err(SessionError::Style { code: "E_FONT_FAMILY", message: format!("FONT_FAMILY_UNREGISTERED: {family:?} is not registered"), details: serde_json::Value::Null });
            }
        }
        if let Some(deadline) = &policy.deadline { engine.set_enclosing_deadline(deadline.clone()); }
        let session = Rc::new_cyclic(|weak: &Weak<Self>| {
            let bridges = Rc::new(BridgeGroup::new());
            let contract = if policy.strict_rejections { ContractPolicy::Sticky } else { ContractPolicy::Throw };
            let lock_authority = if policy.session_lock.is_some() && module.declares_session_lock() {
                Authority::Allowed
            } else {
                Authority::Denied
            };
            let window_authority = if matches!(startup, Startup::Primary { .. }) || module.shell_authority() == Authority::Allowed {
                Authority::Allowed
            } else {
                Authority::Denied
            };
            let shell = Shell::new(ShellConfig {
                generation,
                authority: module.shell_authority(),
                window_authority,
                lock_authority,
                lock_state: policy.session_lock.unwrap_or_default(),
                contract,
            }, waker.clone());
            let window_controls = WindowControls::new(waker.clone());
            let work_session = WorkSession { package: module.as_package().cloned(), generation, assets: policy.assets.clone(), resolved };
            let initial_reduced_motion = policy.reduced_motion.unwrap_or(false);
            let (primary, work) = match startup {
                Startup::Primary { region, surface } => {
                    let initial_title = module
                        .as_package()
                        .and_then(|pkg| pkg.manifest.window.as_ref())
                        .and_then(|win| win.title.clone())
                        .unwrap_or_else(|| "zgui-js".into());
                    window_controls.seed_surface(surface, WindowState {
                        title: initial_title,
                        ..WindowState::default()
                    });
                    let work = Work::new(work_session, region.host.clone(), surface, waker.clone());
                    let options = ui_bridge::BridgeOptions { strict_styles: policy.strict_rejections, reduced_motion: initial_reduced_motion, defer_stylesheet_install: true };
                    let region = Rc::new(Self::build_region(region, "main".into(), surface, generation, options, waker.clone(), weak.clone()));
                    bridges.add(&region.bridge);
                    (Some(region), work)
                }
                Startup::Shell { timers, .. } => (None, Work::new_shell(work_session, timers, waker.clone())),
            };
            work.guard_cleanup_with(shell.clone());
            work.set_gesture_source(bridges.clone());
            let dev_state = if policy.dev_state {
                let scope = DevStateScope::for_module(&module);
                Some(Rc::new(RefCell::new(DevStateStore::new(policy.hydrated_dev_state.clone(), &scope))))
            } else {
                None
            };
            Self {
                disposer: RefCell::new(None), primary, layers: RefCell::new(BTreeMap::new()), bridges, shell, window_controls, work, dnd: crate::dnd::DndState::new(), dev_state, platform_surfaces: RefCell::new(std::collections::HashMap::new()), engine: RefCell::new(None),
                shell_surfaces: Cell::new(0), retirement_reported: Cell::new(false),
                shell_stylesheets: RefCell::new(Vec::new()),
                telemetry: Cell::new(None), shutdown_started: Cell::new(false),
                mount_started: Cell::new(false), stylesheet_installed: Cell::new(false),
                contract_reported: Cell::new(false),
                font_leases: RefCell::new(Vec::new()),
                reported_unregistered_fonts: RefCell::new(std::collections::BTreeSet::new()),
                fonts,
                generation, frames: Cell::new(0), module,
                fault: RefCell::new(None), retired: RefCell::new(None),
                policy, reduced_motion: Cell::new(initial_reduced_motion),
                checkpoint_cursor: Cell::new(0), waker, notifications_after: Cell::new(0),
            }
        });
        engine.with_context(|ctx| {
            if let Some(environment) = &session.policy.environment { environment.install(&ctx)?; }
            if let Some(primary) = &session.primary {
                let api = primary.bridge.api(&ctx)?;
                session.work.install_region_frame_api(&api, primary.region.host.clone())?;
                session.window_controls.install_region_window_api(&api, primary.surface, true)?;
                ctx.globals().set("__zgui", api)?;
                ctx.globals().set("mountRoot", primary.bridge.root(&ctx)?)?;
            }
            let shell_api = session.shell.install(&ctx)?;
            session.shell.install_stylesheet_api(&shell_api, &session)?;
            session.install_dnd(&ctx)?;
            if let Some(dev_state) = &session.dev_state {
                DevStateStore::install(dev_state, &ctx)?;
            }
            session.work.install(&ctx)
        })?;
        let source = session.module.take_or_load_source()?;
        let loaded = engine.load_module(session.module.name(), &source);
        drop(source);
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

    fn build_region(region: MountRegion, name: String, surface: SurfaceId, generation: u64, options: ui_bridge::BridgeOptions, waker: Arc<dyn Waker>, session: Weak<Self>) -> SessionRegion {
        let port = HostPort::live(surface, waker, session, Self::dispatch_step);
        let bridge = UiBridge::new_with_options(NodeRegistry::new(region.dom.clone(), region.root, 1, generation, 100_000),
            region.host.clone(), port, options);
        SessionRegion { region, bridge, surface, name, disposer: RefCell::new(None), lock_claim: None, font_validation_revision: Cell::new(None) }
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

    /// Publish compositor state to this generation in a bounded bridge turn.
    pub(crate) fn update_lock_state(&self, state: SessionLockState) -> Result<bool> {
        if self.shutdown_started.get() || !self.shell.update_lock_state(state) { return Ok(false); }
        let mut engine = self.engine.borrow_mut();
        let engine = engine.as_mut().ok_or("session disposed")?;
        let result = self.bridges.run(engine.turn(), |js| js.with_context(|ctx| self.shell.dispatch_lock_state(&ctx)));
        self.mark_reported(self.check_contract())?;
        result.map(|()| true).map_err(|error| self.diagnostic(error).into())
    }

    /// Synchronize desktop reduced-motion preference into all live bridges unless overridden by policy.
    pub(crate) fn update_reduced_motion(&self, portal: Option<bool>) -> Result<bool> {
        let effective = self.policy.reduced_motion.or(portal).unwrap_or(false);
        if self.shutdown_started.get() || self.reduced_motion.get() == effective {
            return Ok(false);
        }
        self.reduced_motion.set(effective);
        if !self.mount_started.get() {
            for owned in self.primary.iter().chain(self.layers.borrow().values()) {
                owned.bridge.seed_reduced_motion(effective);
            }
            return Ok(false);
        }
        let mut engine = self.engine.borrow_mut();
        let engine = engine.as_mut().ok_or("session disposed")?;
        let regions: Vec<Rc<SessionRegion>> = self.primary.iter().chain(self.layers.borrow().values()).cloned().collect();
        let result = self.bridges.run(engine.turn(), |js| js.with_context(|ctx| {
            for owned in &regions {
                owned.bridge.set_reduced_motion(&ctx, effective)?;
            }
            Ok(())
        }));
        self.mark_reported(self.check_contract())?;
        result.map(|()| true).map_err(|error| self.diagnostic(error).into())
    }

    /// Settle active jobs, retire the popup subtree, and notify its owner after cleanup.
    pub(crate) fn dismiss_popup(&self, id: u32, close: impl FnOnce() -> Result<()>) -> Result<()> {
        if self.shutdown_started.get() { return close(); }
        // QuickJS shares one queue: settle active work before a disposer can enqueue into it.
        self.settle_popup_jobs()?;
        close()?;
        self.settle_popup_jobs()?;
        if !self.shell.owns_popup(id) { return Ok(()); }
        let mut engine = self.engine.borrow_mut();
        let engine = engine.as_mut().ok_or("session disposed")?;
        let result = self.bridges.run(engine.turn(), |js| js.with_context(|ctx| self.shell.dispatch_popup_dismiss(&ctx, id)));
        self.mark_reported(self.check_contract())?;
        result.map_err(|error| self.diagnostic(error).into())
    }

    /// Settle active jobs, retire the shell window and its child popups, and notify its owner.
    pub(crate) fn close_window(&self, id: u32, close: impl FnOnce() -> Result<()>) -> Result<()> {
        if self.shutdown_started.get() { return close(); }
        self.settle_popup_jobs()?;
        close()?;
        self.settle_popup_jobs()?;
        if !self.shell.owns_window(id) { return Ok(()); }
        let mut engine = self.engine.borrow_mut();
        let engine = engine.as_mut().ok_or("session disposed")?;
        let result = self.bridges.run(engine.turn(), |js| js.with_context(|ctx| self.shell.dispatch_window_close(&ctx, id)));
        self.mark_reported(self.check_contract())?;
        result.map_err(|error| self.diagnostic(error).into())
    }

    fn settle_popup_jobs(&self) -> Result<()> {
        let mut engine = self.engine.borrow_mut();
        let engine = engine.as_mut().ok_or("session disposed")?;
        if !engine.is_job_pending() {
            self.shell.clear_cleanup_jobs_pending();
            return Ok(());
        }
        let end = self.deadline(Instant::now() + engine.limits().job_timeout);
        let mut remaining = engine.limits().max_jobs_per_pump;
        while engine.is_job_pending() {
            if remaining == 0 || Instant::now() >= end {
                return Err(js_runtime::Diagnostic {
                    kind: js_runtime::DiagnosticKind::BudgetExceeded, phase: "popup dismissal",
                    message: "pending jobs did not settle before the next popup phase".into(),
                    stack: None, generated_stack: None,
                }.into());
            }
            if let Err(error) = self.bridges.run(engine.turn(), |js| js.execute_job(end.saturating_duration_since(Instant::now()))) {
                self.mark_reported(self.check_contract())?;
                return Err(self.diagnostic(error).into());
            }
            remaining -= 1;
        }
        self.shell.clear_cleanup_jobs_pending();
        self.mark_reported(self.check_contract())
    }

    /// Leave staged lock commands queued while the current checkpoint has more JS jobs to check.
    pub(crate) fn lock_commands_ready(&self) -> bool {
        !self.shutdown_started.get() && self.fault.borrow().is_none()
            && self.engine.borrow().as_ref().is_some_and(|engine| !engine.is_job_pending())
    }

    /// Inspect rejections from synchronous unlock teardown without running any queued jobs.
    pub(crate) fn check_unlock_teardown_rejections(&self) -> Result<()> {
        let mut engine = self.engine.borrow_mut();
        let engine = engine.as_mut().ok_or("session disposed")?;
        let result = self.with_cleanup(engine, |engine| engine.drain_rejections(RejectionPolicy::Strict).map(|_| ()));
        let result = self.check_contract().and(result.map_err(SessionError::from));
        if result.is_err() { self.shell.take_lock_command(); }
        self.mark_reported(result)
    }

    /// Build and mount an auxiliary document in the already-running JS engine.
    pub(crate) fn attach_layer(self: &Rc<Self>, id: u32, name: String, region: MountRegion, surface: SurfaceId, lock_claim: Option<std::sync::Weak<dyn Surface>>) -> Result<()> {
        let attach = self.shell.attach(id).ok_or("shell surface was closed before attachment")?;
        let options = ui_bridge::BridgeOptions { strict_styles: self.policy.strict_rejections, reduced_motion: self.reduced_motion.get(), defer_stylesheet_install: true };
        let mut owned = Self::build_region(region, name, surface, self.generation, options, self.waker.clone(), Rc::downgrade(self));
        owned.lock_claim = lock_claim;
        let owned = Rc::new(owned);
        self.bridges.add(&owned.bridge);
        self.layers.borrow_mut().insert(id, owned.clone());
        self.work.add_frame_host(surface, owned.region.host.clone());
        self.install_sheets(&owned);
        for (name, css) in self.shell_stylesheets.borrow().iter() {
            owned.region.host.install_stylesheet(name, css);
        }
        let is_window = self.shell.owns_window(id);
        if let Some(spec) = self.shell.window_spec(id) {
            self.window_controls.seed_surface(surface, WindowState {
                title: spec.title,
                decorations: spec.decorations,
                ..WindowState::default()
            });
        }
        let mut engine = self.engine.borrow_mut();
        let engine = engine.as_mut().ok_or("session disposed")?;
        let result = self.bridges.run(engine.turn(), |js| js.with_context(|ctx| {
            let callback = attach.restore(&ctx)?;
            let api = owned.bridge.api(&ctx)?;
            self.work.install_region_frame_api(&api, owned.region.host.clone())?;
            self.window_controls.install_region_window_api(&api, surface, is_window)?;
            let disposer = owned.bridge.mount_shell_region_with_api(&ctx, &callback, api)?;
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

    /// Look up the mounted region (primary or shell surface) matching `surface`.
    pub(crate) fn region_for_surface(&self, surface: SurfaceId) -> Option<Rc<SessionRegion>> {
        if let Some(primary) = &self.primary && primary.surface == surface {
            return Some(primary.clone());
        }
        self.layers.borrow().values().find(|owned| owned.surface == surface).cloned()
    }

    /// Invoke registered synchronous `onCloseRequest` guards for `surface`.
    /// Returns `Ok(true)` if any guard called `event.preventDefault()` (vetoing close).
    pub(crate) fn request_close(&self, surface: SurfaceId) -> Result<bool> {
        if self.shutdown_started.get() {
            return Ok(false);
        }
        let mut engine = self.engine.borrow_mut();
        let engine = engine.as_mut().ok_or("session disposed")?;
        let result = self.bridges.run(engine.turn(), |js| js.with_context(|ctx| {
            self.window_controls.dispatch_close_request(&ctx, surface)
        }));
        self.mark_reported(self.check_contract())?;
        result.map_err(|error| self.diagnostic(error).into())
    }

    /// Synchronize platform-observed fullscreen/maximized/minimized state into JS subscribers.
    pub(crate) fn sync_window_flags(
        &self,
        surface: SurfaceId,
        fullscreen: bool,
        maximized: bool,
        minimized: Option<bool>,
    ) -> Result<bool> {
        if self.shutdown_started.get() || !self.mount_started.get() {
            return Ok(false);
        }
        let mut engine = self.engine.borrow_mut();
        let engine = engine.as_mut().ok_or("session disposed")?;
        let result = self.bridges.run(engine.turn(), |js| js.with_context(|ctx| {
            self.window_controls.sync_platform_flags(&ctx, surface, fullscreen, maximized, minimized)
        }));
        self.mark_reported(self.check_contract())?;
        result.map_err(|error| self.diagnostic(error).into())
    }

    pub(crate) fn has_window_commands(&self) -> bool {
        self.window_controls.has_commands()
    }

    pub(crate) fn take_window_commands(&self) -> Vec<(SurfaceId, WindowCommand)> {
        self.window_controls.take_commands()
    }

    pub(crate) fn drain_clipboard(&self, clipboard: &dyn zgui::platform::Clipboard) -> bool {
        if self.shutdown_started.get() {
            return false;
        }
        self.work.drain_clipboard(clipboard)
    }

    pub(crate) fn owns_clipboard_read(&self, serial: zgui::platform::ClipboardSerial) -> bool {
        !self.shutdown_started.get() && self.work.owns_clipboard_read(serial)
    }

    pub(crate) fn has_pending_sqlite_ops(&self) -> bool {
        !self.shutdown_started.get() && self.work.has_pending_sqlite_ops()
    }

    pub(crate) fn has_sqlite_events(&self) -> bool {
        !self.shutdown_started.get() && self.work.has_sqlite_events()
    }

    pub(crate) fn has_pending_file_ops(&self) -> bool {
        !self.shutdown_started.get() && self.work.has_pending_file_ops()
    }

    pub(crate) fn has_file_events(&self) -> bool {
        !self.shutdown_started.get() && self.work.has_file_events()
    }

    pub(crate) fn has_network_events(&self) -> bool {
        !self.shutdown_started.get() && self.work.has_network_events()
    }

    /// Marks the session as a staged mount candidate so irreversible outbound network
    /// requests are deferred until native acceptance.
    pub(crate) fn begin_staged_mount(&self) {
        self.work.begin_staged_network();
    }

    /// Releases deferred outbound network operations once the session is accepted.
    pub(crate) fn commit_staged_mount(&self) {
        self.work.commit_staged_network();
    }

    pub(crate) fn register_platform_surface(&self, id: SurfaceId, surface: &Arc<dyn Surface>) {
        self.platform_surfaces
            .borrow_mut()
            .insert(id, Arc::downgrade(surface));
    }

    pub(crate) fn platform_surface(&self, id: SurfaceId) -> Option<Arc<dyn Surface>> {
        self.platform_surfaces
            .borrow()
            .get(&id)
            .and_then(std::sync::Weak::upgrade)
    }

    pub(crate) fn bridge_for_surface(&self, surface: SurfaceId) -> Option<Rc<UiBridge>> {
        if let Some(primary) = &self.primary
            && primary.surface == surface
        {
            return Some(primary.bridge.clone());
        }
        self.layers
            .borrow()
            .values()
            .find(|owned| owned.surface == surface)
            .map(|owned| owned.bridge.clone())
    }

    pub(crate) fn resolve_node_and_surface(
        &self,
        ctx: &rquickjs::Ctx<'_>,
        node: &rquickjs::Class<'_, ui_bridge::NativeNode>,
    ) -> rquickjs::Result<(SurfaceId, zgui::view::NodeId)> {
        if let Some(primary) = &self.primary
            && let Ok(native) = primary.bridge.resolve_native_node(ctx, node)
        {
            return Ok((primary.surface, native));
        }
        for owned in self.layers.borrow().values() {
            if let Ok(native) = owned.bridge.resolve_native_node(ctx, node) {
                return Ok((owned.surface, native));
            }
        }
        Err(Exception::throw_reference(
            ctx,
            "drag source node does not belong to any live surface in this session",
        ))
    }

    pub(crate) fn finish_clipboard_read(
        &self,
        serial: zgui::platform::ClipboardSerial,
        result: std::result::Result<zgui::platform::ClipboardData, zgui::platform::ClipboardError>,
    ) {
        if !self.shutdown_started.get() {
            self.work.finish_clipboard_read(serial, result);
        }
    }

    /// Dispose one auxiliary document while the native surface still exists.
    pub(crate) fn detach_layer(&self, id: u32) -> Result<()> {
        let Some(owned) = self.layers.borrow_mut().remove(&id) else { return Ok(()); };
        // Disposers can remove sheets from the session set after this region leaves it.
        for (name, _) in self.shell_stylesheets.borrow().iter() {
            owned.region.host.remove_stylesheet(name);
        }
        let disposer = owned.disposer.borrow_mut().take();
        let cleanup = self.cleanup(disposer);
        self.window_controls.remove_surface(owned.surface);
        self.bridges.remove(&owned.bridge);
        let scenes = owned.bridge.canvas_scene_count();
        owned.bridge.dispose();
        self.remove_sheets(&owned);
        self.work.remove_frame_host(&owned.region.host);
        if scenes != 0 { return Err(format!("E_TEARDOWN: shell surface {:?} retained {scenes} canvas scenes", owned.name).into()); }
        self.check_contract()?;
        cleanup.map_err(SessionError::from)
    }

    /// Update live occlusion state for a shell surface so unscoped frame callbacks migrate off
    /// occluded surfaces while each visible surface continues ticking on its own cadence.
    pub(crate) fn set_surface_occluded(&self, surface: SurfaceId, occluded: bool) {
        self.work.set_frame_host_occluded(surface, occluded);
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
        if let Some(package) = self.module.as_package() {
            let nodes = self.primary().map_or(0, |primary| primary.bridge.live_count());
            eprintln!("mounted: package={} build={} nodes={nodes}", package.manifest.id, package.manifest.build_id);
        }
        self.record_live_telemetry(engine);
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
            self.fault.borrow_mut().get_or_insert(failure);
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
        let result = self.check_contract().and(result);
        if result.is_err() { self.shell.take_lock_command(); }
        result
    }

    /// Only a mounted generation with a registered live handler accepts CLI requests.
    pub(crate) fn has_control_handler(&self) -> bool { self.work.has_control_handler() }

    /// The control listener namespace authorized by this active live package.
    pub(crate) fn control_app_id(&self) -> Option<&str> {
        if self.shutdown_started.get() { return None; }
        self.module.as_package().filter(|package| package.manifest.capabilities.control_channel)
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
        let mut remaining = [32, 32, 32, max_jobs, 32, 16];
        // Interleave work classes and preserve the next class across deadlines.
        // Ready native work and network floods must not starve timers or promise jobs.
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
                } else if let Some(completion) = self.work.next_clipboard_completion() {
                    self.bridges.run(engine.turn(), |js| js.with_context(|ctx| self.work.settle_clipboard(&ctx, completion)))
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
                } else if let Some((db_id, event)) = self.work.next_sqlite_event() {
                    self.bridges.run(engine.turn(), |js| js.with_context(|ctx| self.work.settle_sqlite(&ctx, db_id, event)))
                        .map_err(|error| self.diagnostic(error))?;
                    true
                } else if let Some(event) = self.work.next_file_event() {
                    self.bridges.run(engine.turn(), |js| js.with_context(|ctx| self.work.settle_file_event(&ctx, event)))
                        .map_err(|error| self.diagnostic(error))?;
                    true
                } else { false },
                5 => if let Some(event) = self.work.next_network_event() {
                    self.bridges.run(engine.turn(), |js| js.with_context(|ctx| self.work.settle_network_event(&ctx, event)))
                        .map_err(|error| self.diagnostic(error))?;
                    true
                } else { false },
                _ => {
                    let job_budget = engine.limits().job_timeout;
                    self.bridges.run(engine.turn(), |js| js.execute_job(job_budget))
                        .map_err(|error| self.diagnostic(error))?
                }
            };
            if attempted { worked = true; } else { remaining[phase] = 0; }
        }
        let policy = if self.strict_rejections() { RejectionPolicy::Strict } else { RejectionPolicy::Log };
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
        let jobs_pending = engine.is_job_pending();
        if !jobs_pending { self.shell.clear_cleanup_jobs_pending(); }
        if jobs_pending || queued || self.work.has_ready() || self.work.has_completions() || self.work.has_clipboard_completions() || self.work.has_clipboard_ops() || self.work.has_external() || self.work.has_sqlite_events() || self.work.has_file_events() || self.work.has_network_events() { self.wake_owner(); }
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

    pub(crate) fn diagnostic(&self, error: js_runtime::Diagnostic) -> js_runtime::Diagnostic {
        match self.module.as_package() { Some(package) => package.remap_diagnostic(error), None => error }
    }

    fn strict_rejections(&self) -> bool {
        self.policy.strict_rejections || self.shell.lock_is_retained()
    }

    fn deadline(&self, end: Instant) -> Instant {
        match &self.policy.deadline { Some(deadline) => end.min(deadline.get()), None => end }
    }

    /// A provider violation is sticky: catching the JavaScript exception cannot
    /// turn an attempted live-service access into a successful preview.
    pub(crate) fn contract_failure(&self) -> Option<ContractFailure> { self.shell.failure().or_else(|| self.work.contract_failure()) }

    fn check_contract(&self) -> Result<()> {
        if let Some(failure) = self.contract_failure() { return Err(failure.into()); }
        // Shell sheets validate synchronously, so a JS exception must not mask their strict fault.
        if let Some(failure) = self.fault.borrow_mut().take_if(|error| matches!(error, SessionError::Style { .. })) {
            return Err(failure);
        }
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

    pub(crate) fn stylesheet_rejected(&self, name: &str, diagnostics: zgui_style::CssDiagnostics) -> String {
        let message = diagnostics.iter().map(|diagnostic| format!("{}:{}: {}", diagnostic.location.line, diagnostic.location.column, diagnostic.message)).collect::<Vec<_>>().join("; ");
        self.report_stylesheet_rejection(name, "STYLESHEET_PARSE_INVALID", &message);
        message
    }

    fn report_stylesheet_rejection(&self, name: &str, code: &'static str, message: &str) {
        if self.policy.strict_rejections {
            self.record_fault(SessionError::Style {
                code: "E_STYLE",
                message: message.to_owned(),
                details: serde_json::json!({"code":code,"sheet":name}),
            });
        } else {
            eprintln!("style-diagnostic: {code}: {name}: {message}");
        }
    }

    pub(crate) fn package(&self) -> Option<&Rc<Package>> { self.module.as_package() }
    pub(crate) fn image_blobs(&self) -> &ImageBlobs { self.work.image_blobs() }

    /// The generation's resolved path authority.
    #[must_use]
    pub(crate) fn resolved_capabilities(&self) -> &ResolvedCapabilities { self.work.resolved_capabilities() }
    pub(crate) fn fixture_image_files(&self) -> impl Iterator<Item = (String, Arc<[u8]>)> + '_ {
        self.policy.assets.fixture_image_files(self.module.as_package().map(Rc::as_ref))
    }
    pub(crate) fn relinquish_stylesheets(&self) { self.stylesheet_installed.set(false); }

    pub(crate) fn restore_stylesheets(&self) {
        if let Some(primary) = self.primary() { self.install_sheets(primary); }
    }

    fn install_sheets(&self, owned: &SessionRegion) {
        for (index, sheet) in self.module.styles().iter().enumerate() {
            owned.region.host.install_stylesheet(&format!("application:{index}"), &sheet.css);
        }
    }

    fn remove_sheets(&self, owned: &SessionRegion) {
        for index in 0..self.module.styles().len() {
            owned.region.host.remove_stylesheet(&format!("application:{index}"));
        }
    }

    /// Validates before changing the shared sheet, even when no mount region exists.
    pub(crate) fn install_shell_stylesheet(&self, name: String, css: String) -> std::result::Result<(), String> {
        let mut sheets = self.shell_stylesheets.borrow_mut();
        let current = sheets.iter_mut().find(|(key, _)| key == &name);
        if current.as_ref().is_some_and(|(_, installed)| installed == &css) { return Ok(()); }
        if self.module.as_package().is_some()
            && let Err(message) = zgui::runtime::image_policy::audit_stylesheet_image_urls(&css)
        {
            self.report_stylesheet_rejection(&name, "STYLESHEET_IMAGE_URL_FORBIDDEN", message);
            return Err(message.to_owned());
        }
        zgui_style::preflight_stylesheet(&css).map_err(|diagnostics| self.stylesheet_rejected(&name, diagnostics))?;
        for owned in self.primary.iter().chain(self.layers.borrow().values()) {
            owned.region.host.install_stylesheet(&name, &css);
        }
        match current {
            Some((_, installed)) => *installed = css,
            None => sheets.push((name, css)),
        }
        Ok(())
    }

    /// Removes the sheet from both current windows and the set inherited by future windows.
    pub(crate) fn remove_shell_stylesheet(&self, name: &str) {
        let mut sheets = self.shell_stylesheets.borrow_mut();
        let Some(index) = sheets.iter().position(|(key, _)| key == name) else { return; };
        sheets.remove(index);
        for owned in self.primary.iter().chain(self.layers.borrow().values()) {
            owned.region.host.remove_stylesheet(name);
        }
    }

    fn clear_shell_stylesheets(&self) {
        let sheets = std::mem::take(&mut *self.shell_stylesheets.borrow_mut());
        if sheets.is_empty() { return; }
        for owned in self.primary.iter().chain(self.layers.borrow().values()) {
            for (name, _) in &sheets {
                owned.region.host.remove_stylesheet(name);
            }
        }
    }

    pub(crate) fn remove_surplus_stylesheets(&self, successor: &Self) {
        if let Some(primary) = self.primary() {
            for index in successor.module.styles().len()..self.module.styles().len() {
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
        let mut missing = std::collections::BTreeSet::new();
        for node in owned.bridge.snapshot(100_000).nodes {
            let Some(key) = zgui_view_dom::id::to_document(node.native) else { continue; };
            for box_id in layout.boxes_of(key) {
                let Some(node) = layout.get(*box_id) else { continue; };
                let entries = zgui_text_style::lower::font::family(node.style.get_font());
                if self.policy.strict_rejections {
                    for family in entries.entries() {
                        if let zgui_text_style::FamilyName::Named(name) = family {
                            let name = name.as_str();
                            if !is_css_generic_font_family(name) && !owned.region.fonts.contains_family(name) {
                                missing.insert(name.to_owned());
                            }
                        }
                    }
                } else {
                    let resolves_cleanly = entries.entries().iter().any(|family| match family {
                        zgui_text_style::FamilyName::Generic(_) => true,
                        zgui_text_style::FamilyName::Named(name) => {
                            let name = name.as_str();
                            is_css_generic_font_family(name) || owned.region.fonts.contains_family(name)
                        }
                    });
                    if !resolves_cleanly {
                        for family in entries.entries() {
                            if let zgui_text_style::FamilyName::Named(name) = family {
                                let name = name.as_str();
                                if !is_css_generic_font_family(name) && !owned.region.fonts.contains_family(name) {
                                    missing.insert(name.to_owned());
                                }
                            }
                        }
                    }
                }
            }
        }
        for family in missing {
            let message = format!("FONT_FAMILY_UNREGISTERED: {family:?} is not registered");
            if self.policy.strict_rejections {
                return Err(SessionError::Style { code: "E_FONT_FAMILY", message, details: serde_json::Value::Null });
            }
            if self.reported_unregistered_fonts.borrow_mut().insert(family) {
                eprintln!("style-diagnostic: {message}");
            }
        }
        owned.font_validation_revision.set(Some(revision));
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn reported_unregistered_fonts(&self) -> Vec<String> {
        self.reported_unregistered_fonts.borrow().iter().cloned().collect()
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
        let sqlite = self.work.sqlite_counts();
        let files = self.work.file_counts();
        let network = self.work.network_counts();
        let mut counts = ResourceCounts {
            callbacks: self.shell.callback_count() + self.window_controls.callback_count(), timers, frame_callbacks, requests,
            image_handles: self.image_blobs().len(),
            unix_sockets: external.sockets,
            child_processes: external.processes,
            control_listeners: external.listeners,
            sqlite_databases: sqlite.databases,
            sqlite_transactions: sqlite.transactions,
            file_handles: files.handles,
            fs_watchers: files.watchers,
            http_requests: network.http_requests,
            event_streams: network.event_streams,
            web_sockets: network.web_sockets,
            active_drags: self.dnd.active_count(),
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
        self.with_cleanup(engine, |engine| {
            let call = |ctx: rquickjs::Ctx<'_>| disposer.restore(&ctx)?.call::<_, ()>(());
            // Cleanup may still release process/image leases after the native bridges faulted.
            if self.primary.iter().chain(self.layers.borrow().values()).any(|owned| owned.bridge.is_disposed()) {
                engine.with_cleanup_context(call)
            } else {
                self.bridges.run(engine.turn(), |js| js.with_cleanup_context(call))
            }
        })
    }

    fn with_cleanup(&self, engine: &mut JsRuntime, cleanup: impl FnOnce(&mut JsRuntime) -> std::result::Result<(), js_runtime::Diagnostic>) -> std::result::Result<(), js_runtime::Diagnostic> {
        self.shell.with_cleanup(|| {
            let result = cleanup(engine);
            // Keep synchronous cleanup restrictions active in queued continuations.
            if engine.is_job_pending() { self.shell.mark_cleanup_jobs_pending(); }
            result.map_err(|error| self.diagnostic(error))
        })
    }

    pub(crate) fn hide_lock_ui(&self) {
        for owned in self.layers.borrow().values() {
            if let Some(claim) = owned.lock_claim.as_ref().and_then(std::sync::Weak::upgrade) {
                claim.set_visible(false);
            }
        }
    }

    /// Dispose JavaScript while native mount regions still exist, visiting popup children first.
    pub(crate) fn dispose(&self, popups_in_close_order: impl IntoIterator<Item = u32>) {
        if self.retired.borrow().is_some() { return; }
        let before = self.counts();
        let pre_dispose_snapshot = self.engine.borrow().as_ref().map(|engine| {
            let live_heap = engine.memory_usage_bytes();
            RuntimeTelemetry {
                max_entry_duration: engine.max_entry_duration(),
                js_heap_bytes: live_heap,
                js_heap_live_bytes: live_heap,
                js_heap_post_dispose_bytes: 0,
                js_stack_limit_bytes: engine.stack_limit_bytes(),
                process_memory: sample_process_memory(),
            }
        });
        self.telemetry.set(RuntimeTelemetry::merge(self.telemetry.get(), pre_dispose_snapshot));
        self.shutdown_started.set(true);
        self.hide_lock_ui();
        self.bridges.begin_stop();
        self.shell.begin_stop();
        self.work.begin_stop();
        self.fault.borrow_mut().take();
        let mut cleanup_error = None;
        for id in popups_in_close_order {
            let disposer = self.layers.borrow().get(&id).and_then(|owned| owned.disposer.borrow_mut().take());
            if let Err(error) = self.cleanup(disposer) {
                cleanup_error.get_or_insert(error);
            }
        }
        if let Err(error) = self.cleanup(self.disposer.borrow_mut().take()) {
            cleanup_error.get_or_insert(error);
        }
        for owned in self.layers.borrow().values() {
            if let Err(error) = self.cleanup(owned.disposer.borrow_mut().take()) {
                cleanup_error.get_or_insert(error);
            }
        }
        if self.strict_rejections() && let Some(engine) = self.engine.borrow_mut().as_mut()
            && let Err(error) = engine.drain_rejections(RejectionPolicy::Strict)
        {
            cleanup_error.get_or_insert_with(|| self.diagnostic(error));
        }
        if let Some(error) = &cleanup_error { eprintln!("disposer error: {error}"); }
        let external = self.work.external_counts();
        let sqlite = self.work.sqlite_counts();
        let files = self.work.file_counts();
        let network = self.work.network_counts();
        let active_drags = self.dnd.active_count();
        self.cancel_outbound_drag_on_platform();
        self.dnd.clear();
        if let Some(dev_state) = &self.dev_state {
            dev_state.borrow_mut().clear();
        }
        self.platform_surfaces.borrow_mut().clear();
        self.work.finish_stop();
        // Report remaining leases before forced disposal releases the native scenes.
        let canvas_scenes = self.counts().canvas_scenes;
        let window_callbacks = self.window_controls.callback_count();
        self.window_controls.clear();
        self.bridges.dispose();
        self.shell.clear();
        self.clear_shell_stylesheets();
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
        let mut counts = self.counts();
        counts.callbacks += window_callbacks;
        counts.canvas_scenes = canvas_scenes;
        counts.unix_sockets = external.sockets;
        counts.child_processes = external.processes;
        counts.control_listeners = external.listeners;
        counts.sqlite_databases = sqlite.databases;
        counts.sqlite_transactions = sqlite.transactions;
        counts.file_handles = files.handles;
        counts.fs_watchers = files.watchers;
        counts.http_requests = network.http_requests;
        counts.event_streams = network.event_streams;
        counts.web_sockets = network.web_sockets;
        counts.active_drags = active_drags;
        self.work.clear_images();
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

    /// Current read-only runtime telemetry without mutating or evaluating JS.
    pub(crate) fn live_telemetry(&self) -> Option<RuntimeTelemetry> {
        let sampled = self.engine.borrow().as_ref().map(|engine| {
            let live_heap = engine.memory_usage_bytes();
            RuntimeTelemetry {
                max_entry_duration: engine.max_entry_duration(),
                js_heap_bytes: live_heap,
                js_heap_live_bytes: live_heap,
                js_heap_post_dispose_bytes: 0,
                js_stack_limit_bytes: engine.stack_limit_bytes(),
                process_memory: None,
            }
        });
        RuntimeTelemetry::merge(self.telemetry.get(), sampled)
    }

    /// Pending checkpoint work depth (events + work items) for dev profiling.
    pub(crate) fn checkpoint_queue_depth(&self) -> usize {
        self.work.pending_count()
    }

    /// Capture a bounded, plain-data dev reload snapshot if this generation is
    /// settled, unlocked, non-faulted, and matches the target module's scope.
    pub(crate) fn capture_dev_state(&self, target: &SessionModule) -> Option<DevStateSnapshot> {
        let store = self.dev_state.as_ref()?;
        if self.shutdown_started.get()
            || self.fault.borrow().is_some()
            || self.shell.lock_is_retained()
            || !self.mount_started.get()
        {
            return None;
        }
        let own_scope = DevStateScope::for_module(&self.module);
        let next_scope = DevStateScope::for_module(target);
        if own_scope != next_scope {
            return None;
        }
        store.borrow().snapshot(own_scope)
    }

    /// Report residual resources after the host has attempted native surface closure.
    pub(crate) fn retire(&self) -> SessionRetirement {
        self.dispose([]);
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

    fn record_live_telemetry(&self, engine: &JsRuntime) {
        let live_heap = engine.memory_usage_bytes();
        let sample = RuntimeTelemetry {
            max_entry_duration: engine.max_entry_duration(),
            js_heap_bytes: live_heap,
            js_heap_live_bytes: live_heap,
            js_heap_post_dispose_bytes: 0,
            js_stack_limit_bytes: engine.stack_limit_bytes(),
            process_memory: None,
        };
        self.telemetry.set(RuntimeTelemetry::merge(self.telemetry.get(), Some(sample)));
    }

    fn snapshot_telemetry(engine: &JsRuntime) -> RuntimeTelemetry {
        let post_dispose = engine.memory_usage_bytes();
        RuntimeTelemetry {
            max_entry_duration: engine.max_entry_duration(),
            js_heap_bytes: post_dispose,
            js_heap_live_bytes: 0,
            js_heap_post_dispose_bytes: post_dispose,
            js_stack_limit_bytes: engine.stack_limit_bytes(),
            process_memory: None,
        }
    }
}
impl Drop for Session { fn drop(&mut self) { let _ = self.retire(); } }

fn sample_process_memory() -> Option<ProcessMemory> {
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/self/smaps_rollup").ok()?;
        let mut memory = ProcessMemory::default();
        let mut saw_rss = false;
        for line in text.lines() {
            let Some((key, rest)) = line.split_once(':') else { continue; };
            let Some(kb_str) = rest.split_whitespace().next() else { continue; };
            let Ok(kb) = kb_str.parse::<u64>() else { continue; };
            let bytes = kb.saturating_mul(1024);
            match key {
                "Rss" => { memory.rss_bytes = bytes; saw_rss = true; }
                "Pss" => memory.pss_bytes = bytes,
                "Pss_Anon" => memory.pss_anon_bytes = bytes,
                "Pss_File" => memory.pss_file_bytes = bytes,
                "Private_Dirty" => memory.private_dirty_bytes = bytes,
                _ => {}
            }
        }
        saw_rss.then_some(memory)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

fn is_css_generic_font_family(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "serif"
            | "sans-serif"
            | "monospace"
            | "cursive"
            | "fantasy"
            | "system-ui"
            | "ui-serif"
            | "ui-sans-serif"
            | "ui-monospace"
            | "ui-rounded"
            | "emoji"
            | "math"
            | "fangsong"
    )
}

fn remap(module: &SessionModule, error: js_runtime::Diagnostic) -> SessionError {
    SessionError::Diagnostic(match module.as_package() {
        Some(package) => package.remap_diagnostic(error),
        None => error,
    })
}

#[cfg(test)]
mod tests;
