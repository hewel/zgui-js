use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

use js_runtime::{JsRuntime, RuntimeLimits};
use ui_bridge::{UiBridge, registry::NodeRegistry};
use zgui::{view::{AttrName, DomHandle, ElementName, HostHandle, NodeId, PropKey, UiState, stub::StubHost}, vocab::{Invalid, Orientation, Role, SemanticFlags, Toggled}};
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
            ['disabled', 'false'], ['disabled', 0], ['role', 'paragraph'], ['role', 1],
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
        ("display", "flex"), ("position", "fixed"), ("inset", "0px"), ("top", "1px"),
        ("right", "2px"), ("bottom", "3px"), ("left", "4px"), ("visibility", "hidden"),
        ("flex-direction", "column"), ("flex-wrap", "wrap"),
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

#[test]
fn image_sources_use_real_native_attributes_and_cannot_leak_to_other_elements() {
    let mut native = NativeProperties::new();
    let heard = Rc::new(RefCell::new(Vec::new()));
    let changes = Rc::clone(&heard);
    native.backend.set_attribute_hook(Rc::new(move |_, name, value| {
        if name.as_str() == "src" { changes.borrow_mut().push(value.map(str::to_owned)); }
    }));
    native.script(r#"
        const image = __zgui.createElement('image');
        __zgui.insertNode(root, image, null);
        __zgui.setProperty(image, 'src', 'assets/first.png');
        __zgui.setProperty(image, 'src', 'assets/second.png');
        __zgui.setProperty(image, 'src', null);
        __zgui.setProperty(image, 'src', 'assets/third.png');
        __zgui.setProperty(image, 'src', undefined);
        for (const target of [root, node, __zgui.createTextNode('text')]) {
            let rejected = false;
            try { __zgui.setProperty(target, 'src', 'ambient.png'); }
            catch (error) { rejected = String(error).includes('src requires an image'); }
            if (!rejected) throw Error('src accepted on non-image');
        }
        for (const invalid of [3, {}, false, 'x'.repeat(4097)]) {
            let rejected = false;
            try { __zgui.setProperty(image, 'src', invalid); } catch { rejected = true; }
            if (!rejected) throw Error('invalid src accepted');
        }
        let childrenRejected = false;
        try { __zgui.insertNode(image, __zgui.createTextNode('forbidden'), null); }
        catch (error) { childrenRejected = String(error).includes('image elements cannot contain'); }
        if (!childrenRejected) throw Error('image accepted children');
        'sources'
    "#);
    assert_eq!(&*heard.borrow(), &[
        Some("assets/first.png".into()), Some("assets/second.png".into()), None,
        Some("assets/third.png".into()), None,
    ]);
    assert_eq!(native.bridge.snapshot(10).nodes[2].kind, "image");
    assert_eq!(native.attribute("src"), None);
    native.bridge.begin_stop();
    native.script(r#"
        let rejected = false;
        try { __zgui.setProperty(image, 'src', 'assets/late.png'); }
        catch (error) { rejected = error instanceof ReferenceError; }
        if (!rejected) throw Error('retiring owner allocated image work');
        __zgui.setProperty(image, 'src', null);
        'retired'
    "#);
}

#[test]
fn data_attributes_use_native_attributes_and_reject_invalid_names_without_writes() {
    let mut native = NativeProperties::new();
    native.script("__zgui.setProperty(node, 'data-action', 'save'); __zgui.setProperty(node, 'data-count-1_x', 'two'); 'set'");
    assert_eq!(native.attribute("data-action").as_deref(), Some("save"));
    assert_eq!(native.attribute("data-count-1_x").as_deref(), Some("two"));
    native.script("__zgui.setProperty(node, 'data-action', 'apply'); 'change'");
    assert_eq!(native.attribute("data-action").as_deref(), Some("apply"));
    for empty in ["null", "undefined"] {
        native.script(&format!("__zgui.setProperty(node, 'data-action', 'keep'); __zgui.setProperty(node, 'data-action', {empty}); 'clear'"));
        assert_eq!(native.attribute("data-action"), None);
    }
    native.script("__zgui.setProperty(node, 'data-action', ''); 'empty'");
    assert_eq!(native.attribute("data-action").as_deref(), Some(""));
    native.script(r#"
        __zgui.setProperty(node, 'data-action', 'preserved');
        const invalid = [
            ['data-', 'value'], ['data-Upper', 'value'], ['data-sp ace', 'value'], ['data-é', 'value'],
            ['data-action', 3], ['data-action', false], ['data-action', {}],
        ];
        for (const [name, value] of invalid) {
            let rejected = false;
            try { __zgui.setProperty(node, name, value); } catch { rejected = true; }
            if (!rejected) throw Error('accepted invalid ' + name + ': ' + String(value));
        }
        'rejected'
    "#);
    assert_eq!(native.attribute("data-action").as_deref(), Some("preserved"));
    assert_eq!(native.attribute("data-"), None);
    assert_eq!(native.attribute("data-Upper"), None);
}

#[test]
fn tri_state_and_flag_properties_sync_native_state_with_semantics() {
    let mut native = NativeProperties::new();
    native.dom.set_ui_state(native.node, UiState::HOVER, true);
    native.script("__zgui.setProperty(node, 'role', 'checkBox'); __zgui.setProperty(node, 'ariaLabel', 'Select all'); 'labeled'");
    let assert_preserved = |native: &NativeProperties| {
        assert!(native.backend.ui_state(native.node).contains(UiState::HOVER));
        let semantics = native.backend.semantics(native.node).unwrap();
        assert_eq!(semantics.role, Role::CheckBox);
        assert_eq!(semantics.label.as_deref(), Some("Select all"));
        semantics
    };

    native.script("__zgui.setProperty(node, 'checked', 'mixed'); 'mixed'");
    let state = native.backend.ui_state(native.node);
    assert!(state.contains(UiState::INDETERMINATE));
    assert!(!state.contains(UiState::CHECKED));
    assert_eq!(assert_preserved(&native).toggled, Some(Toggled::Mixed));

    native.script("__zgui.setProperty(node, 'checked', true); 'true'");
    let state = native.backend.ui_state(native.node);
    assert!(state.contains(UiState::CHECKED));
    assert!(!state.contains(UiState::INDETERMINATE));
    assert_eq!(assert_preserved(&native).toggled, Some(Toggled::True));

    native.script("__zgui.setProperty(node, 'checked', false); 'false'");
    let state = native.backend.ui_state(native.node);
    assert!(!state.contains(UiState::CHECKED));
    assert!(!state.contains(UiState::INDETERMINATE));
    assert_eq!(assert_preserved(&native).toggled, Some(Toggled::False));

    for empty in ["null", "undefined"] {
        native.script("__zgui.setProperty(node, 'checked', 'mixed'); 'mixed again'");
        native.script(&format!("__zgui.setProperty(node, 'checked', {empty}); 'clear'"));
        let state = native.backend.ui_state(native.node);
        assert!(!state.contains(UiState::CHECKED));
        assert!(!state.contains(UiState::INDETERMINATE));
        assert_eq!(assert_preserved(&native).toggled, None);
    }

    native.script("__zgui.setProperty(node, 'expanded', false); 'collapsed'");
    assert_eq!(assert_preserved(&native).expanded, Some(false));
    native.script("__zgui.setProperty(node, 'expanded', true); 'expanded'");
    assert_eq!(assert_preserved(&native).expanded, Some(true));
    native.script("__zgui.setProperty(node, 'selected', false); 'unselected'");
    assert_eq!(assert_preserved(&native).selected, Some(false));
    for empty in ["null", "undefined"] {
        native.script(&format!("__zgui.setProperty(node, 'expanded', {empty}); __zgui.setProperty(node, 'selected', {empty}); 'clear optionals'"));
        let semantics = assert_preserved(&native);
        assert_eq!(semantics.expanded, None);
        assert_eq!(semantics.selected, None);
    }

    native.script("__zgui.setProperty(node, 'required', true); __zgui.setProperty(node, 'invalid', true); __zgui.setProperty(node, 'ariaHidden', true); __zgui.setProperty(node, 'ariaModal', true); 'flags'");
    let state = native.backend.ui_state(native.node);
    assert!(state.contains(UiState::REQUIRED));
    assert!(state.contains(UiState::INVALID));
    let semantics = assert_preserved(&native);
    assert!(semantics.flags.contains(SemanticFlags::REQUIRED));
    assert!(semantics.flags.contains(SemanticFlags::HIDDEN));
    assert!(semantics.flags.contains(SemanticFlags::MODAL));
    assert_eq!(semantics.invalid, Some(Invalid::True));

    native.script("__zgui.setProperty(node, 'ariaHidden', false); 'revealed'");
    let semantics = assert_preserved(&native);
    assert!(!semantics.flags.contains(SemanticFlags::HIDDEN));
    assert!(semantics.flags.contains(SemanticFlags::MODAL));
    assert!(semantics.flags.contains(SemanticFlags::REQUIRED));
    native.script("__zgui.setProperty(node, 'invalid', null); 'valid'");
    assert!(!native.backend.ui_state(native.node).contains(UiState::INVALID));
    assert_eq!(assert_preserved(&native).invalid, None);
    assert!(native.backend.ui_state(native.node).contains(UiState::REQUIRED));
    native.script("__zgui.setProperty(node, 'required', undefined); __zgui.setProperty(node, 'ariaModal', false); 'cleared'");
    let state = native.backend.ui_state(native.node);
    assert!(!state.contains(UiState::REQUIRED));
    assert!(state.contains(UiState::HOVER));
    let semantics = assert_preserved(&native);
    assert!(!semantics.flags.contains(SemanticFlags::REQUIRED));
    assert!(!semantics.flags.contains(SemanticFlags::MODAL));

    native.script("__zgui.setProperty(node, 'orientation', 'horizontal'); 'horizontal'");
    assert_eq!(assert_preserved(&native).orientation, Some(Orientation::Horizontal));
    native.script("__zgui.setProperty(node, 'orientation', 'vertical'); 'vertical'");
    assert_eq!(assert_preserved(&native).orientation, Some(Orientation::Vertical));
    native.script("__zgui.setProperty(node, 'orientation', null); 'clear'");
    assert_eq!(assert_preserved(&native).orientation, None);

    native.script("__zgui.setProperty(node, 'open', true); 'open'");
    assert!(native.backend.ui_state(native.node).contains(UiState::OPEN));
    assert_eq!(assert_preserved(&native).expanded, None);
    native.script("__zgui.setProperty(node, 'open', false); 'closed'");
    assert!(!native.backend.ui_state(native.node).contains(UiState::OPEN));

    native.script(r#"
        const invalid = [
            ['checked', 'yes'], ['checked', 1], ['checked', {}], ['expanded', 'yes'], ['selected', 0],
            ['open', 'open'], ['invalid', 'true'], ['required', 1], ['ariaHidden', 'yes'], ['ariaModal', 0],
            ['orientation', 'diagonal'], ['orientation', true], ['role', 'paragraph'], ['role', 'checkbox'],
        ];
        for (const [name, value] of invalid) {
            let rejected = false;
            try { __zgui.setProperty(node, name, value); } catch { rejected = true; }
            if (!rejected) throw Error('accepted invalid ' + name + ': ' + String(value));
        }
        'rejected'
    "#);
    assert_preserved(&native);
}

#[test]
fn vectors_write_drawing_properties_through_the_imperative_pathway() {
    let mut native = NativeProperties::new();
    native.script(r#"
        const vector = __zgui.createElement('vector');
        __zgui.insertNode(root, vector, null);
        __zgui.setProperty(vector, 'd', 'M0 0 L8 8');
        __zgui.setProperty(vector, 'viewBox', '0 0 16 16');
        'drawn'
    "#);
    let parent = native.dom.parent(native.node).unwrap();
    let vector = native.backend.children(parent)[1];
    assert_eq!(native.backend.property(vector, PropKey::new("d")).as_str(), Some("M0 0 L8 8"));
    assert_eq!(native.backend.property(vector, PropKey::new("viewBox")).as_str(), Some("0 0 16 16"));
    assert_eq!(native.backend.attribute(vector, AttrName::new("d")), None);
    assert_eq!(native.backend.attribute(vector, AttrName::new("viewBox")), None);
    assert_eq!(native.bridge.snapshot(10).nodes[2].kind, "vector");
    native.script(r#"
        for (const target of [root, node]) {
            let rejected = false;
            try { __zgui.setProperty(target, 'd', 'M1 1'); }
            catch (error) { rejected = String(error).includes('require a vector'); }
            if (!rejected) throw Error('d accepted on a non-vector');
            rejected = false;
            try { __zgui.setProperty(target, 'viewBox', '0 0 1 1'); }
            catch (error) { rejected = String(error).includes('require a vector'); }
            if (!rejected) throw Error('viewBox accepted on a non-vector');
        }
        for (const invalid of [3, false, {}]) {
            let rejected = false;
            try { __zgui.setProperty(vector, 'd', invalid); } catch { rejected = true; }
            if (!rejected) throw Error('invalid d accepted');
        }
        let rejected = false;
        try { __zgui.insertNode(vector, __zgui.createTextNode('forbidden'), null); }
        catch (error) { rejected = String(error).includes('vector elements cannot contain'); }
        if (!rejected) throw Error('vector accepted children');
        'rejected'
    "#);
    assert_eq!(native.backend.property(vector, PropKey::new("d")).as_str(), Some("M0 0 L8 8"));
    native.script("__zgui.setProperty(vector, 'd', null); __zgui.setProperty(vector, 'viewBox', undefined); 'cleared'");
    assert!(native.backend.property(vector, PropKey::new("d")).is_unset());
    assert!(native.backend.property(vector, PropKey::new("viewBox")).is_unset());
    native.script("__zgui.removeNode(root, vector); 'removed'");
    assert_eq!(native.bridge.snapshot(10).nodes.len(), 2);
    native.backend.end_frame();
    assert!(!native.dom.is_live(vector));
}

#[test]
fn relations_resolve_real_accessibility_ids_and_reject_foreign_or_fabricated_targets() {
    let mut native = NativeProperties::new();
    native.script(r#"
        const label = __zgui.createElement('label');
        const owned = __zgui.createElement('box');
        __zgui.insertNode(root, label, null);
        __zgui.insertNode(root, owned, null);
        __zgui.setProperty(node, 'role', 'button');
        __zgui.setProperty(node, 'ariaLabel', 'Control');
        __zgui.setProperty(node, 'ariaLabelledBy', label);
        __zgui.setProperty(node, 'ariaOwns', [owned, label]);
        'related'
    "#);
    let children = native.backend.children(native.dom.parent(native.node).expect("mounted"));
    let (label, owned) = (children[1], children[2]);
    let semantics = native.backend.semantics(native.node).unwrap();
    assert_eq!(semantics.relations.labelled_by, [zgui::vocab::NodeId(label.as_u64())]);
    assert_eq!(semantics.relations.owns, [zgui::vocab::NodeId(owned.as_u64()), zgui::vocab::NodeId(label.as_u64())]);

    native.script(r#"
        __zgui.setProperty(node, 'ariaDescribedBy', [label]);
        __zgui.setProperty(node, 'ariaControls', label);
        __zgui.setProperty(node, 'ariaRadioGroup', owned);
        __zgui.setProperty(node, 'ariaLabelledBy', null);
        'replaced'
    "#);
    let semantics = native.backend.semantics(native.node).unwrap();
    assert!(semantics.relations.labelled_by.is_empty());
    assert_eq!(semantics.relations.described_by, [zgui::vocab::NodeId(label.as_u64())]);
    assert_eq!(semantics.relations.controls, [zgui::vocab::NodeId(label.as_u64())]);
    assert_eq!(semantics.relations.radio_group, [zgui::vocab::NodeId(owned.as_u64())]);
    native.script(r#"
        __zgui.setProperty(node, 'ariaRadioGroup', null);
        'cleared'
    "#);
    assert!(native.backend.semantics(native.node).unwrap().relations.radio_group.is_empty());

    native.script(r#"
        const before = ['ariaLabelledBy','ariaDescribedBy','ariaControls','ariaOwns','ariaRadioGroup'];
        for (const name of before) {
            for (const bad of [7, 'id', { length: 1 }, [7], [null], true, () => {}]) {
                let rejected = false;
                try { __zgui.setProperty(node, name, bad); } catch (error) { rejected = error instanceof TypeError; }
                if (!rejected) throw Error(`${name} accepted ${bad}`);
            }
        }
        __zgui.removeNode(root, owned);
        'rejected'
    "#);
    let foreign = UiBridge::new(
        NodeRegistry::new(native.dom.clone(), root_handle(&native), 38, 1, 100),
        HostHandle::new(StubHost::default()),
    );
    native.engine.with_context(|ctx| {
        ctx.globals().set("foreign", foreign.root(&ctx)?)
    }).unwrap();
    native.script(r#"
        let foreignRejected = false;
        try { __zgui.setProperty(node, 'ariaOwns', [label, foreign]); }
        catch (error) { foreignRejected = error instanceof TypeError; }
        if (!foreignRejected) throw Error('relation accepted a foreign-session target');
        'foreign'
    "#);
    foreign.dispose();
    native.backend.end_frame();
    native.script(r#"
        let stale = false;
        try { __zgui.setProperty(node, 'ariaOwns', [label, owned]); }
        catch (error) { stale = error instanceof TypeError && error.message.includes('stale'); }
        if (!stale) throw Error('relation accepted a stale target');
        'stale'
    "#);
    let semantics = native.backend.semantics(native.node).unwrap();
    assert_eq!(semantics.relations.owns, [zgui::vocab::NodeId(owned.as_u64()), zgui::vocab::NodeId(label.as_u64())]);
    assert_eq!(semantics.relations.described_by, [zgui::vocab::NodeId(label.as_u64())]);
    assert_eq!(semantics.role, Role::Button);
    assert_eq!(semantics.label.as_deref(), Some("Control"));
}

fn root_handle(native: &NativeProperties) -> NodeId {
    native.dom.parent(native.node).expect("mounted")
}

#[test]
fn relations_ignore_mutable_array_globals_and_bound_indexed_getters() {
    let mut native = NativeProperties::new();
    native.script(r#"
        const label = __zgui.createElement('label');
        __zgui.insertNode(root, label, null);
        __zgui.setProperty(node, 'ariaControls', [label]);
        'baseline'
    "#);
    native.script(r#"
        const builtin = Array.isArray;
        try {
            Array.isArray = () => true;
            let rejected = false;
            try { __zgui.setProperty(node, 'ariaControls', { 0: label, length: 1 }); }
            catch (error) { rejected = error instanceof TypeError; }
            if (!rejected) throw Error('forged array-like accepted');
        } finally { Array.isArray = builtin; }
        try {
            Array.isArray = () => false;
            __zgui.setProperty(node, 'ariaControls', [label]);
        } finally { Array.isArray = builtin; }
        const items = [label];
        Object.defineProperty(items, 0, {
            get() { items.length = 0xffffffff; return label; },
        });
        __zgui.setProperty(node, 'ariaControls', items);
        'done'
    "#);
    let children = native.backend.children(native.dom.parent(native.node).expect("mounted"));
    let label = children[1];
    let semantics = native.backend.semantics(native.node).unwrap();
    assert_eq!(semantics.relations.controls, [zgui::vocab::NodeId(label.as_u64())]);
}

#[test]
fn relations_are_bounded_by_the_session_node_budget() {
    let mut native = NativeProperties::new();
    native.script(r#"
        const label = __zgui.createElement('label');
        __zgui.insertNode(root, label, null);
        __zgui.setProperty(node, 'ariaOwns', [label]);
        'related'
    "#);
    native.script(r#"
        let getterRan = false;
        const oversized = new Array(101);
        Object.defineProperty(oversized, 0, {
            get() { getterRan = true; return label; },
        });
        let rejected = false;
        try { __zgui.setProperty(node, 'ariaOwns', oversized); }
        catch (error) { rejected = error instanceof RangeError; }
        if (!rejected) throw Error('oversized relation array accepted');
        if (getterRan) throw Error('relation getter ran before the budget check');
        rejected = false;
        try { __zgui.setProperty(node, 'ariaOwns', new Array(0xffffffff)); }
        catch (error) { rejected = error instanceof RangeError; }
        if (!rejected) throw Error('huge relation array accepted');
        'bounded'
    "#);
    let children = native.backend.children(native.dom.parent(native.node).expect("mounted"));
    let label = children[1];
    let semantics = native.backend.semantics(native.node).unwrap();
    assert_eq!(semantics.relations.owns, [zgui::vocab::NodeId(label.as_u64())]);
}

#[test]
fn busy_live_and_numeric_semantics_apply_and_clear_independently() {
    let mut native = NativeProperties::new();
    native.script(r#"
        __zgui.setProperty(node, 'role', 'progressIndicator');
        __zgui.setProperty(node, 'ariaBusy', true);
        __zgui.setProperty(node, 'ariaLive', 'polite');
        __zgui.setProperty(node, 'ariaValueMin', 0);
        __zgui.setProperty(node, 'ariaValueMax', 100);
        __zgui.setProperty(node, 'ariaValueNow', 25.5);
        __zgui.setProperty(node, 'ariaValueStep', 0.5);
        'semantic'
    "#);
    let semantics = native.backend.semantics(native.node).unwrap();
    assert!(semantics.flags.contains(SemanticFlags::BUSY));
    assert_eq!(semantics.live, Some(zgui::vocab::Live::Polite));
    assert_eq!(semantics.numeric.min, Some(0.0));
    assert_eq!(semantics.numeric.max, Some(100.0));
    assert_eq!(semantics.numeric.value, Some(25.5));
    assert_eq!(semantics.numeric.step, Some(0.5));

    native.script(r#"
        __zgui.setProperty(node, 'ariaBusy', false);
        __zgui.setProperty(node, 'ariaLive', 'off');
        __zgui.setProperty(node, 'ariaValueNow', null);
        __zgui.setProperty(node, 'ariaValueMin', undefined);
        'cleared'
    "#);
    let semantics = native.backend.semantics(native.node).unwrap();
    assert!(!semantics.flags.contains(SemanticFlags::BUSY));
    assert_eq!(semantics.live, Some(zgui::vocab::Live::Off));
    assert_eq!(semantics.numeric.value, None);
    assert_eq!(semantics.numeric.min, None);
    assert_eq!(semantics.numeric.max, Some(100.0));
    assert_eq!(semantics.numeric.step, Some(0.5));
    assert_eq!(semantics.role, Role::ProgressIndicator);

    native.script(r#"
        for (const [name, bad] of [['ariaLive', 'loud'], ['ariaValueNow', '50'], ['ariaValueMin', NaN], ['ariaValueMax', Infinity], ['ariaValueStep', true]]) {
            let rejected = false;
            try { __zgui.setProperty(node, name, bad); } catch (error) { rejected = error instanceof TypeError; }
            if (!rejected) throw Error(`${name} accepted ${bad}`);
        }
        'strict'
    "#);
    let semantics = native.backend.semantics(native.node).unwrap();
    assert_eq!(semantics.live, Some(zgui::vocab::Live::Off));
    assert_eq!(semantics.numeric.max, Some(100.0));
}

#[test]
fn heading_levels_apply_clear_and_reject_invalid_values_atomically() {
    let mut native = NativeProperties::new();
    native.script(r#"__zgui.setProperty(node, 'role', 'heading'); __zgui.setProperty(node, 'ariaLevel', 3); 'set'"#);
    assert_eq!(native.backend.semantics(native.node).unwrap().position.level, Some(3));
    native.script(r#"
      for (const value of [0, -1, 1.5, NaN, Infinity, 4294967296]) {
        let rejected = false;
        try { __zgui.setProperty(node, 'ariaLevel', value); } catch (e) { rejected = e instanceof RangeError; }
        if (!rejected) throw Error('invalid heading level accepted');
      }
      for (const value of ['3', true, {}]) {
        let rejected = false;
        try { __zgui.setProperty(node, 'ariaLevel', value); } catch (e) { rejected = e instanceof TypeError; }
        if (!rejected) throw Error('heading level was coerced');
      }
      'rejected'
    "#);
    assert_eq!(native.backend.semantics(native.node).unwrap().position.level, Some(3));
    native.script(r#"__zgui.setProperty(node, 'ariaLevel', 4294967295); 'max'"#);
    assert_eq!(native.backend.semantics(native.node).unwrap().position.level, Some(u32::MAX as usize));
    native.script(r#"__zgui.setProperty(node, 'ariaLevel', null); 'clear'"#);
    let semantics = native.backend.semantics(native.node).unwrap();
    assert_eq!(semantics.position.level, None);
    assert_eq!(semantics.role, Role::Heading);
    native.script(r#"__zgui.setProperty(node, 'ariaLevel', 1); __zgui.setProperty(node, 'ariaLevel', undefined); 'clear'"#);
    assert_eq!(native.backend.semantics(native.node).unwrap().position.level, None);
}

#[test]
fn popup_semantics_apply_replace_and_clear_without_changing_the_role() {
    let mut native = NativeProperties::new();
    native.script("const trigger = __zgui.createElement('control'); __zgui.insertNode(root, trigger, null); __zgui.setProperty(node, 'role', 'dialog'); __zgui.setProperty(node, 'ariaHasPopup', 'dialog'); __zgui.setProperty(node, 'ariaPopupFor', trigger); 'set'");
    let trigger = native.backend.children(root_handle(&native))[1];
    let semantics = native.backend.semantics(native.node).unwrap();
    assert_eq!(semantics.has_popup, Some(zgui::vocab::HasPopup::Dialog));
    assert_eq!(semantics.relations.popup_for, Some(zgui::vocab::NodeId(trigger.as_u64())));
    native.script("__zgui.setProperty(node, 'ariaHasPopup', 'listbox'); 'replace'");
    assert_eq!(native.backend.semantics(native.node).unwrap().has_popup, Some(zgui::vocab::HasPopup::Listbox));
    for empty in ["null", "undefined"] {
        native.script(&format!("__zgui.setProperty(node, 'ariaHasPopup', 'dialog'); __zgui.setProperty(node, 'ariaPopupFor', trigger); __zgui.setProperty(node, 'ariaHasPopup', {empty}); __zgui.setProperty(node, 'ariaPopupFor', {empty}); 'clear'"));
        let semantics = native.backend.semantics(native.node).unwrap();
        assert_eq!(semantics.has_popup, None);
        assert_eq!(semantics.relations.popup_for, None);
        assert_eq!(semantics.role, Role::Dialog);
    }
}

#[test]
fn popup_semantics_reject_invalid_and_foreign_targets_before_mutating() {
    let mut native = NativeProperties::new();
    native.script(r#"
        const trigger = __zgui.createElement('control');
        __zgui.insertNode(root, trigger, null);
        __zgui.setProperty(node, 'ariaHasPopup', 'dialog');
        __zgui.setProperty(node, 'ariaPopupFor', trigger);
        for (const [name, values] of [
            ['ariaHasPopup', ['other', true, false, 1, {}, []]],
            ['ariaPopupFor', ['id', 1, true, {}, [], [trigger]]],
        ]) for (const value of values) {
            let rejected = false;
            try { __zgui.setProperty(node, name, value); } catch (e) { rejected = e instanceof TypeError; }
            if (!rejected) throw Error(name + ' accepted invalid input');
        }
        let getterRan = false;
        const targets = [trigger];
        Object.defineProperty(targets, 0, { get() { getterRan = true; return trigger; } });
        try { __zgui.setProperty(node, 'ariaPopupFor', targets); } catch (e) { if (!(e instanceof TypeError)) throw e; }
        if (getterRan) throw Error('scalar relation read an array getter');
        'rejected'
    "#);
    let trigger = native.backend.children(root_handle(&native))[1];
    let foreign = UiBridge::new(NodeRegistry::new(native.dom.clone(), root_handle(&native), 38, 1, 100), HostHandle::new(StubHost::default()));
    native.engine.with_context(|ctx| ctx.globals().set("foreign", foreign.root(&ctx)?)).unwrap();
    native.script(r#"
        let foreignRejected = false;
        try { __zgui.setProperty(node, 'ariaPopupFor', foreign); } catch (e) { foreignRejected = e instanceof TypeError; }
        if (!foreignRejected) throw Error('foreign popup owner accepted');
        'foreign'
    "#);
    foreign.dispose();
    let semantics = native.backend.semantics(native.node).unwrap();
    assert_eq!(semantics.has_popup, Some(zgui::vocab::HasPopup::Dialog));
    assert_eq!(semantics.relations.popup_for, Some(zgui::vocab::NodeId(trigger.as_u64())));
    native.script("__zgui.removeNode(root, trigger); 'remove'");
    native.backend.end_frame();
    native.script(r#"
        let staleRejected = false;
        try { __zgui.setProperty(node, 'ariaPopupFor', trigger); } catch (e) { staleRejected = e instanceof TypeError; }
        if (!staleRejected) throw Error('stale popup owner accepted');
        'stale'
    "#);
}

#[test]
fn keyboard_shortcuts_preserve_other_semantics_and_reject_non_strings() {
    let mut native = NativeProperties::new();
    native.script("__zgui.setProperty(node, 'role', 'menuItem'); __zgui.setProperty(node, 'ariaLabel', 'Save'); __zgui.setProperty(node, 'ariaKeyShortcuts', 'Control+S'); 'set'");
    assert_eq!(native.backend.semantics(native.node).unwrap().keyboard_shortcut.as_deref(), Some("Control+S"));
    native.script("__zgui.setProperty(node, 'ariaKeyShortcuts', '⌘S'); 'replace'");
    assert_eq!(native.backend.semantics(native.node).unwrap().keyboard_shortcut.as_deref(), Some("⌘S"));
    native.script(r#"
        for (const value of [1, true, {}, [], () => 'x']) {
            let rejected = false;
            try { __zgui.setProperty(node, 'ariaKeyShortcuts', value); } catch (error) { rejected = error instanceof TypeError; }
            if (!rejected) throw Error('invalid shortcut accepted');
        }
        'rejected'
    "#);
    assert_eq!(native.backend.semantics(native.node).unwrap().keyboard_shortcut.as_deref(), Some("⌘S"));
    for empty in ["null", "undefined"] {
        native.script(&format!("__zgui.setProperty(node, 'ariaKeyShortcuts', 'Control+S'); __zgui.setProperty(node, 'ariaKeyShortcuts', {empty}); 'clear'"));
        let semantics = native.backend.semantics(native.node).unwrap();
        assert_eq!(semantics.keyboard_shortcut, None);
        assert_eq!(semantics.role, Role::MenuItem);
        assert_eq!(semantics.label.as_deref(), Some("Save"));
    }
    native.script("__zgui.setProperty(node, 'ariaKeyShortcuts', ''); 'empty'");
    assert_eq!(native.backend.semantics(native.node).unwrap().keyboard_shortcut.as_deref(), Some(""));
}
