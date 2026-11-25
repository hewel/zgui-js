//! Native frame-synchronous background blur resolution, visibility/opacity gating,
//! single effective writer transitions, idle metadata updates, bootstrap suppression,
//! and held/failed handover.

mod support;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zgui_atlas::TextureSink;
use zgui_bits::DamageSet;
use zgui_geom::{Css, CssPx, Device, DevicePx, Edges, Point, Rect, Size};
use zgui_interned::{ClassName, ElementName};
use zgui_platform::{
    Anchor as PlatformAnchor, AppHandler, BlurRegion, Clipboard, Clock, ColorScheme, Constrain,
    CursorStyle, Decorations, FullscreenMode, InputRegion, LayerPlacement, MonitorInfo,
    PlatformCapabilities, PlatformCx, PlatformError, PopupPlacement, SessionLockState, Surface,
    SurfaceAttributes, SurfaceEvent, SurfaceId, SurfaceRole, TextInput, Waker,
};
use zgui_platform_headless::{Harness, Headless, RecordedBlurRegion};
use zgui_render::{
    ExternalTexture, FrameOutcome, FrameStats, MemoryReport, RenderCapabilities, RenderTarget,
    Renderer, SkipReason, TextureHandle,
};
use zgui_runtime::{App, BlurRegistration, PopupAutoHeight, Runtime, WindowContent, WindowSpec};
use zgui_scene::Scene;
use zgui_view::{Anchor, BuildCx, Dom, DomHandle, IntoView, NodeId, View};

struct SingleNode(NodeId);

impl Anchor for SingleNode {
    fn mount(&mut self, dom: &DomHandle, parent: NodeId, before: Option<NodeId>) {
        dom.insert(parent, self.0, before);
    }

    fn unmount(&mut self, dom: &DomHandle) {
        dom.detach(self.0);
    }

    fn first_node(&self) -> Option<NodeId> {
        Some(self.0)
    }
}

struct BuildClosure<F>(Option<F>);

