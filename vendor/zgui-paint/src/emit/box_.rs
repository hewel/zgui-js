//! A box's own decorations: its shadows, its background, its border and its outline.
//!
//! The order here is CSS 2.1 Appendix E's, and it is the whole reason these four live in one
//! function: shadows behind the background, the background behind the border, and the outline
//! last — after the box's descendants, which is why it is emitted separately rather than in the
//! run below.

use smallvec::SmallVec;
use zgui_color::Color;
use zgui_geom::{Corners, Device, DevicePx, Edges, Point, Rect, Size, Vec2};
use zgui_scene::{
    ClipId, Paint, PaintOwner, PaintRef, PaintRole, PropertyOwner, Quad, Scene, Shadow, SpatialId,
};

use crate::emit::paint::gradient_paint;
use crate::lower::PaintStyle;
use crate::lower::anim::{OwnedPaints, border_paint};
use crate::lower::border::{LineStyle, inner_radii};

/// Where a box's decorations are drawn.
#[derive(Clone, Copy, Debug)]
pub struct BoxPlacement {
    /// The border box, in absolute device pixels.
    pub border_box: Rect<DevicePx, Device>,
    /// The border widths, already resolved and snapped.
    pub border: Edges<DevicePx>,
    /// The border box's elliptical corner radii.
    pub radii: Corners<Vec2<DevicePx>>,
    /// The chain the box is drawn through.
    pub clip: ClipId,
    /// The transform it is drawn under.
    pub transform: SpatialId,
    /// How many device pixels one CSS pixel is, for resolving a gradient's stop positions.
    pub scale: f32,
    /// The box that owns the slots its animated colours are drawn through, and which colours.
    ///
    /// `None` for a box with no animated colour, which draws through interned paints.
    pub owned: Option<(PropertyOwner, OwnedPaints)>,
}

impl BoxPlacement {
    /// The owned slots this placement draws `role` through, if it does.
    fn owner_of(&self, role: PaintRole) -> Option<PaintOwner> {
        let (owner, paints) = self.owned?;
        let drawn = match role {
            PaintRole::Background => paints.background(),
            PaintRole::BorderSides => paints.border(),
            PaintRole::Outline => paints.outline(),
        };
        drawn.then_some(PaintOwner::new(owner, role))
    }
}

/// Emits the shadows a box casts outwards, behind everything else it draws.
pub fn outer_shadows(scene: &mut Scene, style: &PaintStyle, placement: BoxPlacement) -> usize {
    shadows(scene, style, placement, false)
}

/// Emits the shadows a box casts inwards, over its background and under its border.
pub fn inset_shadows(scene: &mut Scene, style: &PaintStyle, placement: BoxPlacement) -> usize {
    shadows(scene, style, placement, true)
}

/// Emits one direction's shadows, last written first so the first written ends up on top.
///
/// A drop shadow is cast by the border box. An inset shadow is clipped to the padding box, so it
/// never covers the border, and its hole is concentric with the padding box's corners.
fn shadows(scene: &mut Scene, style: &PaintStyle, placement: BoxPlacement, inset: bool) -> usize {
    let (element, element_radii) = if inset {
        (
            placement.border_box.inset(placement.border),
            inner_radii(placement.radii, placement.border),
        )
    } else {
        (placement.border_box, placement.radii)
    };
    if inset && element.is_empty() {
        return 0;
    }
    let mut pushed = 0;
    for spec in style.shadows.iter().rev() {
        if spec.inset != inset || spec.is_invisible() {
            continue;
        }
        let mut shadow = if inset {
            Shadow::inset_shadow(
                element,
                (spec.offset_x, spec.offset_y),
                spec.spread,
                spec.deviation,
                spec.color,
            )
        } else {
            Shadow::drop_shadow(
                element,
                (spec.offset_x, spec.offset_y),
                spec.spread,
                spec.deviation,
                spec.color,
            )
        };
        // A spread grows the shape a drop shadow casts and *shrinks* the one an inset shadow leaves
        // open, so the corners of the two move in opposite directions: an inset shadow's shape is
        // concentric inside the box, and growing its radii would round the hole the wrong way.
        let spread = if inset { -spec.spread } else { spec.spread };
        shadow.radii = flatten(grow(element_radii, spread));
        shadow.element_radii = flatten(element_radii);
        // A shadow is the box's own shape blurred, so it is cut the same way: a squircle casting a
        // rounded-rectangle shadow shows the shadow's corners outside its own.
        shadow.shape = style.corner_shape.get();
        shadow.transform = placement.transform.index();
        pushed += usize::from(scene.push_shadow(shadow.clipped(placement.clip)).is_some());
    }
    pushed
}

