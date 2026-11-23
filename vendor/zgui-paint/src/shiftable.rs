//! Whether a scrollport's pixels may be moved instead of drawn again.
//!
//! A renderer that keeps its composed frame can answer a scroll by translating the part of the port
//! that is still valid. What it cannot do is decide whether that is the same picture, because the
//! two things that would make it a different picture are properties of the document:
//!
//! * **something else draws in the port.** A tooltip over a list, a dialog, a sibling that overlaps
//!   it — the composed pixels are the composite, so moving them moves whatever was composited in.
//! * **the port is not opaque.** The composite includes whatever showed through the content, and
//!   what showed through does not scroll. Moving it smears it.
//!
//! Both are refusals rather than corrections. The fallback is the frame the window would have drawn
//! anyway, so a refusal costs the optimisation and never the picture.
//!
//! # What is not tested here, because it cannot happen
//!
//! That the movement was rigid. The fragment pass reports a move only for a scroll container that
//! draws its content in a space of its own, and it reports one move for the whole port. A sticky
//! or fixed box below it does not move with the content, and the fragment pass damages it beyond
//! the move: where the moved pixels put it, and where it is now. A container that keeps its
//! children in its own space reports no move, so the frame draws its port again.

use smallvec::SmallVec;
use zgui_css::ComputedStyle;
use zgui_css::values::effect::MixBlendModeValue;
use zgui_css::values::image::ImageValue;
use zgui_dom::NodeKey;
use zgui_dom::side::BoxKey;
use zgui_geom::{Corners, Device, DevicePx, Rect, Size, Vec2};
use zgui_layout::FragmentKind;
use zgui_layout::LayoutStore;
use zgui_layout::scroll_region;
use zgui_scene::{ClipId, ClipLink, ClipNode, ClipTable, Placements, SpatialId};

use crate::walk::stacking::children_in_paint_order;

/// Why a port's pixels may not be moved.
///
/// Reported rather than folded into a bool so that a counter can say which refusal a document keeps
/// hitting: "scrolling is not being shifted" and "scrolling is not being shifted *because the list
/// has no background*" are different bugs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The element does not generate a box, or generates no scrollable region.
    NotAScroller,
    /// Its background lets what is behind it through, so the composite is not the content's alone.
    NotOpaque,
    /// Something outside the container draws inside the port.
    Overdrawn,
    /// A rounded corner, non-rectangular clip, filter, or non-rigid transform shapes pixels inside
    /// the port.
    Clipped,
}

/// Whether `container`'s scrollport may have its pixels moved, and the port if it may.
///
/// The port is in device pixels, in the same space the damage set is measured in: `placements`
/// puts it there, and `clips` intersects it against every ancestor clip chain. The tests below
/// compare it against other fragments on the device too, because fragments outside the container
/// can measure their rectangles in other anchors.
pub fn port_may_be_shifted(
    store: &LayoutStore,
    container: NodeKey,
    placements: &Placements,
    clips: &ClipTable,
    scale: f32,
) -> Result<Rect<DevicePx, Device>, Refusal> {
    let Some(&box_key) = store.boxes_of(container).first() else {
        return Err(Refusal::NotAScroller);
    };
    if scroll_region::region_of(store, box_key).is_none() {
        return Err(Refusal::NotAScroller);
    }
    // An overlay scrollbar paints its floating capsule inside the scrollport over the scrolled
    // content, so blitting the port's pixels would smear the floating thumb.
    if store.scrollbar_mode(box_key) == scroll_region::ScrollbarMode::Overlay {
        return Err(Refusal::Overdrawn);
    }
    let Some(port) = scrollport_of(store, box_key) else {
        return Err(Refusal::NotAScroller);
    };
    let piece = store
        .fragments_of_box(box_key)
        .first()
        .and_then(|frag| store.fragment(*frag))
        .ok_or(Refusal::NotAScroller)?;
    let Some(own_scale) = axis_aligned_scale(piece.transform, placements) else {
        return Err(Refusal::Clipped);
    };
    // The port is the padding box, which is measured in the space the box is laid out in, as
    // every border box the tests below read.
    let port =
        zgui_layout::fragment::transform::placed::onto_device(port, piece.transform, placements);
    let port = clip_port(port, piece.clip, clips, placements)?;
    let node = store.get(box_key).ok_or(Refusal::NotAScroller)?;
    let padding_box = piece.padding_box();
    let outer_radii = crate::lower::border::radii_of(&node.style, piece.border_box, scale);
    let padding_radii = crate::lower::border::inner_radii(outer_radii, piece.border)
        .fit_within(padding_box.size);
    let padding_on_device = zgui_layout::fragment::transform::placed::onto_device(
        padding_box,
        piece.transform,
        placements,
    );
    if !rounded_rect_covers(
        padding_on_device,
        scale_radii(padding_radii, own_scale),
        port,
    ) {
        return Err(Refusal::Clipped);
    }
    ancestor_effects_allow_shift(store, box_key)?;
    let device = OnDevice {
        store,
        placements,
        scale,
    };
    // Two independent conditions, and neither implies the other. A port with an opaque background
    // of its own still cannot be moved if a dialog is drawn over it, and a port with nothing over
    // it still cannot be moved if what shows through it is a gradient.
    if !only_this_box_draws_in(device, box_key, port) {
        return Err(Refusal::Overdrawn);
    }
    backing(device, box_key, port)?;
    Ok(port)
}

