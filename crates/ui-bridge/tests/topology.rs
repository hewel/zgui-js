use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use ui_bridge::registry::{BridgeError, Handle, NodeRegistry};
use zgui::view::{DomHandle, ElementName, NodeId};
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
        Self::with_id(zgui_dom::DocumentId::FIRST)
    }

    fn with_id(id: zgui_dom::DocumentId) -> Self {
        let document = Rc::new(RefCell::new(Document::with_id(id)));
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

    fn assert_children(&self, registry: &NodeRegistry, parent: Handle, expected: &[Handle]) {
        let mut logical = Vec::new();
        let mut child = registry.first_child(parent).unwrap();
        while let Some(node) = child {
            assert!(logical.len() < expected.len(), "duplicate or cyclic sibling chain");
            assert_eq!(registry.parent(node).unwrap(), Some(parent));
            logical.push(node);
            child = registry.next_sibling(node).unwrap();
        }
        assert_eq!(logical, expected);
        let native: Vec<_> = expected.iter().map(|&node| registry.native(node).unwrap()).collect();
        assert_eq!(self.backend.children(registry.native(parent).unwrap()), native);
    }
}

#[test]
fn mixed_children_empty_anchors_and_moves_preserve_native_identity() {
    let native = NativeDocument::new();
    let mut registry = native.registry(1, 1, 100);
    let root = registry.root();
    registry.begin_turn();
    let row = registry.create_element("row").unwrap();
    let left = registry.create_text("left").unwrap();
    let anchor = registry.create_text("").unwrap();
    let right = registry.create_element("text").unwrap();
    let text = registry.create_text("right").unwrap();
    registry.insert(root, row, None).unwrap();
    registry.insert(row, left, None).unwrap();
    registry.insert(row, anchor, None).unwrap();
    registry.insert(row, right, None).unwrap();
    registry.insert(right, text, None).unwrap();
    assert!(registry.is_text(anchor).unwrap());
    assert!(!registry.is_text(right).unwrap());
    assert_eq!(registry.replace_text(right, "not text"), Err(BridgeError::NotText));
    registry.replace_text(left, "LEFT").unwrap();
    assert!(native.dom.text_is(registry.native(left).unwrap(), "LEFT"));
    assert_eq!(native.backend.text_content(native.mount), "LEFTright");
    let right_native = registry.native(right).unwrap();
    registry.insert(row, right, Some(anchor)).unwrap();
    native.assert_children(&registry, row, &[left, right, anchor]);
    registry.insert(row, right, Some(right)).unwrap();
    registry.insert(row, right, Some(anchor)).unwrap();
    native.assert_children(&registry, row, &[left, right, anchor]);
    registry.insert(row, left, None).unwrap();
    native.assert_children(&registry, row, &[right, anchor, left]);
    registry.remove(row, right).unwrap();
    registry.insert(root, right, Some(row)).unwrap();
    assert_eq!(registry.native(right).unwrap(), right_native);
    native.assert_children(&registry, root, &[right, row]);
    native.assert_children(&registry, row, &[anchor, left]);
    assert!(registry.finish_turn().is_empty());
    native.backend.end_frame();
    assert_eq!(registry.native(right).unwrap(), right_native);
    native.assert_children(&registry, root, &[right, row]);
    registry.dispose();
    native.backend.end_frame();
}

