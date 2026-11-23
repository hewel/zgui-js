//! A press that landed on a scrollbar.
//!
//! A bar belongs to the element that scrolls, so an ordinary hit test answers with the scroll
//! container and loses the only two things the press was about: which of its bars was pressed, and
//! which piece of that bar. Both are properties of the *fragment*, which the index already names,
//! so this asks the same index the same question and keeps the answer it throws away.

use zgui_dom::{DocumentStore, NodeKey};
use zgui_geom::{Device, DevicePx, Point};
use zgui_layout::fragment::ScrollbarPart;
use zgui_layout::scroll_region::ScrollbarMode;
use zgui_layout::scroll_region::bar::live;
use zgui_layout::{Axis, FragmentKind, HitIndex, LayoutStore};
use zgui_scene::{ClipTable, SpatialTree};

/// Decides whether an overlay scrollbar on `(container, axis)` is currently interactive.
///
/// Classic scrollbars are always interactive and are never filtered out. An overlay scrollbar
/// whose visual opacity has faded to zero (`alpha == 0.0`) returns `false` so presses and hover
/// pass straight through to the underlying content.
pub trait ScrollbarHitFilter {
    /// Returns `true` when the overlay scrollbar on `(container, axis)` may intercept hits.
    fn is_scrollbar_interactive(&self, container: NodeKey, axis: Axis) -> bool;
}

/// Default filter that treats every scrollbar as interactive.
#[derive(Clone, Copy, Debug, Default)]
pub struct AllScrollbarsInteractive;

impl ScrollbarHitFilter for AllScrollbarsInteractive {
    fn is_scrollbar_interactive(&self, _container: NodeKey, _axis: Axis) -> bool {
        true
    }
}

/// Where a press on a scrollbar landed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Press {
    /// The element that scrolls.
    pub container: NodeKey,
    /// Which of its bars was pressed.
    pub axis: Axis,
    /// Which piece of that bar.
    pub part: ScrollbarPart,
    /// Where the press landed along the bar's own axis, in the bar's own space.
    ///
    /// Not in device pixels: the thumb, the track and the travel between them are all in the
    /// scroller's own space, and a press is only ever compared against those. See
    /// [`into_bar_space`](zgui_layout::scroll_region::bar::live::into_bar_space) for what carrying
    /// the pointer there is worth.
    pub along: f32,
    /// Where the thumb's near edge was along the same axis, or nothing when there is no thumb.
    ///
    /// A track with no thumb beside it belongs to a gutter that was reserved for content which
    /// turned out to fit — the strip is filled so that it is not a hole in the window, and there is
    /// nothing in it to page towards.
    pub thumb: Option<f32>,
}

impl Press {
    /// How far the press landed from the thumb's near edge.
    ///
    /// This is what a drag has to keep. Without it the thumb snaps its near edge — or, worse, its
    /// centre — to the pointer the instant the button goes down, so grabbing a thumb anywhere but
    /// its very top throws the content somewhere nobody asked for before the drag has begun.
    pub fn grab(&self) -> Option<f32> {
        self.thumb.map(|thumb| self.along - thumb)
    }

    /// Whether a press on the track is asking to move towards the end rather than the start.
    pub fn pages_forward(&self) -> Option<bool> {
        self.thumb.map(|thumb| self.along >= thumb)
    }
}

/// The scrollbar under `point`, if the topmost non-inert thing there is one.
///
/// Topmost non-inert and no further: an effectively inert overlay or scrollbar chrome is skipped,
/// while a non-inert element covering the bar blocks it.
pub fn at(
    document: &DocumentStore,
    layout: &LayoutStore,
    index: &HitIndex,
    clips: &ClipTable,
    spatial: &SpatialTree,
    point: Point<DevicePx, Device>,
) -> Option<Press> {
    at_with_filter(
        document,
        layout,
        index,
        clips,
        spatial,
        point,
        &AllScrollbarsInteractive,
    )
}

/// Like [`at`], skipping hidden overlay scrollbars rejected by `filter`.
pub fn at_with_filter(
    document: &DocumentStore,
    layout: &LayoutStore,
    index: &HitIndex,
    clips: &ClipTable,
    spatial: &SpatialTree,
    point: Point<DevicePx, Device>,
    filter: &dyn ScrollbarHitFilter,
) -> Option<Press> {
    for frag in index.hit(point, clips, spatial) {
        let Some(fragment) = layout.fragment(frag) else {
            continue;
        };
        let owner = match fragment.kind {
            FragmentKind::Scrollbar { .. } => layout.get(fragment.box_).and_then(|b| b.source),
            _ => crate::hit::element_of(layout, frag),
        };
        let Some(owner) = owner else {
            continue;
        };
        if document.is_effectively_inert(owner) {
            continue;
        }
        let FragmentKind::Scrollbar { axis, part } = fragment.kind else {
            return None;
        };
        if layout.scrollbar_mode(fragment.box_) == ScrollbarMode::Overlay
            && !filter.is_scrollbar_interactive(owner, axis)
        {
            continue;
        }
        let local = live::into_bar_space(layout, spatial, fragment.box_, point)?;
        let thumb =
            live::thumb_of(layout, fragment.box_, axis).map(|thumb| along(axis, thumb.origin));
        return Some(Press {
            container: owner,
            axis,
            part,
            along: along(axis, local),
            thumb,
        });
    }
    None
}

/// Where a device-space point falls along one bar of `container`, in that bar's own space.
///
/// What every move of a drag is measured with, and it asks the fragment tree the question afresh
/// rather than reusing anything the press worked out — for the same reason the travel is read
/// afresh: the scroller may have been resized, moved or animated under the pointer since, and a
/// drag is meant to go on meaning the same fraction of whatever track is there now.
///
/// Answers with nothing for an element that generated no box, which is a drag whose container has
/// gone and which therefore moves nothing.
pub fn along_bar(
    layout: &LayoutStore,
    spatial: &SpatialTree,
    container: NodeKey,
    axis: Axis,
    point: Point<DevicePx, Device>,
) -> Option<f32> {
    let box_ = *layout.boxes_of(container).first()?;
    let local = live::into_bar_space(layout, spatial, box_, point)?;
    Some(along(axis, local))
}

/// One coordinate of a point, taken along `axis`.
///
/// Private, and it is worth saying why: this is the step that throws away which space the point was
/// in, so anything reaching for it from outside would be projecting whatever it happened to hold —
/// and a device pointer projected onto a bar's axis is the whole of the fault this module carries
/// [`along_bar`] to avoid.
fn along(axis: Axis, point: Point<DevicePx, Device>) -> f32 {
    match axis {
        Axis::Vertical => point.y.0,
        Axis::Horizontal => point.x.0,
    }
}
