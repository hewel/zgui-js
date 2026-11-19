use std::{cell::RefCell, rc::Rc};
use js_runtime::{JsRuntime, RuntimeLimits};
use ui_bridge::{HostPort, UiBridge, registry::NodeRegistry};
use zgui::{
    geom::{Device, DevicePx, Point, Rect, Size},
    view::{DomHandle, ElementName, HostHandle, NodeId, ObservedValue, ScrollPosition, stub::StubHost},
};
use zgui_dom::Document;
use zgui_view_dom::DocumentDom;

struct Geometry {
    bridge: Rc<UiBridge>,
    engine: JsRuntime,
    backend: Rc<DocumentDom>,
    dom: DomHandle,
    host: Rc<StubHost>,
    root: NodeId,
}

impl Geometry {
    fn new(session: u64, limit: usize) -> Self {
        Self::with_backend(session, limit, Rc::new(DocumentDom::new(Rc::new(RefCell::new(Document::new())))), Rc::new(StubHost::default()))
    }

    fn with_backend(session: u64, limit: usize, backend: Rc<DocumentDom>, host: Rc<StubHost>) -> Self {
        let dom = DomHandle::from_rc(backend.clone());
        let root = dom.create_element(ElementName::new("column"));
        dom.insert(backend.root_node(), root, None);
        let mut engine = JsRuntime::new(RuntimeLimits::default()).unwrap();
        let bridge = UiBridge::new(
            NodeRegistry::new(dom.clone(), root, session, 1, limit),
            HostHandle::from_rc(host.clone()),
            HostPort::queued(),
        );
        engine.with_context(|ctx| {
            bridge.install(&ctx)?;
            ctx.globals().set("root", bridge.root(&ctx)?)
        }).unwrap();
        Self { bridge, engine, backend, dom, host, root }
    }

    fn script(&mut self, script: &str) {
        self.bridge.run_turn(self.engine.turn(), |_, js| js.evaluate_script(script)).unwrap();
    }

    fn child(&self, index: usize) -> NodeId {
        self.backend.children(self.root)[index]
    }

    fn deliver(&mut self, node: NodeId, value: ObservedValue) {
        self.backend.deliver(node, value);
        self.flush();
    }

    fn flush(&mut self) {
        while self
            .bridge
            .flush_notification(self.engine.turn(), |js, cb| js.with_context(|ctx| cb.call(&ctx)))
            .unwrap()
        {}
    }

    fn deliver_rejects(&mut self, node: NodeId, value: ObservedValue, constructor: &'static str) {
        self.backend.invalidate_observations();
        self.backend.deliver(node, value);
        let mut count = 0usize;
        while self
            .bridge
            .flush_notification(self.engine.turn(), |js, cb| {
                count += 1;
                js.with_context(|ctx| {
                    assert!(cb.call(&ctx).is_err());
                    let thrown = ctx.catch();
                    ctx.globals().set("thrown", thrown)?;
                    let accepted = ctx.eval::<bool, _>(format!("thrown instanceof {constructor}"))?;
                    assert!(accepted, "expected {constructor}");
                    Ok(())
                })
            })
            .unwrap()
        {
            self.engine.evaluate_script("thrown = undefined; 'cleared'").unwrap();
        }
        assert!(count > 0, "rejection case produced no notification");
    }
}

fn device_rect(x: f32, y: f32, width: f32, height: f32) -> Rect<DevicePx, Device> {
    Rect::new(Point::new(DevicePx(x), DevicePx(y)), Size::new(DevicePx(width), DevicePx(height)))
}