/// Emits a box's background and its border, which are one quad when the background is a flat colour.
///
/// A gradient layer is a quad of its own drawn over the colour, and the border rides on the last
/// quad pushed so that its stroke is drawn against the outermost fill rather than under it.
pub fn background_and_border(
    scene: &mut Scene,
    style: &PaintStyle,
    placement: BoxPlacement,
    shaders: &dyn crate::content::shader::ShaderSource,
    pointer: Option<zgui_geom::Point<zgui_geom::DevicePx, zgui_geom::Device>>,
) -> usize {
    if let Some(pushed) = shaded(scene, style, placement, shaders, pointer) {
        return pushed;
    }
    let (fills, draws_border) = fills_of(scene, style, placement);
    let borders = border_widths(placement.border);
    let uniform_border = draws_border
        .then(|| style.border.single_quad_style(borders))
        .flatten();
    if let Some(border_style) = uniform_border {
        let stroke = match placement.owner_of(PaintRole::BorderSides) {
            Some(name) => scene
                .paints
                .owned(name, border_paint(style.border.colors, placement.border)),
            None => border_stroke(scene, style, borders, true),
        };
        let last = fills.len().saturating_sub(1);
        let mut pushed = 0;
        for (index, fill) in fills.into_iter().enumerate() {
            pushed += usize::from(
                push(
                    scene,
                    placement,
                    style,
                    fill,
                    (index == last).then_some((borders, stroke, border_style)),
                )
                .is_some(),
            );
        }
        return pushed;
    }
    let mut pushed = 0;
    for fill in fills {
        if fill == PaintRef::NONE {
            continue;
        }
        pushed += usize::from(push(scene, placement, style, fill, None).is_some());
    }
    if draws_border {
        pushed += emit_mixed_borders(scene, placement, style, borders);
    }
    pushed
}

/// Emits independent per-side border quads when sides mix line styles or include `double`.
fn emit_mixed_borders(
    scene: &mut Scene,
    placement: BoxPlacement,
    style: &PaintStyle,
    borders: [f32; 4],
) -> usize {
    let mut pushed = 0;
    let owned_stroke = placement.owner_of(PaintRole::BorderSides).map(|name| {
        scene
            .paints
            .owned(name, border_paint(style.border.colors, placement.border))
    });
    for line in [LineStyle::Solid, LineStyle::Dashed, LineStyle::Dotted] {
        let mut masked = [0.0f32; 4];
        let mut any = false;
        for side in 0..4 {
            if style.border.styles[side] == line
                && borders[side] > 0.0
                && (owned_stroke.is_some() || style.border.colors[side].alpha() > 0.0)
            {
                masked[side] = borders[side];
                any = true;
            }
        }
        if !any {
            continue;
        }
        let stroke = owned_stroke.unwrap_or_else(|| border_stroke(scene, style, masked, true));
        pushed += usize::from(
            push(
                scene,
                placement,
                style,
                PaintRef::NONE,
                Some((masked, stroke, line.to_scene())),
            )
            .is_some(),
        );
    }
    let mut outer_widths = [0.0f32; 4];
    let mut inner_widths = [0.0f32; 4];
    let mut inner_insets = [0.0f32; 4];
    let mut any_outer = false;
    let mut any_inner = false;
    for side in 0..4 {
        let width = borders[side];
        if style.border.styles[side] != LineStyle::Double
            || width <= 0.0
            || (owned_stroke.is_none() && style.border.colors[side].alpha() == 0.0)
        {
            inner_insets[side] = width;
            continue;
        }
        if width < 3.0 {
            outer_widths[side] = width;
            any_outer = true;
            inner_insets[side] = width;
        } else {
            let band = (width / 3.0).floor().max(1.0);
            outer_widths[side] = band;
            inner_widths[side] = band;
            inner_insets[side] = width - band;
            any_outer = true;
            any_inner = true;
        }
    }
    if any_outer {
        let stroke = owned_stroke.unwrap_or_else(|| border_stroke(scene, style, outer_widths, true));
        pushed += usize::from(
            push(
                scene,
                placement,
                style,
                PaintRef::NONE,
                Some((outer_widths, stroke, zgui_scene::prim::quad::BorderStyle::Solid)),
            )
            .is_some(),
        );
    }
    if any_inner {
        let inset_edges = Edges {
            top: DevicePx(inner_insets[0]),
            right: DevicePx(inner_insets[1]),
            bottom: DevicePx(inner_insets[2]),
            left: DevicePx(inner_insets[3]),
        };
        let inner_box = placement.border_box.inset(inset_edges);
        if !inner_box.is_empty() {
            let inner_placement = BoxPlacement {
                border_box: inner_box,
                radii: inner_radii(placement.radii, inset_edges),
                ..placement
            };
            let stroke =
                owned_stroke.unwrap_or_else(|| border_stroke(scene, style, inner_widths, true));
            pushed += usize::from(
                push(
                    scene,
                    inner_placement,
                    style,
                    PaintRef::NONE,
                    Some((inner_widths, stroke, zgui_scene::prim::quad::BorderStyle::Solid)),
                )
                .is_some(),
            );
        }
    }
    pushed
}

