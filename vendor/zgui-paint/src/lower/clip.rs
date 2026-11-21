//! `clip-path`: what a box cuts its own painting to.
//!
//! Overflow clipping is not here. That one is layout's, because the chain a descendant is drawn
//! under has to exist before a fragment does — a fragment carries its chain, and this stage reads
//! it rather than building a second one.
//!
//! What is here is the other clip, the one that cuts a box's *own* pixels (and clipped replaced
//! images) to a CSS `<basic-shape>` such as `path(...)`, `circle(...)`, `ellipse(...)`,
//! `polygon(...)`, or `inset(...)`.

use std::sync::Arc;

use style::values::computed::basic_shape::{
    BasicShape, FillRule, InsetRect, PathOrShapeFunction, ShapeRadius,
};
use style::values::computed::{Length, LengthPercentage, Position};
use style::values::generics::position::GenericPositionOrAuto;
use style_traits::{CssWriter, ToCss};
use zgui_css::parity::Support;
use zgui_css::values::effect::ClipPathValue;
use zgui_css::{ComputedStyle, register_properties};
use zgui_geom::{Device, DevicePx, Rect};
use zgui_scene::ContentHash;
use zgui_scene::kurbo::{self, Affine, BezPath, Circle, Ellipse, PathEl, Point, RoundedRect, Shape};
use zgui_scene::peniko;

register_properties! {
    clip_path => Support::Implemented("zgui-paint::lower::clip"),
    clip_rule => Support::Absent(zgui_css::parity::AbsentReason::GeckoOnly),
}

/// How far a flattened clip outline may stray from a curve, in device pixels.
const FLATTEN_TOLERANCE: f64 = 0.05;

/// A lowered CSS `clip-path` shape with a deterministic content hash for caching.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipPathShape {
    hash: u64,
    spec: ClipPathSpec,
}

#[derive(Clone, Debug, PartialEq)]
enum ClipPathSpec {
    /// `path("...")` in CSS pixels relative to the reference box origin.
    Path {
        fill_rule: peniko::Fill,
        path: Arc<BezPath>,
    },
    /// `circle(radius at x y)` resolved against the reference box.
    Circle {
        position: GenericPositionOrAuto<Position>,
        radius: ShapeRadius,
    },
    /// `ellipse(rx ry at x y)` resolved against the reference box.
    Ellipse {
        position: GenericPositionOrAuto<Position>,
        rx: ShapeRadius,
        ry: ShapeRadius,
    },
    /// `polygon(fill-rule, x y, ...)` resolved against the reference box.
    Polygon {
        fill_rule: peniko::Fill,
        points: Vec<(LengthPercentage, LengthPercentage)>,
    },
    /// `inset(top right bottom left round radii)` resolved against the reference box.
    Inset(Box<InsetRect>),
}

impl ClipPathShape {
    #[cfg(test)]
    pub(crate) fn from_path(path: BezPath) -> Self {
        Self {
            hash: 1,
            spec: ClipPathSpec::Path {
                fill_rule: peniko::Fill::NonZero,
                path: Arc::new(path),
            },
        }
    }

    /// Deterministic hash identifying this clip-path geometry.
    #[must_use]
    pub fn hash(&self) -> u64 {
        self.hash
    }

