use std::{cell::{Cell, RefCell}, rc::{Rc, Weak}, time::{Duration, Instant}};

use serde_json::{Value, json};
use zgui::{geom::{DevicePx, Size}, platform::{AppHandler, ColorScheme, IdlePolicy, PlatformCx, SurfaceEvent, SurfaceId, WakeReason}, render::{FrameOutcome, SkipReason}, runtime::{FrameProbe, Runtime, Window}};
use zgui_platform_headless::{Harness, Headless};
use zgui::platform::Surface as _;

use crate::{package::Package, session::{ResourceCounts, Session, SessionError, SessionModule, SessionPolicy}};
use crate::session_host::{ImageChoice, RecoveryPolicy, Root, RuntimeSurface, SessionHost};
use super::{Error, capture::{self, CaptureHandle}, evidence::{self, CaptureIdentity, CapturePaths, FrameEvidence, FrameStamp, HistoricalStatus, Presentation, TreeEvidence}, fixture::FixtureDefinition, inspect, output::Output, profile::{self, Limits, LoadedProfile, Surface}, resources};
use crate::{assets::{ContractFailure, fixture::FixtureRuntime}, environment::Environment};
use super::semantics::SemanticModel;

mod actions;
mod timeline;

pub type SharedOutput = Rc<RefCell<Output>>;

#[derive(Clone)]
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
        }
    }
}

struct OwnedFrame {
    generation: u64,
    revision: u64,
    configuration: u64,
    metadata_revision: u64,
    resource_revision: u64,
    surface_configuration: u64,
    native_nodes: Vec<ui_bridge::registry::SnapshotNode>,
    presentation: u64,
    frame: FrameEvidence,
    tree: TreeEvidence,
}

