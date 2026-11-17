//! Asking a window which element holds a pointer's capture.
//!
//! A host that shares the pointer with other content reads this to keep a drag that started on
//! the interface with the interface, so the answer has to follow the capture a handler asked for
//! and its release.

mod support;

use zgui_geom::{CssPx, Point};
use zgui_platform::SurfaceEvent;
use zgui_view::{BuildCx, IntoView, View};
use zgui_vocab::{Modifiers, PointerAction, PointerEvent, PointerId, Timestamp};

const CSS: &str = "root { display: block; width: 400px; height: 300px }
                   column { display: block; width: 400px; height: 300px }";

/// A mouse event in the middle of the window.
fn mouse(action: PointerAction) -> SurfaceEvent {
    SurfaceEvent::Pointer {
        action,
        event: PointerEvent::mouse(Point::new(CssPx(200.0), CssPx(150.0))),
        modifiers: Modifiers::NONE,
        timestamp: Timestamp::ORIGIN,
    }
}

#[test]
fn a_capture_a_handler_asked_for_is_reported_until_a_handler_releases_it() {
    let mut app = support::app(CSS, |cx: &mut BuildCx<'_>| {
        Box::new(
            zgui_elements::column()
                .class("root")
                .on(zgui_view::events::POINTER_DOWN, |cx| cx.capture_pointer())
                .on(zgui_view::events::POINTER_UP, |cx| cx.release_pointer())
                .into_view()
                .build(cx),
        )
    });
    app.settle(4);
    let window = || &app.app().windows()[0];
    assert!(!window().has_pointer_capture());
    assert_eq!(window().pointer_capture(PointerId::MOUSE), None);

    app.deliver_to_first(mouse(PointerAction::Pressed));
    app.settle(4);
    let window = &app.app().windows()[0];
    assert!(window.has_pointer_capture(), "the press captured the mouse");
    assert!(window.pointer_capture(PointerId::MOUSE).is_some());
    assert_eq!(window.pointer_capture(PointerId::new(7)), None);

    app.deliver_to_first(mouse(PointerAction::Released));
    app.settle(4);
    let window = &app.app().windows()[0];
    assert!(
        !window.has_pointer_capture(),
        "the handler on the release ended the capture"
    );
    app.shut_down();
}

fn pen(id: u64, action: PointerAction) -> SurfaceEvent {
    let mut event = PointerEvent::mouse(Point::new(CssPx(200.0), CssPx(150.0)))
        .with_button(zgui_vocab::PointerButton::Primary);
    event.id = PointerId::new(id);
    event.kind = zgui_vocab::PointerKind::Pen;
    event.primary = true;
    event.pressure = Some(0.5);
    SurfaceEvent::Pointer {
        action,
        event,
        modifiers: Modifiers::NONE,
        timestamp: Timestamp::ORIGIN,
    }
}

#[test]
fn captures_follow_the_event_pointer_and_release_only_that_pointer() {
    let mut app = support::app(CSS, |cx: &mut BuildCx<'_>| {
        Box::new(
            zgui_elements::column()
                .class("root")
                .on(zgui_view::events::POINTER_DOWN, |cx| cx.capture_pointer())
                .on(zgui_view::events::POINTER_UP, |cx| cx.release_pointer())
                .on(zgui_view::events::POINTER_CANCEL, |cx| cx.release_pointer())
                .into_view()
                .build(cx),
        )
    });
    app.settle(4);
    let seven = PointerId::new(7);
    let nine = PointerId::new(9);

    app.deliver_to_first(pen(7, PointerAction::Pressed));
    app.settle(4);
    let window = &app.app().windows()[0];
    assert!(
        window.pointer_capture(seven).is_some(),
        "the pressing pointer holds the capture; mouse {:?}",
        window.pointer_capture(PointerId::MOUSE),
    );
    assert_eq!(window.pointer_capture(PointerId::MOUSE), None);

    app.deliver_to_first(pen(9, PointerAction::Pressed));
    app.settle(4);
    let window = &app.app().windows()[0];
    assert!(window.pointer_capture(seven).is_some());
    assert!(window.pointer_capture(nine).is_some());

    app.deliver_to_first(pen(7, PointerAction::Released));
    app.settle(4);
    let window = &app.app().windows()[0];
    assert_eq!(window.pointer_capture(seven), None, "the released pointer let go");
    assert!(window.pointer_capture(nine).is_some(), "the held pointer stays captured");

    app.deliver_to_first(pen(9, PointerAction::Cancelled));
    app.settle(4);
    let window = &app.app().windows()[0];
    assert!(!window.has_pointer_capture());
    app.shut_down();
}

