use std::{cell::{Cell, RefCell}, collections::BTreeMap, rc::Rc, time::{Duration, Instant}};

use serde_json::{Value, json};
use zgui::{geom::{DevicePx, Point, Size}, platform::{AppHandler, ColorScheme, IdlePolicy, MonitorInfo, PlatformCapabilities, PlatformCx, Surface as PlatformSurface, SurfaceEvent, SurfaceId, WakeReason}, runtime::{Runtime, Window}};
use zgui_platform_headless::{Headless, SessionLockDecision};

use crate::{package::Package, session::{ResourceCounts, RetirementFailure, RuntimeTelemetry, Session, SessionError, SessionModule, SessionPolicy, SessionRetirement}};
use crate::session_host::{ImageChoice, RecoveryPolicy, RuntimeSurface, SessionHost, SurfaceOpacity, TurnError};
use crate::shell::Monitor;
use super::{Error, fixture::FixtureDefinition, output::Output, pipeline::{self, CaptureRequest, CheckpointSink, FrameObserver, FrameObservers, ObservationMode, OutputSink, Pipeline}, profile::{self, Limits, LoadedProfile, Surface}, resources};
use crate::{assets::{ContractFailure, fixture::{FixtureRuntime, LockOutcome}}, environment::Environment};

mod actions;
mod timeline;

pub type SharedOutput = Rc<RefCell<Output>>;

#[derive(Clone, Debug)]
pub struct Fault {
    pub error: Error,
    pub diagnostic: Option<Value>,
}
impl From<Error> for Fault {
    fn from(error: Error) -> Self { Self { error, diagnostic: None } }
}
impl From<ContractFailure> for Fault {
    fn from(error:ContractFailure)->Self {
        Error::new(error.code,if error.code=="E_ASSERTION"{4}else{6},error.message).into()
    }
}
impl Fault {
    pub(crate) fn application(error: SessionError) -> Self {
        match error {
            SessionError::Contract(contract) => contract.into(),
            SessionError::Diagnostic(diagnostic) => {
                let (code, exit) = match diagnostic.kind {
                    js_runtime::DiagnosticKind::Interrupted | js_runtime::DiagnosticKind::BudgetExceeded => ("E_JOB_BUDGET", 5),
                    _ if diagnostic.phase == "promise rejection" => ("E_UNHANDLED_REJECTION", 3),
                    _ => ("E_JS_EXCEPTION", 3),
                };
                Self {
                    error: Error::new(code, exit, &diagnostic.message),
                    diagnostic: Some(json!({"kind":format!("{:?}",diagnostic.kind),"phase":diagnostic.phase,
                        "message":diagnostic.message,"generatedStack":diagnostic.generated_stack.as_ref().or(diagnostic.stack.as_ref()),
                        "mappedStack":diagnostic.stack,"mappingApplied":diagnostic.generated_stack.is_some()})),
                }
            }
            SessionError::InvalidState(message) => Error::new("E_HOST_INVARIANT", 9, message).into(),
            SessionError::Style { code, message, details } => Error::new(code, 3, message).with_details(details).into(),
        }
    }
}

struct PreviewHost {
    native: Runtime,
    host: SessionHost,
    deadline: Rc<Cell<Instant>>,
    run_deadline: Instant,
    step_budget: Duration,
    teardown_budget: Duration,
    observers: FrameObservers,
    monitors: Vec<Monitor>,
    recreate_shell: Option<String>,
    pending_reload: Option<SessionModule>,
    last_reload: Option<crate::session_host::ReloadOutcome>,
    fault: Option<Fault>,
    retired: Option<SessionRetirement>,
    retired_telemetry: Option<RuntimeTelemetry>,
}
impl PreviewHost {
    fn session(&self) -> Option<&Rc<Session>> { self.host.sessions().next() }
    fn fail(&mut self, cx: &dyn PlatformCx, fault: Fault) {
        if self.fault.is_none() { self.fault = Some(fault); }
        self.host.hide_lock_ui();
        // The handler guards stop queued entries. The driver publishes the cause
        // before bounded disposal, so a blocked cleanup cannot erase that cause.
        cx.request_exit();
    }
    fn after_native(&mut self, cx: &dyn PlatformCx) {
        if self.fault.is_some() { return; }
        if let Some(error) = self.native.failure().take() { self.fail(cx, pipeline::preview_app_error(error).into()); return; }
        if self.host.pending_regions() != 0 {
            self.host.set_monitor_snapshot(&self.monitors);
            if self.host.sessions().next().is_some() || self.native.windows().len() != 1 {
                self.fail(cx, Error::new("E_HOST_INVARIANT", 9, "preview requires exactly one current native document").into()); return;
            }
            let retry: Rc<dyn Fn(zgui::view::DocumentId, u64)> = Rc::new(|_, _| {});
            let attached = {
                let mut surface = RuntimeSurface::new(&mut self.native, cx, ImageChoice::PackageOnly);
                self.host.attach(&mut surface, cx.waker(), retry)
            };
            if let Err(error) = attached { self.fail(cx, Fault::application(error)); return; }
        }
        let retry: Rc<dyn Fn(zgui::view::DocumentId, u64)> = Rc::new(|_, _| {});
        if let Err(TurnError::Checkpoint(error) | TurnError::Shell(error)) =
            self.host.turn(&mut self.native, cx, ImageChoice::PackageOnly, &self.monitors, &retry)
        {
            self.fail(cx, Fault::application(error));
            return;
        }
        if let Some(name) = self.recreate_shell.take()
            && let Err(error) = self.host.recreate_shell_surface(&mut self.native, cx, ImageChoice::PackageOnly, &self.monitors, &name)
        {
            self.fail(cx, Fault::application(error));
            return;
        }
        if let Some(module) = self.pending_reload.take() {
            let primary_document = self.host.region_documents().next();
            let outcome = if self.host.is_pure_shell() {
                self.host.replace_shell(&mut self.native, cx, ImageChoice::PackageOnly, module)
            } else if let Some(document) = primary_document {
                let mut surface = RuntimeSurface::new(&mut self.native, cx, ImageChoice::PackageOnly);
                self.host.replace(&mut surface, cx.waker(), document, module, retry.clone())
            } else {
                crate::session_host::ReloadOutcome::Rejected("no active mount region to reload".into())
            };
            self.last_reload = Some(outcome);
            if let Err(error) = self.host.shell_turn(&mut self.native, cx, ImageChoice::PackageOnly, &self.monitors, &retry) {
                self.fail(cx, Fault::application(error));
                return;
            }
        }
        let Some(session) = self.session() else { return; };
        for window in self.native.windows() {
            if let Some(region) = session.mount_region(window.document_id())
                && let Some(observer) = self.observers.get(region.surface)
            {
                observer.bind_session(Rc::downgrade(session));
                observer.request_metadata_frame(window, region.bridge.metadata_revision());
            }
        }
        self.observers.retain(self.native.windows());
    }
}
impl AppHandler for PreviewHost {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) {
        self.native.surfaces_available(cx);
        // Cold headless graphics / Lavapipe initialization runs inside `native.surfaces_available`
        // before any JS executes; start the initial bootstrap step budget afterward while still
        // clamping to `run_deadline`.
        if self.host.generation() == 0 {
            self.deadline.set(self.run_deadline.min(Instant::now() + self.step_budget));
        }
        self.after_native(cx);
    }
    fn surfaces_lost(&mut self, cx: &dyn PlatformCx) {
        let deadline = self.deadline.get();
        self.deadline.set(deadline.min(Instant::now() + self.teardown_budget));
        self.host.stop_shell_surfaces(&mut self.native, cx);
        if let Some(session) = self.session() {
            let report = session.retire();
            if let Some(fault) = evaluate_retirement(Some(&report)) {
                self.fail(cx, fault);
            }
            self.retired_telemetry = RuntimeTelemetry::merge(self.retired_telemetry, report.telemetry);
            self.retired = Some(report);
        }
        self.host.stop(&mut RuntimeSurface::new(&mut self.native, cx, ImageChoice::PackageOnly));
        self.deadline.set(deadline);
        self.observers.reset();
        self.native.surfaces_lost(cx);
    }
    fn surface_event(&mut self, cx: &dyn PlatformCx, surface: SurfaceId, event: SurfaceEvent) {
        if self.fault.is_some() { return; }
        if let Err(error) = self.host.surface_event(&mut self.native, cx, surface, event) {
            self.fail(cx, Fault::application(error));
            return;
        }
        self.after_native(cx);
    }
    fn wake(&mut self, cx: &dyn PlatformCx, reason: WakeReason) {
        if self.fault.is_some() { return; }
        match reason {
            WakeReason::ClipboardRead { serial, result } => {
                if let Err(result) = self.host.resolve_clipboard_read(serial, result) {
                    self.native.wake(cx, WakeReason::ClipboardRead { serial, result });
                }
            }
            other => self.native.wake(cx, other),
        }
        self.after_native(cx);
    }
    fn idle(&mut self, cx: &dyn PlatformCx) -> IdlePolicy { if self.fault.is_some() { IdlePolicy::Block } else { self.native.idle(cx) } }
    fn deadline_reached(&mut self, cx: &dyn PlatformCx) { if self.fault.is_none() { self.native.deadline_reached(cx); self.after_native(cx); } }
    fn shutting_down(&mut self, cx: &dyn PlatformCx) {
        self.host.stop_shell_surfaces(&mut self.native, cx);
        for session in self.host.sessions() { let _ = session.retire(); }
        self.native.shutting_down(cx);
    }
}

