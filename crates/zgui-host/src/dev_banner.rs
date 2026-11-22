//! Dev-only, Rust-owned non-blocking error/status overlay banner.
//!
//! Shown strictly in desktop `--watch` mode when `.zgui-build-status.json`
//! reports build/typecheck failures or when a live reload candidate is
//! rejected. Keeps the last healthy frame visible, executes zero app JS,
//! supports one-click dismissal, and is hard-forbidden outside dev `--watch`
//! or when a `SessionLockSurface` is active.

use std::{cell::Cell, rc::Rc};
use zgui::{
    view::{AttrName, ClassName, DomHandle, ElementName, ListenerId, NodeId, Semantics},
    vocab::{EventKind, ListenerOptions, Role},
};

use crate::session_host::MountRegion;

pub(crate) const SHEET_NAME: &str = "host-dev-banner";
const DISMISS_LABEL: &str = "Dismiss";
const MAX_DETAIL_CHARS: usize = 600;

const SHEET: &str = r#"
.zgui-dev-banner {
    position: fixed;
    left: 14px;
    right: 14px;
    bottom: 14px;
    z-index: 2147483647;
    box-sizing: border-box;
    padding: 12px 16px;
    gap: 8px;
    border-radius: 10px;
    border: 1px solid #f87171;
    background: rgba(24, 15, 19, 0.96);
    color: #fee2e2;
    font-family: sans-serif;
    font-size: 13px;
}
.zgui-dev-banner .zgui-dev-banner__header {
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
}
.zgui-dev-banner .zgui-dev-banner__title {
    font-size: 14px;
    font-weight: 700;
    color: #fca5a5;
}
.zgui-dev-banner .zgui-dev-banner__detail {
    max-width: 100%;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: #fecaca;
}
.zgui-dev-banner .zgui-dev-banner__dismiss {
    padding: 4px 10px;
    border-radius: 6px;
    border: 1px solid #f87171;
    background: #7f1d1d;
    color: #ffffff;
    font-size: 12px;
    font-weight: 600;
}
"#;

struct BannerSlot {
    region: MountRegion,
    tree: Cell<Option<NodeId>>,
    listener: Cell<Option<(NodeId, ListenerId)>>,
    active: Cell<bool>,
}

impl BannerSlot {
    fn clear(&self) {
        let Some(tree) = self.tree.take() else {
            return;
        };
        self.active.set(false);
        if let Some((node, registration)) = self.listener.take() {
            self.region.dom.remove_listener(node, registration);
        }
        self.region.dom.detach(tree);
        self.region.host.remove_stylesheet(SHEET_NAME);
    }
}

pub(crate) struct DevBanner {
    slot: Rc<BannerSlot>,
    signature: String,
}

pub(crate) fn banner_signature(title: &str, detail: &str) -> String {
    format!("{title}\n{}", truncate_detail(detail))
}

impl DevBanner {
    pub(crate) fn new(region: MountRegion, title: &str, detail: &str) -> Self {
        let dom = &region.dom;
        let truncated = truncate_detail(detail);
        let signature = banner_signature(title, detail);

        let tree = dom.create_element(ElementName::new("column"));
        dom.set_classes(tree, &[ClassName::new("zgui-dev-banner")]);
        dom.set_attribute(tree, AttrName::new("data-testid"), Some("zgui-dev-banner"));
        dom.set_semantics(
            tree,
            Some(&Semantics {
                role: Role::Alert,
                label: Some(title.into()),
                ..Semantics::default()
            }),
        );

        let header = dom.create_element(ElementName::new("row"));
        dom.set_classes(header, &[ClassName::new("zgui-dev-banner__header")]);
        text(dom, header, "zgui-dev-banner__title", title);

        let button = dom.create_element(ElementName::new("control"));
        dom.set_classes(button, &[ClassName::new("zgui-dev-banner__dismiss")]);
        dom.set_attribute(button, AttrName::new("tabindex"), Some("0"));
        dom.set_attribute(button, AttrName::new("data-testid"), Some("zgui-dev-banner-dismiss"));
        dom.insert(button, dom.create_text(DISMISS_LABEL), None);
        dom.set_semantics(
            button,
            Some(&Semantics {
                role: Role::Button,
                label: Some(DISMISS_LABEL.into()),
                ..Semantics::default()
            }),
        );
        dom.insert(header, button, None);
        dom.insert(tree, header, None);
        text(dom, tree, "zgui-dev-banner__detail", &truncated);

        let slot = Rc::new(BannerSlot {
            region: region.clone(),
            tree: Cell::new(Some(tree)),
            listener: Cell::new(None),
            active: Cell::new(true),
        });
        let weak_slot = Rc::downgrade(&slot);
        let registration = dom.add_listener(
            button,
            EventKind::Click,
            ListenerOptions::DEFAULT,
            Rc::new(move |_| {
                if let Some(slot) = weak_slot.upgrade()
                    && slot.active.get()
                {
                    slot.clear();
                }
            }),
        );
        slot.listener.set(Some((button, registration)));

        region.host.install_stylesheet(SHEET_NAME, SHEET);
        let container = dom.parent(region.root).unwrap_or(region.root);
        dom.insert(container, tree, None);
        Self { slot, signature }
    }

