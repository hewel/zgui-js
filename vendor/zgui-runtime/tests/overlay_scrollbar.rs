//! Native overlay scrollbar (`--zgui-scrollbar-mode: overlay`) runtime lifecycle:
//! - Scroll reveal with zero restyle / relayout / hit-index rebuild,
//! - Deterministic 800ms hold + 200ms auto-fade back to zero idle wakeups,
//! - Hidden-state click pass-through to underlying controls,
//! - 16px edge hotzone reveal, 4px -> 6px hover/drag widening, and thumb dragging,
//! - `reduced_motion` instantaneous reveal/hide and emphasis with zero fade frames.

mod support;

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use zgui_dom::NodeKey;
use zgui_geom::{Css, CssPx, Point};
use zgui_layout::Axis;
use zgui_platform::SurfaceEvent;
use zgui_profile::Counter;
use zgui_scene::Paint;
use zgui_testkit_scene::counters::Recording;
use zgui_view::{BuildCx, IntoView, View};
use zgui_vocab::{
    Modifiers, PointerAction, PointerEvent, ScrollDelta, ScrollPhase, Timestamp, WheelEvent,
};

const CSS: &str = "
root { --zgui-scrollbar-mode: overlay; display: block; width: 400px; height: 300px }
.port {
    display: block;
    width: 400px;
    height: 120px;
    overflow-y: auto;
    background-color: #ffffff;
    --zgui-scrollbar-thumb: rgb(255, 0, 0);
}
.row { display: block; width: 100%; height: 24px; background-color: #204080 }
";

fn window(
    harness: &zgui_platform_headless::Harness<zgui_runtime::Runtime>,
) -> &zgui_runtime::Window {
    harness
        .app()
        .windows()
        .first()
        .expect("application opened a window")
}

fn port_node(harness: &zgui_platform_headless::Harness<zgui_runtime::Runtime>) -> NodeKey {
    let win = window(harness);
    let layout = win.layout().borrow();
    layout
        .keys()
        .into_iter()
        .find(|&box_key| {
            layout.scrollbar_mode(box_key) == zgui_layout::scroll_region::ScrollbarMode::Overlay
                && zgui_layout::scroll_region::region_of(&layout, box_key).is_some()
        })
        .and_then(|box_key| layout.node(box_key).source)
        .expect("overlay scroll container box")
}

/// Finds all red thumb quads (our test `--zgui-scrollbar-thumb: rgb(255, 0, 0)`) in the scene.
fn thumb_quads(harness: &zgui_platform_headless::Harness<zgui_runtime::Runtime>) -> Vec<[f32; 4]> {
    let scene = window(harness).scene();
    scene
        .primitives
        .quads
        .iter()
        .filter(|quad| {
            let Some(id) = quad.fill.id() else {
                return false;
            };
            matches!(scene.paints.get(id), Some(Paint::Solid(c))
                if c.components()[0] > 0.9 && c.components()[1] < 0.05 && c.components()[2] < 0.05)
        })
        .map(|quad| quad.bounds)
        .collect()
}

fn listing(clicks: Rc<Cell<u32>>) -> zgui_platform_headless::Harness<zgui_runtime::Runtime> {
    support::app_with_text(CSS, move |cx: &mut BuildCx<'_>| {
        let mut port = zgui_elements::column().class("port");
        for i in 0..30 {
            let mut row = zgui_elements::column().class("row");
            if i == 0 {
                let clicks = Rc::clone(&clicks);
                row = row.on(zgui_view::events::CLICK, move |_| {
                    clicks.set(clicks.get() + 1);
                });
            }
            port = port.child(row);
        }
        Box::new(
            zgui_elements::column()
                .class("root")
                .child(port)
                .into_view()
                .build(cx),
        )
    })
}

fn pointer(
    harness: &mut zgui_platform_headless::Harness<zgui_runtime::Runtime>,
    action: PointerAction,
    x: f32,
    y: f32,
) {
    harness.deliver_to_first(SurfaceEvent::Pointer {
        action,
        event: PointerEvent::mouse(Point::new(CssPx(x), CssPx(y))),
        modifiers: Modifiers::NONE,
        timestamp: Timestamp::ORIGIN,
    });
}