#[derive(Clone, Copy, PartialEq)]
struct ShellGeometry {
    width: f64,
    height: f64,
    scale: f64,
    refresh: Option<u32>,
}

pub struct Driver {
    pipeline: Pipeline<PreviewHost>,
    observers: FrameObservers,
    shell_geometry: BTreeMap<SurfaceId, ShellGeometry>,
    deadline: Rc<Cell<Instant>>,
    run_deadline: Instant,
    teardown_budget: Duration,
    step_start_frame: Cell<u64>,
    limits: Limits,
    dimensions: (u32,u32),
    surface: Surface,
    environment: Rc<Environment>,
    pub(super) fixtures: Rc<FixtureRuntime>,
    maximum_time_us: u64,
    pure_shell: bool,
    shell_capable: bool,
    lock_capable: bool,
    bundle_dir: std::path::PathBuf,
    scenario_dir: std::path::PathBuf,
    /// A mount fault observed while the harness already owned the session; the
    /// first `check` reports it so the run can persist the cause before finish.
    startup_fault: Option<Fault>,
}
pub(super) fn stylesheet(profile:&LoadedProfile)->String {
    format!("{} :root {{ display:block; box-sizing:border-box; width:100%; height:100%; background-color:{}; font-family:{}; {} }}",
        crate::APPLICATION_STYLESHEET,profile.profile.surface.background,profile.font_family_css,
        if profile.profile.theme=="light" { "color:#171b24;" } else { "" })
}

#[derive(Default)]
pub struct ReloadConfig {
    pub dev_state: bool,
    pub bundle_dir: std::path::PathBuf,
    pub scenario_dir: std::path::PathBuf,
}

impl Driver {
    pub fn new(
        profile: &LoadedProfile,
        package: Rc<Package>,
        fixture: FixtureDefinition,
        output: SharedOutput,
        run_deadline: Instant,
        stylesheet: String,
        reload: ReloadConfig,
    ) -> Result<Self, Fault> {
        Self::with_sink_configured(
            profile,
            package,
            fixture,
            Box::new(OutputSink(output)),
            run_deadline,
            stylesheet,
            reload,
        )
    }

    #[cfg(test)]
    pub(super) fn with_sink(profile: &LoadedProfile, package: Rc<Package>, fixture:FixtureDefinition, sink: Box<dyn CheckpointSink>, run_deadline: Instant, stylesheet:String) -> Result<Self, Fault> {
        Self::with_sink_configured(profile, package, fixture, sink, run_deadline, stylesheet, ReloadConfig::default())
    }

