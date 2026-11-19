//! Headless native rendering pipeline, witness-checked frame observation, and checkpoint persistence.

use std::{
    cell::{Cell, Ref, RefCell},
    collections::BTreeMap,
    rc::{Rc, Weak},
    sync::Arc,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use zgui::{
    app::Fonts,
    geom::{DevicePx, Size},
    platform::{AppHandler, Surface as _, SurfaceEvent, SurfaceId},
    render::{FrameOutcome, SkipReason},
    runtime::{App, AppError, FrameProbe, Runtime, Window},
    view::{IntoView, NodeId, View},
};
use zgui_platform_headless::{Harness, Headless, OffscreenSurface};
use zgui_render_wgpu::Pixels;

use crate::{
    environment::Environment,
    package::Package,
    session::Session,
    session_host::{MountRegion, Root},
};
use super::{
    Error,
    driver::{Fault, SharedOutput},
    evidence::{
        self, CaptureIdentity, CapturePaths, FrameEvidence, FrameStamp, HistoricalStatus,
        TreeEvidence,
    },
    output,
    profile::LoadedProfile,
    resources,
};

mod capture;
mod inspect;
mod ledger;
mod semantics;

use capture::{CaptureHandle, PresentedPixels};
use ledger::{CommittedMeta, FrameInput, FrameLedger, FrameStatus, SettledQuery};
use semantics::SemanticModel;

pub(crate) struct CaptureRequest<'a> {
    pub id: &'a str,
    pub dimensions: (u32, u32),
    pub opaque: bool,
    pub readiness: Value,
    pub resource: Value,
    pub allowed_pending: &'a [String],
}

/// Internal persistence port separating frame observation from disk output.
pub(crate) trait CheckpointSink {
    fn write_checkpoint(
        &mut self,
        tree_path: &str,
        tree: &Value,
        frame_path: &str,
        frame: &Value,
    ) -> Result<(), Error>;
    fn write_capture(
        &mut self,
        paths: &CapturePaths,
        png: &[u8],
        tree_bytes: &[u8],
        frame: &Value,
    ) -> Result<(), Error>;
    fn publish_historical(
        &mut self,
        tree: &TreeEvidence,
        frame: &FrameEvidence,
        pixels: Option<&[u8]>,
        status: HistoricalStatus,
    ) -> Result<Value, Error>;
}

pub(crate) struct OutputSink(pub SharedOutput);

impl CheckpointSink for OutputSink {
    fn write_checkpoint(
        &mut self,
        tree_path: &str,
        tree: &Value,
        frame_path: &str,
        frame: &Value,
    ) -> Result<(), Error> {
        let mut output = self.0.borrow_mut();
        output.json(tree_path, "validation-tree", tree)?;
        output.json(frame_path, "validation-frame", frame)
    }

    fn write_capture(
        &mut self,
        paths: &CapturePaths,
        png: &[u8],
        tree_bytes: &[u8],
        frame: &Value,
    ) -> Result<(), Error> {
        let mut output = self.0.borrow_mut();
        output.bytes(&paths.png(), "capture-pixels", png)?;
        output.bytes(&paths.tree(), "capture-tree", tree_bytes)?;
        output.json(&paths.frame(), "capture-frame", frame)
    }

    fn publish_historical(
        &mut self,
        tree: &TreeEvidence,
        frame: &FrameEvidence,
        pixels: Option<&[u8]>,
        status: HistoricalStatus,
    ) -> Result<Value, Error> {
        let mut output = self.0.borrow_mut();
        evidence::publish_historical(&mut output, tree, frame, pixels, status)
    }
}

#[cfg(test)]
#[derive(Default)]
struct MemorySinkState {
    checkpoints: Vec<(String, Value, String, Value)>,
    captures: Vec<(String, Vec<u8>, Vec<u8>, Value)>,
    historical: Option<(TreeEvidence, FrameEvidence, Option<Vec<u8>>)>,
    fail_checkpoint_after: Option<usize>,
}

#[cfg(test)]
#[derive(Clone)]
pub(crate) struct MemorySink(Rc<RefCell<MemorySinkState>>);

