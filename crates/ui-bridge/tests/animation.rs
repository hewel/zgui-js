use std::{cell::RefCell, rc::Rc};
use js_runtime::{JsRuntime, RuntimeLimits};
use ui_bridge::{UiBridge, registry::NodeRegistry};
use zgui::view::{DomHandle, ElementName, HostHandle, NodeId, stub::StubHost};
use zgui_dom::Document;
use zgui_view_dom::DocumentDom;

struct Animating {
    bridge: Rc<UiBridge>,
    engine: JsRuntime,
    backend: Rc<DocumentDom>,
    dom: DomHandle,
    host: Rc<StubHost>,
    root: NodeId,
}

impl Animating {
    fn new(session: u64) -> Self {
        let backend = Rc::new(DocumentDom::new(Rc::new(RefCell::new(Document::new()))));
        let dom = DomHandle::from_rc(backend.clone());
        let root = dom.create_element(ElementName::new("column"));
        dom.insert(backend.root_node(), root, None);
        let host = Rc::new(StubHost::default());
        let mut engine = JsRuntime::new(RuntimeLimits::default()).unwrap();
        let bridge = UiBridge::new(NodeRegistry::new(dom.clone(), root, session, 1, 100), HostHandle::from_rc(host.clone()));
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
fn is_animating_reports_the_host_running_count() {
    let mut native = Animating::new(83);
    native.script(r#"
        const a = __zgui.createElement('box');
        const b = __zgui.createElement('box');
        __zgui.insertNode(root, a, null);
        __zgui.insertNode(root, b, null);
        if (__zgui.isAnimating(a) !== false || __zgui.isAnimating(b) !== false)
            throw Error('a node without running animations is not false');
        'idle'
    "#);
    let (a, b) = (child(&native.backend, native.root, 0), child(&native.backend, native.root, 1));
    native.host.set_running_animations(a, 2);
    native.host.set_running_animations(b, 1);
    native.script(r#"
        if (__zgui.isAnimating(a) !== true || __zgui.isAnimating(b) !== true)
            throw Error('a node with running animations is not true');
        'running'
    "#);
    native.host.set_running_animations(a, 0);
    native.host.set_running_animations(b, 3);
    native.script(r#"
        if (__zgui.isAnimating(a) !== false || __zgui.isAnimating(b) !== true)
            throw Error('the last styled frame does not drive the answer');
        'updated'
    "#);
    native.bridge.dispose();
    drop(native);
}

#[test]
fn is_animating_rejects_fabricated_foreign_stale_and_disposed_nodes() {
    let mut native = Animating::new(84);
    native.script(r#"
        const a = __zgui.createElement('box');
        __zgui.insertNode(root, a, null);
        'mounted'
    "#);
    native.script(r#"
        const forged = Object.assign(Object.create(Object.getPrototypeOf(a)), a);
        { let rejected = false;
        try { __zgui.isAnimating(forged); } catch (error) { rejected = error instanceof TypeError; }
        if (!rejected) throw Error('fabricated wrapper accepted'); }
        'forged'
    "#);
    let session_foreign = UiBridge::new(
        NodeRegistry::new(native.dom.clone(), native.root, 92, 1, 100),
        HostHandle::new(StubHost::default()),
    );
    let generation_foreign = UiBridge::new(
        NodeRegistry::new(native.dom.clone(), native.root, 84, 2, 100),
        HostHandle::new(StubHost::default()),
    );
    native.engine.with_context(|ctx| {
        ctx.globals().set("foreignSession", session_foreign.root(&ctx)?)?;
        ctx.globals().set("foreignGeneration", generation_foreign.root(&ctx)?)
    }).unwrap();
    native.script(r#"
        for (const foreign of [foreignSession, foreignGeneration]) {
            let rejected = false;
            try { __zgui.isAnimating(foreign); } catch (error) { rejected = error instanceof TypeError; }
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
        try { __zgui.isAnimating(a); } catch (error) { rejected = error instanceof TypeError; }
        if (!rejected) throw Error('stale node accepted'); }
        'stale'
    "#);
    native.bridge.dispose();
    native.engine.evaluate_script(r#"
        { let rejected = false;
        try { __zgui.isAnimating(root); } catch (error) { rejected = error instanceof TypeError; }
        if (!rejected) throw Error('disposed owner answered a query'); }
        'disposed'
    "#).unwrap();
    drop(native);
}
