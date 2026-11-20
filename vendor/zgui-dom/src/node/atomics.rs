//! Style traversal bookkeeping stored in one worker-accessible atomic word.
//!
//! Temporary invalidation flags are separate from the document's indexed frame obligations.

/// A snapshot of this element's pre-mutation state is recorded and not yet consumed.
pub const HAS_SNAPSHOT: u32 = 1 << 0;

/// The traversal has consumed this element's snapshot.
pub const SNAPSHOT_HANDLED: u32 = 1 << 1;

/// This element's style data has been established at least once, and not cleared since.
///
/// The style data itself is stored inline in every record, so "does this element have data?"
/// cannot be answered by its presence — the naive answer is always yes, which makes the engine's
/// subtree-clearing walk descend into subtrees it should have pruned. This bit is the real answer:
/// establishing data sets it, clearing data clears it *and* resets the inline data in the same
/// call, so the two answers cannot disagree.
pub const STYLED: u32 = 1 << 2;

/// An animation-only restyle reached a descendant during the animation pass.
pub const ANIMATION_DIRTY_DESCENDANTS: u32 = 1 << 3;

/// The custom property map this element inherits from moved since its style was built.
///
/// Raised on every child of an element whose inherited map changed, by the traversal step that
/// changed it, and consumed by the child's own visit: the child either cascades again, because
/// something it declares reads the map, or takes the new map into its existing style.
pub const CUSTOM_MAP_CHANGED: u32 = 1 << 4;

/// Selector invalidation reached a descendant during the current style pass.
pub const SELECTOR_DIRTY_DESCENDANTS: u32 = 1 << 5;

/// Every bit this word defines.
pub const ALL: u32 = HAS_SNAPSHOT
    | SNAPSHOT_HANDLED
    | STYLED
    | ANIMATION_DIRTY_DESCENDANTS
    | CUSTOM_MAP_CHANGED
    | SELECTOR_DIRTY_DESCENDANTS;

#[cfg(test)]
mod tests {
    use super::{
        ALL, ANIMATION_DIRTY_DESCENDANTS, CUSTOM_MAP_CHANGED, HAS_SNAPSHOT, SNAPSHOT_HANDLED,
        SELECTOR_DIRTY_DESCENDANTS, STYLED,
    };

    #[test]
    fn the_bits_are_distinct_and_the_union_is_exactly_them() {
        let bits = [
            HAS_SNAPSHOT,
            SNAPSHOT_HANDLED,
            STYLED,
            ANIMATION_DIRTY_DESCENDANTS,
            CUSTOM_MAP_CHANGED,
            SELECTOR_DIRTY_DESCENDANTS,
        ];
        assert_eq!(bits.iter().fold(0, |all, bit| all | bit), ALL);
        assert_eq!(ALL.count_ones(), 6);
    }
}