#[cfg(test)]
impl MemorySink {
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(MemorySinkState::default())))
    }

    pub fn boxed(&self) -> Box<dyn CheckpointSink> {
        Box::new(self.clone())
    }

    pub fn fail_checkpoint_after(&self, limit: Option<usize>) {
        self.0.borrow_mut().fail_checkpoint_after = limit;
    }

    pub fn checkpoint_count(&self) -> usize {
        self.0.borrow().checkpoints.len()
    }

    pub fn last_historical(&self) -> Option<(TreeEvidence, FrameEvidence, Option<Vec<u8>>)> {
        self.0.borrow().historical.clone()
    }
}

#[cfg(test)]
impl CheckpointSink for MemorySink {
    fn write_checkpoint(
        &mut self,
        tree_path: &str,
        tree: &Value,
        frame_path: &str,
        frame: &Value,
    ) -> Result<(), Error> {
        let mut state = self.0.borrow_mut();
        if state
            .fail_checkpoint_after
            .is_some_and(|limit| state.checkpoints.len() >= limit)
        {
            return Err(Error::new("E_OUTPUT_LIMIT", 8, "simulated checkpoint limit"));
        }
        state.checkpoints.push((
            tree_path.to_owned(),
            tree.clone(),
            frame_path.to_owned(),
            frame.clone(),
        ));
        Ok(())
    }

    fn write_capture(
        &mut self,
        paths: &CapturePaths,
        png: &[u8],
        tree_bytes: &[u8],
        frame: &Value,
    ) -> Result<(), Error> {
        self.0.borrow_mut().captures.push((
            paths.id().to_owned(),
            png.to_vec(),
            tree_bytes.to_vec(),
            frame.clone(),
        ));
        Ok(())
    }

    fn publish_historical(
        &mut self,
        tree: &TreeEvidence,
        frame: &FrameEvidence,
        pixels: Option<&[u8]>,
        status: HistoricalStatus,
    ) -> Result<Value, Error> {
        tree.validate_frame(frame)?;
        tree.validate_topology()?;
        let mut frame = frame.clone();
        frame.extra.insert("historical".into(), json!(true));
        frame.extra.insert(
            "satisfiesFailureRevision".into(),
            status.satisfies_failure_revision.map_or(Value::Null, Value::from),
        );
        if let Some(readiness) = status.readiness {
            frame.extra.insert("failureReadiness".into(), readiness);
        }
        frame.extra.insert("pixelsAvailable".into(), json!(pixels.is_some()));
        self.0.borrow_mut().historical =
            Some((tree.clone(), frame, pixels.map(<[u8]>::to_vec)));
        Ok(json!({
            "available": true,
            "historical": true,
            "tree": "failure/tree.json",
            "frame": "failure/frame.json",
            "pixels": pixels.map(|_| "failure/last-valid.png"),
        }))
    }
}

/// Owns a headless `Harness` and enforces ordered teardown (`shut_down -> drop -> reactive::shutdown`)
/// across normal exits, early `?` returns, and test panics.
pub(crate) struct Pipeline<H: AppHandler> {
    harness: Option<Harness<H>>,
    shut_down: bool,
}

impl<H: AppHandler> Pipeline<H> {
    fn from_harness(mut harness: Harness<H>) -> Self {
        // Explicit callers drive every clock advance once constructed.
        harness.hold_clock(true);
        Self { harness: Some(harness), shut_down: false }
    }

    fn inner(&self) -> &Harness<H> {
        self.harness.as_ref().expect("pipeline harness active")
    }

    fn inner_mut(&mut self) -> &mut Harness<H> {
        self.harness.as_mut().expect("pipeline harness active")
    }

    pub fn app(&self) -> &H {
        self.inner().app()
    }

    pub fn app_mut(&mut self) -> &mut H {
        self.inner_mut().app_mut()
    }

    pub fn platform(&self) -> &Headless {
        self.inner().platform()
    }

    pub fn now(&self) -> Instant {
        self.inner().now()
    }

    pub fn parked_deadline(&self) -> Option<Instant> {
        self.inner().parked_deadline()
    }

    pub fn advance(&mut self, duration: Duration) {
        self.inner_mut().advance(duration);
    }

    pub fn deliver(&mut self, surface: SurfaceId, event: SurfaceEvent) {
        self.inner_mut().deliver(surface, event);
    }