struct Probe {
    capture: CaptureHandle,
    session: RefCell<Weak<Session>>,
    output: SharedOutput,
    build_id: String,
    action: Cell<u64>,
    frames: Cell<u64>,
    surface_configuration: Cell<u64>,
    last: RefCell<Option<OwnedFrame>>,
    presentation: RefCell<Option<Presentation>>,
    fault: RefCell<Option<Error>>,
    surface: RefCell<Option<std::sync::Arc<zgui_platform_headless::OffscreenSurface>>>,
    semantics: RefCell<SemanticModel>,
    environment: Rc<Environment>,
}
impl FrameProbe for Probe {
    fn frame_ended(&self, window: &Window) {
        let Some(observation) = window.completed_frame() else { return; };
        self.frames.set(self.frames.get()+1);
        if self.fault.borrow().is_some() { return; }
        let Some(session) = self.session.borrow().upgrade() else { return; };
        if session.region.document != window.document_id()
            || self.surface.borrow().as_ref().is_none_or(|surface|surface.id()!=window.surface().id()) { return; }
        if let Some(surface) = self.surface.borrow().as_ref()
            && let Err(error) = self.semantics.borrow_mut().consume(surface, window) {
            *self.fault.borrow_mut() = Some(error);
            return;
        }
        let resources=match resources::snapshot(window,&[]) {
            Ok(resources)=>resources,
            Err(error)=>{ *self.fault.borrow_mut()=Some(error); return; }
        };
        let state = self.capture.state();
        let stamp = FrameStamp { ordinal:observation.ordinal,
            time_us:u64::try_from(observation.timestamp.since_origin().as_micros()).unwrap_or(u64::MAX),
            draw_id:state.draw_id, outcome:format!("{:?}",observation.outcome) };
        if matches!(observation.outcome, FrameOutcome::Presented(_)) {
            *self.presentation.borrow_mut() = Some(Presentation { frame:stamp.clone(), sequence:state.presentation_id,
                configuration_id:state.configuration_id, draw_id:state.presented_draw });
        }
        if !observation.layout_document_coherent || !matches!(observation.outcome,
            FrameOutcome::Presented(_) | FrameOutcome::Skipped(SkipReason::Undamaged)) { return; }
        let Some(presentation) = self.presentation.borrow().clone() else { return; };
        if presentation.configuration_id != state.configuration_id { return; }
        let identity = CaptureIdentity { build_id:self.build_id.clone(), presentation_sequence:state.presentation_id,
            resource_revision:Some(window.image_resource_revision()), surface_configuration:Some(self.surface_configuration.get()) };
        let snapshot = inspect::snapshot(window, &session, observation.ordinal, &self.semantics.borrow(), &identity);
        let tree = snapshot.tree;
        let mut extra = serde_json::Map::new();
        for (key, value) in [
            ("documentIdentity", json!(format!("{:?}",window.document_id()))),
            ("actionSequence", json!(self.action.get())),
            ("requiredStateRevision", json!(observation.document_revision)),
            ("validatedStateRevision", json!(observation.document_revision)),
            ("layoutRevision", json!(observation.layout_revision)),
            ("configurationId", json!(state.configuration_id)),
            ("timingDeterministic", json!(self.environment.is_virtual())),
            ("resources", resources),
            ("scale", json!(window.surface().scale_factor())),
            ("renderer", self.capture.metadata()),
        ] { extra.insert(key.to_owned(), value); }
        let frame = FrameEvidence { schema_version:1, build_id:self.build_id.clone(), generation:session.generation,
            surface:"main".into(), metadata_revision:snapshot.metadata_revision,
            resource_revision:Some(window.image_resource_revision()), surface_configuration:Some(self.surface_configuration.get()),
            validation_frame:stamp, pixel_presentation:presentation, layout_document_coherent:true,
            capture_id:None, tree_digest:None, png_digest:None, extra };
        // Persist complete owned evidence before another callback can fail or block.
        // No readback, redraw request, signal write, or application entry occurs in this probe.
        let (tree_path, frame_path) = evidence::checkpoint_paths(session.generation, observation.ordinal);
        let tree_value = serde_json::to_value(&tree);
        let frame_value = serde_json::to_value(&frame);
        let publication = (|| {
            let mut output = self.output.borrow_mut();
            let tree_value = tree_value.map_err(|error| Error::new("E_OUTPUT_IO",8,error.to_string()))?;
            output.json(&tree_path, "validation-tree", &tree_value)?;
            output.json(&frame_path, "validation-frame", &frame_value.map_err(|error| Error::new("E_OUTPUT_IO",8,error.to_string()))?)
        })();
        if let Err(error) = publication { *self.fault.borrow_mut() = Some(error); return; }
        *self.last.borrow_mut() = Some(OwnedFrame { generation:session.generation, revision: observation.document_revision,
            configuration: state.configuration_id, presentation: state.presentation_id, frame, tree,
            metadata_revision:snapshot.metadata_revision,resource_revision:window.image_resource_revision(),
            surface_configuration:self.surface_configuration.get(),native_nodes:snapshot.native_nodes });
    }
}

