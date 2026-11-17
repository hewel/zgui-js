//! Turning the wheel over a document whose only scrolling thing is the document itself.
//!
//! Almost every application is this shape: nothing inside the page scrolls, the page does. It is
//! also the shape with no coverage of its own — the wheel's default was asserted against a document
//! with *nothing* scrollable, which answers "scroll nothing" whatever the wiring does, and against
//! a scrollport nested inside the root, which reaches the container through a different chain.
//!
//! The second test is the one that stops a floating surface from breaking the page under it. A
//! modal's scrim covers the whole window and is fixed to it; a wheel turned over the page is
//! therefore a wheel turned over the scrim, and the container it scrolls is found by walking out of
//! that scrim rather than out of what is visible beneath it.

mod support;

use zgui_geom::{Css, CssPx, Point};
use zgui_platform::{Clock, PlatformCx, SurfaceEvent};
use zgui_view::{BuildCx, IntoView, View, ViewHost};
use zgui_vocab::{
    Modifiers, PointerId, PointerKind, ScrollDelta, ScrollPhase, Timestamp, WheelEvent,
};

/// A page taller than the window, scrolled by the root and by nothing else.
const PAGE_CSS: &str = ":root { display: block; overflow: auto }
                        .tall { display: block; width: 400px; height: 3000px }";

/// The same page with a surface fixed over the whole of it, as a modal's scrim is.
const COVERED_CSS: &str = ":root { display: block; overflow: auto }
                           .tall { display: block; width: 400px; height: 3000px }
                           .scrim { position: fixed; left: 0; top: 0; width: 100%; height: 100%;
                                    background-color: rgba(0, 0, 0, 0.4) }";

/// Turns the wheel three lines down, in the middle of the window.
fn wheel(harness: &mut zgui_platform_headless::Harness<zgui_runtime::Runtime>) {
    harness.deliver_to_first(SurfaceEvent::Wheel {
        event: WheelEvent {
            id: PointerId::MOUSE,
            kind: PointerKind::Mouse,
            position: Point::<CssPx, Css>::new(CssPx(100.0), CssPx(100.0)),
            delta: ScrollDelta::Lines { x: 0.0, y: 3.0 },
            phase: ScrollPhase::Discrete,
        },
        modifiers: Modifiers::NONE,
        timestamp: Timestamp::ORIGIN,
    });
    // A detent is carried to its destination over the following frames rather than landing in the
    // one it arrived in, so the clock has to run before anything is read off the offsets.
    for _ in 0..24 {
        harness.advance(std::time::Duration::from_millis(20));
        harness.pump();
    }
    harness.settle(4);
}

/// How far **the page itself** has moved down.
///
/// The root's own offset and not the largest offset in the window. "Something in here scrolled" is
/// true of a wheel that scrolled the wrong container, and the whole claim being made is about which
/// container moved — so a maximum over all of them would pass on precisely the defect these tests
/// are about.
fn scrolled(harness: &zgui_platform_headless::Harness<zgui_runtime::Runtime>) -> f32 {
    let window = harness
        .app()
        .windows()
        .first()
        .expect("the application opened a window");
    let scroll = window.scroll().borrow();
    let layout = window.layout().borrow();
    let root = layout.root().expect("the document has a root box");
    let element = layout
        .node(root)
        .source
        .expect("the root box came from the root element");
    scroll.offset_of(element).y.0
}

/// How far the furthest-scrolled container in the window has moved down.
fn anything_scrolled(harness: &zgui_platform_headless::Harness<zgui_runtime::Runtime>) -> f32 {
    let window = harness
        .app()
        .windows()
        .first()
        .expect("the application opened a window");
    let scroll = window.scroll().borrow();
    let layout = window.layout().borrow();
    layout
        .keys()
        .into_iter()
        .filter_map(|key| layout.node(key).source)
        .map(|element| scroll.offset_of(element).y.0)
        .fold(0.0, f32::max)
}

#[test]
fn a_wheel_over_a_page_that_scrolls_at_the_root_scrolls_it() {
    let mut harness = support::app_with_text(PAGE_CSS, move |cx: &mut BuildCx<'_>| {
        let view = zgui_elements::column().class("tall").into_view();
        Box::new(view.build(cx)) as Box<dyn zgui_view::Anchor>
    });
    harness.settle(8);
    assert_eq!(scrolled(&harness), 0.0);

    wheel(&mut harness);
    assert!(
        scrolled(&harness) > 0.0,
        "the wheel was turned over a page that scrolls and the page did not move"
    );
}