    pub(crate) fn signature(&self) -> &str {
        &self.signature
    }

    pub(crate) fn is_visible(&self) -> bool {
        self.slot.active.get() && self.slot.tree.get().is_some()
    }

    pub(crate) fn clear(&mut self) {
        self.slot.clear();
    }
}

impl Drop for DevBanner {
    fn drop(&mut self) {
        self.clear();
    }
}

fn truncate_detail(detail: &str) -> String {
    let trimmed = detail.trim();
    if trimmed.chars().count() <= MAX_DETAIL_CHARS {
        return trimmed.to_owned();
    }
    let prefix: String = trimmed.chars().take(MAX_DETAIL_CHARS).collect();
    format!("{prefix}…")
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
    use std::cell::RefCell;
    use zgui::{
        geom::{CssPx, Point},
        view::{DiscardCommands, EventControl, EventCx, HostHandle, stub::StubHost},
        vocab::{Modifiers, Payload, Phase, PointerEvent, Timestamp},
    };
    use zgui_dom::Document;
    use zgui_view_dom::DocumentDom;

    fn make_region() -> (Rc<DocumentDom>, Rc<StubHost>, MountRegion, NodeId) {
        let backend = Rc::new(DocumentDom::new(Rc::new(RefCell::new(Document::new()))));
        let dom = DomHandle::from_rc(backend.clone());
        let root = dom.create_element(ElementName::new("column"));
        dom.insert(backend.root_node(), root, None);
        let app_child = dom.create_element(ElementName::new("text"));
        dom.insert(app_child, dom.create_text("healthy app"), None);
        dom.insert(root, app_child, None);
        let host = Rc::new(StubHost::default());
        let region = MountRegion {
            document: backend.document_id(),
            root,
            dom,
            host: HostHandle::from_rc(host.clone()),
            fonts: zgui::app::Fonts::shipped_only(),
            root_anchor: Rc::new(Cell::new(root)),
        };
        (backend, host, region, app_child)
    }

    #[test]
    fn dev_banner_preserves_healthy_tree_dismisses_on_click_and_cleans_up() {
        let (backend, host, region, app_child) = make_region();
        let banner = DevBanner::new(region.clone(), "Build failed", "src/main.tsx(4,1): TS2322");
        assert!(banner.is_visible());
        assert!(host.stylesheet(SHEET_NAME).is_some());
        // Healthy app root remains untouched.
        assert_eq!(backend.children(region.root), [app_child]);
        let tree = banner.slot.tree.get().unwrap();
        assert!(backend.text_content(tree).contains("Build failed"));
        assert!(backend.text_content(tree).contains("TS2322"));
        assert_eq!(backend.handler_count(), 1);

        // Click the Dismiss button.
        let header = backend.children(tree)[0];
        let button = backend.children(header)[1];
        let listener_id = {
            let doc = backend.document();
            let key = doc.store().key_of(backend.index_of(button));
            doc.store().columns().listeners.get(key).unwrap().iter().next().unwrap().id
        };
        let handler = backend.handler(listener_id).unwrap();
        let payload = Payload::Pointer(PointerEvent::mouse(Point::new(CssPx(1.0), CssPx(1.0))));
        let control = EventControl::new();
        let mut commands = DiscardCommands;
        let mut event = EventCx::new(
            EventKind::Click,
            button,
            button,
            Phase::Target,
            Modifiers::NONE,
            Timestamp::ORIGIN,
            &payload,
            &control,
            &mut commands,
        );
        handler(&mut event);

        assert!(!banner.is_visible());
        assert!(host.stylesheet(SHEET_NAME).is_none());
        assert_eq!(backend.handler_count(), 0);
        assert_eq!(backend.children(region.root), [app_child]);
    }
}
