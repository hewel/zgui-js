//! End-to-end native `inert` subtree isolation tests over a real headless runtime window.
//!
//! Covers:
//! 1. Stationary-pointer rehit landing on an eligible underlying sibling & click pass-through.
//! 2. Mid-capture & armed-press cancellation without firing release-click or allowing reacquisition.
//! 3. Mid-IME-preedit focus evacuation settling the inert field without redirecting queued commits.
//! 4. Nested `trapFocus` evacuation to live outer trap + empty live trap confining Tab.
//! 5. Rejection of inbound AccessKit actions targeting effectively inert nodes.
//! 6. Programmatic `focus_with_reveal` rejection (no focus, no scroll) and inert window shortcuts.

mod support;

use std::cell::RefCell;
use std::rc::Rc;

use zgui_a11y::{Action, ActionRequest, TreeId};
use zgui_geom::{CssPx, Point};
use zgui_platform::SurfaceEvent;
use zgui_reactive::prelude::GetUntracked;
use zgui_view::{FocusTrapOptions, NodeId, NodeRef, ViewHost, events};
use zgui_vocab::{
    ImeEvent, KeyCode, KeyEvent, KeyState, Modifiers, NamedKey, PhysicalKey, PointerAction,
    PointerButton, PointerEvent, Timestamp, UiState,
};

fn pointer_event(x: f32, y: f32, button: Option<PointerButton>) -> PointerEvent {
    let mut ev = PointerEvent::mouse(Point::new(CssPx(x), CssPx(y)));
    ev.button = button;
    ev
}

fn set_inert(
    harness: &mut zgui_platform_headless::Harness<zgui_runtime::Runtime>,
    node: NodeId,
    inert: bool,
) {
    let dom = harness.app().windows()[0].dom().clone();
    zgui_view::Dom::set_ui_state(&*dom, node, UiState::INERT, inert);
    harness.app().windows()[0].request_frame();
}

fn focused(harness: &zgui_platform_headless::Harness<zgui_runtime::Runtime>) -> Option<NodeId> {
    harness.app().windows()[0].host().focused().get_untracked()
}

