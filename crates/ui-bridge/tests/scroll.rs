use std::{cell::RefCell, rc::Rc};
use js_runtime::{JsRuntime, RuntimeLimits};
use ui_bridge::{HostPort, UiBridge, registry::NodeRegistry};
use zgui::view::{DomHandle, ElementName, HostHandle, NodeId, stub::StubHost};
use zgui_dom::Document;
use zgui_view_dom::DocumentDom;

struct Locks {
    bridge: Rc<UiBridge>,
    engine: JsRuntime,
    backend: Rc<DocumentDom>,
    dom: DomHandle,
    host: Rc<StubHost>,
    root: NodeId,
}

impl Locks {
    fn new(session: u64, limit: usize) -> Self {
        let backend = Rc::new(DocumentDom::new(Rc::new(RefCell::new(Document::new()))));
        let host = Rc::new(StubHost::default());
        Self::on(backend, host, session, limit)
    }

    fn on(backend: Rc<DocumentDom>, host: Rc<StubHost>, session: u64, limit: usize) -> Self {
        let dom = DomHandle::from_rc(backend.clone());
        let root = dom.create_element(ElementName::new("column"));
        dom.insert(backend.root_node(), root, None);
        let mut engine = JsRuntime::new(RuntimeLimits::default()).unwrap();
        let bridge = UiBridge::new(NodeRegistry::new(dom.clone(), root, session, 1, limit), HostHandle::from_rc(host.clone()), HostPort::queued());
        engine.with_context(|ctx| {
            bridge.install(&ctx)?;
            ctx.globals().set("root", bridge.root(&ctx)?)
        }).unwrap();
        Self { bridge, engine, backend, dom, host, root }
    }

    fn script(&mut self, script: &str) {
        self.bridge.run_turn(self.engine.turn(), |_, js| js.evaluate_script(script)).unwrap();
    }
}

const LEASE: &str = r#"
    const a = __zgui.createElement('box');
    __zgui.insertNode(root, a, null);
    globalThis.release = __zgui.lockScroll(a);
    'locked'
"#;