    fn with_sink_configured(
        profile: &LoadedProfile,
        package: Rc<Package>,
        fixture: FixtureDefinition,
        sink: Box<dyn CheckpointSink>,
        run_deadline: Instant,
        stylesheet: String,
        reload: ReloadConfig,
    ) -> Result<Self, Fault> {
        let ReloadConfig { dev_state, bundle_dir, scenario_dir } = reload;
        let mut capabilities = PlatformCapabilities::none();
        capabilities.session_lock = true;
        capabilities.clipboard_formats = vec![
            zgui::platform::ClipboardFormat::Text,
            zgui::platform::ClipboardFormat::Image,
        ];
        let platform = Headless::new().with_capabilities(capabilities)
            .with_blur_capabilities(profile.profile.background_blur, profile.profile.blur_shapes);
        if let Some(clip) = &fixture.services.clipboard {
            use zgui::platform::{Clipboard, ClipboardData, ClipboardKind, ClipboardWriteOptions};
            let opts = if clip.secret { ClipboardWriteOptions::SECRET } else { ClipboardWriteOptions::default() };
            if let Some(text) = &clip.text {
                let _ = platform.memory_clipboard().write(ClipboardKind::Standard, ClipboardData::Text(text.clone().into()), opts);
            } else if let Ok(Some(bytes)) = clip.image_bytes() {
                let _ = platform.memory_clipboard().write(
                    ClipboardKind::Standard,
                    ClipboardData::Image { media_type: "image/png".into(), bytes },
                    opts,
                );
            }
        }
        if let Some(lock) = &fixture.services.session_lock {
            platform.set_session_lock_decision(match lock.outcome {
                LockOutcome::Locked => SessionLockDecision::Lock,
                LockOutcome::Finished => SessionLockDecision::Refuse,
            });
        }
        let (epoch,start)=match profile.profile.clock {
            profile::Clock::Virtual{epoch_ms,start_us}=>(Some(epoch_ms),start_us),
            profile::Clock::Wall{}=>(None,0),
        };
        platform.virtual_clock().advance(Duration::from_micros(start));
        let environment=Environment::new(platform.clock(),epoch,profile.profile.random.seed);
        let fixtures=FixtureRuntime::new(fixture,environment.clone());
        let lock_capable = package.manifest.capabilities.session_lock;
        let shell_capable = package.manifest.capabilities.shell_surfaces || lock_capable;
        let pure_shell = profile.profile.pure_shell;
        if pure_shell && !shell_capable {
            return Err(Error::new("E_CAPABILITY", 6, "pureShell requires capabilities.shellSurfaces or capabilities.sessionLock").into());
        }
        let mode = if pure_shell { ObservationMode::ShellOnly }
            else if shell_capable { ObservationMode::PrimaryAndShell }
            else { ObservationMode::PrimaryOnly };
        let observers = FrameObservers::new(package.manifest.build_id.clone(), environment.clone(), sink, mode);
        observers.set_emit_blur_shapes(profile.profile.blur_shapes);
        let pending = Rc::new(RefCell::new(Vec::new()));
        let surface = &profile.profile.surface;
        let step_budget = Duration::from_millis(profile.profile.limits.max_step_wall_ms);
        let deadline = Rc::new(Cell::new(run_deadline.min(Instant::now() + step_budget)));
        let defaults = js_runtime::RuntimeLimits::default();
        let entry_timeout = Duration::from_millis(profile.profile.limits.max_js_entry_wall_ms);
        let limits = js_runtime::RuntimeLimits {
            entry_timeout,
            bootstrap_timeout: step_budget,
            cleanup_timeout: defaults.cleanup_timeout.max(entry_timeout.min(Duration::from_millis(250))),
            max_jobs_per_pump:profile.profile.limits.max_jobs_per_pump as usize,
            job_timeout:Duration::from_millis(profile.profile.limits.max_pump_wall_ms),
            stack_limit_bytes: profile.profile.limits.max_js_stack_bytes.unwrap_or(defaults.stack_limit_bytes),
            ..defaults
        };
        // Native surface shutdown and telemetry share the enclosing window, not the JS entry's budget.
        let teardown_budget = step_budget.min(Duration::from_secs(1)).max(limits.cleanup_timeout);
        let shell_opacity = if surface.opaque { SurfaceOpacity::Opaque } else { SurfaceOpacity::Transparent };
        let mut session_host = SessionHost::new(pending.clone(), SessionModule::package(package.clone()),
            SessionPolicy::preview(limits, deadline.clone(), environment.clone(), fixtures.clone())
                .with_reduced_motion(profile.profile.reduced_motion)
                .with_dev_state(dev_state),
            RecoveryPolicy::Propagate, shell_opacity);
        if pure_shell { session_host.set_pure_shell(profile.fonts.clone()); }
        platform.set_color_scheme(Some(if profile.profile.theme == "dark" { ColorScheme::Dark } else { ColorScheme::Light }));
        let observer_ref = observers.clone();
        let deadline_ref = deadline.clone();
        let pipeline = pipeline::for_preview(profile, &package, platform, stylesheet, &observers, pending, move |native| PreviewHost {
            native,
            host: session_host,
            deadline: deadline_ref,
            run_deadline,
            step_budget,
            teardown_budget,
            observers: observer_ref,
            monitors: Vec::new(),
            recreate_shell: None,
            pending_reload: None,
            last_reload: None,
            fault: None,
            retired: None,
            retired_telemetry: None,
        })?;
        let mut driver = Self { pipeline,observers,shell_geometry:BTreeMap::new(),deadline,run_deadline,teardown_budget,step_start_frame:Cell::new(0),limits:profile.profile.limits.clone(),
            dimensions:(profile.physical_width,profile.physical_height),surface:surface.clone(),
            environment,fixtures,maximum_time_us:start+profile.profile.limits.max_virtual_duration_ms*1000,startup_fault:None,pure_shell,shell_capable,lock_capable,bundle_dir,scenario_dir };
        let setup = if pure_shell { driver.check() } else { driver.bind_main() };
        if let Err(fault) = setup.and_then(|_| driver.sync_shell_surfaces()) { driver.startup_fault = Some(fault); }
        Ok(driver)
    }
    fn bind_main(&mut self) -> Result<(), Fault> {
        self.check()?;
        let surface = self.window("main")?.surface().id();
        let offscreen = self.pipeline.platform().offscreen(surface)
            .ok_or_else(||Error::new("E_HOST_INVARIANT",9,"current native main has no headless surface"))?;
        offscreen.set_refresh_rate_millihertz(Some(self.surface.refresh_rate_milli_hz));
        offscreen.set_logical_size(Size::new(
            zgui::geom::CssPx(self.surface.width as f32),
            zgui::geom::CssPx(self.surface.height as f32),
        ));
        self.observer(surface)?.bind_surface(offscreen);
        self.pipeline.deliver(surface, SurfaceEvent::ScaleFactorChanged { scale_factor:self.surface.scale,
            size:Size::new(DevicePx(self.dimensions.0 as f32),DevicePx(self.dimensions.1 as f32)) });
        self.pipeline.deliver(surface, SurfaceEvent::Focused(true));
        self.window("main")?.request_frame();
        self.check()
    }

    fn sync_shell_surfaces(&mut self) -> Result<(), Fault> {
        let app = self.pipeline.app();
        self.shell_geometry.retain(|id, _| app.native.windows().iter().any(|window| window.surface().id() == *id));
        // The platform context can change independently of a shell declaration.
        // Keep synthetic configuration separate from explicit scenario resizes.
        let changed: Vec<_> = app.native.windows().iter().filter_map(|window| {
            let id = window.surface().id();
            let (visual_width, visual_height, output) = app.host.shell_geometry(id)?;
            let visual_height = window.popup_auto_height_resolved().map_or(visual_height, f64::from);
            let insets = app.host.shell_geometry_insets(id);
            let width = visual_width + insets.left + insets.right;
            let height = visual_height + insets.top + insets.bottom;
            let monitor = match output {
                Some(output) => app.monitors.iter().find(|monitor| monitor.id == output),
                None => app.monitors.first(),
            };
            let next = ShellGeometry { width, height, scale: monitor.map_or(1.0, |monitor| monitor.scale),
                refresh: monitor.and_then(|monitor| monitor.refresh_rate_milli_hz) };
            (self.shell_geometry.get(&id) != Some(&next)).then_some((id, next))
        }).collect();
        for (surface, geometry) in changed {
            if self.by_surface(surface).is_none() { continue; }
            let dimensions = profile::physical_dimensions(geometry.width, geometry.height, geometry.scale)?;
            let previous = self.shell_geometry.insert(surface, geometry);
            let offscreen = self.pipeline.platform().offscreen(surface)
                .ok_or_else(|| Error::new("E_HOST_INVARIANT",9,"named native window has no headless surface"))?;
            let observer = self.observer(surface)?;
            offscreen.set_refresh_rate_millihertz(geometry.refresh);
            let target_size = Size::new(DevicePx(dimensions.0 as f32), DevicePx(dimensions.1 as f32));
            let target_logical = Size::new(
                zgui::geom::CssPx(geometry.width as f32),
                zgui::geom::CssPx(geometry.height as f32),
            );
            if previous.is_some()
                && offscreen.size() == target_size
                && offscreen.logical_size() == target_logical
                && (offscreen.scale_factor() - geometry.scale).abs() < f64::EPSILON
            {
                continue;
            }
            offscreen.set_logical_size(target_logical);
            if previous.is_none() { observer.bind_surface(offscreen); }
            else { observer.bump_surface_configuration(); }
            self.pipeline.deliver(surface, SurfaceEvent::ScaleFactorChanged {
                scale_factor: geometry.scale, size: target_size,
            });
            if previous.is_none() { self.pipeline.deliver(surface, SurfaceEvent::Focused(true)); }
            if let Some(window) = self.by_surface(surface) { window.request_frame(); }
            self.check()?;
        }
        Ok(())
    }