#[test]
fn stationary_pointer_rehit_and_click_pass_through_to_lower_sibling() {
    const CSS: &str = "
        .root { position: relative; display: block; width: 200px; height: 200px; }
        .lower { position: absolute; left: 0px; top: 0px; width: 120px; height: 60px; }
        .overlay { position: absolute; left: 0px; top: 0px; width: 120px; height: 60px; }
    ";
    let lower_ref = NodeRef::new();
    let overlay_ref = NodeRef::new();
    let log = Rc::new(RefCell::new(Vec::<&'static str>::new()));

    let log_l = Rc::clone(&log);
    let log_o = Rc::clone(&log);
    let mut harness = support::app_with_text(CSS, move |cx: &mut zgui_view::BuildCx<'_>| {
        use zgui_view::{IntoView, View};
        let l1 = Rc::clone(&log_l);
        let l2 = Rc::clone(&log_l);
        let o1 = Rc::clone(&log_o);
        let o2 = Rc::clone(&log_o);
        let o3 = Rc::clone(&log_o);
        Box::new(
            zgui_elements::column()
                .class("root")
                .child(
                    zgui_elements::control()
                        .class("lower")
                        .node_ref(lower_ref)
                        .on(events::POINTER_ENTER, move |_| {
                            l1.borrow_mut().push("enter:lower");
                        })
                        .on(events::CLICK, move |_| {
                            l2.borrow_mut().push("click:lower");
                        }),
                )
                .child(
                    zgui_elements::column()
                        .class("overlay")
                        .node_ref(overlay_ref)
                        .on(events::POINTER_ENTER, move |_| {
                            o1.borrow_mut().push("enter:overlay");
                        })
                        .on(events::POINTER_LEAVE, move |_| {
                            o2.borrow_mut().push("leave:overlay");
                        })
                        .on(events::CLICK, move |_| {
                            o3.borrow_mut().push("click:overlay");
                        }),
                )
                .into_view()
                .build(cx),
        )
    });
    harness.settle(8);

    // Move stationary pointer over the shared (40, 20) region -> lands on overlay.
    harness.deliver_to_first(SurfaceEvent::Pointer {
        action: PointerAction::Moved,
        event: pointer_event(40.0, 20.0, None),
        modifiers: Modifiers::NONE,
        timestamp: Timestamp::ORIGIN,
    });
    harness.settle(4);
    assert_eq!(*log.borrow(), vec!["enter:overlay"]);
    log.borrow_mut().clear();

    // Mark overlay inert WITHOUT moving the pointer -> forced stationary re-hit lands on lower!
    let overlay_id = overlay_ref.get().expect("overlay mounted");
    set_inert(&mut harness, overlay_id, true);
    harness.settle(4);
    assert_eq!(
        *log.borrow(),
        vec!["leave:overlay", "enter:lower"],
        "stationary pointer must re-hit lower sibling even when geometry did not change"
    );
    log.borrow_mut().clear();

    // Click at (40, 20) passes through inert overlay to lower sibling.
    harness.deliver_to_first(SurfaceEvent::Pointer {
        action: PointerAction::Pressed,
        event: pointer_event(40.0, 20.0, Some(PointerButton::Primary)),
        modifiers: Modifiers::NONE,
        timestamp: Timestamp::ORIGIN,
    });
    harness.deliver_to_first(SurfaceEvent::Pointer {
        action: PointerAction::Released,
        event: pointer_event(40.0, 20.0, Some(PointerButton::Primary)),
        modifiers: Modifiers::NONE,
        timestamp: Timestamp::ORIGIN,
    });
    harness.settle(4);
    assert_eq!(*log.borrow(), vec!["click:lower"]);
}

#[test]
fn inert_mid_capture_cancels_capture_and_prevents_release_click() {
    const CSS: &str = "
        .root { display: block; width: 200px; height: 200px; }
        control { display: block; width: 120px; height: 60px; }
    ";
    let btn_ref = NodeRef::new();
    let clicked = Rc::new(RefCell::new(0usize));
    let clicked_c = Rc::clone(&clicked);

    let mut harness = support::app_with_text(CSS, move |cx: &mut zgui_view::BuildCx<'_>| {
        use zgui_view::{IntoView, View};
        let clicked_c = Rc::clone(&clicked_c);
        Box::new(
            zgui_elements::column()
                .class("root")
                .child(
                    zgui_elements::control()
                        .node_ref(btn_ref)
                        .on(events::POINTER_DOWN, move |cx| {
                            cx.capture_pointer();
                        })
                        .on(events::CLICK, move |_| {
                            *clicked_c.borrow_mut() += 1;
                        }),
                )
                .into_view()
                .build(cx),
        )
    });
    harness.settle(8);

    // Press button -> acquires pointer capture & arms click.
    harness.deliver_to_first(SurfaceEvent::Pointer {
        action: PointerAction::Pressed,
        event: pointer_event(40.0, 20.0, Some(PointerButton::Primary)),
        modifiers: Modifiers::NONE,
        timestamp: Timestamp::ORIGIN,
    });
    harness.settle(4);
    assert!(
        harness.app().windows()[0].has_pointer_capture(),
        "button should hold pointer capture while active"
    );

    // Toggle inert mid-press -> capture must drop, and release must NOT fire click!
    let btn_id = btn_ref.get().expect("button mounted");
    set_inert(&mut harness, btn_id, true);
    harness.settle(4);
    assert!(
        !harness.app().windows()[0].has_pointer_capture(),
        "making capturing node inert must cancel active capture"
    );

    harness.deliver_to_first(SurfaceEvent::Pointer {
        action: PointerAction::Released,
        event: pointer_event(40.0, 20.0, Some(PointerButton::Primary)),
        modifiers: Modifiers::NONE,
        timestamp: Timestamp::ORIGIN,
    });
    harness.settle(4);
    assert_eq!(
        *clicked.borrow(),
        0,
        "release after inert cancellation must never fire click"
    );
}

