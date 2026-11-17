//! Adapter-boundary tests. StubHost records value loads; native caret, IME, and
//! user editing behavior require the runtime integration tests, not a second model here.
use std::{cell::RefCell, rc::Rc};

use js_runtime::{JsRuntime, RuntimeLimits};
use ui_bridge::{UiBridge, registry::NodeRegistry};
use zgui::{
    view::{AttrName, DiscardCommands, DomHandle, ElementName, EventControl, EventCx, HostHandle, NodeId, UiState, stub::StubHost},
    vocab::{EventKind, Modifiers, Payload, Phase, Role, SemanticFlags, Timestamp, ValueChange, ValueEvent},
};
use zgui_dom::Document;
use zgui_view_dom::DocumentDom;

struct Editing {
    // Release persistent wrappers before destroying their engine, including on unwind.
    bridge: Rc<UiBridge>,
    engine: JsRuntime,
    backend: Rc<DocumentDom>,
    dom: DomHandle,
    host: Rc<StubHost>,
    root: NodeId,
    node: NodeId,
}

impl Editing {
    fn new(tag: &str) -> Self {
        let backend = Rc::new(DocumentDom::new(Rc::new(RefCell::new(Document::new()))));
        let dom = DomHandle::from_rc(backend.clone());
        let root = dom.create_element(ElementName::new("column"));
        dom.insert(backend.root_node(), root, None);
        let host = Rc::new(StubHost::default());
        let mut engine = JsRuntime::new(RuntimeLimits::default()).unwrap();
        let bridge = UiBridge::new(NodeRegistry::new(dom.clone(), root, 61, 1, 100), HostHandle::from_rc(host.clone()));
        engine.with_context(|ctx| {
            bridge.install(&ctx)?;
            ctx.globals().set("root", bridge.root(&ctx)?)
        }).unwrap();
        bridge.run_turn(|| engine.evaluate_script(&format!(
            "const node = __zgui.createElement('{tag}'); __zgui.insertNode(root, node, null); 'mounted'"
        ))).unwrap();
        let node = backend.children(root)[0];
        Self { bridge, engine, backend, dom, host, root, node }
    }

    fn script(&mut self, script: &str) {
        self.bridge.run_turn(|| self.engine.evaluate_script(script)).unwrap();
    }

    fn value(&self) -> Option<String> { self.host.value(self.node) }

    fn internal_text(&self) -> NodeId {
        let span = self.dom.create_element(ElementName::new("text"));
        let text = self.dom.create_text("native-owned content");
        self.dom.insert(self.node, span, None);
        self.dom.insert(span, text, None);
        text
    }
}

#[test]
fn defaults_initialize_once_and_explicit_values_win_in_either_property_order() {
    for tag in ["field", "editor"] {
        let mut native = Editing::new(tag);
        native.script("__zgui.setProperty(node, 'defaultValue', 'initial'); 'default'");
        assert_eq!(native.value().as_deref(), Some("initial"));
        native.script("__zgui.setProperty(node, 'defaultValue', 'later'); __zgui.setProperty(node, 'defaultValue', null); 'ignored'");
        assert_eq!(native.value().as_deref(), Some("initial"));
        native.script("__zgui.setProperty(node, 'value', 'é🙂'); __zgui.setProperty(node, 'defaultValue', 'fallback'); 'controlled'");
        assert_eq!(native.value().as_deref(), Some("é🙂"));
        for empty in ["null", "undefined"] {
            native.script(&format!("__zgui.setProperty(node, 'value', 'nonempty'); __zgui.setProperty(node, 'value', {empty}); __zgui.setProperty(node, 'defaultValue', 'revive'); 'clear'"));
            assert_eq!(native.value().as_deref(), Some(""));
        }
        // The adapter must not realize values by authoring native text children.
        assert!(native.backend.children(native.node).is_empty());

        let mut explicit_first = Editing::new(tag);
        explicit_first.script("__zgui.setProperty(node, 'value', 'explicit'); __zgui.setProperty(node, 'defaultValue', 'default'); 'explicit first'");
        assert_eq!(explicit_first.value().as_deref(), Some("explicit"));

        let mut cleared_first = Editing::new(tag);
        cleared_first.script("__zgui.setProperty(node, 'value', undefined); __zgui.setProperty(node, 'defaultValue', 'default'); 'cleared first'");
        assert_eq!(cleared_first.value().as_deref(), Some(""));

        let mut empty_default = Editing::new(tag);
        empty_default.script("__zgui.setProperty(node, 'defaultValue', null); __zgui.setProperty(node, 'defaultValue', 'too late'); 'empty default'");
        assert_eq!(empty_default.value().as_deref(), Some(""));
    }
}