    /// Toggle live offscreen compositor background blur support and settle reactive updates.
    pub fn set_background_blur_supported(&mut self, supported: bool) -> Result<Value, Fault> {
        self.settle(false)?;
        self.pipeline.platform_mut().set_background_blur_supported(supported);
        self.settle(false)?;
        self.sync_shell_surfaces()?;
        Ok(json!({
            "supported": supported,
            "timeUs": self.environment.now_us(),
            "readiness": self.readiness(),
        }))
    }

    pub fn set_blur_shapes_supported(&mut self, supported: bool) -> Result<Value, Fault> {
        self.settle(false)?;
        self.observers.set_emit_blur_shapes(supported);
        self.pipeline.platform_mut().set_blur_shapes_supported(supported);
        self.settle(false)?;
        self.sync_shell_surfaces()?;
        Ok(json!({
            "supported": supported,
            "timeUs": self.environment.now_us(),
            "readiness": self.readiness(),
        }))
    }

    /// Apply a preview output snapshot and settle the resulting shell surface changes.
    pub fn set_monitors(&mut self, monitors: &[Monitor]) -> Result<Value, Fault> {
        self.settle(false)?;
        let normalized: Vec<Monitor> = monitors.iter().cloned().map(|mut monitor| {
            monitor.normalize_logical_size();
            monitor
        }).collect();
        if self.pipeline.app().monitors != normalized {
            let native = normalized.iter().map(|monitor| {
                let mut info = MonitorInfo::new(
                    Point::new(DevicePx(monitor.x as f32), DevicePx(monitor.y as f32)),
                    Size::new(DevicePx(monitor.width as f32), DevicePx(monitor.height as f32)), monitor.scale,
                )
                .with_logical_size(Size::new(
                    zgui::geom::CssPx(monitor.logical_width() as f32),
                    zgui::geom::CssPx(monitor.logical_height() as f32),
                ))
                .with_name(monitor.id.clone());
                info.refresh_rate_millihertz = monitor.refresh_rate_milli_hz;
                info
            }).collect();
            self.pipeline.platform().set_monitors(native);
            let app = self.pipeline.app_mut();
            app.monitors = normalized.clone();
        }
        self.settle(false)?;
        let surfaces = self.pipeline.app().session().map(|session| {
            session.mount_regions().into_iter().map(|(name, _)| name).collect::<Vec<_>>()
        }).unwrap_or_default();
        Ok(json!({"monitors":normalized,"surfaces":surfaces,"timeUs":self.environment.now_us(),"readiness":self.readiness()}))
    }
    pub fn generation(&self) -> u64 { self.pipeline.app().host.generation() }
    /// Recreate the real native document while retaining run time, RNG and fixture history.
    pub fn lifecycle_reload(
        &mut self,
        bundle: Option<&str>,
        expect: super::scenario::ReloadExpect,
    ) -> Result<Value, Fault> {
        use super::scenario::ReloadExpect;
        self.check()?;
        let target_path = match bundle {
            Some(path) => {
                let pb = std::path::PathBuf::from(path);
                if pb.is_absolute() { pb } else { self.scenario_dir.join(pb) }
            }
            None => self.bundle_dir.clone(),
        };
        let package = match Package::load(&target_path) {
            Ok(package) => Rc::new(package),
            Err(error) => {
                if expect == ReloadExpect::Rejected {
                    return Ok(json!({
                        "status": "rejected",
                        "stage": "package",
                        "generation": self.generation(),
                        "diagnostic": error.to_string(),
                    }));
                }
                return Err(Error::new("E_BUNDLE", 3, error.to_string()).into());
            }
        };
        let build_id = package.manifest.build_id.clone();
        self.pipeline.app_mut().pending_reload = Some(SessionModule::package(package));
        self.pipeline.platform().waker().wake(WakeReason::AppWork);
        self.pump_application()?;
        let outcome = self.pipeline.app_mut().last_reload.take();
        match outcome {
            Some(crate::session_host::ReloadOutcome::Mounted { generation, predecessor_leak, css_only }) => {
                if expect == ReloadExpect::Rejected {
                    return Err(Error::new(
                        "E_ASSERTION",
                        4,
                        format!("expected reload candidate to be rejected, but generation {generation} was accepted"),
                    ).into());
                }
                self.bundle_dir = target_path;
                self.settle(false)?;
                let mut trace = json!({
                    "status": "accepted",
                    "generation": generation,
                    "buildId": build_id,
                });
                if css_only {
                    trace["cssOnly"] = json!(true);
                }
                if let Some(leak) = predecessor_leak {
                    trace["predecessorLeak"] = json!(leak);
                }
                Ok(trace)
            }
            Some(crate::session_host::ReloadOutcome::Rejected(error) | crate::session_host::ReloadOutcome::Fallback(error)) => {
                if expect == ReloadExpect::Accepted {
                    return Err(Fault::application(error));
                }
                self.settle(false)?;
                Ok(json!({
                    "status": "rejected",
                    "generation": self.generation(),
                    "diagnostic": error.to_string(),
                }))
            }
            None => Err(Error::new("E_HOST_INVARIANT", 9, "lifecycle.reload did not produce a reload outcome").into()),
        }
    }

