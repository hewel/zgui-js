//! Native frame-synchronous background blur registration, visibility/opacity gating, and
//! handover state for one [`Window`](crate::Window).
//!
//! Each window is the single effective writer for its surface's background blur across `"none"`,
//! `"full"`, explicit rectangle lists, and live node lists. Registration intent lives in a shared
//! [`BlurSlot`] so host bridge writes before composition take effect on the current frame, while
//! writes on an idle window wake the window without marking DOM dirty flags or forcing a
//! restyle/relayout.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use zgui_css::values::effect::ClipPathValue;
use zgui_css::values::size::VisibilityValue;
use zgui_dom::{Document, NodeKey};
use zgui_geom::{Css, CssPx, Rect};
use zgui_layout::{BlurPiece, BlurSurfaceMapping, BoxKind, LayoutStore, resolve_blur_rects};
use zgui_paint::lower::anim::AnimOverrides;
use zgui_platform::BlurRegion;
use zgui_reactive::FrameWaker;
use zgui_scene::{ClipTable, SpatialTree};

use crate::wake::RuntimeWaker;

/// Authoritative declarative background blur intent registered on one window/surface.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum BlurRegistration {
    /// No background blur is requested.
    #[default]
    None,
    /// Full-surface background blur.
    Full,
    /// Explicit surface-local CSS rectangles (preserves legacy outward/fractional behavior).
    Rects(Vec<Rect<CssPx, Css>>),
    /// Native frame-synchronous resolution over target document nodes.
    Nodes {
        /// Ordered target node keys owned by this window's document.
        nodes: Vec<NodeKey>,
        /// Accumulated effective-opacity cutoff in `[0.0, 1.0]`. Default `0.5`; `0.0` skips only
        /// actual `<= 0` opacity factors and `visibility: hidden`.
        min_opacity: f64,
    },
}

/// Shared live blur registration slot bound to a mounted [`Window`](crate::Window).
///
/// Writes update the window's registered intent immediately and wake the window without marking
/// pixels or layout dirty.
pub struct BlurSlot {
    registration: RefCell<BlurRegistration>,
    intent_version: Cell<u64>,
    waker: Arc<RuntimeWaker>,
}

impl core::fmt::Debug for BlurSlot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BlurSlot")
            .field("intent_version", &self.intent_version.get())
            .finish_non_exhaustive()
    }
}

impl BlurSlot {
    /// Creates an empty blur slot wired to `waker`.
    #[must_use]
    pub fn new(waker: Arc<RuntimeWaker>) -> Self {
        Self {
            registration: RefCell::new(BlurRegistration::None),
            intent_version: Cell::new(0),
            waker,
        }
    }

    /// Replaces the registered blur intent and wakes the owning window when it changes.
    ///
    /// Returns `true` if the registration changed.
    pub fn set(&self, next: BlurRegistration) -> bool {
        let mut current = self.registration.borrow_mut();
        if *current == next {
            return false;
        }
        *current = next;
        drop(current);
        self.intent_version
            .set(self.intent_version.get().wrapping_add(1));
        self.waker.wake();
        true
    }

    /// Snapshots the current registration and its monotonic intent version.
    #[must_use]
    pub fn snapshot(&self) -> (BlurRegistration, u64) {
        (
            self.registration.borrow().clone(),
            self.intent_version.get(),
        )
    }

    /// The current monotonic intent version.
    #[must_use]
    pub fn version(&self) -> u64 {
        self.intent_version.get()
    }
}

/// Canonical resolved blur region ready for platform handover or diffing.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum ResolvedBlur {
    #[default]
    None,
    Full,
    Rects(Vec<Rect<CssPx, Css>>),
}

impl ResolvedBlur {
    /// Borrows this resolved blur region as the platform [`BlurRegion`] contract.
    #[must_use]
    pub(crate) fn as_platform(&self) -> BlurRegion<'_> {
        match self {
            Self::None => BlurRegion::None,
            Self::Full => BlurRegion::Full,
            Self::Rects(rects) => BlurRegion::Rects(rects),
        }
    }
}

/// Picture-bound blur candidate associated with a composed frame held by the renderer.
#[derive(Clone, Debug)]
pub(crate) struct HeldBlur {
    pub(crate) resolved: ResolvedBlur,
    pub(crate) version: u64,
    pub(crate) bootstrap: bool,
}

