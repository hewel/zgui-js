//! A frame the platform is not ready for, held by the renderer and presented on the platform's
//! word.
//!
//! The renderer answers [`SkipReason::Held`] when the platform refuses another buffer: the frame is
//! composed and its damage retires, and nothing is presented until
//! [`SurfaceEvent::PresentReady`] arrives. These cases assert the runtime's half of that contract
//! over a real window: a held frame asks for no other frame, the platform's word presents it
//! exactly once, and a window is shown only by a frame that reached its surface.

mod support;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use zgui_atlas::TextureSink;
use zgui_bits::DamageSet;
use zgui_platform::{Clock, Surface, SurfaceEvent};
use zgui_render::{
    ExternalTexture, FrameOutcome, FrameStats, MemoryReport, RenderCapabilities, RenderTarget,
    Renderer, SkipReason, TextureHandle,
};
use zgui_runtime::{App, AppError, CompletedFrame, FrameProbe, HostBinding, Runtime, Window};
use zgui_scene::Scene;
use zgui_view::{BuildCx, DomHandle, IntoView, NodeId, Observed, View};

/// What a test sets and reads about the renderer under its window.
#[derive(Default)]
struct Gate {
    /// Whether the next frames are held.
    hold: Cell<bool>,
    /// How many frames were drawn.
    drawn: Cell<u32>,
    /// Whether a drawn frame waits to be presented.
    held: Cell<bool>,
    /// How many held frames were presented.
    presented: Cell<u32>,
    /// A renderer outcome injected after it consumes the real native scene.
    next_outcome: Cell<Option<FrameOutcome>>,
    /// Frame evidence read from inside the probe, rather than reconstructed by the test.
    frames: RefCell<Vec<CompletedFrame>>,
}

impl FrameProbe for Gate {
    fn frame_ended(&self, window: &Window) {
        let frame = window.completed_frame().expect("evidence precedes the callback");
        assert_eq!(frame.document_revision, window.dom().revision());
        self.frames.borrow_mut().push(frame);
    }
}

/// A capture renderer that holds its frames while the test says so.
struct Holding {
    inner: zgui_testkit_scene::CaptureRenderer,
    gate: Rc<Gate>,
}

