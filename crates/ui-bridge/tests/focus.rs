use std::{cell::RefCell, rc::Rc};
use js_runtime::{JsRuntime, RuntimeLimits};
use ui_bridge::{UiBridge, registry::NodeRegistry};
use zgui::view::{DomHandle, ElementName, FocusTrapOptions, HostHandle, NodeId, stub::StubHost};
use zgui_dom::Document;
use zgui_view_dom::DocumentDom;

struct Focus {
    bridge: Rc<UiBridge>,
    engine: JsRuntime,
    backend: Rc<DocumentDom>,
    dom: DomHandle,
    host: Rc<StubHost>,
    root: NodeId,
}

impl Focus {
    fn new(session: u64, limit: usize) -> Self {
        let backend = Rc::new(DocumentDom::new(Rc::new(RefCell::new(Document::new()))));
        let dom = DomHandle::from_rc(backend.clone());
        let root = dom.create_element(ElementName::new("column"));
        dom.insert(backend.root_node(), root, None);
        let host = Rc::new(StubHost::default());
        let mut engine = JsRuntime::new(RuntimeLimits::default()).unwrap();
        let bridge = UiBridge::new(NodeRegistry::new(dom.clone(), root, session, 1, limit), HostHandle::from_rc(host.clone()));
        engine.with_context(|ctx| {
            bridge.install(&ctx)?;
            ctx.globals().set("root", bridge.root(&ctx)?)
        }).unwrap();
        Self { bridge, engine, backend, dom, host, root }
    }

    fn script(&mut self, script: &str) {
        self.bridge.run_turn(|| self.engine.evaluate_script(script)).unwrap();
    }
}

fn child(backend: &Rc<DocumentDom>, parent: NodeId, index: usize) -> NodeId {
    backend.children(parent)[index]
}