    pub fn recreate(&mut self, name: &str) -> Result<Value, Fault> {
        self.check()?;
        let old_surface = self.window(name)?.surface().id();
        let old_document = format!("{:?}",self.window(name)?.document_id());
        let old_generation = self.generation();
        let time = self.environment.now_us();
        if name != "main" {
            self.pipeline.app_mut().recreate_shell = Some(name.to_owned());
            self.pipeline.platform().waker().wake(WakeReason::AppWork);
            self.settle_surface(Some(name))?;
            let window = self.window(name)?;
            let new_surface = window.surface().id();
            let new_document = format!("{:?}", window.document_id());
            if new_surface == old_surface || new_document == old_document || self.generation() != old_generation {
                return Err(Error::new("E_HOST_INVARIANT",9,"named recreate did not replace exactly one mount region").into());
            }
            return Ok(json!({"oldSurfaceId":format!("{old_surface:?}"),"newSurfaceId":format!("{new_surface:?}"),
                "oldDocumentId":old_document,"newDocumentId":new_document,"oldGeneration":old_generation,
                "generation":self.generation(),"fromTimeUs":time,"timeUs":self.environment.now_us(),
                "readiness":self.readiness_for(Some(name))}));
        }
        self.pipeline.suspend();
        self.check()?;
        if !self.pipeline.app().native.windows().is_empty() {
            return Err(Error::new("E_HOST_INVARIANT",9,"suspended runtime retained native windows").into());
        }
        // The native harness may retain old offscreen entries. Only Runtime.windows owns main.
        self.pipeline.resume();
        self.check()?;
        let new_surface = self.window("main")?.surface().id();
        let new_document = format!("{:?}",self.window("main")?.document_id());
        if new_surface==old_surface || new_document==old_document || self.generation()!=old_generation+1 {
            return Err(Error::new("E_HOST_INVARIANT",9,"surface resume did not create a fresh native generation").into());
        }
        self.observer(new_surface)?.bump_surface_configuration();
        self.bind_main()?;
        self.settle(true)?;
        let retired = self.pipeline.app().retired.as_ref().map(|report| json!({
            "generation": report.generation,
            "before": report.before,
            "resources": report.counts,
            "clean": report.counts == ResourceCounts::default() && report.failure.is_none(),
        }));
        Ok(json!({"oldSurfaceId":format!("{old_surface:?}"),"newSurfaceId":format!("{new_surface:?}"),
            "oldDocumentId":old_document,"newDocumentId":new_document,"oldGeneration":old_generation,
            "generation":self.generation(),"fromTimeUs":time,"timeUs":self.environment.now_us(),
            "retired":retired,"readiness":self.readiness()}))
    }
    pub fn begin_step(&self, action: u64, timeout_ms: Option<u64>) {
        self.step_start_frame.set(self.observers.begin_step(action));
        self.deadline.set(self.run_deadline.min(Instant::now()+Duration::from_millis(timeout_ms.unwrap_or(self.limits.max_step_wall_ms).min(self.limits.max_step_wall_ms))));
    }
    pub fn check(&mut self) -> Result<(), Fault> {
        if let Some(fault) = self.startup_fault.take() { return Err(fault); }
        if let Some(fault) = self.pipeline.app_mut().fault.take() { return Err(fault); }
        if let Some(error) = self.observers.take_fault() { return Err(error.into()); }
        if self.frames().saturating_sub(self.step_start_frame.get()) > u64::from(self.limits.max_frames_per_step) {
            return Err(Error::new("E_JOB_BUDGET",5,"native frame budget exhausted").with_details(self.readiness()).into());
        }
        if Instant::now() >= self.deadline.get() { return Err(Error::new("E_READINESS_TIMEOUT",5,"step exhausted its real wall-clock deadline").with_details(self.readiness()).into()); }
        Ok(())
    }
    fn window(&self, name: &str) -> Result<&Window, Error> {
        let app = self.pipeline.app();
        let session = app.session().ok_or_else(||Error::new("E_HOST_INVARIANT",9,"no mounted session"))?;
        app.native.windows().iter().find(|window| session.mount_region(window.document_id()).is_some_and(|owned| owned.name == name))
            .ok_or_else(||Error::new("E_SURFACE_NOT_FOUND",4,format!("surface {name:?} is not mounted")))
    }
    fn window_mut(&mut self, name: &str)->Result<&mut Window,Error> {
        let id = self.window(name)?.surface().id();
        self.pipeline.app_mut().native.windows_mut().iter_mut().find(|window|window.surface().id()==id)
            .ok_or_else(||Error::new("E_HOST_INVARIANT",9,"selected native window disappeared"))
    }
    fn by_surface(&self, id: SurfaceId) -> Option<&Window> {
        self.pipeline.app().native.windows().iter().find(|window| window.surface().id() == id)
    }
    fn observer(&self, surface: SurfaceId) -> Result<FrameObserver, Error> {
        self.observers.get(surface).ok_or_else(||Error::new("E_HOST_INVARIANT",9,"selected native window has no renderer observer"))
    }
    fn window_current(&self, window: &Window) -> bool {
        self.observers.get(window.surface().id()).is_some_and(|observer| observer.settled(window, self.pipeline.app().session()).is_some())
    }
    fn frame_current(&self, name: &str) -> bool {
        self.window(name).is_ok_and(|window| self.window_current(window))
    }
    fn runnable(&self) -> bool {
        self.pipeline.platform().has_pending_wakes()
            || self.pipeline.app().session().is_some_and(|session| {
                session.has_deliverable_work() || session.has_inflight_host_ops()
            })
            || self.pipeline.app().native.windows().iter().any(|window| {
                self.pipeline.platform().offscreen(window.surface().id()).is_some_and(|surface|surface.has_pending_redraw())
            })
            || self.pipeline.parked_deadline().is_some_and(|next|next<=self.pipeline.now())
            || self.fixtures.next_deadline_us().is_some_and(|next|next<=self.environment.now_us())
    }
    pub fn readiness(&self) -> Value {
        self.readiness_for(None)
    }
    fn readiness_for(&self, name: Option<&str>) -> Value {
        let app = self.pipeline.app();
        let window = if let Some(name) = name { self.window(name).ok() }
            else if self.pure_shell { app.native.windows().first() }
            else { self.window("main").ok() };
        let owned = window.and_then(|window| app.session()?.mount_region(window.document_id()));
        let resources=window.map(|window|resources::snapshot(window,&[],app.session().map(AsRef::as_ref)));
        let resource=resources.as_ref().and_then(|result|result.as_ref().ok());
        let mut readiness = json!({"appReady":self.pipeline.app().session().is_some(),"currentWorkSettled":!self.runnable(),
            "requiredAssetsReady":resource.map(|value|&value["requiredAssetsReady"]),"nativeAssetReadinessAvailable":true,"frameCurrent":window.is_some_and(|window|self.window_current(window)),
            "resourceRevision":window.map(Window::image_resource_revision),
            "requiredFontsReady":resource.map(|value|&value["requiredFontsReady"]),
            "requiredStyleRevision":window.map(Window::style_revision),
            "validatedStyleRevision":window.and_then(|window|window.completed_frame().map(|frame|frame.style_revision)),
            "pendingStylesheets":window.map(Window::has_pending_stylesheets),
            "nativeAcceptance":app.host.acceptance(),
            "pendingAssets":resource.and_then(|value|value["pendingIds"].as_array()).map(|items|items.len()),
            "timeUs":self.environment.now_us(),"nextFixtureDeadlineUs":self.fixtures.next_deadline_us(),
            "nextNativeDeadlineUs":self.pipeline.parked_deadline().map(|next|next.saturating_duration_since(self.pipeline.platform().clock().origin()).as_micros()),
            "requiredMetadataRevision":owned.map(|owned|owned.bridge.metadata_revision()),
            "requiredStateRevision":window.map(|window|window.dom().revision()),"generation":self.generation()});
        if self.lock_capable { readiness["sessionLock"] = self.session_lock_snapshot(); }
        readiness
    }

