use std::cell::RefCell;
use std::rc::Rc;

use ui_bridge::registry::{BridgeError, NodeRegistry};
use zgui::view::{DomHandle, ElementName, NodeId, OverlayLayer};
use zgui_dom::Document;
use zgui_view_dom::DocumentDom;

struct NativeDocument {
    document: Rc<RefCell<Document>>,
    backend: Rc<DocumentDom>,
    dom: DomHandle,
    mount: NodeId,
}

impl NativeDocument {
    fn new() -> Self {
        let document = Rc::new(RefCell::new(Document::with_id(zgui_dom::DocumentId::FIRST)));
        let backend = Rc::new(DocumentDom::new(document.clone()));
        let dom = DomHandle::from_rc(backend.clone());
        let mount = dom.create_element(ElementName::new("box"));
        dom.insert(backend.root_node(), mount, None);
        Self { document, backend, dom, mount }
    }

    fn registry(&self, session: u64, generation: u64, max_nodes: usize) -> NodeRegistry {
        NodeRegistry::new(self.dom.clone(), self.mount, session, generation, max_nodes)
    }

    fn count(&self) -> usize {
        self.document.borrow().len()
    }
}

#[test]
fn portal_content_escapes_its_source_and_keeps_logical_order() {
    let native = NativeDocument::new();
    let baseline = native.count();
    let mut registry = native.registry(1, 1, 100);
    let root = registry.root();
    let band = native.backend.overlay_layer(OverlayLayer::Modal);
    registry.begin_turn();
    let left = registry.create_text("[").unwrap();
    let portal = registry.create_portal(OverlayLayer::Modal).unwrap();
    let floating = registry.create_text("floating").unwrap();
    let right = registry.create_text("]").unwrap();
    registry.insert(portal, floating, None).unwrap();
    registry.insert(root, left, None).unwrap();
    registry.insert(root, portal, None).unwrap();
    registry.insert(root, right, None).unwrap();
    assert_eq!(native.backend.text_content(native.mount), "[]");
    assert_eq!(native.backend.text_content(band), "floating");
    let content = registry.native(portal).unwrap();
    assert_eq!(native.dom.parent(content), Some(band));

    assert_eq!(registry.first_child(root).unwrap(), Some(left));
    assert_eq!(registry.next_sibling(left).unwrap(), Some(portal));
    assert_eq!(registry.next_sibling(portal).unwrap(), Some(right));
    assert_eq!(registry.parent(portal).unwrap(), Some(root));
    assert_eq!(registry.parent(root).unwrap(), None);

    let snapshot = registry.snapshot(100);
    let kinds: Vec<&str> = snapshot.nodes.iter().map(|node| node.kind).collect();
    assert_eq!(kinds, ["root", "text", "portal", "text", "text"]);
    assert_eq!(snapshot.nodes[2].native, content);

    registry.insert(root, right, Some(portal)).unwrap();
    registry.insert(root, portal, Some(left)).unwrap();
    assert_eq!(registry.first_child(root).unwrap(), Some(portal));
    assert_eq!(registry.next_sibling(portal).unwrap(), Some(left));
    assert_eq!(registry.next_sibling(left).unwrap(), Some(right));
    assert_eq!(native.backend.text_content(native.mount), "[]");
    assert_eq!(registry.native(portal).unwrap(), content);
    assert_eq!(native.dom.parent(content), Some(band));

    assert_eq!(registry.insert(portal, root, None), Err(BridgeError::RootBoundary));
    assert_eq!(registry.insert(left, portal, None), Err(BridgeError::TextParent));

    assert!(registry.finish_turn().is_empty());
    native.backend.end_frame();
    assert_eq!(registry.native(portal).unwrap(), content);
    assert_eq!(native.dom.parent(content), Some(band));

    registry.dispose();
    native.backend.end_frame();
    assert_eq!(native.count(), baseline);
    assert!(native.backend.children(band).is_empty());
}

#[test]
fn disconnected_portals_park_without_losing_identity() {
    let native = NativeDocument::new();
    let baseline = native.count();
    let mut registry = native.registry(2, 1, 100);
    let root = registry.root();
    let band = native.backend.overlay_layer(OverlayLayer::Popover);
    registry.begin_turn();
    let orphan = registry.create_element("box").unwrap();
    let stash = registry.create_element("box").unwrap();
    let portal = registry.create_portal(OverlayLayer::Popover).unwrap();
    let text = registry.create_text("held").unwrap();
    registry.insert(portal, text, None).unwrap();
    registry.insert(orphan, portal, None).unwrap();
    assert!(registry.finish_turn().is_empty());
    native.backend.end_frame();
    assert!(native.backend.children(band).is_empty());
    assert_eq!(registry.snapshot(100).nodes.len(), 1);
    let content = registry.native(portal).unwrap();
    assert_eq!(native.backend.text_content(content), "held");

    registry.begin_turn();
    registry.insert(root, orphan, None).unwrap();
    assert_eq!(native.backend.text_content(band), "held");
    assert!(registry.finish_turn().is_empty());
    native.backend.end_frame();
    assert_eq!(registry.native(portal).unwrap(), content);

    registry.begin_turn();
    registry.insert(stash, orphan, None).unwrap();
    assert!(native.backend.children(band).is_empty());
    assert!(registry.finish_turn().is_empty());
    native.backend.end_frame();
    assert_eq!(registry.native(portal).unwrap(), content);
    assert_eq!(native.backend.text_content(content), "held");
    registry.begin_turn();
    registry.insert(root, orphan, None).unwrap();
    assert_eq!(native.backend.text_content(band), "held");
    assert!(registry.finish_turn().is_empty());

    registry.begin_turn();
    registry.remove(root, orphan).unwrap();
    registry.insert(root, orphan, None).unwrap();
    assert!(registry.finish_turn().is_empty());
    assert_eq!(registry.native(portal).unwrap(), content);
    native.backend.end_frame();
    assert_eq!(native.backend.text_content(band), "held");

    registry.dispose();
    native.backend.end_frame();
    assert_eq!(native.count(), baseline);
}