/// The store, the matrices that put its fragments on the device, and the surface scale.
#[derive(Clone, Copy)]
struct OnDevice<'a> {
    /// The boxes and their fragments.
    store: &'a LayoutStore,
    /// The matrices of the drawn frame.
    placements: &'a Placements,
    /// Device pixels per CSS pixel, for resolving corner radii.
    scale: f32,
}

/// Where the pixels `box_key` scrolls stand, in the space the container is laid out in.
///
/// A scroll region reports its port at the origin, because that is the space its offsets are
/// measured in. Every test below compares the port against fragments on the device. So the port
/// is taken from the container's own fragment: the padding box, less the scrollbar gutter on
/// the sides the layout engine took it off. A port read at the origin names the pixels at the top
/// left of the window, which only coincide with the list's when the list stands there.
fn scrollport_of(store: &LayoutStore, box_key: BoxKey) -> Option<Rect<DevicePx, Device>> {
    let layout = store.layout_of(box_key)?;
    let piece = store.fragment(*store.fragments_of_box(box_key).first()?)?;
    let padding = piece.padding_box();
    let width = (padding.size.width.0 - layout.scrollbar_size.width.0).max(0.0);
    let height = (padding.size.height.0 - layout.scrollbar_size.height.0).max(0.0);
    Some(Rect::new(
        padding.origin,
        Size::new(DevicePx(width), DevicePx(height)),
    ))
}

/// Whether nothing painted *after* `box_key` puts ink inside `port`.
///
/// Walks from the box up to the root, and at each level asks which of that level's children paint
/// after the one leading down to the container. Only those can be over it; anything earlier is
/// behind it and is [`backing`]'s question rather than this one's.
///
/// A later sibling whose whole folded subtree misses the port is dismissed without being entered,
/// so the cost is the container's depth and the fan-out along it rather than the document.
fn only_this_box_draws_in(device: OnDevice<'_>, box_key: BoxKey, port: Pixels) -> bool {
    let store = device.store;
    let Some(root) = store.root() else {
        return false;
    };
    let mut inside = box_key;
    while inside != root {
        let Some(parent) = store.get(inside).and_then(|node| node.parent) else {
            return false;
        };
        let order = children_in_paint_order(store, parent);
        let Some(position) = order.iter().position(|&child| child == inside) else {
            return false;
        };
        for &later in &order[position + 1..] {
            if paints_in(device, later, port) {
                return false;
            }
        }
        inside = parent;
    }
    true
}