    pub fn deliver_all(
        &mut self,
        surface: SurfaceId,
        events: impl IntoIterator<Item = SurfaceEvent>,
    ) {
        self.inner_mut().deliver_all(surface, events);
    }

    /// Drain queued application wakes before routing the resulting surface redraws.
    pub fn dispatch_wakes(&mut self) {
        self.inner_mut().dispatch_wakes();
    }

    pub fn pump(&mut self) -> u64 {
        self.inner_mut().pump()
    }

    pub fn suspend(&mut self) {
        self.inner_mut().suspend();
    }

    pub fn resume(&mut self) {
        self.inner_mut().resume();
    }

    pub fn shut_down(&mut self) {
        if !self.shut_down
            && let Some(harness) = self.harness.as_mut()
        {
            harness.shut_down();
            self.shut_down = true;
        }
    }
}

impl<H: AppHandler> Drop for Pipeline<H> {
    fn drop(&mut self) {
        if let Some(mut harness) = self.harness.take() {
            if !self.shut_down {
                harness.shut_down();
            }
            drop(harness);
        }
        zgui::reactive::shutdown();
    }
}

fn configure_app(
    fonts: &Fonts,
    width: u32,
    height: u32,
    opaque: bool,
    title: &str,
    stylesheet: String,
    renderer: zgui::runtime::RendererFactory,
) -> App {
    let shaping = fonts.clone();
    let metrics = fonts.clone();
    let raster = fonts.clone();
    let mut app = App::new()
        .with_title(title)
        .with_size(width as f32, height as f32)
        .with_stylesheet(stylesheet)
        .with_renderer(renderer)
        .with_metrics(Box::new(move || metrics.metrics()))
        .with_text_engine(Box::new(move || Box::new(zgui_layout::Paragraphs::new(shaping.shaper()))))
        .with_glyph_raster(Box::new(move || raster.raster()));
    app.attributes_mut().transparent = !opaque;
    app
}

pub(crate) fn preview_app_error(error: AppError) -> Error {
    match error {
        AppError::GpuUnavailable(_) => Error::new("E_GRAPHICS_UNAVAILABLE", 7, error.to_string()),
        _ => Error::new("E_HOST_INVARIANT", 9, error.to_string()),
    }
}

/// Constructs a headless interactive preview `Pipeline` wired to `observer`.
pub(crate) fn for_preview<H: AppHandler>(
    profile: &LoadedProfile,
    package: &Package,
    platform: Headless,
    stylesheet: String,
    observers: &FrameObservers,
    pending: Rc<RefCell<Vec<MountRegion>>>,
    build_host: impl FnOnce(Runtime) -> H,
) -> Result<Pipeline<H>, Error> {
    let surface = &profile.profile.surface;
    let registry = observers.clone();
    let app = configure_app(
        &profile.fonts,
        surface.width,
        surface.height,
        surface.opaque,
        "zgui TSX preview",
        stylesheet,
        capture::renderer_factory(move |surface| registry.register_renderer(surface)),
    )
    .with_probe(Rc::new(observers.clone()));
    let app = resources::configure(app, package);
    let fonts = profile.fonts.clone();
    let mut native = app
        .into_handler(move |cx| {
            zgui::reactive::provide_context(fonts.clone());
            Box::new(View::build(Root(pending.clone()), cx))
        })
        .map_err(preview_app_error)?;
    if profile.profile.pure_shell { native.without_primary_surface(); }
    let host = build_host(native);
    Ok(Pipeline::from_harness(Harness::over(platform, host)))
}

/// Frame returned by [`StaticPreview::await_frame`] for static calibration inspection.
pub(crate) struct StaticFrame(PresentedPixels);

impl StaticFrame {
    pub fn pixels(&self) -> &Pixels {
        &self.0.pixels
    }

    pub fn target_size(&self) -> Size<i32, zgui::geom::Device> {
        self.0.target.size
    }

    pub fn draw_id(&self) -> u64 {
        self.0.draw_id
    }

    pub fn presentation_id(&self) -> u64 {
        self.0.presentation_id
    }

    pub fn configuration_id(&self) -> u64 {
        self.0.configuration_id
    }
}

