#![doc = "Native geometry observation delivery regressions."]

mod support;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use zgui_geom::{Device, DevicePx, Rect, Size};
use zgui_platform::SurfaceEvent;
use zgui_reactive::prelude::{Get, GetUntracked, Set};
use zgui_view::{BuildCx, DomHandle, IntoView, NodeId, ObservationHandle, Observed, ObservedValue, View};

#[test]
fn zero_border_size_is_delivered_once() {
    zgui_reactive::install().ok();
    let seen: Rc<RefCell<Vec<Size<DevicePx, Device>>>> = Rc::new(RefCell::new(Vec::new()));
    let leases: Rc<RefCell<Vec<ObservationHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let mut app = support::app(
        "root { display: block; width: 400px; height: 300px } .zero { display: block; width: 0px; height: 0px }",
        {
            let seen = Rc::clone(&seen);
            let leases = Rc::clone(&leases);
            move |cx: &mut BuildCx<'_>| {
                let built = zgui_elements::column().class("zero").into_view().build(cx);
                let node = built.node();
                let recorded = Rc::clone(&seen);
                leases.borrow_mut().push(cx.dom().observe(
                    node,
                    Observed::BorderSize,
                    Rc::new(move |value: ObservedValue| {
                        recorded.borrow_mut().push(
                            value.as_border_size().expect("a BorderSize delivery carries a size"),
                        );
                    }),
                ));
                Box::new(built)
            }
        },
    );
    app.settle(16);
    assert_eq!(
        seen.borrow().as_slice(),
        [Size::new(DevicePx(0.0), DevicePx(0.0))],
        "the first real sample is delivered exactly once, with no nonzero placeholder"
    );
    app.settle(16);
    assert_eq!(
        seen.borrow().len(),
        1,
        "a settled zero is delivered a second time"
    );
    leases.borrow_mut().clear();
    app.shut_down();
}

#[test]
fn late_subscriptions_finish_across_the_existing_pass_budget() {
    zgui_reactive::install().ok();
    fn subscribe(
        dom: DomHandle,
        node: NodeId,
        count: Rc<Cell<usize>>,
        leases: Rc<RefCell<Vec<ObservationHandle>>>,
    ) {
        let dom_again = dom.clone();
        let count_again = Rc::clone(&count);
        let leases_again = Rc::clone(&leases);
        leases.borrow_mut().push(dom.observe(
            node,
            Observed::BorderSize,
            Rc::new(move |_: ObservedValue| {
                let next = count_again.get() + 1;
                count_again.set(next);
                if next < 5 {
                    subscribe(
                        dom_again.clone(),
                        node,
                        Rc::clone(&count_again),
                        Rc::clone(&leases_again),
                    );
                }
            }),
        ));
    }
    let count: Rc<Cell<usize>> = Rc::new(Cell::new(0));
    let leases: Rc<RefCell<Vec<ObservationHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let mut app = support::app(
        "root { display: block; width: 400px; height: 300px } .watched { display: block; width: 100px; height: 30px }",
        {
            let count = Rc::clone(&count);
            let leases = Rc::clone(&leases);
            move |cx: &mut BuildCx<'_>| {
                let built = zgui_elements::column().class("watched").into_view().build(cx);
                let node = built.node();
                subscribe(cx.dom_handle(), node, Rc::clone(&count), Rc::clone(&leases));
                Box::new(built)
            }
        },
    );
    app.settle(16);
    assert_eq!(
        count.get(),
        5,
        "each late registration's first sample arrives, and none of them is replayed"
    );
    leases.borrow_mut().clear();
    app.shut_down();
}

