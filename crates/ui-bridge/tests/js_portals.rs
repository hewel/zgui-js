use std::{cell::RefCell, rc::Rc};
use js_runtime::{JsRuntime, RuntimeLimits};
use ui_bridge::{UiBridge, registry::NodeRegistry};
use zgui::view::{Dom, DomHandle, ElementName, HostHandle, NodeId, OverlayLayer, stub::StubHost};
use zgui_dom::Document;
use zgui_view_dom::DocumentDom;

struct Native {
    bridge: Rc<UiBridge>,
    engine: JsRuntime,
    document: Rc<RefCell<Document>>,
    backend: Rc<DocumentDom>,
    dom: DomHandle,
    host: Rc<StubHost>,
    root: NodeId,
}

impl Native {
    fn new(session: u64, limit: usize) -> Self {
        let document = Rc::new(RefCell::new(Document::new()));
        let backend = Rc::new(DocumentDom::new(document.clone()));
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
        Self { bridge, engine, document, backend, dom, host, root }
    }

    fn script(&mut self, script: &str) {
        self.bridge.run_turn(|| self.engine.evaluate_script(script)).unwrap();
    }
}

#[test]
fn portal_seam_exposes_logical_tree_and_releases_native_records() {
    let mut native = Native::new(80, 100);
    let baseline = native.document.borrow().len();
    let band = native.backend.overlay_layer(OverlayLayer::Toast);
    native.script(r#"
        const portal = __zgui.createPortal('toast');
        const text = __zgui.createTextNode('held');
        __zgui.insertNode(portal, text, null);
        __zgui.insertNode(root, portal, null);
        if (__zgui.getParentNode(portal) !== root) throw Error('portal parent is not the logical root');
        if (__zgui.getFirstChild(root) !== portal) throw Error('portal is not the first child of root');
        if (__zgui.getParentNode(text) !== portal) throw Error('text parent is not the portal');
        'mounted'
    "#);
    let snapshot = native.bridge.snapshot(100);
    let kinds: Vec<&str> = snapshot.nodes.iter().map(|node| node.kind).collect();
    assert_eq!(kinds, ["root", "portal", "text"]);
    let content = snapshot.nodes[1].native;
    assert_eq!(native.dom.parent(content), Some(band));
    assert!(snapshot.nodes.iter().all(|node| node.native != band));

    let live = native.bridge.live_count();
    native.script(r#"
        { let rejected = false;
        try { __zgui.createPortal('bogus'); } catch (error) { rejected = error instanceof TypeError; }
        if (!rejected) throw Error('unsupported layer accepted'); }
        'rejected'
    "#);
    assert_eq!(native.bridge.live_count(), live);

    native.bridge.begin_stop();
    native.script(r#"
        { let rejected = false;
        try { __zgui.createPortal('toast'); } catch (error) { rejected = error instanceof ReferenceError; }
        if (!rejected) throw Error('stopping session accepted a portal'); }
        'stopped'
    "#);
    assert_eq!(native.dom.parent(content), Some(band));

    native.bridge.dispose();
    assert_eq!(native.bridge.live_count(), 0);
    assert_eq!(native.bridge.wrapper_count(), 0);
    native.backend.end_frame();
    assert_eq!(native.document.borrow().len(), baseline);
}

#[test]
fn a_faulted_turn_leaves_shared_bands_empty() {
    let mut native = Native::new(81, 100);
    let baseline = native.document.borrow().len();
    let band = native.backend.overlay_layer(OverlayLayer::Toast);
    let result = native.bridge.run_turn(|| native.engine.evaluate_script(r#"
        const portal = __zgui.createPortal('toast');
        const text = __zgui.createTextNode('held');
        __zgui.insertNode(portal, text, null);
        __zgui.insertNode(root, portal, null);
        throw Error('abort');
    "#));
    assert!(result.is_err());
    native.backend.end_frame();
    assert_eq!(native.backend.text_content(band), "");
    assert_eq!(native.document.borrow().len(), baseline);
}