/// Emits a box's background layers clipped to a CSS `clip-path` basic shape.
pub(crate) fn clipped_background(
    scene: &mut Scene,
    id: zgui_scene::VectorId,
    style: &PaintStyle,
    placement: BoxPlacement,
    clip_shape: &crate::lower::ClipPathShape,
    reference_box: Rect<DevicePx, Device>,
) -> usize {
    use zgui_scene::kurbo::Shape as _;

    if placement.border_box.is_empty() {
        return 0;
    }
    let Some((mut path, fill_rule)) = clip_shape.to_device_path(reference_box, placement.scale)
    else {
        return 0;
    };
    let (fills, _) = fills_of(scene, style, placement);
    if fills.is_empty() {
        return 0;
    }
    let dx = f64::from(reference_box.origin.x.0 - placement.border_box.origin.x.0);
    let dy = f64::from(reference_box.origin.y.0 - placement.border_box.origin.y.0);
    if dx != 0.0 || dy != 0.0 {
        path = zgui_scene::kurbo::Affine::translate((dx, dy)) * path;
    }
    let path = std::sync::Arc::new(path);
    let place = zgui_scene::kurbo::Affine::translate((
        f64::from(placement.border_box.origin.x.0),
        f64::from(placement.border_box.origin.y.0),
    ));
    let local = placement.border_box;
    let ink = crate::emit::vector::under(scene, placement.transform, local);
    // Clip the vector path in its own local space to [0..width, 0..height] and clip the
    // composited item to the element's border box so shapes extending past border_box
    // (e.g., circle(45%) on a wide/short card) never bleed outside the box or corrupt
    // neighboring rasterized regions.
    let box_rect = zgui_scene::kurbo::Rect::new(
        0.0,
        0.0,
        f64::from(placement.border_box.size.width.0),
        f64::from(placement.border_box.size.height.0),
    );
    let vector_clips = vec![zgui_scene::VectorClip {
        path: std::sync::Arc::new(box_rect.to_path(0.05)),
        rule: zgui_scene::peniko::Fill::NonZero,
    }];
    let box_clip = scene.clips.push(
        placement.clip,
        zgui_scene::ClipLink::shaped(
            placement.border_box,
            placement.radii,
            style.corner_shape,
            placement.transform,
        ),
    );
    scene.note_minted_clip(box_clip);
    let mut pushed = 0;
    for fill in fills {
        if fill == PaintRef::NONE {
            continue;
        }
        let mut item = zgui_scene::VectorItem::filled(id, std::sync::Arc::clone(&path), fill)
            .placed(place)
            .clipped(box_clip)
            .inside(vector_clips.clone());
        item.fill_rule = fill_rule;
        item.ink = ink;
        item.local_ink = local;
        item.transform = Some(placement.transform);
        pushed += usize::from(scene.push_vector(item).is_some());
    }
    pushed
}