#[test]
fn traps_track_options_idempotence_and_node_removal() {
    let mut native = Focus::new(75, 100);
    native.script(r#"
        const a = __zgui.createElement('box');
        const b = __zgui.createElement('box');
        __zgui.insertNode(root, a, null);
        __zgui.insertNode(a, b, null);
        globalThis.outer = __zgui.trapFocus(a, true, true, true);
        globalThis.inner = __zgui.trapFocus(b, false, false, false);
        'mounted'
    "#);
    let a_node = child(&native.backend, native.root, 0);
    let b_node = child(&native.backend, a_node, 0);
    assert_eq!(native.host.live_focus_traps(), 2);
    assert_eq!(native.bridge.focus_trap_count(), 2);
    let (_, top_root, options) = native.host.topmost_focus_trap().expect("two traps stacked");
    assert_eq!(top_root, b_node);
    assert!(!options.wrap && !options.auto_focus && !options.restore);

    native.script("inner(); inner(); 'released'");
    assert_eq!(native.host.live_focus_traps(), 1);
    assert_eq!(native.bridge.focus_trap_count(), 1);
    let (_, top_root, options) = native.host.topmost_focus_trap().expect("outer remains");
    assert_eq!(top_root, a_node);
    assert_eq!(options, FocusTrapOptions::MODAL);

    native.script("__zgui.removeNode(root, a); __zgui.insertNode(root, a, null); 'reinserted'");
    assert_eq!(native.host.live_focus_traps(), 1);
    assert_eq!(native.bridge.focus_trap_count(), 1);

    native.script("__zgui.removeNode(root, a); 'removed'");
    assert_eq!(native.host.live_focus_traps(), 0);
    assert_eq!(native.bridge.focus_trap_count(), 0);

    native.script("outer(); inner(); 'gone'");
    assert_eq!(native.host.live_focus_traps(), 0);
    assert_eq!(native.bridge.focus_trap_count(), 0);
    native.bridge.dispose();
}

#[test]
fn trap_limit_is_bounded_and_identifiers_reuse_freed_slots() {
    let mut native = Focus::new(76, 2);
    native.script(r#"
        globalThis.first = __zgui.trapFocus(root, true, true, true);
        globalThis.second = __zgui.trapFocus(root, true, true, true);
        { let rejected = false;
        try { __zgui.trapFocus(root, true, true, true); } catch (error) { rejected = error instanceof RangeError; }
        if (!rejected) throw Error('third lease exceeded the limit'); }
        'bounded'
    "#);
    assert_eq!(native.host.live_focus_traps(), 2);

    native.script("first(); globalThis.third = __zgui.trapFocus(root, true, true, true); first(); 'reused'");
    assert_eq!(native.host.live_focus_traps(), 2);
    assert_eq!(native.bridge.focus_trap_count(), 2);

    native.bridge.dispose();
    assert_eq!(native.host.live_focus_traps(), 0);
    assert_eq!(native.bridge.focus_trap_count(), 0);
    native.engine.evaluate_script("second(); third(); 'post-dispose'").unwrap();
}

#[test]
fn stopping_blocks_registration_but_releases_existing_leases() {
    let mut native = Focus::new(77, 100);
    native.script("globalThis.keep = __zgui.trapFocus(root, true, true, true); 'leased'");
    assert_eq!(native.host.live_focus_traps(), 1);

    native.bridge.begin_stop();
    native.script(r#"
        { let rejected = false;
        try { __zgui.trapFocus(root, true, true, true); } catch (error) { rejected = error instanceof ReferenceError; }
        if (!rejected) throw Error('stopping session accepted a registration'); }
        'stopped'
    "#);
    assert_eq!(native.host.live_focus_traps(), 1);
    assert_eq!(native.bridge.focus_trap_count(), 1);

    native.script("keep(); 'released'");
    assert_eq!(native.host.live_focus_traps(), 0);
    assert_eq!(native.bridge.focus_trap_count(), 0);
    native.bridge.dispose();
}

#[test]
fn a_faulted_turn_releases_the_traps_it_installed() {
    let mut native = Focus::new(78, 100);
    let result = native.bridge.run_turn(|| native.engine.evaluate_script(r#"
        __zgui.trapFocus(root, true, true, true);
        throw Error('abort');
    "#));
    assert!(result.is_err());
    assert_eq!(native.host.live_focus_traps(), 0);
    assert_eq!(native.bridge.focus_trap_count(), 0);
    native.bridge.dispose();
}

#[test]
fn traps_reject_fabricated_foreign_stale_and_disposed_nodes() {
    let mut native = Focus::new(79, 100);
    native.script(r#"
        const a = __zgui.createElement('box');
        __zgui.insertNode(root, a, null);
        'mounted'
    "#);
    native.script(r#"
        const forged = Object.assign(Object.create(Object.getPrototypeOf(a)), a);
        { let rejected = false;
        try { __zgui.trapFocus(forged, true, true, true); } catch (error) { rejected = error instanceof TypeError; }
        if (!rejected) throw Error('fabricated wrapper accepted'); }
        'forged'
    "#);
    let session_foreign = UiBridge::new(
        NodeRegistry::new(native.dom.clone(), native.root, 91, 1, 100),
        HostHandle::new(StubHost::default()),
    );
    let generation_foreign = UiBridge::new(
        NodeRegistry::new(native.dom.clone(), native.root, 79, 2, 100),
        HostHandle::new(StubHost::default()),
    );
    native.engine.with_context(|ctx| {
        ctx.globals().set("foreignSession", session_foreign.root(&ctx)?)?;
        ctx.globals().set("foreignGeneration", generation_foreign.root(&ctx)?)
    }).unwrap();
    native.script(r#"
        for (const foreign of [foreignSession, foreignGeneration]) {
            let rejected = false;
            try { __zgui.trapFocus(foreign, true, true, true); } catch (error) { rejected = error instanceof TypeError; }
            if (!rejected) throw Error('foreign node accepted');
        }
        'foreign'
    "#);
    session_foreign.dispose();
    generation_foreign.dispose();
    native.script("__zgui.removeNode(root, a); 'removed'");
    native.backend.end_frame();
    native.script(r#"
        { let rejected = false;
        try { __zgui.trapFocus(a, true, true, true); } catch (error) { rejected = error instanceof TypeError; }
        if (!rejected) throw Error('stale node accepted'); }
        'stale'
    "#);
    assert_eq!(native.host.live_focus_traps(), 0);
    assert_eq!(native.bridge.focus_trap_count(), 0);
    native.bridge.dispose();
    native.engine.evaluate_script(r#"
        { let rejected = false;
        try { __zgui.trapFocus(root, true, true, true); } catch (error) { rejected = error instanceof ReferenceError; }
        if (!rejected) throw Error('disposed session accepted a registration'); }
        'disposed'
    "#).unwrap();
    drop(native);
}
