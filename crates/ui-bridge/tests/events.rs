//! Integration tests crossing the public [`UiBridge`] and [`HostPort`] interface
//! with real `DocumentDom` registrations and `EventCx` controls.
use std::{cell::{Cell, RefCell}, rc::Rc, time::Duration};

use js_runtime::{JsRuntime, RuntimeLimits};
use ui_bridge::{
    BridgeStep, DispatchOutcome, HostPort, MAX_QUEUED_NOTIFICATIONS, QueueOverflow, UiBridge,
    registry::NodeRegistry,
};
use zgui::{
    geom::{CssPx, Point},
    view::{
        DiscardCommands, DomHandle, ElementName, EventControl, EventCx, EventSink, HostHandle,
        Ident, NodeId, stub::StubHost,
    },
    vocab::{
        AnimationEvent, AnimationPhase, DefaultAction, EventKind, FocusCause, FocusEvent, Key,
        KeyCode, KeyEvent, KeyLocation, Modifiers, NamedKey, Payload, Phase, PhysicalKey,
        PointerButton, PointerEvent, PointerId, PointerKind, Propagation, Pseudo, Timestamp,
        TransitionEvent, TransitionPhase, ValueChange, ValueEvent,
    },
};
use zgui_dom::{Document, side::listeners::Listener};
use zgui_view_dom::DocumentDom;

struct BridgeRunner {
    engine: RefCell<JsRuntime>,
    last_error: RefCell<Option<String>>,
    wakes: Cell<usize>,
    force_busy: Cell<bool>,
    force_fault: Cell<bool>,
}

impl BridgeRunner {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            engine: RefCell::new(JsRuntime::new(RuntimeLimits::default()).unwrap()),
            last_error: RefCell::new(None),
            wakes: Cell::new(0),
            force_busy: Cell::new(false),
            force_fault: Cell::new(false),
        })
    }

    fn enter(&self, step: BridgeStep<'_>) -> DispatchOutcome {
        self.wakes.set(self.wakes.get() + 1);
        if self.force_fault.get() {
            return DispatchOutcome::Faulted;
        }
        if self.force_busy.get() {
            return DispatchOutcome::Busy;
        }
        let Ok(mut engine) = self.engine.try_borrow_mut() else {
            return DispatchOutcome::Busy;
        };
        if let Err(err) = step.run_turn(engine.turn(), |js, cb| js.with_context(|ctx| cb.call(&ctx))) {
            *self.last_error.borrow_mut() = Some(err.to_string());
        }
        DispatchOutcome::Executed
    }
}

struct EventHarness {
    bridge: Rc<UiBridge>,
    runner: Rc<BridgeRunner>,
    backend: Rc<DocumentDom>,
    mount: NodeId,
}

impl EventHarness {
    fn synchronous(session: u64, limit: usize) -> Self {
        let backend = Rc::new(DocumentDom::new(Rc::new(RefCell::new(Document::new()))));
        let dom = DomHandle::from_rc(backend.clone());
        let mount = dom.create_element(ElementName::new("control"));
        dom.insert(backend.root_node(), mount, None);
        let runner = BridgeRunner::new();
        let port = HostPort::synchronous(Rc::downgrade(&runner), BridgeRunner::enter);
        let bridge = UiBridge::new(
            NodeRegistry::new(dom, mount, session, 1, limit),
            HostHandle::new(StubHost::default()),
            port,
        );
        runner
            .engine
            .borrow_mut()
            .with_context(|ctx| {
                bridge.install(&ctx)?;
                ctx.globals().set("root", bridge.root(&ctx)?)
            })
            .unwrap();
        Self { bridge, runner, backend, mount }
    }

    fn queued(session: u64, limit: usize) -> Self {
        let backend = Rc::new(DocumentDom::new(Rc::new(RefCell::new(Document::new()))));
        let dom = DomHandle::from_rc(backend.clone());
        let mount = dom.create_element(ElementName::new("control"));
        dom.insert(backend.root_node(), mount, None);
        let runner = BridgeRunner::new();
        let bridge = UiBridge::new(
            NodeRegistry::new(dom, mount, session, 1, limit),
            HostHandle::new(StubHost::default()),
            HostPort::queued(),
        );
        runner
            .engine
            .borrow_mut()
            .with_context(|ctx| {
                bridge.install(&ctx)?;
                ctx.globals().set("root", bridge.root(&ctx)?)
            })
            .unwrap();
        Self { bridge, runner, backend, mount }
    }