#[test]
fn inert_mid_ime_preedit_evacuates_without_redirecting_pending_commit() {
    const CSS: &str = "
        .root { display: block; width: 300px; height: 200px; }
        .trap { display: block; width: 300px; height: 200px; }
        field { display: block; width: 200px; height: 32px; }
    ";
    let live_field = NodeRef::new();
    let inert_panel = NodeRef::new();
    let exiting_field = NodeRef::new();
    let trap_root = NodeRef::new();
    let live_inputs = Rc::new(RefCell::new(Vec::<String>::new()));
    let exiting_changes = Rc::new(RefCell::new(Vec::<String>::new()));

    let live_inputs_c = Rc::clone(&live_inputs);
    let exiting_changes_c = Rc::clone(&exiting_changes);
    let mut harness = support::app_with_text(CSS, move |cx: &mut zgui_view::BuildCx<'_>| {
        use zgui_view::{IntoView, View};
        let live_inputs_c = Rc::clone(&live_inputs_c);
        let exiting_changes_c = Rc::clone(&exiting_changes_c);
        Box::new(
            zgui_elements::column()
                .class("root")
                .child(
                    zgui_elements::column()
                        .class("trap")
                        .node_ref(trap_root)
                        .child(
                            zgui_elements::field()
                                .node_ref(live_field)
                                .on(events::INPUT, move |cx| {
                                    live_inputs_c.borrow_mut().push(cx.value.to_string());
                                }),
                        )
                        .child(
                            zgui_elements::column()
                                .node_ref(inert_panel)
                                .child(
                                    zgui_elements::field()
                                        .node_ref(exiting_field)
                                        .on(events::CHANGE, move |cx| {
                                            exiting_changes_c
                                                .borrow_mut()
                                                .push(cx.value.to_string());
                                        }),
                                ),
                        ),
                )
                .into_view()
                .build(cx),
        )
    });
    harness.settle(8);

    // Install outer live trap so evacuation moves focus directly to `live_field`.
    let _trap_guard = trap_root.trap_focus(FocusTrapOptions::CONFINE_ONLY);

    // Focus exiting_field and start an active IME preedit.
    let exiting_id = exiting_field.get().expect("exiting field");
    harness.app().windows()[0].host().focus(exiting_id);
    harness.settle(4);
    assert_eq!(focused(&harness), Some(exiting_id));

    harness.deliver_to_first(SurfaceEvent::Ime(ImeEvent::Enabled));
    harness.deliver_to_first(SurfaceEvent::Ime(ImeEvent::Preedit {
        text: "に".into(),
        cursor: Some(3..3),
    }));
    harness.settle(4);

    // Mark inert_panel inert AND queue a stale ImeEvent::Commit in the very next batch.
    let panel_id = inert_panel.get().expect("panel");
    set_inert(&mut harness, panel_id, true);
    harness.deliver_to_first(SurfaceEvent::Ime(ImeEvent::Commit("日".into())));
    harness.settle(4);

    let live_id = live_field.get().expect("live field");
    assert_eq!(
        focused(&harness),
        Some(live_id),
        "focus must evacuate from inert subtree to live outer trap's first eligible control"
    );
    assert!(
        live_inputs.borrow().is_empty(),
        "stale IME commit from evacuated inert field must never redirect to newly focused field: {:?}",
        *live_inputs.borrow()
    );
    assert_eq!(
        *exiting_changes.borrow(),
        vec!["に".to_owned()],
        "inert field's provisional preedit settled cleanly on the leaving field"
    );
}

#[test]
fn nested_focus_traps_evacuate_to_live_outer_and_empty_live_trap_blocks_tab_escape() {
    const CSS: &str = "
        .root { display: block; width: 300px; height: 240px; }
        control { display: block; width: 100px; height: 24px; }
    ";
    let outside_btn = NodeRef::new();
    let outer_trap_ref = NodeRef::new();
    let outer_btn = NodeRef::new();
    let inner_trap_ref = NodeRef::new();
    let inner_btn = NodeRef::new();

    let mut harness = support::app_with_text(CSS, move |cx: &mut zgui_view::BuildCx<'_>| {
        use zgui_view::{IntoView, View};
        Box::new(
            zgui_elements::column()
                .class("root")
                .child(zgui_elements::control().node_ref(outside_btn))
                .child(
                    zgui_elements::column()
                        .node_ref(outer_trap_ref)
                        .child(zgui_elements::control().node_ref(outer_btn))
                        .child(
                            zgui_elements::column()
                                .node_ref(inner_trap_ref)
                                .child(zgui_elements::control().node_ref(inner_btn)),
                        ),
                )
                .into_view()
                .build(cx),
        )
    });
    harness.settle(8);

    let inner_trap_id = inner_trap_ref.get().expect("inner trap");
    let outer_btn_id = outer_btn.get().expect("outer btn");
    let inner_btn_id = inner_btn.get().expect("inner btn");

    let _outer_guard = outer_trap_ref.trap_focus(FocusTrapOptions::MODAL);
    let _inner_guard = inner_trap_ref.trap_focus(FocusTrapOptions::MODAL);
    harness.settle(8);
    assert_eq!(focused(&harness), Some(inner_btn_id));

    // Make inner trap root inert -> focus evacuates into live outer trap (`outer_btn`).
    set_inert(&mut harness, inner_trap_id, true);
    harness.settle(8);
    assert_eq!(focused(&harness), Some(outer_btn_id));

    // Tab stays inside live outer trap and skips inert inner trap.
    harness.deliver_to_first(SurfaceEvent::Key {
        state: KeyState::Pressed,
        event: KeyEvent::named(NamedKey::Tab, PhysicalKey::Code(KeyCode::Tab)),
        modifiers: Modifiers::NONE,
        timestamp: Timestamp::ORIGIN,
    });
    harness.settle(4);
    assert_eq!(focused(&harness), Some(outer_btn_id));

    // Now make `outer_btn` inert too while `outer_trap_ref` itself stays LIVE (empty live trap!).
    set_inert(&mut harness, outer_btn_id, true);
    harness.settle(4);
    assert_eq!(
        focused(&harness),
        None,
        "empty live trap clears focus when its last focusable child becomes inert"
    );

    // Pressing Tab must NOT escape the empty live trap to `outside_btn`!
    harness.deliver_to_first(SurfaceEvent::Key {
        state: KeyState::Pressed,
        event: KeyEvent::named(NamedKey::Tab, PhysicalKey::Code(KeyCode::Tab)),
        modifiers: Modifiers::NONE,
        timestamp: Timestamp::ORIGIN,
    });
    harness.settle(4);
    assert_eq!(
        focused(&harness),
        None,
        "Tab must not escape an empty live trap to outside controls"
    );
}

