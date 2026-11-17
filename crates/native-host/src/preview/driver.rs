use std::{cell::{Cell, RefCell}, rc::Rc, time::{Duration, Instant}};

use serde_json::{Value, json};
use zgui::{geom::{DevicePx, Size}, platform::{AppHandler, ColorScheme, IdlePolicy, PlatformCx, SurfaceEvent, SurfaceId, WakeReason}, runtime::{Runtime, Window}};
use zgui_platform_headless::Headless;

use crate::{package::Package, session::{ResourceCounts, RetirementFailure, RuntimeTelemetry, Session, SessionError, SessionModule, SessionPolicy, SessionRetirement}};
use crate::session_host::{ImageChoice, RecoveryPolicy, RuntimeSurface, SessionHost};
use super::{Error, fixture::FixtureDefinition, output::Output, pipeline::{self, CaptureRequest, CheckpointSink, FrameObserver, OutputSink, Pipeline, SettledFrame}, profile::{self, Limits, LoadedProfile, Surface}, resources};
use crate::{assets::{ContractFailure, fixture::FixtureRuntime}, environment::Environment};

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
        }
    }
}

struct PreviewHost {
    native: Runtime,
    host: SessionHost,
    deadline: Rc<Cell<Instant>>,
    observer: FrameObserver,
    font_css: String,
    fault: Option<Fault>,
    requested_metadata: u64,
    retired: Option<Value>,
    retired_telemetry: Option<RuntimeTelemetry>,
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
        if let Some(error) = self.native.failure().take() { self.fail(cx, pipeline::preview_app_error(error).into()); return; }
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
            // inspect it; the observer binds to whichever generation is current.
            self.observer.bind_session(self.host.sessions().next().map(Rc::downgrade).unwrap_or_default());
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
            let Some(session) = self.host.retire(document) else { continue; };
            let report = session.retire();
            let clean = report.counts == ResourceCounts::default() && report.failure.is_none();
            self.retired = Some(json!({
                "generation": report.generation,
                "before": report.before,
                "resources": report.counts,
                "clean": clean,
            }));
            if let Some(fault) = evaluate_retirement(Some(&report)) {
                self.fail(cx, fault);
            }
            self.retired_telemetry = RuntimeTelemetry::merge(self.retired_telemetry, report.telemetry);
            drop(session);
        }
        self.host.stop();
        self.deadline.set(deadline);
        self.requested_metadata = 0;
        self.observer.reset_surface();
        self.native.surfaces_lost(cx);
    }
    fn surface_event(&mut self, cx: &dyn PlatformCx, surface: SurfaceId, event: SurfaceEvent) { if self.fault.is_none() { self.native.surface_event(cx, surface, event); self.after_native(cx); } }
    fn wake(&mut self, cx: &dyn PlatformCx, reason: WakeReason) { if self.fault.is_none() { self.native.wake(cx, reason); self.after_native(cx); } }
    fn idle(&mut self, cx: &dyn PlatformCx) -> IdlePolicy { if self.fault.is_some() { IdlePolicy::Block } else { self.native.idle(cx) } }
    fn deadline_reached(&mut self, cx: &dyn PlatformCx) { if self.fault.is_none() { self.native.deadline_reached(cx); self.after_native(cx); } }
    fn shutting_down(&mut self, cx: &dyn PlatformCx) { for session in self.host.sessions() { let _ = session.retire(); } self.native.shutting_down(cx); }
}