/// Static headless preview runner shared with calibration, hiding `CaptureHandle` and `Pipeline` teardown.
pub(crate) struct StaticPreview {
    pipeline: Pipeline<Runtime>,
    capture: CaptureHandle,
    map_err: fn(AppError) -> Error,
}

impl StaticPreview {
    pub fn check_runtime(&self) -> Result<(), Error> {
        self.pipeline
            .app()
            .failure()
            .take()
            .map_or(Ok(()), |error| Err((self.map_err)(error)))
    }

    pub fn window_count(&self) -> usize {
        self.pipeline.app().windows().len()
    }

    pub fn configure(&mut self, width: u32, height: u32, scale: f64) {
        self.pipeline
            .inner_mut()
            .deliver_to_first(SurfaceEvent::ScaleFactorChanged {
                scale_factor: scale,
                size: Size::new(DevicePx(width as f32), DevicePx(height as f32)),
            });
    }

    pub fn await_frame(&mut self, deadline: Instant) -> Result<StaticFrame, Error> {
        self.capture.request();
        for _ in 0..512 {
            if Instant::now() >= deadline {
                return Err(Error::new(
                    "E_READINESS_TIMEOUT",
                    5,
                    "calibration exceeded its real wall-clock budget",
                ));
            }
            let ran = self.pipeline.pump();
            self.check_runtime()?;
            if Instant::now() >= deadline {
                return Err(Error::new(
                    "E_READINESS_TIMEOUT",
                    5,
                    "calibration exceeded its real wall-clock budget",
                ));
            }
            if let Some(frame) = self.capture.take() {
                return Ok(StaticFrame(frame));
            }
            if matches!(
                self.capture.last_outcome(),
                Some(FrameOutcome::Skipped(SkipReason::DeviceUnavailable))
            ) {
                return Err(Error::new(
                    "E_GRAPHICS_UNAVAILABLE",
                    7,
                    "the graphics device became unavailable",
                ));
            }
            if ran == 0 {
                if let Some(next) = self.pipeline.parked_deadline() {
                    let now = self.pipeline.now();
                    self.pipeline.advance(next.saturating_duration_since(now));
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        Err(Error::new(
            "E_READINESS_TIMEOUT",
            5,
            "calibration exhausted its native pump bound without a presented frame",
        ))
    }

    pub fn encode_png(&self, frame: &StaticFrame, opaque: bool) -> Result<Vec<u8>, Error> {
        capture::png(&frame.0.pixels, opaque)
    }

    pub fn metadata(&self) -> Value {
        self.capture.metadata()
    }
}

/// Runs a static view through the shared RAII headless pipeline and cleans up before returning.
pub(crate) fn render_static<V, T>(
    profile: &LoadedProfile,
    title: &str,
    stylesheet: String,
    map_err: fn(AppError) -> Error,
    mut build_view: impl FnMut() -> V + 'static,
    run: impl FnOnce(&mut StaticPreview) -> Result<T, Error>,
) -> Result<T, Error>
where
    V: IntoView,
{
    let capture = CaptureHandle::default();
    let handle = capture.clone();
    let surface = &profile.profile.surface;
    let app = configure_app(
        &profile.fonts,
        surface.width,
        surface.height,
        surface.opaque,
        title,
        stylesheet,
        capture::renderer_factory(move |_| handle.clone()),
    );
    let runtime = app
        .into_handler(move |cx| Box::new(build_view().into_view().build(cx)))
        .map_err(map_err)?;
    let mut preview = StaticPreview {
        pipeline: Pipeline::from_harness(Harness::new(runtime)),
        capture,
        map_err,
    };
    run(&mut preview)
}

struct OwnedFrame {
    configuration: u64,
    native_nodes: Vec<ui_bridge::registry::SnapshotNode>,
    presentation: u64,
    metadata_revision: u64,
    frame: FrameEvidence,
    tree: TreeEvidence,
}

struct ObserverData {
    requested_metadata: u64,
    semantics: SemanticModel,
    ledger: FrameLedger,
    session: Weak<Session>,
    surface: Option<Arc<OffscreenSurface>>,
    last: Option<OwnedFrame>,
}

struct ObservationContext {
    sink: RefCell<Box<dyn CheckpointSink>>,
    build_id: String,
    environment: Rc<Environment>,
    multiple: bool,
    action: Cell<u64>,
    frames: Cell<u64>,
    current_checkpoint: Cell<Option<SurfaceId>>,
    graphics: RefCell<Value>,
    fault: RefCell<Option<Error>>,
}

struct ObserverInner {
    capture: CaptureHandle,
    context: Rc<ObservationContext>,
    data: RefCell<ObserverData>,
}

struct ObserverRegistry {
    context: Rc<ObservationContext>,
    main: Option<FrameObserver>,
    next_is_main: Cell<bool>,
    surfaces: RefCell<BTreeMap<SurfaceId, FrameObserver>>,
}

/// Which native surfaces contribute frame evidence for the preview.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ObservationMode {
    PrimaryOnly,
    PrimaryAndShell,
    ShellOnly,
}

/// Routes renderer and frame callbacks by native surface identity.
#[derive(Clone)]
pub(crate) struct FrameObservers(Rc<ObserverRegistry>);

impl FrameObservers {
    pub fn new(build_id: String, environment: Rc<Environment>, sink: Box<dyn CheckpointSink>, mode: ObservationMode) -> Self {
        let has_primary = !matches!(mode, ObservationMode::ShellOnly);
        let context = Rc::new(ObservationContext {
            sink: RefCell::new(sink), build_id, environment, multiple: !matches!(mode, ObservationMode::PrimaryOnly),
            action: Cell::new(0), frames: Cell::new(0), current_checkpoint: Cell::new(None),
            graphics: RefCell::new(Value::Null), fault: RefCell::new(None),
        });
        let main = has_primary.then(|| FrameObserver::new(context.clone()));
        Self(Rc::new(ObserverRegistry { context, main, next_is_main: Cell::new(has_primary), surfaces: RefCell::default() }))
    }

    fn register_renderer(&self, surface: SurfaceId) -> CaptureHandle {
        let mut surfaces = self.0.surfaces.borrow_mut();
        surfaces.entry(surface).or_insert_with(|| {
            if self.0.next_is_main.replace(false) {
                self.0.main.as_ref().expect("registered main observer").clone()
            } else {
                FrameObserver::new(self.0.context.clone())
            }
        }).0.capture.clone()
    }

    pub fn get(&self, surface: SurfaceId) -> Option<FrameObserver> {
        self.0.surfaces.borrow().get(&surface).cloned()
    }

    pub fn retain(&self, windows: &[Window]) {
        self.0.surfaces.borrow_mut().retain(|id, _| windows.iter().any(|window| window.surface().id() == *id));
    }

    pub fn reset(&self) {
        self.0.surfaces.borrow_mut().clear();
        if let Some(main) = &self.0.main {
            main.reset_surface();
            self.0.next_is_main.set(true);
        }
    }

    pub fn begin_step(&self, action: u64) -> u64 {
        self.0.context.action.set(action);
        self.frames()
    }

    pub fn frames(&self) -> u64 { self.0.context.frames.get() }
    pub fn take_fault(&self) -> Option<Error> { self.0.context.fault.borrow_mut().take() }
    pub fn device_unavailable(&self) -> bool {
        self.0.surfaces.borrow().values().any(FrameObserver::device_unavailable)
    }
    pub fn graphics_metadata(&self) -> Value {
        if let Some(main) = &self.0.main { main.0.capture.metadata() }
        else { self.0.context.graphics.borrow().clone() }
    }
    pub fn latest_checkpoint(&self) -> Option<SurfaceId> { self.0.context.current_checkpoint.get() }
}

impl FrameProbe for FrameObservers {
    fn frame_ended(&self, window: &Window) {
        let sequence = self.frames() + 1;
        self.0.context.frames.set(sequence);
        if let Some(observer) = self.get(window.surface().id()) {
            observer.0.on_frame_ended(window, sequence);
        }
    }
}

impl ObserverInner {
    fn on_frame_ended(&self, window: &Window, sequence: u64) {
        let Some(observation) = window.completed_frame() else {
            return;
        };
        if self.context.fault.borrow().is_some() { return; }
        let data = &mut *self.data.borrow_mut();
        let Some(session) = data.session.upgrade() else { return; };
        let Some(region) = session.mount_region(window.document_id()) else { return; };
        if region.surface != window.surface().id() { return; }
        if let Some(surface) = data.surface.as_ref()
            && let Err(error) = data.semantics.consume(surface, window)
        {
            self.context.fault.replace(Some(error));
            return;
        }
        let state = self.capture.state();
        let time_us =
            u64::try_from(observation.timestamp.since_origin().as_micros()).unwrap_or(u64::MAX);
        let input = FrameInput {
            ordinal: observation.ordinal,
            time_us,
            layout_document_coherent: observation.layout_document_coherent,
            status: FrameStatus::from(observation.outcome),
            outcome_debug: format!("{:?}", observation.outcome),
            draw_id: state.draw_id,
            presentation_id: state.presentation_id,
            configuration_id: state.configuration_id,
            presented_draw: state.presented_draw,
            style_revision: observation.style_revision,
            document_revision: observation.document_revision,
            metadata_revision: region.bridge.metadata_revision(),
            resource_revision: window.image_resource_revision(),
            generation: session.generation,
        };
        let Some((stamp, presentation)) = data.ledger.record_outcome(&input) else {
            return;
        };
        if data.surface.is_none() { return; }
        let resources = match resources::snapshot(window, &[], Some(&session)) {
            Ok(resources) => resources,
            Err(error) => {
                self.context.fault.replace(Some(error));
                return;
            }
        };
        let surface_configuration = data.ledger.surface_configuration();
        let resource_revision = window.image_resource_revision();
        let identity = CaptureIdentity {
            build_id: self.context.build_id.clone(),
            presentation_sequence: state.presentation_id,
            resource_revision: Some(resource_revision),
            surface_configuration: Some(surface_configuration),
            style_revision: Some(observation.style_revision),
        };
        let snapshot =
            inspect::snapshot(window, &region, session.generation, observation.ordinal, &data.semantics, &identity);
        let mut tree = snapshot.tree;
        let mut extra = serde_json::Map::new();
        for (key, value) in [
            ("documentIdentity", json!(format!("{:?}", window.document_id()))),
            ("actionSequence", json!(self.context.action.get())),
            ("requiredStateRevision", json!(observation.document_revision)),
            ("validatedStateRevision", json!(observation.document_revision)),
            ("layoutRevision", json!(observation.layout_revision)),
            ("configurationId", json!(state.configuration_id)),
            ("timingDeterministic", json!(self.context.environment.is_virtual())),
            ("resources", resources),
            ("scale", json!(window.surface().scale_factor())),
            ("renderer", self.capture.metadata()),
        ] {
            extra.insert(key.to_owned(), value);
        }
        if self.context.multiple {
            tree.extra.insert("checkpointSequence".into(), json!(sequence));
            extra.insert("checkpointSequence".into(), json!(sequence));
        }
        let frame = FrameEvidence {
            schema_version: 1,
            build_id: self.context.build_id.clone(),
            generation: session.generation,
            surface: region.name.clone(),
            metadata_revision: snapshot.metadata_revision,
            resource_revision: Some(resource_revision),
            surface_configuration: Some(surface_configuration),
            style_revision: Some(observation.style_revision),
            validation_frame: stamp,
            pixel_presentation: presentation,
            layout_document_coherent: true,
            capture_id: None,
            tree_digest: None,
            png_digest: None,
            extra,
        };
        // Persist complete owned evidence before another callback can fail or block.
        // Do not advance committed metadata or `last` if persistence fails.
        let (tree_path, frame_path) = evidence::checkpoint_paths(
            session.generation, observation.ordinal, (region.name != "main").then_some(sequence));
        let publication = (|| {
            let tree_value =
                serde_json::to_value(&tree).map_err(|e| Error::new("E_OUTPUT_IO", 8, e.to_string()))?;
            let frame_value =
                serde_json::to_value(&frame).map_err(|e| Error::new("E_OUTPUT_IO", 8, e.to_string()))?;
            self.context.sink
                .borrow_mut()
                .write_checkpoint(&tree_path, &tree_value, &frame_path, &frame_value)
        })();
        if let Err(error) = publication {
            self.context.fault.replace(Some(error));
            return;
        }
        data.ledger.commit(CommittedMeta {
            generation: session.generation,
            document_revision: observation.document_revision,
            metadata_revision: snapshot.metadata_revision,
            resource_revision,
            style_revision: observation.style_revision,
            surface_configuration,
            configuration_id: state.configuration_id,
            presentation_id: state.presentation_id,
            ordinal: observation.ordinal,
            time_us,
        });
        data.last = Some(OwnedFrame {
            configuration: state.configuration_id,
            presentation: state.presentation_id,
            metadata_revision: snapshot.metadata_revision,
            frame,
            tree,
            native_nodes: snapshot.native_nodes,
        });
        self.context.current_checkpoint.set(Some(region.surface));
        if self.context.graphics.borrow().is_null() {
            self.context.graphics.replace(self.capture.metadata());
        }
    }
}

/// Coherence-certified read-only witness proving `SettledFrame`'s tree, frame, and native nodes
/// match the current live window, session, and GPU presentation.
pub(crate) struct SettledFrame<'a> {
    data: Ref<'a, ObserverData>,
}

impl SettledFrame<'_> {
    fn owned(&self) -> &OwnedFrame {
        self.data.last.as_ref().expect("settled witness holds owned frame")
    }