#[test]
fn inbound_a11y_actions_and_focus_with_reveal_and_shortcuts_reject_inert_nodes() {
    const CSS: &str = "
        .root { display: block; width: 300px; height: 200px; }
        .panel { display: block; width: 200px; height: 80px; }
        control { display: block; width: 100px; height: 24px; }
    ";
    let panel_ref = NodeRef::new();
    let inert_btn_ref = NodeRef::new();
    let live_btn_ref = NodeRef::new();
    let log = Rc::new(RefCell::new(Vec::<&'static str>::new()));
    let log_i = Rc::clone(&log);
    let log_ik = Rc::clone(&log);
    let log_l = Rc::clone(&log);

    let mut harness = support::app_with_text(CSS, move |cx: &mut zgui_view::BuildCx<'_>| {
        use zgui_view::{IntoView, View};
        let log_i = Rc::clone(&log_i);
        let log_ik = Rc::clone(&log_ik);
        let log_l = Rc::clone(&log_l);
        Box::new(
            zgui_elements::column()
                .class("root")
                .child(
                    zgui_elements::column()
                        .class("panel")
                        .node_ref(panel_ref)
                        .child(
                            zgui_elements::control()
                                .node_ref(inert_btn_ref)
                                .on(events::CLICK, move |_| {
                                    log_i.borrow_mut().push("click:inert");
                                })
                                .on(events::KEY_DOWN, move |_| {
                                    log_ik.borrow_mut().push("key:inert");
                                }),
                        ),
                )
                .child(
                    zgui_elements::control()
                        .node_ref(live_btn_ref)
                        .on(events::CLICK, move |_| {
                            log_l.borrow_mut().push("click:live");
                        }),
                )
                .into_view()
                .build(cx),
        )
    });
    harness.settle(8);

    let panel_id = panel_ref.get().expect("panel");
    let inert_btn_id = inert_btn_ref.get().expect("inert btn");
    let live_btn_id = live_btn_ref.get().expect("live btn");

    // Register inert_btn as a window shortcut, then make its panel inert.
    harness.app().windows()[0]
        .host()
        .add_window_shortcut(inert_btn_id);
    set_inert(&mut harness, panel_id, true);
    harness.settle(4);

    // 1. Programmatic focus_with_reveal on inert node must not focus it.
    harness.app().windows()[0]
        .host()
        .focus_with_reveal(inert_btn_id, true);
    harness.settle(4);
    assert_eq!(focused(&harness), None);

    // 2. Window shortcut on inert node must not fire.
    harness.deliver_to_first(SurfaceEvent::Key {
        state: KeyState::Pressed,
        event: KeyEvent::named(NamedKey::Escape, PhysicalKey::Code(KeyCode::Escape)),
        modifiers: Modifiers::NONE,
        timestamp: Timestamp::ORIGIN,
    });
    harness.settle(4);
    assert!(log.borrow().is_empty());

    // 3. Inbound AccessKit actions targeting inert node return false and do nothing.
    let inert_key = zgui_view_dom::id::to_document(inert_btn_id).expect("doc key");
    let live_key = zgui_view_dom::id::to_document(live_btn_id).expect("doc key");
    let inert_a11y_id = zgui_a11y::id::to_a11y(inert_key);
    let live_a11y_id = zgui_a11y::id::to_a11y(live_key);

    let click_inert = ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: inert_a11y_id,
        data: None,
    };
    let focus_inert = ActionRequest {
        action: Action::Focus,
        target_tree: TreeId::ROOT,
        target_node: inert_a11y_id,
        data: None,
    };
    let click_live = ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: live_a11y_id,
        data: None,
    };
    {
        let window = &mut harness.app_mut().windows_mut()[0];
        assert!(
            !window.apply_a11y_action(&click_inert, Timestamp::ORIGIN),
            "AT click on inert node must be rejected"
        );
        assert!(
            !window.apply_a11y_action(&focus_inert, Timestamp::ORIGIN),
            "AT focus on inert node must be rejected"
        );
        assert!(
            window.apply_a11y_action(&click_live, Timestamp::ORIGIN),
            "AT click on live sibling must succeed"
        );
    }
    harness.settle(4);
    assert_eq!(*log.borrow(), vec!["click:live"]);
    assert_eq!(focused(&harness), None);
}

