use std::{cell::RefCell, rc::Rc};

use js_runtime::{JsRuntime, RuntimeLimits};
use ui_bridge::{UiBridge, registry::NodeRegistry};
use zgui::{
    geom::{CssPx, Point},
    view::{DiscardCommands, DomHandle, ElementName, EventControl, EventCx, HostHandle, NodeId,
        stub::StubHost},
    vocab::{DefaultAction, EventKind, KeyCode, KeyEvent, Modifiers, NamedKey, Payload, Phase,
        PointerEvent, Propagation, Timestamp},
};
use zgui_dom::{Document, side::listeners::Listener};
use zgui_view_dom::DocumentDom;

struct Native {
    bridge: Rc<UiBridge>,
    engine: Rc<RefCell<JsRuntime>>,
    backend: Rc<DocumentDom>,
    dom: DomHandle,
    mount: NodeId,
    window: NodeId,
}

impl Native {
    fn on(backend: Rc<DocumentDom>, dom: DomHandle, mount: NodeId, session: u64, limit: usize) -> Self {
        let host = Rc::new(StubHost::default());
        let bridge = UiBridge::new(NodeRegistry::new(dom.clone(), mount, session, 1, limit), HostHandle::from_rc(host));
        let engine = Rc::new(RefCell::new(JsRuntime::new(RuntimeLimits::default()).unwrap()));
        engine.borrow_mut().with_context(|ctx| {
            bridge.install(&ctx)?;
            ctx.globals().set("root", bridge.root(&ctx)?)?;
            ctx.eval::<(), _>("globalThis.calls = []; globalThis.options = (layer = 'modal', outside = true, escape = true, exclude = null, leaving = false) => ({ layer, dismissOnOutsidePress: outside, dismissOnEscape: escape, exclude, leaving });")
        }).unwrap();
        let weak = Rc::downgrade(&bridge);
        let engine_weak = Rc::downgrade(&engine);
        bridge.set_dispatcher(Rc::new(move |id, event| {
            let (Some(bridge), Some(engine)) = (weak.upgrade(), engine_weak.upgrade()) else { return; };
            bridge.run_turn(|| engine.borrow_mut().with_context(|ctx| bridge.invoke_event(&ctx, id, event)))
                .expect("dismissal callback succeeds");
        }));
        let window = backend.root_node();
        Self { bridge, engine, backend, dom, mount, window }
    }

    fn new(session: u64, limit: usize) -> Self {
        let backend = Rc::new(DocumentDom::new(Rc::new(RefCell::new(Document::new()))));
        let dom = DomHandle::from_rc(backend.clone());
        let mount = dom.create_element(ElementName::new("box"));
        dom.insert(backend.root_node(), mount, None);
        Self::on(backend, dom, mount, session, limit)
    }

    fn script(&self, script: &str) {
        self.bridge.run_turn(|| self.engine.borrow_mut().evaluate_script(script)).unwrap();
    }

    fn calls(&self) -> String {
        self.engine.borrow_mut().with_context(|ctx| ctx.eval("calls.join(',')")).unwrap()
    }
}

fn registrations(backend: &DocumentDom, node: NodeId) -> Vec<Listener> {
    let index = backend.index_of(node);
    let document = backend.document();
    let store = document.store();
    store.columns().listeners.get(store.key_of(index)).map(|listeners| listeners.iter().copied().collect()).unwrap_or_default()
}

fn emit(backend: &DocumentDom, root: NodeId, kind: EventKind, payload: &Payload, target: NodeId) -> (DefaultAction, Propagation) {
    let control = EventControl::new();
    let mut sink = DiscardCommands;
    for listener in registrations(backend, root) {
        if listener.kind != kind { continue; }
        if control.propagation() == Propagation::StopImmediate { break; }
        let Some(handler) = backend.handler(listener.id) else { continue; };
        let mut event = EventCx::new(kind, target, root, Phase::Capture, Modifiers::NONE, Timestamp::ORIGIN, payload, &control, &mut sink);
        handler(&mut event);
    }
    (control.default_action(), control.propagation())
}