    /// Resolves this shape into a device-pixel [`BezPath`] relative to `reference_box.origin`.
    #[must_use]
    pub fn to_device_path(
        &self,
        reference_box: Rect<DevicePx, Device>,
        scale: f32,
    ) -> Option<(BezPath, peniko::Fill)> {
        let s = f64::from(scale.max(f32::EPSILON));
        let ref_w = (f64::from(reference_box.size.width.0) / s).max(0.0);
        let ref_h = (f64::from(reference_box.size.height.0) / s).max(0.0);
        match &self.spec {
            ClipPathSpec::Path { fill_rule, path } => {
                let scaled = Affine::scale(s) * (**path).clone();
                Some((scaled, *fill_rule))
            }
            ClipPathSpec::Circle { position, radius } => {
                let (cx, cy) = resolve_position(position, ref_w, ref_h);
                let r = resolve_circle_radius(radius, cx, cy, ref_w, ref_h).max(0.0);
                let path = Circle::new(Point::new(cx * s, cy * s), r * s).to_path(FLATTEN_TOLERANCE);
                Some((path, peniko::Fill::NonZero))
            }
            ClipPathSpec::Ellipse { position, rx, ry } => {
                let (cx, cy) = resolve_position(position, ref_w, ref_h);
                let rx = resolve_axis_radius(rx, cx, ref_w).max(0.0);
                let ry = resolve_axis_radius(ry, cy, ref_h).max(0.0);
                let path = Ellipse::new(Point::new(cx * s, cy * s), (rx * s, ry * s), 0.0)
                    .to_path(FLATTEN_TOLERANCE);
                Some((path, peniko::Fill::NonZero))
            }
            ClipPathSpec::Polygon { fill_rule, points } => {
                if points.is_empty() {
                    return None;
                }
                let mut path = BezPath::new();
                for (index, (x, y)) in points.iter().enumerate() {
                    let px = resolve_lp(x, ref_w) * s;
                    let py = resolve_lp(y, ref_h) * s;
                    if index == 0 {
                        path.move_to(Point::new(px, py));
                    } else {
                        path.line_to(Point::new(px, py));
                    }
                }
                path.close_path();
                Some((path, *fill_rule))
            }
            ClipPathSpec::Inset(inset) => {
                let top = resolve_lp(&inset.rect.0, ref_h);
                let right = resolve_lp(&inset.rect.1, ref_w);
                let bottom = resolve_lp(&inset.rect.2, ref_h);
                let left = resolve_lp(&inset.rect.3, ref_w);
                let x0 = left * s;
                let y0 = top * s;
                let x1 = (ref_w - right).max(left) * s;
                let y1 = (ref_h - bottom).max(top) * s;
                let rect = kurbo::Rect::new(x0, y0, x1, y1);
                let tl = resolve_lp(&inset.round.top_left.0.width.0, ref_w).max(0.0) * s;
                let tr = resolve_lp(&inset.round.top_right.0.width.0, ref_w).max(0.0) * s;
                let br = resolve_lp(&inset.round.bottom_right.0.width.0, ref_w).max(0.0) * s;
                let bl = resolve_lp(&inset.round.bottom_left.0.width.0, ref_w).max(0.0) * s;
                let radii = kurbo::RoundedRectRadii::new(tl, tr, br, bl);
                let path = RoundedRect::from_rect(rect, radii).to_path(FLATTEN_TOLERANCE);
                Some((path, peniko::Fill::NonZero))
            }
        }
    }

    /// Rasterises this shape into an 8-bit anti-aliased coverage buffer of `width * height`
    /// device pixels covering `target_box`.
    #[must_use]
    pub fn rasterize_mask(
        &self,
        reference_box: Rect<DevicePx, Device>,
        target_box: Rect<DevicePx, Device>,
        scale: f32,
        width: u32,
        height: u32,
    ) -> Option<Vec<u8>> {
        if width == 0 || height == 0 {
            return None;
        }
        let (mut path, fill_rule) = self.to_device_path(reference_box, scale)?;
        let dx = f64::from(reference_box.origin.x.0 - target_box.origin.x.0);
        let dy = f64::from(reference_box.origin.y.0 - target_box.origin.y.0);
        if dx != 0.0 || dy != 0.0 {
            path = Affine::translate((dx, dy)) * path;
        }
        let mut commands = Vec::new();
        let to_point = |point: Point| zeno::Point::new(point.x as f32, point.y as f32);
        kurbo::flatten(path.iter(), FLATTEN_TOLERANCE, |element| {
            commands.push(match element {
                PathEl::MoveTo(p) => zeno::Command::MoveTo(to_point(p)),
                PathEl::LineTo(p) => zeno::Command::LineTo(to_point(p)),
                PathEl::ClosePath => zeno::Command::Close,
                PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => {
                    zeno::Command::LineTo(to_point(p))
                }
            });
        });
        let len = (width as usize).checked_mul(height as usize)?;
        let mut out = vec![0u8; len];
        let mut mask = zeno::Mask::new(&commands);
        mask.style(match fill_rule {
            peniko::Fill::NonZero => zeno::Fill::NonZero,
            peniko::Fill::EvenOdd => zeno::Fill::EvenOdd,
        });
        mask.size(width, height);
        mask.render_into(&mut out, None);
        Some(out)
    }
}