#[test]
fn wheel_scroll_reveals_overlay_bar_without_restyle_or_relayout_and_fades_to_idle() {
    let mut recording = Recording::begin();
    let mut harness = listing(Rc::new(Cell::new(0)));
    let built = recording.measure(|| {
        harness.settle(8);
    });
    let restyle = built.control(Counter::ElementsRestyled);
    let relayout = built.control(Counter::NodesRelaidOut);
    let rebuild = built.control(Counter::HitIndexRebuilds);

    let port = port_node(&harness);
    // Starts completely hidden (0 scrollbar quads emitted).
    assert_eq!(
        window(&harness)
            .overlay_scrollbar_visual(port, Axis::Vertical)
            .alpha,
        0.0
    );
    assert!(
        thumb_quads(&harness).is_empty(),
        "overlay scrollbar must emit zero quads before any scroll or hotzone hover"
    );

    // Wheel scroll reveals the 4px resting capsule with ZERO restyle, relayout, or hit rebuild.
    let scrolled = recording.measure(|| {
        harness.deliver_to_first(SurfaceEvent::Wheel {
            event: WheelEvent {
                id: zgui_vocab::PointerId::MOUSE,
                kind: zgui_vocab::PointerKind::Mouse,
                position: Point::<CssPx, Css>::new(CssPx(100.0), CssPx(60.0)),
                delta: ScrollDelta::Lines { x: 0.0, y: 3.0 },
                phase: ScrollPhase::Discrete,
            },
            modifiers: Modifiers::NONE,
            timestamp: Timestamp::ORIGIN,
        });
        harness.pump();
    });
    scrolled.assert_zero(Counter::ElementsRestyled, &restyle);
    scrolled.assert_zero(Counter::NodesRelaidOut, &relayout);
    scrolled.assert_zero(Counter::HitIndexRebuilds, &rebuild);

    let visual = window(&harness).overlay_scrollbar_visual(port, Axis::Vertical);
    assert_eq!(visual.alpha, 1.0);
    assert_eq!(visual.emphasis, 0.0);
    let quads = thumb_quads(&harness);
    assert_eq!(quads.len(), 1, "one resting 4px thumb quad must be emitted on reveal");
    assert!(
        (quads[0][2] - 4.0).abs() < 0.05,
        "resting width must be 4px, got {:?}",
        quads[0]
    );

    // Let the wheel glide finish completely so we enter the idle 800ms hold phase.
    for _ in 0..20 {
        harness.advance(Duration::from_millis(20));
        harness.pump();
    }
    harness.settle(4);
    assert_eq!(
        window(&harness)
            .overlay_scrollbar_visual(port, Axis::Vertical)
            .alpha,
        1.0
    );
    assert!(
        !window(&harness).is_animating(),
        "during the 800ms hold phase, the window must NOT spin at 60fps"
    );

    // Advance past 800ms hold + 200ms fade -> smoothly fades out to alpha == 0.0 with zero DOM work.
    let faded = recording.measure(|| {
        for _ in 0..20 {
            harness.advance(Duration::from_millis(60));
            harness.pump();
        }
        harness.settle(4);
    });
    faded.assert_zero(Counter::ElementsRestyled, &restyle);
    faded.assert_zero(Counter::NodesRelaidOut, &relayout);
    faded.assert_zero(Counter::HitIndexRebuilds, &rebuild);

    assert_eq!(
        window(&harness)
            .overlay_scrollbar_visual(port, Axis::Vertical)
            .alpha,
        0.0,
        "overlay scrollbar must fade completely to alpha == 0 after hold + fade"
    );
    assert!(
        thumb_quads(&harness).is_empty(),
        "faded overlay scrollbar must emit zero quads"
    );
    assert!(
        !window(&harness).is_animating(),
        "window must stop animating once faded out"
    );
}

#[test]
fn hidden_overlay_bar_passes_edge_clicks_to_underlying_control() {
    let _exclusive = Recording::begin();
    let clicks = Rc::new(Cell::new(0));
    let mut harness = listing(Rc::clone(&clicks));
    harness.settle(8);

    // Without hovering first, press & release directly inside the 12px right-edge strip over row 0.
    // Because the overlay scrollbar is hidden (`alpha == 0`), the click must pass straight through
    // to row 0!
    pointer(&mut harness, PointerAction::Pressed, 394.0, 12.0);
    pointer(&mut harness, PointerAction::Released, 394.0, 12.0);
    harness.settle(4);
    assert_eq!(
        clicks.get(),
        1,
        "clicking over a hidden overlay scrollbar must hit the underlying row"
    );
}