#[test]
fn unopened_and_same_turn_removed_portals_leave_no_native_nodes() {
    let native = NativeDocument::new();
    let baseline = native.count();
    let mut registry = native.registry(3, 1, 100);
    let root = registry.root();

    registry.begin_turn();
    let portal = registry.create_portal(OverlayLayer::Toast).unwrap();
    let text = registry.create_text("x").unwrap();
    registry.insert(portal, text, None).unwrap();
    registry.insert(root, portal, None).unwrap();
    registry.remove(root, portal).unwrap();
    assert_eq!(registry.finish_turn(), vec![portal, text]);
    native.backend.end_frame();
    assert_eq!(native.count(), baseline);

    registry.begin_turn();
    let portal = registry.create_portal(OverlayLayer::Content).unwrap();
    let text = registry.create_text("x").unwrap();
    registry.insert(portal, text, None).unwrap();
    assert!(registry.finish_turn().is_empty());
    native.backend.end_frame();
    let band = native.backend.overlay_layer(OverlayLayer::Content);
    assert!(native.backend.children(band).is_empty());
    assert_eq!(native.backend.text_content(registry.native(portal).unwrap()), "x");

    registry.dispose();
    native.backend.end_frame();
    assert_eq!(native.count(), baseline);
}

#[test]
fn shared_bands_keep_other_session_content_and_reject_cross_session_nodes() {
    let native = NativeDocument::new();
    let mount_a = native.dom.create_element(ElementName::new("box"));
    native.dom.insert(native.backend.root_node(), mount_a, None);
    let mount_b = native.dom.create_element(ElementName::new("box"));
    native.dom.insert(native.backend.root_node(), mount_b, None);
    let baseline = native.count();
    let mut first = NodeRegistry::new(native.dom.clone(), mount_a, 1, 1, 100);
    let mut second = NodeRegistry::new(native.dom.clone(), mount_b, 2, 1, 100);
    let band = native.backend.overlay_layer(OverlayLayer::Modal);

    first.begin_turn();
    let first_portal = first.create_portal(OverlayLayer::Modal).unwrap();
    let text_a = first.create_text("A").unwrap();
    first.insert(first_portal, text_a, None).unwrap();
    first.insert(first.root(), first_portal, None).unwrap();
    assert!(first.finish_turn().is_empty());
    second.begin_turn();
    let second_portal = second.create_portal(OverlayLayer::Modal).unwrap();
    let text_b = second.create_text("B").unwrap();
    second.insert(second_portal, text_b, None).unwrap();
    second.insert(second.root(), second_portal, None).unwrap();
    assert!(second.finish_turn().is_empty());
    assert_eq!(native.backend.text_content(band), "AB");

    first.begin_turn();
    assert_eq!(first.insert(first.root(), second_portal, None), Err(BridgeError::WrongSession));
    assert!(first.finish_turn().is_empty());
    assert_eq!(native.backend.text_content(band), "AB");
    native.backend.end_frame();

    first.dispose();
    native.backend.end_frame();
    assert_eq!(native.backend.text_content(band), "B");
    assert!(second.native(second_portal).is_ok());
    assert!(native.dom.is_live(band));

    second.dispose();
    native.backend.end_frame();
    assert_eq!(native.backend.text_content(band), "");
    assert_eq!(native.count(), baseline);
}

#[test]
fn portal_allocation_obeys_node_limit() {
    let native = NativeDocument::new();
    let baseline = native.count();
    let mut registry = native.registry(5, 1, 0);
    registry.begin_turn();
    assert_eq!(registry.create_portal(OverlayLayer::Modal), Err(BridgeError::NodeLimit));
    assert_eq!(native.count(), baseline);
}

#[test]
fn nested_portals_retire_with_their_parent_unless_moved_out() {
    for rescued in [false, true] {
        let native = NativeDocument::new();
        let baseline = native.count();
        let mut registry = native.registry(6, 1, 100);
        let root = registry.root();
        registry.begin_turn();
        let outer = registry.create_portal(OverlayLayer::Modal).unwrap();
        let inner = registry.create_portal(OverlayLayer::Toast).unwrap();
        let text = registry.create_text("nested").unwrap();
        registry.insert(inner, text, None).unwrap();
        registry.insert(outer, inner, None).unwrap();
        registry.insert(root, outer, None).unwrap();
        registry.finish_turn();
        native.backend.end_frame();
        let inner_native = registry.native(inner).unwrap();
        registry.begin_turn();
        registry.remove(root, outer).unwrap();
        if rescued { registry.insert(root, inner, None).unwrap(); }
        let invalid = registry.finish_turn();
        native.backend.end_frame();
        assert!(invalid.contains(&outer));
        assert!(native.backend.children(native.backend.overlay_layer(OverlayLayer::Modal)).is_empty());
        if rescued {
            assert!(!invalid.contains(&inner));
            assert_eq!(registry.native(inner).unwrap(), inner_native);
            assert_eq!(native.backend.text_content(native.backend.overlay_layer(OverlayLayer::Toast)), "nested");
            registry.begin_turn();
            registry.remove(root, inner).unwrap();
            registry.finish_turn();
            native.backend.end_frame();
        } else {
            assert!(invalid.contains(&inner) && invalid.contains(&text));
        }
        assert!(native.backend.children(native.backend.overlay_layer(OverlayLayer::Toast)).is_empty());
        assert_eq!(native.count(), baseline);
        registry.dispose();
    }
}