    pub fn tree(&self) -> &TreeEvidence {
        &self.owned().tree
    }

    pub fn metadata_revision(&self) -> u64 {
        self.owned().metadata_revision
    }

    pub fn time_us(&self) -> u64 {
        self.owned().frame.validation_frame.time_us
    }

    pub fn validation_frame(&self) -> &FrameStamp {
        &self.owned().frame.validation_frame
    }

    pub fn native_node(&self, index: usize) -> Option<NodeId> {
        self.owned().native_nodes.get(index).map(|node| node.native)
    }

    pub fn node_by_document_key(
        &self,
        key: zgui_dom::NodeKey,
    ) -> Option<&evidence::NodeEvidence> {
        let owned = self.owned();
        let idx = owned
            .native_nodes
            .iter()
            .position(|node| zgui_view_dom::id::to_document(node.native) == Some(key))?;
        owned.tree.nodes.get(idx)
    }
}

/// Encapsulates frame observation, semantic consumption, readback identity, and checkpoint writes.
#[derive(Clone)]
pub(crate) struct FrameObserver(Rc<ObserverInner>);

impl FrameObserver {
    fn new(context: Rc<ObservationContext>) -> Self {
        Self(Rc::new(ObserverInner {
            capture: CaptureHandle::default(),
            context,
            data: RefCell::new(ObserverData {
                requested_metadata: 0,
                semantics: SemanticModel::default(),
                ledger: FrameLedger::default(),
                session: Weak::new(),
                surface: None,
                last: None,
            }),
        }))
    }