#[test]
fn editing_properties_reject_noneditable_nodes_and_wrong_types_without_native_changes() {
    let mut native = Editing::new("field");
    native.script(r#"
        const ordinary = __zgui.createElement('control');
        const text = __zgui.createTextNode('ordinary text');
        for (const target of [root, ordinary, text]) {
            for (const [name, value] of [['value', 'bad'], ['defaultValue', 'bad'], ['readOnly', true], ['value', null], ['defaultValue', undefined], ['readOnly', null]]) {
                let rejected = false;
                try { __zgui.setProperty(target, name, value); }
                catch (error) { rejected = error instanceof TypeError && error.message.includes('require a field or editor'); }
                if (!rejected) throw Error('editing property accepted on noneditable node');
            }
        }
        for (const [name, value] of [['value', 1], ['value', false], ['defaultValue', {}], ['readOnly', 'false'], ['readOnly', 0]]) {
            let rejected = false;
            try { __zgui.setProperty(node, name, value); } catch (error) { rejected = error instanceof TypeError; }
            if (!rejected) throw Error('invalid editing property accepted');
        }
        'rejected'
    "#);
    assert_eq!(native.value(), None);
    assert!(!native.backend.ui_state(native.node).contains(UiState::READ_ONLY));
    assert_eq!(native.backend.semantics(native.node), None);
    native.script("__zgui.setProperty(node, 'defaultValue', 'valid after rejection'); 'valid'");
    assert_eq!(native.value().as_deref(), Some("valid after rejection"));
}

#[test]
fn editor_owned_descendants_stay_opaque_through_moves_and_are_reclaimed_with_the_control() {
    for tag in ["field", "editor"] {
        let mut native = Editing::new(tag);
        let internal = native.internal_text();
        let native_children = native.backend.children(native.node);
        native.script(r#"
            const authored = __zgui.createTextNode('authored');
            const element = __zgui.createElement('box');
            __zgui.insertNode(root, authored, null);
            __zgui.insertNode(root, element, null);
            for (const child of [authored, element]) {
                let rejected = false;
                try { __zgui.insertNode(node, child, null); }
                catch (error) { rejected = error.message.includes('cannot contain reconciled children'); }
                if (!rejected || __zgui.getParentNode(child) !== root) throw Error('child moved into editable control');
            }
            if (__zgui.getFirstChild(node) != null || __zgui.isTextNode(node)) throw Error('native internals exposed');
            let rejected = false;
            try { __zgui.replaceText(node, 'overwrite internals'); } catch { rejected = true; }
            if (!rejected) throw Error('control accepted text replacement');
            __zgui.removeNode(root, node);
            __zgui.insertNode(root, node, element);
            if (__zgui.getFirstChild(node) != null || __zgui.getNextSibling(node) !== element) throw Error('move changed opacity');
            'moved'
        "#);
        assert_eq!(native.backend.children(native.node), native_children);
        assert!(native.dom.is_live(internal));
        assert_eq!(native.bridge.live_count(), 3);
        assert_eq!(native.bridge.wrapper_count(), 4);
        native.script("__zgui.removeNode(root, node); 'removed'");
        assert_eq!(native.bridge.live_count(), 2);
        assert_eq!(native.bridge.wrapper_count(), 3);
        assert!(native.dom.is_live(internal));
        native.script(r#"
            let stale = false;
            try { __zgui.setProperty(node, 'defaultValue', 'late'); } catch (error) { stale = error.message.includes('stale'); }
            if (!stale) throw Error('invalidated editor accepted a value');
            const replacement = __zgui.createElement('editor');
            __zgui.insertNode(root, replacement, null);
            __zgui.setProperty(replacement, 'defaultValue', 'fresh');
            'replaced'
        "#);
        let replacement = *native.backend.children(native.root).last().unwrap();
        assert_eq!(native.host.value(replacement).as_deref(), Some("fresh"));
        native.backend.end_frame();
        assert!(!native.dom.is_live(native.node));
        assert!(!native.dom.is_live(internal));
        native.bridge.dispose();
        native.bridge.dispose();
        native.backend.end_frame();
        assert!(native.backend.children(native.root).is_empty());
        assert_eq!(native.bridge.wrapper_count(), 0);
    }
}

#[test]
fn readonly_updates_merge_with_live_accessibility_and_disabled_state_independently() {
    for (tag, role, native_role) in [("field", "textInput", Role::TextInput), ("editor", "multilineTextInput", Role::MultilineTextInput)] {
        let mut native = Editing::new(tag);
        native.dom.set_ui_state(native.node, UiState::HOVER, true);
        native.script(&format!("__zgui.setProperty(node, 'role', '{role}'); __zgui.setProperty(node, 'ariaLabel', 'Draft'); __zgui.setProperty(node, 'ariaDescription', 'Editable notes'); __zgui.setProperty(node, 'disabled', true); __zgui.setProperty(node, 'readOnly', true); 'readonly'"));
        let state = native.backend.ui_state(native.node);
        assert!(state.contains(UiState::READ_ONLY | UiState::DISABLED | UiState::HOVER));
        let semantics = native.backend.semantics(native.node).unwrap();
        assert_eq!(semantics.role, native_role);
        assert_eq!(semantics.label.as_deref(), Some("Draft"));
        assert!(semantics.flags.contains(SemanticFlags::READ_ONLY | SemanticFlags::DISABLED));
        for empty in ["false", "null", "undefined"] {
            native.script(&format!("__zgui.setProperty(node, 'readOnly', true); __zgui.setProperty(node, 'ariaLabel', 'Updated'); __zgui.setProperty(node, 'readOnly', {empty}); 'writable'"));
            let state = native.backend.ui_state(native.node);
            assert!(!state.contains(UiState::READ_ONLY));
            assert!(state.contains(UiState::DISABLED | UiState::HOVER));
            let semantics = native.backend.semantics(native.node).unwrap();
            assert!(!semantics.flags.contains(SemanticFlags::READ_ONLY));
            assert!(semantics.flags.contains(SemanticFlags::DISABLED));
            assert_eq!(semantics.role, native_role);
            assert_eq!(semantics.label.as_deref(), Some("Updated"));
            assert_eq!(semantics.description.as_deref(), Some("Editable notes"));
        }
        native.script("__zgui.setProperty(node, 'readOnly', true); __zgui.setProperty(node, 'disabled', false); __zgui.setProperty(node, 'role', null); __zgui.setProperty(node, 'ariaLabel', null); __zgui.setProperty(node, 'ariaDescription', undefined); 'readonly only'");
        assert_eq!(native.backend.semantics(native.node).unwrap().flags, SemanticFlags::READ_ONLY);
        native.script("__zgui.setProperty(node, 'readOnly', undefined); 'clear final'");
        assert_eq!(native.backend.semantics(native.node), None);
        assert_eq!(native.backend.ui_state(native.node), UiState::HOVER);
    }
}

#[test]
fn native_internal_input_and_change_targets_retarget_without_losing_listener_current_target() {
    for tag in ["field", "editor"] {
        let mut native = Editing::new(tag);
        let internal = native.internal_text();
        let dispatched = Rc::new(RefCell::new(Vec::new()));
        let ids = dispatched.clone();
        native.bridge.set_dispatcher(Rc::new(move |id, _| ids.borrow_mut().push(id)));
        native.script(r#"
            const received = [];
            __zgui.setProperty(node, 'onInput', event => {
                if (event.target !== node || event.currentTarget !== node) throw Error('control retarget');
                received.push(event);
            });
            __zgui.setProperty(root, 'onChange', event => {
                if (event.target !== node || event.currentTarget !== root) throw Error('ancestor retarget');
                received.push(event);
            });
            'listening'
        "#);
        let wrapper_count = native.bridge.wrapper_count();
        for (kind, current, phase, change) in [
            (EventKind::Input, native.node, Phase::Target, ValueChange::Input),
            (EventKind::Change, native.root, Phase::Bubble, ValueChange::Committed),
        ] {
            let listener = {
                let document = native.backend.document();
                let store = document.store();
                let key = store.key_of(native.backend.index_of(current));
                *store.columns().listeners.get(key).unwrap().iter().find(|listener| listener.kind == kind).unwrap()
            };
            let payload = Payload::Value(ValueEvent { value: "é🙂".into(), selection: 2..6, kind: change });
            let control = EventControl::new();
            let mut sink = DiscardCommands;
            let mut event = EventCx::new(kind, internal, current, phase, Modifiers::NONE, Timestamp::ORIGIN, &payload, &control, &mut sink);
            native.backend.handler(listener.id).unwrap()(&mut event);
            let id = dispatched.borrow_mut().pop().unwrap();
            let bridge = native.bridge.clone();
            bridge.run_turn(|| native.engine.with_context(|ctx| bridge.invoke_event(&ctx, id, &mut event))).unwrap();
        }
        assert_eq!(native.bridge.wrapper_count(), wrapper_count, "internal event target fabricated a wrapper");
        assert_eq!(native.bridge.live_count(), 1);
        native.script(r#"
            if (received.length !== 2 || received[0].type !== 'input' || received[1].type !== 'change') throw Error('event kinds');
            if (received[0].valueChange !== 'input' || received[1].valueChange !== 'committed') throw Error('value change kinds');
            for (const event of received) {
                if (event.value !== 'é🙂' || event.selection.start !== 2 || event.selection.end !== 6) throw Error('snapshot lost UTF-8 selection');
            }
            if (__zgui.getFirstChild(node) != null) throw Error('event exposed internals');
            'retargeted'
        "#);
    }
}

#[test]
fn placeholder_stays_inside_the_editing_boundary_and_preserves_semantics() {
    for tag in ["field", "editor"] {
        let mut native = Editing::new(tag);
        native.script(r#"
            __zgui.setProperty(node, 'placeholder', 'Search…');
            __zgui.setProperty(node, 'placeholder', null);
            __zgui.setProperty(node, 'placeholder', 'Updated');
            const ordinary = __zgui.createElement('control');
            const text = __zgui.createTextNode('x');
            __zgui.insertNode(root, ordinary, null);
            __zgui.insertNode(root, text, null);
            for (const target of [ordinary, text]) {
                let rejected = false;
                try { __zgui.setProperty(target, 'placeholder', 'bad'); }
                catch (error) { rejected = error instanceof TypeError && error.message.includes('require a field or editor'); }
                if (!rejected) throw Error('placeholder accepted on a non-editable node');
            }
            'placeholder'
        "#);
        let semantics = native.backend.semantics(native.node).unwrap();
        assert_eq!(semantics.placeholder.as_deref(), Some("Updated"));
        native.script("__zgui.setProperty(node, 'placeholder', undefined); 'cleared'");
        assert_eq!(native.backend.semantics(native.node).and_then(|semantics| semantics.placeholder), None);
    }
}

#[test]
fn secure_is_a_field_only_native_attribute_in_either_write_order() {
    for order in ["secure_first", "value_first"] {
        let mut native = Editing::new("field");
        let script = if order == "secure_first" {
            "__zgui.setProperty(node, 'secure', true); __zgui.setProperty(node, 'defaultValue', 'synthetic'); 'secure then value'"
        } else {
            "__zgui.setProperty(node, 'defaultValue', 'synthetic'); __zgui.setProperty(node, 'secure', true); 'value then secure'"
        };
        native.script(script);
        assert_eq!(native.backend.attribute(native.node, AttrName::new("secure")).as_deref(), Some(""));
        assert_eq!(native.value().as_deref(), Some("synthetic"));
        native.script(r#"
            __zgui.setProperty(node, 'secure', null);
            __zgui.setProperty(node, 'secure', true);
            __zgui.setProperty(node, 'secure', false);
            'cleared'
        "#);
        assert_eq!(native.backend.attribute(native.node, AttrName::new("secure")), None);
        assert_eq!(native.value().as_deref(), Some("synthetic"));
        native.script(r#"
            for (const bad of ['yes', 1, {}]) {
                let rejected = false;
                try { __zgui.setProperty(node, 'secure', bad); } catch (error) { rejected = error instanceof TypeError; }
                if (!rejected) throw Error('secure accepted a non-boolean value');
            }
            'strict'
        "#);
        assert_eq!(native.backend.attribute(native.node, AttrName::new("secure")), None);
    }
    for (tag, message) in [("editor", "secure requires a field"), ("control", "secure requires a field")] {
        let mut native = Editing::new(tag);
        native.script(&format!(r#"
            let rejected = false;
            try {{ __zgui.setProperty(node, 'secure', true); }}
            catch (error) {{ rejected = error instanceof TypeError && error.message.includes('{message}'); }}
            if (!rejected) throw Error('secure accepted on a {tag}');
            let cleared = false;
            try {{ __zgui.setProperty(node, 'secure', null); }}
            catch (error) {{ cleared = error instanceof TypeError && error.message.includes('{message}'); }}
            if (!cleared) throw Error('secure clear accepted on a {tag}');
            'rejected'
        "#));
        assert_eq!(native.backend.attribute(native.node, AttrName::new("secure")), None);
    }
}

#[test]
fn focus_moves_through_the_host_and_rejects_stale_or_foreign_wrappers() {
    use zgui::{reactive::prelude::GetUntracked, view::ViewHost};
    let mut native = Editing::new("field");
    native.script("__zgui.focus(node); 'focused'");
    assert_eq!(native.host.focused().get_untracked(), Some(native.node));

    let foreign = UiBridge::new(
        NodeRegistry::new(native.dom.clone(), native.root, 61, 2, 10),
        HostHandle::from_rc(native.host.clone()),
    );
    native.engine.with_context(|ctx| {
        ctx.globals().set("foreign", foreign.root(&ctx)?)
    }).unwrap();
    native.script(r#"
        let rejected = false;
        try { __zgui.focus(foreign); }
        catch (error) { rejected = error instanceof TypeError && error.message.includes('generation'); }
        if (!rejected) throw Error('focus accepted a foreign generation wrapper');
        const forged = Object.assign(Object.create(Object.getPrototypeOf(node)), node);
        let fabricated = false;
        try { __zgui.focus(forged); } catch { fabricated = true; }
        if (!fabricated) throw Error('focus accepted a fabricated wrapper');
        __zgui.removeNode(root, node);
        'rejected'
    "#);
    assert_eq!(native.host.focused().get_untracked(), Some(native.node));
    native.backend.end_frame();
    native.script(r#"
        let stale = false;
        try { __zgui.focus(node); }
        catch (error) { stale = error instanceof TypeError && error.message.includes('stale'); }
        if (!stale) throw Error('focus accepted a stale handle');
        'stale'
    "#);
    foreign.dispose();
}
