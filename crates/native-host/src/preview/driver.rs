use std::{cell::{Cell, RefCell}, rc::{Rc, Weak}, time::{Duration, Instant}};

use serde_json::{Value, json};
use zgui::{geom::{DevicePx, Size}, platform::{AppHandler, ColorScheme, IdlePolicy, PlatformCx, SurfaceEvent, SurfaceId, WakeReason}, render::{FrameOutcome, SkipReason}, runtime::{FrameProbe, Runtime, Window}};
use zgui_platform_headless::{Harness, Headless};

use crate::{Binding, MountRegion, Root, package::Package, session::{ResourceCounts, Session, SessionPolicy}};
use super::{Error, capture::{self, CaptureHandle}, inspect, output::Output, profile::{Limits, LoadedProfile}};
use super::semantics::SemanticModel;

mod actions;

pub type SharedOutput = Rc<RefCell<Output>>;

pub struct Fault {
    pub error: Error,
    pub diagnostic: Option<Value>,
}
impl From<Error> for Fault {
    fn from(error: Error) -> Self { Self { error, diagnostic: None } }
}
impl Fault {
    fn application(error: Box<dyn std::error::Error>) -> Self {
        if let Some(contract) = error.downcast_ref::<crate::session::ContractFailure>() {
            return Error::new(contract.code, 6, &contract.message).into();
        }
        if let Some(diagnostic) = error.downcast_ref::<js_runtime::Diagnostic>() {
            let (code, exit) = match diagnostic.kind {
                js_runtime::DiagnosticKind::Interrupted | js_runtime::DiagnosticKind::BudgetExceeded => ("E_JOB_BUDGET", 5),
                _ if diagnostic.phase == "promise rejection" => ("E_UNHANDLED_REJECTION", 3),
                _ => ("E_JS_EXCEPTION", 3),
            };
            return Self {
                error: Error::new(code, exit, &diagnostic.message),
                diagnostic: Some(json!({"kind":format!("{:?}",diagnostic.kind),"phase":diagnostic.phase,
                    "message":diagnostic.message,"generatedStack":diagnostic.generated_stack.as_ref().or(diagnostic.stack.as_ref()),
                    "mappedStack":diagnostic.stack,"mappingApplied":diagnostic.generated_stack.is_some()})),
            };
        }
        Error::new("E_BUNDLE", 3, error.to_string()).into()
    }
}

struct OwnedFrame {
    revision: u64,
    configuration: u64,
    metadata_revision: u64,
    native_nodes: Vec<ui_bridge::registry::SnapshotNode>,
    presentation: u64,
    frame: Value,
    tree: Value,
}

