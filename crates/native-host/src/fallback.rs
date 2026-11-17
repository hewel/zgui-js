//! Rust-owned mount-failure UI, independent of the retired JavaScript session.

use std::{cell::Cell, rc::Rc};
use zgui::{
    view::{AttrName, ClassName, DomHandle, ElementName, ListenerId, NodeId, Semantics, UiState},
    vocab::{EventKind, ListenerOptions, Role, SemanticFlags},
};

use crate::session_host::MountRegion;

const SHEET_NAME: &str = "host-fallback";
const RETRY_LABEL: &str = "Retry last successful build";
const NO_PRIOR_BUILD: &str = "No prior successful build is available to retry.";
const SHEET: &str = r#"
.zgui-host-fallback {
    box-sizing: border-box;
    width: 100%;
    padding: 24px;
    gap: 16px;
    align-items: flex-start;
    background: #181b20;
    color: #f2f4f8;
    font-family: sans-serif;
    font-size: 16px;
}
.zgui-host-fallback .zgui-host-fallback__heading {
    font-size: 24px;
    font-weight: 700;
}
.zgui-host-fallback .zgui-host-fallback__diagnostic {
    max-width: 100%;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
}
.zgui-host-fallback .zgui-host-fallback__retry {
    padding: 10px 16px;
    border: 2px solid transparent;
    border-radius: 6px;
    background: #2b6cff;
    color: #ffffff;
}
.zgui-host-fallback .zgui-host-fallback__retry:focus {
    border-color: #ffffff;
}
.zgui-host-fallback .zgui-host-fallback__retry:disabled {
    background: #454b56;
    color: #c6cbd4;
}
"#;

struct Retry {
    active: Cell<bool>,
    callback: Rc<dyn Fn()>,
}

pub(crate) struct Fallback {
    region: MountRegion,
    tree: Option<NodeId>,
    listener: Option<(NodeId, ListenerId)>,
    retry: Option<Rc<Retry>>,
}

impl Fallback {
    pub(crate) fn new(region: MountRegion, diagnostic: &str, retry: Option<Rc<dyn Fn()>>) -> Self {
        let dom = &region.dom;
        let tree = dom.create_element(ElementName::new("column"));
        dom.set_classes(tree, &[ClassName::new("zgui-host-fallback")]);
        dom.set_semantics(tree, Some(&Semantics {
            role: Role::Group,
            label: Some("Application error".into()),
            ..Semantics::default()
        }));
        text(dom, tree, "zgui-host-fallback__heading", "Application error");
        // create_text copies the diagnostic into the native document; no JS or
        // borrowed error data survives construction.
        text(dom, tree, "zgui-host-fallback__diagnostic", diagnostic);

        let button = dom.create_element(ElementName::new("control"));
        dom.set_classes(button, &[ClassName::new("zgui-host-fallback__retry")]);
        dom.insert(button, dom.create_text(RETRY_LABEL), None);
        let disabled = retry.is_none();
        dom.set_attribute(button, AttrName::new("tabindex"), Some(if disabled { "-1" } else { "0" }));
        dom.set_ui_state(button, UiState::DISABLED, disabled);
        dom.set_semantics(button, Some(&Semantics {
            role: Role::Button,
            label: Some(RETRY_LABEL.into()),
            description: disabled.then(|| NO_PRIOR_BUILD.into()),
            flags: if disabled { SemanticFlags::DISABLED } else { SemanticFlags::default() },
            ..Semantics::default()
        }));
        dom.insert(tree, button, None);
        if disabled {
            text(dom, tree, "zgui-host-fallback__unavailable", NO_PRIOR_BUILD);
        }

        let retry = retry.map(|callback| Rc::new(Retry { active: Cell::new(true), callback }));
        let listener = retry.as_ref().map(|retry| {
            // Only this view owns the registration root, even when the caller
            // retains its callback. Revocation also covers handlers cloned by
            // native dispatch before clear, including reentrant dispatch.
            let retry = Rc::downgrade(retry);
            let registration = dom.add_listener(button, EventKind::Click, ListenerOptions::DEFAULT, Rc::new(move |_| {
                if let Some(retry) = retry.upgrade() && retry.active.get() { (retry.callback)(); }
            }));
            (button, registration)
        });
        region.host.install_stylesheet(SHEET_NAME, SHEET);
        dom.insert(region.root, tree, None);
        Self { region, tree: Some(tree), listener, retry }
    }