/// Every paint a box's background resolves to, last written first, and whether it draws a border.
///
/// CSS paints the first layer on top, so the list is reversed here and the border goes on whatever
/// is drawn last — which is not knowable from the layer *count*, because a layer that resolves to
/// no paint at all is not drawn. It is shared with the shaded path because a box whose shape an
/// effect decides is filled by exactly the same cascade: a gradient, an image and a colour all keep
/// working, and a list built twice is a list the two paths can disagree about.
fn fills_of(
    scene: &mut Scene,
    style: &PaintStyle,
    placement: BoxPlacement,
) -> (SmallVec<[PaintRef; 2]>, bool) {
    let mut fills: SmallVec<[PaintRef; 2]> = SmallVec::new();
    // An owned colour is drawn at any alpha, so the slot is there when an animation brings the
    // colour in from transparent.
    if let Some(name) = placement.owner_of(PaintRole::Background) {
        fills.push(
            scene
                .paints
                .owned(name, Paint::Solid(style.background.color)),
        );
    } else if style.background.color.alpha() != 0.0 {
        fills.push(
            scene
                .paints
                .add(zgui_scene::Paint::Solid(style.background.color)),
        );
    }
    // A ramp is resolved against the box with its corner at the origin rather than where the box
    // is standing, so what it is interned under is the box's *size* and the gradient written on
    // it. Two identical rows anywhere on the surface then share one entry, and a box carried along
    // by a scroll goes on sharing the entry it already had instead of minting one per position for
    // as long as the scroll runs. Where the box is standing is put back by the quad, which carries
    // the corner as the origin its paints are described from.
    let local = Rect::new(
        Point::new(DevicePx(0.0), DevicePx(0.0)),
        placement.border_box.size,
    );
    for layer in style.background.layers.iter().rev() {
        if let Some(fill) = gradient_paint(scene, layer, local, placement.scale) {
            fills.push(fill);
        }
    }
    let draws_border = (!style.border.invisible
        || placement.owner_of(PaintRole::BorderSides).is_some())
        && border_widths(placement.border)
            .iter()
            .any(|width| *width > 0.0);
    // A box with a border and no fill at all still draws its border, and that is the case a
    // fill-driven loop misses entirely.
    if fills.is_empty() && draws_border {
        fills.push(PaintRef::NONE);
    }
    (fills, draws_border)
}

/// Emits the box through the effect a style sheet named, or `None` when it named none.
///
/// A coverage effect reshapes the box: the fill and the border are the ordinary paints, and only
/// which pixels are inside is the effect's. A paint effect replaces the background outright, and
/// the border is left to the ordinary quad beneath it, so that a shaded box with a border still
/// has one.
///
/// A name nothing answers, or a device that cannot draw an effect at all, falls back to the
/// ordinary path — a smoothed corner becomes a rounded one, which is the appearance the box would
/// have had.
fn shaded(
    scene: &mut Scene,
    style: &PaintStyle,
    placement: BoxPlacement,
    shaders: &dyn crate::content::shader::ShaderSource,
    pointer: Option<zgui_geom::Point<zgui_geom::DevicePx, zgui_geom::Device>>,
) -> Option<usize> {
    let named = style.shader.as_ref()?.named.as_ref()?;
    let binding = shaders.effect(&named.name)?;
    if binding.mode != named.mode {
        // The property said what the effect is expected to do and the effect does something else.
        // Drawing it anyway would put a colour where a shape was asked for.
        return None;
    }
    let params = scene.shader_params.intern(crate::emit::shader::block_at(
        named,
        binding,
        pointer,
        placement.border_box,
    ));
    let shaped = binding.mode == zgui_scene::ShaderMode::Coverage;
    // A paint effect shades the whole box itself, so it is one rectangle and reads no fill. A
    // coverage effect decides only which pixels are inside, so it is drawn once per background
    // layer — the same list, in the same order, with the border on the last — and the cascade goes
    // on deciding what fills the shape.
    let (fills, draws_border) = if shaped {
        fills_of(scene, style, placement)
    } else {
        (SmallVec::from_slice(&[PaintRef::NONE]), false)
    };
    let borders = border_widths(placement.border);
    // The shaded pipeline strokes in one colour.
    let stroke = draws_border.then(|| border_stroke(scene, style, borders, false));
    let last = fills.len().saturating_sub(1);
    let mut pushed = 0;
    for (index, fill) in fills.into_iter().enumerate() {
        let mut shaded = zgui_scene::ShadedQuad::new(placement.border_box, binding.id, params)
            .clipped(placement.clip)
            .transformed(placement.transform)
            .with_radii(placement.radii)
            .with_fill(fill);
        if let Some(stroke) = stroke.filter(|_| index == last) {
            shaded = shaded.with_border(borders, stroke);
        }
        // Only a paint that is read at a point has an origin to be read from, which is why a box
        // filled flat carries none.
        if shaded.samples_its_paint() {
            shaded.reanchor_paint(Size::new(
                placement.border_box.origin.x,
                placement.border_box.origin.y,
            ));
        }
        pushed += usize::from(scene.push_shaded(shaded).is_some());
    }
    Some(pushed)
}