struct Probe {
    capture: CaptureHandle,
    session: RefCell<Weak<Session>>,
    output: SharedOutput,
    build_id: String,
    action: Cell<u64>,
    frames: Cell<u64>,
    last: RefCell<Option<OwnedFrame>>,
    presentation: RefCell<Option<Value>>,
    fault: RefCell<Option<Error>>,
    surface: RefCell<Option<std::sync::Arc<zgui_platform_headless::OffscreenSurface>>>,
    semantics: RefCell<SemanticModel>,
}
impl FrameProbe for Probe {
    fn frame_ended(&self, window: &Window) {
        let Some(observation) = window.completed_frame() else { return; };
        self.frames.set(observation.ordinal);
        if self.fault.borrow().is_some() { return; }
        if let Some(surface) = self.surface.borrow().as_ref()
            && let Err(error) = self.semantics.borrow_mut().consume(surface, window) {
            *self.fault.borrow_mut() = Some(error);
            return;
        }
        let Some(session) = self.session.borrow().upgrade() else { return; };
        if session.region.document != window.document_id() { return; }
        let state = self.capture.state();
        let stamp = json!({"ordinal":observation.ordinal,"timeUs":observation.timestamp.since_origin().as_micros(),
            "drawId":state.draw_id,"outcome":format!("{:?}",observation.outcome)});
        if matches!(observation.outcome, FrameOutcome::Presented(_)) {
            *self.presentation.borrow_mut() = Some(json!({"frame":stamp,"sequence":state.presentation_id,
                "configurationId":state.configuration_id,"drawId":state.presented_draw}));
        }
        if !observation.layout_document_coherent || !matches!(observation.outcome,
            FrameOutcome::Presented(_) | FrameOutcome::Skipped(SkipReason::Undamaged)) { return; }
        let Some(presentation) = self.presentation.borrow().clone() else { return; };
        if presentation["configurationId"].as_u64() != Some(state.configuration_id) { return; }
        let snapshot = inspect::snapshot(window, &session, observation.ordinal, &self.semantics.borrow());
        let mut tree = snapshot.tree;
        tree["buildId"] = json!(self.build_id);
        tree["presentationSequence"] = json!(state.presentation_id);
        let frame = json!({"schemaVersion":1,"buildId":self.build_id,"generation":session.generation,"surface":"main",
            "documentIdentity":format!("{:?}",window.document_id()),"actionSequence":self.action.get(),
            "requiredStateRevision":observation.document_revision,"validatedStateRevision":observation.document_revision,
            "layoutRevision":observation.layout_revision,"configurationId":state.configuration_id,
            "metadataRevision":snapshot.metadata_revision,
            "validationFrame":stamp,"pixelPresentation":presentation,"layoutDocumentCoherent":true,
            "timingDeterministic":false,"resources":{"nativeReadinessAvailable":false,"generation":null},
            "scale":window.surface().scale_factor(),"renderer":self.capture.metadata()});
        // Persist complete owned evidence before another callback can fail or block.
        // No readback, redraw request, signal write, or application entry occurs in this probe.
        let stem = format!("evidence/frame-{:010}", observation.ordinal);
        let publication = (|| {
            let mut output = self.output.borrow_mut();
            output.json(&format!("{stem}.tree.json"), "validation-tree", &tree)?;
            output.json(&format!("{stem}.frame.json"), "validation-frame", &frame)
        })();
        if let Err(error) = publication { *self.fault.borrow_mut() = Some(error); return; }
        *self.last.borrow_mut() = Some(OwnedFrame { revision: observation.document_revision,
            configuration: state.configuration_id, presentation: state.presentation_id, frame, tree,
            metadata_revision:snapshot.metadata_revision,native_nodes:snapshot.native_nodes });
    }
}