    pub(crate) fn clear(&mut self) {
        let Some(tree) = self.tree.take() else { return; };
        if let Some((node, registration)) = self.listener.take() {
            self.region.dom.remove_listener(node, registration);
        }
        if let Some(retry) = self.retry.take() { retry.active.set(false); }
        self.region.dom.detach(tree);
        // Taking tree is also the sheet-ownership guard: clearing or dropping an
        // old view again must not remove a replacement view's named stylesheet.
        self.region.host.remove_stylesheet(SHEET_NAME);
    }
}

impl Drop for Fallback {
    fn drop(&mut self) { self.clear(); }
}

fn text(dom: &DomHandle, parent: NodeId, class: &str, value: &str) {
    let element = dom.create_element(ElementName::new("text"));
    dom.set_classes(element, &[ClassName::new(class)]);
    dom.insert(element, dom.create_text(value), None);
    dom.insert(parent, element, None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use zgui::{
        geom::{CssPx, Point},
        view::{DiscardCommands, EventControl, EventCx, HostHandle, host::ViewHost, stub::StubHost},
        vocab::{Modifiers, Payload, Phase, PointerEvent, Timestamp},
    };
    use zgui_dom::Document;
    use zgui_view_dom::DocumentDom;

    struct Native {
        backend: Rc<DocumentDom>,
        dom: DomHandle,
        host: Rc<StubHost>,
        root: NodeId,
        sibling: NodeId,
    }

    impl Native {
        fn new() -> Self {
            let backend = Rc::new(DocumentDom::new(Rc::new(RefCell::new(Document::new()))));
            let dom = DomHandle::from_rc(backend.clone());
            let root = dom.create_element(ElementName::new("column"));
            dom.insert(backend.root_node(), root, None);
            let sibling = dom.create_element(ElementName::new("text"));
            dom.insert(root, sibling, None);
            let host = Rc::new(StubHost::default());
            Self { backend, dom, host, root, sibling }
        }

        fn region(&self) -> MountRegion {
            MountRegion {
                document: self.backend.document_id(),
                root: self.root,
                dom: self.dom.clone(),
                host: HostHandle::from_rc(self.host.clone()),
            }
        }

        fn handler(&self, node: NodeId) -> Rc<dyn Fn(&mut EventCx<'_>)> {
            let id = {
                let document = self.backend.document();
                let key = document.store().key_of(self.backend.index_of(node));
                let listeners = document.store().columns().listeners.get(key).unwrap();
                assert_eq!(listeners.len(), 1);
                let listener = listeners.iter().next().unwrap();
                assert_eq!(listener.kind, EventKind::Click);
                listener.id
            };
            self.backend.handler(id).unwrap()
        }
    }

    fn click(node: NodeId, handler: &dyn Fn(&mut EventCx<'_>)) {
        let payload = Payload::Pointer(PointerEvent::mouse(Point::new(CssPx(1.0), CssPx(2.0))));
        let control = EventControl::new();
        let mut commands = DiscardCommands;
        let mut event = EventCx::new(EventKind::Click, node, node, Phase::Target, Modifiers::NONE,
            Timestamp::ORIGIN, &payload, &control, &mut commands);
        handler(&mut event);
    }

    #[test]
    fn disabled_retry_owns_diagnostic_and_preserves_the_protected_root() {
        let native = Native::new();
        native.dom.set_attribute(native.root, AttrName::new("id"), Some("protected"));
        native.host.install_stylesheet("application", ".app { color: red }");
        let diagnostic = String::from("Mount failed:\nowned error details");
        let mut fallback = Fallback::new(native.region(), &diagnostic, None);
        drop(diagnostic);
        let tree = fallback.tree.unwrap();
        let children = native.backend.children(tree);
        assert_eq!(native.backend.text_content(children[1]), "Mount failed:\nowned error details");
        assert!(native.backend.text_content(tree).contains(NO_PRIOR_BUILD));
        let button = children[2];
        assert!(native.backend.ui_state(button).contains(UiState::DISABLED));
        let semantics = native.backend.semantics(button).unwrap();
        assert_eq!(semantics.role, Role::Button);
        assert!(semantics.flags.contains(SemanticFlags::DISABLED));
        assert_eq!(semantics.description.as_deref(), Some(NO_PRIOR_BUILD));
        assert_eq!(native.backend.handler_count(), 0);
        fallback.clear();
        fallback.clear();
        drop(fallback);
        assert_eq!(native.backend.children(native.root), [native.sibling]);
        assert_eq!(native.backend.attribute(native.root, AttrName::new("id")).as_deref(), Some("protected"));
        assert_eq!(native.dom.parent(native.root), Some(native.backend.root_node()));
        assert_eq!(native.dom.parent(tree), None);
        assert!(native.host.stylesheet(SHEET_NAME).is_none());
        assert_eq!(native.host.stylesheet("application").as_deref(), Some(".app { color: red }"));
    }

    #[test]
    fn repeated_clear_and_drop_do_not_remove_a_replacement_sheet_or_tree() {
        let native = Native::new();
        let mut first = Fallback::new(native.region(), "first", None);
        first.clear();
        let second = Fallback::new(native.region(), "second", Some(Rc::new(|| {})));
        let tree = second.tree.unwrap();
        first.clear();
        drop(first);
        assert_eq!(native.backend.children(native.root), [native.sibling, tree]);
        assert_eq!(native.host.stylesheet(SHEET_NAME).as_deref(), Some(SHEET));
        assert_eq!(native.backend.handler_count(), 1);
        drop(second);
        assert_eq!(native.backend.children(native.root), [native.sibling]);
        assert_eq!(native.backend.handler_count(), 0);
        assert!(native.host.stylesheet(SHEET_NAME).is_none());
    }

    #[test]
    fn clear_revokes_cloned_native_handler_even_when_caller_retains_callback() {
        let native = Native::new();
        let calls = Rc::new(Cell::new(0));
        let called = calls.clone();
        let retry: Rc<dyn Fn()> = Rc::new(move || called.set(called.get() + 1));
        let mut fallback = Fallback::new(native.region(), "failure", Some(retry.clone()));
        let button = native.backend.children(fallback.tree.unwrap())[2];
        let handler = native.handler(button);
        click(button, &*handler);
        assert_eq!(calls.get(), 1);
        fallback.clear();
        click(button, &*handler);
        assert_eq!(calls.get(), 1);
        retry();
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn native_click_can_clear_reentrantly_and_stale_handlers_do_not_retain_retry() {
        let native = Native::new();
        let owner = Rc::new(RefCell::new(None::<Fallback>));
        let weak_owner = Rc::downgrade(&owner);
        let calls = Rc::new(Cell::new(0));
        let called = calls.clone();
        let retained = Rc::new(());
        let weak_retained = Rc::downgrade(&retained);
        let dom = native.dom.clone();
        let sibling = native.sibling;
        let retry: Rc<dyn Fn()> = Rc::new(move || {
            let _retained = &retained;
            called.set(called.get() + 1);
            dom.set_attribute(sibling, AttrName::new("title"), Some("retried"));
            weak_owner.upgrade().unwrap().borrow_mut().as_mut().unwrap().clear();
        });
        let fallback = Fallback::new(native.region(), "failure", Some(retry));
        let button = native.backend.children(fallback.tree.unwrap())[2];
        assert_eq!(native.backend.attribute(button, AttrName::new("tabindex")).as_deref(), Some("0"));
        assert!(!native.backend.ui_state(button).contains(UiState::DISABLED));
        assert_eq!(native.backend.semantics(button).unwrap().role, Role::Button);
        let handler = native.handler(button);
        *owner.borrow_mut() = Some(fallback);
        click(button, &*handler);
        assert_eq!(calls.get(), 1);
        assert!(weak_retained.upgrade().is_none());
        assert_eq!(native.backend.handler_count(), 0);
        assert_eq!(native.backend.attribute(sibling, AttrName::new("title")).as_deref(), Some("retried"));
        assert_eq!(native.backend.children(native.root), [sibling]);
        click(button, &*handler);
        assert_eq!(calls.get(), 1);
    }
}