/// Pushes one quad of a box, optionally carrying the border.
fn push(
    scene: &mut Scene,
    placement: BoxPlacement,
    style: &PaintStyle,
    fill: PaintRef,
    border: Option<([f32; 4], PaintRef, zgui_scene::prim::quad::BorderStyle)>,
) -> Option<zgui_scene::DrawOrder> {
    let mut quad = Quad::filled(placement.border_box, fill)
        .clipped(placement.clip)
        .transformed(placement.transform)
        .with_radii(placement.radii)
        .with_corner_shape(style.corner_shape);
    if let Some((widths, stroke, border_style)) = border {
        quad = quad.with_border(widths, stroke, border_style);
    }
    // Only a paint that is read at a point has an origin to be read from, and saying so is what
    // keeps every flat-filled box in the document carrying a zero it does not use.
    if quad.samples_its_paint() {
        quad.reanchor_paint(Size::new(
            placement.border_box.origin.x,
            placement.border_box.origin.y,
        ));
    }
    scene.push_quad(quad)
}

/// Emits a box's outline, which is drawn outside the border box and after the box's descendants.
pub fn outline(scene: &mut Scene, style: &PaintStyle, placement: BoxPlacement) -> usize {
    let Some(outline) = &style.outline else {
        return 0;
    };
    let reach = outline.offset + outline.width;
    let rect = placement.border_box.outset(Edges {
        top: DevicePx(reach),
        right: DevicePx(reach),
        bottom: DevicePx(reach),
        left: DevicePx(reach),
    });
    if rect.is_empty() {
        return 0;
    }
    let stroke = match placement.owner_of(PaintRole::Outline) {
        Some(name) => scene.paints.owned(name, Paint::Solid(outline.color)),
        None => scene.paints.add(Paint::Solid(outline.color)),
    };
    let quad = Quad::filled(rect, PaintRef::NONE)
        .clipped(placement.clip)
        .transformed(placement.transform)
        .with_radii(grow(placement.radii, reach))
        // An outline follows the box it outlines: a squircle with a square outline round it reads
        // as two boxes rather than one.
        .with_corner_shape(style.corner_shape)
        .with_border([outline.width; 4], stroke, outline.style.to_scene());
    usize::from(scene.push_quad(quad).is_some())
}

/// The border widths a quad carries, in top, right, bottom, left order.
fn border_widths(border: Edges<DevicePx>) -> [f32; 4] {
    [border.top.0, border.right.0, border.bottom.0, border.left.0]
}

/// The paint a box's border is stroked with, given the widths in top, right, bottom, left order.
///
/// The sides that have a width decide. One colour on all of them is a solid stroke. Different
/// colours are a [`Paint::Sides`](zgui_scene::Paint::Sides) stroke when `sides` allows it, and the
/// first visible colour when it does not.
fn border_stroke(scene: &mut Scene, style: &PaintStyle, widths: [f32; 4], sides: bool) -> PaintRef {
    let colors = style.border.colors;
    let shown = || {
        (0..4)
            .filter(|&side| widths[side] > 0.0)
            .map(|side| colors[side])
    };
    let color = match shown().next() {
        Some(first) if shown().all(|color| color == first) => first,
        Some(_) if sides => return scene.paints.add(zgui_scene::Paint::Sides(colors)),
        Some(first) => shown().find(|color| color.alpha() != 0.0).unwrap_or(first),
        // No side has a width, so the stroke draws nothing whatever its colour.
        None => colors
            .iter()
            .copied()
            .find(|color| color.alpha() != 0.0)
            .unwrap_or(Color::TRANSPARENT),
    };
    if color.alpha() == 0.0 {
        return PaintRef::NONE;
    }
    scene.paints.add(zgui_scene::Paint::Solid(color))
}