#[test]
fn scale_change_redelivers_unchanged_device_geometry() {
    zgui_reactive::install().ok();
    let seen: Rc<RefCell<Vec<(Rect<DevicePx, Device>, f32)>>> = Rc::new(RefCell::new(Vec::new()));
    let leases: Rc<RefCell<Vec<ObservationHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let mut app = support::app(
        "root, .sized { display: block; width: 100%; height: 100% }",
        {
            let seen = Rc::clone(&seen);
            let leases = Rc::clone(&leases);
            move |cx: &mut BuildCx<'_>| {
                let built = zgui_elements::column().class("sized").into_view().build(cx);
                let node = built.node();
                let host = cx.host_handle();
                let recorded = Rc::clone(&seen);
                leases.borrow_mut().push(cx.dom().observe(
                    node,
                    Observed::BorderBox,
                    Rc::new(move |value: ObservedValue| {
                        recorded.borrow_mut().push((
                            value.as_border_box().expect("a BorderBox delivery carries a rect"),
                            host.scale(),
                        ));
                    }),
                ));
                Box::new(built)
            }
        },
    );
    app.settle(16);
    assert_eq!(seen.borrow().len(), 1, "the initial sample is delivered once");
    app.deliver_to_first(SurfaceEvent::ScaleFactorChanged {
        scale_factor: 2.0,
        size: Size::new(DevicePx(400.0), DevicePx(300.0)),
    });
    app.settle(16);
    {
        let samples = seen.borrow();
        assert_eq!(samples.len(), 2, "unchanged device geometry is delivered again at the new scale");
        let (first, first_scale) = samples[0];
        let (second, second_scale) = samples[1];
        assert_eq!(first.size, Size::new(DevicePx(400.0), DevicePx(300.0)));
        assert_eq!(first_scale, 1.0);
        assert_eq!(second, first, "the device rectangle itself did not move");
        assert_eq!(second_scale, 2.0);
        assert_eq!(second.size.width.0 / second_scale, 200.0);
        assert_eq!(second.size.height.0 / second_scale, 150.0);
    }
    app.settle(16);
    assert_eq!(seen.borrow().len(), 2, "a settled scale is delivered a third time");
    leases.borrow_mut().clear();
    app.shut_down();
}

#[test]
fn late_observation_text_uses_current_paragraph_after_slot_reuse() {
    zgui_reactive::install().ok();
    let widened = zgui_reactive::RwSignal::new(false);
    let follower_wide = zgui_reactive::RwSignal::new(false);
    let count = zgui_reactive::RwSignal::new(1);
    let leases: Rc<RefCell<Vec<ObservationHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let mut app = support::app_with_text(
        ".root { display: block; width: 400px; height: 300px } .anchor, .follower { display: block; width: 100px; height: 10px } .wide { width: 200px } text { display: block; width: 40px; height: 20px }",
        {
            let leases = Rc::clone(&leases);
            move |cx: &mut BuildCx<'_>| {
                let anchor = zgui_view::NodeRef::new();
                let follower = zgui_view::NodeRef::new();
                let built = zgui_elements::column().class("root")
                    .child(zgui_elements::column().class("anchor")
                        .class_toggle(zgui_view::ClassName::new("wide"), move || widened.get())
                        .node_ref(anchor))
                    .child(zgui_elements::column().class("follower")
                        .class_toggle(zgui_view::ClassName::new("wide"), move || follower_wide.get())
                        .node_ref(follower))
                    .child(zgui_elements::text().child(move || count.get().to_string()))
                    .into_view().build(cx);
                leases.borrow_mut().push(cx.dom().observe(anchor.get_untracked().unwrap(), Observed::BorderSize, Rc::new(move |value: ObservedValue| {
                    if value.as_border_size().unwrap().width.0 == 200.0 {
                        count.set(7);
                        follower_wide.set(true);
                    }
                })));
                leases.borrow_mut().push(cx.dom().observe(follower.get_untracked().unwrap(), Observed::BorderSize, Rc::new(move |value: ObservedValue| {
                    if value.as_border_size().unwrap().width.0 == 200.0 { count.set(9); }
                })));
                Box::new(built)
            }
        },
    );
    app.settle(8);
    let paragraph = |window: &zgui_runtime::Window| {
        let layout = window.layout().borrow();
        let mut held = Vec::new();
        layout.each_fragment(|fragment| {
            if let zgui_layout::FragmentKind::Line { paragraph, .. } = fragment.kind {
                held.push((paragraph, layout.paragraph_key(paragraph).unwrap()));
            }
        });
        assert_eq!(held.len(), 1);
        held[0]
    };
    let tile = |window: &zgui_runtime::Window| {
        let glyphs = &window.scene().primitives.mono_sprites;
        assert_eq!(glyphs.len(), 1);
        (glyphs[0].tile.texture, glyphs[0].tile.tile)
    };
    let before_paragraph = paragraph(&app.app().windows()[0]);
    let before_tile = tile(&app.app().windows()[0]);
    widened.set(true);
    app.settle(8);
    assert_eq!(count.get_untracked(), 9);
    let after_paragraph = paragraph(&app.app().windows()[0]);
    assert_eq!(before_paragraph.0, after_paragraph.0, "fixture must exercise a recycled paragraph slot");
    assert_ne!(before_paragraph.1, after_paragraph.1, "the recycled slot must hold different shaped content");
    assert_ne!(before_tile, tile(&app.app().windows()[0]), "the final observed digit must replace the previously painted digit");
    leases.borrow_mut().clear();
    app.shut_down();
}
