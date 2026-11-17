use std::{cell::RefCell, rc::Rc};

use js_runtime::{JsRuntime, RuntimeLimits};
use ui_bridge::{UiBridge, registry::NodeRegistry};
use zgui::view::{DomHandle, ElementName, HostHandle, ViewHost, stub::StubHost};
use zgui_dom::Document;
use zgui_view_dom::DocumentDom;

struct Owner {
    bridge: Rc<UiBridge>,
    engine: JsRuntime,
}

struct Sheets {
    host: Rc<StubHost>,
    backend: Rc<DocumentDom>,
    dom: DomHandle,
}

impl Sheets {
    fn new() -> Self {
        let backend = Rc::new(DocumentDom::new(Rc::new(RefCell::new(Document::new()))));
        let dom = DomHandle::from_rc(backend.clone());
        Self { host: Rc::new(StubHost::default()), backend, dom }
    }

    fn owner(&self, session: u64, generation: u64) -> Owner {
        let root = self.dom.create_element(ElementName::new("column"));
        self.dom.insert(self.backend.root_node(), root, None);
        let mut engine = JsRuntime::new(RuntimeLimits::default()).unwrap();
        let bridge = UiBridge::new(
            NodeRegistry::new(self.dom.clone(), root, session, generation, 100),
            HostHandle::from_rc(self.host.clone()),
        );
        engine.with_context(|ctx| bridge.install(&ctx)).unwrap();
        Owner { bridge, engine }
    }
}

impl Owner {
    fn script(&mut self, script: &str) {
        self.bridge.run_turn(|| self.engine.evaluate_script(script)).unwrap();
    }

    fn failed(&mut self, script: &str) -> String {
        self.bridge.run_turn(|| self.engine.evaluate_script(script)).unwrap_err().message
    }
}

#[test]
fn names_are_scoped_to_the_owning_generation_and_updates_keep_cascade_position() {
    let sheets = Sheets::new();
    sheets.host.install_stylesheet("application", "text { color: black; }");
    let mut first = sheets.owner(5, 1);
    let mut second = sheets.owner(5, 2);
    first.script("__zgui.installStylesheet('theme', 'text { color: red; }'); 'a'");
    second.script("__zgui.installStylesheet('theme', 'text { color: blue; }'); 'b'");
    assert_eq!(sheets.host.stylesheet("zgui-js-5-1-theme").as_deref(), Some("text { color: red; }"));
    assert_eq!(sheets.host.stylesheet("zgui-js-5-2-theme").as_deref(), Some("text { color: blue; }"));
    assert_eq!(sheets.host.stylesheet("theme"), None);
    assert_eq!(sheets.host.stylesheet_names(), ["application", "zgui-js-5-1-theme", "zgui-js-5-2-theme"]);
    assert_eq!(sheets.host.stylesheet_installs(), 3);

    first.script("__zgui.installStylesheet('theme', 'text { color: green; }'); 'updated'");
    assert_eq!(sheets.host.stylesheet("zgui-js-5-1-theme").as_deref(), Some("text { color: green; }"));
    assert_eq!(sheets.host.stylesheet_names(), ["application", "zgui-js-5-1-theme", "zgui-js-5-2-theme"]);
    assert_eq!(sheets.host.stylesheet_installs(), 4);
    let installs = sheets.host.stylesheet_installs();
    first.script("__zgui.installStylesheet('theme', 'text { color: green; }'); 'duplicate'");
    assert_eq!(sheets.host.stylesheet_installs(), installs);

    first.bridge.dispose();
    assert_eq!(sheets.host.stylesheet_names(), ["application", "zgui-js-5-2-theme"]);
    assert_eq!(sheets.host.stylesheet("application").as_deref(), Some("text { color: black; }"));
    drop(first);
    drop(second);
}

#[test]
fn removal_is_tracked_and_repeated_removal_is_a_no_op() {
    let sheets = Sheets::new();
    let mut owner = sheets.owner(6, 1);
    owner.script("__zgui.installStylesheet('theme', 'box { width: 1px; }'); 'installed'");
    owner.script("__zgui.removeStylesheet('theme'); __zgui.removeStylesheet('theme'); 'removed'");
    assert_eq!(sheets.host.stylesheet_count(), 0);
    owner.script("__zgui.removeStylesheet('never-installed'); 'absent'");
    assert_eq!(sheets.host.stylesheet_count(), 0);
}