#[test]
fn geometry_reports_css_pixel_snapshots_for_every_quantity() {
    let mut native = Geometry::new(71, 100);
    native.host.set_scale(1.75);
    native.script(r#"
        const a = __zgui.createElement('box');
        __zgui.insertNode(root, a, null);
        globalThis.samples = { borderBox: [], borderSize: [], contentSize: [], scrollPosition: [] };
        for (const kind of ['borderBox', 'borderSize', 'contentSize', 'scrollPosition']) {
            __zgui.observeGeometry(a, kind, value => samples[kind].push(value));
        }
        if (samples.borderBox.length !== 0) throw Error('registration invoked a callback');
        'mounted'
    "#);
    assert_eq!(native.bridge.take_wakes(), 4);
    let a = native.child(0);
    native.deliver(a, ObservedValue::BorderBox(device_rect(87.5, -17.5, 350.0, 70.0)));
    native.deliver(a, ObservedValue::BorderSize(Size::new(DevicePx(350.0), DevicePx(70.0))));
    native.deliver(a, ObservedValue::ContentSize(Size::new(DevicePx(280.0), DevicePx(52.5))));
    native.deliver(a, ObservedValue::ScrollPosition(ScrollPosition {
        offset: Point::new(DevicePx(35.0), DevicePx(70.0)),
        content_size: Size::new(DevicePx(350.0), DevicePx(700.0)),
        scrollport: Size::new(DevicePx(175.0), DevicePx(87.5)),
    }));
    native.script(r#"
        const box = samples.borderBox[0];
        if (samples.borderBox.length !== 1 || box.x !== 50 || box.y !== -10 || box.width !== 200 || box.height !== 40)
            throw Error('borderBox ' + JSON.stringify(box));
        const border = samples.borderSize[0];
        if (samples.borderSize.length !== 1 || border.width !== 200 || border.height !== 40)
            throw Error('borderSize ' + JSON.stringify(border));
        const content = samples.contentSize[0];
        if (samples.contentSize.length !== 1 || content.width !== 160 || content.height !== 30)
            throw Error('contentSize ' + JSON.stringify(content));
        const scroll = samples.scrollPosition[0];
        if (samples.scrollPosition.length !== 1 || scroll.offset.x !== 20 || scroll.offset.y !== 40
            || scroll.contentSize.width !== 200 || scroll.contentSize.height !== 400
            || scroll.scrollport.width !== 100 || scroll.scrollport.height !== 50)
            throw Error('scrollPosition ' + JSON.stringify(scroll));
        box.x = -999;
        'checked'
    "#);
    native.deliver(a, ObservedValue::BorderSize(Size::new(DevicePx(350.0), DevicePx(70.0))));
    native.host.set_scale(3.5);
    native.deliver(a, ObservedValue::BorderBox(device_rect(175.0, -35.0, 700.0, 140.0)));
    native.backend.invalidate_observations();
    native.deliver(a, ObservedValue::BorderBox(device_rect(87.5, -17.5, 350.0, 70.0)));
    native.script(r#"
        const scaled = samples.borderBox[1];
        if (scaled.x !== 25 || scaled.y !== -5 || scaled.width !== 100 || scaled.height !== 20)
            throw Error('rescaled borderBox ' + JSON.stringify(scaled));
        if (scaled === samples.borderBox[0]) throw Error('rescaled snapshot aliases the first');
        scaled.x = 7;
        'mutated'
    "#);
    native.backend.invalidate_observations();
    native.deliver(a, ObservedValue::BorderBox(device_rect(87.5, -17.5, 350.0, 70.0)));
    native.script(r#"
        if (samples.borderBox.length !== 2) throw Error('expected two borderBox callbacks, got ' + samples.borderBox.length);
        const latest = samples.borderBox[1];
        if (latest.width !== 100 || latest.height !== 20)
            throw Error('rescaled borderBox ' + JSON.stringify(latest));
        if (samples.borderBox[0].x !== -999) throw Error('host state rewrote an emitted snapshot');
        if (samples.borderSize.length !== 1) throw Error('equal physical sample re-entered JavaScript');
        'verified'
    "#);
    assert_eq!(native.bridge.callback_count(), 4);
    native.bridge.dispose();
    assert_eq!(native.backend.observation_count(), 0);
    drop(native);
}

#[test]
fn geometry_subscriptions_are_independent_idempotent_and_retire_with_nodes() {
    let mut native = Geometry::new(72, 100);
    native.script(r#"
        const a = __zgui.createElement('box');
        const b = __zgui.createElement('box');
        __zgui.insertNode(root, a, null);
        __zgui.insertNode(root, b, null);
        globalThis.hits = { a: 0, b: 0 };
        globalThis.cancelA = __zgui.observeGeometry(a, 'borderSize', () => { hits.a += 1; });
        globalThis.cancelB = __zgui.observeGeometry(b, 'borderSize', () => { hits.b += 1; });
        'mounted'
    "#);
    let a = native.child(0);
    let b = native.child(1);
    assert_eq!(native.backend.observation_count(), 2);
    native.script("cancelA(); cancelA(); 'cancelled'");
    assert_eq!(native.bridge.callback_count(), 1);
    assert_eq!(native.backend.observation_count(), 1);
    native.deliver(a, ObservedValue::BorderSize(Size::new(DevicePx(10.0), DevicePx(10.0))));
    native.deliver(b, ObservedValue::BorderSize(Size::new(DevicePx(10.0), DevicePx(10.0))));
    native.script("if (hits.a !== 0 || hits.b !== 1) throw Error('cancelled observer received a sample'); 'checked'");
    native.script("__zgui.removeNode(root, b); __zgui.insertNode(root, b, null); 'reinserted'");
    assert_eq!(native.backend.observation_count(), 1);
    native.deliver(b, ObservedValue::BorderSize(Size::new(DevicePx(20.0), DevicePx(20.0))));
    native.script("if (hits.b !== 2) throw Error('reinserted node lost its observation'); 'checked'");
    native.script("__zgui.removeNode(root, b); 'removed'");
    assert_eq!(native.backend.observation_count(), 0);
    assert_eq!(native.bridge.callback_count(), 0);
    native.backend.deliver(b, ObservedValue::BorderSize(Size::new(DevicePx(30.0), DevicePx(30.0))));
    native.script("if (hits.b !== 2) throw Error('removed node observed again'); 'still'");
    native.bridge.dispose();
    native.engine.evaluate_script("cancelA(); cancelB(); 'late'").unwrap();
    drop(native);
}

#[test]
fn opt_in_geometry_observer_reports_mount_attachment_changes_synchronously() {
    let mut native = Geometry::new(75, 100);
    native.script(r#"
        const branch = __zgui.createElement('box');
        const node = __zgui.createElement('box');
        __zgui.insertNode(branch, node, null);
        __zgui.insertNode(root, branch, null);
        globalThis.samples = [];
        globalThis.attachments = [];
        __zgui.observeGeometry(node, 'borderBox', rect => samples.push(rect),
            attached => attachments.push(attached));
        'mounted'
    "#);
    let node = native.backend.children(native.child(0))[0];
    native.deliver(node, ObservedValue::BorderBox(device_rect(2.0, 4.0, 10.0, 20.0)));
    native.script(r#"
        __zgui.removeNode(root, branch);
        if (attachments.join() !== 'true,false') throw Error('ancestor detach was not reported');
        __zgui.insertNode(root, branch, null);
        if (attachments.join() !== 'true,false,true') throw Error('reattach was not reported');
        'moved'
    "#);
    native.backend.deliver(node, ObservedValue::BorderBox(device_rect(8.0, 6.0, 10.0, 20.0)));
    native.script(r#"
        __zgui.clearRoot(root);
        if (attachments.join() !== 'true,false,true,false') throw Error('root clear was not reported');
        'removed'
    "#);
    native.flush();
    native.script("if (samples.length !== 1) throw Error('removed node observed again'); 'checked'");
    assert_eq!(native.bridge.callback_count(), 0);
    assert_eq!(native.backend.observation_count(), 0);
}

#[test]
fn geometry_rejects_forged_foreign_stale_and_invalid_arguments() {
    let mut native = Geometry::new(73, 100);
    native.script(r#"
        const a = __zgui.createElement('box');
        __zgui.insertNode(root, a, null);
        'mounted'
    "#);
    native.script(r#"
        const forged = Object.assign(Object.create(Object.getPrototypeOf(a)), a);
        for (const attempt of [
            () => __zgui.observeGeometry(forged, 'borderSize', () => {}),
            () => __zgui.observeGeometry(forged, 'viewport', () => {}),
            () => __zgui.observeGeometry(a, 'diagonal', () => {}),
            () => __zgui.observeGeometry(a, 'borderSize', 'not a function'),
            () => __zgui.observeGeometry(a, 'borderSize', null),
        ]) {
            let rejected = false;
            try { attempt(); } catch (error) { rejected = error instanceof TypeError; }
            if (!rejected) throw Error('invalid geometry registration accepted');
        }
        'rejected'
    "#);
    assert_eq!(native.backend.observation_count(), 0);
    assert_eq!(native.bridge.callback_count(), 0);
    let session_foreign = UiBridge::new(
        NodeRegistry::new(native.dom.clone(), native.root, 91, 1, 100),
        HostHandle::new(StubHost::default()),
        HostPort::queued(),
    );
    let generation_foreign = UiBridge::new(
        NodeRegistry::new(native.dom.clone(), native.root, 73, 2, 100),
        HostHandle::new(StubHost::default()),
        HostPort::queued(),
    );
    native.engine.with_context(|ctx| {
        ctx.globals().set("foreignSession", session_foreign.root(&ctx)?)?;
        ctx.globals().set("foreignGeneration", generation_foreign.root(&ctx)?)
    }).unwrap();
    native.script(r#"
        for (const foreign of [foreignSession, foreignGeneration]) {
            for (const kind of ['borderSize', 'viewport']) {
                let rejected = false;
                try { __zgui.observeGeometry(foreign, kind, () => {}); } catch (error) { rejected = error instanceof TypeError; }
                if (!rejected) throw Error('foreign node accepted');
            }
        }
        'foreign'
    "#);
    session_foreign.dispose();
    generation_foreign.dispose();
    assert_eq!(native.backend.observation_count(), 0);
    native.script("__zgui.removeNode(root, a); 'removed'");
    native.backend.end_frame();
    native.script(r#"
        for (const kind of ['borderSize', 'viewport']) {
            let rejected = false;
            try { __zgui.observeGeometry(a, kind, () => {}); } catch (error) { rejected = error instanceof TypeError; }
            if (!rejected) throw Error('stale node accepted');
        }
        'stale'
    "#);
    assert_eq!(native.backend.observation_count(), 0);
    drop(native);
}

#[test]
fn geometry_registration_requires_an_active_turn_but_cancellation_stays_harmless() {
    let mut native = Geometry::new(74, 100);
    native.script("globalThis.cancelA = __zgui.observeGeometry(root, 'borderSize', () => {}); 'registered'");
    assert!(native.engine.evaluate_script("__zgui.observeGeometry(root, 'borderSize', () => {}); 'unreachable'").is_err());
    native.engine.evaluate_script("cancelA(); 'cancelled'").unwrap();
    assert_eq!(native.bridge.callback_count(), 0);
    native.script("globalThis.cancelB = __zgui.observeGeometry(root, 'borderSize', () => {}); 'registered'");
    native.bridge.begin_stop();
    native.script(r#"
        try { __zgui.observeGeometry(root, 'borderSize', () => {}); throw Error('stopping session accepted a registration'); }
        catch (error) { if (!(error instanceof ReferenceError)) throw error; }
        cancelB();
        'stopped'
    "#);
    assert_eq!(native.bridge.callback_count(), 0);
    drop(native);
}

#[test]
fn geometry_pool_is_bounded_by_four_times_the_node_limit() {
    let mut native = Geometry::new(75, 1);
    native.script(r#"
        globalThis.cancels = [];
        globalThis.hits = 0;
        for (let index = 0; index < 4; index += 1) {
            cancels.push(__zgui.observeGeometry(root, 'borderSize', () => { hits += 1; }));
        }
        let overflow = null;
        try { __zgui.observeGeometry(root, 'borderSize', () => {}); } catch (error) { overflow = error; }
        if (!(overflow instanceof RangeError)) throw Error('fifth observation was not a RangeError');
        'bounded'
    "#);
    assert_eq!(native.bridge.callback_count(), 4);
    native.script(r#"
        cancels[0]();
        cancels.push(__zgui.observeGeometry(root, 'borderSize', () => { hits += 1; }));
        'refilled'
    "#);
    assert_eq!(native.bridge.callback_count(), 4);
    native.bridge.dispose();
    assert_eq!(native.backend.observation_count(), 0);
    drop(native);
}

#[test]
fn geometry_rejects_nonfinite_scale_and_components() {
    let mut native = Geometry::new(76, 100);
    native.host.set_scale(1.75);
    native.script(r#"
        const a = __zgui.createElement('box');
        __zgui.insertNode(root, a, null);
        globalThis.hits = 0;
        __zgui.observeGeometry(a, 'borderBox', () => { hits += 1; });
        'mounted'
    "#);
    let a = native.child(0);
    for scale in [0.0_f32, f32::NAN, f32::INFINITY] {
        native.host.set_scale(scale);
        native.deliver_rejects(a, ObservedValue::BorderBox(device_rect(1.0, 2.0, 3.0, 4.0)), "RangeError");
    }
    native.host.set_scale(1.75);
    native.deliver_rejects(a, ObservedValue::BorderBox(device_rect(f32::INFINITY, 0.0, 3.0, 4.0)), "RangeError");
    native.deliver_rejects(a, ObservedValue::BorderBox(device_rect(f32::NAN, 0.0, 3.0, 4.0)), "RangeError");
    native.script("if (hits !== 0) throw Error('nonfinite sample reached JavaScript'); 'clean'");
    native.deliver(a, ObservedValue::BorderBox(device_rect(87.5, -17.5, 350.0, 70.0)));
    native.script("if (hits !== 1) throw Error('valid sample was not delivered after rejections'); 'alive'");
    drop(native);
}

#[test]
fn viewport_samples_follow_the_window_and_keep_density_only_changes() {
    let mut native = Geometry::new(81, 100);
    native.host.set_scale(1.75);
    native.script(r#"
        const source = __zgui.createElement('box');
        globalThis.samples = [];
        globalThis.cancel = __zgui.observeGeometry(source, 'viewport', value => samples.push(value));
        if (samples.length) throw Error('registration delivered synchronously');
        __zgui.insertNode(root, source, null);
        'mounted'
    "#);
    assert_eq!(native.backend.observation_count(), 1);
    let window = native.backend.root_node();
    native.deliver(native.child(0), ObservedValue::BorderBox(device_rect(1.0, 2.0, 3.0, 4.0)));
    native.script("if (samples.length) throw Error('source box delivered as viewport'); 'checked'");
    native.deliver(window, ObservedValue::BorderBox(device_rect(17.5, 35.0, 1400.0, 1050.0)));
    native.script(r#"
        const first = samples[0];
        if (samples.length !== 1 || first.x !== 10 || first.y !== 20 || first.width !== 800 || first.height !== 600 || first.scale !== 1.75)
            throw Error('viewport units ' + JSON.stringify(samples));
        if (Object.keys(first).sort().join(',') !== 'height,scale,width,x,y') throw Error('unexpected viewport authority');
        first.scale = -1;
        'checked'
    "#);
    native.host.set_scale(2.0);
    native.deliver(window, ObservedValue::BorderBox(device_rect(20.0, 40.0, 1600.0, 1200.0)));
    native.deliver(window, ObservedValue::BorderBox(device_rect(20.0, 40.0, 1600.0, 1200.0)));
    native.script(r#"
        const next = samples[1];
        if (samples.length !== 2 || next.width !== 800 || next.height !== 600 || next.x !== 10 || next.y !== 20 || next.scale !== 2)
            throw Error('density-only delivery ' + JSON.stringify(samples));
        if (samples[0].scale !== -1 || next === samples[0]) throw Error('snapshot alias');
        cancel(); cancel(); 'cancelled'
    "#);
    assert_eq!(native.backend.observation_count(), 0);
    assert_eq!(native.bridge.callback_count(), 0);
}

#[test]
fn shared_viewport_leases_retire_with_their_source_and_session() {
    let mut a = Geometry::new(82, 100);
    let mut b = Geometry::with_backend(83, 100, a.backend.clone(), a.host.clone());
    let mount = r#"
        const source = __zgui.createElement('box');
        __zgui.insertNode(root, source, null);
        globalThis.hits = 0;
        globalThis.cancel = __zgui.observeGeometry(source, 'viewport', () => { hits += 1; });
        'mounted'
    "#;
    a.script(mount); b.script(mount);
    assert_eq!(a.backend.observation_count(), 1);
    let window = a.backend.root_node();
    a.backend.deliver(window, ObservedValue::BorderBox(device_rect(0.0, 0.0, 800.0, 600.0)));
    a.script("__zgui.removeNode(root, source); 'removed'");
    a.flush(); b.flush();
    a.script("if (hits !== 0) throw Error('queued callback survived owner removal'); cancel(); 'checked'");
    b.script("if (hits !== 1) throw Error('other session lost its viewport callback'); 'checked'");
    assert_eq!(a.backend.observation_count(), 1);
    assert_eq!(a.bridge.callback_count(), 0);
    a.script("globalThis.again = __zgui.observeGeometry(root, 'viewport', () => { hits += 1; }); 'rejoined'");
    a.backend.deliver(window, ObservedValue::BorderBox(device_rect(0.0, 0.0, 800.0, 600.0)));
    a.flush(); b.flush();
    a.script("if (hits !== 1) throw Error('late viewport subscription missed first sample'); 'checked'");
    b.script("if (hits !== 1) throw Error('unchanged viewport repeated'); 'checked'");
    a.bridge.dispose();
    assert_eq!(a.backend.observation_count(), 1);
    b.deliver(window, ObservedValue::BorderBox(device_rect(0.0, 0.0, 900.0, 650.0)));
    b.script("if (hits !== 2) throw Error('other session disposal removed viewport'); 'checked'");
    b.bridge.dispose();
    assert_eq!(a.backend.observation_count(), 0);
    assert_eq!(b.bridge.callback_count(), 0);
}

#[test]
fn geometry_root_and_child_observers_retire_on_abort() {
    let mut native = Geometry::new(77, 100);
    let faulted = native.bridge.run_turn(native.engine.turn(), |_, js| js.evaluate_script(r#"
        const a = __zgui.createElement('box');
        __zgui.insertNode(root, a, null);
        __zgui.observeGeometry(root, 'borderSize', () => {});
        __zgui.observeGeometry(a, 'contentSize', () => {});
        throw Error('fault');
    "#));
    assert!(faulted.is_err());
    assert!(native.bridge.is_disposed());
    assert_eq!(native.bridge.callback_count(), 0);
    assert_eq!(native.backend.observation_count(), 0);
    drop(native);
}