impl<F: FnOnce(&mut BuildCx<'_>) -> NodeId + 'static> View for BuildClosure<F> {
    type State = SingleNode;

    fn build(mut self, cx: &mut BuildCx<'_>) -> Self::State {
        SingleNode(self.0.take().expect("build once")(cx))
    }

    fn rebuild(self, _state: &mut Self::State, _cx: &mut BuildCx<'_>) {}
}

#[derive(Default)]
struct ControlledGate {
    hold: Cell<bool>,
    held: Cell<bool>,
    next_outcome: Cell<Option<FrameOutcome>>,
}

struct ControlledRenderer {
    inner: zgui_testkit_scene::CaptureRenderer,
    gate: Rc<ControlledGate>,
    surface: Arc<dyn Surface>,
}

impl Renderer for ControlledRenderer {
    fn capabilities(&self) -> RenderCapabilities {
        self.inner.capabilities()
    }

    fn configure(&mut self, target: RenderTarget) {
        Renderer::configure(&mut self.inner, target);
    }

    fn target(&self) -> Option<RenderTarget> {
        Renderer::target(&self.inner)
    }

    fn draw(&mut self, scene: &Scene, damage: &DamageSet) -> FrameOutcome {
        let drawn = self.inner.draw(scene, damage);
        if self.gate.hold.get() {
            self.gate.held.set(true);
            return FrameOutcome::Skipped(SkipReason::Held);
        }
        if let Some(outcome) = self.gate.next_outcome.take() {
            return outcome;
        }
        if damage.is_empty() {
            return FrameOutcome::Skipped(SkipReason::Undamaged);
        }
        let size = self.surface.size();
        self.surface.pre_present_frame(
            Size::new(size.width.0.round() as i32, size.height.0.round() as i32),
            self.surface.scale_factor(),
        );
        drawn
    }

    fn present_held(&mut self) -> FrameOutcome {
        if !self.gate.held.replace(false) {
            return FrameOutcome::Skipped(SkipReason::Undamaged);
        }
        if let Some(outcome) = self.gate.next_outcome.take() {
            return outcome;
        }
        let size = self.surface.size();
        self.surface.pre_present_frame(
            Size::new(size.width.0.round() as i32, size.height.0.round() as i32),
            self.surface.scale_factor(),
        );
        FrameOutcome::Presented(FrameStats::default())
    }

    fn register_external(&mut self, texture: ExternalTexture) -> TextureHandle {
        self.inner.register_external(texture)
    }

    fn release_external(&mut self, handle: TextureHandle) {
        self.inner.release_external(handle);
    }

    fn memory(&self) -> MemoryReport {
        self.inner.memory()
    }

    fn texture_sink(&mut self) -> &mut dyn TextureSink {
        self.inner.texture_sink()
    }
}

fn controlled_harness(
    css: &str,
    build: impl FnOnce(&mut BuildCx<'_>) -> NodeId + 'static,
) -> (Harness<Runtime>, Rc<ControlledGate>) {
    let gate = Rc::new(ControlledGate::default());
    let handed = Rc::clone(&gate);
    let build_cell = Rc::new(RefCell::new(Some(build)));
    let handler = App::new()
        .with_title("blur-test")
        .with_size(200.0, 200.0)
        .with_stylesheet(css)
        .with_renderer(Box::new(move |surface, _size| {
            Ok(Box::new(ControlledRenderer {
                inner: zgui_testkit_scene::CaptureRenderer::new(),
                gate: Rc::clone(&handed),
                surface: Arc::clone(surface),
            }))
        }))
        .into_handler(move |cx| {
            let f = build_cell.borrow_mut().take().expect("build view once");
            Box::new(BuildClosure(Some(f)).build(cx))
        })
        .expect("install runtime");
    let platform = Headless::new();
    (Harness::over(platform, handler), gate)
}

#[test]
fn rounded_node_blur_visibility_opacity_and_clip_path_gates() {
    let css = r#"
        :root { display: block; width: 100%; height: 100%; margin: 0; padding: 0; }
        box { display: block; }
        .root { width: 200px; height: 200px; }
        .hidden-parent { visibility: hidden; opacity: 0.8; width: 120px; height: 80px; }
        .visible-child {
            visibility: visible;
            width: 100px;
            height: 60px;
            border-radius: 16px;
            --zgui-corner-shape: superellipse(2.83);
            background-color: rgba(20, 20, 30, 0.42);
        }
        .clipped-parent { clip-path: inset(4px); width: 100px; height: 60px; }
        .clipped-child { width: 80px; height: 40px; }
    "#;
    let visible_child_id = Rc::new(Cell::new(None));
    let clipped_child_id = Rc::new(Cell::new(None));
    let hidden_parent_id = Rc::new(Cell::new(None));
    let vc = Rc::clone(&visible_child_id);
    let cc = Rc::clone(&clipped_child_id);
    let hp = Rc::clone(&hidden_parent_id);

    let (mut harness, _gate) = controlled_harness(css, move |cx| {
        let dom = cx.dom();
        let root = dom.create_element(ElementName::new("box"));
        dom.set_classes(root, &[ClassName::new("root")]);

        let parent = dom.create_element(ElementName::new("box"));
        dom.set_classes(parent, &[ClassName::new("hidden-parent")]);
        let child = dom.create_element(ElementName::new("box"));
        dom.set_classes(child, &[ClassName::new("visible-child")]);
        dom.insert(parent, child, None);
        dom.insert(root, parent, None);

        let cp = dom.create_element(ElementName::new("box"));
        dom.set_classes(cp, &[ClassName::new("clipped-parent")]);
        let cchild = dom.create_element(ElementName::new("box"));
        dom.set_classes(cchild, &[ClassName::new("clipped-child")]);
        dom.insert(cp, cchild, None);
        dom.insert(root, cp, None);

        hp.set(Some(parent));
        vc.set(Some(child));
        cc.set(Some(cchild));
        root
    });

    harness.pump();
    let surface_id = harness.app().windows()[0].surface().id();
    let surface = harness.platform().offscreen(surface_id).expect("offscreen");
    let child_key = zgui_view_dom::id::to_document(visible_child_id.get().unwrap()).unwrap();
    let clipped_key = zgui_view_dom::id::to_document(clipped_child_id.get().unwrap()).unwrap();
    let parent_node = hidden_parent_id.get().unwrap();

    // Register visible child under hidden parent (effective opacity = 0.8 >= 0.5, rgba alpha 0.42 ignored).
    let slot = harness.app().windows()[0].blur_slot();
    slot.set(BlurRegistration::Nodes {
        nodes: vec![child_key],
        min_opacity: 0.5,
    });
    assert!(harness.app().windows()[0].blur_pending());
    harness.pump();

    assert!(!harness.app().windows()[0].blur_pending());
    let RecordedBlurRegion::Rects(rects) = surface.blur_region() else {
        panic!("expected rounded strip rects, got {:?}", surface.blur_region());
    };
    assert!(
        rects.len() > 1,
        "rounded superellipse card must emit multiple horizontal strips"
    );

    // Idle metadata-only minOpacity raise (0.8 < 0.9) clears blur without restyling/relayouting.
    slot.set(BlurRegistration::Nodes {
        nodes: vec![child_key],
        min_opacity: 0.9,
    });
    harness.pump();
    let after_report = harness.app().windows()[0].last_frame_report().unwrap();
    assert_eq!(after_report.restyled, 0, "minOpacity update must not trigger restyle");
    assert_eq!(after_report.outcome, FrameOutcome::Skipped(SkipReason::Undamaged));
    assert_eq!(surface.blur_region(), RecordedBlurRegion::None);
    assert!(!harness.app().windows()[0].blur_pending());

    // Lowering minOpacity back to 0.0 restores blur (effective opacity = 0.8 > 0).
    slot.set(BlurRegistration::Nodes {
        nodes: vec![child_key],
        min_opacity: 0.0,
    });
    harness.pump();
    assert!(matches!(surface.blur_region(), RecordedBlurRegion::Rects(_)));

    // Setting parent opacity to 0 skips even at minOpacity == 0.0.
    harness.app().windows()[0]
        .dom()
        .set_style_text(parent_node, Some("opacity: 0;"));
    harness.app().windows()[0].request_frame();
    harness.pump();
    assert_eq!(surface.blur_region(), RecordedBlurRegion::None);

    // Ancestor clip-path conservatively omits the target.
    slot.set(BlurRegistration::Nodes {
        nodes: vec![clipped_key],
        min_opacity: 0.5,
    });
    harness.pump();
    assert_eq!(surface.blur_region(), RecordedBlurRegion::None);
}

#[test]
fn min_opacity_zero_handles_deep_chain_underflow_and_animated_ancestor_opacity_crossings() {
    let css = r#"
        @keyframes fade-dip {
            0% { opacity: 0.9; }
            50% { opacity: 0.2; }
            100% { opacity: 0.9; }
        }
        :root { display: block; width: 100%; height: 100%; margin: 0; padding: 0; }
        box { display: block; }
        .root { width: 200px; height: 200px; }
        .fade-parent {
            width: 80px;
            height: 40px;
            animation: fade-dip 100ms linear forwards;
        }
        .fade-child {
            width: 80px; height: 40px; border-radius: 10px;
            --zgui-corner-shape: superellipse(2.83);
        }
        .dim { opacity: 0.0001; width: 60px; height: 30px; }
        .deep-leaf { width: 60px; height: 30px; }
    "#;
    let fade_child_id = Rc::new(Cell::new(None));
    let deep_leaf_id = Rc::new(Cell::new(None));
    let fc = Rc::clone(&fade_child_id);
    let dl = Rc::clone(&deep_leaf_id);

    let (mut harness, _gate) = controlled_harness(css, move |cx| {
        let dom = cx.dom();
        let root = dom.create_element(ElementName::new("box"));
        dom.set_classes(root, &[ClassName::new("root")]);

        let fade_parent = dom.create_element(ElementName::new("box"));
        dom.set_classes(fade_parent, &[ClassName::new("fade-parent")]);
        let fade_child = dom.create_element(ElementName::new("box"));
        dom.set_classes(fade_child, &[ClassName::new("fade-child")]);
        dom.insert(fade_parent, fade_child, None);
        dom.insert(root, fade_parent, None);
        fc.set(Some(fade_child));

        // 90 nested elements with opacity 1e-4: (1e-4)^90 = 1e-360 underflows f64 to 0.0 even
        // though every factor is strictly positive; `minOpacity: 0.0` must still keep the target
        // eligible.
        let mut cursor = root;
        for _ in 0..90 {
            let wrap = dom.create_element(ElementName::new("box"));
            dom.set_classes(wrap, &[ClassName::new("dim")]);
            dom.insert(cursor, wrap, None);
            cursor = wrap;
        }
        let leaf = dom.create_element(ElementName::new("box"));
        dom.set_classes(leaf, &[ClassName::new("deep-leaf")]);
        dom.insert(cursor, leaf, None);
        dl.set(Some(leaf));
        root
    });

    harness.pump();
    let surface_id = harness.app().windows()[0].surface().id();
    let surface = harness.platform().offscreen(surface_id).expect("offscreen");
    let fade_key = zgui_view_dom::id::to_document(fade_child_id.get().unwrap()).unwrap();
    let leaf_key = zgui_view_dom::id::to_document(deep_leaf_id.get().unwrap()).unwrap();
    let slot = harness.app().windows()[0].blur_slot();

    // At t=0: animated parent opacity = 0.9 >= 0.5 -> blurred.
    slot.set(BlurRegistration::Nodes {
        nodes: vec![fade_key],
        min_opacity: 0.5,
    });
    harness.pump();
    assert!(
        matches!(surface.blur_region(), RecordedBlurRegion::Rects(_)),
        "expected blur at animated opacity 0.9, got {:?}",
        surface.blur_region()
    );

    // At t=50ms: animated parent opacity = 0.2 < 0.5 -> cleared.
    harness.advance(Duration::from_millis(50));
    harness.pump();
    assert_eq!(
        surface.blur_region(),
        RecordedBlurRegion::None,
        "animated ancestor opacity below 0.5 must drop blur"
    );

    // Re-resolve the same layout and animation sample without any DOM or registration write.
    harness.platform_mut().set_blur_shapes_supported(true);
    assert!(harness.app().windows()[0].blur_pending());
    harness.pump();
    let RecordedBlurRegion::Shapes(shapes) = surface.blur_region() else {
        panic!("continuous blur below minOpacity was lost");
    };
    assert_eq!(shapes.len(), 1);
    assert_eq!(shapes[0].opacity, 51.0 / 256.0);
    assert_eq!(shapes[0].corner_exponent, 724.0 / 256.0);
    assert_eq!(shapes[0].radii, [10.0; 4]);
    assert!(!harness.app().windows()[0].blur_pending());
    harness.platform_mut().set_blur_shapes_supported(false);
    harness.pump();
    assert_eq!(surface.blur_region(), RecordedBlurRegion::None);

    // At t=100ms: animated parent opacity returns to 0.9 >= 0.5 -> restored!
    harness.advance(Duration::from_millis(50));
    harness.pump();
    assert!(
        matches!(surface.blur_region(), RecordedBlurRegion::Rects(_)),
        "animated ancestor opacity returning above 0.5 must restore blur, got {:?}",
        surface.blur_region()
    );

    // Deep positive opacity chain (0.2^500 underflows f64 to 0.0): kept at minOpacity == 0.0,
    // dropped at minOpacity > 0.0.
    slot.set(BlurRegistration::Nodes {
        nodes: vec![leaf_key],
        min_opacity: 0.0,
    });
    harness.pump();
    assert!(
        matches!(surface.blur_region(), RecordedBlurRegion::Rects(_)),
        "minOpacity=0.0 must not underflow-drop deep positive opacity chains"
    );

    slot.set(BlurRegistration::Nodes {
        nodes: vec![leaf_key],
        min_opacity: 1e-12,
    });
    harness.pump();
    assert_eq!(surface.blur_region(), RecordedBlurRegion::None);
}

#[test]
fn held_frame_and_failed_present_never_publish_or_mark_applied_early() {
    let css = r#"
        :root { display: block; width: 100%; height: 100%; margin: 0; padding: 0; }
        box { display: block; }
        .root { width: 200px; height: 200px; }
        .card { width: 80px; height: 40px; }
    "#;
    let card_id = Rc::new(Cell::new(None));
    let cid = Rc::clone(&card_id);
    let (mut harness, gate) = controlled_harness(css, move |cx| {
        let dom = cx.dom();
        let root = dom.create_element(ElementName::new("box"));
        dom.set_classes(root, &[ClassName::new("root")]);
        let card = dom.create_element(ElementName::new("box"));
        dom.set_classes(card, &[ClassName::new("card")]);
        dom.insert(root, card, None);
        cid.set(Some(card));
        root
    });

    harness.pump();
    let surface_id = harness.app().windows()[0].surface().id();
    let surface = harness.platform().offscreen(surface_id).expect("offscreen");
    let card_key = zgui_view_dom::id::to_document(card_id.get().unwrap()).unwrap();
    let slot = harness.app().windows()[0].blur_slot();

    // Hold the next frame while registering a node target + causing damage.
    gate.hold.set(true);
    slot.set(BlurRegistration::Nodes {
        nodes: vec![card_key],
        min_opacity: 0.5,
    });
    harness.app().windows()[0]
        .dom()
        .set_style_text(card_id.get().unwrap(), Some("background-color: #224466;"));
    harness.pump();

    // Held frame must NOT have published blur to the surface yet.
    assert_eq!(surface.blur_region(), RecordedBlurRegion::None);
    assert!(harness.app().windows()[0].blur_pending());

    // Delivering PresentReady presents the held buffer and commits its staged blur together.
    gate.hold.set(false);
    harness.deliver(surface_id, SurfaceEvent::PresentReady);
    harness.pump();
    let expected = RecordedBlurRegion::Rects(vec![Rect::new(
        Point::new(CssPx(0.0), CssPx(0.0)),
        Size::new(CssPx(80.0), CssPx(40.0)),
    )]);
    assert_eq!(surface.blur_region(), expected);
    assert!(!harness.app().windows()[0].blur_pending());

    // Mode transition: Nodes -> Full -> Fixed Rect -> None (single effective writer).
    slot.set(BlurRegistration::Full);
    harness.pump();
    assert_eq!(surface.blur_region(), RecordedBlurRegion::Full);

    let fixed = vec![Rect::new(
        Point::new(CssPx(10.0), CssPx(12.0)),
        Size::new(CssPx(50.0), CssPx(30.0)),
    )];
    slot.set(BlurRegistration::Rects(fixed.clone()));
    harness.pump();
    assert_eq!(surface.blur_region(), RecordedBlurRegion::Rects(fixed.clone()));

    // Failed acquisition on switching to None must roll back / not mark applied.
    gate.next_outcome
        .set(Some(FrameOutcome::Skipped(SkipReason::Unacquired)));
    slot.set(BlurRegistration::None);
    harness.app().windows()[0]
        .dom()
        .set_style_text(card_id.get().unwrap(), Some("background-color: #335577;"));
    harness.pump();
    assert_eq!(surface.blur_region(), RecordedBlurRegion::Rects(fixed));
    assert!(harness.app().windows()[0].blur_pending());

    // Next presented frame applies None cleanly.
    harness.advance(Duration::from_millis(20));
    harness.pump();
    assert_eq!(surface.blur_region(), RecordedBlurRegion::None);

    // Hold an exact union, then withdraw shape support before its buffer arrives.
    let explicit = vec![
        Rect::new(Point::new(CssPx(-0.25), CssPx(0.0)), Size::new(CssPx(8.0), CssPx(4.0))),
        Rect::new(Point::new(CssPx(3.0), CssPx(0.0)), Size::new(CssPx(8.0), CssPx(4.0))),
    ];
    harness.platform_mut().set_blur_shapes_supported(true);
    gate.hold.set(true);
    slot.set(BlurRegistration::Rects(explicit.clone()));
    harness.pump();
    assert_eq!(surface.blur_region(), RecordedBlurRegion::None);
    harness.platform_mut().set_blur_shapes_supported(false);
    gate.hold.set(false);
    harness.deliver(surface_id, SurfaceEvent::PresentReady);
    assert_eq!(surface.blur_region(), RecordedBlurRegion::None);
    assert!(harness.app().windows()[0].blur_pending());
    harness.pump();
    assert_eq!(surface.blur_region(), RecordedBlurRegion::Rects(explicit));
    harness.platform_mut().set_blur_shapes_supported(true);
    harness.pump();
    let RecordedBlurRegion::Shapes(shapes) = surface.blur_region() else {
        panic!("mode flip did not re-resolve explicit rects");
    };
    assert_eq!(shapes.len(), 1);
    assert_eq!((shapes[0].x, shapes[0].width), (-0.25, 11.25));
    assert!(!harness.app().windows()[0].blur_pending());
}

#[test]
fn animated_transform_fast_path_updates_blur_region_frame_synchronously() {
    let css = r#"
        @keyframes slide {
            from { transform: translateX(0px); }
            to { transform: translateX(100px); }
        }
        :root { display: block; width: 100%; height: 100%; margin: 0; padding: 0; }
        box { display: block; }
        .root { width: 200px; height: 200px; }
        .mover {
            width: 40px;
            height: 40px;
            animation: slide 100ms linear forwards;
        }
    "#;
    let mover_id = Rc::new(Cell::new(None));
    let mid = Rc::clone(&mover_id);
    let (mut harness, _gate) = controlled_harness(css, move |cx| {
        let dom = cx.dom();
        let root = dom.create_element(ElementName::new("box"));
        dom.set_classes(root, &[ClassName::new("root")]);
        let mover = dom.create_element(ElementName::new("box"));
        dom.set_classes(mover, &[ClassName::new("mover")]);
        dom.insert(root, mover, None);
        mid.set(Some(mover));
        root
    });

    harness.pump();
    let surface_id = harness.app().windows()[0].surface().id();
    let surface = harness.platform().offscreen(surface_id).expect("offscreen");
    let mover_key = zgui_view_dom::id::to_document(mover_id.get().unwrap()).unwrap();
    harness.app().windows()[0]
        .blur_slot()
        .set(BlurRegistration::Nodes {
            nodes: vec![mover_key],
            min_opacity: 0.5,
        });
    harness.pump();

    let initial_x = match surface.blur_region() {
        RecordedBlurRegion::Rects(rects) => rects[0].origin.x.0,
        other => panic!("expected rect at start, got {other:?}"),
    };
    harness.advance(Duration::from_millis(50));
    harness.pump();
    let mid_x = match surface.blur_region() {
        RecordedBlurRegion::Rects(rects) => rects[0].origin.x.0,
        other => panic!("expected rect at midpoint, got {other:?}"),
    };
    assert!(
        mid_x > initial_x + 20.0,
        "animated transform fast path must move blur mask synchronously (initial={initial_x}, mid={mid_x})"
    );
}

struct AsyncBlurPopupSurface {
    inner: Arc<dyn Surface>,
    size: Mutex<Size<DevicePx, Device>>,
}

impl Surface for AsyncBlurPopupSurface {
    fn id(&self) -> SurfaceId {
        self.inner.id()
    }
    fn size(&self) -> Size<DevicePx, Device> {
        *self.size.lock().expect("size lock")
    }
    fn scale_factor(&self) -> f64 {
        self.inner.scale_factor()
    }
    fn request_size(&self, size: Size<CssPx, Css>) -> Option<Size<DevicePx, Device>> {
        self.inner.request_size(size)
    }
    fn set_min_size(&self, min: Option<Size<CssPx, Css>>) {
        self.inner.set_min_size(min);
    }
    fn set_max_size(&self, max: Option<Size<CssPx, Css>>) {
        self.inner.set_max_size(max);
    }
    fn request_redraw(&self) {
        self.inner.request_redraw();
    }
    fn pre_present_notify(&self) {
        self.inner.pre_present_notify();
    }
    fn pre_present_frame(&self, size: Size<i32, Device>, scale: f64) {
        self.inner.pre_present_frame(size, scale);
    }
    fn set_visible(&self, visible: bool) {
        self.inner.set_visible(visible);
    }
    fn set_title(&self, title: &str) {
        self.inner.set_title(title);
    }
    fn set_resizable(&self, resizable: bool) {
        self.inner.set_resizable(resizable);
    }
    fn set_decorations(&self, decorations: Decorations) {
        self.inner.set_decorations(decorations);
    }
    fn set_maximized(&self, maximized: bool) {
        self.inner.set_maximized(maximized);
    }
    fn set_minimized(&self, minimized: bool) {
        self.inner.set_minimized(minimized);
    }
    fn set_fullscreen(&self, mode: Option<FullscreenMode>) {
        self.inner.set_fullscreen(mode);
    }
    fn set_cursor(&self, cursor: CursorStyle) {
        self.inner.set_cursor(cursor);
    }
    fn logical_extent_for(&self, size: Size<i32, Device>, scale: f64) -> Size<CssPx, Css> {
        self.inner.logical_extent_for(size, scale)
    }
    fn prefers_blur_shapes(&self) -> bool {
        self.inner.prefers_blur_shapes()
    }
    fn stage_blur_region_for_present(&self, region: BlurRegion<'_>) {
        self.inner.stage_blur_region_for_present(region);
    }
    fn clear_staged_blur_region(&self) {
        self.inner.clear_staged_blur_region();
    }
    fn finalize_presented_blur(&self) {
        self.inner.finalize_presented_blur();
    }
    fn rollback_unpresented_blur(&self) {
        self.inner.rollback_unpresented_blur();
    }
    fn apply_idle_blur_region(&self, region: BlurRegion<'_>) {
        self.inner.apply_idle_blur_region(region);
    }
    fn set_shell_input_region(&self, region: &InputRegion) -> Result<(), PlatformError> {
        self.inner.set_shell_input_region(region)
    }
    fn reconfigure_layer(
        &self,
        placement: &LayerPlacement,
        size: Size<CssPx, Css>,
    ) -> Result<(), PlatformError> {
        self.inner.reconfigure_layer(placement, size)
    }
    fn reconfigure_popup(
        &self,
        _placement: &PopupPlacement,
        _size: Size<CssPx, Css>,
    ) -> Result<(), PlatformError> {
        Ok(())
    }
    fn supports_popup_reposition(&self) -> bool {
        true
    }
    fn set_text_input(&self, state: Option<TextInput>) {
        self.inner.set_text_input(state);
    }
    fn push_a11y_update(&self, build: &mut dyn FnMut() -> zgui_a11y::TreeUpdate) {
        self.inner.push_a11y_update(build);
    }
}

struct AsyncBlurPopupPlatform {
    inner: Headless,
    popup_surfaces: Mutex<Vec<Arc<AsyncBlurPopupSurface>>>,
}

impl PlatformCx for AsyncBlurPopupPlatform {
    fn create_surface(&self, attrs: &SurfaceAttributes) -> Result<Arc<dyn Surface>, PlatformError> {
        let is_popup = matches!(attrs.role, SurfaceRole::Popup(_));
        let surface = self.inner.create_surface(attrs)?;
        if !is_popup {
            return Ok(surface);
        }
        let wrapped = Arc::new(AsyncBlurPopupSurface {
            size: Mutex::new(surface.size()),
            inner: surface,
        });
        self.popup_surfaces
            .lock()
            .expect("popup surfaces")
            .push(wrapped.clone());
        Ok(wrapped)
    }
    fn destroy_surface(&self, id: SurfaceId) {
        self.inner.destroy_surface(id);
    }
    fn surfaces(&self) -> Vec<Arc<dyn Surface>> {
        self.inner
            .surfaces()
            .into_iter()
            .map(|s| self.surface(s.id()).unwrap_or(s))
            .collect()
    }
    fn surface(&self, id: SurfaceId) -> Option<Arc<dyn Surface>> {
        if let Some(found) = self
            .popup_surfaces
            .lock()
            .expect("popup surfaces")
            .iter()
            .find(|s| s.id() == id)
            .cloned()
        {
            return Some(found);
        }
        self.inner.surface(id)
    }
    fn monitors(&self) -> Vec<MonitorInfo> {
        self.inner.monitors()
    }
    fn primary_monitor(&self) -> Option<MonitorInfo> {
        self.inner.primary_monitor()
    }
    fn color_scheme(&self) -> Option<ColorScheme> {
        self.inner.color_scheme()
    }
    fn clipboard(&self) -> &dyn Clipboard {
        self.inner.clipboard()
    }
    fn capabilities(&self) -> &PlatformCapabilities {
        self.inner.capabilities()
    }
    fn clock(&self) -> Arc<dyn Clock> {
        self.inner.clock()
    }
    fn waker(&self) -> Arc<dyn Waker> {
        self.inner.waker()
    }
    fn request_exit(&self) {
        self.inner.request_exit();
    }
    fn is_exiting(&self) -> bool {
        self.inner.is_exiting()
    }
    fn session_lock_state(&self) -> SessionLockState {
        self.inner.session_lock_state()
    }
    fn request_session_lock(&self) -> Result<(), PlatformError> {
        self.inner.request_session_lock()
    }
    fn try_unlock_session(&self) -> Result<(), PlatformError> {
        self.inner.try_unlock_session()
    }
    fn unlock_session(&self) {
        self.inner.unlock_session();
    }
}

#[test]
fn suppress_first_paint_bootstrap_frame_emits_no_blur_and_preserves_registered_intent() {
    let platform = AsyncBlurPopupPlatform {
        inner: Headless::new().with_blur_capabilities(false, true),
        popup_surfaces: Mutex::new(Vec::new()),
    };
    let mut runtime = App::new()
        .with_renderer(Box::new(support::capture))
        .into_handler(|cx| Box::new(zgui_elements::column().into_view().build(cx)))
        .expect("install runtime");
    runtime.surfaces_available(&platform);
    let parent = runtime.windows()[0].surface().id();

    let placement = PopupPlacement {
        parent,
        anchor_rect: Rect::new(
            Point::new(CssPx(10.0), CssPx(10.0)),
            Size::new(CssPx(40.0), CssPx(24.0)),
        ),
        anchor: PlatformAnchor::BottomLeft,
        gravity: PlatformAnchor::BottomRight,
        offset: Point::new(CssPx(0.0), CssPx(4.0)),
        constrain: Constrain::default(),
        grab: None,
        reactive: false,
        geometry_insets: Edges::new(CssPx(0.0), CssPx(0.0), CssPx(0.0), CssPx(0.0)),
    };
    let mut attrs = SurfaceAttributes::popup(placement.clone());
    attrs.size = Some(Size::new(CssPx(320.0), CssPx(600.0)));
    attrs.transparent = true;
    let mut content = WindowContent::default();
    content.window_stylesheets.push(
        ":root { display: block; width: 100%; height: 100%; } \
         box { display: block; } \
         .panel { width: 320px; height: 120px; border-radius: 14px; background: rgba(20, 30, 40, 0.5); }"
            .into(),
    );

    let panel_id = Rc::new(Cell::new(None));
    let pid = Rc::clone(&panel_id);
    let token = runtime
        .open_surface(
            &platform,
            WindowSpec::new(
                attrs,
                content,
                Box::new(move |cx| {
                    let dom = cx.dom();
                    let panel = dom.create_element(ElementName::new("box"));
                    dom.set_classes(panel, &[ClassName::new("panel")]);
                    pid.set(Some(panel));
                    Box::new(SingleNode(panel))
                }),
            ),
        )
        .expect("open async auto-height popup");
    let popup_id = runtime.surface_id(token).expect("popup id");
    runtime
        .set_popup_auto_height(
            token,
            Some(PopupAutoHeight {
                placement,
                width: CssPx(320.0),
                min_height: CssPx(48.0),
                max_height: CssPx(600.0),
            }),
        )
        .expect("enable auto-height");

    let panel_key = zgui_view_dom::id::to_document(panel_id.get().unwrap()).unwrap();
    let slot = runtime
        .windows()
        .iter()
        .find(|w| w.surface().id() == popup_id)
        .expect("popup window")
        .blur_slot();
    slot.set(BlurRegistration::Nodes {
        nodes: vec![panel_key],
        min_opacity: 0.5,
    });

    // Frame 1: bootstrap transparent frame must NOT emit node blur yet and must keep intent pending.
    runtime.surface_event(&platform, popup_id, SurfaceEvent::RedrawRequested);
    let offscreen = platform.inner.offscreen(popup_id).expect("offscreen popup");
    assert_eq!(
        offscreen.blur_region(),
        RecordedBlurRegion::None,
        "bootstrap empty frame must not publish node blur"
    );
    assert!(
        runtime
            .windows()
            .iter()
            .find(|w| w.surface().id() == popup_id)
            .expect("popup window")
            .blur_pending(),
        "registered blur intent must remain pending across bootstrap frame"
    );

    // Compositor replies with the repositioned 320x120 size; Frame 2 paints real pixels + rounded blur.
    let popup_surface = platform
        .popup_surfaces
        .lock()
        .expect("popup surfaces")
        .first()
        .expect("wrapped popup")
        .clone();
    let settled = Size::new(DevicePx(320.0), DevicePx(120.0));
    *popup_surface.size.lock().expect("size lock") = settled;
    let _ = offscreen.request_size(Size::new(CssPx(320.0), CssPx(120.0)));
    runtime.surface_event(&platform, popup_id, SurfaceEvent::Resized(settled));
    runtime.surface_event(&platform, popup_id, SurfaceEvent::RedrawRequested);

    let RecordedBlurRegion::Shapes(shapes) = offscreen.blur_region() else {
        panic!(
            "repositioned frame must publish rounded blur mask, got {:?}",
            offscreen.blur_region()
        );
    };
    assert_eq!(shapes.len(), 1);
    assert_eq!(shapes[0].radii, [14.0; 4]);
    assert!(
        !runtime
            .windows()
            .iter()
            .find(|w| w.surface().id() == popup_id)
            .expect("popup window")
            .blur_pending()
    );
}