#[test]
fn snapshots_bound_attached_preorder_without_retaining_detached_subtrees() {
    let native = NativeDocument::new();
    let mut registry = native.registry(1, 1, 10);
    let root = registry.root();
    registry.begin_turn();
    let sibling = registry.create_element("box").unwrap();
    let branch = registry.create_element("row").unwrap();
    let text = registry.create_text("mounted").unwrap();
    let orphan = registry.create_element("box").unwrap();
    let orphan_text = registry.create_text("never mounted").unwrap();
    registry.insert(root, sibling, None).unwrap();
    registry.insert(root, branch, Some(sibling)).unwrap();
    registry.insert(branch, text, None).unwrap();
    registry.insert(orphan, orphan_text, None).unwrap();
    registry.finish_turn();

    let snapshot = registry.snapshot(4);
    assert!(!snapshot.truncated);
    assert_eq!(
        snapshot.nodes.iter().map(|node| (node.token, node.parent)).collect::<Vec<_>>(),
        vec![(0, None), (branch.token(), Some(0)), (text.token(), Some(branch.token())), (sibling.token(), Some(0))],
    );
    for (node, handle) in snapshot.nodes.iter().zip([root, branch, text, sibling]) {
        assert_eq!(node.native, registry.native(handle).unwrap());
    }
    assert_eq!(snapshot.nodes[0].kind, "root");
    assert_eq!(snapshot.nodes[2].kind, "text");
    let empty = registry.snapshot(0);
    assert!(empty.nodes.is_empty());
    assert!(empty.truncated);
    let limited = registry.snapshot(2);
    assert_eq!(limited.nodes, snapshot.nodes[..2]);
    assert!(limited.truncated);

    registry.begin_turn();
    registry.remove(root, branch).unwrap();
    // Removed nodes are still native-live until reclamation, but not attached.
    assert!(native.dom.is_live(snapshot.nodes[2].native));
    let detached = registry.snapshot(2);
    assert_eq!(detached.nodes.iter().map(|node| node.token).collect::<Vec<_>>(), vec![0, sibling.token()]);
    assert!(!detached.truncated);
    registry.insert(root, branch, None).unwrap();
    registry.finish_turn();
    let reordered = registry.snapshot(4);
    assert_eq!(
        reordered.nodes.iter().map(|node| node.token).collect::<Vec<_>>(),
        vec![0, sibling.token(), branch.token(), text.token()],
    );
    assert!(!reordered.truncated);
    // Earlier evidence stays owned even when the live tree changes.
    assert_eq!(snapshot.nodes[1].token, branch.token());
    registry.dispose();
    assert!(registry.snapshot(0).nodes.is_empty());
    assert!(!registry.snapshot(0).truncated);
    native.backend.end_frame();
}

#[test]
fn reordering_a_list_matches_real_native_sibling_order() {
    let native = NativeDocument::new();
    let mut registry = native.registry(1, 1, 100);
    let root = registry.root();
    registry.begin_turn();
    let nodes: Vec<_> = (0..32).map(|index| registry.create_text(&index.to_string()).unwrap()).collect();
    for &node in &nodes {
        registry.insert(root, node, None).unwrap();
    }
    let identities: Vec<_> = nodes.iter().map(|&node| registry.native(node).unwrap()).collect();
    let mut expected = nodes.clone();
    for step in 0..160 {
        let from = (step * 7 + 3) % expected.len();
        let node = expected.remove(from);
        let to = (step * 11 + 5) % (expected.len() + 1);
        let anchor = expected.get(to).copied();
        registry.insert(root, node, anchor).unwrap();
        expected.insert(to, node);
        native.assert_children(&registry, root, &expected);
    }
    assert!(registry.finish_turn().is_empty());
    native.backend.end_frame();
    for (&node, identity) in nodes.iter().zip(identities) {
        assert_eq!(registry.native(node).unwrap(), identity);
    }
    registry.dispose();
    native.backend.end_frame();
}

#[test]
fn invalid_topology_requests_leave_both_trees_unchanged() {
    let native = NativeDocument::new();
    let mut registry = native.registry(1, 1, 100);
    let root = registry.root();
    registry.begin_turn();
    let parent = registry.create_element("column").unwrap();
    let child = registry.create_element("box").unwrap();
    let other = registry.create_element("stack").unwrap();
    let text = registry.create_text("").unwrap();
    registry.insert(root, parent, None).unwrap();
    registry.insert(parent, child, None).unwrap();
    registry.insert(root, other, None).unwrap();
    registry.insert(other, text, None).unwrap();
    assert_eq!(registry.insert(child, parent, None), Err(BridgeError::Cycle));
    assert_eq!(registry.insert(child, child, None), Err(BridgeError::Cycle));
    assert_eq!(registry.insert(parent, child, Some(text)), Err(BridgeError::InvalidAnchor));
    assert_eq!(registry.remove(other, child), Err(BridgeError::WrongParent));
    assert_eq!(registry.insert(text, child, None), Err(BridgeError::TextParent));
    native.assert_children(&registry, root, &[parent, other]);
    native.assert_children(&registry, parent, &[child]);
    native.assert_children(&registry, other, &[text]);
    assert!(registry.finish_turn().is_empty());
    registry.dispose();
    native.backend.end_frame();
}

