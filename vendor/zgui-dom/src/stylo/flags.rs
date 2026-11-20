//! Indexed document obligations and temporary style traversal flags.

use zgui_bits::Dirty;

use crate::node::atomics;
use crate::node::handle::Node;

/// The obligations that mean "the style engine has work at or below this node".
pub const STYLE_WORK: Dirty = Dirty::RESTYLE.union(Dirty::RECASCADE);

impl Node<'_> {
    /// Whether a snapshot of this element's pre-mutation identity is recorded and unconsumed.
    pub fn has_snapshot(self) -> bool {
        self.record().has_atomic(atomics::HAS_SNAPSHOT)
    }

    /// Whether the traversal has already consumed this element's snapshot.
    pub fn handled_snapshot(self) -> bool {
        self.record().has_atomic(atomics::SNAPSHOT_HANDLED)
    }

    /// Records that the traversal has consumed this element's snapshot.
    pub fn set_handled_snapshot(self) {
        self.record().set_atomic(atomics::SNAPSHOT_HANDLED);
    }

    /// Whether the document's dirty-child index records work at or below this node.
    #[must_use]
    pub fn has_style_work_below(self) -> bool {
        self.record().has_dirty_descendants(STYLE_WORK)
    }

    /// Whether selector invalidation reached a descendant outside the document's dirty index.
    #[must_use]
    pub fn has_selector_work_below(self) -> bool {
        self.record().has_atomic(atomics::SELECTOR_DIRTY_DESCENDANTS)
    }

    /// Schedules descent for selector invalidation in this pass only.
    pub(crate) fn note_selector_work_below(self) {
        self.record().set_atomic(atomics::SELECTOR_DIRTY_DESCENDANTS);
    }

    /// Retires the selector descent flag once traversal has dispatched the children.
    pub fn clear_selector_work_below(self) {
        if self.has_selector_work_below() {
            self.record().clear_atomic(atomics::SELECTOR_DIRTY_DESCENDANTS);
        }
    }

    /// Whether the custom property map this element inherits from moved since its style was built.
    pub fn has_custom_map_changed(self) -> bool {
        self.record().has_atomic(atomics::CUSTOM_MAP_CHANGED)
    }

    /// Records that the custom property map this element inherits from has moved.
    pub fn note_custom_map_changed(self) {
        self.record().set_atomic(atomics::CUSTOM_MAP_CHANGED);
    }

    /// Takes the custom-map-changed note, answering whether it was set.
    pub fn take_custom_map_changed(self) -> bool {
        let was = self.has_custom_map_changed();
        if was {
            self.record().clear_atomic(atomics::CUSTOM_MAP_CHANGED);
        }
        was
    }

    /// Whether an animation-only restyle is pending below this node.
    pub fn has_animation_work_below(self) -> bool {
        self.record()
            .has_atomic(atomics::ANIMATION_DIRTY_DESCENDANTS)
    }

    /// Records that an animation-only restyle is pending below this node.
    pub fn note_animation_work_below(self) {
        self.record()
            .set_atomic(atomics::ANIMATION_DIRTY_DESCENDANTS);
    }

    /// Forgets that an animation-only restyle is pending below this node.
    pub fn clear_animation_work_below(self) {
        self.record()
            .clear_atomic(atomics::ANIMATION_DIRTY_DESCENDANTS);
    }
}

#[cfg(test)]
mod tests {
    use zgui_bits::Dirty;
    use zgui_interned::ElementName;

    use crate::arena::document::Document;
    use crate::node::kind::NodeKind;

    #[test]
    fn retiring_selector_work_preserves_document_obligations() {
        let mut document = Document::new();
        let root = document.append(
            document.document_index(),
            NodeKind::Element,
            ElementName::new("root"),
        );
        let child = document.append(root, NodeKind::Element, ElementName::new("item"));

        let node = document.node(root);
        node.note_selector_work_below();
        assert!(!node.has_style_work_below());

        crate::dirty::propagate::mark(document.store_mut(), child, Dirty::RESTYLE);
        let node = document.node(root);
        assert!(node.has_selector_work_below());
        node.clear_selector_work_below();
        assert!(!node.has_selector_work_below());
        assert!(node.has_style_work_below());
        assert!(
            document.node(child).has_style_work_below(),
            "a node's own obligation is part of its own subtree"
        );
    }

    #[test]
    fn a_phase_that_is_not_the_style_engines_does_not_look_like_style_work() {
        let mut document = Document::new();
        let root = document.append(
            document.document_index(),
            NodeKind::Element,
            ElementName::new("root"),
        );
        document.store().core(root).dirty().mark(Dirty::REPAINT);
        assert!(!document.node(root).has_style_work_below());
    }

    #[test]
    fn the_animation_flag_round_trips_because_it_has_one_owner() {
        let mut document = Document::new();
        let root = document.append(
            document.document_index(),
            NodeKind::Element,
            ElementName::new("root"),
        );
        let node = document.node(root);
        assert!(!node.has_animation_work_below());
        node.note_animation_work_below();
        assert!(node.has_animation_work_below());
        node.clear_animation_work_below();
        assert!(!node.has_animation_work_below());
    }
}