fn press(native: &Native, target: NodeId) -> (DefaultAction, Propagation) {
    let payload = Payload::Pointer(PointerEvent::mouse(Point::new(CssPx(1.0), CssPx(2.0))));
    emit(&native.backend, native.window, EventKind::PointerDown, &payload, target)
}

fn key(native: &Native, named: NamedKey, code: KeyCode) -> (DefaultAction, Propagation) {
    let payload = Payload::Key(KeyEvent::named(named, code.into()));
    emit(&native.backend, native.window, EventKind::KeyDown, &payload, native.mount)
}

fn escape(native: &Native) -> (DefaultAction, Propagation) {
    key(native, NamedKey::Escape, KeyCode::Escape)
}

#[test]
fn topmost_layer_owns_each_decision_and_escape_is_consumed() {
    let native = Native::new(90, 100);
    native.script(r#"
        const a = __zgui.createElement('box');
        const b = __zgui.createElement('box');
        const inner = __zgui.createElement('box');
        const trigger = __zgui.createElement('box');
        const triggerChild = __zgui.createElement('box');
        const outside = __zgui.createElement('box');
        __zgui.insertNode(root, a, null);
        __zgui.insertNode(root, b, null);
        __zgui.insertNode(b, inner, null);
        __zgui.insertNode(root, trigger, null);
        __zgui.insertNode(trigger, triggerChild, null);
        __zgui.insertNode(root, outside, null);
        globalThis.aLease = __zgui.registerDismissLayer(a, reason => calls.push('A:' + reason), options('modal', true, true, null, false));
        let bLease;
        bLease = __zgui.registerDismissLayer(b, reason => { calls.push('B:' + reason); if (reason === 'outside-press') bLease.dispose(); }, options('modal', true, true, trigger, false));
        aLease.update(options('modal', true, true, b, false));
        'mounted'
    "#);
    let mounted = native.backend.children(native.mount);
    let (a, b, trigger, outside) = (mounted[0], mounted[1], mounted[2], mounted[3]);
    let inner = native.backend.children(b)[0];
    let trigger_child = native.backend.children(trigger)[0];
    assert_eq!(native.backend.handler_count(), 2);

    press(&native, inner);
    assert_eq!(native.calls(), "");
    press(&native, trigger_child);
    assert_eq!(native.calls(), "");

    let (action, propagation) = escape(&native);
    assert_eq!((action, propagation), (DefaultAction::Prevented, Propagation::StopImmediate));
    assert_eq!(native.calls(), "B:escape");

    let (action, propagation) = press(&native, outside);
    assert_eq!((action, propagation), (DefaultAction::Allowed, Propagation::Continue));
    assert_eq!(native.calls(), "B:escape,B:outside-press");

    press(&native, outside);
    assert_eq!(native.calls(), "B:escape,B:outside-press,A:outside-press");
    press(&native, a);
    assert_eq!(native.calls(), "B:escape,B:outside-press,A:outside-press");
    escape(&native);
    assert_eq!(native.calls(), "B:escape,B:outside-press,A:outside-press,A:escape");
    native.bridge.dispose();
    assert_eq!(native.backend.handler_count(), 0);
}

#[test]
fn band_order_flags_and_leaving_gate_each_decision() {
    let native = Native::new(91, 100);
    native.script(r#"
        const t = __zgui.createElement('box');
        const m = __zgui.createElement('box');
        const mChild = __zgui.createElement('box');
        const out = __zgui.createElement('box');
        __zgui.insertNode(root, t, null);
        __zgui.insertNode(root, m, null);
        __zgui.insertNode(m, mChild, null);
        __zgui.insertNode(root, out, null);
        globalThis.tLease = __zgui.registerDismissLayer(t, reason => calls.push('T:' + reason), options('toast', true, true, null, false));
        globalThis.mLease = __zgui.registerDismissLayer(m, reason => calls.push('M:' + reason), options('modal', true, true, null, false));
        'mounted'
    "#);
    let mounted = native.backend.children(native.mount);
    let (_, _, m_child, out) = (mounted[0], mounted[1], native.backend.children(mounted[1])[0], mounted[2]);

    escape(&native);
    assert_eq!(native.calls(), "T:escape");

    native.script("tLease.update(options('toast', false, false, null, false)); 'updated'");
    escape(&native);
    assert_eq!(native.calls(), "T:escape");
    press(&native, out);
    assert_eq!(native.calls(), "T:escape");

    native.script("tLease.update(options('toast', false, true, null, true)); 'leaving'");
    escape(&native);
    assert_eq!(native.calls(), "T:escape,M:escape");
    press(&native, out);
    assert_eq!(native.calls(), "T:escape,M:escape");

    native.script("tLease.update(options('toast', true, true, null, true)); 'open'");
    press(&native, m_child);
    assert_eq!(native.calls(), "T:escape,M:escape,T:outside-press");

    key(&native, NamedKey::Enter, KeyCode::Enter);
    assert_eq!(native.calls(), "T:escape,M:escape,T:outside-press");
}

#[test]
fn sessions_arbitrate_on_a_shared_window_and_release_handlers() {
    let document = Rc::new(RefCell::new(Document::new()));
    let backend = Rc::new(DocumentDom::new(document));
    let dom = DomHandle::from_rc(backend.clone());
    let first_mount = dom.create_element(ElementName::new("box"));
    let second_mount = dom.create_element(ElementName::new("box"));
    dom.insert(backend.root_node(), first_mount, None);
    dom.insert(backend.root_node(), second_mount, None);
    let first = Native::on(backend.clone(), dom.clone(), first_mount, 92, 100);
    let second = Native::on(backend, dom, second_mount, 93, 100);

    first.script(r#"
        const one = __zgui.createElement('box');
        __zgui.insertNode(root, one, null);
        globalThis.lease = __zgui.registerDismissLayer(one, reason => calls.push('one:' + reason), options('modal', true, true, null, false));
        'mounted'
    "#);
    second.script(r#"
        const two = __zgui.createElement('box');
        __zgui.insertNode(root, two, null);
        globalThis.lease = __zgui.registerDismissLayer(two, reason => calls.push('two:' + reason), options('toast', true, true, null, false));
        'mounted'
    "#);
    assert_eq!(first.backend.handler_count(), 2);

    escape(&first);
    assert_eq!(second.calls(), "two:escape");
    assert_eq!(first.calls(), "");

    second.bridge.dispose();
    escape(&first);
    assert_eq!(first.calls(), "one:escape");
    assert_eq!(first.backend.handler_count(), 2);

    first.script("lease.dispose(); 'released'");
    assert_eq!(first.backend.handler_count(), 0);

    let other = Native::new(94, 100);
    other.script(r#"
        const own = __zgui.createElement('box');
        __zgui.insertNode(root, own, null);
        globalThis.lease = __zgui.registerDismissLayer(own, reason => calls.push('other:' + reason), options('toast', true, true, null, false));
        'mounted'
    "#);
    assert_eq!(other.backend.handler_count(), 2);
    assert_eq!(first.backend.handler_count(), 0);
    escape(&other);
    assert_eq!(other.calls(), "other:escape");
    escape(&first);
    assert_eq!(first.calls(), "one:escape");
    other.bridge.dispose();
    assert_eq!(other.backend.handler_count(), 0);
}

#[test]
fn owner_removal_releases_leases_and_stale_handlers_stay_silent() {
    let native = Native::new(95, 100);
    native.script(r#"
        const wrap = __zgui.createElement('box');
        const l = __zgui.createElement('box');
        const x = __zgui.createElement('box');
        const out = __zgui.createElement('box');
        __zgui.insertNode(root, wrap, null);
        __zgui.insertNode(wrap, l, null);
        __zgui.insertNode(root, x, null);
        __zgui.insertNode(root, out, null);
        globalThis.lease = __zgui.registerDismissLayer(l, reason => calls.push('L:' + reason), options('modal', true, true, x, false));
        'mounted'
    "#);
    let out = native.backend.children(native.mount)[2];

    native.script("__zgui.removeNode(root, wrap); __zgui.insertNode(root, wrap, null); 'moved'");
    press(&native, out);
    assert_eq!(native.calls(), "L:outside-press");

    native.script("__zgui.removeNode(root, wrap); 'gone'");
    assert_eq!(native.backend.handler_count(), 0);
    press(&native, out);
    assert_eq!(native.calls(), "L:outside-press");

    native.script(r#"
        const l2 = __zgui.createElement('box');
        const x2 = __zgui.createElement('box');
        const l3 = __zgui.createElement('box');
        __zgui.insertNode(root, l2, null);
        __zgui.insertNode(root, x2, null);
        globalThis.lease2 = __zgui.registerDismissLayer(l2, reason => calls.push('L2:' + reason), options('modal', true, true, x2, false));
        globalThis.lease3 = __zgui.registerDismissLayer(l3, reason => calls.push('L3:' + reason), options('toast', true, true, null, false));
        __zgui.removeNode(root, x2);
        'renewed'
    "#);
    assert_eq!(native.backend.handler_count(), 2);
    press(&native, out);
    assert_eq!(native.calls(), "L:outside-press,L2:outside-press");

    let stale: Vec<_> = registrations(&native.backend, native.window).iter()
        .filter(|listener| listener.kind == EventKind::PointerDown)
        .filter_map(|listener| native.backend.handler(listener.id))
        .collect();
    assert_eq!(stale.len(), 1);
    native.script("lease2.dispose(); lease3.dispose(); 'released'");
    assert_eq!(native.backend.handler_count(), 0);
    native.script(r#"
        const l4 = __zgui.createElement('box');
        __zgui.insertNode(root, l4, null);
        globalThis.lease4 = __zgui.registerDismissLayer(l4, reason => calls.push('L4:' + reason), options('toast', true, true, null, false));
        'reopened'
    "#);
    assert_eq!(native.backend.handler_count(), 2);

    let control = EventControl::new();
    let mut sink = DiscardCommands;
    let payload = Payload::Pointer(PointerEvent::mouse(Point::new(CssPx(1.0), CssPx(2.0))));
    let mut event = EventCx::new(EventKind::PointerDown, out, native.window, Phase::Capture,
        Modifiers::NONE, Timestamp::ORIGIN, &payload, &control, &mut sink);
    for handler in &stale { handler(&mut event); }
    assert_eq!(native.calls(), "L:outside-press,L2:outside-press");

    press(&native, out);
    assert_eq!(native.calls(), "L:outside-press,L2:outside-press,L4:outside-press");
}

#[test]
fn node_retirement_preserves_other_sessions_trigger_exclusions() {
    let document = Rc::new(RefCell::new(Document::new()));
    let backend = Rc::new(DocumentDom::new(document));
    let dom = DomHandle::from_rc(backend.clone());
    let first_mount = dom.create_element(ElementName::new("box"));
    let second_mount = dom.create_element(ElementName::new("box"));
    dom.insert(backend.root_node(), first_mount, None);
    dom.insert(backend.root_node(), second_mount, None);
    let first = Native::on(backend.clone(), dom.clone(), first_mount, 101, 100);
    let second = Native::on(backend, dom, second_mount, 102, 100);

    first.script(r#"
        const trigger = __zgui.createElement('box');
        const surface = __zgui.createElement('box');
        __zgui.insertNode(root, trigger, null);
        __zgui.insertNode(root, surface, null);
        globalThis.trigger = trigger;
        globalThis.lease = __zgui.registerDismissLayer(surface, reason => calls.push('first:' + reason), options('modal', true, true, trigger, false));
        'mounted'
    "#);
    second.script(r#"
        const trigger = __zgui.createElement('box');
        const surface = __zgui.createElement('box');
        __zgui.insertNode(root, trigger, null);
        __zgui.insertNode(root, surface, null);
        globalThis.trigger = trigger;
        globalThis.lease = __zgui.registerDismissLayer(surface, reason => calls.push('second:' + reason), options('toast', true, true, trigger, false));
        'mounted'
    "#);
    assert_eq!(first.backend.handler_count(), 2);

    let second_trigger = second.backend.children(second.mount)[0];
    press(&second, second_trigger);
    assert_eq!(second.calls(), "");
    assert_eq!(first.calls(), "");

    first.script("__zgui.removeNode(root, trigger); 'retired'");
    press(&second, second_trigger);
    assert_eq!(second.calls(), "");
    assert_eq!(first.calls(), "");

    press(&second, second.window);
    assert_eq!(second.calls(), "second:outside-press");
    assert_eq!(first.calls(), "");
    assert_eq!(first.backend.handler_count(), 2);
}

#[test]
fn limits_validation_and_lifecycle_reject_without_partial_registration() {
    let bounded = Native::new(96, 2);
    bounded.script(r#"
        globalThis.first = __zgui.registerDismissLayer(root, () => calls.push('one'), options('modal', true, true, null, false));
        globalThis.second = __zgui.registerDismissLayer(root, () => calls.push('two'), options('modal', true, true, null, false));
        { let rejected = false;
          try { __zgui.registerDismissLayer(root, () => {}, options('modal', true, true, null, false)); }
          catch (error) { rejected = error instanceof RangeError; }
          if (!rejected) throw Error('third lease accepted'); }
        first.dispose();
        globalThis.third = __zgui.registerDismissLayer(root, () => calls.push('three'), options('modal', true, true, null, false));
        'bounded'
    "#);
    assert_eq!(bounded.bridge.callback_count(), 2);

    let empty = Native::new(97, 0);
    empty.script(r#"
        { let rejected = false;
          try { __zgui.registerDismissLayer(root, () => {}, options('modal', true, true, null, false)); }
          catch (error) { rejected = error instanceof RangeError; }
          if (!rejected) throw Error('zero limit accepted a lease'); }
        'empty'
    "#);

    let native = Native::new(98, 100);
    native.script(r#"
        const a = __zgui.createElement('box');
        const b = __zgui.createElement('box');
        __zgui.insertNode(root, a, null);
        __zgui.insertNode(root, b, null);
        globalThis.a = a;
        globalThis.b = b;
        globalThis.la = __zgui.registerDismissLayer(a, reason => calls.push('A:' + reason), options('modal', true, true, null, false));
        globalThis.lb = __zgui.registerDismissLayer(b, reason => calls.push('B:' + reason), options('modal', true, true, null, false));
        'mounted'
    "#);

    native.script(r#"
        la.dispose();
        globalThis.lc = __zgui.registerDismissLayer(a, reason => calls.push('C:' + reason), options('modal', true, true, null, false));
        { let rejected = false;
          try { __zgui.registerDismissLayer(b, () => {}, options('bogus', true, true, null, false)); }
          catch (error) { rejected = error instanceof TypeError; }
          if (!rejected) throw Error('unsupported layer accepted'); }
        { let rejected = false;
          try { __zgui.registerDismissLayer({}, () => {}, options('modal', true, true, null, false)); }
          catch (error) { rejected = error instanceof TypeError; }
          if (!rejected) throw Error('forged node accepted'); }
        'validated'
    "#);
    assert_eq!(native.bridge.callback_count(), 2);

    native.script(r#"
        const doomed = __zgui.createElement('box');
        __zgui.insertNode(root, doomed, null);
        globalThis.doomed = doomed;
        'seeded'
    "#);
    native.script("__zgui.removeNode(root, doomed); 'stale'");
    native.script(r#"
        { let rejected = false;
          try { __zgui.registerDismissLayer(doomed, () => {}, options('modal', true, true, null, false)); }
          catch (error) { rejected = error instanceof TypeError; }
          if (!rejected) throw Error('stale owner accepted'); }
        { let rejected = false;
          try { __zgui.registerDismissLayer(a, () => {}, options('modal', true, true, doomed, false)); }
          catch (error) { rejected = error instanceof TypeError; }
          if (!rejected) throw Error('stale exclude accepted'); }
        { let rejected = false;
          try { lb.update(options('bogus', true, true, null, false)); }
          catch (error) { rejected = error instanceof TypeError; }
          if (!rejected) throw Error('invalid update layer accepted'); }
        { let rejected = false;
          try { lb.update(options('modal', true, true, doomed, false)); }
          catch (error) { rejected = error instanceof TypeError; }
          if (!rejected) throw Error('stale update exclude accepted'); }
        'rejected'
    "#);
    assert_eq!(native.backend.handler_count(), 2);
    escape(&native);
    assert_eq!(native.calls(), "C:escape");

    native.script(r#"
        lc.dispose();
        { let rejected = false;
          try { lc.update(options()); }
          catch (error) { rejected = error instanceof ReferenceError; }
          if (!rejected) throw Error('update after dispose accepted'); }
        lc.dispose();
        'released'
    "#);
    escape(&native);
    assert_eq!(native.calls(), "C:escape,B:escape");

    native.bridge.begin_stop();
    native.script(r#"
        { let rejected = false;
          try { __zgui.registerDismissLayer(a, () => {}, options('modal', true, true, null, false)); }
          catch (error) { rejected = error instanceof ReferenceError; }
          if (!rejected) throw Error('stopping session accepted a lease'); }
        { let rejected = false;
          try { lb.update(options('modal', true, true, null, false)); }
          catch (error) { rejected = error instanceof ReferenceError; }
          if (!rejected) throw Error('stopping session accepted an update'); }
        'stopping'
    "#);
    escape(&native);
    assert_eq!(native.calls(), "C:escape,B:escape,B:escape");

    native.bridge.dispose();
    native.engine.borrow_mut().evaluate_script("lb.dispose(); 'gone'").unwrap();
    assert_eq!(native.backend.handler_count(), 0);
    assert_eq!(native.bridge.callback_count(), 0);

    let faulted = Native::new(99, 100);
    faulted.script(r#"
        const l = __zgui.createElement('box');
        __zgui.insertNode(root, l, null);
        globalThis.f = __zgui.registerDismissLayer(l, reason => calls.push('F:' + reason), options('modal', true, true, null, false));
        'mounted'
    "#);
    assert_eq!(faulted.backend.handler_count(), 2);
    let fault = faulted.bridge.run_turn(|| faulted.engine.borrow_mut().evaluate_script("throw Error('abort')"));
    assert!(fault.is_err());
    assert_eq!(faulted.backend.handler_count(), 0);
    assert_eq!(faulted.bridge.callback_count(), 0);

    let reasoned = Native::new(100, 100);
    reasoned.script(r#"
        const l = __zgui.createElement('box');
        __zgui.insertNode(root, l, null);
        __zgui.registerDismissLayer(l, (...args) => calls.push(args.length + ':' + typeof args[0] + ':' + args[0]), options('modal', true, true, null, false));
        'mounted'
    "#);
    escape(&reasoned);
    assert_eq!(reasoned.calls(), "1:string:escape");
    press(&reasoned, reasoned.window);
    assert_eq!(reasoned.calls(), "1:string:escape,1:string:outside-press");
}

#[test]
fn foreign_fabricated_and_stale_wrappers_reject_before_registration() {
    let native = Native::new(103, 100);
    native.script(r#"
        const a = __zgui.createElement('box');
        __zgui.insertNode(root, a, null);
        globalThis.a = a;
        globalThis.lease = __zgui.registerDismissLayer(a, reason => calls.push('L:' + reason), options());
        'mounted'
    "#);
    let session_foreign = UiBridge::new(
        NodeRegistry::new(native.dom.clone(), native.mount, 104, 1, 100),
        HostHandle::new(StubHost::default()),
    );
    let generation_foreign = UiBridge::new(
        NodeRegistry::new(native.dom.clone(), native.mount, 103, 2, 100),
        HostHandle::new(StubHost::default()),
    );
    native.engine.borrow_mut().with_context(|ctx| {
        ctx.globals().set("foreignSession", session_foreign.root(&ctx)?)?;
        ctx.globals().set("foreignGeneration", generation_foreign.root(&ctx)?)
    }).unwrap();
    native.script(r#"
        const forged = Object.assign(Object.create(Object.getPrototypeOf(a)), a);
        for (const foreign of [foreignSession, foreignGeneration, forged, {}]) {
            let rejected = false;
            try { __zgui.registerDismissLayer(foreign, () => {}, options()); }
            catch (error) { rejected = error instanceof TypeError; }
            if (!rejected) throw Error('foreign or fabricated owner accepted');
        }
        for (const foreign of [foreignSession, foreignGeneration, forged, {}]) {
            let rejected = false;
            try { __zgui.registerDismissLayer(a, () => {}, options('modal', true, true, foreign)); }
            catch (error) { rejected = error instanceof TypeError; }
            if (!rejected) throw Error('foreign or fabricated exclude accepted');
        }
        for (const foreign of [foreignSession, foreignGeneration, forged]) {
            let rejected = false;
            try { lease.update(options('modal', true, true, foreign)); }
            catch (error) { rejected = error instanceof TypeError; }
            if (!rejected) throw Error('foreign or fabricated update exclude accepted');
        }
        'rejected'
    "#);
    assert_eq!(native.bridge.callback_count(), 1);
    assert_eq!(native.backend.handler_count(), 2);
    session_foreign.dispose();
    generation_foreign.dispose();
    escape(&native);
    assert_eq!(native.calls(), "L:escape");
}

#[test]
fn option_getters_run_before_native_borrows_and_failures_leave_no_state() {
    let native = Native::new(105, 100);
    native.script(r#"
        const a = __zgui.createElement('box');
        __zgui.insertNode(root, a, null);
        globalThis.a = a;
        globalThis.lease = __zgui.registerDismissLayer(a, reason => calls.push('L:' + reason), {
            get layer() { const child = __zgui.createElement('box'); __zgui.insertNode(a, child, null); return 'modal'; },
            dismissOnOutsidePress: true, dismissOnEscape: true, exclude: null, leaving: false,
        });
        'registered'
    "#);
    assert_eq!(native.backend.handler_count(), 2);
    escape(&native);
    assert_eq!(native.calls(), "L:escape");

    native.script(r#"
        { let rejected = false;
          try { lease.update({ get layer() { lease.dispose(); return 'modal'; },
                  dismissOnOutsidePress: true, dismissOnEscape: true, exclude: null, leaving: false }); }
          catch (error) { rejected = error instanceof ReferenceError; }
          if (!rejected) throw Error('update survived its own disposal'); }
        'self-disposed'
    "#);
    assert_eq!(native.bridge.callback_count(), 0);
    assert_eq!(native.backend.handler_count(), 0);

    native.script(r#"
        { let rejected = false;
          try { __zgui.registerDismissLayer(a, () => calls.push('never'), {
                  get layer() { throw Error('getter aborted'); },
                  dismissOnOutsidePress: true, dismissOnEscape: true, exclude: null, leaving: false }); }
          catch (error) { rejected = error instanceof Error; }
          if (!rejected) throw Error('throwing getter registered'); }
        'aborted'
    "#);
    assert_eq!(native.bridge.callback_count(), 0);
    assert_eq!(native.backend.handler_count(), 0);
}
