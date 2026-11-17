//! These tests exercise real DocumentDom registrations and EventCx controls, not
//! the host's default editing/focus/activation pipeline (which needs host tests).
use std::{cell::RefCell, rc::Rc, time::Duration};

use rquickjs::{Context, Function, Object, Runtime, Value};
use ui_bridge::registry;
use zgui::{
    geom::{CssPx, Point},
    view::{DiscardCommands, DomHandle, ElementName, EventControl, EventCx, EventSink, Ident, NodeId},
    vocab::{AnimationEvent, AnimationPhase, DefaultAction, EventKind, FocusCause, FocusEvent, Key,
        KeyCode, KeyEvent, KeyLocation, Modifiers, NamedKey, Payload, Phase, PhysicalKey,
        PointerButton, PointerEvent, PointerId, PointerKind, Propagation, Pseudo, Timestamp,
        TransitionEvent, TransitionPhase, ValueChange, ValueEvent},
};
use zgui_dom::{Document, side::listeners::Listener};
use zgui_view_dom::DocumentDom;

// Exercise the crate-private helper without expanding the production public API.
#[allow(dead_code)]
#[path = "../src/events.rs"]
mod events;
use events::{Dispatcher, EventSnapshot, Events};

fn native() -> (Rc<DocumentDom>, DomHandle, NodeId, registry::Handle) {
    let backend = Rc::new(DocumentDom::new(Rc::new(RefCell::new(Document::new()))));
    let dom = DomHandle::from_rc(backend.clone());
    let root = dom.create_element(ElementName::new("control"));
    dom.insert(backend.root_node(), root, None);
    let registry = registry::NodeRegistry::new(dom.clone(), root, 1, 1, 100);
    (backend, dom, root, registry.root())
}

fn registrations(backend: &DocumentDom, node: NodeId) -> Vec<Listener> {
    let index = backend.index_of(node);
    let document = backend.document();
    let store = document.store();
    store.columns().listeners.get(store.key_of(index)).map(|listeners| listeners.iter().copied().collect()).unwrap_or_default()
}

#[test]
fn replacement_clearing_and_invalid_options_manage_real_native_listeners_and_roots() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let (backend, dom, node, owner) = native();
    let mut events = Events::new(dom);
    let delivered = Rc::new(RefCell::new(Vec::new()));
    let ids = delivered.clone();
    let dispatcher: Rc<Dispatcher> = Rc::new(move |id, _| ids.borrow_mut().push(id));
    context.with(|ctx| {
        let first: Value = ctx.eval("globalThis.calls = []; () => calls.push('first')").unwrap();
        events.set(&ctx, owner, node, "onClick", first, dispatcher.clone()).unwrap();
    });
    let first_registration = registrations(&backend, node)[0];
    let first_handler = backend.handler(first_registration.id).unwrap();
    let payload = Payload::Pointer(PointerEvent::mouse(Point::new(CssPx(1.0), CssPx(2.0))));
    let control = EventControl::new();
    let mut sink = DiscardCommands;
    let mut event = EventCx::new(EventKind::Click, node, node, Phase::Target, Modifiers::NONE, Timestamp::ORIGIN, &payload, &control, &mut sink);
    first_handler(&mut event);
    let first_id = delivered.borrow_mut().pop().unwrap();
    assert!(events.callback(first_id).is_some());
    context.with(|ctx| {
        events.set(&ctx, owner, node, "onClick", ctx.eval("() => calls.push('second')").unwrap(), dispatcher.clone()).unwrap();
        for script in ["({handleEvent() {}})", "[() => {}, {once:true}]", "true", "42"] {
            assert!(events.set(&ctx, owner, node, "onClick", ctx.eval(script).unwrap(), dispatcher.clone()).is_err());
            ctx.catch();
        }
        for name in ["onClickOnce", "onClickPassive", "on:click", "onClickCaptureCapture"] {
            assert!(events.set(&ctx, owner, node, name, ctx.eval("() => {}").unwrap(), dispatcher.clone()).is_err());
            ctx.catch();
        }
    });
    assert_eq!(events.len(), 1);
    assert_eq!(backend.handler_count(), 1);
    assert!(backend.handler(first_registration.id).is_none());
    assert!(events.callback(first_id).is_none());
    // Native dispatch may already have cloned a removed handler. Its old ID must
    // not resolve to the replacement's function.
    first_handler(&mut event);
    assert!(events.callback(delivered.borrow_mut().pop().unwrap()).is_none());
    let current = registrations(&backend, node)[0];
    backend.handler(current.id).unwrap()(&mut event);
    let callback = events.callback(delivered.borrow_mut().pop().unwrap()).unwrap();
    context.with(|ctx| {
        callback.restore(&ctx).unwrap().call::<_, ()>(()).unwrap();
        assert_eq!(ctx.eval::<String, _>("calls.join(',')").unwrap(), "second");
        events.set(&ctx, owner, node, "onClickCapture", ctx.eval("() => {}").unwrap(), dispatcher.clone()).unwrap();
        assert_eq!(events.len(), 2);
        events.set(&ctx, owner, node, "onClick", Value::new_null(ctx.clone()), dispatcher.clone()).unwrap();
        assert_eq!(events.len(), 1);
        events.set(&ctx, owner, node, "onClickCapture", Value::new_undefined(ctx.clone()), dispatcher.clone()).unwrap();
    });
    assert!(registrations(&backend, node).is_empty());
    assert_eq!(backend.handler_count(), 0);
    assert_eq!(events.len(), 0);
}

