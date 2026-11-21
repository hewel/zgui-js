use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

use js_runtime::{JsRuntime, RuntimeLimits};
use ui_bridge::{BridgeOptions, HostPort, UiBridge, registry::NodeRegistry};
use zgui::{view::{AttrName, DomHandle, ElementName, HostHandle, NodeId, PropKey, UiState, stub::StubHost}, vocab::{Invalid, Orientation, Role, SemanticFlags, Toggled}};
use zgui::{canvas::{self, CanvasToken}, vocab::prop::drawing};
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
        Self::with_options(BridgeOptions { strict_styles: true, ..BridgeOptions::default() })
    }

    fn with_options(options: BridgeOptions) -> Self {
        let document = Rc::new(RefCell::new(Document::new()));
        let backend = Rc::new(DocumentDom::new(document));
        let dom = DomHandle::from_rc(backend.clone());
        let root = dom.create_element(ElementName::new("column"));
        dom.insert(backend.root_node(), root, None);
        let mut engine = JsRuntime::new(RuntimeLimits::default()).unwrap();
        let bridge = UiBridge::new_with_options(NodeRegistry::new(dom.clone(), root, 37, 1, 100), HostHandle::new(StubHost::default()), HostPort::queued(), options);
        engine.with_context(|ctx| {
            bridge.install(&ctx)?;
            ctx.globals().set("root", bridge.root(&ctx)?)
        }).unwrap();
        bridge.run_turn(engine.turn(), |_, js| js.evaluate_script(r#"
            const node = __zgui.createElement('control');
            __zgui.insertNode(root, node, null);
            'mounted'
        "#)).unwrap();
        let node = backend.children(root)[0];
        Self { bridge, engine, backend, dom, node }
    }

    fn script(&mut self, script: &str) {
        self.bridge.run_turn(self.engine.turn(), |_, js| js.evaluate_script(script)).unwrap();
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
            ['unknown', null], ['children', 'bad'], ['ref', () => {}],
            ['value', 'not editable'], ['scrollTop', 12], ['d', 'M0 0 L1 1'],
            ['class', 3], ['id', false], ['title', {}], ['ariaLabel', 1], ['ariaDescription', true],
            ['disabled', 'false'], ['disabled', 0], ['role', 'paragraph'], ['role', 1],
            ['tabIndex', '0'], ['tabIndex', false], ['tabIndex', 0.5],
            ['tabIndex', Infinity], ['tabIndex', NaN], ['tabIndex', 2147483648], ['tabIndex', -2147483649],
            ['style', 12], ['style', false], ['style', []], ['style', () => {}],
            ['style', {width: 12}], ['style', {width: true}], ['style', {width: {}}],
            ['style', {width: '3px', height: 12}], ['style', {colour: 'red'}],
            ['style', {'--': 'red'}],
            ['--', 'red'], ['--accent', false]
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
    // The preview style engine enables the full profile before applying inline styles.
    zgui_style::preflight_stylesheet(".visual { backdrop-filter: blur(4px) }").unwrap();
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
        ("boxShadow", "2px 3px 4px red"), ("textShadow", "1px 2px blue"),
        ("filter", "brightness(0.5)"), ("backdropFilter", "blur(4px)"),
        ("mixBlendMode", "multiply"), ("isolation", "isolate"),
        ("fontVariationSettings", r#""wght" 650"#), ("fontStyle", "oblique 12deg"),
        ("fontStretch", "condensed"), ("whiteSpace", "pre-wrap"), ("textOverflow", "ellipsis"),
        ("pointerEvents", "none"), ("userSelect", "none"),
        ("translate", "10px -20%"), ("rotate", "15deg"), ("scale", "1.25 0.75"),
        ("outline", "2px solid blue"), ("outlineColor", "red"), ("outlineStyle", "solid"),
        ("outlineWidth", "2px"), ("outlineOffset", "3px"),
        ("letterSpacing", "0.05em"), ("fontVariantNumeric", "tabular-nums"),
        ("fontFeatureSettings", r#""tnum" 1"#),
        ("textAlign", "center"), ("textTransform", "uppercase"),
        ("textDecoration", "underline"), ("textDecorationLine", "line-through"),
        ("gridTemplateColumns", "1fr 2fr"), ("gridTemplateRows", "auto 1fr"),
        ("gridColumn", "1 / 3"), ("gridRow", "1 / 2"), ("gridArea", "1 / 1 / 2 / 3"),
        ("gridAutoFlow", "dense"),
        ("borderTop", "1px solid red"), ("borderRight", "1px solid green"),
        ("borderBottom", "1px solid blue"), ("borderLeft", "1px solid black"),
        ("borderStyle", "dashed"),
        ("flex", "1 0 auto"), ("alignSelf", "stretch"), ("justifySelf", "end"),
        ("boxSizing", "border-box"), ("overflowX", "hidden"), ("overflowY", "auto"),
    ];
    for (name, value) in declarations {
        native.script(&format!("__zgui.setProperty(node, 'style', {{'{name}': '{value}'}}); 'set'"));
        assert!(!native.style().0.is_empty(), "native parser did not accept {name}: {value}");
        native.script("__zgui.setProperty(node, 'style', null); 'clear'");
        assert!(native.style().0.is_empty(), "native declarations remained after clearing {name}");
    }
    native.script("__zgui.setProperty(node, 'style', { flex: 1 }); 'set'");
    assert!(!native.style().0.is_empty(), "unitless numeric flex must be accepted");
    native.script("__zgui.setProperty(node, 'style', null); 'clear'");
}

#[test]
fn image_object_styles_apply_with_kebab_case_and_camel_case_names() {
    let mut native = NativeProperties::new();
    let names = BTreeSet::from(["object-fit".into(), "object-position".into()]);
    for (fit_name, position_name) in [("object-fit", "object-position"), ("objectFit", "objectPosition")] {
        for fit in ["fill", "contain", "cover", "none", "scale-down"] {
            let mut seen_positions = BTreeSet::new();
            for position in ["center top", "50% 50%"] {
                native.script(&format!(
                    "__zgui.setProperty(node, 'style', {{'{fit_name}': '{fit}', '{position_name}': '{position}'}}); 'set'"
                ));
                let (actual, css) = native.style();
                assert_eq!(actual, names, "{fit_name}: {fit}; {position_name}: {position}");
                assert!(
                    css.contains(&format!("object-fit: {fit};")),
                    "missing object-fit: {fit}; for {fit_name}/{position_name}; CSS: {css}"
                );
                let position_decl = css
                    .split(';')
                    .map(str::trim)
                    .find(|decl| decl.starts_with("object-position:"))
                    .unwrap_or_else(|| panic!("missing object-position for {fit_name}/{position_name}: {css}"));
                seen_positions.insert(position_decl.to_owned());
            }
            assert_eq!(
                seen_positions.len(),
                2,
                "distinct object-position inputs collapsed for {fit_name}/{position_name} ({fit}): {seen_positions:?}"
            );
        }
        native.script(&format!(
            "__zgui.setProperty(node, 'style', {{'{fit_name}': null, '{position_name}': undefined}}); 'clear'"
        ));
        let remaining = native.style().0;
        assert!(
            remaining.is_empty(),
            "declarations remained after clearing {fit_name}/{position_name}: {remaining:?}"
        );
    }
}

#[test]
fn image_object_styles_reject_non_strings_without_replacing_inline_style() {
    let mut native = NativeProperties::new();
    native.script("__zgui.setProperty(node, 'style', {height: '11px'}); 'set'");
    let before = native.style();
    native.script(r#"
        for (const name of ['object-fit', 'object-position', 'objectFit', 'objectPosition']) {
            for (const invalid of [1, true]) {
                try {
                    __zgui.setProperty(node, 'style', {width: '3px', [name]: invalid});
                } catch (error) {
                    if (error.message.startsWith('STYLE_VALUE_INVALID:') && !error.message.includes('STYLE_NAME_INVALID:')) continue;
                    throw error;
                }
                throw Error(`accepted non-string ${typeof invalid} for ${name}`);
            }
        }
        'rejected'
    "#);
    assert_eq!(native.style(), before);
}

#[test]
fn css_strings_reject_the_entire_recovered_subset() {
    let mut native = NativeProperties::new();
    native.script("__zgui.setProperty(node, 'style', 'height: 11px'); try { __zgui.setProperty(node, 'style', 'unknown-property: nope; width: 10px; height: invalid; --ok: 3px'); } catch {} 'parse'");
    assert_eq!(native.style().0, BTreeSet::from(["height".into()]));
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
        __zgui.setProperty(vector, 'svg', '<svg viewBox="0 0 16 16"><rect width="8" height="8" fill="red"/></svg>');
        'drawn'
    "#);
    let parent = native.dom.parent(native.node).unwrap();
    let vector = native.backend.children(parent)[1];
    assert_eq!(native.backend.property(vector, PropKey::new("d")).as_str(), Some("M0 0 L8 8"));
    assert_eq!(native.backend.property(vector, PropKey::new("viewBox")).as_str(), Some("0 0 16 16"));
    assert_eq!(
        native.backend.property(vector, PropKey::new("svg")).as_str(),
        Some(r#"<svg viewBox="0 0 16 16"><rect width="8" height="8" fill="red"/></svg>"#),
        "SVG is stored as drawing text alongside the path source",
    );
    assert_eq!(native.backend.attribute(vector, AttrName::new("d")), None);
    assert_eq!(native.backend.attribute(vector, AttrName::new("viewBox")), None);
    assert_eq!(native.backend.attribute(vector, AttrName::new("svg")), None, "SVG must not become an attribute");
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
            rejected = false;
            try { __zgui.setProperty(target, 'svg', '<svg/>'); }
            catch (error) { rejected = String(error).includes('require a vector'); }
            if (!rejected) throw Error('svg accepted on a non-vector');
        }
        for (const invalid of [3, false, {}]) {
            let rejected = false;
            try { __zgui.setProperty(vector, 'd', invalid); } catch { rejected = true; }
            if (!rejected) throw Error('invalid d accepted');
            rejected = false;
            try { __zgui.setProperty(vector, 'svg', invalid); } catch { rejected = true; }
            if (!rejected) throw Error('invalid svg accepted');
        }
        let rejected = false;
        try { __zgui.insertNode(vector, __zgui.createTextNode('forbidden'), null); }
        catch (error) { rejected = String(error).includes('vector elements cannot contain'); }
        if (!rejected) throw Error('vector accepted children');
        'rejected'
    "#);
    assert_eq!(native.backend.property(vector, PropKey::new("d")).as_str(), Some("M0 0 L8 8"));
    native.script("__zgui.setProperty(vector, 'svg', null); 'cleared document'");
    assert!(native.backend.property(vector, PropKey::new("svg")).is_unset(), "clearing SVG removes the document source");
    assert_eq!(native.backend.property(vector, PropKey::new("d")).as_str(), Some("M0 0 L8 8"), "clearing SVG preserves the path fallback");
    native.script("__zgui.setProperty(vector, 'd', null); __zgui.setProperty(vector, 'viewBox', undefined); 'cleared'");
    assert!(native.backend.property(vector, PropKey::new("d")).is_unset());
    assert!(native.backend.property(vector, PropKey::new("viewBox")).is_unset());
    native.script("__zgui.removeNode(root, vector); 'removed'");
    assert_eq!(native.bridge.snapshot(10).nodes.len(), 2);
    native.backend.end_frame();
    assert!(!native.dom.is_live(vector));
}

#[test]
fn svg_documents_cannot_resolve_images_through_the_host_filesystem() {
    let mut native = NativeProperties::new();
    native.script(r#"
        const vector = __zgui.createElement('vector');
        __zgui.insertNode(root, vector, null);
        __zgui.setProperty(vector, 'svg', '<svg><rect width="8" height="8"/></svg>');
        const sources = [
            '<svg xmlns:s="http://www.w3.org/2000/svg"><s:image href="not-a-package-asset.svg"/></svg>',
            '<svg><filter id="filter"><feImage href="not-a-package-asset.png"/></filter></svg>',
            '<!DOCTYPE svg [<!ENTITY artwork "<image/>">]><svg>&artwork;</svg>',
        ];
        for (const source of sources) {
            try { __zgui.setProperty(vector, 'svg', source); } catch (error) {
                if (!(error instanceof TypeError)) throw error;
                continue;
            }
            throw Error('accepted an unsupported SVG image resolver input');
        }
        'checked'
    "#);
    let parent = native.dom.parent(native.node).expect("control is mounted");
    let vector = native.backend.children(parent)[1];
    assert_eq!(
        native.backend.property(vector, PropKey::new(drawing::DOCUMENT)).as_str(),
        Some(r#"<svg><rect width="8" height="8"/></svg>"#),
        "rejected documents must not replace a drawable source",
    );
}

#[test]
fn a_non_svg_root_cannot_mask_a_vector_path_fallback() {
    let mut native = NativeProperties::new();
    native.script(r#"
        const vector = __zgui.createElement('vector');
        __zgui.insertNode(root, vector, null);
        __zgui.setProperty(vector, 'd', 'M0 0 L8 8');
        for (const source of ['<html><body/></html>', '<svg xmlns="urn:not-svg"/>']) {
            try { __zgui.setProperty(vector, 'svg', source); } catch (error) {
                if (error instanceof TypeError) continue;
                throw error;
            }
            throw Error(`accepted a non-SVG root: ${source}`);
        }
        'rejected'
    "#);
    let parent = native.dom.parent(native.node).expect("control is mounted");
    let vector = native.backend.children(parent)[1];
    assert!(native.backend.property(vector, PropKey::new(drawing::DOCUMENT)).is_unset());
    assert_eq!(native.backend.property(vector, PropKey::new(drawing::PATHS)).as_str(), Some("M0 0 L8 8"));
}

#[test]
fn svg_accepts_foreign_namespace_image_metadata() {
    let mut native = NativeProperties::new();
    let source = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:meta="urn:metadata"><metadata><meta:image href="catalog.svg"/><meta:feImage href="catalog.png"/></metadata><rect width="8" height="8"/></svg>"#;
    native.script(&format!(r#"
        const vector = __zgui.createElement('vector');
        __zgui.insertNode(root, vector, null);
        __zgui.setProperty(vector, 'svg', {source:?});
        'accepted'
    "#));
    let parent = native.dom.parent(native.node).expect("control is mounted");
    let vector = native.backend.children(parent)[1];
    assert_eq!(native.backend.property(vector, PropKey::new(drawing::DOCUMENT)).as_str(), Some(source));
}

#[test]
fn retained_canvas_updates_current_bindings_and_releases_scenes() {
    let mut native = NativeProperties::new();
    native.script(r#"
        const scene = __zgui.createCanvasScene();
        const next = __zgui.createCanvasScene();
        const vectors = Array.from({length: 3}, () => {
            const vector = __zgui.createElement('vector');
            __zgui.insertNode(root, vector, null);
            __zgui.setProperty(vector, 'canvas', scene);
            return vector;
        });
        const reject = (call, pattern) => {
            try { call(); } catch (error) { if (pattern.test(String(error))) return; throw error; }
            throw Error('invalid canvas operation was accepted');
        };
        reject(() => __zgui.setProperty(node, 'canvas', scene), /require a vector/);
        reject(() => __zgui.setProperty(vectors[0], 'canvas', {token: scene.token}), /canvas scene/i);
        scene.update([{kind: 'rect', x: 0, y: 0, width: 10, height: 20, fill: '#02a'}]);
        'drawn'
    "#);
    let parent = native.dom.parent(native.node).expect("control is mounted");
    let children = native.backend.children(parent);
    let [a, b, c] = children[1..] else { panic!("expected three vectors: {children:?}") };
    let packed = |native: &NativeProperties, node| native.backend.property(node, PropKey::new(drawing::CANVAS)).as_integer().map(drawing::canvas_ref);
    let view = |native: &NativeProperties, node| native.backend.property(node, PropKey::new(drawing::CANVAS_VIEW)).as_integer();
    let (token, revision) = packed(&native, a).expect("integer canvas reference");
    let token = CanvasToken(token);
    assert_eq!(revision, 1, "first edit advances content revision");
    assert_eq!(packed(&native, b), Some((token.0, 1)), "edit reaches all bound vectors");
    assert_eq!(packed(&native, c), Some((token.0, 1)), "edit reaches the third vector");

    native.script(r#"
        __zgui.setProperty(vectors[1], 'canvas', next);
        __zgui.setProperty(vectors[2], 'canvas', next);
        __zgui.removeNode(root, vectors[2]);
        scene.update(null, [1, 0, 0, 1, 8, 5]);
        'transformed'
    "#);
    let (next_token, next_revision) = packed(&native, b).expect("replacement canvas reference");
    assert_ne!(token.0, next_token, "scenes have distinct tokens");
    assert_eq!(next_revision, 0, "replacement scene has not been edited");
    assert_eq!(view(&native, b), Some(0), "rebound vector ignores old scene view changes");
    assert_eq!(view(&native, a), Some(1), "transform advances the bound view");
    assert_eq!(packed(&native, a), Some((token.0, 1)), "transform preserves content revision");
    native.backend.end_frame();

    native.script(r#"
        scene.update([
            {kind: 'circle', x: 10, y: 10, radius: 4, stroke: {color: '#0fa8', width: 2, lineCap: 'round', lineJoin: 'bevel'}},
            {kind: 'arc', x: 20, y: 20, radius: 8, startAngle: 0, sweepAngle: 3.14, stroke: {color: '#fff', width: 3, lineCap: 'square', lineJoin: 'round'}},
        ]);
        reject(() => scene.update([{kind: 'circle', x: 10, y: 10, radius: 4, stroke: {color: '#fff', width: 2, lineCap: 'invalid'}}]), /lineCap/);
        reject(() => scene.update([{kind: 'circle', x: 10, y: 10, radius: 4, stroke: {color: '#fff', width: 2, lineJoin: 'invalid'}}]), /lineJoin/);
        __zgui.setProperty(vectors[1], 'canvas', null);
        next.update([], [1, 0, 0, 1, 5, 0]);
        'updated'
    "#);
    let resolved = canvas::resolve(token).expect("live canvas scene");
    {
        let guard = resolved.lock().expect("lock canvas scene");
        let resolved_stroke = guard.shapes()[0].stroke.as_ref().expect("circle stroke");
        assert_eq!(resolved_stroke.style.start_cap, zgui::scene::kurbo::Cap::Round);
        assert_eq!(resolved_stroke.style.end_cap, zgui::scene::kurbo::Cap::Round);
        assert_eq!(resolved_stroke.style.join, zgui::scene::kurbo::Join::Bevel);
        let arc_stroke = guard.shapes()[1].stroke.as_ref().expect("arc stroke");
        assert_eq!(arc_stroke.style.start_cap, zgui::scene::kurbo::Cap::Square);
        assert_eq!(arc_stroke.style.join, zgui::scene::kurbo::Join::Round);
    }
    drop(resolved);
    assert_eq!(packed(&native, a), Some((token.0, 2)), "live edits invalidate current bindings");
    assert_eq!(view(&native, a), Some(1), "content edits preserve the view");
    assert_eq!(packed(&native, b), None, "unbound vector ignores canvas edits");
    assert_eq!(view(&native, b), None, "unbinding also clears view");
    assert!(!native.dom.is_live(c), "removed bound vector was reclaimed");
    native.script(r#"
        const {dispose} = scene;
        dispose();
        dispose();
        reject(() => scene.update([]), /disposed/);
        reject(() => __zgui.setProperty(vectors[0], 'canvas', scene), /disposed/);
        'disposed'
    "#);
    assert_eq!(packed(&native, a), None, "dispose clears the bound canvas");
    assert_eq!(view(&native, a), None, "dispose clears its view");
    assert!(canvas::resolve(token).is_none(), "dispose unregisters the scene");
    assert!(canvas::resolve(CanvasToken(next_token)).is_some(), "unbinding does not dispose");
    native.bridge.dispose();
    assert!(canvas::resolve(CanvasToken(next_token)).is_none(), "bridge disposal drops remaining scenes");
}

#[test]
fn canvas_commands_validate_the_whole_update_before_editing() {
    let mut native = NativeProperties::new();
    native.script(r#"
        const scene = __zgui.createCanvasScene();
        const vector = __zgui.createElement('vector');
        __zgui.insertNode(root, vector, null);
        __zgui.setProperty(vector, 'canvas', scene);
        const rect = {kind: 'roundedRect', x: 0, y: 0, width: 10, height: 12, radius: 2, fill: '#69afde'};
        const arc = {kind: 'arc', x: 10, y: 10, radius: 8, startAngle: 0, sweepAngle: Math.PI, stroke: {color: '#abcd0123', width: 2}};
        const path = {kind: 'path', fill: '#123', segments: [
            {kind: 'moveTo', x: 0, y: 0}, {kind: 'lineTo', x: 3, y: 0},
            {kind: 'quadTo', cx: 8, cy: 1, x: 10, y: 4},
            {kind: 'cubicTo', c1x: 7, c1y: 8, c2x: 1, c2y: 8, x: 0, y: 4}, {kind: 'close'},
        ]};
        scene.update([rect, arc, path]);
        const huge = [];
        huge.length = 4294967295;
        for (const [commands, transform] of [
            [huge], [[{...path, segments: huge}]], [[{...path, segments: [{kind:'lineTo', x: 1, y: 2}]}]],
            [[rect, {...rect, kind: 'unknown'}]], [[{...rect, width: Infinity}]], [[{...rect, width: '12'}]],
            [[{...rect, fill: 'red'}]], [[{...rect, fill: '#12345'}]], [[{...rect, fill: undefined}]],
            [[{...rect, fill: '#+12'}]], [[{...rect, fill: '#\ud80012'}]],
            [[{...arc, sweepAngle: 1000000}]], [[{...arc, stroke: {color: '#abc', width: 0}}]],
            [null, [1, 0, 0, 1, NaN, 0]], [[rect], [1, 0, 0, 1, 0]], [[], huge],
        ]) {
            try { scene.update(commands, transform); } catch (error) {
                if (!(error instanceof TypeError || error instanceof RangeError)) throw error;
                continue;
            }
            throw Error('invalid update was accepted');
        }
        'checked'
    "#);
    let parent = native.dom.parent(native.node).expect("control is mounted");
    let node = native.backend.children(parent)[1];
    let packed = native.backend.property(node, PropKey::new(drawing::CANVAS)).as_integer().expect("canvas reference");
    let (token, revision) = drawing::canvas_ref(packed);
    assert_eq!(revision, 1, "rejected updates do not advance content revision");
    assert_eq!(native.backend.property(node, PropKey::new(drawing::CANVAS_VIEW)).as_integer(), Some(0), "rejected updates do not advance view");
    let scene = canvas::resolve(CanvasToken(token)).expect("scene survives invalid edits");
    assert_eq!(scene.lock().expect("scene readable").shapes().len(), 3, "valid geometry survives invalid batches");
}

#[test]
fn canvas_scene_limit_is_independent_of_nodes_and_disposal_frees_a_slot() {
    let mut native = NativeProperties::new();
    native.script(r#"
        const scenes = Array.from({length: 256}, () => __zgui.createCanvasScene());
        const rejectScene = () => {
            try { __zgui.createCanvasScene(); } catch (error) {
                if (error instanceof RangeError) return;
                throw error;
            }
            throw Error('created a scene over the session limit');
        };
        rejectScene();
        'full'
    "#);
    assert_eq!(native.bridge.canvas_scene_count(), 256, "failed creation leaves the live scenes intact");
    native.script(r#"
        scenes[0].dispose();
        scenes[0].dispose();
        const replacement = __zgui.createCanvasScene();
        if (replacement.token === scenes[0].token) throw Error('reused a disposed token');
        rejectScene();
        'full again'
    "#);
    assert_eq!(native.bridge.canvas_scene_count(), 256, "disposal frees exactly one slot");
}

#[test]
fn canvas_retained_shape_budget_is_shared_and_reclaimed_on_update_and_dispose() {
    let mut native = NativeProperties::new();
    native.script(r#"
        const create = () => __zgui.createCanvasScene();
        const [first, second, extra] = [create(), create(), create()];
        const line = {kind: 'path', segments: [
            {kind: 'moveTo', x: 0, y: 0}, {kind: 'lineTo', x: 8, y: 8},
        ], stroke: {color: '#123', width: 1}};
        const batch = Array(4096).fill(line);
        const reject = () => {
            try { extra.update([line]); } catch (error) {
                if (error instanceof RangeError) return;
                throw error;
            }
            throw Error('exceeded the session shape budget');
        };
        first.update(batch);
        'half full'
    "#);
    native.script("second.update(batch); reject(); 'full'");
    native.script("first.update(batch); reject(); 'replacement at the limit'");
    native.script("first.update([]); extra.update(batch); 'clear freed capacity'");
    native.script("second.dispose(); first.update(batch); 'disposal freed capacity'");
}

#[test]
fn canvas_retained_segment_budget_rejects_geometry_and_transform_atomically() {
    let mut native = NativeProperties::new();
    native.script(r#"
        const [first, second, target] = Array.from({length: 3}, () => __zgui.createCanvasScene());
        const vector = __zgui.createElement('vector');
        __zgui.insertNode(root, vector, null);
        __zgui.setProperty(vector, 'canvas', target);
        const segments = Array(16384).fill({kind: 'lineTo', x: 8, y: 8});
        segments[0] = {kind: 'moveTo', x: 0, y: 0};
        const batch = [{kind: 'path', fill: '#123', segments}];
        const rect = {kind: 'rect', x: 0, y: 0, width: 8, height: 8, fill: '#123'};
        first.update(batch);
        'half full'
    "#);
    native.script(r#"
        second.update(batch);
        try { target.update([rect], [1, 0, 0, 1, 8, 5]); } catch (error) {
            if (!(error instanceof RangeError)) throw error;
        }
        'full'
    "#);
    let parent = native.dom.parent(native.node).expect("control is mounted");
    let vector = native.backend.children(parent)[1];
    let property = |key| native.backend.property(vector, PropKey::new(key)).as_integer();
    assert_eq!(property(drawing::CANVAS).map(drawing::canvas_ref).map(|(_, revision)| revision), Some(0),
        "an over-budget rectangle must not advance the content revision");
    assert_eq!(property(drawing::CANVAS_VIEW), Some(0), "the rejected update must not change the view");
    native.script("first.update([]); target.update(batch); 'clear freed segments'");
    native.script("second.dispose(); first.update(batch); 'disposal freed segments'");
}

#[test]
fn a_canvas_handle_from_another_bridge_cannot_be_bound() {
    let mut native = NativeProperties::new();
    let backend = Rc::new(DocumentDom::new(Rc::new(RefCell::new(Document::new()))));
    let dom = DomHandle::from_rc(backend.clone());
    let second = UiBridge::new(NodeRegistry::new(dom, backend.root_node(), 38, 1, 100), HostHandle::new(StubHost::default()), HostPort::queued());
    native.engine.with_context(|ctx| {
        let globals = ctx.globals();
        let original: rquickjs::Object<'_> = globals.get("__zgui")?;
        second.install(&ctx)?;
        globals.set("foreignApi", globals.get::<_, rquickjs::Object<'_>>("__zgui")?)?;
        globals.set("__zgui", original)
    }).expect("install a second bridge in the same context");
    second.run_turn(native.engine.turn(), |_, js| js.evaluate_script(
        "const foreignScene = foreignApi.createCanvasScene(); 'registered'",
    )).expect("foreign bridge creates a real canvas handle");
    native.script(r#"
        const vector = __zgui.createElement('vector');
        __zgui.insertNode(root, vector, null);
        try { __zgui.setProperty(vector, 'canvas', foreignScene); }
        catch (error) {
            if (!String(error).includes('does not belong to this session')) throw error;
        }
        'checked'
    "#);
    let parent = native.dom.parent(native.node).expect("control is mounted");
    let node = native.backend.children(parent)[1];
    assert!(native.backend.property(node, PropKey::new(drawing::CANVAS)).is_unset(), "foreign handle cannot bind even in the same JS context");
}

#[test]
fn canvas_gradients_and_dashed_strokes_decode_and_enforce_bounds() {
    let mut native = NativeProperties::new();
    native.script(r#"
        const scene = __zgui.createCanvasScene();
        const vector = __zgui.createElement('vector');
        __zgui.insertNode(root, vector, null);
        __zgui.setProperty(vector, 'canvas', scene);
        const linear = {
            kind: 'linear', x0: 0, y0: 0, x1: 100, y1: 0,
            stops: [{offset: 0, color: '#ff0000'}, {offset: 0.5, color: '#00ff00'}, {offset: 1, color: '#0000ff'}],
        };
        const radialCircular = {
            kind: 'radial', cx: 50, cy: 50, radius: 40, repeating: true,
            stops: [{offset: 0, color: '#ffffff'}, {offset: 1, color: '#000000'}],
        };
        const radialElliptical = {
            kind: 'radial', cx: 40, cy: 30, radiusX: 30, radiusY: 15,
            stops: [{offset: 0, color: '#ff0'}, {offset: 1, color: '#00f'}],
        };
        scene.update([
            {kind: 'rect', x: 0, y: 0, width: 100, height: 40, fill: linear},
            {kind: 'circle', x: 50, y: 50, radius: 40, fill: radialCircular, stroke: {color: linear, width: 4, lineDash: [8, 4], lineDashOffset: 1000004}},
            {kind: 'roundedRect', x: 10, y: 10, width: 60, height: 30, radius: 4, fill: radialElliptical, stroke: {color: '#112233', width: 2, lineDash: []}},
        ]);
        const tooManyStops = Array.from({length: 65}, (_, i) => ({offset: i / 64, color: '#fff'}));
        const tooLongDash = Array.from({length: 33}, () => 2);
        for (const bad of [
            [{kind: 'rect', x: 0, y: 0, width: 10, height: 10, fill: {kind: 'linear', x0: 0, y0: 0, x1: 10, y1: 0, stops: [{offset: 0, color: '#fff'}]}}],
            [{kind: 'rect', x: 0, y: 0, width: 10, height: 10, fill: {kind: 'linear', x0: 0, y0: 0, x1: 10, y1: 0, stops: tooManyStops}}],
            [{kind: 'rect', x: 0, y: 0, width: 10, height: 10, fill: {kind: 'linear', x0: 0, y0: 0, x1: 10, y1: 0, stops: [{offset: -0.1, color: '#fff'}, {offset: 1, color: '#000'}]}}],
            [{kind: 'rect', x: 0, y: 0, width: 10, height: 10, fill: {kind: 'linear', x0: 0, y0: 0, x1: 10, y1: 0, stops: [{offset: 0.8, color: '#fff'}, {offset: 0.2, color: '#000'}]}}],
            [{kind: 'rect', x: 0, y: 0, width: 10, height: 10, fill: {kind: 'linear', x0: 0, y0: 0, x1: 10, y1: 0, stops: [{offset: 0, color: 'red'}, {offset: 1, color: '#000'}]}}],
            [{kind: 'rect', x: 0, y: 0, width: 10, height: 10, fill: {kind: 'conic', cx: 5, cy: 5, stops: [{offset: 0, color: '#fff'}, {offset: 1, color: '#000'}]}}],
            [{kind: 'rect', x: 0, y: 0, width: 10, height: 10, fill: {kind: 'radial', cx: 5, cy: 5, radius: 0, stops: [{offset: 0, color: '#fff'}, {offset: 1, color: '#000'}]}}],
            [{kind: 'rect', x: 0, y: 0, width: 10, height: 10, fill: {kind: 'radial', cx: 5, cy: 5, radius: 5, radiusX: 5, radiusY: 5, stops: [{offset: 0, color: '#fff'}, {offset: 1, color: '#000'}]}}],
            [{kind: 'rect', x: 0, y: 0, width: 10, height: 10, fill: {kind: 'linear', x0: 0, y0: 0, x1: 10, y1: 0, repeating: 'yes', stops: [{offset: 0, color: '#fff'}, {offset: 1, color: '#000'}]}}],
            [{kind: 'rect', x: 0, y: 0, width: 10, height: 10, stroke: {color: '#fff', width: 2, lineDash: tooLongDash}}],
            [{kind: 'rect', x: 0, y: 0, width: 10, height: 10, stroke: {color: '#fff', width: 2, lineDash: [-1, 4]}}],
            [{kind: 'rect', x: 0, y: 0, width: 10, height: 10, stroke: {color: '#fff', width: 2, lineDash: [0, 0]}}],
            [{kind: 'rect', x: 0, y: 0, width: 10, height: 10, stroke: {color: '#fff', width: 2, lineDash: [4, 2], lineDashOffset: NaN}}],
            [{kind: 'rect', x: 0, y: 0, width: 100000, height: 100000, stroke: {color: '#fff', width: 2, lineDash: [0.01, 0.01]}}],
        ]) {
            try { scene.update(bad); } catch (error) {
                if (!(error instanceof TypeError || error instanceof RangeError)) throw error;
                continue;
            }
            throw Error('invalid Phase 2a gradient/dash command was accepted');
        }
        // View-only transform update preserves geometry & content revision.
        scene.update(null, [1, 0, 0, 1, 12, 8]);
        'ok'
    "#);
    let parent = native.dom.parent(native.node).expect("mounted");
    let vector = native.backend.children(parent)[1];
    let packed = native.backend.property(vector, PropKey::new(drawing::CANVAS)).as_integer().expect("canvas ref");
    let (token, revision) = drawing::canvas_ref(packed);
    assert_eq!(revision, 1, "rejected commands and view-only transform must not advance content revision");
    assert_eq!(native.backend.property(vector, PropKey::new(drawing::CANVAS_VIEW)).as_integer(), Some(1), "view-only transform advances view");
    let resolved = canvas::resolve(CanvasToken(token)).expect("live scene");
    let guard = resolved.lock().expect("lock scene");
    assert_eq!(guard.shapes().len(), 3);
    let linear_fill = guard.shapes()[0].fill.as_ref().expect("linear fill");
    match &linear_fill.paint {
        canvas::Paint::Gradient(grad) => {
            assert!(!grad.repeating);
            assert_eq!(grad.stops.len(), 3);
            assert!(matches!(grad.kind, canvas::GradientKind::Linear { .. }));
        }
        other => panic!("expected linear gradient paint, got {other:?}"),
    }
    let dashed = guard.shapes()[1].stroke.as_ref().expect("dashed stroke");
    assert_eq!(dashed.style.dash_pattern.as_slice(), &[8.0, 4.0]);
    assert!((dashed.style.dash_offset - (1_000_004.0f64).rem_euclid(12.0)).abs() < 1e-6);
    assert!(matches!(dashed.paint, canvas::Paint::Gradient(_)));
    let radial_fill = guard.shapes()[1].fill.as_ref().expect("radial fill");
    match &radial_fill.paint {
        canvas::Paint::Gradient(grad) => {
            assert!(grad.repeating);
            assert!(matches!(grad.kind, canvas::GradientKind::Radial { radius_x, radius_y, .. } if radius_x == 40.0 && radius_y == 40.0));
        }
        other => panic!("expected radial gradient paint, got {other:?}"),
    }
    let solid_empty_dash = guard.shapes()[2].stroke.as_ref().expect("empty dash stroke");
    assert!(solid_empty_dash.style.dash_pattern.is_empty());
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
        HostPort::queued(),
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
    let foreign = UiBridge::new(NodeRegistry::new(native.dom.clone(), root_handle(&native), 38, 1, 100), HostHandle::new(StubHost::default()), HostPort::queued());
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

#[test]
fn stylex_aliases_keep_internal_classes_and_retain_previous_application_on_conflict() {
    let mut native = NativeProperties::with_options(BridgeOptions::default());
    native.script("__zgui.setProperty(node, '__zguiClass', 'sdk invariant'); __zgui.setProperty(node, 'className', 'app'); __zgui.setProperty(node, 'class', '  '); 'aliases'");
    assert_eq!(native.backend.classes(native.node), ["sdk", "invariant", "app"].map(zgui::view::ClassName::new));
    native.script("__zgui.setProperty(node, 'class', 'conflict'); __zgui.setProperty(node, 'title', 'unrelated'); __zgui.setProperty(node, '__zguiClass', 'sdk changed'); 'conflict'");
    assert_eq!(native.backend.classes(native.node), ["sdk", "changed", "app"].map(zgui::view::ClassName::new));
    assert_eq!(native.attribute("title").as_deref(), Some("unrelated"));
    assert_eq!(native.bridge.take_style_diagnostics()[0].code, "CLASS_ALIAS_CONFLICT");
    native.script("__zgui.setProperty(node, 'className', null); 'switch'");
    assert_eq!(native.backend.classes(native.node), ["sdk", "changed", "conflict"].map(zgui::view::ClassName::new));
}

#[test]
fn alias_updates_reconcile_once_at_turn_end() {
    let mut native = NativeProperties::with_options(BridgeOptions::default());
    native.script("__zgui.setProperty(node, 'className', null); __zgui.setProperty(node, 'class', 'old'); 'initial'");
    native.script("__zgui.setProperty(node, 'class', 'new'); __zgui.setProperty(node, 'className', 'other'); __zgui.setProperty(node, 'title', 'unrelated'); 'conflict'");
    assert_eq!(native.backend.classes(native.node), ["old"].map(zgui::view::ClassName::new));
    assert_eq!(native.attribute("title").as_deref(), Some("unrelated"));
    assert_eq!(native.bridge.take_style_diagnostics()[0].code, "CLASS_ALIAS_CONFLICT");
    native.script("__zgui.setProperty(node, 'className', null); 'switch'");
    assert_eq!(native.backend.classes(native.node), ["new"].map(zgui::view::ClassName::new));
    native.script("__zgui.setProperty(node, 'className', 'temporary'); __zgui.setProperty(node, 'className', null); 'transient'");
    assert_eq!(native.backend.classes(native.node), ["new"].map(zgui::view::ClassName::new));
    assert!(native.bridge.take_style_diagnostics().is_empty(), "clearing the alias in the same turn must not conflict");
}

#[test]
fn inline_transactions_validate_numbers_grammar_and_ownership_before_committing() {
    let mut native = NativeProperties::with_options(BridgeOptions::default());
    native.script(r#"
        __zgui.setProperty(node, '__zguiStyle', {width: '10px', '--Accent': 'default'});
        __zgui.setProperty(node, '__zguiInvariantStyle', {position: 'fixed'});
        __zgui.setProperty(node, 'style', {width: '20px', paddingLeft: '8px', opacity: 0, flexGrow: 1, '--Accent': 0, '--accent': 2});
        'initial'
    "#);
    let accepted = native.style();
    assert!(accepted.1.contains("opacity: 0"));
    assert!(accepted.1.contains("--Accent: 0"));
    assert!(accepted.1.contains("--accent: 2"));
    assert!(accepted.1.contains("padding-left: 8px"));
    for invalid in ["{height: '30px', width: 0}", "{height: '30px', opacity: NaN}", "{height: '30px', '--x': Infinity}", "{height: '30px', width: 'NaNpx'}", "{height: '30px', width: false}", "{height: '30px', width: {}}", "{height: '30px', width: '10px; color: red'}", "'height:30px;width:invalid'"] {
        native.script(&format!("__zgui.setProperty(node, 'style', {invalid}); __zgui.setProperty(node, 'title', 'still live'); 'invalid'"));
        assert_eq!(native.style(), accepted, "{invalid}");
        assert_eq!(native.attribute("title").as_deref(), Some("still live"));
        assert!(!native.bridge.take_style_diagnostics().is_empty());
    }
    native.script("__zgui.setProperty(node, 'style', 'width: 40px; position: relative !important; --Text: 7'); 'string'");
    assert!(native.style().1.contains("width: 40px"));
    assert!(native.style().1.contains("position: fixed !important"));
    assert!(!native.style().0.contains("opacity"));
    native.script("__zgui.setProperty(node, 'style', {padding: '1px', paddingLeft: '8px'}); __zgui.setProperty(node, 'style', {paddingLeft: '8px'}); 'object'");
    assert!(native.style().1.contains("padding-left: 8px"));
    assert!(native.style().1.contains("width: 10px"));
    assert!(native.style().1.contains("--Accent: default"));
    assert!(!native.style().0.contains("--Text"));
    native.script("__zgui.setProperty(node, 'style', null); 'clear'");
    assert_eq!(native.style().0, BTreeSet::from(["width".into(), "--Accent".into(), "position".into()]));
}

#[test]
fn invalid_visual_properties_preserve_inline_style() {
    let mut native = NativeProperties::with_options(BridgeOptions::default());
    native.script("__zgui.setProperty(node, 'style', {height: '11px'}); 'set'");
    let previous = native.style();
    for invalid in ["boxShadow: '1px'", "backdropFilter: 'blur(red)'",
        r#"fontVariationSettings: '"weight" 650'"#, "translate: 0", "rotate: 0", "filter: 0", "scale: Infinity"] {
        native.script(&format!("__zgui.setProperty(node, 'style', {{ width: '10px', {invalid} }}); 'invalid'"));
        assert_eq!(native.style(), previous, "{invalid}");
        assert!(!native.bridge.take_style_diagnostics().is_empty(), "{invalid}");
    }
}

#[test]
fn finite_unitless_numbers_are_admitted_without_length_coercion() {
    let mut native = NativeProperties::new();
    for (name, value) in [("opacity", "0"), ("flexGrow", "1"), ("flexShrink", "0"), ("fontWeight", "700"), ("lineHeight", "1.5"), ("zIndex", "2"), ("order", "1"), ("animationIterationCount", "3"), ("scale", "1.25")] {
        native.script(&format!("__zgui.setProperty(node, 'style', {{{name}: {value}}}); 'unitless'"));
        let (keys, style) = native.style();
        assert_eq!(keys.len(), 1, "{name}: {value}");
        if name == "scale" {
            assert!(style.contains("scale: 1.25"), "actual style: {style}");
        }
    }
}