struct PreviewHost {
    native: Runtime,
    host: SessionHost,
    deadline: Rc<Cell<Instant>>,
    probe: Rc<Probe>,
    font_css: String,
    fault: Option<Fault>,
    requested_metadata: u64,
    retired: Option<Value>,
}
impl PreviewHost {
    fn session(&self) -> Option<&Rc<Session>> { self.host.sessions().next() }
    fn fail(&mut self, cx: &dyn PlatformCx, fault: Fault) {
        if self.fault.is_none() { self.fault = Some(fault); }
        // The handler guards stop queued entries. The driver publishes the cause
        // before bounded disposal, so a blocked cleanup cannot erase that cause.
        cx.request_exit();
    }
    fn after_native(&mut self, cx: &dyn PlatformCx) {
        if self.fault.is_some() { return; }
        if let Some(error) = self.native.failure().take() { self.fail(cx, graphics_error(error).into()); return; }
        if self.host.pending_regions() != 0 {
            if self.host.sessions().next().is_some() || self.native.windows().len() != 1 {
                self.fail(cx, Error::new("E_HOST_INVARIANT", 9, "preview requires exactly one current native document").into()); return;
            }
            let retry: Rc<dyn Fn(zgui::view::DocumentId, u64)> = Rc::new(|_, _| {});
            let attached = {
                let mut surface = RuntimeSurface::new(&mut self.native, ImageChoice::PackageOnly);
                self.host.attach(&mut surface, cx.waker(), retry)
            };
            // A propagated failure keeps its session owned so finish() can
            // inspect it; the probe binds to whichever generation is current.
            *self.probe.session.borrow_mut() = self.host.sessions().next().map(Rc::downgrade).unwrap_or_default();
            if let Err(error) = attached { self.fail(cx, Fault::application(error)); return; }
            if let Some(session) = self.host.sessions().next() {
                session.region.host.install_stylesheet("preview-profile-fonts", &self.font_css);
            }
        }
        let retry: Rc<dyn Fn(zgui::view::DocumentId, u64)> = Rc::new(|_, _| {});
        let checked = {
            let mut surface = RuntimeSurface::new(&mut self.native, ImageChoice::PackageOnly);
            self.host.checkpoint(&mut surface, retry)
        };
        if let Err(error) = checked {
            self.fail(cx, Fault::application(error));
            return;
        }
        let state = self.session().map(|session| (session.bridge.metadata_revision(), session.region.document));
        if let Some((revision, document)) = state && revision != self.requested_metadata {
            self.requested_metadata = revision;
            if let Some(window) = self.native.windows().iter().find(|window|window.document_id()==document) {
                window.request_frame();
            }
        }
    }
}
impl AppHandler for PreviewHost {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) { self.native.surfaces_available(cx); self.after_native(cx); }
    fn surfaces_lost(&mut self, cx: &dyn PlatformCx) {
        let deadline = self.deadline.get();
        self.deadline.set(deadline.min(Instant::now()+Duration::from_millis(20)));
        let documents: Vec<zgui::view::DocumentId> = self.host.sessions().map(|session| session.region.document).collect();
        for document in documents {
            let before = self.host.session(document).map(|session| session.counts());
            let Some(session) = self.host.retire(document) else { continue; };
            let counts = session.counts();
            self.retired = Some(json!({"generation":session.generation,"before":before,"resources":counts,
                "clean":counts==ResourceCounts::default()}));
            if let Some(error) = session.take_cleanup_failure() {
                self.fail(cx, Error::new("E_TEARDOWN",9,error.to_string()).into());
            } else if let Some(error) = session.contract_failure() {
                self.fail(cx, Fault::application(error.into()));
            } else if counts!=ResourceCounts::default() {
                self.fail(cx, Error::new("E_TEARDOWN",9,"retired surface session retained resources").into());
            }
            drop(session);
        }
        self.host.stop();
        self.deadline.set(deadline);
        self.requested_metadata = 0;
        *self.probe.session.borrow_mut() = Weak::new();
        self.probe.surface.borrow_mut().take();
        *self.probe.semantics.borrow_mut() = SemanticModel::default();
        self.probe.last.borrow_mut().take();
        self.probe.presentation.borrow_mut().take();
        self.probe.capture.retire();
        self.native.surfaces_lost(cx);
    }
    fn surface_event(&mut self, cx: &dyn PlatformCx, surface: SurfaceId, event: SurfaceEvent) { if self.fault.is_none() { self.native.surface_event(cx, surface, event); self.after_native(cx); } }
    fn wake(&mut self, cx: &dyn PlatformCx, reason: WakeReason) { if self.fault.is_none() { self.native.wake(cx, reason); self.after_native(cx); } }
    fn idle(&mut self, cx: &dyn PlatformCx) -> IdlePolicy { if self.fault.is_some() { IdlePolicy::Block } else { self.native.idle(cx) } }
    fn deadline_reached(&mut self, cx: &dyn PlatformCx) { if self.fault.is_none() { self.native.deadline_reached(cx); self.after_native(cx); } }
    fn shutting_down(&mut self, cx: &dyn PlatformCx) { for session in self.host.sessions() { session.shutdown(); } self.native.shutting_down(cx); }
}