#[test]
fn capture_mapping_node_cleanup_clear_and_drop_remove_backend_handlers() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let (backend, dom, node, owner) = native();
    let other = dom.create_element(ElementName::new("control"));
    let other_owner = registry::NodeRegistry::new(dom.clone(), other, 2, 1, 10).root();
    // Each Events belongs to exactly one validated owner registry. Use a second
    // token from the first registry instead of a colliding cross-session token.
    let mut registry = registry::NodeRegistry::new(dom.clone(), node, 1, 1, 10);
    registry.begin_turn();
    let child = registry.create_element("control").unwrap();
    let child_node = registry.native(child).unwrap();
    let dispatcher: Rc<Dispatcher> = Rc::new(|_, _| {});
    let mut events = Events::new(dom);
    context.with(|ctx| {
        for (name, kind) in [("onClick", EventKind::Click), ("onPointerDown", EventKind::PointerDown),
            ("onPointerMove", EventKind::PointerMove), ("onPointerEnter", EventKind::PointerEnter),
            ("onPointerLeave", EventKind::PointerLeave), ("onPointerUp", EventKind::PointerUp),
            ("onPointerCancel", EventKind::PointerCancel), ("onContextMenu", EventKind::ContextMenu), ("onKeyDown", EventKind::KeyDown),
            ("onKeyUp", EventKind::KeyUp), ("onInput", EventKind::Input),
            ("onChange", EventKind::Change), ("onFocus", EventKind::FocusIn), ("onBlur", EventKind::FocusOut),
            ("onAnimationStart", EventKind::AnimationStart), ("onAnimationIteration", EventKind::AnimationIteration),
            ("onAnimationEnd", EventKind::AnimationEnd), ("onAnimationCancel", EventKind::AnimationCancel),
            ("onTransitionRun", EventKind::TransitionRun), ("onTransitionStart", EventKind::TransitionStart),
            ("onTransitionEnd", EventKind::TransitionEnd), ("onTransitionCancel", EventKind::TransitionCancel)] {
            let capture = format!("{name}Capture");
            events.set(&ctx, owner, node, &capture, ctx.eval("() => {}").unwrap(), dispatcher.clone()).unwrap();
            let listener = *registrations(&backend, node).last().unwrap();
            assert_eq!(listener.kind, kind);
            assert!(listener.options.capture);
            assert!(!listener.options.once);
            assert!(!listener.options.passive);
        }
        events.set(&ctx, child, child_node, "onClick", ctx.eval("() => {}").unwrap(), dispatcher.clone()).unwrap();
    });
    assert_eq!(backend.handler_count(), 23);
    events.remove(owner.token());
    assert_eq!(events.len(), 1);
    assert_eq!(backend.handler_count(), 1);
    assert!(registrations(&backend, node).is_empty());
    events.clear();
    assert_eq!(events.len(), 0);
    assert_eq!(backend.handler_count(), 0);
    context.with(|ctx| events.set(&ctx, other_owner, other, "onClick", ctx.eval("() => {}").unwrap(), dispatcher).unwrap());
    drop(events);
    assert_eq!(backend.handler_count(), 0);
}