#[test]
fn reclaiming_a_captured_node_ends_its_capture() {
    let mut app = support::app(CSS, |cx: &mut BuildCx<'_>| {
        Box::new(
            zgui_elements::column().class("root").into_view().build(cx),
        )
    });
    app.settle(4);
    let (dom, root) = {
        let window = &app.app().windows()[0];
        (
            zgui_view::DomHandle::from_rc(window.dom().clone()),
            window.nodes_with_class("root")[0],
        )
    };
    let control = dom.create_element(zgui_view::ElementName::new("control"));
    dom.insert(root, control, None);
    dom.set_style_property(control, "display", Some("block"));
    dom.set_style_property(control, "width", Some("120px"));
    dom.set_style_property(control, "height", Some("40px"));
    let moves = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let recording = moves.clone();
    dom.add_listener(control, zgui_vocab::EventKind::PointerDown, zgui_vocab::ListenerOptions::DEFAULT, std::rc::Rc::new(|cx| {
        cx.capture_pointer();
    }));
    dom.add_listener(control, zgui_vocab::EventKind::PointerMove, zgui_vocab::ListenerOptions::DEFAULT, std::rc::Rc::new(move |cx| {
        if let zgui_vocab::Payload::Pointer(event) = cx.payload() {
            recording.borrow_mut().push(event.id);
        }
    }));
    app.app().windows()[0].request_frame();
    app.settle(4);

    let mut press = mouse(PointerAction::Pressed);
    if let SurfaceEvent::Pointer { event, .. } = &mut press {
        event.position = Point::new(CssPx(20.0), CssPx(20.0));
    }
    app.deliver_to_first(press);
    app.settle(4);
    let window = &app.app().windows()[0];
    assert!(window.pointer_capture(PointerId::MOUSE).is_some());

    let mut away = mouse(PointerAction::Moved);
    if let SurfaceEvent::Pointer { event, .. } = &mut away {
        event.position = Point::new(CssPx(300.0), CssPx(150.0));
    }
    app.deliver_to_first(away);
    app.settle(4);
    assert_eq!(*moves.borrow(), vec![PointerId::MOUSE], "the capture routes the out-of-box move");

    let stray = dom.create_element(zgui_view::ElementName::new("box"));
    dom.insert(root, stray, None);
    dom.detach(stray);
    app.app().windows()[0].request_frame();
    app.settle(4);
    assert!(
        app.app().windows()[0].pointer_capture(PointerId::MOUSE).is_some(),
        "an unrelated removal leaves the capture alone",
    );

    dom.detach(control);
    app.app().windows()[0].request_frame();
    app.settle(4);
    assert!(!dom.is_live(control));
    assert!(
        !app.app().windows()[0].has_pointer_capture(),
        "the reclaimed control released its capture",
    );
    app.shut_down();
}

#[test]
fn touch_long_press_dispatches_touch_context_menu_and_suppresses_click() {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::Duration;

    let menus = Rc::new(RefCell::new(Vec::new()));
    let clicks = Rc::new(Cell::new(0u32));
    let recorded_menus = Rc::clone(&menus);
    let recorded_clicks = Rc::clone(&clicks);
    let mut app = support::app(CSS, move |cx: &mut BuildCx<'_>| {
        let recorded_menus = Rc::clone(&recorded_menus);
        let recorded_clicks = Rc::clone(&recorded_clicks);
        Box::new(
            zgui_elements::column()
                .class("root")
                .on(zgui_view::events::CONTEXT_MENU, move |cx| {
                    if let zgui_vocab::Payload::Pointer(event) = cx.payload() {
                        recorded_menus
                            .borrow_mut()
                            .push((event.id, event.kind, event.position));
                    }
                })
                .on(zgui_view::events::CLICK, move |_| {
                    recorded_clicks.set(recorded_clicks.get() + 1);
                })
                .into_view()
                .build(cx),
        )
    });
    app.settle(4);

    // Move the mouse first so stale mouse coordinates exist in pointer state before touch press.
    app.deliver_to_first(SurfaceEvent::Pointer {
        action: PointerAction::Moved,
        event: PointerEvent::mouse(Point::new(CssPx(12.0), CssPx(14.0))),
        modifiers: Modifiers::NONE,
        timestamp: Timestamp::ORIGIN,
    });
    app.settle(4);

    let touch_pos = Point::new(CssPx(180.0), CssPx(120.0));
    let touch_id = PointerId::new(1);
    app.deliver_to_first(SurfaceEvent::Pointer {
        action: PointerAction::Pressed,
        event: PointerEvent {
            id: touch_id,
            kind: zgui_vocab::PointerKind::Touch,
            primary: true,
            position: touch_pos,
            button: Some(zgui_vocab::PointerButton::Primary),
            pressure: None,
        },
        modifiers: Modifiers::NONE,
        timestamp: Timestamp::ORIGIN,
    });
    app.settle(4);
    assert!(menus.borrow().is_empty(), "press alone does not emit contextmenu");

    let frame = Duration::from_millis(20);
    for _ in 0..40 {
        app.advance(frame);
        app.settle(4);
        if !menus.borrow().is_empty() {
            break;
        }
    }
    assert_eq!(
        *menus.borrow(),
        vec![(touch_id, zgui_vocab::PointerKind::Touch, touch_pos)],
        "long-press must dispatch touch coordinates rather than stale mouse state",
    );

    let touch_release_time = Timestamp::from_origin(Duration::from_millis(700));
    app.deliver_to_first(SurfaceEvent::Pointer {
        action: PointerAction::Released,
        event: PointerEvent {
            id: touch_id,
            kind: zgui_vocab::PointerKind::Touch,
            primary: true,
            position: touch_pos,
            button: None,
            pressure: None,
        },
        modifiers: Modifiers::NONE,
        timestamp: touch_release_time,
    });
    app.settle(4);
    assert_eq!(menus.borrow().len(), 1, "release must not emit a duplicate contextmenu");
    assert_eq!(clicks.get(), 0, "touch release after long-press must not emit click");
    app.shut_down();
}