#[test]
fn begin_stop_blocks_installation_but_still_allows_removal() {
    let sheets = Sheets::new();
    let mut owner = sheets.owner(7, 1);
    owner.script("__zgui.installStylesheet('theme', 'box { width: 1px; }'); __zgui.installStylesheet('kept', 'box { width: 2px; }'); 'installed'");
    owner.bridge.begin_stop();
    owner.script(r#"
        let rejected = false;
        try { __zgui.installStylesheet('late', 'box { width: 3px; }'); }
        catch (error) { rejected = String(error).includes('stopping'); }
        if (!rejected) throw Error('install allowed while stopping');
        __zgui.removeStylesheet('theme');
        'removed'
    "#);
    assert_eq!(sheets.host.stylesheet_names(), ["zgui-js-7-1-kept"]);
    drop(owner);
    assert_eq!(sheets.host.stylesheet_count(), 0);
}

#[test]
fn faulted_turns_and_disposal_retire_every_owned_sheet_once() {
    let sheets = Sheets::new();
    let mut owner = sheets.owner(8, 1);
    let error = owner.failed(r#"
        __zgui.installStylesheet('leak', 'box { width: 1px; }');
        throw Error('construction failed');
    "#);
    assert!(error.contains("construction failed"), "{error}");
    assert_eq!(sheets.host.stylesheet_count(), 1);
    owner.bridge.dispose();
    assert_eq!(sheets.host.stylesheet_count(), 0);
    owner.bridge.dispose();
    assert_eq!(sheets.host.stylesheet_count(), 0);
    drop(owner);

    let mut owner = sheets.owner(8, 2);
    owner.script("__zgui.installStylesheet('kept', 'box { width: 2px; }'); __zgui.installStylesheet('also', 'box { width: 3px; }'); 'installed'");
    drop(owner);
    assert_eq!(sheets.host.stylesheet_count(), 0);
}

#[test]
fn sheet_mutations_require_a_turn_and_a_live_owner() {
    let sheets = Sheets::new();
    let mut owner = sheets.owner(9, 1);
    let error = owner.engine.evaluate_script("__zgui.installStylesheet('idle', 'box { width: 1px; }'); 'bad'").unwrap_err().message;
    assert!(error.contains("structural turn"), "{error}");
    let error = owner.engine.evaluate_script("__zgui.removeStylesheet('idle'); 'bad'").unwrap_err().message;
    assert!(error.contains("structural turn"), "{error}");

    owner.script("__zgui.installStylesheet('theme', 'box { width: 1px; }'); 'installed'");
    owner.bridge.dispose();
    let error = owner.engine.evaluate_script("__zgui.installStylesheet('late', 'box { width: 2px; }'); 'bad'").unwrap_err().message;
    assert!(error.contains("disposed"), "{error}");
    let error = owner.engine.evaluate_script("__zgui.removeStylesheet('theme'); 'bad'").unwrap_err().message;
    assert!(error.contains("disposed"), "{error}");
    drop(owner);
}

#[test]
fn invalid_keys_are_rejected_before_any_host_write() {
    let sheets = Sheets::new();
    let mut owner = sheets.owner(10, 1);
    owner.script(r#"
        const invalid = ['', 'has space', 'has/slash', 'unicode-é', 'x'.repeat(129)];
        for (const name of invalid) {
            let rejected = false;
            try { __zgui.installStylesheet(name, 'box { width: 1px; }'); } catch (error) { rejected = error instanceof TypeError; }
            if (!rejected) throw Error('install accepted invalid name ' + JSON.stringify(name));
            rejected = false;
            try { __zgui.removeStylesheet(name); } catch (error) { rejected = error instanceof TypeError; }
            if (!rejected) throw Error('remove accepted invalid name ' + JSON.stringify(name));
        }
        for (const name of ['a', 'A-B_c.d:e', 'x'.repeat(128)]) {
            __zgui.installStylesheet(name, 'box { width: 1px; }');
            __zgui.removeStylesheet(name);
        }
        'checked'
    "#);
    assert_eq!(sheets.host.stylesheet_count(), 0);
}