#[test]
fn pointer_snapshot_is_owned_lossless_and_cancellation_expires_before_native_apply() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let (_, _, node, _) = native();
    let control = EventControl::new();
    let mut sink = DiscardCommands;
    let timestamp = Timestamp::from_origin(Duration::from_micros(12_500));
    let pointer = PointerEvent {
        id: PointerId::new(u64::MAX), kind: PointerKind::Pen, primary: false,
        position: Point::new(CssPx(12.5), CssPx(-4.0)),
        button: Some(PointerButton::Other(12)), pressure: Some(0.75),
    };
    let payload = Payload::Pointer(pointer);
    let samples = [pointer.sample(Timestamp::from_origin(Duration::from_millis(10)))];
    let mut event = EventCx::<zgui::view::events::AnyEvent>::new(EventKind::PointerDown, node, node, Phase::Capture,
        Modifiers::CONTROL | Modifiers::SHIFT, timestamp, &payload, &control, &mut sink).with_samples(&samples);
    let snapshot = EventSnapshot::capture(&event);
    assert_eq!(snapshot.target, node);
    assert_eq!(snapshot.current_target, node);
    let dispatch = context.with(|ctx| {
        let target = Object::new(ctx.clone()).unwrap();
        let current = Object::new(ctx.clone()).unwrap();
        ctx.globals().set("target", target.clone()).unwrap();
        ctx.globals().set("current", current.clone()).unwrap();
        let (object, dispatch) = snapshot.to_js(&ctx, &target, &current).unwrap();
        ctx.globals().set("event", object).unwrap();
        ctx.eval::<(), _>(r#"
            if (event.target !== target || event.currentTarget !== current) throw Error('wrappers');
            if (event.type !== 'pointerdown' || event.phase !== 'capture' || event.timestamp !== 12.5) throw Error('metadata');
            if (event.pointerId !== '18446744073709551615' || event.pointerKind !== 'pen' || event.primary) throw Error('pointer');
            if (event.position.x !== 12.5 || event.position.y !== -4 || event.button !== 'other' || event.buttonIndex !== 12 || event.pressure !== .75) throw Error('pointer payload');
            if (event.coalesced.length !== 1 || event.coalesced[0].timestamp !== 10 || event.coalesced[0].position.x !== 12.5) throw Error('samples');
            if (!event.modifiers.control || !event.modifiers.shift || event.modifiers.meta || event.modifiers.alt) throw Error('modifiers');
            event.preventDefault(); event.stopImmediatePropagation(); event.stopPropagation();
        "#).unwrap();
        dispatch
    });
    assert_eq!(control.default_action(), DefaultAction::Allowed);
    dispatch.expire_and_apply(&mut event);
    assert_eq!(control.default_action(), DefaultAction::Prevented);
    assert_eq!(control.propagation(), Propagation::StopImmediate);
    dispatch.expire_and_apply(&mut event);
    drop(dispatch);
    drop(snapshot);
    drop(payload);
    context.with(|ctx| {
        ctx.eval::<(), _>(r#"
            for (const name of ['preventDefault','stopPropagation','stopImmediatePropagation']) {
                let expired = false;
                try { event[name](); } catch (error) { expired = error instanceof ReferenceError && error.message.includes('expired-dispatch'); }
                if (!expired) throw Error('cancellation retained authority');
            }
            if (event.position.y !== -4 || event.coalesced[0].pressure !== .75) throw Error('snapshot expired');
        "#).unwrap();
    });
}

#[test]
fn pointer_boundaries_keep_snapshots_without_pointer_action_authority() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let (_, _, node, _) = native();
    for (kind, name) in [
        (EventKind::PointerEnter, "pointerenter"),
        (EventKind::PointerLeave, "pointerleave"),
        (EventKind::ContextMenu, "contextmenu"),
    ] {
        let payload = Payload::Pointer(PointerEvent::mouse(Point::new(CssPx(12.5), CssPx(4.0))));
        let control = EventControl::new();
        let mut sink = DiscardCommands;
        let mut native = EventCx::<zgui::view::events::AnyEvent>::new(kind, node, node, Phase::Target,
            Modifiers::NONE, Timestamp::ORIGIN, &payload, &control, &mut sink);
        let dispatch = context.with(|ctx| {
            let target = Object::new(ctx.clone()).unwrap();
            let (value, dispatch) = EventSnapshot::capture(&native).to_js(&ctx, &target, &target).unwrap();
            ctx.globals().set("boundary", value).unwrap();
            assert_eq!(ctx.eval::<String, _>("boundary.type").unwrap(), name);
            ctx.eval::<(), _>(r#"
                if (boundary.position.x !== 12.5 || boundary.position.y !== 4 || boundary.pointerKind !== 'mouse') throw Error('boundary snapshot');
                for (const method of ['capturePointer', 'releasePointer', 'activateNow']) {
                    let rejected = false;
                    try { boundary[method](); } catch (error) { rejected = error instanceof TypeError; }
                    if (!rejected) throw Error('boundary granted ' + method);
                }
                if (boundary.type === 'contextmenu') boundary.preventDefault();
            "#).unwrap();
            dispatch
        });
        assert_eq!(control.default_action(), DefaultAction::Allowed);
        dispatch.expire_and_apply(&mut native);
        assert_eq!(
            control.default_action(),
            if kind == EventKind::ContextMenu { DefaultAction::Prevented } else { DefaultAction::Allowed },
        );
        context.with(|ctx| {
            ctx.eval::<(), _>(r#"{
                let expired = false;
                try { boundary.stopPropagation(); } catch (error) { expired = error instanceof ReferenceError; }
                if (!expired || boundary.position.x !== 12.5) throw Error('boundary lifetime');
            }"#).unwrap();
        });
    }
}

#[test]
fn keyboard_value_and_focus_payloads_preserve_native_meaning() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let (_, _, node, _) = native();
    let mut key = KeyEvent::named(NamedKey::Enter, KeyCode::Enter.into());
    key.location = KeyLocation::Numpad;
    key.repeat = true;
    let mut dead = KeyEvent::character("x");
    dead.key = Key::Dead(Some('´'));
    dead.physical = PhysicalKey::Unidentified(123);
    let payloads = [
        (EventKind::KeyDown, Payload::Key(key)),
        (EventKind::KeyUp, Payload::Key(dead)),
        (EventKind::Input, Payload::Value(ValueEvent { value: "é🙂".into(), selection: 2..6, kind: ValueChange::Input })),
        (EventKind::Change, Payload::Value(ValueEvent::new("done", ValueChange::Committed))),
        (EventKind::FocusIn, Payload::Focus(FocusEvent::new(FocusCause::Keyboard))),
        (EventKind::FocusOut, Payload::Focus(FocusEvent::new(FocusCause::Window))),
    ];
    context.with(|ctx| {
        let target = Object::new(ctx.clone()).unwrap();
        let values = rquickjs::Array::new(ctx.clone()).unwrap();
        for (index, (kind, payload)) in payloads.iter().enumerate() {
            let control = EventControl::new();
            let mut sink = DiscardCommands;
            let native = EventCx::new(*kind, node, node, Phase::Bubble, Modifiers::NONE, Timestamp::ORIGIN, payload, &control, &mut sink);
            let (value, dispatch) = EventSnapshot::capture(&native).to_js(&ctx, &target, &target).unwrap();
            values.set(index, value).unwrap();
            drop(dispatch);
        }
        ctx.globals().set("events", values).unwrap();
        ctx.eval::<(), _>(r#"
            if (events[0].key !== 'Enter' || events[0].keyWithoutModifiers !== 'Enter' || events[0].code !== 'Enter' || events[0].location !== 'numpad' || !events[0].repeat) throw Error('key');
            if (events[1].key !== null || events[1].deadKey !== '´' || events[1].code !== null || events[1].physicalCode !== 123) throw Error('dead key');
            if (events[2].value !== 'é🙂' || events[2].selection.start !== 2 || events[2].selection.end !== 6 || events[2].valueChange !== 'input') throw Error('UTF-8 selection');
            if (events[3].valueChange !== 'committed' || events[3].selection.end !== 4) throw Error('change');
            if (events[4].type !== 'focusin' || events[4].cause !== 'keyboard' || events[5].type !== 'focusout' || events[5].cause !== 'window') throw Error('native focus');
        "#).unwrap();
    });
}

#[test]
fn thrown_handler_can_apply_flags_and_unwinding_drop_revokes_cancellation() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let (_, _, node, _) = native();
    let payload = Payload::Value(ValueEvent::new("value", ValueChange::Input));
    let control = EventControl::new();
    let mut sink = DiscardCommands;
    let mut native = EventCx::new(EventKind::Input, node, node, Phase::Target, Modifiers::NONE, Timestamp::ORIGIN, &payload, &control, &mut sink);
    let dispatch = context.with(|ctx| {
        let target = Object::new(ctx.clone()).unwrap();
        let (value, dispatch) = EventSnapshot::capture(&native).to_js(&ctx, &target, &target).unwrap();
        ctx.globals().set("saved", value.clone()).unwrap();
        let callback: Function = ctx.eval("event => { event.preventDefault(); event.stopPropagation(); throw Error('handler failed'); }").unwrap();
        assert!(callback.call::<_, ()>((value,)).is_err());
        ctx.catch();
        dispatch
    });
    dispatch.expire_and_apply(&mut native);
    assert_eq!(control.default_action(), DefaultAction::Prevented);
    assert_eq!(control.propagation(), Propagation::Stop);
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        context.with(|ctx| {
            let target = Object::new(ctx.clone()).unwrap();
            let (value, _dispatch) = EventSnapshot::capture(&native).to_js(&ctx, &target, &target).unwrap();
            ctx.globals().set("unwound", value).unwrap();
            panic!("simulate owner unwinding");
        });
    }));
    assert!(unwind.is_err());
    context.with(|ctx| ctx.eval::<(), _>(r#"
        for (const event of [saved, unwound]) {
            let expired = false;
            try { event.preventDefault(); } catch (error) { expired = error.message.includes('expired-dispatch'); }
            if (!expired) throw Error('error path retained cancellation');
            if (event.value !== 'value') throw Error('error path destroyed data');
        }
    "#).unwrap());
}

#[test]
fn animation_and_transition_snapshots_retain_payloads_and_expire_cancellation() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let (_, _, node, _) = native();
    let animation = Payload::Animation(AnimationEvent {
        name: Ident::new("fade"), elapsed: Duration::from_millis(125),
        phase: AnimationPhase::Ended, pseudo: Some(Pseudo::Before),
    });
    let control = EventControl::new();
    let mut sink = DiscardCommands;
    let mut native = EventCx::<zgui::view::events::AnyEvent>::new(EventKind::AnimationEnd, node, node,
        Phase::Target, Modifiers::NONE, Timestamp::from_origin(Duration::from_millis(7)),
        &animation, &control, &mut sink);
    let snapshot = EventSnapshot::capture(&native);
    let dispatch = context.with(|ctx| {
        let target = Object::new(ctx.clone()).unwrap();
        let (object, dispatch) = snapshot.to_js(&ctx, &target, &target).unwrap();
        ctx.globals().set("animation", object).unwrap();
        ctx.eval::<(), _>(r#"
            if (animation.type !== 'animationend' || animation.animationName !== 'fade' ||
                animation.elapsedTime !== .125 || animation.pseudoElement !== '::before' ||
                animation.timestamp !== 7) throw Error('animation payload');
            animation.preventDefault();
        "#).unwrap();
        dispatch
    });
    assert_eq!(control.default_action(), DefaultAction::Allowed);
    dispatch.expire_and_apply(&mut native);
    assert_eq!(control.default_action(), DefaultAction::Prevented);
    drop(dispatch);
    drop(snapshot);
    drop(animation);
    let payloads = [
        (EventKind::TransitionCancel, Payload::Transition(TransitionEvent {
            property: Ident::new("opacity"), elapsed: Duration::from_millis(250),
            phase: TransitionPhase::Cancelled, pseudo: None,
        })),
        (EventKind::AnimationStart, Payload::Animation(AnimationEvent {
            name: Ident::new("pulse"), elapsed: Duration::ZERO,
            phase: AnimationPhase::Started, pseudo: Some(Pseudo::After),
        })),
    ];
    context.with(|ctx| {
        let target = Object::new(ctx.clone()).unwrap();
        let values = rquickjs::Array::new(ctx.clone()).unwrap();
        for (index, (kind, payload)) in payloads.iter().enumerate() {
            let control = EventControl::new();
            let mut sink = DiscardCommands;
            let native = EventCx::<zgui::view::events::AnyEvent>::new(*kind, node, node, Phase::Bubble,
                Modifiers::NONE, Timestamp::ORIGIN, payload, &control, &mut sink);
            let (value, dispatch) = EventSnapshot::capture(&native).to_js(&ctx, &target, &target).unwrap();
            values.set(index, value).unwrap();
            drop(dispatch);
        }
        ctx.globals().set("events", values).unwrap();
        ctx.eval::<(), _>(r#"
            if (events[0].type !== 'transitioncancel' || events[0].propertyName !== 'opacity' ||
                events[0].elapsedTime !== .25 || events[0].pseudoElement !== '') throw Error('transition payload');
            if (events[1].type !== 'animationstart' || events[1].animationName !== 'pulse' ||
                events[1].elapsedTime !== 0 || events[1].pseudoElement !== '::after') throw Error('after marker');
            if (animation.animationName !== 'fade' || animation.elapsedTime !== .125 ||
                animation.pseudoElement !== '::before') throw Error('snapshot expired with native event');
            let expired = [false];
            try { animation.preventDefault(); } catch (error) {
                expired[0] = error instanceof ReferenceError && error.message.includes('expired-dispatch');
            }
            if (!expired[0]) throw Error('cancellation retained authority');
        "#).unwrap();
    });
}

#[derive(Default)]
struct Recorder {
    events: Rc<RefCell<Vec<(NodeId, EventKind)>>>,
}
impl EventSink for Recorder {
    fn capture_pointer(&mut self, _node: NodeId) {}
    fn release_pointer(&mut self, _node: NodeId) {}
    fn request_focus(&mut self, _node: NodeId) {}
    fn synthesize(&mut self, node: NodeId, event: EventKind) { self.events.borrow_mut().push((node, event)); }
}

#[test]
fn activate_now_requests_one_click_after_expiry_and_rejects_other_kinds() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let (_, _, node, _) = native();

    let payload = Payload::Pointer(PointerEvent::mouse(Point::new(CssPx(1.0), CssPx(2.0))));
    let control = EventControl::new();
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut sink = Recorder { events: log.clone() };
    let mut down = EventCx::new(EventKind::PointerDown, node, node, Phase::Target,
        Modifiers::NONE, Timestamp::ORIGIN, &payload, &control, &mut sink);
    let dispatch = context.with(|ctx| {
        let target = Object::new(ctx.clone()).unwrap();
        let (object, dispatch) = EventSnapshot::capture(&down).to_js(&ctx, &target, &target).unwrap();
        ctx.globals().set("event", object).unwrap();
        ctx.eval::<(), _>("event.activateNow(); event.activateNow(); 'armed'").unwrap();
        dispatch
    });
    assert!(!control.activation_claimed());
    assert!(log.borrow().is_empty());
    dispatch.expire_and_apply(&mut down);
    assert!(control.activation_claimed());
    dispatch.expire_and_apply(&mut down);
    drop(dispatch);
    assert_eq!(*log.borrow(), vec![(node, EventKind::Click)]);

    context.with(|ctx| {
        ctx.eval::<(), _>(r#"
            let expired = false;
            try { event.activateNow(); } catch (error) { expired = error instanceof ReferenceError && error.message.includes('expired-dispatch'); }
            if (!expired) throw Error('activateNow retained authority');
            'expired'
        "#).unwrap();
    });

    let up_payload = Payload::Pointer(PointerEvent::mouse(Point::new(CssPx(3.0), CssPx(4.0))));
    let up_control = EventControl::new();
    let up_log = Rc::new(RefCell::new(Vec::new()));
    let mut up_sink = Recorder { events: up_log.clone() };
    let up = EventCx::new(EventKind::PointerUp, node, node, Phase::Target,
        Modifiers::NONE, Timestamp::ORIGIN, &up_payload, &up_control, &mut up_sink);
    context.with(|ctx| {
        let target = Object::new(ctx.clone()).unwrap();
        let (object, dispatch) = EventSnapshot::capture(&up).to_js(&ctx, &target, &target).unwrap();
        ctx.globals().set("up", object).unwrap();
        ctx.eval::<(), _>(r#"
            if (typeof up.activateNow !== 'function') throw Error('pointer snapshot lost activateNow');
            let rejected = false;
            try { up.activateNow(); } catch (error) { rejected = error instanceof TypeError; }
            if (!rejected) throw Error('activateNow accepted a pointerup event');
            'rejected'
        "#).unwrap();
        drop(dispatch);
    });
    assert!(up_log.borrow().is_empty());
    assert!(!up_control.activation_claimed());

    let drop_control = EventControl::new();
    let drop_log = Rc::new(RefCell::new(Vec::new()));
    let mut drop_sink = Recorder { events: drop_log.clone() };
    let dropped = EventCx::new(EventKind::PointerDown, node, node, Phase::Target,
        Modifiers::NONE, Timestamp::ORIGIN, &payload, &drop_control, &mut drop_sink);
    context.with(|ctx| {
        let target = Object::new(ctx.clone()).unwrap();
        let (object, dispatch) = EventSnapshot::capture(&dropped).to_js(&ctx, &target, &target).unwrap();
        ctx.globals().set("dropped", object).unwrap();
        ctx.eval::<(), _>("dropped.activateNow(); 'armed'").unwrap();
        drop(dispatch);
    });
    assert!(drop_log.borrow().is_empty());
    assert!(!drop_control.activation_claimed());
}

#[derive(Default)]
struct PointerRecorder {
    log: Rc<RefCell<Vec<(NodeId, bool)>>>,
}
impl EventSink for PointerRecorder {
    fn capture_pointer(&mut self, node: NodeId) { self.log.borrow_mut().push((node, true)); }
    fn release_pointer(&mut self, node: NodeId) { self.log.borrow_mut().push((node, false)); }
    fn request_focus(&mut self, _node: NodeId) {}
    fn synthesize(&mut self, _node: NodeId, _event: EventKind) {}
}

#[test]
fn pointer_capture_requests_apply_once_to_the_current_node_after_expiry() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let (backend, dom, node, _) = native();
    let other = dom.create_element(ElementName::new("control"));
    dom.insert(node, other, None);
    let _ = backend;
    let payload = Payload::Pointer(PointerEvent::mouse(Point::new(CssPx(1.0), CssPx(2.0))));
    let control = EventControl::new();
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut sink = PointerRecorder { log: log.clone() };
    let mut down = EventCx::new(EventKind::PointerDown, other, node, Phase::Bubble,
        Modifiers::NONE, Timestamp::ORIGIN, &payload, &control, &mut sink);
    let dispatch = context.with(|ctx| {
        let target = Object::new(ctx.clone()).unwrap();
        let current = Object::new(ctx.clone()).unwrap();
        let (object, dispatch) = EventSnapshot::capture(&down).to_js(&ctx, &target, &current).unwrap();
        ctx.globals().set("event", object).unwrap();
        ctx.eval::<(), _>("event.capturePointer(); event.capturePointer(); 'armed'").unwrap();
        dispatch
    });
    assert!(log.borrow().is_empty(), "capture is issued only after the handler returns");
    dispatch.expire_and_apply(&mut down);
    assert_eq!(*log.borrow(), vec![(node, true)], "the request uses the current node, not the target");
    dispatch.expire_and_apply(&mut down);
    assert_eq!(*log.borrow(), vec![(node, true)], "a second apply adds nothing");
    drop(dispatch);

    for (script, wanted) in [
        ("held.capturePointer(); held.releasePointer(); 'a'", false),
        ("held.releasePointer(); held.capturePointer(); 'b'", true),
    ] {
        let control = EventControl::new();
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut sink = PointerRecorder { log: log.clone() };
        let mut event = EventCx::new(EventKind::PointerDown, other, node, Phase::Bubble,
            Modifiers::NONE, Timestamp::ORIGIN, &payload, &control, &mut sink);
        let dispatch = context.with(|ctx| {
            let target = Object::new(ctx.clone()).unwrap();
            let (object, dispatch) = EventSnapshot::capture(&event).to_js(&ctx, &target, &target).unwrap();
            ctx.globals().set("held", object).unwrap();
            ctx.eval::<(), _>(script).unwrap();
            dispatch
        });
        dispatch.expire_and_apply(&mut event);
        assert_eq!(*log.borrow(), vec![(node, wanted)], "the last request in a handler wins");
        drop(dispatch);
    }

    for kind in [EventKind::PointerMove, EventKind::PointerUp, EventKind::PointerCancel] {
        let control = EventControl::new();
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut sink = PointerRecorder { log: log.clone() };
        let mut event = EventCx::new(kind, other, node, Phase::Bubble,
            Modifiers::NONE, Timestamp::ORIGIN, &payload, &control, &mut sink);
        let dispatch = context.with(|ctx| {
            let target = Object::new(ctx.clone()).unwrap();
            let (object, dispatch) = EventSnapshot::capture(&event).to_js(&ctx, &target, &target).unwrap();
            ctx.globals().set("held", object).unwrap();
            ctx.eval::<(), _>(r#"
                held.releasePointer();
                { let rejected = false;
                try { held.capturePointer(); } catch (error) { rejected = error instanceof TypeError; }
                if (!rejected) throw Error('capturePointer accepted a non-press pointer event'); }
                'checked'
            "#).unwrap();
            dispatch
        });
        dispatch.expire_and_apply(&mut event);
        assert_eq!(*log.borrow(), vec![(node, false)]);
        drop(dispatch);
    }

    let click_control = EventControl::new();
    let click_log = Rc::new(RefCell::new(Vec::new()));
    let mut click_sink = PointerRecorder { log: click_log.clone() };
    let click = EventCx::new(EventKind::Click, other, node, Phase::Bubble,
        Modifiers::NONE, Timestamp::ORIGIN, &payload, &click_control, &mut click_sink);
    context.with(|ctx| {
        let target = Object::new(ctx.clone()).unwrap();
        let (object, dispatch) = EventSnapshot::capture(&click).to_js(&ctx, &target, &target).unwrap();
        ctx.globals().set("clicked", object).unwrap();
        ctx.eval::<(), _>(r#"
            { let rejected = false;
            try { clicked.releasePointer(); } catch (error) { rejected = error instanceof TypeError; }
            if (!rejected) throw Error('releasePointer accepted a click'); }
            { let rejected = false;
            try { clicked.capturePointer(); } catch (error) { rejected = error instanceof TypeError; }
            if (!rejected) throw Error('capturePointer accepted a click'); }
            'rejected'
        "#).unwrap();
        drop(dispatch);
    });
    assert!(click_log.borrow().is_empty());

    context.with(|ctx| {
        ctx.eval::<(), _>(r#"
            for (const name of ['capturePointer', 'releasePointer']) {
                { let expired = false;
                try { event[name](); } catch (error) { expired = error instanceof ReferenceError && error.message.includes('expired-dispatch'); }
                if (!expired) throw Error(`${name} retained authority`); }
            }
            if (event.pointerId !== '0' || event.position.x !== 1 || event.position.y !== 2) throw Error('pointer snapshot expired');
            'expired'
        "#).unwrap();
    });
}

#[test]
fn dropped_dispatch_issues_no_pointer_capture_commands() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let (_, _, node, _) = native();
    let payload = Payload::Pointer(PointerEvent::mouse(Point::new(CssPx(1.0), CssPx(2.0))));
    let control = EventControl::new();
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut sink = PointerRecorder { log: log.clone() };
    let down = EventCx::new(EventKind::PointerDown, node, node, Phase::Target,
        Modifiers::NONE, Timestamp::ORIGIN, &payload, &control, &mut sink);
    context.with(|ctx| {
        let target = Object::new(ctx.clone()).unwrap();
        let (object, dispatch) = EventSnapshot::capture(&down).to_js(&ctx, &target, &target).unwrap();
        ctx.globals().set("lost", object).unwrap();
        ctx.eval::<(), _>("lost.capturePointer(); 'armed'").unwrap();
        drop(dispatch);
    });
    assert!(log.borrow().is_empty());
    context.with(|ctx| {
        ctx.eval::<(), _>(r#"
            { let expired = false;
            try { lost.capturePointer(); } catch (error) { expired = error instanceof ReferenceError && error.message.includes('expired-dispatch'); }
            if (!expired) throw Error('capturePointer retained authority after drop'); }
            'expired'
        "#).unwrap();
    });
}

#[test]
fn keyboard_character_text_is_explicit_and_survives_dispatch() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let (_, _, node, _) = native();
    let keys = [
        Key::character("é"), Key::character("e\u{301}"), Key::character("𐐷"),
        Key::character("Enter"), Key::character(" "),
        Key::Named(NamedKey::Enter), Key::Named(NamedKey::Space),
        Key::Dead(Some('´')), Key::Other("x".into()), Key::Unidentified,
    ];
    context.with(|ctx| {
        let target = Object::new(ctx.clone()).unwrap();
        let values = rquickjs::Array::new(ctx.clone()).unwrap();
        for (index, key) in keys.into_iter().enumerate() {
            let mut key_event = KeyEvent::character("unused");
            key_event.key_without_modifiers = key.clone();
            key_event.key = key;
            let payload = Payload::Key(key_event);
            let control = EventControl::new();
            let mut sink = DiscardCommands;
            let mut event = EventCx::new(EventKind::KeyDown, node, node, Phase::Bubble,
                Modifiers::NONE, Timestamp::ORIGIN, &payload, &control, &mut sink);
            let (value, dispatch) = EventSnapshot::capture(&event).to_js(&ctx, &target, &target).unwrap();
            dispatch.expire_and_apply(&mut event);
            values.set(index, value).unwrap();
        }
        ctx.globals().set("characters", values).unwrap();
        ctx.eval::<(), _>(r#"
            const expected = ['é', 'é', '𐐷', 'Enter', ' ', null, null, null, null, null];
            for (let i = 0; i < expected.length; i++) {
                const event = characters[i];
                if (event.character !== expected[i]) throw Error('character category ' + i);
                if (i < 5 && event.key !== expected[i]) throw Error('character changed key');
                let expired = false;
                try { event.preventDefault(); } catch (error) { expired = error instanceof ReferenceError; }
                if (!expired) throw Error('dispatch authority survived');
            }
            if (characters[3].key !== characters[5].key || characters[4].key !== characters[6].key)
                throw Error('test must cover identical key strings with different categories');
        "#).unwrap();
    });
}