#[test]
fn hotzone_hover_reveals_widens_to_6px_and_thumb_drag_scrolls_accurately() {
    let _exclusive = Recording::begin();
    let mut harness = listing(Rc::new(Cell::new(0)));
    harness.settle(8);

    let port = port_node(&harness);
    // Move pointer into the 16px right-edge hotzone (x = 388 is 12px from right edge of 400px).
    pointer(&mut harness, PointerAction::Moved, 388.0, 20.0);
    for _ in 0..10 {
        harness.advance(Duration::from_millis(20));
        harness.pump();
    }
    harness.settle(4);

    let visual = window(&harness).overlay_scrollbar_visual(port, Axis::Vertical);
    assert_eq!(visual.alpha, 1.0);
    assert!((visual.emphasis - 1.0).abs() < 0.01);
    let quads = thumb_quads(&harness);
    assert_eq!(quads.len(), 1);
    assert!(
        (quads[0][2] - 6.0).abs() < 0.05,
        "hovered overlay thumb must widen to 6px, got {:?}",
        quads[0]
    );

    // Staying in hotzone past 1200ms keeps bar pinned visible (never fades out under pointer).
    for _ in 0..12 {
        harness.advance(Duration::from_millis(100));
        harness.pump();
    }
    assert_eq!(
        window(&harness)
            .overlay_scrollbar_visual(port, Axis::Vertical)
            .alpha,
        1.0,
        "bar must remain visible while pointer stays in edge hotzone"
    );

    // Grab the visible thumb at (394, 12) and drag down to y = 72 (even while x drifts left to 300).
    let before = window(&harness).scroll().borrow().offset_of(port).y.0;
    pointer(&mut harness, PointerAction::Moved, 394.0, 12.0);
    harness.settle(2);
    pointer(&mut harness, PointerAction::Pressed, 394.0, 12.0);
    harness.settle(2);
    pointer(&mut harness, PointerAction::Moved, 300.0, 72.0);
    harness.settle(4);
    let after = window(&harness).scroll().borrow().offset_of(port).y.0;
    assert!(
        after > before + 100.0,
        "dragging overlay thumb must scroll proportionally ({before} -> {after})"
    );
    assert_eq!(
        window(&harness)
            .overlay_scrollbar_visual(port, Axis::Vertical)
            .alpha,
        1.0,
        "bar must stay pinned during active drag even when pointer drifts off the edge"
    );

    // Release outside hotzone -> holds 800ms then fades cleanly to alpha == 0.
    pointer(&mut harness, PointerAction::Released, 300.0, 72.0);
    for _ in 0..20 {
        harness.advance(Duration::from_millis(60));
        harness.pump();
    }
    harness.settle(4);
    assert_eq!(
        window(&harness)
            .overlay_scrollbar_visual(port, Axis::Vertical)
            .alpha,
        0.0
    );
}

#[test]
fn reduced_motion_snaps_reveal_hide_and_emphasis_with_zero_animation_frames() {
    let _exclusive = Recording::begin();
    let mut harness = listing(Rc::new(Cell::new(0)));
    harness.settle(8);
    harness.app_mut().set_reduced_motion(Some(true));

    let port = port_node(&harness);
    // Hovering hotzone under reduced_motion snaps immediately to emphasis == 1.0 (6px) with
    // zero animation frames.
    pointer(&mut harness, PointerAction::Moved, 390.0, 20.0);
    harness.pump();
    let visual = window(&harness).overlay_scrollbar_visual(port, Axis::Vertical);
    assert_eq!(visual.alpha, 1.0);
    assert_eq!(visual.emphasis, 1.0);
    assert!(
        !window(&harness).is_animating(),
        "reduced_motion must not schedule emphasis animation frames"
    );

    // Leaving hotzone snaps emphasis straight to 0.0 (4px) and enters 800ms hold.
    pointer(&mut harness, PointerAction::Moved, 100.0, 20.0);
    harness.pump();
    let resting = window(&harness).overlay_scrollbar_visual(port, Axis::Vertical);
    assert_eq!(resting.alpha, 1.0);
    assert_eq!(resting.emphasis, 0.0);
    assert!(!window(&harness).is_animating());

    // Jumping exactly 800ms snaps straight to alpha == 0.0 in 1 frame with zero fade frames!
    harness.advance(Duration::from_millis(800));
    harness.pump();
    assert_eq!(
        window(&harness)
            .overlay_scrollbar_visual(port, Axis::Vertical)
            .alpha,
        0.0,
        "reduced_motion must snap straight to hidden when hold expires"
    );
    assert!(!window(&harness).is_animating());
    assert!(thumb_quads(&harness).is_empty());
}
