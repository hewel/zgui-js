//! These tests exercise real DocumentDom registrations and EventCx controls, not
//! the host's default editing/focus/activation pipeline (which needs host tests).
use std::{cell::RefCell, rc::Rc, time::Duration};

use rquickjs::{Context, Function, Object, Runtime, Value};
use ui_bridge::registry;
use zgui::{
    geom::{CssPx, Point},
    view::{DiscardCommands, DomHandle, ElementName, EventControl, EventCx, NodeId},
    vocab::{DefaultAction, EventKind, FocusCause, FocusEvent, Key, KeyCode, KeyEvent,
        KeyLocation, Modifiers, NamedKey, Payload, Phase, PhysicalKey, PointerButton,
        PointerEvent, PointerId, PointerKind, Propagation, Timestamp, ValueChange, ValueEvent},
};
use zgui_dom::{Document, side::listeners::Listener};
use zgui_view_dom::DocumentDom;

// Exercise the crate-private helper without expanding the production public API.
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
            ("onPointerUp", EventKind::PointerUp), ("onKeyDown", EventKind::KeyDown),
            ("onKeyUp", EventKind::KeyUp), ("onInput", EventKind::Input),
            ("onChange", EventKind::Change), ("onFocus", EventKind::FocusIn), ("onBlur", EventKind::FocusOut)] {
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
    assert_eq!(backend.handler_count(), 10);
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
    drop(event);
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