/// Whether what the port is composited over is a uniform colour that nothing draws across.
///
/// The composed pixels inside a scrollport are the composite of everything under it, and moving
/// them moves all of it. That is the same picture on one condition: everything the content was
/// composited *over* is one flat colour, so translating it is the identity, and nothing was
/// composited *between* that colour and the content.
///
/// It is deliberately not "the scroll container has a background". A list usually does not — it
/// inherits the page's — and requiring one would refuse the ordinary case while accepting a
/// contrived one. What is required is that somewhere at or above the container there is an opaque
/// solid colour covering the port, and that between it and the content nothing else marks a pixel
/// inside the port.
///
/// The walk is up the ancestors, and at every level it is the same three questions, so it costs the
/// container's depth and the fan-out along it — never the document.
fn backing(device: OnDevice<'_>, box_key: BoxKey, port: Pixels) -> Result<(), Refusal> {
    let store = device.store;
    let Some(root) = store.root() else {
        return Err(Refusal::NotAScroller);
    };

    // The container's own background comes first: one that is opaque and flat hides everything
    // behind it, and then nothing above matters at all.
    if flat_and_opaque_over(device, box_key, port) {
        return Ok(());
    }

    let mut inside = box_key;
    loop {
        let Some(parent) = store.get(inside).and_then(|node| node.parent) else {
            // Not under the root this frame, which is not a document anything should be moving
            // pixels for.
            return Err(Refusal::Overdrawn);
        };

        // Every sibling at this level that paints *before* the one leading down to the container.
        // It is painted over whatever backs it and under the content, so it is between them, and it
        // does not scroll. What paints after is the other condition's to refuse.
        let order = children_in_paint_order(store, parent);
        let position = order.iter().position(|&child| child == inside);
        for &sibling in &order[..position.unwrap_or(0)] {
            if paints_in(device, sibling, port) {
                return Err(Refusal::Overdrawn);
            }
        }

        // The parent's own painting is behind everything at this level. Flat and opaque over the
        // whole port, it *is* the backing and the walk is done.
        if flat_and_opaque_over(device, parent, port) {
            return Ok(());
        }
        // Anything else it marks inside the port is between whatever backs it and the content.
        if marks_pixels_in(device, parent, port) {
            return Err(Refusal::NotOpaque);
        }
        if parent == root {
            // Nothing at or below the root declared a colour for these pixels. What is actually
            // behind them is whatever the surface was last cleared to, which is not this walk's to
            // know.
            return Err(Refusal::NotOpaque);
        }
        inside = parent;
    }
}

/// Whether `key` paints one flat opaque colour across the whole of `port`.
///
/// Flat because a gradient or an image is not the same picture when it is translated, and opaque
/// because anything less lets what is behind it into the composite. Across the whole port, because
/// a colour that covers half of it backs half of it — including avoiding any rounded corner of
/// `key`'s own border box.
fn flat_and_opaque_over(device: OnDevice<'_>, key: BoxKey, port: Pixels) -> bool {
    let store = device.store;
    let Some(style) = store.get(key).map(|node| &node.style) else {
        return false;
    };
    if !paints_opaquely(style) {
        return false;
    }
    // The background is painted over the border box clipped by its own border radii, so that
    // rounded shape has to cover the entire port.
    store
        .fragments_of_box(key)
        .first()
        .and_then(|frag| store.fragment(*frag))
        .is_some_and(|piece| {
            let Some(scale) = axis_aligned_scale(piece.transform, device.placements) else {
                return false;
            };
            let border_box = zgui_layout::fragment::transform::placed::onto_device(
                piece.border_box,
                piece.transform,
                device.placements,
            );
            let outer_radii = crate::lower::border::radii_of(style, piece.border_box, device.scale);
            rounded_rect_covers(border_box, scale_radii(outer_radii, scale), port)
        })
}

/// Whether a box's own background hides everything behind it, with one flat colour.
///
/// A solid, fully opaque colour is the one case where moving a composite is exactly moving the
/// content drawn on it: the colour is uniform, so translating it is the identity, and nothing
/// behind it contributed a pixel. A gradient or an image is not uniform and would smear; a
/// translucent colour, partial opacity, blend mode, filter, or clip-path lets what is behind it
/// through or alters edge pixels, and what is behind it did not scroll.
fn paints_opaquely(style: &ComputedStyle) -> bool {
    let effects = style.get_effects();
    if effects.opacity < 1.0
        || effects.mix_blend_mode != MixBlendModeValue::Normal
        || !effects.filter.0.is_empty()
        || !effects.backdrop_filter.0.is_empty()
        || crate::lower::clip::of(style).needs_target()
    {
        return false;
    }
    let background = style.get_background();
    // Not `is_empty`: the initial value of `background-image` is a one-element list holding `none`,
    // so an emptiness test refuses every box in every document.
    if background
        .background_image
        .0
        .iter()
        .any(|image| !matches!(image, ImageValue::None))
    {
        return false;
    }
    // `currentColor` resolves against the box's own colour, which is already absolute here.
    let colour = background
        .background_color
        .resolve_to_absolute(&style.get_inherited_text().clone_color());
    colour.alpha >= 1.0
}