    pub fn bind_session(&self, session: Weak<Session>) {
        self.0.data.borrow_mut().session = session;
    }

    pub fn request_metadata_frame(&self, window: &Window, revision: u64) {
        let mut data = self.0.data.borrow_mut();
        if data.requested_metadata != revision {
            data.requested_metadata = revision;
            window.request_frame();
        }
    }

    pub fn bind_surface(&self, surface: Arc<OffscreenSurface>) {
        self.0.data.borrow_mut().surface = Some(surface);
    }

    pub fn reset_surface(&self) {
        let mut data = self.0.data.borrow_mut();
        data.session = Weak::new();
        data.surface = None;
        data.requested_metadata = 0;
        data.semantics = SemanticModel::default();
        data.last = None;
        data.ledger.reset_surface();
        self.0.capture.retire();
    }

    pub fn bump_surface_configuration(&self) {
        self.0.data.borrow_mut().ledger.bump_surface_configuration();
    }

    pub fn last_time_us(&self) -> Option<u64> {
        self.0.data.borrow().ledger.committed_time_us()
    }

    pub fn device_unavailable(&self) -> bool {
        matches!(
            self.0.capture.last_outcome(),
            Some(FrameOutcome::Skipped(SkipReason::DeviceUnavailable))
        )
    }