impl Renderer for Holding {
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
        self.gate.drawn.set(self.gate.drawn.get() + 1);
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
        drawn
    }

    fn present_held(&mut self) -> FrameOutcome {
        if !self.gate.held.replace(false) {
            return FrameOutcome::Skipped(SkipReason::Undamaged);
        }
        self.gate.presented.set(self.gate.presented.get() + 1);
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

/// A still document over a holding renderer, with nothing settled yet when `hold` is set.
fn window(hold: bool) -> (zgui_platform_headless::Harness<Runtime>, Rc<Gate>) {
    let gate = Rc::new(Gate::default());
    gate.hold.set(hold);
    let handed = Rc::clone(&gate);
    let handler = App::new()
        .with_title("held")
        .with_size(400.0, 300.0)
        .with_probe(gate.clone())
        .with_stylesheet(
            "root { display: block; width: 400px; height: 300px }
             .block { display: block; width: 100px; height: 100px;
                      background-color: rgb(200, 200, 200) }",
        )
        .with_renderer(Box::new(
            move |_surface: &Arc<dyn Surface>, target: RenderTarget| {
                let mut renderer = Holding {
                    inner: zgui_testkit_scene::CaptureRenderer::new(),
                    gate: Rc::clone(&handed),
                };
                Renderer::configure(&mut renderer, target);
                Ok::<_, AppError>(Box::new(renderer) as Box<dyn Renderer>)
            },
        ))
        .into_handler(move |cx: &mut BuildCx<'_>| {
            let view = zgui_elements::column()
                .class("root")
                .child(zgui_elements::r#box().class("block"))
                .into_view();
            Box::new(view.build(cx)) as Box<dyn zgui_view::Anchor>
        })
        .expect("the reactive runtime installs");
    let harness = zgui_platform_headless::Harness::new(handler);
    harness.platform().offscreens()[0].set_refresh_rate_millihertz(Some(60_000));
    (harness, gate)
}

#[test]
fn a_held_frame_asks_for_nothing_until_the_platform_is_ready() {
    let (mut harness, gate) = window(false);
    harness.settle(8);
    gate.hold.set(true);
    let id = harness.platform().offscreens()[0].id();
    harness.platform().offscreens()[0].request_redraw();
    harness.settle(4);
    assert_eq!(gate.drawn.get(), 2, "the held frame was drawn again");
    assert!(gate.held.get());
    assert_eq!(gate.presented.get(), 0);

    harness.deliver(id, SurfaceEvent::PresentReady);
    assert_eq!(gate.presented.get(), 1);
    harness.deliver(id, SurfaceEvent::PresentReady);
    assert_eq!(
        gate.presented.get(),
        1,
        "one held frame was presented twice"
    );
    harness.assert_park_invariant();
}

#[test]
fn a_window_is_shown_by_the_first_frame_that_reaches_its_surface() {
    let (mut harness, gate) = window(true);
    harness.settle(8);
    let surface = Arc::clone(&harness.platform().offscreens()[0]);
    assert!(gate.drawn.get() >= 1, "the first frame never ran");
    assert!(
        !surface.is_visible(),
        "a window was shown before its first frame was presented"
    );

    harness.deliver(surface.id(), SurfaceEvent::PresentReady);
    assert!(surface.is_visible());
}

#[test]
fn completed_evidence_keeps_actual_outcomes_even_when_draw_is_bypassed() {
    let (mut harness, gate) = window(false);
    assert!(harness.app().windows()[0].completed_frame().is_none());
    harness.settle(8);
    let initial = *gate.frames.borrow().last().expect("first frame");
    assert_eq!(initial.ordinal, 1);
    assert!(matches!(initial.outcome, FrameOutcome::Presented(_)));
    assert!(initial.layout_document_coherent);
    assert_eq!(initial.layout_revision, Some(initial.document_revision));
    assert_eq!(initial.timestamp, harness.platform().virtual_clock().timestamp());

    gate.hold.set(true);
    let surface = Arc::clone(&harness.platform().offscreens()[0]);
    surface.request_redraw();
    harness.settle(4);
    let held = *gate.frames.borrow().last().expect("held frame");
    assert_eq!(held.ordinal, initial.ordinal + 1);
    assert_eq!(held.outcome, FrameOutcome::Skipped(SkipReason::Held));
    harness.deliver(surface.id(), SurfaceEvent::PresentReady);
    assert_eq!(
        harness.app().windows()[0].completed_frame(),
        Some(held),
        "presenting held pixels is not another native layout frame"
    );

    gate.hold.set(false);
    harness.advance(std::time::Duration::from_millis(17));
    surface.request_redraw();
    harness.settle(4);
    let undamaged = *gate.frames.borrow().last().expect("undamaged frame");
    assert_eq!(undamaged.ordinal, held.ordinal + 1);
    assert_eq!(undamaged.outcome, FrameOutcome::Skipped(SkipReason::Undamaged));
    assert!(undamaged.layout_document_coherent);
    assert_eq!(undamaged.layout_revision, Some(undamaged.document_revision));
    assert_eq!(undamaged.timestamp, harness.platform().virtual_clock().timestamp());
    assert_ne!(undamaged.timestamp, held.timestamp);

    gate.next_outcome.set(Some(FrameOutcome::Recovered));
    surface.request_redraw();
    harness.pump();
    let recovered = *gate.frames.borrow().last().expect("recovery frame");
    assert_eq!(recovered.ordinal, undamaged.ordinal + 1);
    assert_eq!(recovered.outcome, FrameOutcome::Recovered);

    let drawn = gate.drawn.get();
    harness.deliver(surface.id(), SurfaceEvent::Occluded(true));
    surface.request_redraw();
    harness.pump();
    let occluded = *gate.frames.borrow().last().expect("occluded frame");
    assert_eq!(occluded.ordinal, recovered.ordinal + 1);
    assert_eq!(occluded.outcome, FrameOutcome::Skipped(SkipReason::Occluded));
    assert_eq!(gate.drawn.get(), drawn, "occlusion must bypass renderer.draw");
}

/// A real script binding that edits after the last layout, as late frame callbacks can.
struct ResizeBeforePaint {
    dom: DomHandle,
    node: NodeId,
    changed: bool,
}

impl HostBinding for ResizeBeforePaint {
    fn before_paint(&mut self, _: zgui_vocab::Timestamp) {
        if !self.changed {
            self.changed = true;
            self.dom.set_style_property(self.node, "width", Some("180px"));
        }
    }
}

#[test]
fn a_late_script_write_is_not_certified_against_the_previous_layout() {
    let (mut harness, gate) = window(false);
    harness.settle(8);
    let window = &mut harness.app_mut().windows_mut()[0];
    let before = window.completed_frame().expect("initial frame");
    let node = window.nodes_with_class("block")[0];
    window.install_binding(Box::new(ResizeBeforePaint {
        dom: window.dom_handle().clone(),
        node,
        changed: false,
    }));
    let surface = Arc::clone(&harness.platform().offscreens()[0]);
    surface.request_redraw();
    harness.pump();

    let late = *gate.frames.borrow().last().expect("late mutation frame");
    assert_eq!(late.layout_revision, Some(before.document_revision));
    assert!(late.document_revision > before.document_revision);
    assert!(!late.layout_document_coherent);
    assert_eq!(
        harness.app().windows()[0].box_with_class("block").expect("old box").size.width.0,
        100.0
    );

    harness.settle(8);
    let settled = *gate.frames.borrow().last().expect("corrected frame");
    assert!(settled.layout_document_coherent);
    assert_eq!(settled.layout_revision, Some(settled.document_revision));
    assert_eq!(settled.document_revision, late.document_revision);
    assert_eq!(
        harness.app().windows()[0].box_with_class("block").expect("new box").size.width.0,
        180.0
    );
}

#[test]
fn geometry_delivery_records_the_revision_of_its_actual_relayout() {
    let (mut harness, gate) = window(false);
    harness.settle(8);
    let window = &harness.app().windows()[0];
    let before = window.completed_frame().expect("initial frame");
    let node = window.nodes_with_class("block")[0];
    let dom = window.dom_handle().clone();
    let write = dom.clone();
    let _observation = dom.observe(
        node,
        Observed::BorderSize,
        Rc::new(move |value| {
            if let zgui_view::ObservedValue::BorderSize(size) = value
                && size.width.0 != 180.0
            {
                write.set_style_property(node, "width", Some("180px"));
            }
        }),
    );
    harness.platform().offscreens()[0].request_redraw();
    harness.pump();

    let delivered = *gate.frames.borrow().last().expect("delivery frame");
    assert!(delivered.document_revision > before.document_revision);
    assert_eq!(delivered.layout_revision, Some(delivered.document_revision));
    assert!(delivered.layout_document_coherent);
    assert_eq!(
        harness.app().windows()[0].box_with_class("block").expect("delivered box").size.width.0,
        180.0,
        "the evidence must describe the relayout in the same frame"
    );
    harness.settle(8);
}

#[test]
fn a_redundant_geometry_callback_write_requires_a_new_validation_frame() {
    let (mut harness, gate) = window(false);
    harness.settle(8);
    let window = &harness.app().windows()[0];
    let node = window.nodes_with_class("block")[0];
    let dom = window.dom_handle().clone();
    let write = dom.clone();
    let deliveries = Rc::new(Cell::new(0));
    let count = Rc::clone(&deliveries);
    let _observation = dom.observe(
        node,
        Observed::BorderSize,
        Rc::new(move |_| {
            count.set(count.get() + 1);
            write.set_style_property(node, "width", Some("180px"));
        }),
    );
    harness.platform().offscreens()[0].request_redraw();
    harness.pump();

    let delivered = *gate.frames.borrow().last().expect("delivery frame");
    assert_eq!(deliveries.get(), 2, "initial and relaid-out sizes were delivered");
    assert_eq!(
        delivered.layout_revision.map(|revision| revision + 1),
        Some(delivered.document_revision),
        "the redundant second write advanced the view revision after the last layout"
    );
    assert!(!delivered.layout_document_coherent);
    assert_eq!(
        harness.app().windows()[0].box_with_class("block").expect("delivered box").size.width.0,
        180.0
    );

    // Re-stating the declaration created no layout obligation, so the delivery's tail did not
    // lay out again. Do not certify the newer batch revision merely because geometry looks equal.
    harness.platform().offscreens()[0].request_redraw();
    harness.settle(8);
    let validated = *gate.frames.borrow().last().expect("validation frame");
    assert!(validated.ordinal > delivered.ordinal);
    assert_eq!(validated.document_revision, delivered.document_revision);
    assert_eq!(validated.layout_revision, Some(validated.document_revision));
    assert!(validated.layout_document_coherent);
    assert_eq!(deliveries.get(), 2, "unchanged geometry is not delivered again");
}

#[test]
fn an_unprobed_window_does_not_retain_completed_frame_evidence() {
    let mut harness = support::app(
        "root { display: block; width: 400px; height: 300px }",
        |cx: &mut BuildCx<'_>| Box::new(zgui_elements::column().into_view().build(cx)),
    );
    harness.settle(8);
    assert!(harness.app().windows()[0].completed_frame().is_none());
}

#[test]
fn a_semantic_only_update_is_coherent_when_layout_is_reused_and_draw_is_undamaged() {
    let (mut harness, gate) = window(false);
    harness.settle(8);
    let window = &harness.app().windows()[0];
    let before = window.completed_frame().expect("initial frame");
    let old_box = window.box_with_class("block").expect("initial geometry");
    let node = window.nodes_with_class("block")[0];
    let mut semantics = zgui_vocab::Semantics::new(zgui_vocab::Role::Button);
    semantics.label = Some("Updated semantic name".into());
    // Native semantics edits mark only accessibility work, not style or layout. This advances
    // the document revision while the layout gate can reuse its already measured boxes.
    window.dom_handle().set_semantics(node, Some(&semantics));
    let required_revision = window.dom().revision();
    assert!(required_revision > before.document_revision);
    assert_eq!(window.completed_frame(), Some(before));

    harness.platform().offscreens()[0].request_redraw();
    harness.pump();
    let frame = *gate.frames.borrow().last().expect("semantic validation frame");
    assert_eq!(frame.ordinal, before.ordinal + 1);
    assert_eq!(frame.outcome, FrameOutcome::Skipped(SkipReason::Undamaged));
    assert_eq!(frame.document_revision, required_revision);
    assert_eq!(frame.layout_revision, Some(required_revision));
    assert!(frame.layout_document_coherent);
    assert_eq!(
        harness.app().windows()[0].box_with_class("block"),
        Some(old_box),
        "semantic-only validation must preserve the standing geometry"
    );
    harness.settle(8);
}