/// The eight-float form of four elliptical radii.
pub fn flatten(radii: Corners<Vec2<DevicePx>>) -> [f32; 8] {
    [
        radii.top_left.x.0,
        radii.top_left.y.0,
        radii.top_right.x.0,
        radii.top_right.y.0,
        radii.bottom_right.x.0,
        radii.bottom_right.y.0,
        radii.bottom_left.x.0,
        radii.bottom_left.y.0,
    ]
}

/// Radii grown outwards by `by`, which is what an outline's and a spread shadow's corners take.
///
/// A square corner stays square however far it is grown: growing a rectangle with a sharp corner
/// does not round it.
pub fn grow(radii: Corners<Vec2<DevicePx>>, by: f32) -> Corners<Vec2<DevicePx>> {
    let out = |radius: Vec2<DevicePx>| {
        if radius.x.0 == 0.0 && radius.y.0 == 0.0 {
            return radius;
        }
        Vec2::new(
            DevicePx((radius.x.0 + by).max(0.0)),
            DevicePx((radius.y.0 + by).max(0.0)),
        )
    };
    Corners {
        top_left: out(radii.top_left),
        top_right: out(radii.top_right),
        bottom_right: out(radii.bottom_right),
        bottom_left: out(radii.bottom_left),
    }
}

/// The radii of a box's padding box, which a caller clipping content to the box needs.
pub fn padding_radii(
    radii: Corners<Vec2<DevicePx>>,
    border: Edges<DevicePx>,
) -> Corners<Vec2<DevicePx>> {
    inner_radii(radii, border)
}

#[cfg(test)]
mod tests {
    use zgui_geom::{Corners, Device, DevicePx, Point, Rect, Size, Vec2};

    use super::{BoxPlacement, grow, inset_shadows, outer_shadows};
    use crate::lower::PaintStyle;
    use crate::lower::shadow::ShadowSpec;

    /// A 100 by 50 box with a uniform ten-pixel radius.
    fn placement() -> BoxPlacement {
        BoxPlacement {
            border_box: Rect::new(
                Point::new(DevicePx(0.0), DevicePx(0.0)),
                Size::new(DevicePx(100.0), DevicePx(50.0)),
            ),
            border: zgui_geom::Edges::ZERO,
            radii: Corners::uniform(Vec2::new(DevicePx(10.0), DevicePx(10.0))),
            clip: zgui_scene::ClipId::ROOT,
            transform: zgui_scene::SpatialId::VIEWPORT,
            scale: 1.0,
            owned: None,
        }
    }

    /// The initial style carrying one shadow with the given spread and direction.
    fn with_shadow(spread: f32, inset: bool) -> PaintStyle {
        let mut style = crate::lower::lower(&zgui_css::StyleDraft::initial().build(), 1.0);
        style.shadows.push(ShadowSpec {
            offset_x: 0.0,
            offset_y: 0.0,
            deviation: 0.0,
            spread,
            color: zgui_color::Color::BLACK,
            inset,
        });
        style
    }

    /// The corner radii of the only shadow a scene holds.
    fn only_shadow_radius(style: &PaintStyle, inset: bool) -> f32 {
        let mut scene = zgui_scene::Scene::new();
        scene.begin_frame(Size::<i32, Device>::new(256, 256));
        let pushed = if inset {
            inset_shadows(&mut scene, style, placement())
        } else {
            outer_shadows(&mut scene, style, placement())
        };
        assert_eq!(pushed, 1, "the fixture has to push exactly one shadow");
        scene.primitives.shadows[0].radii[0]
    }

    #[test]
    fn a_spread_grows_a_drop_shadows_corners_and_shrinks_an_inset_ones() {
        // The two shapes move in opposite directions: a drop shadow's shape is the box grown by the
        // spread, an inset shadow's is the box shrunk by it, and a corner stays concentric with the
        // edge it belongs to. One sign for both would round the hole an inset shadow leaves the
        // wrong way, by twice the spread.
        assert_eq!(only_shadow_radius(&with_shadow(6.0, false), false), 16.0);
        assert_eq!(only_shadow_radius(&with_shadow(6.0, true), true), 4.0);
    }