#[test]
fn leases_nest_release_idempotently_and_follow_node_removal() {
    let mut native = Locks::new(85, 100);
    native.script(r#"
        const a = __zgui.createElement('box');
        const b = __zgui.createElement('box');
        __zgui.insertNode(root, a, null);
        __zgui.insertNode(a, b, null);
        globalThis.outer = __zgui.lockScroll(a);
        globalThis.inner = __zgui.lockScroll(b);
        'mounted'
    "#);
    assert_eq!(native.bridge.scroll_lock_count(), 2);
    assert!(native.host.scrolling_frozen());
    native.script("inner(); inner(); 'released'");
    assert_eq!(native.bridge.scroll_lock_count(), 1);
    assert!(native.host.scrolling_frozen());
    native.script("__zgui.removeNode(root, a); __zgui.insertNode(root, a, null); 'reinserted'");
    assert_eq!(native.bridge.scroll_lock_count(), 1);
    assert!(native.host.scrolling_frozen());
    native.script("__zgui.removeNode(root, a); 'removed'");
    assert_eq!(native.bridge.scroll_lock_count(), 0);
    assert!(!native.host.scrolling_frozen());
    native.script("outer(); inner(); 'gone'");
    assert_eq!(native.bridge.scroll_lock_count(), 0);
    assert!(!native.host.scrolling_frozen());
    native.bridge.dispose();
}

#[test]
fn lease_limit_is_bounded_and_identifiers_reuse_freed_slots() {
    let mut native = Locks::new(86, 2);
    native.script(r#"
        globalThis.first = __zgui.lockScroll(root);
        globalThis.second = __zgui.lockScroll(root);
        { let rejected = false;
        try { __zgui.lockScroll(root); } catch (error) { rejected = error instanceof RangeError; }
        if (!rejected) throw Error('third lease exceeded the limit'); }
        'bounded'
    "#);
    assert_eq!(native.bridge.scroll_lock_count(), 2);
    assert!(native.host.scrolling_frozen());

    native.script("first(); globalThis.third = __zgui.lockScroll(root); first(); 'reused'");
    assert_eq!(native.bridge.scroll_lock_count(), 2);
    assert!(native.host.scrolling_frozen());

    native.bridge.dispose();
    assert_eq!(native.bridge.scroll_lock_count(), 0);
    assert!(!native.host.scrolling_frozen());
    native.engine.evaluate_script("second(); third(); 'post-dispose'").unwrap();
}

#[test]
fn stopping_blocks_new_leases_but_releases_existing() {
    let mut native = Locks::new(87, 100);
    let outside = native.engine.evaluate_script(r#"
        { let rejected = false;
        try { __zgui.lockScroll(root); } catch (error) { rejected = error instanceof TypeError; }
        if (!rejected) throw Error('outside-turn mutation accepted'); }
        'rejected'
    "#);
    assert_eq!(outside.unwrap(), "rejected");
    assert_eq!(native.bridge.scroll_lock_count(), 0);
    assert!(!native.host.scrolling_frozen());

    native.script("globalThis.keep = __zgui.lockScroll(root); 'leased'");
    assert_eq!(native.bridge.scroll_lock_count(), 1);
    assert!(native.host.scrolling_frozen());

    native.bridge.begin_stop();
    native.script(r#"
        { let rejected = false;
        try { __zgui.lockScroll(root); } catch (error) { rejected = error instanceof ReferenceError; }
        if (!rejected) throw Error('stopping session accepted a lease'); }
        'stopped'
    "#);
    assert_eq!(native.bridge.scroll_lock_count(), 1);
    assert!(native.host.scrolling_frozen());

    native.script("keep(); 'released'");
    assert_eq!(native.bridge.scroll_lock_count(), 0);
    assert!(!native.host.scrolling_frozen());
    native.bridge.dispose();
}

#[test]
fn a_faulted_turn_releases_its_locks_and_fixture_drop_releases_without_dispose() {
    let mut native = Locks::new(88, 100);
    let result = native.bridge.run_turn(native.engine.turn(), |_, js| js.evaluate_script(r#"
        __zgui.lockScroll(root);
        throw Error('abort');
    "#));
    assert!(result.is_err());
    assert_eq!(native.bridge.scroll_lock_count(), 0);
    assert!(!native.host.scrolling_frozen());
    native.bridge.dispose();

    let host = {
        let mut dropped = Locks::new(89, 100);
        let host = dropped.host.clone();
        dropped.script("__zgui.lockScroll(root); 'held'");
        assert!(host.scrolling_frozen());
        drop(dropped);
        host
    };
    assert!(!host.scrolling_frozen());
}

#[test]
fn leases_reject_fabricated_foreign_stale_and_disposed_nodes() {
    let mut native = Locks::new(90, 100);
    native.script(r#"
        const a = __zgui.createElement('box');
        __zgui.insertNode(root, a, null);
        'mounted'
    "#);
    native.script(r#"
        const forged = Object.assign(Object.create(Object.getPrototypeOf(a)), a);
        { let rejected = false;
        try { __zgui.lockScroll(forged); } catch (error) { rejected = error instanceof TypeError; }
        if (!rejected) throw Error('fabricated wrapper accepted'); }
        'forged'
    "#);
    let session_foreign = UiBridge::new(
        NodeRegistry::new(native.dom.clone(), native.root, 92, 1, 100),
        HostHandle::new(StubHost::default()),
        HostPort::queued(),
    );
    let generation_foreign = UiBridge::new(
        NodeRegistry::new(native.dom.clone(), native.root, 90, 2, 100),
        HostHandle::new(StubHost::default()),
        HostPort::queued(),
    );
    native.engine.with_context(|ctx| {
        ctx.globals().set("foreignSession", session_foreign.root(&ctx)?)?;
        ctx.globals().set("foreignGeneration", generation_foreign.root(&ctx)?)
    }).unwrap();
    native.script(r#"
        for (const foreign of [foreignSession, foreignGeneration]) {
            let rejected = false;
            try { __zgui.lockScroll(foreign); } catch (error) { rejected = error instanceof TypeError; }
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
        try { __zgui.lockScroll(a); } catch (error) { rejected = error instanceof TypeError; }
        if (!rejected) throw Error('stale node accepted'); }
        'stale'
    "#);
    assert_eq!(native.bridge.scroll_lock_count(), 0);
    assert!(!native.host.scrolling_frozen());
    native.bridge.dispose();
    native.engine.evaluate_script(r#"
        { let rejected = false;
        try { __zgui.lockScroll(root); } catch (error) { rejected = error instanceof ReferenceError; }
        if (!rejected) throw Error('disposed session accepted a lease'); }
        'disposed'
    "#).unwrap();
    drop(native);
}

#[test]
fn leases_share_a_window_across_sessions() {
    let backend = Rc::new(DocumentDom::new(Rc::new(RefCell::new(Document::new()))));
    let host = Rc::new(StubHost::default());
    let mut second = Locks::on(backend.clone(), host.clone(), 93, 100);
    second.script(LEASE);
    assert_eq!(second.bridge.scroll_lock_count(), 1);
    assert!(host.scrolling_frozen());

    let mut first = Locks::on(backend.clone(), host.clone(), 94, 100);
    first.script(LEASE);
    assert_eq!(first.bridge.scroll_lock_count(), 1);
    assert_eq!(second.bridge.scroll_lock_count(), 1);
    assert!(host.scrolling_frozen());

    first.script("__zgui.removeNode(root, a); 'removed'");
    assert_eq!(first.bridge.scroll_lock_count(), 0);
    assert_eq!(second.bridge.scroll_lock_count(), 1);
    assert!(host.scrolling_frozen(), "the peer lease keeps the shared window frozen");

    first.script(r#"
        const b = __zgui.createElement('box');
        __zgui.insertNode(root, b, null);
        globalThis.again = __zgui.lockScroll(b);
        'reacquired'
    "#);
    assert_eq!(first.bridge.scroll_lock_count(), 1);
    assert!(host.scrolling_frozen());
    first.script("again(); 'released'");
    assert_eq!(first.bridge.scroll_lock_count(), 0);
    assert!(host.scrolling_frozen(), "the peer lease still holds the window");

    first.script("globalThis.finalHold = __zgui.lockScroll(b); 'held before disposal'");
    assert_eq!(first.bridge.scroll_lock_count(), 1);
    first.bridge.dispose();
    assert_eq!(first.bridge.scroll_lock_count(), 0);
    assert_eq!(second.bridge.scroll_lock_count(), 1);
    assert!(host.scrolling_frozen(), "disposing an active owner must preserve the peer hold");
    second.bridge.dispose();
    assert_eq!(second.bridge.scroll_lock_count(), 0);
    assert!(!host.scrolling_frozen());

    let mut peer = Locks::on(backend.clone(), host.clone(), 95, 100);
    peer.script(LEASE);
    assert!(host.scrolling_frozen());
    let mut faulted = Locks::on(backend.clone(), host.clone(), 96, 100);
    faulted.script(LEASE);
    let result = faulted.bridge.run_turn(faulted.engine.turn(), |_, js| js.evaluate_script(r#"
        __zgui.lockScroll(root);
        throw Error('abort');
    "#));
    assert!(result.is_err());
    assert_eq!(faulted.bridge.scroll_lock_count(), 0);
    assert_eq!(peer.bridge.scroll_lock_count(), 1);
    assert!(host.scrolling_frozen(), "a faulted owner does not disturb its peer");
    peer.script("release(); 'released'");
    assert_eq!(peer.bridge.scroll_lock_count(), 0);
    assert!(!host.scrolling_frozen());
    peer.bridge.dispose();
    faulted.bridge.dispose();
}

#[test]
fn leases_are_isolated_between_independent_windows() {
    let mut first = Locks::new(97, 100);
    let mut second = Locks::new(98, 100);
    first.script(LEASE);
    assert!(first.host.scrolling_frozen());
    assert!(!second.host.scrolling_frozen());
    second.script(LEASE);
    assert!(first.host.scrolling_frozen());
    assert!(second.host.scrolling_frozen());
    first.script("release(); 'released'");
    assert!(!first.host.scrolling_frozen());
    assert!(second.host.scrolling_frozen());
    first.bridge.dispose();
    second.bridge.dispose();
    assert_eq!(first.bridge.scroll_lock_count(), 0);
    assert_eq!(second.bridge.scroll_lock_count(), 0);
}