/// What a box's `clip-path` asks for.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum ClipShape {
    /// Nothing is clipped, which is what nearly every box says.
    #[default]
    None,
    /// A CSS `<basic-shape>` clip-path (`path(...)`, `circle(...)`, `ellipse(...)`,
    /// `polygon(...)`, or `inset(...)`).
    Shape(Arc<ClipPathShape>),
    /// A geometry-box or unsupported clip-path shape.
    Masked,
}

impl ClipShape {
    /// Whether this shape needs a target of its own (or prevents direct scroll-shift copies).
    #[must_use]
    pub fn needs_target(&self) -> bool {
        !matches!(self, Self::None)
    }

    /// The lowered basic shape, when one was parsed.
    #[must_use]
    pub fn shape(&self) -> Option<&Arc<ClipPathShape>> {
        match self {
            Self::Shape(shape) => Some(shape),
            Self::None | Self::Masked => None,
        }
    }

    /// Deterministic hash folded into the paint-style cache key.
    #[must_use]
    pub fn key_hash(&self) -> u64 {
        match self {
            Self::None => 0,
            Self::Masked => 1,
            Self::Shape(shape) => shape.hash.max(2),
        }
    }
}

/// Lowers a style's `clip-path`.
///
/// A `url()` reference names a shape defined by a document this engine does not resolve, and a
/// shape it cannot resolve clips nothing — which is what every engine does with a reference that
/// does not resolve.
pub fn of(style: &ComputedStyle) -> ClipShape {
    let clip = &style.get_svg().clip_path;
    match clip {
        ClipPathValue::None | ClipPathValue::Url(_) => ClipShape::None,
        ClipPathValue::Box(_) => ClipShape::Masked,
        ClipPathValue::Shape(basic_shape, _) => {
            let mut css_text = String::new();
            let _ = clip.to_css(&mut CssWriter::new(&mut css_text));
            let hash = ContentHash::new().bytes(css_text.as_bytes()).finish().max(2);
            match lower_basic_shape(basic_shape) {
                Some(spec) => ClipShape::Shape(Arc::new(ClipPathShape { hash, spec })),
                None => ClipShape::Masked,
            }
        }
    }
}

fn lower_basic_shape(shape: &BasicShape) -> Option<ClipPathSpec> {
    match shape {
        BasicShape::PathOrShape(PathOrShapeFunction::Path(path)) => {
            let mut svg = String::new();
            path.path.to_css(&mut CssWriter::new(&mut svg), false).ok()?;
            let bez = BezPath::from_svg(&svg).ok()?;
            Some(ClipPathSpec::Path {
                fill_rule: to_fill_rule(path.fill),
                path: Arc::new(bez),
            })
        }
        BasicShape::PathOrShape(PathOrShapeFunction::Shape(_)) => None,
        BasicShape::Circle(circle) => Some(ClipPathSpec::Circle {
            position: circle.position.clone(),
            radius: circle.radius.clone(),
        }),
        BasicShape::Ellipse(ellipse) => Some(ClipPathSpec::Ellipse {
            position: ellipse.position.clone(),
            rx: ellipse.semiaxis_x.clone(),
            ry: ellipse.semiaxis_y.clone(),
        }),
        BasicShape::Polygon(polygon) => Some(ClipPathSpec::Polygon {
            fill_rule: to_fill_rule(polygon.fill),
            points: polygon
                .coordinates
                .iter()
                .map(|coord| (coord.0.clone(), coord.1.clone()))
                .collect(),
        }),
        BasicShape::Rect(inset) => Some(ClipPathSpec::Inset(Box::new(inset.clone()))),
    }
}

