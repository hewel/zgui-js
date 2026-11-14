use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

use js_runtime::{JsRuntime, RuntimeLimits};
use ui_bridge::{UiBridge, registry::NodeRegistry};
use zgui::{view::{AttrName, DomHandle, ElementName, HostHandle, NodeId, UiState, stub::StubHost}, vocab::{Role, SemanticFlags}};
use zgui_dom::Document;
use zgui_view_dom::DocumentDom;

struct NativeProperties {
    // Persistent wrapper roots must be released before the engine, including on unwind.
    bridge: Rc<UiBridge>,
    engine: JsRuntime,
    backend: Rc<DocumentDom>,
    dom: DomHandle,
    node: NodeId,
}

impl NativeProperties {
    fn new() -> Self {
        let document = Rc::new(RefCell::new(Document::new()));
        let backend = Rc::new(DocumentDom::new(document));
        let dom = DomHandle::from_rc(backend.clone());
        let root = dom.create_element(ElementName::new("column"));
        dom.insert(backend.root_node(), root, None);
        let mut engine = JsRuntime::new(RuntimeLimits::default()).unwrap();
        let bridge = UiBridge::new(NodeRegistry::new(dom.clone(), root, 37, 1, 100), HostHandle::new(StubHost::default()));
        engine.with_context(|ctx| {
            bridge.install(&ctx)?;
            ctx.globals().set("root", bridge.root(&ctx)?)
        }).unwrap();
        bridge.run_turn(|| engine.evaluate_script(r#"
            const node = __zgui.createElement('control');
            __zgui.insertNode(root, node, null);
            'mounted'
        "#)).unwrap();
        let node = backend.children(root)[0];
        Self { bridge, engine, backend, dom, node }
    }

    fn script(&mut self, script: &str) {
        self.bridge.run_turn(|| self.engine.evaluate_script(script)).unwrap();
    }

    fn attribute(&self, name: &str) -> Option<String> {
        self.backend.attribute(self.node, AttrName::new(name))
    }

    // Read the exact declaration block the native cascade consumes, not a stub's
    // separate style/custom maps (which would hide representation-clearing bugs).
    fn style(&self) -> (BTreeSet<String>, String) {
        let document = self.backend.document();
        let key = document.store().key_of(self.backend.index_of(self.node));
        let guard = document.store().lock().read();
        let Some(block) = document.store().columns().inline_style.get(key).and_then(Option::as_ref) else {
            return (BTreeSet::new(), String::new());
        };
        let block = block.read_with(&guard);
        let names = block.declarations().iter().map(|declaration| declaration.id().name().into_owned()).collect();
        let mut css = String::new();
        block.to_css(&mut css).unwrap();
        (names, css)
    }
}

#[test]
fn classes_are_replaced_deduplicated_and_split_on_native_ascii_whitespace() {
    let mut native = NativeProperties::new();
    native.script("__zgui.setProperty(node, 'class', '  alpha\\tbeta\\nalpha  gamma\\u00a0delta '); 'set'");
    let classes: Vec<_> = native.backend.classes(native.node).iter().map(|class| class.as_str()).collect();
    assert_eq!(classes, ["alpha", "beta", "gamma\u{a0}delta"]);
    native.script("__zgui.setProperty(node, 'class', 'replacement'); 'replace'");
    assert_eq!(native.backend.classes(native.node), [zgui::view::ClassName::new("replacement")]);
    for empty in ["null", "undefined"] {
        native.script(&format!("__zgui.setProperty(node, 'class', 'old'); __zgui.setProperty(node, 'class', {empty}); 'clear'"));
        assert!(native.backend.classes(native.node).is_empty());
    }
}

#[test]
fn attributes_use_typed_native_names_and_both_empty_values_remove_them() {
    let mut native = NativeProperties::new();
    for empty in ["null", "undefined"] {
        native.script("__zgui.setProperty(node, 'id', 'save'); __zgui.setProperty(node, 'title', 'Save changes'); __zgui.setProperty(node, 'tabIndex', -1); 'set'");
        assert_eq!(native.attribute("id").as_deref(), Some("save"));
        assert_eq!(native.attribute("title").as_deref(), Some("Save changes"));
        assert_eq!(native.attribute("tabindex").as_deref(), Some("-1"));
        assert_eq!(native.attribute("tabIndex"), None);
        native.script(&format!("for (const name of ['id', 'title', 'tabIndex']) __zgui.setProperty(node, name, {empty}); 'clear'"));
        for name in ["id", "title", "tabindex"] { assert_eq!(native.attribute(name), None); }
    }
    native.script("__zgui.setProperty(node, 'id', ''); __zgui.setProperty(node, 'tabIndex', 0); 'set'");
    assert_eq!(native.attribute("id").as_deref(), Some(""));
    assert_eq!(native.attribute("tabindex").as_deref(), Some("0"));
}

#[test]
fn test_ids_follow_native_ownership_without_changing_dom_attributes() {
    let mut native = NativeProperties::new();
    let revision = native.backend.revision();
    native.script("__zgui.setProperty(node, 'testId', 'save'); 'set'");
    let first = native.bridge.snapshot(10);
    let token = first.nodes[1].token;
    assert_eq!(first.nodes[1].test_id.as_deref(), Some("save"));
    assert_eq!(native.backend.revision(), revision);
    assert_eq!(native.attribute("testId"), None);
    native.script("__zgui.setProperty(node, 'testId', 'save'); 'unchanged'");
    assert_eq!(native.bridge.metadata_revision(), first.metadata_revision);
    native.script("__zgui.setProperty(node, 'testId', 'renamed'); 'changed'");
    assert!(native.bridge.metadata_revision() > first.metadata_revision);
    assert_eq!(first.nodes[1].test_id.as_deref(), Some("save"), "saved evidence owns the old value");
    for empty in ["null", "undefined"] {
        native.script(&format!("__zgui.setProperty(node, 'testId', 'set'); __zgui.setProperty(node, 'testId', {empty}); 'clear'"));
        assert_eq!(native.bridge.snapshot(10).nodes[1].test_id, None);
    }
    native.script(r#"
        __zgui.setProperty(node, 'testId', 'save');
        try { __zgui.setProperty(node, 'testId', '界'.repeat(171)); throw Error('accepted oversized testId'); }
        catch (error) { if (!String(error).includes('512 UTF-8 bytes')) throw error; }
        __zgui.removeNode(root, node);
        'removed'
    "#);
    assert_eq!(native.bridge.snapshot(10).nodes.len(), 1);
    native.script(r#"
        const replacement = __zgui.createElement('control');
        __zgui.setProperty(replacement, 'testId', 'save');
        __zgui.insertNode(root, replacement, null);
        'replacement'
    "#);
    let replacement = native.bridge.snapshot(10);
    assert_ne!(replacement.nodes[1].token, token);
    assert_eq!(replacement.nodes[1].test_id.as_deref(), Some("save"));
}

#[test]
fn disabled_native_state_and_accessibility_stay_in_sync_without_erasing_other_state() {
    let mut native = NativeProperties::new();
    native.dom.set_ui_state(native.node, UiState::HOVER, true);
    for empty in ["false", "null", "undefined"] {
        native.script(r#"
            __zgui.setProperty(node, 'role', 'button');
            __zgui.setProperty(node, 'ariaLabel', 'Save');
            __zgui.setProperty(node, 'ariaDescription', 'Writes changes');
            __zgui.setProperty(node, 'disabled', true);
            'disabled'
        "#);
        assert!(native.backend.ui_state(native.node).contains(UiState::DISABLED));
        let semantics = native.backend.semantics(native.node).unwrap();
        assert_eq!(semantics.role, Role::Button);
        assert_eq!(semantics.label.as_deref(), Some("Save"));
        assert_eq!(semantics.description.as_deref(), Some("Writes changes"));
        assert!(semantics.flags.contains(SemanticFlags::DISABLED));
        native.script(&format!("__zgui.setProperty(node, 'disabled', {empty}); 'enabled'"));
        assert!(!native.backend.ui_state(native.node).contains(UiState::DISABLED));
        assert!(native.backend.ui_state(native.node).contains(UiState::HOVER));
        let semantics = native.backend.semantics(native.node).unwrap();
        assert!(!semantics.flags.contains(SemanticFlags::DISABLED));
        assert_eq!(semantics.role, Role::Button);
        assert_eq!(semantics.label.as_deref(), Some("Save"));
    }
}

#[test]
fn semantic_fields_clear_independently_and_release_the_final_native_record() {
    let mut native = NativeProperties::new();
    for empty in ["null", "undefined"] {
        native.script("__zgui.setProperty(node, 'role', 'group'); __zgui.setProperty(node, 'ariaLabel', 'Actions'); __zgui.setProperty(node, 'ariaDescription', 'Description'); 'set'");
        native.script(&format!("__zgui.setProperty(node, 'role', {empty}); __zgui.setProperty(node, 'ariaLabel', {empty}); 'clear some'"));
        let semantics = native.backend.semantics(native.node).unwrap();
        assert_eq!(semantics.role, Role::GenericContainer);
        assert_eq!(semantics.label, None);
        assert_eq!(semantics.description.as_deref(), Some("Description"));
        native.script(&format!("__zgui.setProperty(node, 'ariaDescription', {empty}); 'clear all'"));
        assert_eq!(native.backend.semantics(native.node), None);
        native.script("__zgui.setProperty(node, 'disabled', true); 'disabled only'");
        assert!(native.backend.semantics(native.node).unwrap().flags.contains(SemanticFlags::DISABLED));
        native.script(&format!("__zgui.setProperty(node, 'disabled', {empty}); 'clear disabled'"));
        assert_eq!(native.backend.semantics(native.node), None);
    }
}

#[test]
fn object_updates_remove_disappeared_keys_and_preserve_css_enumeration_order() {
    let mut native = NativeProperties::new();
    native.script("__zgui.setProperty(node, 'style', {width: '10px', height: '20px', '--Accent': '7px'}); 'initial'");
    assert_eq!(native.style().0, BTreeSet::from(["width".into(), "height".into(), "--Accent".into()]));
    assert!(native.style().1.contains("--Accent: 7px"));
    native.script("__zgui.setProperty(node, 'style', {height: '30px'}); 'update'");
    assert_eq!(native.style().0, BTreeSet::from(["height".into()]));
    assert!(native.style().1.contains("height: 30px"));

    // A shorthand removal erases its longhands in the actual native block.
    // An unchanged longhand must therefore be restored, not skipped by a value diff.
    native.script("__zgui.setProperty(node, 'style', {padding: '1px', 'padding-left': '8px'}); 'shorthand'");
    native.script("__zgui.setProperty(node, 'style', {'padding-left': '8px'}); 'remove shorthand'");
    assert_eq!(native.style().0, BTreeSet::from(["padding-left".into()]));
    assert!(native.style().1.contains("padding-left: 8px"));
    native.script("__zgui.setProperty(node, 'style', {'padding-left': '8px', padding: '2px'}); 'ordered'");
    assert!(native.style().1.contains("padding: 2px"), "{}", native.style().1);
    native.script("__zgui.setProperty(node, 'style', {}); 'clear object'");
    assert!(native.style().0.is_empty());
}

#[test]
fn style_string_object_transitions_and_clearing_remove_custom_declarations_too() {
    let mut native = NativeProperties::new();
    native.script("__zgui.setProperty(node, 'style', {width: '10px', '--object': '7px'}); __zgui.setProperty(node, '--direct', '9px'); 'object'");
    assert!(native.style().0.contains("--direct"));
    assert!(!native.style().0.contains("----direct"));
    native.script("__zgui.setProperty(node, 'style', 'height: 20px; --text: 3px'); 'text'");
    assert_eq!(native.style().0, BTreeSet::from(["height".into(), "--text".into()]));
    native.script("__zgui.setProperty(node, 'style', {width: '30px', '--next': '5px'}); 'object again'");
    assert_eq!(native.style().0, BTreeSet::from(["width".into(), "--next".into()]));
    for empty in ["null", "undefined"] {
        native.script(&format!("__zgui.setProperty(node, 'style', {{width: '1px', '--object': '2px'}}); __zgui.setProperty(node, '--direct', '3px'); __zgui.setProperty(node, 'style', {empty}); 'clear'"));
        assert!(native.style().0.is_empty());
    }
}

#[test]
fn direct_and_object_custom_properties_share_one_case_sensitive_native_block() {
    let mut native = NativeProperties::new();
    native.script("__zgui.setProperty(node, 'style', {'--Accent': '1px', '--accent': '2px'}); __zgui.setProperty(node, '--Accent', '3px'); 'direct'");
    assert_eq!(native.style().0, BTreeSet::from(["--Accent".into(), "--accent".into()]));
    assert!(native.style().1.contains("--Accent: 3px"));
    // Reassigning the same style object must not trust a cache invalidated by a
    // direct custom-property write, and null must clear even an untracked key.
    native.script("__zgui.setProperty(node, 'style', {'--Accent': '1px', '--accent': '2px'}); 'restore'");
    assert!(native.style().1.contains("--Accent: 1px"));
    native.script("__zgui.setProperty(node, '--direct', '9px'); __zgui.setProperty(node, 'style', {'--Accent': null, '--accent': undefined, '--direct': null}); 'clear'");
    assert!(native.style().0.is_empty());
    for empty in ["null", "undefined"] {
        native.script(&format!("__zgui.setProperty(node, '--direct', '4px'); __zgui.setProperty(node, '--direct', {empty}); 'clear direct'"));
        assert!(native.style().0.is_empty());
    }
    // Reapplying equal text replaces direct declarations too: all native writes
    // share one block, not separate string/object/custom property stores.
    native.script("__zgui.setProperty(node, 'style', 'width: 10px'); __zgui.setProperty(node, '--direct', '4px'); __zgui.setProperty(node, 'style', 'width: 10px'); 'same text'");
    assert_eq!(native.style().0, BTreeSet::from(["width".into()]));
}

#[test]
fn style_getters_can_reenter_the_bridge_and_native_mutation_waits_for_complete_parsing() {
    let mut native = NativeProperties::new();
    native.script(r#"
        __zgui.setProperty(node, 'style', {height: '11px'});
        __zgui.setProperty(node, 'style', {
            get width() {
                __zgui.setProperty(node, 'title', 'getter reentered');
                __zgui.setProperty(node, 'style', {height: '12px', '--nested': '7px'});
                const text = __zgui.createTextNode('getter child');
                __zgui.insertNode(node, text, null);
                return '30px';
            }
        });
        'reentered'
    "#);
    assert_eq!(native.attribute("title").as_deref(), Some("getter reentered"));
    assert_eq!(native.backend.text_content(native.node), "getter child");
    assert_eq!(native.style().0, BTreeSet::from(["width".into()]));
    assert!(native.style().1.contains("width: 30px"));
    native.script(r#"
        let rejected = false;
        try {
            __zgui.setProperty(node, 'style', {
                height: '99px', get width() { throw new Error('getter failed'); }
            });
        } catch (error) { rejected = error.message === 'getter failed'; }
        if (!rejected) throw Error('getter exception not preserved');
        'caught'
    "#);
    assert_eq!(native.style().0, BTreeSet::from(["width".into()]));
    assert!(native.style().1.contains("width: 30px"));
}

#[test]
fn unsupported_properties_and_wrong_types_throw_without_partial_native_writes() {
    let mut native = NativeProperties::new();
    native.script(r#"
        __zgui.setProperty(node, 'style', {height: '11px'});
        __zgui.setProperty(node, 'disabled', true);
        const invalid = [
            ['className', 'bad'], ['unknown', null], ['children', 'bad'], ['ref', () => {}],
            ['value', 'not editable'], ['scrollTop', 12], ['d', 'M0 0 L1 1'],
            ['class', 3], ['id', false], ['title', {}], ['ariaLabel', 1], ['ariaDescription', true],
            ['disabled', 'false'], ['disabled', 0], ['role', 'link'], ['role', 1],
            ['tabIndex', '0'], ['tabIndex', false], ['tabIndex', 0.5],
            ['tabIndex', Infinity], ['tabIndex', NaN], ['tabIndex', 2147483648], ['tabIndex', -2147483649],
            ['style', 12], ['style', false], ['style', []], ['style', () => {}],
            ['style', {width: 12}], ['style', {width: true}], ['style', {width: {}}],
            ['style', {width: '3px', height: 12}], ['style', {colour: 'red'}],
            ['style', {'backgroundColor': 'red'}], ['style', {'--': 'red'}],
            ['--', 'red'], ['--accent', 12], ['--accent', false]
        ];
        for (const [name, value] of invalid) {
            let rejected = false;
            try { __zgui.setProperty(node, name, value); } catch { rejected = true; }
            if (!rejected) throw Error('accepted invalid ' + name + ': ' + String(value));
        }
        'rejected'
    "#);
    assert_eq!(native.style().0, BTreeSet::from(["height".into()]));
    assert!(native.style().1.contains("height: 11px"));
    assert!(native.backend.ui_state(native.node).contains(UiState::DISABLED));
    assert_eq!(native.attribute("id"), None);
    assert_eq!(native.attribute("title"), None);
}

#[test]
fn documented_object_style_vocabulary_is_accepted_by_the_pinned_native_parser() {
    let mut native = NativeProperties::new();
    let declarations = [
        ("display", "flex"), ("flex-direction", "column"), ("flex-wrap", "wrap"),
        ("flex-grow", "1"), ("flex-shrink", "0"), ("flex-basis", "20px"),
        ("align-items", "center"), ("justify-content", "space-between"),
        ("gap", "2px"), ("row-gap", "3px"), ("column-gap", "4px"),
        ("padding", "1px"), ("padding-top", "1px"), ("padding-right", "2px"),
        ("padding-bottom", "3px"), ("padding-left", "4px"),
        ("margin", "1px"), ("margin-top", "1px"), ("margin-right", "2px"),
        ("margin-bottom", "3px"), ("margin-left", "4px"),
        ("width", "10px"), ("height", "11px"), ("min-width", "2px"),
        ("min-height", "3px"), ("max-width", "20px"), ("max-height", "30px"),
        ("color", "red"), ("background", "blue"), ("background-color", "green"),
        ("border", "1px solid red"), ("border-color", "red"), ("border-width", "1px"),
        ("border-radius", "2px"), ("font-family", "sans-serif"), ("font-size", "16px"),
        ("font-weight", "700"), ("line-height", "1.5"), ("opacity", "0.5"), ("overflow", "hidden"),
    ];
    for (name, value) in declarations {
        native.script(&format!("__zgui.setProperty(node, 'style', {{'{name}': '{value}'}}); 'set'"));
        assert!(!native.style().0.is_empty(), "native parser did not accept {name}: {value}");
        native.script("__zgui.setProperty(node, 'style', null); 'clear'");
        assert!(native.style().0.is_empty(), "native declarations remained after clearing {name}");
    }
}

#[test]
fn css_strings_deliberately_keep_native_error_recovery() {
    let mut native = NativeProperties::new();
    native.script("__zgui.setProperty(node, 'style', 'unknown-property: nope; width: 10px; height: invalid; --ok: 3px'); 'parse'");
    assert_eq!(native.style().0, BTreeSet::from(["width".into(), "--ok".into()]));
}