pub struct Driver {
    pipeline: Pipeline<PreviewHost>,
    observer: FrameObserver,
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
        Self::with_sink(profile, package, fixture, Box::new(OutputSink(output)), run_deadline, stylesheet)
    }

    pub(super) fn with_sink(profile: &LoadedProfile, package: Rc<Package>, fixture:FixtureDefinition, sink: Box<dyn CheckpointSink>, run_deadline: Instant, stylesheet:String) -> Result<Self, Fault> {
        let platform=Headless::new();
        let (epoch,start)=match profile.profile.clock {
            profile::Clock::Virtual{epoch_ms,start_us}=>(Some(epoch_ms),start_us),
            profile::Clock::Wall{}=>(None,0),
        };
        platform.virtual_clock().advance(Duration::from_micros(start));
        let environment=Environment::new(platform.clock(),epoch,profile.profile.random.seed);
        let fixtures=FixtureRuntime::new(fixture,environment.clone());
        let observer = FrameObserver::new(package.manifest.build_id.clone(), environment.clone(), sink);
        let pending = Rc::new(RefCell::new(Vec::new()));
        let surface = &profile.profile.surface;
        let deadline = Rc::new(Cell::new(run_deadline.min(Instant::now()+Duration::from_millis(profile.profile.limits.max_step_wall_ms))));
        let limits = js_runtime::RuntimeLimits {
            entry_timeout:Duration::from_millis(profile.profile.limits.max_js_entry_wall_ms),
            bootstrap_timeout:Duration::from_millis(profile.profile.limits.max_step_wall_ms),
            max_jobs_per_pump:profile.profile.limits.max_jobs_per_pump as usize,
            job_timeout:Duration::from_millis(profile.profile.limits.max_pump_wall_ms),
            stack_limit_bytes:profile.profile.limits.max_js_stack_bytes.unwrap_or_else(|| js_runtime::RuntimeLimits::default().stack_limit_bytes),
            ..Default::default()
        };
        let session_host = SessionHost::new(pending.clone(), SessionModule::package(package.clone()),
            SessionPolicy::preview(limits, deadline.clone(), environment.clone(), fixtures.clone()),
            RecoveryPolicy::Propagate);
        platform.set_color_scheme(Some(if profile.profile.theme == "dark" { ColorScheme::Dark } else { ColorScheme::Light }));
        let font_css = format!(":root {{ font-family:{}; }}", profile.font_family_css);
        let observer_ref = observer.clone();
        let deadline_ref = deadline.clone();
        let pipeline = pipeline::for_preview(profile, &package, platform, stylesheet, &observer, pending, move |native| PreviewHost {
            native,
            host: session_host,
            deadline: deadline_ref,
            observer: observer_ref,
            font_css,
            fault: None,
            requested_metadata: 0,
            retired: None,
            retired_telemetry: None,
        })?;
        let mut driver = Self { pipeline,observer,deadline,run_deadline,step_start_frame:Cell::new(0),limits:profile.profile.limits.clone(),
            opaque:surface.opaque,dimensions:(profile.physical_width,profile.physical_height),surface:surface.clone(),
            environment,fixtures,maximum_time_us:start+profile.profile.limits.max_virtual_duration_ms*1000,startup_fault:None };
        if let Err(fault) = driver.bind_surface() { driver.startup_fault = Some(fault); }
        Ok(driver)
    }
    fn bind_surface(&mut self) -> Result<(), Fault> {
        self.check()?;
        let surface = self.window()?.surface().id();
        let offscreen = self.pipeline.platform().offscreen(surface)
            .ok_or_else(||Error::new("E_HOST_INVARIANT",9,"current native main has no headless surface"))?;
        offscreen.set_refresh_rate_millihertz(Some(self.surface.refresh_rate_milli_hz));
        self.observer.bind_surface(offscreen);
        self.pipeline.deliver(surface, SurfaceEvent::ScaleFactorChanged { scale_factor:self.surface.scale,
            size:Size::new(DevicePx(self.dimensions.0 as f32),DevicePx(self.dimensions.1 as f32)) });
        self.pipeline.deliver(surface, SurfaceEvent::Focused(true));
        self.window()?.request_frame();
        self.check()
    }
    pub fn generation(&self) -> u64 { self.pipeline.app().host.generation() }
    /// Recreate the real native document while retaining run time, RNG and fixture history.
    pub fn recreate(&mut self) -> Result<Value, Fault> {
        self.check()?;
        let old_surface = self.window()?.surface().id();
        let old_document = format!("{:?}",self.window()?.document_id());
        let old_generation = self.generation();
        let time = self.environment.now_us();
        self.pipeline.suspend();
        self.check()?;
        if !self.pipeline.app().native.windows().is_empty() {
            return Err(Error::new("E_HOST_INVARIANT",9,"suspended runtime retained native windows").into());
        }
        // The native harness may retain old offscreen entries. Only Runtime.windows owns main.
        self.pipeline.resume();
        self.check()?;
        let new_surface = self.window()?.surface().id();
        let new_document = format!("{:?}",self.window()?.document_id());
        if new_surface==old_surface || new_document==old_document || self.generation()!=old_generation+1 {
            return Err(Error::new("E_HOST_INVARIANT",9,"surface resume did not create a fresh native generation").into());
        }
        self.observer.bump_surface_configuration();
        self.bind_surface()?;
        self.settle(true)?;
        Ok(json!({"oldSurfaceId":format!("{old_surface:?}"),"newSurfaceId":format!("{new_surface:?}"),
            "oldDocumentId":old_document,"newDocumentId":new_document,"oldGeneration":old_generation,
            "generation":self.generation(),"fromTimeUs":time,"timeUs":self.environment.now_us(),
            "retired":self.pipeline.app().retired,"readiness":self.readiness()}))
    }
    pub fn begin_step(&self, action: u64, timeout_ms: Option<u64>) {
        self.step_start_frame.set(self.observer.begin_step(action));
        self.deadline.set(self.run_deadline.min(Instant::now()+Duration::from_millis(timeout_ms.unwrap_or(self.limits.max_step_wall_ms).min(self.limits.max_step_wall_ms))));
    }
    pub fn check(&mut self) -> Result<(), Fault> {
        if let Some(fault) = self.startup_fault.take() { return Err(fault); }
        if let Some(fault) = self.pipeline.app_mut().fault.take() { return Err(fault); }
        if let Some(error) = self.observer.take_fault() { return Err(error.into()); }
        if self.observer.exceeded_step_budget(self.step_start_frame.get(), self.limits.max_frames_per_step) {
            return Err(Error::new("E_JOB_BUDGET",5,"native frame budget exhausted").into());
        }
        if Instant::now() >= self.deadline.get() { return Err(Error::new("E_READINESS_TIMEOUT",5,"step exhausted its real wall-clock deadline").into()); }
        Ok(())
    }
    fn window(&self) -> Result<&Window, Error> {
        let session = self.pipeline.app().session().ok_or_else(||Error::new("E_HOST_INVARIANT",9,"no mounted session"))?;
        self.pipeline.app().native.windows().iter().find(|window|window.document_id()==session.region.document)
            .ok_or_else(||Error::new("E_HOST_INVARIANT",9,"logical main surface has no current native window"))
    }
    fn window_mut(&mut self)->Result<&mut Window,Error> {
        let document=self.pipeline.app().session().ok_or_else(||Error::new("E_HOST_INVARIANT",9,"no mounted session"))?.region.document;
        self.pipeline.app_mut().native.windows_mut().iter_mut().find(|window|window.document_id()==document)
            .ok_or_else(||Error::new("E_HOST_INVARIANT",9,"logical main surface has no current native window"))
    }
    fn settled(&self) -> Option<SettledFrame<'_>> {
        let window = self.window().ok()?;
        self.observer.settled(window, self.pipeline.app().session())
    }
    fn frame_current(&self) -> bool {
        self.settled().is_some()
    }
    fn runnable(&self) -> bool {
        self.pipeline.platform().has_pending_wakes()
            || self.window().ok().and_then(|window|self.pipeline.platform().offscreen(window.surface().id())).is_some_and(|surface|surface.has_pending_redraw())
            || self.pipeline.parked_deadline().is_some_and(|next|next<=self.pipeline.now())
            || self.fixtures.next_deadline_us().is_some_and(|next|next<=self.environment.now_us())
    }
    pub fn readiness(&self) -> Value {
        let resources=self.window().ok().map(|window|resources::snapshot(window,&[]));
        let resource=resources.as_ref().and_then(|result|result.as_ref().ok());
        json!({"appReady":self.pipeline.app().session().is_some(),"currentWorkSettled":!self.runnable(),
            "requiredAssetsReady":resource.map(|value|&value["requiredAssetsReady"]),"nativeAssetReadinessAvailable":true,"frameCurrent":self.frame_current(),
            "resourceRevision":self.window().ok().map(Window::image_resource_revision),
            "pendingAssets":resource.and_then(|value|value["pendingIds"].as_array()).map(|items|items.len()),
            "timeUs":self.environment.now_us(),"nextFixtureDeadlineUs":self.fixtures.next_deadline_us(),
            "nextNativeDeadlineUs":self.pipeline.parked_deadline().map(|next|next.saturating_duration_since(self.pipeline.platform().clock().origin()).as_micros()),
            "requiredMetadataRevision":self.pipeline.app().session().map(|session|session.bridge.metadata_revision()),
            "requiredStateRevision":self.window().ok().map(|window|window.dom().revision()),"generation":self.generation()})
    }
    fn notify_fixture(&self)->Result<(),Error> {
        let surface=self.window()?.surface().id();
        self.pipeline.platform().waker().wake(WakeReason::ReactiveWork{surfaces:vec![surface].into_boxed_slice()});
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
            let now = self.pipeline.now();
            self.pipeline.advance(Instant::now().saturating_duration_since(now));
        }
        if self.fixtures.release_due()? { self.notify_fixture()?; }
        self.pipeline.pump();
        self.check()
    }
    pub fn settle(&mut self, require_frame: bool) -> Result<Value, Fault> {
        for _ in 0..self.limits.max_settle_turns {
            self.pump_current()?;
            if self.observer.device_unavailable() {
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
            if self.observer.last_time_us()!=Some(time) {
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
        let window=self.window()?;
        let settled=self.observer.settled(window,self.pipeline.app().session())
            .ok_or_else(||Error::new("E_CAPTURE_NOT_READY",5,"no owned validated tree").with_details(readiness.clone()))?;
        if required_time.is_some_and(|time|settled.time_us()!=time) {
            return Err(Error::new("E_CAPTURE_NOT_READY",5,"validation frame timestamp differs from requiredFrameTimeUs").with_details(readiness).into());
        }
        let paths = self.observer.write_capture(&settled,window,CaptureRequest {
            id,
            dimensions: self.dimensions,
            opaque: self.opaque,
            readiness,
            resource,
            allowed_pending,
        })?;
        drop(settled);
        self.check()?;
        if Instant::now()>=self.deadline.get() { return Err(Error::new("E_READINESS_TIMEOUT",5,"capture publication exceeded its step deadline").into()); }
        Ok(paths)
    }
    pub fn graphics(&self) -> Value { self.observer.graphics_metadata() }
    pub fn frames(&self) -> u64 { self.observer.frames() }
    pub fn failure_evidence(&self) -> Result<Value, Error> {
        let readiness = self.readiness();
        let satisfies = self.frame_current();
        self.observer.publish_failure_evidence(self.window().ok(), self.opaque, readiness, satisfies)
    }
    pub fn finish(mut self) -> (Value, Option<Fault>, Option<RuntimeTelemetry>) {
        self.deadline.set(self.run_deadline.min(Instant::now()+Duration::from_millis(20)));
        self.pipeline.shut_down();
        self.fixtures.dispose();
        let report = self.pipeline.app().session().map(|session| session.retire());
        let telemetry = RuntimeTelemetry::merge(
            self.pipeline.app().retired_telemetry,
            report.as_ref().and_then(|report| report.telemetry),
        );
        let counts = report.as_ref().map(|report| report.counts.clone()).unwrap_or_default();
        let fault = evaluate_retirement(report.as_ref());
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
    use super::*;
    use crate::session::{RetirementFailure, SessionRetirement};

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
            counts: ResourceCounts { timers: 1, ..Default::default() },
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

        let profile_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../preview/profiles/default.json");
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
        let deadline = Instant::now() + Duration::from_secs(10);
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
            Instant::now() + Duration::from_secs(10),
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
}
