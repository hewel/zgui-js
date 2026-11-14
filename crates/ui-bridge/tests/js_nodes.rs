use std::{cell::RefCell, rc::Rc};
use js_runtime::{JsRuntime, RuntimeLimits};
use ui_bridge::{UiBridge, registry::NodeRegistry};
use zgui::view::{DomHandle, ElementName, HostHandle, stub::StubHost};
use zgui_dom::Document;
use zgui_view_dom::DocumentDom;

#[test]
fn wrappers_preserve_identity_and_reject_fabrication_and_cross_frame_reuse() {
    let document = Rc::new(RefCell::new(Document::new()));
    let backend = Rc::new(DocumentDom::new(document));
    let dom = DomHandle::from_rc(backend.clone());
    let root = dom.create_element(ElementName::new("column"));
    dom.insert(backend.root_node(), root, None);
    // Runtime declared first: bridge roots are dropped before engine on unwind too.
    let mut engine = JsRuntime::new(RuntimeLimits::default()).unwrap();
    let bridge = UiBridge::new(NodeRegistry::new(dom.clone(), root, 7, 1, 100), HostHandle::new(StubHost::default()));
    engine.with_context(|ctx| { bridge.install(&ctx)?; ctx.globals().set("root", bridge.root(&ctx)?) }).unwrap();
    bridge.run_turn(|| engine.evaluate_script(r#"
        const a = __zgui.createTextNode('A');
        const b = __zgui.createTextNode('B');
        const anchor = __zgui.createTextNode('');
        __zgui.insertNode(root, a, null);
        __zgui.insertNode(root, anchor, null);
        __zgui.insertNode(root, b, anchor);
        if (__zgui.getFirstChild(root) !== a || __zgui.getNextSibling(a) !== b || __zgui.getNextSibling(b) !== anchor || __zgui.getParentNode(a) !== root) throw Error('identity lost');
        if (__zgui.getParentNode(root) != null) throw Error('root escaped');
        let rejected = false;
        try { __zgui.replaceText(Object.assign(Object.create(Object.getPrototypeOf(a)), a), 'forged'); } catch { rejected = true; }
        if (!rejected) throw Error('fabricated object accepted');
        __zgui.removeNode(root, b);
        __zgui.insertNode(root, b, a);
        'mounted'
    "#)).unwrap();
    assert_eq!(dom.text_content(root), "BA");
    backend.end_frame();
    bridge.run_turn(|| engine.evaluate_script("__zgui.removeNode(root, b); 'removed'")).unwrap();
    backend.end_frame();
    bridge.run_turn(|| engine.evaluate_script(r#"
        let staleRejected = false;
        try { __zgui.insertNode(root, b, null); } catch { staleRejected = true; }
        if (!staleRejected) throw Error('stale wrapper reused');
        const replacement = __zgui.createTextNode('C');
        __zgui.insertNode(root, replacement, a);
        if (replacement === b) throw Error('stale identity recycled');
        'recreated'
    "#)).unwrap();
    assert_eq!(dom.text_content(root), "CA");
    bridge.dispose();
    backend.end_frame();
    assert_eq!(dom.text_content(root), "");
    assert_eq!(bridge.live_count(), 0);
    assert_eq!(bridge.wrapper_count(), 0);
    bridge.dispose();
    drop(bridge);
    drop(engine);
}

#[test]
fn failed_construction_releases_attached_and_never_linked_native_nodes() {
    let document = Rc::new(RefCell::new(Document::new()));
    let backend = Rc::new(DocumentDom::new(document));
    let dom = DomHandle::from_rc(backend.clone());
    let root = dom.create_element(ElementName::new("column"));
    dom.insert(backend.root_node(), root, None);
    let mut engine = JsRuntime::new(RuntimeLimits::default()).unwrap();
    let bridge = UiBridge::new(NodeRegistry::new(dom.clone(), root, 8, 1, 100), HostHandle::new(StubHost::default()));
    engine.with_context(|ctx| { bridge.install(&ctx)?; ctx.globals().set("root", bridge.root(&ctx)?) }).unwrap();
    let error = bridge.run_turn(|| engine.evaluate_script(r#"
        const attached = __zgui.createTextNode('partial');
        const orphan = __zgui.createElement('column');
        const child = __zgui.createTextNode('never linked');
        __zgui.insertNode(orphan, child, null);
        __zgui.insertNode(root, attached, null);
        throw Error('construction failed');
    "#)).unwrap_err();
    assert!(error.message.contains("construction failed"));
    backend.end_frame();
    assert_eq!(dom.text_content(root), "");
    assert_eq!(bridge.live_count(), 0);
    assert_eq!(bridge.wrapper_count(), 0);
    let error = engine.evaluate_script("__zgui.createTextNode('after fault'); 'bad'").unwrap_err();
    assert!(error.message.contains("disposed"));
}

#[test]
fn stopping_rejects_new_work_but_allows_listener_and_tree_cleanup() {
    let document = Rc::new(RefCell::new(Document::new()));
    let backend = Rc::new(DocumentDom::new(document));
    let dom = DomHandle::from_rc(backend.clone());
    let root = dom.create_element(ElementName::new("column"));
    dom.insert(backend.root_node(), root, None);
    let mut engine = JsRuntime::new(RuntimeLimits::default()).unwrap();
    let bridge = UiBridge::new(NodeRegistry::new(dom.clone(), root, 9, 1, 100), HostHandle::new(StubHost::default()));
    bridge.set_dispatcher(Rc::new(|_, _| {}));
    engine.with_context(|ctx| { bridge.install(&ctx)?; ctx.globals().set("root", bridge.root(&ctx)?) }).unwrap();
    bridge.run_turn(|| engine.evaluate_script(r#"
        const control = __zgui.createElement('control');
        __zgui.insertNode(root, control, null);
        __zgui.setProperty(control, 'onClick', () => {});
        'mounted'
    "#)).unwrap();
    assert_eq!(bridge.callback_count(), 1);
    bridge.begin_stop();
    bridge.run_turn(|| engine.evaluate_script(r#"
        for (const create of [
            () => __zgui.createElement('box'),
            () => __zgui.createTextNode('late'),
            () => __zgui.insertNode(root, control, null),
            () => __zgui.setProperty(control, 'onClick', () => {})
        ]) {
            let rejected = false;
            try { create(); } catch (error) { rejected = error.message.includes('stopping'); }
            if (!rejected) throw Error('cleanup admitted new work');
        }
        __zgui.setProperty(control, 'onClick', null);
        __zgui.clearRoot(root);
        'cleaned'
    "#)).unwrap();
    backend.end_frame();
    assert_eq!(dom.text_content(root), "");
    assert!(dom.is_live(root));
    assert_eq!(bridge.live_count(), 0);
    assert_eq!(bridge.callback_count(), 0);
    bridge.dispose();
    assert_eq!(bridge.wrapper_count(), 0);
}