#[test]
fn a_wheel_over_a_surface_fixed_across_the_window_still_scrolls_the_page_under_it() {
    let mut harness = support::app_with_text(COVERED_CSS, move |cx: &mut BuildCx<'_>| {
        let view = zgui_elements::column()
            .child(zgui_elements::column().class("tall"))
            .child(zgui_elements::r#box().class("scrim"))
            .into_view();
        Box::new(view.build(cx)) as Box<dyn zgui_view::Anchor>
    });
    harness.settle(8);

    wheel(&mut harness);
    assert!(
        scrolled(&harness) > 0.0,
        "the wheel landed on the surface covering the window and scrolled nothing at all"
    );
}

/// The same surface, made scrollable, so the wheel it receives has somewhere of its own to go.
const SCROLLING_SCRIM_CSS: &str = ":root { display: block; overflow: auto }
                                   .tall { display: block; width: 400px; height: 3000px }
                                   .scrim { position: fixed; left: 0; top: 0;
                                            width: 400px; height: 300px; overflow: auto }";

#[test]
fn the_surface_fixed_across_the_window_is_really_the_one_the_wheel_lands_on() {
    // The control for the test above, and the reason it is not vacuous. That the page scrolls under
    // a covering surface only says anything if the surface is genuinely between the pointer and the
    // page: a `position: fixed` box that laid out to nothing, or that took no part in hit testing,
    // would let the wheel through to the page and pass that test while covering nothing at all.
    //
    // So the same surface is given something to scroll. If it is really the box under the pointer,
    // the wheel stops there and the page behind it does not move.
    let mut harness = support::app_with_text(SCROLLING_SCRIM_CSS, move |cx: &mut BuildCx<'_>| {
        let view = zgui_elements::column()
            .child(zgui_elements::column().class("tall"))
            .child(
                zgui_elements::r#box()
                    .class("scrim")
                    .child(zgui_elements::column().class("tall")),
            )
            .into_view();
        Box::new(view.build(cx)) as Box<dyn zgui_view::Anchor>
    });
    harness.settle(8);

    wheel(&mut harness);
    assert!(
        anything_scrolled(&harness) > 0.0,
        "the wheel scrolled nothing anywhere, so it never reached the surface either"
    );
    assert_eq!(
        scrolled(&harness),
        0.0,
        "the page moved, so the surface covering the window is not what the wheel landed on and \
         the test that the page scrolls under it proves nothing"
    );
}