    /// The only inset shadow a 48-pixel box with a two-pixel border and six-pixel corners emits.
    fn bordered_inset(offset: (f32, f32), spread: f32) -> zgui_scene::Shadow {
        let mut style = with_shadow(spread, true);
        style.shadows[0].offset_x = offset.0;
        style.shadows[0].offset_y = offset.1;
        let placement = BoxPlacement {
            border_box: Rect::new(
                Point::new(DevicePx(10.0), DevicePx(10.0)),
                Size::new(DevicePx(48.0), DevicePx(48.0)),
            ),
            border: zgui_geom::Edges::uniform(DevicePx(2.0)),
            radii: Corners::uniform(Vec2::new(DevicePx(6.0), DevicePx(6.0))),
            ..placement()
        };
        let mut scene = zgui_scene::Scene::new();
        scene.begin_frame(Size::<i32, Device>::new(256, 256));
        assert_eq!(inset_shadows(&mut scene, &style, placement), 1);
        scene.primitives.shadows[0]
    }

    #[test]
    fn an_inset_spread_shrinks_the_hole_inside_the_padding_box() {
        let shadow = bordered_inset((0.0, 0.0), 2.0);
        assert_eq!(shadow.inset, 1);
        // The padding box: the border box less the two-pixel border.
        assert_eq!(shadow.bounds, [12.0, 12.0, 44.0, 44.0]);
        assert_eq!(shadow.element_bounds, [12.0, 12.0, 44.0, 44.0]);
        assert_eq!(shadow.element_radii, [4.0; 8]);
        // The hole: the padding box less the spread, with concentric corners.
        assert_eq!(shadow.shape_bounds, [14.0, 14.0, 40.0, 40.0]);
        assert_eq!(shadow.radii, [2.0; 8]);
    }

    #[test]
    fn an_inset_offset_moves_the_hole_and_keeps_the_painted_box() {
        let shadow = bordered_inset((0.0, 2.0), 0.0);
        assert_eq!(shadow.bounds, [12.0, 12.0, 44.0, 44.0]);
        assert_eq!(shadow.shape_bounds, [12.0, 14.0, 44.0, 44.0]);
    }

    #[test]
    fn a_square_corner_stays_square_however_far_it_grows() {
        let radii = Corners {
            top_left: Vec2::new(DevicePx(0.0), DevicePx(0.0)),
            top_right: Vec2::new(DevicePx(4.0), DevicePx(8.0)),
            bottom_right: Vec2::new(DevicePx(0.0), DevicePx(0.0)),
            bottom_left: Vec2::new(DevicePx(0.0), DevicePx(0.0)),
        };
        let grown = grow(radii, 6.0);
        assert_eq!(grown.top_left, Vec2::new(DevicePx(0.0), DevicePx(0.0)));
        assert_eq!(grown.top_right, Vec2::new(DevicePx(10.0), DevicePx(14.0)));
    }

    #[test]
    fn a_negative_spread_never_makes_a_radius_negative() {
        let radii = Corners::uniform(Vec2::new(DevicePx(2.0), DevicePx(2.0)));
        let shrunk = grow(radii, -10.0);
        assert_eq!(shrunk.top_left, Vec2::new(DevicePx(0.0), DevicePx(0.0)));
    }