/// Per-window three-state blur tracker: registered intent, held picture candidate, and last
/// confirmed applied region.
#[derive(Debug)]
pub(crate) struct WindowBlurState {
    pub(crate) slot: Rc<BlurSlot>,
    pub(crate) last_applied: ResolvedBlur,
    pub(crate) applied_version: u64,
    pub(crate) snapshot_version: u64,
    pub(crate) held: Option<HeldBlur>,
    pub(crate) stale: bool,
}

impl WindowBlurState {
    pub(crate) fn new(slot: Rc<BlurSlot>) -> Self {
        let initial = slot.version();
        Self {
            slot,
            last_applied: ResolvedBlur::None,
            applied_version: initial,
            snapshot_version: initial,
            held: None,
            stale: false,
        }
    }

    /// Whether the surface's applied blur has not yet converged with the latest registration or
    /// composed picture. Kept strictly separate from `owes_further_work()`.
    #[must_use]
    pub(crate) fn is_pending(&self) -> bool {
        self.slot.version() != self.applied_version || self.stale
    }

    /// Whether a registration write landed after this frame's blur snapshot.
    #[must_use]
    pub(crate) fn updated_since_snapshot(&self) -> bool {
        self.slot.version() != self.snapshot_version
    }
}

/// Resolves a [`BlurRegistration`] against the current frame's layout, live animation overrides,
/// ancestor clip chains, and live spatial tree.
#[must_use]
pub(crate) fn resolve_registration(
    registration: &BlurRegistration,
    layout: &LayoutStore,
    document: &Document,
    clips: &ClipTable,
    spatial: &SpatialTree,
    mapping: BlurSurfaceMapping,
    scale: f32,
) -> ResolvedBlur {
    match registration {
        BlurRegistration::None => ResolvedBlur::None,
        BlurRegistration::Full => ResolvedBlur::Full,
        BlurRegistration::Rects(rects) => {
            if rects.is_empty() {
                ResolvedBlur::None
            } else {
                ResolvedBlur::Rects(rects.clone())
            }
        }
        BlurRegistration::Nodes { nodes, min_opacity } => {
            if nodes.is_empty()
                || !min_opacity.is_finite()
                || !(0.0..=1.0).contains(min_opacity)
            {
                return ResolvedBlur::None;
            }
            let mut pieces = Vec::new();
            for &node_key in nodes {
                if document.store().index_of(node_key).is_none() {
                    continue;
                }
                for &box_key in layout.boxes_of(node_key) {
                    let box_node = layout.node(box_key);
                    // Only the element's own generated box participates (never pseudo/anonymous).
                    if box_node.kind != BoxKind::Element
                        || box_node.pseudo.is_some()
                        || box_node.source != Some(node_key)
                    {
                        continue;
                    }
                    // Test target's own computed visibility: a `visibility: visible` child inside
                    // a `visibility: hidden` parent stays eligible while inheriting ancestor
                    // opacity multiplication.
                    if box_node.style.get_inherited_box().visibility != VisibilityValue::Visible {
                        continue;
                    }
                    let mut ancestor = Some(box_key);
                    let mut effective_opacity = 1.0_f64;
                    let mut has_zero_factor = false;
                    let mut has_clip_path = false;
                    while let Some(key) = ancestor {
                        let current = layout.node(key);
                        if !matches!(current.style.get_svg().clip_path, ClipPathValue::None) {
                            has_clip_path = true;
                            break;
                        }
                        let opacity = current
                            .source
                            .and_then(|src| AnimOverrides::get(document, src))
                            .and_then(|over| over.opacity)
                            .unwrap_or_else(|| current.style.get_effects().opacity);
                        if !opacity.is_finite() || opacity <= 0.0 {
                            has_zero_factor = true;
                            break;
                        }
                        effective_opacity *= f64::from(opacity.clamp(0.0, 1.0));
                        ancestor = current.parent;
                    }
                    if has_clip_path || has_zero_factor {
                        continue;
                    }
                    if *min_opacity > 0.0 && effective_opacity < *min_opacity {
                        continue;
                    }
                    let declared = layout.custom_properties(box_key, &box_node.style);
                    for &frag_key in layout.fragments_of_box(box_key) {
                        let Some(fragment) = layout.fragment(frag_key) else {
                            continue;
                        };
                        if let Some(piece) =
                            BlurPiece::from_fragment(fragment, &box_node.style, declared, scale)
                        {
                            pieces.push(piece);
                        }
                    }
                }
            }
            if pieces.is_empty() {
                return ResolvedBlur::None;
            }
            let rects = resolve_blur_rects(&pieces, clips, |id| spatial.resolve(id), mapping);
            if rects.is_empty() {
                ResolvedBlur::None
            } else {
                ResolvedBlur::Rects(rects)
            }
        }
    }
}