/// Intersects `port` with every ancestor clip link and refuses any non-rectangular or rounded clip
/// whose curve or mask touches the resulting rectangle.
fn clip_port(
    mut port: Pixels,
    clip: ClipId,
    clips: &ClipTable,
    placements: &Placements,
) -> Result<Pixels, Refusal> {
    if port.is_empty() {
        return Err(Refusal::NotAScroller);
    }
    let mut rounded: SmallVec<[(Pixels, Corners<Vec2<DevicePx>>); 4]> = SmallVec::new();
    let mut cursor = clip;
    while let Some(ClipNode::Link { link, parent, .. }) = clips.get(cursor) {
        match *link {
            ClipLink::Mask { .. } => return Err(Refusal::Clipped),
            ClipLink::RoundedRect {
                rect, radii, space, ..
            } => {
                let Some(scale) = axis_aligned_scale(Some(space), placements) else {
                    return Err(Refusal::Clipped);
                };
                let clip_rect = placements.place(space, rect);
                let Some(next) = port.intersection(clip_rect) else {
                    return Err(Refusal::Clipped);
                };
                if next.is_empty() {
                    return Err(Refusal::Clipped);
                }
                port = next;
                if !radii.is_square() {
                    rounded.push((clip_rect, scale_radii(radii, scale)));
                }
            }
        }
        cursor = *parent;
    }
    for (clip_rect, radii) in rounded {
        if !rounded_rect_covers(clip_rect, radii, port) {
            return Err(Refusal::Clipped);
        }
    }
    Ok(port)
}

/// Whether `box_key` and every ancestor above it avoid compositing effects that couple shifted
/// pixels to non-scrolling surroundings (opacity, filters, blend modes, or clip-paths).
fn ancestor_effects_allow_shift(store: &LayoutStore, box_key: BoxKey) -> Result<(), Refusal> {
    let Some(node) = store.get(box_key) else {
        return Err(Refusal::NotAScroller);
    };
    let own = node.style.get_effects();
    if crate::lower::clip::of(&node.style).needs_target() || !own.filter.0.is_empty() {
        return Err(Refusal::Clipped);
    }
    if own.mix_blend_mode != MixBlendModeValue::Normal || !own.backdrop_filter.0.is_empty() {
        return Err(Refusal::NotOpaque);
    }
    let mut cursor = node.parent;
    while let Some(ancestor) = cursor {
        let Some(ancestor_node) = store.get(ancestor) else {
            return Err(Refusal::Overdrawn);
        };
        let effects = ancestor_node.style.get_effects();
        if crate::lower::clip::of(&ancestor_node.style).needs_target()
            || !effects.filter.0.is_empty()
        {
            return Err(Refusal::Clipped);
        }
        if effects.opacity < 1.0 || effects.mix_blend_mode != MixBlendModeValue::Normal {
            return Err(Refusal::NotOpaque);
        }
        cursor = ancestor_node.parent;
    }
    Ok(())
}

/// Positive axis-aligned scale factors for `space`, or `None` when rotated, skewed, flipped, or
/// perspective-projected.
fn axis_aligned_scale(space: Option<SpatialId>, placements: &Placements) -> Option<(f32, f32)> {
    let Some(id) = space else {
        return Some((1.0, 1.0));
    };
    let Some(matrix) = placements.get(id) else {
        return Some((1.0, 1.0));
    };
    let [x, y, z, w] = matrix.columns;
    let axis_aligned = x[1] == 0.0
        && x[2] == 0.0
        && x[3] == 0.0
        && y[0] == 0.0
        && y[2] == 0.0
        && y[3] == 0.0
        && z[0] == 0.0
        && z[1] == 0.0
        && z[3] == 0.0
        && w[3] == 1.0
        && x[0] > 0.0
        && y[1] > 0.0;
    axis_aligned.then_some((x[0], y[1]))
}

/// Scales elliptical corner radii by positive axis-aligned device scale factors.
fn scale_radii(radii: Corners<Vec2<DevicePx>>, (sx, sy): (f32, f32)) -> Corners<Vec2<DevicePx>> {
    let scale_one = |radius: Vec2<DevicePx>| Vec2::new(DevicePx(radius.x.0 * sx), DevicePx(radius.y.0 * sy));
    Corners {
        top_left: scale_one(radii.top_left),
        top_right: scale_one(radii.top_right),
        bottom_right: scale_one(radii.bottom_right),
        bottom_left: scale_one(radii.bottom_left),
    }
}