    fn script(&self, source: &str) {
        let mut engine = self.runner.engine.borrow_mut();
        self.bridge
            .run_turn(engine.turn(), |_, js| js.evaluate_script(source))
            .unwrap();
    }

    fn eval(&self, source: &str) -> String {
        self.runner.engine.borrow_mut().evaluate_script(source).unwrap()
    }

    fn flush_all(&self) -> usize {
        let mut count = 0;
        let mut engine = self.runner.engine.borrow_mut();
        while self
            .bridge
            .flush_notification(engine.turn(), |js, cb| js.with_context(|ctx| cb.call(&ctx)))
            .unwrap()
        {
            count += 1;
        }
        count
    }
}

fn registrations(backend: &DocumentDom, node: NodeId) -> Vec<Listener> {
    let index = backend.index_of(node);
    let document = backend.document();
    let store = document.store();
    store
        .columns()
        .listeners
        .get(store.key_of(index))
        .map(|listeners| listeners.iter().copied().collect())
        .unwrap_or_default()
}

#[test]
fn replacement_clearing_and_invalid_options_manage_real_native_listeners_and_roots() {
    let h = EventHarness::synchronous(1, 100);
    h.script("globalThis.calls = []; __zgui.setProperty(root, 'onClick', () => calls.push('first')); 'armed'");
    let first_reg = registrations(&h.backend, h.mount)[0];
    let first_handler = h.backend.handler(first_reg.id).unwrap();

    let payload = Payload::Pointer(PointerEvent::mouse(Point::new(CssPx(1.0), CssPx(2.0))));
    let control = EventControl::new();
    let mut sink = DiscardCommands;
    let mut event = EventCx::new(
        EventKind::Click,
        h.mount,
        h.mount,
        Phase::Target,
        Modifiers::NONE,
        Timestamp::ORIGIN,
        &payload,
        &control,
        &mut sink,
    );
    first_handler(&mut event);
    assert_eq!(h.eval("calls.join(',')"), "first");

    h.script(r#"
        __zgui.setProperty(root, 'onClick', () => calls.push('second'));
        for (const bad of [{ handleEvent() {} }, [() => {}, { once: true }], true, 42]) {
            let rejected = false;
            try { __zgui.setProperty(root, 'onClick', bad); } catch { rejected = true; }
            if (!rejected) throw Error('invalid listener accepted');
        }
        for (const name of ['onClickOnce', 'onClickPassive', 'on:click', 'onClickCaptureCapture']) {
            let rejected = false;
            try { __zgui.setProperty(root, name, () => {}); } catch { rejected = true; }
            if (!rejected) throw Error('invalid event name accepted: ' + name);
        }
        'replaced'
    "#);
    assert_eq!(h.bridge.callback_count(), 1);
    assert_eq!(h.backend.handler_count(), 1);
    assert!(h.backend.handler(first_reg.id).is_none());

    // A cloned handler from before replacement is detached from the backend.
    first_handler(&mut event);
    assert_eq!(h.eval("calls.join(',')"), "first");

    let current = registrations(&h.backend, h.mount)[0];
    h.backend.handler(current.id).unwrap()(&mut event);
    assert_eq!(h.eval("calls.join(',')"), "first,second");

    h.script(r#"
        __zgui.setProperty(root, 'onClickCapture', () => {});
        __zgui.setProperty(root, 'onClick', null);
        'cleared'
    "#);
    assert_eq!(h.bridge.callback_count(), 1);
    h.script("__zgui.setProperty(root, 'onClickCapture', undefined); 'empty'");
    assert!(registrations(&h.backend, h.mount).is_empty());
    assert_eq!(h.backend.handler_count(), 0);
    assert_eq!(h.bridge.callback_count(), 0);
}

#[test]
fn capture_mapping_node_cleanup_clear_and_drop_remove_backend_handlers() {
    let h = EventHarness::synchronous(2, 100);
    h.script(r#"
        const box = __zgui.createElement('control');
        __zgui.insertNode(root, box, null);
        for (const name of [
            'onClick', 'onPointerDown', 'onPointerMove', 'onPointerEnter',
            'onPointerLeave', 'onPointerUp', 'onPointerCancel', 'onContextMenu',
            'onKeyDown', 'onKeyUp', 'onInput', 'onChange', 'onFocus', 'onBlur',
            'onAnimationStart', 'onAnimationIteration', 'onAnimationEnd', 'onAnimationCancel',
            'onTransitionRun', 'onTransitionStart', 'onTransitionEnd', 'onTransitionCancel',
        ]) {
            __zgui.setProperty(root, name + 'Capture', () => {});
        }
        __zgui.setProperty(box, 'onClick', () => {});
        globalThis.box = box;
        'registered'
    "#);
    assert_eq!(h.backend.handler_count(), 23);
    assert_eq!(h.bridge.callback_count(), 23);

    h.script("__zgui.removeNode(root, box); 'removed'");
    assert_eq!(h.bridge.callback_count(), 22);
    assert_eq!(h.backend.handler_count(), 22);

    h.bridge.dispose();
    assert_eq!(h.bridge.callback_count(), 0);
    assert_eq!(h.backend.handler_count(), 0);
    assert!(registrations(&h.backend, h.mount).is_empty());
}

#[test]
fn pointer_snapshot_is_owned_lossless_and_cancellation_expires_before_native_apply() {
    let h = EventHarness::synchronous(3, 100);
    h.script(r#"
        const box = __zgui.createElement('control');
        __zgui.insertNode(root, box, null);
        globalThis.box = box;
        __zgui.setProperty(box, 'onPointerDownCapture', event => {
            globalThis.saved = event;
            if (event.target !== box || event.currentTarget !== box) throw Error('wrappers');
            if (event.type !== 'pointerdown' || event.phase !== 'capture' || event.timestamp !== 12.5) throw Error('metadata');
            if (event.pointerId !== '18446744073709551615' || event.pointerKind !== 'pen' || event.primary) throw Error('pointer');
            if (event.position.x !== 12.5 || event.position.y !== -4 || event.button !== 'other' || event.buttonIndex !== 12 || event.pressure !== 0.75) throw Error('pointer payload');
            if (event.coalesced.length !== 1 || event.coalesced[0].timestamp !== 10 || event.coalesced[0].position.x !== 12.5) throw Error('samples');
            if (!event.modifiers.control || !event.modifiers.shift || event.modifiers.meta || event.modifiers.alt) throw Error('modifiers');
            event.preventDefault();
            event.stopImmediatePropagation();
            event.stopPropagation();
        });
        'armed'
    "#);
    let node = h.backend.children(h.mount)[0];
    let handler = h.backend.handler(registrations(&h.backend, node)[0].id).unwrap();

    let control = EventControl::new();
    let mut sink = DiscardCommands;
    let timestamp = Timestamp::from_origin(Duration::from_micros(12_500));
    let pointer = PointerEvent {
        id: PointerId::new(u64::MAX),
        kind: PointerKind::Pen,
        primary: false,
        position: Point::new(CssPx(12.5), CssPx(-4.0)),
        button: Some(PointerButton::Other(12)),
        pressure: Some(0.75),
    };
    let payload = Payload::Pointer(pointer);
    let samples = [pointer.sample(Timestamp::from_origin(Duration::from_millis(10)))];
    let mut event = EventCx::<zgui::view::events::AnyEvent>::new(
        EventKind::PointerDown,
        node,
        node,
        Phase::Capture,
        Modifiers::CONTROL | Modifiers::SHIFT,
        timestamp,
        &payload,
        &control,
        &mut sink,
    )
    .with_samples(&samples);
    handler(&mut event);
    assert!(h.runner.last_error.borrow().is_none());
    assert_eq!(control.default_action(), DefaultAction::Prevented);
    assert_eq!(control.propagation(), Propagation::StopImmediate);
    drop(payload);

    assert_eq!(h.eval(r#"
        for (const name of ['preventDefault', 'stopPropagation', 'stopImmediatePropagation']) {
            let expired = false;
            try { saved[name](); }
            catch (error) { expired = error instanceof ReferenceError && error.message.includes('expired-dispatch'); }
            if (!expired) throw Error('cancellation retained authority');
        }
        if (saved.position.y !== -4 || saved.coalesced[0].pressure !== 0.75) throw Error('snapshot expired');
        'verified'
    "#), "verified");
}

#[test]
fn pointer_boundaries_keep_snapshots_without_pointer_action_authority() {
    for (prop, kind, name) in [
        ("onPointerEnter", EventKind::PointerEnter, "pointerenter"),
        ("onPointerLeave", EventKind::PointerLeave, "pointerleave"),
        ("onContextMenu", EventKind::ContextMenu, "contextmenu"),
    ] {
        let h = EventHarness::synchronous(4, 100);
        h.script(&format!(r#"
            __zgui.setProperty(root, '{prop}', event => {{
                globalThis.boundary = event;
                if (event.type !== '{name}') throw Error('type');
                if (event.position.x !== 12.5 || event.position.y !== 4 || event.pointerKind !== 'mouse') throw Error('snapshot');
                for (const method of ['capturePointer', 'releasePointer', 'activateNow']) {{
                    let rejected = false;
                    try {{ event[method](); }} catch (error) {{ rejected = error instanceof TypeError; }}
                    if (!rejected) throw Error('boundary granted ' + method);
                }}
                if (event.type === 'contextmenu') event.preventDefault();
            }});
            'armed'
        "#));
        let handler = h.backend.handler(registrations(&h.backend, h.mount)[0].id).unwrap();
        let payload = Payload::Pointer(PointerEvent::mouse(Point::new(CssPx(12.5), CssPx(4.0))));
        let control = EventControl::new();
        let mut sink = DiscardCommands;
        let mut native = EventCx::<zgui::view::events::AnyEvent>::new(
            kind,
            h.mount,
            h.mount,
            Phase::Target,
            Modifiers::NONE,
            Timestamp::ORIGIN,
            &payload,
            &control,
            &mut sink,
        );
        handler(&mut native);
        assert!(h.runner.last_error.borrow().is_none());
        assert_eq!(
            control.default_action(),
            if kind == EventKind::ContextMenu { DefaultAction::Prevented } else { DefaultAction::Allowed },
        );
        assert_eq!(h.eval(r#"
            let expired = false;
            try { boundary.stopPropagation(); } catch (error) { expired = error instanceof ReferenceError; }
            if (!expired || boundary.position.x !== 12.5) throw Error('boundary lifetime');
            'verified'
        "#), "verified");
    }
}

#[test]
fn keyboard_value_and_focus_payloads_preserve_native_meaning() {
    let h = EventHarness::synchronous(5, 100);
    h.script(r#"
        globalThis.events = [];
        for (const prop of ['onKeyDown', 'onKeyUp', 'onInput', 'onChange', 'onFocus', 'onBlur']) {
            __zgui.setProperty(root, prop, event => events.push(event));
        }
        'armed'
    "#);
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
    let regs = registrations(&h.backend, h.mount);
    for (kind, payload) in &payloads {
        let reg = regs.iter().find(|r| r.kind == *kind).unwrap();
        let control = EventControl::new();
        let mut sink = DiscardCommands;
        let mut native = EventCx::new(
            *kind,
            h.mount,
            h.mount,
            Phase::Bubble,
            Modifiers::NONE,
            Timestamp::ORIGIN,
            payload,
            &control,
            &mut sink,
        );
        h.backend.handler(reg.id).unwrap()(&mut native);
    }
    assert_eq!(h.eval(r#"
        if (events[0].key !== 'Enter' || events[0].keyWithoutModifiers !== 'Enter' || events[0].code !== 'Enter' || events[0].location !== 'numpad' || !events[0].repeat) throw Error('key');
        if (events[1].key !== null || events[1].deadKey !== '´' || events[1].code !== null || events[1].physicalCode !== 123) throw Error('dead key');
        if (events[2].value !== 'é🙂' || events[2].selection.start !== 2 || events[2].selection.end !== 6 || events[2].valueChange !== 'input') throw Error('UTF-8 selection');
        if (events[3].valueChange !== 'committed' || events[3].selection.end !== 4) throw Error('change');
        if (events[4].type !== 'focusin' || events[4].cause !== 'keyboard' || events[5].type !== 'focusout' || events[5].cause !== 'window') throw Error('native focus');
        'verified'
    "#), "verified");
}

#[test]
fn thrown_handler_applies_prethrow_flags_disposes_bridge_and_revokes_cancellation() {
    let h = EventHarness::synchronous(6, 100);
    h.script(r#"
        __zgui.installStylesheet('leak', 'box { width: 1px; }');
        __zgui.setProperty(root, 'onInput', event => {
            globalThis.saved = event;
            event.preventDefault();
            event.stopPropagation();
            throw Error('handler failed');
        });
        'armed'
    "#);
    let handler = h.backend.handler(registrations(&h.backend, h.mount)[0].id).unwrap();
    let payload = Payload::Value(ValueEvent::new("value", ValueChange::Input));
    let control = EventControl::new();
    let mut sink = DiscardCommands;
    let mut native = EventCx::new(
        EventKind::Input,
        h.mount,
        h.mount,
        Phase::Target,
        Modifiers::NONE,
        Timestamp::ORIGIN,
        &payload,
        &control,
        &mut sink,
    );
    handler(&mut native);
    assert!(h.runner.last_error.borrow().as_ref().is_some_and(|e| e.contains("handler failed")));
    assert!(h.bridge.is_disposed());
    assert_eq!(h.backend.handler_count(), 0);
    assert_eq!(control.default_action(), DefaultAction::Prevented);
    assert_eq!(control.propagation(), Propagation::Stop);
    assert_eq!(h.eval(r#"
        let expired = false;
        try { saved.preventDefault(); } catch (error) { expired = error.message.includes('expired-dispatch'); }
        if (!expired) throw Error('error path retained cancellation');
        if (saved.value !== 'value') throw Error('error path destroyed data');
        'verified'
    "#), "verified");
}

#[test]
fn animation_and_transition_snapshots_retain_payloads_and_expire_cancellation() {
    let h = EventHarness::synchronous(7, 100);
    h.script(r#"
        globalThis.events = [];
        for (const prop of ['onAnimationEnd', 'onTransitionCancel', 'onAnimationStart']) {
            __zgui.setProperty(root, prop, event => {
                if (event.type === 'animationend') event.preventDefault();
                events.push(event);
            });
        }
        'armed'
    "#);
    let regs = registrations(&h.backend, h.mount);

    let anim_end = Payload::Animation(AnimationEvent {
        name: Ident::new("fade"),
        elapsed: Duration::from_millis(125),
        phase: AnimationPhase::Ended,
        pseudo: Some(Pseudo::Before),
    });
    let control = EventControl::new();
    let mut sink = DiscardCommands;
    let mut cx1 = EventCx::<zgui::view::events::AnyEvent>::new(
        EventKind::AnimationEnd,
        h.mount,
        h.mount,
        Phase::Target,
        Modifiers::NONE,
        Timestamp::from_origin(Duration::from_millis(7)),
        &anim_end,
        &control,
        &mut sink,
    );
    let r1 = regs.iter().find(|r| r.kind == EventKind::AnimationEnd).unwrap();
    h.backend.handler(r1.id).unwrap()(&mut cx1);
    assert_eq!(control.default_action(), DefaultAction::Prevented);

    let trans_cancel = Payload::Transition(TransitionEvent {
        property: Ident::new("opacity"),
        elapsed: Duration::from_millis(250),
        phase: TransitionPhase::Cancelled,
        pseudo: None,
    });
    let anim_start = Payload::Animation(AnimationEvent {
        name: Ident::new("pulse"),
        elapsed: Duration::ZERO,
        phase: AnimationPhase::Started,
        pseudo: Some(Pseudo::After),
    });
    for (kind, payload) in [
        (EventKind::TransitionCancel, &trans_cancel),
        (EventKind::AnimationStart, &anim_start),
    ] {
        let ctrl = EventControl::new();
        let mut s = DiscardCommands;
        let mut cx = EventCx::<zgui::view::events::AnyEvent>::new(
            kind,
            h.mount,
            h.mount,
            Phase::Bubble,
            Modifiers::NONE,
            Timestamp::ORIGIN,
            payload,
            &ctrl,
            &mut s,
        );
        let r = regs.iter().find(|r| r.kind == kind).unwrap();
        h.backend.handler(r.id).unwrap()(&mut cx);
    }

    assert_eq!(h.eval(r#"
        const [a, t, s] = events;
        if (a.type !== 'animationend' || a.animationName !== 'fade' || a.elapsedTime !== 0.125 || a.pseudoElement !== '::before' || a.timestamp !== 7) throw Error('anim end');
        if (t.type !== 'transitioncancel' || t.propertyName !== 'opacity' || t.elapsedTime !== 0.25 || t.pseudoElement !== '') throw Error('trans cancel');
        if (s.type !== 'animationstart' || s.animationName !== 'pulse' || s.elapsedTime !== 0 || s.pseudoElement !== '::after') throw Error('anim start');
        let expired = false;
        try { a.preventDefault(); } catch (error) { expired = error instanceof ReferenceError && error.message.includes('expired-dispatch'); }
        if (!expired) throw Error('cancellation retained authority');
        'verified'
    "#), "verified");
}

#[derive(Default)]
struct Recorder {
    events: Rc<RefCell<Vec<(NodeId, EventKind)>>>,
}
impl EventSink for Recorder {
    fn capture_pointer(&mut self, _node: NodeId) {}
    fn release_pointer(&mut self, _node: NodeId) {}
    fn request_focus(&mut self, _node: NodeId) {}
    fn synthesize(&mut self, node: NodeId, event: EventKind) {
        self.events.borrow_mut().push((node, event));
    }
}

#[test]
fn activate_now_requests_one_click_after_expiry_and_rejects_other_kinds() {
    let h = EventHarness::synchronous(8, 100);
    h.script(r#"
        globalThis.events = [];
        __zgui.setProperty(root, 'onPointerDown', e => { events.push(e); e.activateNow(); e.activateNow(); });
        __zgui.setProperty(root, 'onPointerUp', e => {
            events.push(e);
            let rejected = false;
            try { e.activateNow(); } catch (error) { rejected = error instanceof TypeError; }
            if (!rejected) throw Error('activateNow accepted pointerup');
        });
        'armed'
    "#);
    let regs = registrations(&h.backend, h.mount);
    let payload = Payload::Pointer(PointerEvent::mouse(Point::new(CssPx(1.0), CssPx(2.0))));
    let control = EventControl::new();
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut sink = Recorder { events: log.clone() };
    let mut down = EventCx::new(
        EventKind::PointerDown,
        h.mount,
        h.mount,
        Phase::Target,
        Modifiers::NONE,
        Timestamp::ORIGIN,
        &payload,
        &control,
        &mut sink,
    );
    h.backend.handler(regs.iter().find(|r| r.kind == EventKind::PointerDown).unwrap().id).unwrap()(&mut down);
    assert!(control.activation_claimed());
    assert_eq!(*log.borrow(), vec![(h.mount, EventKind::Click)]);

    let up_control = EventControl::new();
    let up_log = Rc::new(RefCell::new(Vec::new()));
    let mut up_sink = Recorder { events: up_log.clone() };
    let mut up = EventCx::new(
        EventKind::PointerUp,
        h.mount,
        h.mount,
        Phase::Target,
        Modifiers::NONE,
        Timestamp::ORIGIN,
        &payload,
        &up_control,
        &mut up_sink,
    );
    h.backend.handler(regs.iter().find(|r| r.kind == EventKind::PointerUp).unwrap().id).unwrap()(&mut up);
    assert!(h.runner.last_error.borrow().is_none());
    assert!(up_log.borrow().is_empty());
    assert!(!up_control.activation_claimed());

    assert_eq!(h.eval(r#"
        let expired = false;
        try { events[0].activateNow(); }
        catch (error) { expired = error instanceof ReferenceError && error.message.includes('expired-dispatch'); }
        if (!expired) throw Error('activateNow retained authority');
        'verified'
    "#), "verified");
}

#[derive(Default)]
struct PointerRecorder {
    log: Rc<RefCell<Vec<(NodeId, bool)>>>,
}
impl EventSink for PointerRecorder {
    fn capture_pointer(&mut self, node: NodeId) {
        self.log.borrow_mut().push((node, true));
    }
    fn release_pointer(&mut self, node: NodeId) {
        self.log.borrow_mut().push((node, false));
    }
    fn request_focus(&mut self, _node: NodeId) {}
    fn synthesize(&mut self, _node: NodeId, _event: EventKind) {}
}

#[test]
fn pointer_capture_requests_apply_once_to_the_current_node_after_expiry() {
    let h = EventHarness::synchronous(9, 100);
    h.script(r#"
        const child = __zgui.createElement('control');
        __zgui.insertNode(root, child, null);
        globalThis.seen = [];
        __zgui.setProperty(root, 'onPointerDown', e => { seen.push(e); e.capturePointer(); e.capturePointer(); });
        __zgui.setProperty(root, 'onPointerMove', e => {
            e.releasePointer();
            let rejected = false;
            try { e.capturePointer(); } catch (err) { rejected = err instanceof TypeError; }
            if (!rejected) throw Error('capturePointer accepted move');
        });
        __zgui.setProperty(root, 'onClick', e => {
            for (const m of ['capturePointer', 'releasePointer']) {
                let rejected = false;
                try { e[m](); } catch (err) { rejected = err instanceof TypeError; }
                if (!rejected) throw Error(m + ' accepted click');
            }
        });
        'armed'
    "#);
    let child = h.backend.children(h.mount)[0];
    let regs = registrations(&h.backend, h.mount);
    let payload = Payload::Pointer(PointerEvent::mouse(Point::new(CssPx(1.0), CssPx(2.0))));

    let control = EventControl::new();
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut sink = PointerRecorder { log: log.clone() };
    let mut down = EventCx::new(
        EventKind::PointerDown,
        child,
        h.mount,
        Phase::Bubble,
        Modifiers::NONE,
        Timestamp::ORIGIN,
        &payload,
        &control,
        &mut sink,
    );
    h.backend.handler(regs.iter().find(|r| r.kind == EventKind::PointerDown).unwrap().id).unwrap()(&mut down);
    assert_eq!(*log.borrow(), vec![(h.mount, true)], "capture applies to current node once");

    let mut mv = EventCx::new(
        EventKind::PointerMove,
        child,
        h.mount,
        Phase::Bubble,
        Modifiers::NONE,
        Timestamp::ORIGIN,
        &payload,
        &control,
        &mut sink,
    );
    h.backend.handler(regs.iter().find(|r| r.kind == EventKind::PointerMove).unwrap().id).unwrap()(&mut mv);
    assert_eq!(*log.borrow(), vec![(h.mount, true), (h.mount, false)]);

    let mut click = EventCx::new(
        EventKind::Click,
        child,
        h.mount,
        Phase::Bubble,
        Modifiers::NONE,
        Timestamp::ORIGIN,
        &payload,
        &control,
        &mut sink,
    );
    h.backend.handler(regs.iter().find(|r| r.kind == EventKind::Click).unwrap().id).unwrap()(&mut click);
    assert!(h.runner.last_error.borrow().is_none());
    assert_eq!(log.borrow().len(), 2);

    assert_eq!(h.eval(r#"
        for (const name of ['capturePointer', 'releasePointer']) {
            let expired = false;
            try { seen[0][name](); } catch (e) { expired = e instanceof ReferenceError && e.message.includes('expired-dispatch'); }
            if (!expired) throw Error(name + ' retained authority');
        }
        'verified'
    "#), "verified");
}

#[test]
fn queued_and_reentrant_events_drop_native_cancellation_and_overflow_disposes_bridge() {
    // 1. Re-entrant dispatch inside an active Bridge turn queues without cancellation authority.
    let sync = EventHarness::synchronous(10, 100);
    sync.script(r#"
        globalThis.calls = [];
        __zgui.setProperty(root, 'onPointerDown', event => {
            calls.push(event.type);
            try { event.preventDefault(); calls.push('cancel-allowed'); }
            catch (e) { calls.push(e.message); }
            try { event.capturePointer(); calls.push('capture-allowed'); }
            catch (e) { calls.push(e.message); }
        });
        'armed'
    "#);
    let handler = sync.backend.handler(registrations(&sync.backend, sync.mount)[0].id).unwrap();
    let payload = Payload::Pointer(PointerEvent::mouse(Point::new(CssPx(3.0), CssPx(4.0))));

    let nested_control = EventControl::new();
    let nested_log = Rc::new(RefCell::new(Vec::new()));
    let mut nested_sink = PointerRecorder { log: nested_log.clone() };
    let mut nested = EventCx::new(
        EventKind::PointerDown,
        sync.mount,
        sync.mount,
        Phase::Target,
        Modifiers::NONE,
        Timestamp::ORIGIN,
        &payload,
        &nested_control,
        &mut nested_sink,
    );
    {
        let mut engine = sync.runner.engine.borrow_mut();
        sync.bridge
            .run_turn(engine.turn(), |_, _| -> Result<(), ()> {
                handler(&mut nested);
                Ok(())
            })
            .unwrap();
    }
    assert_eq!(nested_control.default_action(), DefaultAction::Allowed);
    assert!(nested_log.borrow().is_empty());
    assert_eq!(sync.bridge.has_queued_notifications(), Ok(true));
    assert_eq!(sync.flush_all(), 1);
    assert_eq!(sync.bridge.has_queued_notifications(), Ok(false));
    let log = sync.eval("calls.join('|')");
    assert!(log.contains("expired-dispatch"), "{log}");
    assert!(!log.contains("cancel-allowed"), "{log}");

    // 2. HostPort::Busy falls back to the internal queue and revokes synchronous authority.
    sync.runner.force_busy.set(true);
    let busy_control = EventControl::new();
    let mut discard = DiscardCommands;
    let mut busy = EventCx::new(
        EventKind::PointerDown,
        sync.mount,
        sync.mount,
        Phase::Target,
        Modifiers::NONE,
        Timestamp::ORIGIN,
        &payload,
        &busy_control,
        &mut discard,
    );
    handler(&mut busy);
    assert_eq!(busy_control.default_action(), DefaultAction::Allowed);
    assert_eq!(sync.bridge.has_queued_notifications(), Ok(true));
    sync.runner.force_busy.set(false);
    assert_eq!(sync.flush_all(), 1);

    // 3. Overflowing MAX_QUEUED_NOTIFICATIONS (256) disposes the bridge and yields Err(QueueOverflow).
    let q = EventHarness::queued(11, 100);
    q.script(r#"
        __zgui.setProperty(root, 'onPointerDown', () => {});
        'armed'
    "#);
    let q_handler = q.backend.handler(registrations(&q.backend, q.mount)[0].id).unwrap();
    for _ in 0..MAX_QUEUED_NOTIFICATIONS {
        let ctrl = EventControl::new();
        let mut s = DiscardCommands;
        let mut ev = EventCx::new(
            EventKind::PointerDown,
            q.mount,
            q.mount,
            Phase::Target,
            Modifiers::NONE,
            Timestamp::ORIGIN,
            &payload,
            &ctrl,
            &mut s,
        );
        q_handler(&mut ev);
    }
    assert_eq!(q.bridge.has_queued_notifications(), Ok(true));
    assert!(!q.bridge.is_disposed());

    // 257th notification overflows the queue and retires the Mount region.
    let ctrl = EventControl::new();
    let mut s = DiscardCommands;
    let mut overflow_ev = EventCx::new(
        EventKind::PointerDown,
        q.mount,
        q.mount,
        Phase::Target,
        Modifiers::NONE,
        Timestamp::ORIGIN,
        &payload,
        &ctrl,
        &mut s,
    );
    q_handler(&mut overflow_ev);
    assert!(q.bridge.is_disposed());
    assert_eq!(q.backend.handler_count(), 0);
    assert_eq!(q.bridge.has_queued_notifications(), Err(QueueOverflow));
}

#[test]
fn keyboard_character_text_is_explicit_and_survives_dispatch() {
    let h = EventHarness::queued(12, 100);
    h.script(r#"
        globalThis.characters = [];
        __zgui.setProperty(root, 'onKeyDown', event => characters.push(event));
        'armed'
    "#);
    let handler = h.backend.handler(registrations(&h.backend, h.mount)[0].id).unwrap();
    let keys = [
        Key::character("é"),
        Key::character("e\u{301}"),
        Key::character("𐐷"),
        Key::character("Enter"),
        Key::character(" "),
        Key::Named(NamedKey::Enter),
        Key::Named(NamedKey::Space),
        Key::Dead(Some('´')),
        Key::Other("x".into()),
        Key::Unidentified,
    ];
    for key in keys {
        let mut key_event = KeyEvent::character("unused");
        key_event.key_without_modifiers = key.clone();
        key_event.key = key;
        let payload = Payload::Key(key_event);
        let control = EventControl::new();
        let mut sink = DiscardCommands;
        let mut event = EventCx::new(
            EventKind::KeyDown,
            h.mount,
            h.mount,
            Phase::Bubble,
            Modifiers::NONE,
            Timestamp::ORIGIN,
            &payload,
            &control,
            &mut sink,
        );
        handler(&mut event);
    }
    assert_eq!(h.flush_all(), 10);
    assert_eq!(h.eval(r#"
        const expected = ['é', 'e\u0301', '𐐷', 'Enter', ' ', null, null, null, null, null];
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
        'verified'
    "#), "verified");
}

#[test]
fn synchronous_faulted_outcome_disposes_bridge_without_queueing() {
    let h = EventHarness::synchronous(11, 32);
    h.script("(__zgui.setProperty(root, 'onKeyDown', () => {}), 'armed')");
    let listener = h.backend.handler(registrations(&h.backend, h.mount)[0].id).unwrap();
    h.runner.force_fault.set(true);
    let payload = Payload::Key(KeyEvent::character("a"));
    let control = EventControl::new();
    let mut sink = DiscardCommands;
    let mut event = EventCx::new(
        EventKind::KeyDown,
        h.mount,
        h.mount,
        Phase::Target,
        Modifiers::NONE,
        Timestamp::ORIGIN,
        &payload,
        &control,
        &mut sink,
    );
    listener(&mut event);
    assert!(h.bridge.is_disposed());
    assert!(!h.bridge.has_queued_notifications().unwrap());
    assert_eq!(h.bridge.live_count(), 0);
}