fn to_fill_rule(rule: FillRule) -> peniko::Fill {
    match rule {
        FillRule::Nonzero => peniko::Fill::NonZero,
        FillRule::Evenodd => peniko::Fill::EvenOdd,
    }
}

fn resolve_lp(value: &LengthPercentage, basis: f64) -> f64 {
    f64::from(value.resolve(Length::new(basis as f32)).px())
}

fn resolve_position(pos: &GenericPositionOrAuto<Position>, width: f64, height: f64) -> (f64, f64) {
    match pos {
        GenericPositionOrAuto::Auto => (width * 0.5, height * 0.5),
        GenericPositionOrAuto::Position(p) => (
            resolve_lp(&p.horizontal, width),
            resolve_lp(&p.vertical, height),
        ),
    }
}

fn resolve_circle_radius(
    radius: &ShapeRadius,
    cx: f64,
    cy: f64,
    width: f64,
    height: f64,
) -> f64 {
    let dx_near = cx.abs().min((width - cx).abs());
    let dy_near = cy.abs().min((height - cy).abs());
    let dx_far = cx.abs().max((width - cx).abs());
    let dy_far = cy.abs().max((height - cy).abs());
    match radius {
        ShapeRadius::Length(lp) => {
            let ref_len = (width * width + height * height).sqrt() / core::f64::consts::SQRT_2;
            resolve_lp(&lp.0, ref_len)
        }
        ShapeRadius::ClosestSide => dx_near.min(dy_near),
        ShapeRadius::FarthestSide => dx_far.max(dy_far),
        ShapeRadius::ClosestCorner => dx_near.hypot(dy_near),
        ShapeRadius::FarthestCorner => dx_far.hypot(dy_far),
    }
}

fn resolve_axis_radius(radius: &ShapeRadius, center: f64, extent: f64) -> f64 {
    let near = center.abs().min((extent - center).abs());
    let far = center.abs().max((extent - center).abs());
    match radius {
        ShapeRadius::Length(lp) => resolve_lp(&lp.0, extent),
        ShapeRadius::ClosestSide | ShapeRadius::ClosestCorner => near,
        ShapeRadius::FarthestSide | ShapeRadius::FarthestCorner => far,
    }
}

#[cfg(test)]
mod tests {
    use zgui_css::StyleDraft;
    use zgui_geom::{DevicePx, Point, Rect, Size};

    use super::{ClipShape, of};

    #[test]
    fn a_box_with_no_clip_path_needs_no_target() {
        let style = StyleDraft::initial().build();
        assert_eq!(of(&style), ClipShape::None);
        assert!(!of(&style).needs_target());
    }

    #[test]
    fn only_a_masked_shape_needs_a_target() {
        assert!(ClipShape::Masked.needs_target());
        assert!(!ClipShape::None.needs_target());
    }

    #[test]
    fn path_clip_rasterizes_interior_and_excludes_corners() {
        let bez = zgui_scene::kurbo::BezPath::from_svg("M 20 0 L 40 20 L 20 40 L 0 20 Z")
            .expect("valid diamond SVG path");
        let shape = super::ClipPathShape {
            hash: 42,
            spec: super::ClipPathSpec::Path {
                fill_rule: zgui_scene::peniko::Fill::NonZero,
                path: std::sync::Arc::new(bez),
            },
        };
        let rect = Rect::new(
            Point::new(DevicePx(0.0), DevicePx(0.0)),
            Size::new(DevicePx(40.0), DevicePx(40.0)),
        );
        let mask = shape
            .rasterize_mask(rect, rect, 1.0, 40, 40)
            .expect("rasterize diamond mask");
        assert_eq!(mask.len(), 1600);
        // Center (20, 20) is fully inside the diamond.
        assert_eq!(mask[20 * 40 + 20], 255);
        // Corner (0, 0) is outside the diamond.
        assert_eq!(mask[0], 0);
        assert_eq!(mask[39], 0);
        assert_eq!(mask[39 * 40 + 39], 0);
    }
}