#[test]
fn a_latched_background_gesture_cannot_move_a_frozen_root_via_an_inner_scroller() {
    let mut harness = support::app_with_text(
        ":root { display: block; overflow: auto }
         .tall { display: block; width: 400px; height: 3000px }
         .port { position: fixed; left: 0; top: 0; width: 400px; height: 120px; overflow: auto }",
        move |cx: &mut BuildCx<'_>| {
            Box::new(zgui_elements::column()
                .child(zgui_elements::column().class("tall"))
                .child(zgui_elements::r#box().class("port")
                    .child(zgui_elements::column().class("tall")))
                .into_view().build(cx)) as Box<dyn zgui_view::Anchor>
        },
    );
    harness.settle(8);
    let pad = |harness: &mut zgui_platform_headless::Harness<zgui_runtime::Runtime>, y: f32, delta: f32, phase| {
        harness.advance(std::time::Duration::from_millis(8));
        let timestamp = harness.platform().clock().timestamp();
        harness.deliver_to_first(SurfaceEvent::Wheel {
            event: WheelEvent {
                id: PointerId::MOUSE,
                kind: PointerKind::Mouse,
                position: Point::<CssPx, Css>::new(CssPx(100.0), CssPx(y)),
                delta: ScrollDelta::Pixels(zgui_geom::Size::new(CssPx(0.0), CssPx(delta))),
                phase,
            },
            modifiers: Modifiers::NONE,
            timestamp,
        });
        harness.settle(4);
    };
    pad(&mut harness, 200.0, 0.0, ScrollPhase::Started);
    pad(&mut harness, 200.0, 40.0, ScrollPhase::Moved);
    let before = scrolled(&harness);
    assert!(before > 0.0, "control: the gesture really scrolled the window root");
    let host = harness.app().windows()[0].host().clone();
    host.freeze_scrolling(true);
    harness.settle(8);
    assert!(harness.app().windows()[0].scrolling_frozen());
    assert_eq!(scrolled(&harness), before, "acquiring freeze preserves the offset");
    pad(&mut harness, 60.0, 80.0, ScrollPhase::Moved);
    assert_eq!(scrolled(&harness), before, "the previously latched root must stay frozen over the inner scroller");
    pad(&mut harness, 60.0, 80.0, ScrollPhase::Momentum);
    assert_eq!(scrolled(&harness), before, "old momentum cannot move the frozen root");
    assert_eq!(anything_scrolled(&harness), before, "old background momentum is not transferred to the inner scroller");
    pad(&mut harness, 60.0, 0.0, ScrollPhase::Ended);
    pad(&mut harness, 60.0, 0.0, ScrollPhase::Started);
    pad(&mut harness, 60.0, 120.0, ScrollPhase::Moved);
    assert_eq!(scrolled(&harness), before);
    assert!(anything_scrolled(&harness) > before, "a fresh inner gesture still scrolls its own container");
    pad(&mut harness, 60.0, 0.0, ScrollPhase::Ended);
    let inner_before = anything_scrolled(&harness);
    pad(&mut harness, 200.0, 0.0, ScrollPhase::Started);
    pad(&mut harness, 60.0, 120.0, ScrollPhase::Moved);
    assert_eq!(scrolled(&harness), before);
    assert!(anything_scrolled(&harness) > inner_before, "a fresh touch over frozen background can enter an inner scroller");
    pad(&mut harness, 60.0, 0.0, ScrollPhase::Ended);
    host.freeze_scrolling(false);
    harness.settle(8);
    pad(&mut harness, 200.0, 0.0, ScrollPhase::Started);
    pad(&mut harness, 200.0, 40.0, ScrollPhase::Moved);
    assert!(scrolled(&harness) > before, "releasing the freeze restores root movement");
    pad(&mut harness, 200.0, 0.0, ScrollPhase::Ended);
}

#[test]
fn a_frozen_root_rejects_programmatic_and_accessibility_scrolls_until_released() {
    let mut harness = support::app_with_text(PAGE_CSS, move |cx: &mut BuildCx<'_>| {
        Box::new(zgui_elements::column().class("tall").into_view().build(cx)) as Box<dyn zgui_view::Anchor>
    });
    harness.settle(8);
    let (host, root) = {
        let window = &harness.app().windows()[0];
        let layout = window.layout().borrow();
        let key = layout.node(layout.root().unwrap()).source.unwrap();
        (window.host().clone(), zgui_view_dom::id::to_view(key))
    };
    let move_to = |y| zgui_view::ScrollTarget::Offset(zgui_geom::Point::new(zgui_geom::DevicePx(0.0), zgui_geom::DevicePx(y)));
    host.scroll_to(root, move_to(100.0), zgui_view::ScrollBehavior::Instant);
    harness.settle(8);
    assert_eq!(scrolled(&harness), 100.0);
    host.freeze_scrolling(true);
    harness.settle(8);
    wheel(&mut harness);
    assert_eq!(scrolled(&harness), 100.0, "discrete wheels leave the frozen root in place");
    host.scroll_to(root, move_to(200.0), zgui_view::ScrollBehavior::Instant);
    harness.settle(8);
    assert_eq!(scrolled(&harness), 100.0, "programmatic scroll cannot move the frozen root");
    let action = |y| zgui_platform::WakeReason::A11yAction(zgui_a11y::ActionRequest {
        action: zgui_a11y::Action::SetScrollOffset,
        target_tree: zgui_a11y::TreeId::ROOT,
        target_node: zgui_a11y::NodeId(root.as_u64()),
        data: Some(zgui_a11y::ActionData::SetScrollOffset(zgui_a11y::Point::new(0.0, y))),
    });
    harness.platform().waker().wake(action(300.0));
    harness.settle(8);
    assert_eq!(scrolled(&harness), 100.0, "accessibility scroll cannot move the frozen root");
    host.freeze_scrolling(false);
    harness.settle(8);
    harness.platform().waker().wake(action(300.0));
    harness.settle(8);
    assert_eq!(scrolled(&harness), 300.0, "control: the same accessibility request really scrolls after release");
    host.scroll_to(root, move_to(400.0), zgui_view::ScrollBehavior::Instant);
    harness.settle(8);
    assert_eq!(scrolled(&harness), 400.0);
}