#[test]
fn host_root_is_a_borrowed_boundary_not_a_managed_node() {
    let native = NativeDocument::new();
    let rust_sibling = native.dom.create_element(ElementName::new("row"));
    native.dom.insert(native.backend.root_node(), rust_sibling, None);
    let rust_child = native.dom.create_text("host owned");
    native.dom.insert(native.mount, rust_child, None);
    let before = native.count();
    let mut registry = native.registry(1, 1, 10);
    let root = registry.root();
    assert_eq!(registry.parent(root).unwrap(), None);
    assert_eq!(registry.next_sibling(root).unwrap(), None);
    assert_eq!(registry.first_child(root).unwrap(), None);
    registry.begin_turn();
    let child = registry.create_element("box").unwrap();
    registry.insert(root, child, None).unwrap();
    assert_eq!(registry.insert(child, root, None), Err(BridgeError::RootBoundary));
    assert_eq!(registry.remove(root, root), Err(BridgeError::RootBoundary));
    assert_eq!(registry.first_child(root).unwrap(), Some(child));
    registry.finish_turn();
    let invalidated: HashSet<_> = registry.dispose().into_iter().collect();
    assert_eq!(invalidated, HashSet::from([root, child]));
    assert_eq!(registry.native(root), Err(BridgeError::Disposed));
    assert!(registry.dispose().is_empty());
    native.backend.end_frame();
    assert!(native.dom.is_live(native.mount));
    assert!(native.dom.is_live(rust_sibling));
    assert!(native.dom.is_live(rust_child));
    assert_eq!(native.count(), before);
}

#[test]
fn handles_reject_foreign_sessions_documents_and_generations_before_native_access() {
    let native = NativeDocument::new();
    let other_document = NativeDocument::with_id(zgui_dom::DocumentId::new(1).unwrap());
    let mut registry = native.registry(1, 1, 10);
    let foreign_session = native.registry(2, 1, 10);
    let foreign_generation = native.registry(1, 2, 10);
    let foreign_document = other_document.registry(1, 1, 10);
    registry.begin_turn();
    assert_eq!(registry.native(foreign_session.root()), Err(BridgeError::WrongSession));
    assert_eq!(registry.parent(foreign_generation.root()), Err(BridgeError::WrongGeneration));
    assert_eq!(registry.is_text(foreign_document.root()), Err(BridgeError::WrongDocument));
    assert_eq!(registry.insert(registry.root(), foreign_document.root(), None), Err(BridgeError::WrongDocument));
    let child = registry.create_text("owned").unwrap();
    assert_eq!(registry.insert(registry.root(), child, Some(foreign_session.root())), Err(BridgeError::WrongSession));
    assert_eq!(registry.parent(child).unwrap(), None);
    registry.abort_turn();
    native.backend.end_frame();
}

#[test]
fn removal_invalidates_the_remaining_subtree_before_native_frame_reclamation() {
    let native = NativeDocument::new();
    let baseline = native.count();
    let mut registry = native.registry(1, 1, 10);
    let root = registry.root();
    registry.begin_turn();
    let parent = registry.create_element("box").unwrap();
    let child = registry.create_text("child").unwrap();
    registry.insert(root, parent, None).unwrap();
    registry.insert(parent, child, None).unwrap();
    let parent_native = registry.native(parent).unwrap();
    let child_native = registry.native(child).unwrap();
    registry.remove(root, parent).unwrap();
    assert_eq!(registry.parent(parent).unwrap(), None);
    assert_eq!(registry.parent(child).unwrap(), Some(parent));
    assert!(registry.native(child).is_ok());
    let invalidated: HashSet<_> = registry.finish_turn().into_iter().collect();
    assert_eq!(invalidated, HashSet::from([parent, child]));
    assert_eq!(registry.native(parent), Err(BridgeError::StaleHandle));
    assert_eq!(registry.first_child(child), Err(BridgeError::StaleHandle));
    assert!(native.dom.is_live(parent_native));
    assert!(native.dom.is_live(child_native));
    native.backend.end_frame();
    assert!(!native.dom.is_live(parent_native));
    assert!(!native.dom.is_live(child_native));
    assert_eq!(registry.live_count(), 0);
    assert_eq!(native.count(), baseline);
    registry.dispose();
}

