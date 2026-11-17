use std::{cell::RefCell, rc::Rc};
use js_runtime::{JsRuntime, RuntimeLimits};
use ui_bridge::{UiBridge, registry::NodeRegistry};
use zgui::{
    geom::{Device, DevicePx, Point, Rect, Size},
    view::{DomHandle, ElementName, HostHandle, NodeId, ScrollBehavior, ScrollTarget, ViewHost, stub::StubHost},
};
use zgui::reactive::prelude::GetUntracked;
use zgui_dom::Document;
use zgui_view_dom::DocumentDom;

struct Queries {
    bridge: Rc<UiBridge>,
    engine: JsRuntime,
    backend: Rc<DocumentDom>,
    dom: DomHandle,
    host: Rc<StubHost>,
    root: NodeId,
}

impl Queries {
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

fn device_rect(x: f32, y: f32, width: f32, height: f32) -> Rect<DevicePx, Device> {
    Rect::new(Point::new(DevicePx(x), DevicePx(y)), Size::new(DevicePx(width), DevicePx(height)))
}

#[test]
fn window_rect_reports_css_pixels_from_the_last_frame() {
    let mut native = Queries::new(71);
    native.script(r#"
        const a = __zgui.createElement('box');
        __zgui.insertNode(root, a, null);
        if (__zgui.getWindowRect(a) !== null) throw Error('unanswered layout read is not null');
        'mounted'
    "#);
    let a = child(&native.backend, native.root, 0);
    native.host.set_scale(1.75);
    native.host.set_border_box(a, device_rect(87.5, -17.5, 350.0, 70.0));
    native.script(r#"
        const scaled = __zgui.getWindowRect(a);
        if (scaled.x !== 50 || scaled.y !== -10 || scaled.width !== 200 || scaled.height !== 40)
            throw Error(`wrong rect: ${JSON.stringify(scaled)}`);
        globalThis.saved = scaled;
        'scaled'
    "#);
    native.host.set_border_box(a, device_rect(175.0, 0.0, 175.0, 35.0));
    native.script(r#"
        const rect = __zgui.getWindowRect(a);
        if (rect.x !== 100 || rect.y !== 0 || rect.width !== 100 || rect.height !== 20)
            throw Error(`changed rect: ${JSON.stringify(rect)}`);
        if (saved.x !== 50 || saved.width !== 200) throw Error('earlier rect was not a snapshot');
        saved.x = 999;
        const fresh = __zgui.getWindowRect(a);
        if (fresh === saved || fresh.x !== 100) throw Error('rect result is shared or backed by host state');
        'snapshots'
    "#);
    native.bridge.dispose();
    drop(native);
}

#[test]
fn precedes_follows_host_tree_order_not_registration() {
    let mut native = Queries::new(72);
    native.script(r#"
        const a = __zgui.createElement('box');
        const b = __zgui.createElement('box');
        const c = __zgui.createElement('box');
        const detached = __zgui.createElement('box');
        __zgui.insertNode(root, a, null);
        __zgui.insertNode(root, b, null);
        __zgui.insertNode(b, c, null);
        'mounted'
    "#);
    let (a, b) = (child(&native.backend, native.root, 0), child(&native.backend, native.root, 1));
    let c = child(&native.backend, b, 0);
    native.host.set_tree_order(vec![native.root, a, b, c]);
    native.script(r#"
        if (!__zgui.precedes(a, b)) throw Error('a should precede b');
        if (!__zgui.precedes(b, c)) throw Error('b should precede c');
        if (__zgui.precedes(a, a)) throw Error('a precedes itself');
        if (__zgui.precedes(c, a)) throw Error('c should not precede a');
        if (__zgui.precedes(detached, a) || __zgui.precedes(a, detached))
            throw Error('unattached node participates in order');
        'ordered'
    "#);
    native.script("__zgui.insertNode(root, b, a); 'moved'");
    native.host.set_tree_order(vec![native.root, b, c, a]);
    native.script(r#"
        if (!__zgui.precedes(c, a)) throw Error('moved descendant should precede a');
        if (__zgui.precedes(a, b)) throw Error('a should not precede b after the move');
        'reordered'
    "#);
    native.bridge.dispose();
    drop(native);
}

#[test]
fn queries_reject_fabricated_foreign_stale_and_disposed_nodes() {
    let mut native = Queries::new(73);
    native.script(r#"
        const a = __zgui.createElement('box');
        __zgui.insertNode(root, a, null);
        'mounted'
    "#);
    native.script(r#"
        const forged = Object.assign(Object.create(Object.getPrototypeOf(a)), a);
        for (const query of [
            () => __zgui.getWindowRect(forged),
            () => __zgui.getTextContent(forged),
            () => __zgui.precedes(forged, a),
            () => __zgui.precedes(a, forged),
        ]) {
            let rejected = false;
            try { query(); } catch (error) { rejected = error instanceof TypeError; }
            if (!rejected) throw Error('fabricated wrapper accepted');
        }
        'forged'
    "#);
    let session_foreign = UiBridge::new(
        NodeRegistry::new(native.dom.clone(), native.root, 91, 1, 100),
        HostHandle::new(StubHost::default()),
    );
    let generation_foreign = UiBridge::new(
        NodeRegistry::new(native.dom.clone(), native.root, 73, 2, 100),
        HostHandle::new(StubHost::default()),
    );
    native.engine.with_context(|ctx| {
        ctx.globals().set("foreignSession", session_foreign.root(&ctx)?)?;
        ctx.globals().set("foreignGeneration", generation_foreign.root(&ctx)?)
    }).unwrap();
    native.script(r#"
        for (const foreign of [foreignSession, foreignGeneration]) {
            for (const query of [
                () => __zgui.getWindowRect(foreign),
                () => __zgui.getTextContent(foreign),
                () => __zgui.precedes(foreign, a),
                () => __zgui.precedes(a, foreign),
            ]) {
                let rejected = false;
                try { query(); } catch (error) { rejected = error instanceof TypeError; }
                if (!rejected) throw Error('foreign node accepted');
            }
        }
        'foreign'
    "#);
    session_foreign.dispose();
    generation_foreign.dispose();
    native.script("__zgui.removeNode(root, a); 'removed'");
    native.backend.end_frame();
    native.script(r#"
        for (const query of [
            () => __zgui.getWindowRect(a),
            () => __zgui.getTextContent(a),
            () => __zgui.precedes(a, root),
            () => __zgui.precedes(root, a),
        ]) {
            let rejected = false;
            try { query(); } catch (error) { rejected = error instanceof TypeError; }
            if (!rejected) throw Error('stale node accepted');
        }
        for (const bad of [{}, null, 'node']) {
            let rejected = false;
            try { __zgui.getTextContent(bad); } catch (error) { rejected = error instanceof TypeError; }
            if (!rejected) throw Error('non-node argument accepted');
        }
        'stale'
    "#);
    native.bridge.dispose();
    native.engine.evaluate_script(r#"
        for (const query of [() => __zgui.getWindowRect(root), () => __zgui.getTextContent(root)]) {
            let rejected = false;
            try { query(); } catch (error) { rejected = error instanceof TypeError; }
            if (!rejected) throw Error('disposed owner answered a query');
        }
        'disposed'
    "#).unwrap();
    drop(native);
}

#[test]
fn text_queries_read_native_updates_and_moves_as_owned_unicode_snapshots() {
    let mut native = Queries::new(76);
    native.script(r#"
        const a = __zgui.createElement('box');
        const first = __zgui.createTextNode('  Save α');
        const b = __zgui.createElement('box');
        const last = __zgui.createTextNode('漢字');
        const detached = __zgui.createElement('box');
        if (__zgui.getTextContent(detached) !== '') throw Error('empty subtree');
        __zgui.insertNode(a, first, null);
        __zgui.insertNode(b, last, null);
        __zgui.insertNode(a, b, null);
        if (__zgui.getTextContent(a) !== '  Save α漢字') throw Error('detached live subtree');
        __zgui.insertNode(root, a, null);
        globalThis.saved = __zgui.getTextContent(a);
        if (saved !== '  Save α漢字' || __zgui.getTextContent(first) !== '  Save α') throw Error('native text');
        __zgui.replaceText(last, 'é𐐷');
        if (__zgui.getTextContent(a) !== '  Save αé𐐷') throw Error('updated unicode');
        if (saved !== '  Save α漢字') throw Error('snapshot changed');
        __zgui.insertNode(root, b, a);
        if (__zgui.getTextContent(root) !== 'é𐐷  Save α') throw Error('tree order');
        'read'
    "#);
    let a = child(&native.backend, native.root, 1);
    let first = child(&native.backend, a, 0);
    native.dom.set_text(first, "Native change");
    native.script("if (__zgui.getTextContent(a) !== 'Native change') throw Error('cached JS text'); 'native'");
    native.bridge.dispose();
}

#[test]
fn nonfinite_scale_or_bounds_reject_instead_of_leaking() {
    let mut native = Queries::new(74);
    native.script(r#"
        const a = __zgui.createElement('box');
        __zgui.insertNode(root, a, null);
        'mounted'
    "#);
    let a = child(&native.backend, native.root, 0);
    native.host.set_border_box(a, device_rect(0.0, 0.0, 10.0, 10.0));
    for scale in [0.0, f32::NAN] {
        native.host.set_scale(scale);
        native.script(r#"
            { let rejected = false;
            try { __zgui.getWindowRect(a); } catch (error) { rejected = error instanceof RangeError; }
            if (!rejected) throw Error('invalid scale accepted'); }
            'scale'
        "#);
    }
    native.host.set_scale(1.0);
    for bounds in [
        device_rect(f32::NAN, 0.0, 10.0, 10.0),
        device_rect(0.0, 0.0, f32::INFINITY, 10.0),
    ] {
        native.host.set_border_box(a, bounds);
        native.script(r#"
            { let rejected = false;
            try { __zgui.getWindowRect(a); } catch (error) { rejected = error instanceof RangeError; }
            if (!rejected) throw Error('nonfinite bounds accepted'); }
            'bounds'
        "#);
    }
    native.bridge.dispose();
    drop(native);
}

#[test]
fn focus_reveal_is_an_explicit_opt_in_queued_behind_the_focus_command() {
    let mut native = Queries::new(75);
    native.script(r#"
        const a = __zgui.createElement('box');
        __zgui.insertNode(root, a, null);
        __zgui.focus(a);
        'plain'
    "#);
    let a = child(&native.backend, native.root, 0);
    assert_eq!(native.host.focused().get_untracked(), Some(a));
    assert!(native.host.scroll_commands().is_empty());
    native.script("__zgui.focus(a, false); 'explicit-default'");
    assert!(native.host.scroll_commands().is_empty());
    native.script("__zgui.focus(a, true); 'revealed'");
    assert_eq!(
        native.host.scroll_commands(),
        [(a, ScrollTarget::IntoView, ScrollBehavior::Instant)],
    );
    native.script(r#"
        const forged = Object.assign(Object.create(Object.getPrototypeOf(a)), a);
        for (const bad of [{}, forged]) {
            let rejected = false;
            try { __zgui.focus(bad, true); } catch (error) { rejected = error instanceof TypeError; }
            if (!rejected) throw Error('reveal accepted an invalid handle');
        }
        'rejected'
    "#);
    assert_eq!(native.host.scroll_commands().len(), 1, "a rejected handle must not enqueue a reveal");
    native.bridge.dispose();
    drop(native);
}