pub struct Driver {
    harness: Harness<PreviewHost>,
    probe: Rc<Probe>,
    capture: CaptureHandle,
    deadline: Rc<Cell<Instant>>,
    run_deadline: Instant,
    step_start_frame: Cell<u64>,
    limits: Limits,
    opaque: bool,
    dimensions: (u32,u32),
    surface: Surface,
    environment: Rc<Environment>,
    pub(super) fixtures: Rc<FixtureRuntime>,
    maximum_time_us: u64,
    /// A mount fault observed while the harness already owned the session; the
    /// first `check` reports it so the run can persist the cause before finish.
    startup_fault: Option<Fault>,
}
pub(super) fn stylesheet(profile:&LoadedProfile)->String {
    format!("{} :root {{ display:block; box-sizing:border-box; width:100%; height:100%; background-color:{}; {} }}",
        crate::APPLICATION_STYLESHEET,profile.profile.surface.background,
        if profile.profile.theme=="light" { "color:#171b24;" } else { "" })
}

impl Driver {
    pub fn new(profile: &LoadedProfile, package: Rc<Package>, fixture:FixtureDefinition, output: SharedOutput, run_deadline: Instant, stylesheet:String) -> Result<Self, Fault> {
        let platform=Headless::new();
        let (epoch,start)=match profile.profile.clock {
            profile::Clock::Virtual{epoch_ms,start_us}=>(Some(epoch_ms),start_us),
            profile::Clock::Wall{}=>(None,0),
        };
        platform.virtual_clock().advance(Duration::from_micros(start));
        let environment=Environment::new(platform.clock(),epoch,profile.profile.random.seed);
        let fixtures=FixtureRuntime::new(fixture,environment.clone());
        let capture = CaptureHandle::default();
        let probe = Rc::new(Probe { capture:capture.clone(),session:RefCell::new(Weak::new()),output,
            build_id:package.manifest.build_id.clone(),action:Cell::new(0),frames:Cell::new(0),
            surface_configuration:Cell::new(0),
            last:RefCell::new(None),presentation:RefCell::new(None),fault:RefCell::new(None),
            surface:RefCell::new(None),semantics:RefCell::new(SemanticModel::default()),environment:environment.clone() });
        let pending = Rc::new(RefCell::new(Vec::new()));
        let fonts = profile.fonts.clone();
        let metrics = profile.fonts.clone();
        let raster = profile.fonts.clone();
        let surface = &profile.profile.surface;
        let mut app = zgui::runtime::App::new().with_title("zgui TSX preview")
            .with_size(surface.width as f32, surface.height as f32)
            .with_stylesheet(stylesheet)
            .with_probe(probe.clone()).with_renderer(capture::renderer_factory(capture.clone()))
            .with_metrics(Box::new(move ||metrics.metrics()))
            .with_text_engine(Box::new(move ||Box::new(zgui_layout::Paragraphs::new(fonts.shaper()))))
            .with_glyph_raster(Box::new(move ||raster.raster()));
        app.attributes_mut().transparent = !surface.opaque;
        app=resources::configure(app,&package);
        let mount_regions = pending.clone();
        let native = app.into_handler(move |cx| Box::new(zgui::view::View::build(Root(mount_regions.clone()), cx))).map_err(graphics_error)?;
        let deadline = Rc::new(Cell::new(run_deadline.min(Instant::now()+Duration::from_millis(profile.profile.limits.max_step_wall_ms))));
        let limits = js_runtime::RuntimeLimits {
            entry_timeout:Duration::from_millis(profile.profile.limits.max_js_entry_wall_ms),
            bootstrap_timeout:Duration::from_millis(profile.profile.limits.max_step_wall_ms),
            max_jobs_per_pump:profile.profile.limits.max_jobs_per_pump as usize,
            job_timeout:Duration::from_millis(profile.profile.limits.max_pump_wall_ms),
            ..Default::default()
        };
        let session_host = SessionHost::new(pending.clone(), SessionModule::package(package.clone()),
            SessionPolicy::preview(limits, deadline.clone(), environment.clone(), fixtures.clone()),
            RecoveryPolicy::Propagate);
        let host = PreviewHost { native,host:session_host,
            deadline:deadline.clone(),probe:probe.clone(),
            font_css:format!(":root {{ font-family:{}; }}",profile.font_family_css),fault:None,requested_metadata:0,
            retired:None };
        platform.set_color_scheme(Some(if profile.profile.theme == "dark" { ColorScheme::Dark } else { ColorScheme::Light }));
        let mut harness = Harness::over(platform, host);
        harness.hold_clock(true);
        let mut driver = Self { harness,probe,capture,deadline,run_deadline,step_start_frame:Cell::new(0),limits:profile.profile.limits.clone(),
            opaque:surface.opaque,dimensions:(profile.physical_width,profile.physical_height),surface:surface.clone(),
            environment,fixtures,maximum_time_us:start+profile.profile.limits.max_virtual_duration_ms*1000,startup_fault:None };
        if let Err(fault) = driver.bind_surface() { driver.startup_fault = Some(fault); }
        Ok(driver)
    }
    fn bind_surface(&mut self) -> Result<(), Fault> {
        self.check()?;
        let surface = self.window()?.surface().id();
        let offscreen = self.harness.platform().offscreen(surface)
            .ok_or_else(||Error::new("E_HOST_INVARIANT",9,"current native main has no headless surface"))?;
        offscreen.set_refresh_rate_millihertz(Some(self.surface.refresh_rate_milli_hz));
        *self.probe.surface.borrow_mut() = Some(offscreen);
        self.harness.deliver(surface, SurfaceEvent::ScaleFactorChanged { scale_factor:self.surface.scale,
            size:Size::new(DevicePx(self.dimensions.0 as f32),DevicePx(self.dimensions.1 as f32)) });
        self.harness.deliver(surface, SurfaceEvent::Focused(true));
        self.window()?.request_frame();
        self.check()
    }
    pub fn generation(&self) -> u64 { self.harness.app().host.generation() }
    /// Recreate the real native document while retaining run time, RNG and fixture history.
    pub fn recreate(&mut self) -> Result<Value, Fault> {
        self.check()?;
        let old_surface = self.window()?.surface().id();
        let old_document = format!("{:?}",self.window()?.document_id());
        let old_generation = self.generation();
        let time = self.environment.now_us();
        self.harness.suspend();
        self.check()?;
        if !self.harness.app().native.windows().is_empty() {
            return Err(Error::new("E_HOST_INVARIANT",9,"suspended runtime retained native windows").into());
        }
        // The native harness may retain old offscreen entries. Only Runtime.windows owns main.
        self.harness.resume();
        self.check()?;
        let new_surface = self.window()?.surface().id();
        let new_document = format!("{:?}",self.window()?.document_id());
        if new_surface==old_surface || new_document==old_document || self.generation()!=old_generation+1 {
            return Err(Error::new("E_HOST_INVARIANT",9,"surface resume did not create a fresh native generation").into());
        }
        self.probe.surface_configuration.set(self.probe.surface_configuration.get()+1);
        self.bind_surface()?;
        self.settle(true)?;
        Ok(json!({"oldSurfaceId":format!("{old_surface:?}"),"newSurfaceId":format!("{new_surface:?}"),
            "oldDocumentId":old_document,"newDocumentId":new_document,"oldGeneration":old_generation,
            "generation":self.generation(),"fromTimeUs":time,"timeUs":self.environment.now_us(),
            "retired":self.harness.app().retired,"readiness":self.readiness()}))
    }
    pub fn begin_step(&self, action: u64, timeout_ms: Option<u64>) {
        self.probe.action.set(action);
        self.step_start_frame.set(self.probe.frames.get());
        self.deadline.set(self.run_deadline.min(Instant::now()+Duration::from_millis(timeout_ms.unwrap_or(self.limits.max_step_wall_ms).min(self.limits.max_step_wall_ms))));
    }
    pub fn check(&mut self) -> Result<(), Fault> {
        if let Some(fault) = self.startup_fault.take() { return Err(fault); }
        if let Some(fault) = self.harness.app_mut().fault.take() { return Err(fault); }
        if let Some(error) = self.probe.fault.borrow_mut().take() { return Err(error.into()); }
        if self.probe.frames.get()-self.step_start_frame.get()>u64::from(self.limits.max_frames_per_step) {
            return Err(Error::new("E_JOB_BUDGET",5,"native frame budget exhausted").into());
        }
        if Instant::now() >= self.deadline.get() { return Err(Error::new("E_READINESS_TIMEOUT",5,"step exhausted its real wall-clock deadline").into()); }
        Ok(())
    }
    fn window(&self) -> Result<&Window, Error> {
        let session = self.harness.app().session().ok_or_else(||Error::new("E_HOST_INVARIANT",9,"no mounted session"))?;
        self.harness.app().native.windows().iter().find(|window|window.document_id()==session.region.document)
            .ok_or_else(||Error::new("E_HOST_INVARIANT",9,"logical main surface has no current native window"))
    }
    fn window_mut(&mut self)->Result<&mut Window,Error> {
        let document=self.harness.app().session().ok_or_else(||Error::new("E_HOST_INVARIANT",9,"no mounted session"))?.region.document;
        self.harness.app_mut().native.windows_mut().iter_mut().find(|window|window.document_id()==document)
            .ok_or_else(||Error::new("E_HOST_INVARIANT",9,"logical main surface has no current native window"))
    }
    fn frame_current(&self) -> bool {
        let Ok(window) = self.window() else { return false; };
        let state = self.capture.state();
        let last = self.probe.last.borrow();
        last.as_ref().is_some_and(|last| last.revision==window.dom().revision() && last.configuration==state.configuration_id
            && last.presentation==state.presentation_id && state.presented_draw.is_some()
            && last.generation==self.generation()
            && self.harness.app().session().is_some_and(|session|session.bridge.metadata_revision()==last.metadata_revision)
            && last.resource_revision==window.image_resource_revision()
            && last.surface_configuration==self.probe.surface_configuration.get()
            && window.completed_frame().is_some_and(|frame| frame.layout_document_coherent
                && frame.ordinal==last.frame.validation_frame.ordinal
                && matches!(frame.outcome,FrameOutcome::Presented(_)|FrameOutcome::Skipped(SkipReason::Undamaged))))
    }
    fn runnable(&self) -> bool {
        self.harness.platform().has_pending_wakes()
            || self.window().ok().and_then(|window|self.harness.platform().offscreen(window.surface().id())).is_some_and(|surface|surface.has_pending_redraw())
            || self.harness.parked_deadline().is_some_and(|next|next<=self.harness.now())
            || self.fixtures.next_deadline_us().is_some_and(|next|next<=self.environment.now_us())
    }
    pub fn readiness(&self) -> Value {
        let resources=self.window().ok().map(|window|resources::snapshot(window,&[]));
        let resource=resources.as_ref().and_then(|result|result.as_ref().ok());
        json!({"appReady":self.harness.app().session().is_some(),"currentWorkSettled":!self.runnable(),
            "requiredAssetsReady":resource.map(|value|&value["requiredAssetsReady"]),"nativeAssetReadinessAvailable":true,"frameCurrent":self.frame_current(),
            "resourceRevision":self.window().ok().map(Window::image_resource_revision),
            "pendingAssets":resource.and_then(|value|value["pendingIds"].as_array()).map(|items|items.len()),
            "timeUs":self.environment.now_us(),"nextFixtureDeadlineUs":self.fixtures.next_deadline_us(),
            "nextNativeDeadlineUs":self.harness.parked_deadline().map(|next|next.saturating_duration_since(self.harness.platform().clock().origin()).as_micros()),
            "requiredMetadataRevision":self.harness.app().session().map(|session|session.bridge.metadata_revision()),
            "requiredStateRevision":self.window().ok().map(|window|window.dom().revision()),"generation":self.generation()})
    }
    fn notify_fixture(&self)->Result<(),Error> {
        let surface=self.window()?.surface().id();
        self.harness.platform().waker().wake(WakeReason::ReactiveWork{surfaces:vec![surface].into_boxed_slice()});
        Ok(())
    }
    fn wait_asset_barrier(&mut self)->Result<(),Fault> {
        self.check()?;
        loop {
            if self.window_mut()?.poll_image_barrier() { return Ok(()); }
            self.check().map_err(|mut fault| {
                if fault.error.code=="E_READINESS_TIMEOUT" { fault.error=Error::new("E_ASSET_TIMEOUT",5,"native resource publication exceeded its real deadline"); }
                fault
            })?;
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn pump_current(&mut self)->Result<(),Fault> {
        self.wait_asset_barrier()?;
        if !self.environment.is_virtual() {
            self.harness.advance(Instant::now().saturating_duration_since(self.harness.now()));
        }
        if self.fixtures.release_due()? { self.notify_fixture()?; }
        self.harness.pump();
        self.check()
    }
    pub fn settle(&mut self, require_frame: bool) -> Result<Value, Fault> {
        for _ in 0..self.limits.max_settle_turns {
            self.pump_current()?;
            if matches!(self.capture.last_outcome(),Some(FrameOutcome::Skipped(SkipReason::DeviceUnavailable))) {
                return Err(Error::new("E_DEVICE_LOST",7,"native graphics device became unavailable").into());
            }
            if !self.runnable() && (!require_frame || self.frame_current()) { return Ok(self.readiness()); }
            if !self.runnable() && self.environment.is_virtual() {
                return Err(Error::new("E_CAPTURE_NOT_READY",5,"a future native deadline requires explicit clock.advance")
                    .with_details(self.readiness()).into());
            }
            if !self.runnable() { std::thread::sleep(Duration::from_millis(1)); }
        }
        Err(Error::new("E_JOB_BUDGET",5,format!("bounded native pump exhausted current-work turns: {}",self.readiness())).into())
    }
    pub fn capture(&mut self, id: &str,required_time:Option<u64>,allowed_pending:&[String]) -> Result<Value, Fault> {
        if let Some(time)=required_time {
            if time!=self.environment.now_us() {
                return Err(Error::new("E_CAPTURE_NOT_READY",5,"requiredFrameTimeUs differs from the current virtual clock")
                    .with_details(self.readiness()).into());
            }
            if self.probe.last.borrow().as_ref().map(|last|last.frame.validation_frame.time_us)!=Some(time) {
                self.window()?.request_frame();
            }
        }
        // An explicit placeholder capture may reuse an already coherent frame before publishing
        // asynchronous arrivals. Whether a worker happened to finish cannot change this decision.
        if allowed_pending.is_empty() || !self.frame_current() || self.runnable() { self.settle(true)?; }
        let mut resource=resources::snapshot(self.window()?,allowed_pending)?;
        for _ in 0..self.limits.max_settle_turns {
            if resource["requiredAssetsReady"]==true { break; }
            self.settle(true)?;
            resource=resources::snapshot(self.window()?,allowed_pending)?;
        }
        if resource["requiredAssetsReady"]!=true {
            return Err(Error::new("E_ASSET_TIMEOUT",5,"visible resources did not reach current native demand").into());
        }
        let readiness=self.readiness();
        if required_time.is_some_and(|time|self.probe.last.borrow().as_ref().map(|last|last.frame.validation_frame.time_us)!=Some(time)) {
            return Err(Error::new("E_CAPTURE_NOT_READY",5,"validation frame timestamp differs from requiredFrameTimeUs").with_details(readiness).into());
        }
        let pixels = self.capture.read_current(self.window()?.renderer())?;
        self.check()?;
        if pixels.pixels.size().width != self.dimensions.0 as i32 || pixels.pixels.size().height != self.dimensions.1 as i32 {
            return Err(Error::new("E_READBACK",7,"presented extent differs from resolved profile").into());
        }
        let png = capture::png(&pixels.pixels,self.opaque)?;
        self.check()?;
        let last = self.probe.last.borrow();
        let last = last.as_ref().ok_or_else(||Error::new("E_CAPTURE_NOT_READY",5,"no owned validated tree"))?;
        if last.presentation!=pixels.presentation_id || last.configuration!=pixels.configuration_id {
            return Err(Error::new("E_CAPTURE_NOT_READY",5,"pixel presentation and owned tree have different identities").into());
        }
        let paths = CapturePaths::new(id).map_err(Fault::from)?;
        let mut frame = last.frame.clone();
        frame.capture_id = Some(paths.id().to_owned());
        frame.extra.insert("readiness".into(),readiness);
        frame.extra.insert("resources".into(),resource);
        frame.extra.insert("allowedPendingAssets".into(),json!(allowed_pending));
        frame.extra.insert("physicalWidth".into(),json!(pixels.pixels.size().width));
        frame.extra.insert("physicalHeight".into(),json!(pixels.pixels.size().height));
        frame.png_digest = Some(super::output::digest(&png));
        let tree_value = serde_json::to_value(&last.tree).map_err(|error| Error::new("E_OUTPUT_IO",8,error.to_string()))?;
        let tree_bytes=super::output::json_bytes(&tree_value)?;
        frame.tree_digest = Some(super::output::digest(&tree_bytes));
        let frame_value = serde_json::to_value(&frame).map_err(|error| Error::new("E_OUTPUT_IO",8,error.to_string()))?;
        let mut output = self.probe.output.borrow_mut();
        output.bytes(&paths.png(),"capture-pixels",&png)?;
        output.bytes(&paths.tree(),"capture-tree",&tree_bytes)?;
        output.json(&paths.frame(),"capture-frame",&frame_value)?;
        if Instant::now()>=self.deadline.get() { return Err(Error::new("E_READINESS_TIMEOUT",5,"capture publication exceeded its step deadline").into()); }
        Ok(paths.summary())
    }
    pub fn graphics(&self) -> Value { self.capture.metadata() }
    pub fn frames(&self) -> u64 { self.probe.frames.get() }
    pub fn failure_evidence(&self) -> Result<Value, Error> {
        let last = self.probe.last.borrow();
        let Some(last) = last.as_ref() else { return Ok(json!({"available":false})); };
        // An empty post-fault document is not publishable evidence; recovery
        // selects the last complete checkpoint instead.
        if last.tree.nodes.is_empty() { return Ok(json!({"available":false})); }
        last.tree.validate_topology()?;
        let readiness = self.readiness();
        let satisfies = self.frame_current();
        // Read only an already presented buffer; never pump or enter application code.
        let pixels = self.window().ok().and_then(|window|self.capture.read_current(window.renderer()).ok())
            .filter(|pixels|pixels.presentation_id==last.presentation && pixels.configuration_id==last.configuration)
            .map(|pixels|capture::png(&pixels.pixels,self.opaque)).transpose()?;
        let mut output = self.probe.output.borrow_mut();
        evidence::publish_historical(&mut output, &last.tree, &last.frame, pixels.as_deref(),
            HistoricalStatus { satisfies_failure_revision:Some(satisfies), readiness:Some(readiness), tree_role:"failure-tree" })
    }
    pub fn finish(mut self) -> (Value, Option<Fault>) {
        self.deadline.set(self.run_deadline.min(Instant::now()+Duration::from_millis(20)));
        let previous_contract = self.harness.app().session().and_then(|session|session.contract_failure()).is_some();
        self.harness.shut_down();
        self.fixtures.dispose();
        let session = self.harness.app().session();
        let counts = session.map(|session|session.counts()).unwrap_or_default();
        let cleanup = session.and_then(|session|session.take_cleanup_failure()).map(|error| {
            let mut fault = Fault::application(error.into());
            fault.error = Error::new("E_TEARDOWN",9,fault.error.message);
            fault
        });
        let contract = if previous_contract { None } else { session.and_then(|session|session.contract_failure()) };
        let fault = contract.map(|error|Fault::application(error.into())).or(cleanup);
        let clean = counts==ResourceCounts::default() && fault.is_none();
        let teardown = json!({"verified":true,"clean":clean,"resources":counts,"sessionCreated":session.is_some()});
        drop(self);
        zgui::reactive::shutdown();
        (teardown,fault)
    }
}

fn graphics_error(error: zgui::runtime::AppError) -> Error {
    match error {
        zgui::runtime::AppError::GpuUnavailable(_) => Error::new("E_GRAPHICS_UNAVAILABLE",7,error.to_string()),
        _ => Error::new("E_HOST_INVARIANT",9,error.to_string()),
    }
}