#[test]
fn descendants_moved_out_of_detached_trees_survive_and_nested_removals_are_finalized() {
    let native = NativeDocument::new();
    let baseline = native.count();
    let mut registry = native.registry(1, 1, 20);
    let root = registry.root();
    registry.begin_turn();
    let branch = registry.create_element("box").unwrap();
    let survivor = registry.create_element("label").unwrap();
    let survivor_text = registry.create_text("escape").unwrap();
    let discarded = registry.create_element("box").unwrap();
    let leaf = registry.create_text("discard").unwrap();
    registry.insert(root, branch, None).unwrap();
    registry.insert(branch, survivor, None).unwrap();
    registry.insert(survivor, survivor_text, None).unwrap();
    registry.insert(branch, discarded, None).unwrap();
    registry.insert(discarded, leaf, None).unwrap();
    registry.remove(root, branch).unwrap();
    registry.remove(branch, discarded).unwrap();
    registry.insert(root, survivor, None).unwrap();
    let invalidated: HashSet<_> = registry.finish_turn().into_iter().collect();
    assert_eq!(invalidated, HashSet::from([branch, discarded, leaf]));
    native.backend.end_frame();
    native.assert_children(&registry, root, &[survivor]);
    native.assert_children(&registry, survivor, &[survivor_text]);
    assert_eq!(native.backend.text_content(native.mount), "escape");
    assert_eq!(registry.live_count(), 2);
    registry.dispose();
    native.backend.end_frame();
    assert_eq!(native.count(), baseline);
}

#[test]
fn detached_tree_reinserted_under_another_pending_tree_is_invalidated_with_its_new_owner() {
    let native = NativeDocument::new();
    let baseline = native.count();
    let mut registry = native.registry(1, 1, 20);
    let root = registry.root();
    registry.begin_turn();
    let first = registry.create_element("box").unwrap();
    let second = registry.create_element("box").unwrap();
    registry.insert(root, first, None).unwrap();
    registry.insert(root, second, None).unwrap();
    registry.remove(root, first).unwrap();
    registry.remove(root, second).unwrap();
    registry.insert(second, first, None).unwrap();
    let invalidated: HashSet<_> = registry.finish_turn().into_iter().collect();
    assert_eq!(invalidated, HashSet::from([first, second]));
    native.backend.end_frame();
    assert_eq!(native.count(), baseline);
    registry.dispose();
}

#[test]
fn successful_never_linked_allocations_stay_owned_until_disposal() {
    let native = NativeDocument::new();
    let baseline = native.count();
    let mut registry = native.registry(1, 1, 10);
    registry.begin_turn();
    let orphan = registry.create_element("scroll").unwrap();
    let child = registry.create_text("never mounted").unwrap();
    registry.insert(orphan, child, None).unwrap();
    let orphan_native = registry.native(orphan).unwrap();
    let child_native = registry.native(child).unwrap();
    assert!(registry.finish_turn().is_empty());
    native.backend.end_frame();
    assert_eq!(registry.live_count(), 2);
    assert_eq!(registry.parent(orphan).unwrap(), None);
    assert_eq!(registry.parent(child).unwrap(), Some(orphan));
    registry.dispose();
    assert!(native.dom.is_live(orphan_native));
    assert!(native.dom.is_live(child_native));
    native.backend.end_frame();
    assert!(!native.dom.is_live(orphan_native));
    assert!(!native.dom.is_live(child_native));
    assert_eq!(native.count(), baseline);
}