#[test]
fn same_batch_inert_mutation_blocks_capture_reacquisition_and_next_queued_event() {
    const CSS: &str = "
        .root { display: block; width: 200px; height: 200px; }
        control { display: block; width: 120px; height: 60px; }
    ";
    let btn_ref = NodeRef::new();
    let clicked = Rc::new(RefCell::new(0usize));
    let clicked_c = Rc::clone(&clicked);

    let mut harness = support::app_with_text(CSS, move |cx: &mut zgui_view::BuildCx<'_>| {
        use zgui_view::{IntoView, View};
        let clicked_c = Rc::clone(&clicked_c);
        let dom = cx.dom().clone();
        Box::new(
            zgui_elements::column()
                .class("root")
                .child(
                    zgui_elements::control()
                        .node_ref(btn_ref)
                        .on(events::POINTER_DOWN, move |cx| {
                            // Queue capture and mark self inert in the same listener turn.
                            cx.capture_pointer();
                            if let Some(id) = btn_ref.get_untracked() {
                                zgui_view::Dom::set_ui_state(&*dom, id, UiState::INERT, true);
                            }
                        })
                        .on(events::CLICK, move |_| {
                            *clicked_c.borrow_mut() += 1;
                        }),
                )
                .into_view()
                .build(cx),
        )
    });
    harness.settle(8);

    // Focus the button first.
    let btn_id = btn_ref.get().expect("btn mounted");
    harness.app().windows()[0].host().focus(btn_id);
    harness.settle(4);
    assert_eq!(focused(&harness), Some(btn_id));

    // Deliver PointerDown (sets inert + queues capture) AND Enter KeyDown AND PointerUp in ONE batch!
    harness.deliver_to_first(SurfaceEvent::Pointer {
        action: PointerAction::Pressed,
        event: pointer_event(40.0, 20.0, Some(PointerButton::Primary)),
        modifiers: Modifiers::NONE,
        timestamp: Timestamp::ORIGIN,
    });
    harness.deliver_to_first(SurfaceEvent::Key {
        state: KeyState::Pressed,
        event: KeyEvent::named(NamedKey::Enter, PhysicalKey::Code(KeyCode::Enter)),
        modifiers: Modifiers::NONE,
        timestamp: Timestamp::ORIGIN,
    });
    harness.deliver_to_first(SurfaceEvent::Pointer {
        action: PointerAction::Released,
        event: pointer_event(40.0, 20.0, Some(PointerButton::Primary)),
        modifiers: Modifiers::NONE,
        timestamp: Timestamp::ORIGIN,
    });
    harness.settle(4);

    assert!(
        !harness.app().windows()[0].has_pointer_capture(),
        "queued capture reacquisition on an inert node must be rejected"
    );
    assert_eq!(
        focused(&harness),
        None,
        "focus must evacuate before subsequent events in the same drain_input batch"
    );
    assert_eq!(
        *clicked.borrow(),
        0,
        "neither queued Enter nor PointerUp may activate a node made inert earlier in the batch"
    );
}