    pub fn settled<'a>(
        &'a self,
        window: &Window,
        session: Option<&Rc<Session>>,
    ) -> Option<SettledFrame<'a>> {
        let session = session?;
        let region = session.mount_region(window.document_id())?;
        let data = self.0.data.borrow();
        if data.surface.as_ref()?.id() != region.surface { return None; }
        let capture_state = self.0.capture.state();
        let completed = window.completed_frame();
        if window.has_pending_stylesheets() { return None; }
        let query = SettledQuery {
            generation: session.generation,
            document_revision: window.dom().revision(),
            metadata_revision: region.bridge.metadata_revision(),
            resource_revision: window.image_resource_revision(),
            style_revision: window.style_revision(),
            configuration_id: capture_state.configuration_id,
            presentation_id: capture_state.presentation_id,
            presented_draw: capture_state.presented_draw,
            completed_coherent: completed
                .as_ref()
                .is_some_and(|frame| frame.layout_document_coherent),
            completed_ordinal: completed.as_ref().map(|frame| frame.ordinal),
            completed_status: completed.as_ref().map(|frame| FrameStatus::from(frame.outcome)),
        };
        if data.ledger.is_settled(&query) && data.last.is_some() {
            Some(SettledFrame { data })
        } else {
            None
        }
    }

    pub fn write_capture(
        &self,
        settled: &SettledFrame<'_>,
        window: &Window,
        request: CaptureRequest<'_>,
    ) -> Result<Value, Fault> {
        let owned = settled.owned();
        let pixels = self.0.capture.read_current(window.renderer())?;
        if pixels.pixels.size().width != request.dimensions.0 as i32
            || pixels.pixels.size().height != request.dimensions.1 as i32
        {
            return Err(Error::new("E_READBACK", 7, "presented extent differs from current surface configuration").into());
        }
        let png = capture::png(&pixels.pixels, request.opaque)?;
        if owned.presentation != pixels.presentation_id || owned.configuration != pixels.configuration_id {
            return Err(Error::new(
                "E_CAPTURE_NOT_READY",
                5,
                "pixel presentation and owned tree have different identities",
            )
            .into());
        }
        let paths = CapturePaths::new(request.id, &owned.frame.surface).map_err(Fault::from)?;
        let mut frame = owned.frame.clone();
        frame.capture_id = Some(paths.id().to_owned());
        frame.extra.insert("readiness".into(), request.readiness);
        frame.extra.insert("resources".into(), request.resource);
        frame.extra.insert("allowedPendingAssets".into(), json!(request.allowed_pending));
        frame.extra.insert("physicalWidth".into(), json!(pixels.pixels.size().width));
        frame.extra.insert("physicalHeight".into(), json!(pixels.pixels.size().height));
        frame.png_digest = Some(output::digest(&png));
        let tree_value =
            serde_json::to_value(&owned.tree).map_err(|e| Error::new("E_OUTPUT_IO", 8, e.to_string()))?;
        let tree_bytes = output::json_bytes(&tree_value)?;
        frame.tree_digest = Some(output::digest(&tree_bytes));
        let frame_value =
            serde_json::to_value(&frame).map_err(|e| Error::new("E_OUTPUT_IO", 8, e.to_string()))?;
        self.0
            .context.sink
            .borrow_mut()
            .write_capture(&paths, &png, &tree_bytes, &frame_value)?;
        Ok(paths.summary())
    }

    pub fn publish_failure_evidence(
        &self,
        window: Option<&Window>,
        opaque: bool,
        readiness: Value,
        satisfies: bool,
    ) -> Result<Value, Error> {
        let data = self.0.data.borrow();
        let Some(last) = data.last.as_ref() else {
            return Ok(json!({"available": false}));
        };
        // An empty post-fault document is not publishable evidence; recovery
        // selects the last complete checkpoint instead.
        if last.tree.nodes.is_empty() {
            return Ok(json!({"available": false}));
        }
        last.tree.validate_topology()?;
        // Read only an already presented buffer; never pump or enter application code.
        let pixels = window
            .and_then(|window| self.0.capture.read_current(window.renderer()).ok())
            .filter(|pixels| pixels.presentation_id == last.presentation && pixels.configuration_id == last.configuration)
            .map(|pixels| capture::png(&pixels.pixels, opaque))
            .transpose()?;
        self.0.context.sink.borrow_mut().publish_historical(
            &last.tree,
            &last.frame,
            pixels.as_deref(),
            HistoricalStatus {
                satisfies_failure_revision: Some(satisfies),
                readiness: Some(readiness),
                tree_role: "failure-tree",
            },
        )
    }
}