    #[test]
    fn clipped_background_clips_oversized_shape_to_border_box() {
        let mut style = crate::lower::lower(&zgui_css::StyleDraft::initial().build(), 1.0);
        style.background.color = zgui_color::Color::srgb(0.15, 0.39, 0.92, 1.0);
        let box_rect = Rect::new(
            Point::new(DevicePx(20.0), DevicePx(40.0)),
            Size::new(DevicePx(265.0), DevicePx(120.0)),
        );
        let place = BoxPlacement {
            border_box: box_rect,
            radii: Corners::uniform(Vec2::new(DevicePx(0.0), DevicePx(0.0))),
            ..placement()
        };
        // Oversized shape extending 30px above and below border_box.
        let mut raw = zgui_scene::kurbo::BezPath::new();
        raw.move_to((0.0, -30.0));
        raw.line_to((265.0, -30.0));
        raw.line_to((265.0, 150.0));
        raw.line_to((0.0, 150.0));
        raw.close_path();
        let shape = crate::lower::ClipPathShape::from_path(raw);
        let mut scene = zgui_scene::Scene::new();
        scene.begin_frame(Size::<i32, Device>::new(600, 400));
        assert_eq!(
            super::clipped_background(
                &mut scene,
                zgui_scene::VectorId(7),
                &style,
                place,
                &shape,
                box_rect,
            ),
            1
        );
        let item = &scene.primitives.vectors[0];
        assert_ne!(item.clip, zgui_scene::ClipId::ROOT, "must push a border_box scene clip");
        assert_eq!(item.clips.len(), 1, "must clip vector path to local border_box bounds");
        let clip_bounds = zgui_scene::kurbo::Shape::bounding_box(&*item.clips[0].path);
        assert_eq!(clip_bounds, zgui_scene::kurbo::Rect::new(0.0, 0.0, 265.0, 120.0));
    }

    #[test]
    fn mixed_per_side_borders_and_double_emit_independent_quads() {
        use zgui_scene::prim::quad::BorderStyle;

        let mut style = crate::lower::lower(&zgui_css::StyleDraft::initial().build(), 1.0);
        style.background.color = zgui_color::Color::srgb(0.9, 0.9, 0.9, 1.0);
        style.border.invisible = false;
        style.border.colors = [
            zgui_color::Color::srgb(0.2, 0.5, 1.0, 1.0),
            zgui_color::Color::srgb(0.1, 0.7, 0.5, 1.0),
            zgui_color::Color::srgb(1.0, 0.6, 0.0, 1.0),
            zgui_color::Color::srgb(0.9, 0.3, 0.6, 1.0),
        ];
        style.border.styles = [
            super::LineStyle::Dashed,
            super::LineStyle::Dotted,
            super::LineStyle::Double,
            super::LineStyle::Solid,
        ];
        let place = BoxPlacement {
            border_box: Rect::new(
                Point::new(DevicePx(10.0), DevicePx(10.0)),
                Size::new(DevicePx(200.0), DevicePx(60.0)),
            ),
            border: zgui_geom::Edges {
                top: DevicePx(3.0),
                right: DevicePx(3.0),
                bottom: DevicePx(5.0),
                left: DevicePx(4.0),
            },
            radii: Corners::uniform(Vec2::new(DevicePx(0.0), DevicePx(0.0))),
            ..placement()
        };
        let mut scene = zgui_scene::Scene::new();
        scene.begin_frame(Size::<i32, Device>::new(400, 200));
        let pushed = super::background_and_border(
            &mut scene,
            &style,
            place,
            &crate::content::shader::NoShaders,
            None,
        );
        // 1 fill + 1 solid (left) + 1 dashed (top) + 1 dotted (right) + 2 double bands (bottom) = 6 quads.
        assert_eq!(pushed, 6);
        let quads = &scene.primitives.quads;
        assert_eq!(quads.len(), 6);
        assert!(quads.iter().any(|q| q.style == BorderStyle::Dashed as u32 && q.border == [3.0, 0.0, 0.0, 0.0]));
        assert!(quads.iter().any(|q| q.style == BorderStyle::Dotted as u32 && q.border == [0.0, 3.0, 0.0, 0.0]));
        assert!(quads.iter().any(|q| q.style == BorderStyle::Solid as u32 && q.border == [0.0, 0.0, 0.0, 4.0]));
        // Bottom 5px double splits into two 1px solid bands separated by a 3px gap (outer at bottom edge, inner inset 4px from bottom).
        let double_bands: Vec<_> = quads
            .iter()
            .filter(|q| q.style == BorderStyle::Solid as u32 && q.border == [0.0, 0.0, 1.0, 0.0])
            .collect();
        assert_eq!(double_bands.len(), 2);
        assert_eq!(double_bands[0].bounds, [10.0, 10.0, 200.0, 60.0]);
        assert_eq!(double_bands[1].bounds, [14.0, 13.0, 193.0, 53.0]);
    }
}