struct PreviewHost {
    native: Runtime,
    pending: Rc<RefCell<Vec<MountRegion>>>,
    package: Rc<Package>,
    session: Option<Rc<Session>>,
    executor: host_services::Executor,
    deadline: Rc<Cell<Instant>>,
    limits: js_runtime::RuntimeLimits,
    probe: Rc<Probe>,
    font_css: String,
    fault: Option<Fault>,
    requested_metadata: u64,
}
impl PreviewHost {
    fn fail(&mut self, cx: &dyn PlatformCx, fault: Fault) {
        if self.fault.is_none() { self.fault = Some(fault); }
        // The handler guards stop queued entries. The driver publishes the cause
        // before bounded disposal, so a blocked cleanup cannot erase that cause.
        cx.request_exit();
    }
    fn after_native(&mut self, cx: &dyn PlatformCx) {
        if self.fault.is_some() { return; }
        if let Some(error) = self.native.failure().take() { self.fail(cx, graphics_error(error).into()); return; }
        let regions = std::mem::take(&mut *self.pending.borrow_mut());
        for region in regions {
            if self.session.is_some() || self.native.windows().len() != 1 {
                self.fail(cx, Error::new("E_HOST_INVARIANT", 9, "preview requires exactly one current native document").into()); return;
            }
            let Some(window) = self.native.windows_mut().iter_mut().find(|window| window.document_id() == region.document) else {
                self.fail(cx, Error::new("E_HOST_INVARIANT", 9, "mount region has no current surface").into()); return;
            };
            let prepared = Session::prepare(region, Some(self.package.clone()), 1, cx.waker(), window.surface().id(),
                self.executor.clone(), SessionPolicy::Preview { limits:self.limits.clone(), deadline:self.deadline.clone() });
            let session = match prepared { Ok(session) => session, Err(error) => { self.fail(cx, Fault::application(error)); return; } };
            window.install_binding(Box::new(Binding(Rc::downgrade(&session))));
            *self.probe.session.borrow_mut() = Rc::downgrade(&session);
            self.session = Some(session.clone());
            if let Err(error) = session.mount() { self.fail(cx, Fault::application(error)); return; }
            session.region.host.install_stylesheet("preview-profile-fonts", &self.font_css);
            window.request_frame();
        }
        if let Some(session) = &self.session && let Err(error) = session.checkpoint() {
            self.fail(cx, Fault::application(error));
            return;
        }
        if let Some(session) = &self.session {
            let revision = session.bridge.metadata_revision();
            if revision != self.requested_metadata {
                self.requested_metadata = revision;
                if let Some(window) = self.native.windows().iter().find(|window|window.document_id()==session.region.document) {
                    window.request_frame();
                }
            }
        }
    }
}
impl AppHandler for PreviewHost {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) { self.native.surfaces_available(cx); self.after_native(cx); }
    fn surfaces_lost(&mut self, cx: &dyn PlatformCx) { if let Some(session) = &self.session { session.shutdown(); } self.native.surfaces_lost(cx); }
    fn surface_event(&mut self, cx: &dyn PlatformCx, surface: SurfaceId, event: SurfaceEvent) { if self.fault.is_none() { self.native.surface_event(cx, surface, event); self.after_native(cx); } }
    fn wake(&mut self, cx: &dyn PlatformCx, reason: WakeReason) { if self.fault.is_none() { self.native.wake(cx, reason); self.after_native(cx); } }
    fn idle(&mut self, cx: &dyn PlatformCx) -> IdlePolicy { if self.fault.is_some() { IdlePolicy::Block } else { self.native.idle(cx) } }
    fn deadline_reached(&mut self, cx: &dyn PlatformCx) { if self.fault.is_none() { self.native.deadline_reached(cx); self.after_native(cx); } }
    fn shutting_down(&mut self, cx: &dyn PlatformCx) { if let Some(session) = &self.session { session.shutdown(); } self.native.shutting_down(cx); }
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
}
impl Driver {
    pub fn new(profile: &LoadedProfile, package: Rc<Package>, output: SharedOutput, run_deadline: Instant) -> Result<Self, Fault> {
        let capture = CaptureHandle::default();
        let probe = Rc::new(Probe { capture:capture.clone(),session:RefCell::new(Weak::new()),output,
            build_id:package.manifest.build_id.clone(),action:Cell::new(0),frames:Cell::new(0),
            last:RefCell::new(None),presentation:RefCell::new(None),fault:RefCell::new(None),
            surface:RefCell::new(None),semantics:RefCell::new(SemanticModel::default()) });
        let pending = Rc::new(RefCell::new(Vec::new()));
        let fonts = profile.fonts.clone();
        let metrics = profile.fonts.clone();
        let raster = profile.fonts.clone();
        let surface = &profile.profile.surface;
        let mut app = zgui::runtime::App::new().with_title("zgui TSX preview")
            .with_size(surface.width as f32, surface.height as f32)
            .with_stylesheet(format!(":root {{ display:block; width:100%; height:100%; background-color:{}; }}",surface.background))
            .with_probe(probe.clone()).with_renderer(capture::renderer_factory(capture.clone()))
            .with_metrics(Box::new(move ||metrics.metrics()))
            .with_text_engine(Box::new(move ||Box::new(zgui_layout::Paragraphs::new(fonts.shaper()))))
            .with_glyph_raster(Box::new(move ||raster.raster()));
        app.attributes_mut().transparent = !surface.opaque;
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
        let host = PreviewHost { native,pending,package,session:None,
            executor:host_services::Executor::new(2,64).map_err(|error|Error::new("E_HOST_INVARIANT",9,error.to_string()))?,
            deadline:deadline.clone(),limits,probe:probe.clone(),
            font_css:format!(":root {{ font-family:{}; }}",profile.font_family_css),fault:None,requested_metadata:0 };
        let platform = Headless::new();
        platform.set_color_scheme(Some(if profile.profile.theme == "dark" { ColorScheme::Dark } else { ColorScheme::Light }));
        let mut harness = Harness::over(platform, host);
        harness.hold_clock(true);
        if let Some(window) = harness.app().native.windows().first() {
            let surface_id = window.surface().id();
            if let Some(offscreen) = harness.platform().offscreen(surface_id) {
                offscreen.set_refresh_rate_millihertz(Some(surface.refresh_rate_milli_hz));
                *probe.surface.borrow_mut() = Some(offscreen);
            }
            harness.deliver(surface_id, SurfaceEvent::ScaleFactorChanged { scale_factor:surface.scale,
                size:Size::new(DevicePx(profile.physical_width as f32),DevicePx(profile.physical_height as f32)) });
            harness.deliver(surface_id, SurfaceEvent::Focused(true));
        }
        Ok(Self { harness,probe,capture,deadline,run_deadline,step_start_frame:Cell::new(0),limits:profile.profile.limits.clone(),
            opaque:surface.opaque,dimensions:(profile.physical_width,profile.physical_height) })
    }
    pub fn begin_step(&self, action: u64, timeout_ms: Option<u64>) {
        self.probe.action.set(action);
        self.step_start_frame.set(self.probe.frames.get());
        self.deadline.set(self.run_deadline.min(Instant::now()+Duration::from_millis(timeout_ms.unwrap_or(self.limits.max_step_wall_ms).min(self.limits.max_step_wall_ms))));
    }
    pub fn check(&mut self) -> Result<(), Fault> {
        if let Some(fault) = self.harness.app_mut().fault.take() { return Err(fault); }
        if let Some(error) = self.probe.fault.borrow_mut().take() { return Err(error.into()); }
        if self.probe.frames.get()-self.step_start_frame.get()>u64::from(self.limits.max_frames_per_step) {
            return Err(Error::new("E_JOB_BUDGET",5,"native frame budget exhausted").into());
        }
        if Instant::now() >= self.deadline.get() { return Err(Error::new("E_READINESS_TIMEOUT",5,"step exhausted its real wall-clock deadline").into()); }
        Ok(())
    }
    fn window(&self) -> Result<&Window, Error> {
        let session = self.harness.app().session.as_ref().ok_or_else(||Error::new("E_HOST_INVARIANT",9,"no mounted session"))?;
        self.harness.app().native.windows().iter().find(|window|window.document_id()==session.region.document)
            .ok_or_else(||Error::new("E_HOST_INVARIANT",9,"logical main surface has no current native window"))
    }
    fn frame_current(&self) -> bool {
        let Ok(window) = self.window() else { return false; };
        let state = self.capture.state();
        let last = self.probe.last.borrow();
        last.as_ref().is_some_and(|last| last.revision==window.dom().revision() && last.configuration==state.configuration_id
            && last.presentation==state.presentation_id && state.presented_draw.is_some()
            && self.harness.app().session.as_ref().is_some_and(|session|session.bridge.metadata_revision()==last.metadata_revision)
            && window.completed_frame().is_some_and(|frame| frame.layout_document_coherent
                && Some(frame.ordinal)==last.frame["validationFrame"]["ordinal"].as_u64()
                && matches!(frame.outcome,FrameOutcome::Presented(_)|FrameOutcome::Skipped(SkipReason::Undamaged))))
    }
    fn runnable(&self) -> bool {
        self.harness.platform().has_pending_wakes()
            || self.window().ok().and_then(|window|self.harness.platform().offscreen(window.surface().id())).is_some_and(|surface|surface.has_pending_redraw())
            || self.harness.parked_deadline().is_some_and(|next|next<=self.harness.now())
    }
    pub fn readiness(&self) -> Value {
        json!({"appReady":self.harness.app().session.is_some(),"currentWorkSettled":!self.runnable(),
            "requiredAssetsReady":null,"nativeAssetReadinessAvailable":false,"frameCurrent":self.frame_current(),
            "timeUs":self.harness.platform().clock().timestamp().since_origin().as_micros(),
            "nextNativeDeadlineUs":self.harness.parked_deadline().map(|next|self.harness.platform().clock().timestamp().since_origin().as_micros()+next.saturating_duration_since(self.harness.now()).as_micros()),
            "requiredMetadataRevision":self.harness.app().session.as_ref().map(|session|session.bridge.metadata_revision()),
            "requiredStateRevision":self.window().ok().map(|window|window.dom().revision()),"generation":1})
    }
    pub fn settle(&mut self, require_frame: bool) -> Result<Value, Fault> {
        for _ in 0..self.limits.max_settle_turns {
            self.check()?;
            // Wall profiles mirror elapsed monotonic time, never a fabricated fixed frame step.
            self.harness.advance(Instant::now().saturating_duration_since(self.harness.now()));
            self.harness.pump();
            self.check()?;
            if !self.runnable() && (!require_frame || self.frame_current()) { return Ok(self.readiness()); }
            if matches!(self.capture.last_outcome(),Some(FrameOutcome::Skipped(SkipReason::DeviceUnavailable))) {
                return Err(Error::new("E_DEVICE_LOST",7,"native graphics device became unavailable").into());
            }
            if !self.runnable() { std::thread::sleep(Duration::from_millis(1)); }
        }
        Err(Error::new("E_CAPTURE_NOT_READY",5,format!("bounded native pump did not reach readiness: {}",self.readiness())).into())
    }
    pub fn capture(&mut self, id: &str) -> Result<Value, Fault> {
        let readiness = self.settle(true)?;
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
        let mut frame = last.frame.clone();
        frame["captureId"] = json!(id);
        frame["readiness"] = readiness;
        frame["physicalWidth"] = json!(pixels.pixels.size().width);
        frame["physicalHeight"] = json!(pixels.pixels.size().height);
        frame["pngDigest"] = json!(super::output::digest(&png));
        let tree_bytes=super::output::json_bytes(&last.tree)?;
        frame["treeDigest"] = json!(super::output::digest(&tree_bytes));
        let mut output = self.probe.output.borrow_mut();
        output.bytes(&format!("captures/{id}.png"),"capture-pixels",&png)?;
        output.bytes(&format!("captures/{id}.tree.json"),"capture-tree",&tree_bytes)?;
        output.json(&format!("captures/{id}.frame.json"),"capture-frame",&frame)?;
        if Instant::now()>=self.deadline.get() { return Err(Error::new("E_READINESS_TIMEOUT",5,"capture publication exceeded its step deadline").into()); }
        Ok(json!({"captureId":id,"png":format!("captures/{id}.png"),"frame":format!("captures/{id}.frame.json"),"tree":format!("captures/{id}.tree.json")}))
    }
    pub fn graphics(&self) -> Value { self.capture.metadata() }
    pub fn frames(&self) -> u64 { self.probe.frames.get() }
    pub fn failure_evidence(&self) -> Result<Value, Error> {
        let last = self.probe.last.borrow();
        let Some(last) = last.as_ref() else { return Ok(json!({"available":false})); };
        let mut frame = last.frame.clone();
        frame["historical"] = json!(true);
        frame["satisfiesFailureRevision"] = json!(self.frame_current());
        frame["failureReadiness"] = self.readiness();
        let mut output = self.probe.output.borrow_mut();
        output.json("failure/tree.json","failure-tree",&last.tree)?;
        // Read only an already presented buffer; never pump or enter application code.
        let pixel_path = self.window().ok().and_then(|window|self.capture.read_current(window.renderer()).ok())
            .filter(|pixels|pixels.presentation_id==last.presentation && pixels.configuration_id==last.configuration)
            .map(|pixels|capture::png(&pixels.pixels,self.opaque)).transpose()?;
        if let Some(png) = pixel_path.as_ref() { output.bytes("failure/last-valid.png","historical-pixels",png)?; }
        frame["pixelsAvailable"] = json!(pixel_path.is_some());
        output.json("failure/frame.json","historical-frame",&frame)?;
        Ok(json!({"available":true,"historical":true,"tree":"failure/tree.json","frame":"failure/frame.json",
            "pixels":pixel_path.map(|_|"failure/last-valid.png")}))
    }
    pub fn finish(mut self) -> (Value, Option<Fault>) {
        self.deadline.set(self.run_deadline.min(Instant::now()+Duration::from_millis(20)));
        let previous_contract = self.harness.app().session.as_ref().and_then(|session|session.contract_failure()).is_some();
        self.harness.shut_down();
        let session = self.harness.app().session.as_ref();
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