    fn session_lock_snapshot(&self) -> Value {
        let snapshot = self.pipeline.platform().session_lock_snapshot();
        json!({"state":snapshot.state.as_str(), "unlockCount":snapshot.unlock_count,
            "coveredOutputs":snapshot.covered_outputs, "fallbackOutputs":snapshot.fallback_outputs})
    }
    fn notify_fixture(&self) {
        let surfaces: Box<[_]> = self.pipeline.app().native.windows().iter().map(|window| window.surface().id()).collect();
        let wake = if surfaces.is_empty() { WakeReason::AppWork } else { WakeReason::ReactiveWork{surfaces} };
        self.pipeline.platform().waker().wake(wake);
    }
    fn wait_asset_barrier(&mut self)->Result<(),Fault> {
        self.check()?;
        loop {
            if self.pipeline.app_mut().native.windows_mut().iter_mut().all(Window::poll_image_barrier) { return Ok(()); }
            self.check().map_err(|mut fault| {
                if fault.error.code=="E_READINESS_TIMEOUT" { fault.error=Error::new("E_ASSET_TIMEOUT",5,"native resource publication exceeded its real deadline"); }
                fault
            })?;
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn wait_host_workers(&mut self) -> Result<(), Fault> {
        loop {
            let pending = self.pipeline.app().session().is_some_and(|session| {
                session.has_inflight_host_ops() && !session.has_deliverable_work()
            });
            if !pending || self.pipeline.platform().has_pending_wakes() {
                return Ok(());
            }
            self.check()?;
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn pump_current(&mut self)->Result<(),Fault> {
        self.wait_asset_barrier()?;
        self.wait_host_workers()?;
        if !self.environment.is_virtual() {
            let now = self.pipeline.now();
            self.pipeline.advance(Instant::now().saturating_duration_since(now));
        }
        if self.fixtures.release_due()? { self.notify_fixture(); }
        self.pump_application()
    }

    fn pump_application(&mut self) -> Result<(), Fault> {
        let sync_shells = self.shell_capable || self.pipeline.app().host.has_shell_surfaces();
        if sync_shells {
            // Wakes may open windows; bind their native configuration and evidence
            // before the harness takes its first redraw at the current clock instant.
            self.pipeline.dispatch_wakes();
            self.check()?;
            self.sync_shell_surfaces()?;
        }
        self.pipeline.pump();
        self.check()?;
        if sync_shells || self.pipeline.app().host.has_shell_surfaces() {
            self.sync_shell_surfaces()?;
        }
        Ok(())
    }
    pub fn settle(&mut self, require_frame: bool) -> Result<Value, Fault> {
        self.settle_surface(require_frame.then_some("main"))
    }
    fn settle_surface(&mut self, name: Option<&str>) -> Result<Value, Fault> {
        for _ in 0..self.limits.max_settle_turns {
            self.pump_current()?;
            if self.observers.device_unavailable() {
                return Err(Error::new("E_DEVICE_LOST",7,"native graphics device became unavailable").into());
            }
            if let Some(name) = name { self.window(name)?; }
            if !self.runnable() && name.is_none_or(|name| self.frame_current(name)) { return Ok(self.readiness_for(name)); }
            if !self.runnable() && self.environment.is_virtual() {
                return Err(Error::new("E_CAPTURE_NOT_READY",5,"a future native deadline requires explicit clock.advance")
                    .with_details(self.readiness_for(name)).into());
            }
            if !self.runnable() { std::thread::sleep(Duration::from_millis(1)); }
        }
        Err(Error::new("E_JOB_BUDGET",5,format!("bounded native pump exhausted current-work turns: {}",self.readiness_for(name))).into())
    }
    pub fn capture(&mut self, id: &str, name: &str, required_time:Option<u64>,allowed_pending:&[String]) -> Result<Value, Fault> {
        self.check()?;
        if let Some(time)=required_time {
            if time!=self.environment.now_us() {
                return Err(Error::new("E_CAPTURE_NOT_READY",5,"requiredFrameTimeUs differs from the current virtual clock")
                    .with_details(self.readiness_for(Some(name))).into());
            }
            let window = self.window(name)?;
            if self.observer(window.surface().id())?.last_time_us()!=Some(time) {
                window.request_frame();
            }
        }
        // An explicit placeholder capture may reuse an already coherent frame before publishing
        // asynchronous arrivals. Whether a worker happened to finish cannot change this decision.
        if allowed_pending.is_empty() || !self.frame_current(name) || self.runnable() { self.settle_surface(Some(name))?; }
        let mut resource=resources::snapshot(self.window(name)?,allowed_pending,self.pipeline.app().session().map(AsRef::as_ref))?;
        for _ in 0..self.limits.max_settle_turns {
            if resource["requiredAssetsReady"]==true { break; }
            self.settle_surface(Some(name))?;
            resource=resources::snapshot(self.window(name)?,allowed_pending,self.pipeline.app().session().map(AsRef::as_ref))?;
        }
        if resource["requiredAssetsReady"]!=true {
            return Err(Error::new("E_ASSET_TIMEOUT",5,"visible resources did not reach current native demand").into());
        }
        let readiness=self.readiness_for(Some(name));
        let window=self.window(name)?;
        let observer=self.observer(window.surface().id())?;
        let settled=observer.settled(window,self.pipeline.app().session())
            .ok_or_else(||Error::new("E_CAPTURE_NOT_READY",5,"no owned validated tree").with_details(readiness.clone()))?;
        if required_time.is_some_and(|time|settled.time_us()!=time) {
            return Err(Error::new("E_CAPTURE_NOT_READY",5,"validation frame timestamp differs from requiredFrameTimeUs").with_details(readiness).into());
        }
        let size = window.surface().size();
        let paths = observer.write_capture(&settled,window,CaptureRequest {
            id,
            dimensions: (size.width.0 as u32,size.height.0 as u32),
            readiness,
            resource,
            allowed_pending,
        })?;
        drop(settled);
        self.check()?;
        if Instant::now()>=self.deadline.get() { return Err(Error::new("E_READINESS_TIMEOUT",5,"capture publication exceeded its step deadline").with_details(self.readiness_for(Some(name))).into()); }
        Ok(paths)
    }
    pub fn graphics(&self) -> Value { self.observers.graphics_metadata() }
    pub fn frames(&self) -> u64 { self.observers.frames() }
    pub fn failure_evidence(&self) -> Result<Value, Error> {
        let Some(id) = self.observers.latest_checkpoint() else { return Ok(json!({"available":false})); };
        let Some(window) = self.by_surface(id) else { return Ok(json!({"available":false})); };
        let name = self.pipeline.app().session().and_then(|session|session.mount_region(window.document_id()));
        let readiness = self.readiness_for(name.as_ref().map(|region|region.name.as_str()));
        self.observer(id)?.publish_failure_evidence(Some(window), readiness, self.window_current(window))
    }
    pub fn finish(mut self) -> (Value, Option<Fault>, Option<RuntimeTelemetry>) {
        self.deadline.set(self.run_deadline.min(Instant::now() + self.teardown_budget));
        self.pipeline.shut_down();
        if self.lock_capable { self.fixtures.record_session_lock(self.session_lock_snapshot()); }
        {
            use sha2::{Digest, Sha256};
            use zgui::platform::{ClipboardData, ClipboardKind};
            let clip = self.pipeline.platform().memory_clipboard();
            let snapshot = clip.snapshot(ClipboardKind::Standard);
            let options = clip.last_write_options(ClipboardKind::Standard);
            if snapshot.is_some() || options.is_some() {
                let record = match snapshot {
                    Some(ClipboardData::Text(text)) => json!({
                        "format": "text",
                        "text": text.as_str(),
                        "excludeFromHistory": options.is_some_and(|o| o.exclude_from_history),
                    }),
                    Some(ClipboardData::Image { media_type, bytes }) => {
                        let digest: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
                        json!({
                            "format": "image",
                            "mediaType": media_type.as_str(),
                            "byteLength": bytes.len(),
                            "sha256": digest,
                            "excludeFromHistory": options.is_some_and(|o| o.exclude_from_history),
                        })
                    }
                    _ => json!({
                        "format": "empty",
                        "excludeFromHistory": options.is_some_and(|o| o.exclude_from_history),
                    }),
                };
                self.fixtures.record_clipboard(record);
            }
        }
        self.fixtures.dispose();
        let report = self.pipeline.app().session().map(|session| session.retire());
        let telemetry = RuntimeTelemetry::merge(
            self.pipeline.app().retired_telemetry,
            report.as_ref().and_then(|report| report.telemetry),
        );
        let report = report.as_ref().or(self.pipeline.app().retired.as_ref());
        let counts = report.map(|report| report.counts.clone()).unwrap_or_default();
        let fault = evaluate_retirement(report);
        let clean = counts == ResourceCounts::default() && fault.is_none();
        let teardown = json!({"verified":true,"clean":clean,"resources":counts,"sessionCreated":report.is_some()});
        (teardown,fault,telemetry)
    }
}

pub(super) fn evaluate_retirement(report: Option<&SessionRetirement>) -> Option<Fault> {
    let report = report?;
    match &report.failure {
        Some(RetirementFailure::Contract(contract)) => Some(Fault::from(contract.clone())),
        Some(RetirementFailure::Cleanup(diagnostic)) => {
            let mut fault = Fault::application(SessionError::Diagnostic(diagnostic.clone()));
            fault.error = Error::new("E_TEARDOWN", 9, fault.error.message);
            Some(fault)
        }
        None if report.counts != ResourceCounts::default() => {
            Some(Error::new("E_TEARDOWN", 9, "retired session retained resources").into())
        }
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard, PoisonError};
    use super::*;
    use crate::session::{RetirementFailure, SessionRetirement};

    // Headless `Driver` unit tests spin up real wgpu/Lavapipe pipelines in one process;
    // serialize them to prevent Mesa 24 concurrent-teardown SIGSEGV and 4-vCPU startup starvation.
    static HEADLESS_DRIVER_LOCK: Mutex<()> = Mutex::new(());

    fn lock_headless_driver() -> MutexGuard<'static, ()> {
        HEADLESS_DRIVER_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn run_deadline_for(profile: &LoadedProfile) -> Instant {
        Instant::now() + Duration::from_millis(profile.profile.limits.max_run_wall_ms)
    }

    #[test]
    fn evaluate_retirement_enforces_teardown_and_precedence() {
        assert!(evaluate_retirement(None).is_none());

        let clean = SessionRetirement {
            generation: 1,
            before: ResourceCounts { nodes: 2, ..Default::default() },
            counts: ResourceCounts::default(),
            failure: None,
            telemetry: None,
        };
        assert!(evaluate_retirement(Some(&clean)).is_none());

        let leaked = SessionRetirement {
            generation: 2,
            before: ResourceCounts { timers: 1, ..Default::default() },
            counts: ResourceCounts { shell_surfaces: 1, ..Default::default() },
            failure: None,
            telemetry: None,
        };
        let leaked_fault = evaluate_retirement(Some(&leaked)).expect("leaked resource must fail");
        assert_eq!((leaked_fault.error.code, leaked_fault.error.exit_code), ("E_TEARDOWN", 9));

        let cleanup = SessionRetirement {
            generation: 3,
            before: ResourceCounts::default(),
            counts: ResourceCounts::default(),
            failure: Some(RetirementFailure::Cleanup(js_runtime::Diagnostic {
                kind: js_runtime::DiagnosticKind::Exception,
                phase: "cleanup",
                message: "disposer failed".into(),
                stack: Some("app.mjs:4:9".into()),
                generated_stack: Some("dist/main.js:12:3".into()),
            })),
            telemetry: None,
        };
        let cleanup_fault = evaluate_retirement(Some(&cleanup)).expect("cleanup failure must fail");
        assert_eq!((cleanup_fault.error.code, cleanup_fault.error.exit_code), ("E_TEARDOWN", 9));
        assert_eq!(
            cleanup_fault.diagnostic.as_ref().and_then(|d| d["mappedStack"].as_str()),
            Some("app.mjs:4:9"),
        );

        let contract = SessionRetirement {
            generation: 4,
            before: ResourceCounts::default(),
            counts: ResourceCounts { timers: 1, ..Default::default() },
            failure: Some(RetirementFailure::Contract(ContractFailure {
                code: "E_FIXTURE_MISSING_SERVICE",
                message: "missing".into(),
            })),
            telemetry: None,
        };
        let contract_fault = evaluate_retirement(Some(&contract)).expect("contract must preserve type");
        assert_eq!((contract_fault.error.code, contract_fault.error.exit_code), ("E_FIXTURE_MISSING_SERVICE", 6));
    }

    #[test]
    fn driver_memory_sink_preserves_historical_evidence_enforces_teardown_and_isolates_sequential_runs() {
        use super::pipeline::MemorySink;
        use crate::test_package::Fixture;

        let _driver_guard = lock_headless_driver();
        let profile_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../preview/profiles/functional.json");
        let profile = profile::load(&profile_path, &profile::Overrides::default()).unwrap();
        let fixture_def: FixtureDefinition = serde_json::from_value(serde_json::json!({
            "schemaVersion": 1, "id": "driver-test", "services": {}, "events": []
        })).unwrap();

        // 1. Checkpoint E_OUTPUT_LIMIT on second frame retains frame 1 Historical evidence.
        let updating = Fixture::new(r#"
            export const hostApiVersion = 1;
            export function mount(root) {
                const label = __zgui.createTextNode('frame-one');
                __zgui.insertNode(root, label, null);
                const t = setTimeout(() => __zgui.replaceText(label, 'frame-two'), 50);
                return () => { clearTimeout(t); __zgui.clearRoot(root); };
            }
        "#, "");
        let sink1 = MemorySink::new();
        sink1.fail_checkpoint_after(Some(1));
        let deadline = run_deadline_for(&profile);
        let mut driver1 = Driver::with_sink(
            &profile,
            updating.package(),
            fixture_def.clone(),
            sink1.boxed(),
            deadline,
            stylesheet(&profile),
        ).unwrap();
        driver1.begin_step(0, None);
        driver1.settle(true).unwrap();
        assert_eq!(sink1.checkpoint_count(), 1);
        driver1.begin_step(1, None);
        let err = driver1.advance(60).unwrap_err();
        assert_eq!((err.error.code, err.error.exit_code), ("E_OUTPUT_LIMIT", 8));
        let historical = driver1.failure_evidence().unwrap();
        assert_eq!(historical["available"], true);
        assert!(sink1.last_historical().is_some_and(|(tree, _, _)| {
            tree.nodes.iter().any(|n| n.text == "frame-one")
        }));
        let (teardown1, fault1, _) = driver1.finish();
        assert_eq!(teardown1["verified"], true);
        assert!(fault1.is_none());

        // 2. Second Driver on same Rust thread proves RAII Pipeline cleanup + E_TEARDOWN exit 9.
        let bad_cleanup = Fixture::new(r#"
            export const hostApiVersion = 1;
            export function mount(root) {
                __zgui.insertNode(root, __zgui.createTextNode('ok'), null);
                return () => { throw Error('disposer boom'); };
            }
        "#, "");
        let sink2 = MemorySink::new();
        let mut driver2 = Driver::with_sink(
            &profile,
            bad_cleanup.package(),
            fixture_def,
            sink2.boxed(),
            run_deadline_for(&profile),
            stylesheet(&profile),
        ).unwrap();
        driver2.begin_step(0, None);
        driver2.settle(true).unwrap();
        let (teardown2, fault2, telemetry2) = driver2.finish();
        assert_eq!(teardown2["verified"], true);
        assert_eq!(teardown2["clean"], false);
        assert_eq!(teardown2["sessionCreated"], true);
        assert!(telemetry2.is_some());
        let fault2 = fault2.expect("throwing disposer must fail Driver::finish");
        assert_eq!((fault2.error.code, fault2.error.exit_code), ("E_TEARDOWN", 9));
        assert!(fault2.diagnostic.as_ref().and_then(|d| d["mappedStack"].as_str())
            .is_some_and(|s| s.contains("app.mjs")));
    }

    #[test]
    fn runaway_disposer_is_interrupted_during_preview_teardown() {
        use super::pipeline::MemorySink;
        use crate::test_package::Fixture;

        let _driver_guard = lock_headless_driver();
        let profile_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../preview/profiles/functional.json");
        let profile = profile::load(&profile_path, &profile::Overrides::default()).unwrap();
        let fixture: FixtureDefinition = serde_json::from_value(json!({
            "schemaVersion": 1, "id": "runaway-disposer", "services": {}, "events": []
        })).unwrap();
        let app = Fixture::new(r#"
            export const hostApiVersion = 1;
            export function mount() {
                return () => { while (true) {} };
            }
        "#, "");
        let mut driver = Driver::with_sink(
            &profile,
            app.package(),
            fixture,
            MemorySink::new().boxed(),
            run_deadline_for(&profile),
            stylesheet(&profile),
        ).unwrap();
        driver.check().unwrap();

        let start = Instant::now();
        let (teardown, fault, _) = driver.finish();
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_secs(2), "teardown took {elapsed:?}");
        assert_eq!(teardown["clean"], false);
        let fault = fault.expect("runaway disposer must fail teardown");
        assert_eq!((fault.error.code, fault.error.exit_code), ("E_TEARDOWN", 9));
        let diagnostic = fault.diagnostic.expect("interrupted disposer must have a diagnostic");
        assert_eq!(diagnostic["phase"], "cleanup");
        assert_eq!(diagnostic["kind"], "Interrupted");
        assert!(diagnostic["generatedStack"].as_str().is_some_and(|s| s.contains("app.mjs")));
    }

    #[test]
    fn recovers_latest_main_checkpoint_after_undeclared_named_window() {
        use crate::test_package::Fixture;
        use super::super::evidence;

        let _driver_guard = lock_headless_driver();
        let profile_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../preview/profiles/functional.json");
        let profile = profile::load(&profile_path, &profile::Overrides::default()).unwrap();
        let fixture: FixtureDefinition = serde_json::from_value(json!({
            "schemaVersion": 1, "id": "checkpoint-recovery", "services": {}, "events": []
        })).unwrap();
        let app = Fixture::new(r#"
            export const hostApiVersion = 1;
            export function mount(root) {
                const label = __zgui.createTextNode('initial main');
                __zgui.insertNode(root, label, null);
                let windowId;
                setTimeout(() => {
                    windowId = __zguiShell.openWindow('aux', {
                        title: 'Aux', width: 320, height: 240, transparent: false,
                    }, (auxRoot, bridge) => {
                        bridge.insertNode(auxRoot, bridge.createTextNode('auxiliary'), null);
                        return () => bridge.clearRoot(auxRoot);
                    }, () => {});
                }, 50);
                setTimeout(() => {
                    __zguiShell.closeWindow(windowId);
                    __zgui.replaceText(label, 'updated main');
                }, 150);
                return () => __zgui.clearRoot(root);
            }
        "#, "");
        let package = app.package();
        let root = package.directory.join("preview-output");
        let output = Rc::new(RefCell::new(Output::create(&root, profile.profile.limits.max_output_bytes).unwrap()));
        let mut driver = Driver::new(
            &profile, package, fixture, output.clone(),
            run_deadline_for(&profile), stylesheet(&profile), ReloadConfig::default(),
        ).unwrap();
        driver.begin_step(0, None);
        driver.settle(true).unwrap();
        driver.begin_step(1, None);
        driver.advance(100).unwrap();
        driver.begin_step(2, None);
        driver.advance(100).unwrap();
        drop(driver);

        let recovered = evidence::recover(&mut output.borrow_mut()).unwrap();
        assert_eq!(recovered["available"], true);
        let read = |path: &std::path::Path| -> Value {
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
        };
        let named_frame_path = std::fs::read_dir(root.join("evidence")).unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.file_name().and_then(|name| name.to_str())
                .is_some_and(|name| name.contains("-checkpoint-") && name.ends_with(".frame.json")))
            .max().expect("auxiliary window checkpoint");
        let named_frame = read(&named_frame_path);
        let named_tree = read(&named_frame_path.with_file_name(
            named_frame_path.file_name().unwrap().to_str().unwrap().replace(".frame.json", ".tree.json"),
        ));
        assert_eq!(named_frame["surface"], "aux");
        let named_sequence = named_frame["checkpointSequence"].as_u64().expect("auxiliary checkpoint sequence");
        assert_eq!(named_tree["checkpointSequence"], named_sequence);
        let frame = read(&root.join(recovered["frame"].as_str().unwrap()));
        let tree = read(&root.join(recovered["tree"].as_str().unwrap()));
        assert_eq!(frame["surface"], "main");
        let main_sequence = frame["checkpointSequence"].as_u64().expect("main checkpoint sequence");
        assert!(main_sequence > named_sequence);
        assert_eq!(tree["checkpointSequence"], main_sequence);
        assert!(tree["nodes"].as_array().unwrap().iter().any(|node| node["text"] == "updated main"));
    }
}