#[test]
fn aborted_construction_reclaims_attached_preexisting_and_never_linked_nodes() {
    let native = NativeDocument::new();
    let baseline = native.count();
    for generation in 1..=20 {
        let mut registry = native.registry(1, generation, 10);
        let root = registry.root();
        registry.begin_turn();
        let existing = registry.create_element("column").unwrap();
        let existing_text = registry.create_text("preexisting").unwrap();
        registry.insert(root, existing, None).unwrap();
        registry.insert(existing, existing_text, None).unwrap();
        registry.finish_turn();
        native.backend.end_frame();
        registry.begin_turn();
        registry.remove(root, existing).unwrap();
        let orphan = registry.create_element("control").unwrap();
        let orphan_child = registry.create_text("orphan").unwrap();
        registry.insert(orphan, orphan_child, None).unwrap();
        let isolated = registry.create_text("").unwrap();
        let attached = registry.create_element("row").unwrap();
        registry.insert(root, attached, None).unwrap();
        let owned = [existing, existing_text, orphan, orphan_child, isolated, attached];
        let native_nodes: Vec<_> = owned.iter().map(|&node| registry.native(node).unwrap()).collect();
        let invalidated: HashSet<_> = registry.abort_turn().into_iter().collect();
        assert_eq!(invalidated.len(), owned.len() + 1);
        assert!(owned.iter().all(|node| invalidated.contains(node)));
        assert!(invalidated.contains(&root));
        assert_eq!(registry.live_count(), 0);
        assert_eq!(registry.native(existing), Err(BridgeError::Disposed));
        assert_eq!(registry.create_text("late"), Err(BridgeError::Disposed));
        native.backend.end_frame();
        assert!(native_nodes.iter().all(|&node| !native.dom.is_live(node)));
        assert_eq!(native.count(), baseline, "orphan growth after generation {generation}");
    }
}

#[test]
fn native_liveness_is_checked_even_before_logical_invalidation() {
    let native = NativeDocument::new();
    let mut registry = native.registry(1, 1, 10);
    let root = registry.root();
    registry.begin_turn();
    let node = registry.create_text("external reclamation").unwrap();
    registry.insert(root, node, None).unwrap();
    let id = registry.native(node).unwrap();
    registry.finish_turn();
    // Structural ownership forbids this in production. It proves that stale
    // native generations cannot reach strict insertion/text APIs defensively.
    native.dom.detach(id);
    native.backend.end_frame();
    assert_eq!(registry.native(node), Err(BridgeError::StaleHandle));
    assert_eq!(registry.is_text(node), Err(BridgeError::StaleHandle));
    assert_eq!(registry.parent(node), Err(BridgeError::StaleHandle));
    assert_eq!(registry.first_child(root), Err(BridgeError::StaleHandle));
    registry.begin_turn();
    assert_eq!(registry.replace_text(node, "late"), Err(BridgeError::StaleHandle));
    assert_eq!(registry.insert(root, node, None), Err(BridgeError::StaleHandle));
    assert_eq!(registry.remove(root, node), Err(BridgeError::StaleHandle));
    registry.abort_turn();
    native.backend.end_frame();
}

#[test]
fn allocation_limits_and_turn_guards_do_not_allocate_native_orphans() {
    let native = NativeDocument::new();
    let baseline = native.count();
    let mut registry = native.registry(1, 1, 2);
    assert_eq!(registry.create_text("outside turn"), Err(BridgeError::TurnNotActive));
    registry.begin_turn();
    assert!(matches!(registry.create_element("script"), Err(BridgeError::UnsupportedTag(_))));
    assert_eq!(native.count(), baseline);
    let first = registry.create_text("").unwrap();
    let second = registry.create_element("box").unwrap();
    assert_ne!(first.token(), second.token());
    assert_eq!(registry.create_text("over limit"), Err(BridgeError::NodeLimit));
    assert_eq!(native.count(), baseline + 2);
    registry.finish_turn();
    assert_eq!(registry.replace_text(first, "outside turn"), Err(BridgeError::TurnNotActive));
    registry.dispose();
    native.backend.end_frame();
    assert_eq!(native.count(), baseline);
}