/// Whether `target` lies wholly inside `rect` without entering any rounded corner's bounding box
/// (plus a 1-device-pixel anti-aliasing margin on non-square corners).
fn rounded_rect_covers(rect: Pixels, radii: Corners<Vec2<DevicePx>>, target: Pixels) -> bool {
    if target.is_empty() || rect.intersection(target) != Some(target) {
        return false;
    }
    // One extra device pixel keeps anti-aliased corner coverage and curved border fringes out of
    // the copied rectangle regardless of corner shape (ellipse or squircle).
    const AA_MARGIN: f32 = 1.0;
    let tl = radii.top_left;
    if tl.x.0 > 0.0
        && tl.y.0 > 0.0
        && target.left().0 < rect.left().0 + tl.x.0 + AA_MARGIN
        && target.top().0 < rect.top().0 + tl.y.0 + AA_MARGIN
    {
        return false;
    }
    let tr = radii.top_right;
    if tr.x.0 > 0.0
        && tr.y.0 > 0.0
        && target.right().0 > rect.right().0 - tr.x.0 - AA_MARGIN
        && target.top().0 < rect.top().0 + tr.y.0 + AA_MARGIN
    {
        return false;
    }
    let br = radii.bottom_right;
    if br.x.0 > 0.0
        && br.y.0 > 0.0
        && target.right().0 > rect.right().0 - br.x.0 - AA_MARGIN
        && target.bottom().0 > rect.bottom().0 - br.y.0 - AA_MARGIN
    {
        return false;
    }
    let bl = radii.bottom_left;
    if bl.x.0 > 0.0
        && bl.y.0 > 0.0
        && target.left().0 < rect.left().0 + bl.x.0 + AA_MARGIN
        && target.bottom().0 > rect.bottom().0 - bl.y.0 - AA_MARGIN
    {
        return false;
    }
    true
}

/// Whether `key`'s own fragments mark a pixel inside `port`, ignoring its descendants.
fn marks_pixels_in(device: OnDevice<'_>, key: BoxKey, port: Pixels) -> bool {
    let store = device.store;
    store.fragments_of_box(key).iter().any(|&frag| {
        store.fragment(frag).is_some_and(|piece| {
            piece
                .device_ink(device.placements)
                .intersection(port)
                .is_some()
                && marks_pixels(store, key, piece.kind)
        })
    })
}

/// Whether anything in `key`'s subtree actually marks a pixel inside `port`.
///
/// The folded subtree ink is the *refusal* test and not the answer: a container whose descendants
/// straddle the port but whose painted boxes all avoid it marks nothing there, and taking the union
/// as the answer refuses every document with a full-height sibling in it. So the union is used the
/// way the emit walk uses it — to dismiss a whole subtree in constant time — and the question is
/// then asked of each box's own ink.
fn paints_in(device: OnDevice<'_>, key: BoxKey, port: Pixels) -> bool {
    let store = device.store;
    let Some(folded) = subtree_ink(device, key) else {
        return false;
    };
    if folded.intersection(port).is_none() {
        return false;
    }
    for &frag in store.fragments_of_box(key) {
        let Some(piece) = store.fragment(frag) else {
            continue;
        };
        if piece
            .device_ink(device.placements)
            .intersection(port)
            .is_none()
        {
            continue;
        }
        if marks_pixels(store, key, piece.kind) {
            return true;
        }
    }
    store.get(key).is_some_and(|node| {
        node.children
            .iter()
            .any(|&child| paints_in(device, child, port))
    })
}

/// Whether a fragment of this box actually marks a pixel, rather than merely occupying a rectangle.
///
/// A fragment's ink is its geometric extent, so a full-height wrapper with no background "inks" the
/// whole window while drawing nothing at all. Asking the extent alone therefore refuses every
/// document with a layout wrapper in it, which is every document.
///
/// The lowerings are the paint stage's own, so this cannot drift from what is actually drawn: if
/// something new starts marking pixels, it starts marking them here too.
fn marks_pixels(store: &LayoutStore, key: BoxKey, kind: FragmentKind) -> bool {
    // Anything that is not a plain box is content: a run of text, a replaced element, a drawing, a
    // scrollbar. All of them draw.
    if !matches!(kind, FragmentKind::Box) {
        return true;
    }
    let Some(style) = store.get(key).map(|node| &node.style) else {
        return false;
    };
    let background = crate::lower::background::of(style);
    if background.color.alpha() > 0.0 || !background.layers.is_empty() {
        return true;
    }
    if !crate::lower::border::of(style).invisible {
        return true;
    }
    if crate::lower::outline::of(style, 1.0).is_some() {
        return true;
    }
    !style.get_effects().box_shadow.0.is_empty()
}

/// The union of a box's ink and everything below it, as the last fragment pass folded it, on the
/// device.
fn subtree_ink(device: OnDevice<'_>, key: BoxKey) -> Option<Pixels> {
    let store = device.store;
    let frag = *store.fragments_of_box(key).first()?;
    store
        .fragment(frag)
        .map(|fragment| fragment.device_subtree_ink(device.placements))
}

/// A rectangle in device pixels.
type Pixels = Rect<DevicePx, Device>;
