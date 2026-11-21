//! The four border sides and the four corners.
//!
//! Widths are already resolved and snapped onto the fragment, because layout had to resolve them to
//! know where the content box was; what is lowered here is what layout has no reason to look at —
//! the colours and the line styles.
//!
//! Corner radii are *not* lowered, and that is deliberate: a percentage radius is a percentage of
//! the box's own extent, so two boxes with one style have different radii and a cache shared
//! between them cannot hold either. [`radii_of`] resolves them against a box, once, through the
//! same function that decides what a box clips its children to — so a background painted to a curve
//! and a child clipped to it cannot round differently.

use zgui_color::Color;
use zgui_css::parity::Support;
use zgui_css::values::border::BorderStyleValue;
use zgui_css::values::color::{ColorValue, current, resolve};
use zgui_css::{ComputedStyle, register_properties};
use zgui_geom::{Corners, Device, DevicePx, Rect, Vec2};
use zgui_scene::prim::quad::BorderStyle;

register_properties! {
    border_top_color           => Support::Implemented("zgui-paint::lower::border"),
    border_right_color         => Support::Implemented("zgui-paint::lower::border"),
    border_block_start_color   => Support::Implemented("zgui-paint::lower::border"),
    border_block_end_color     => Support::Implemented("zgui-paint::lower::border"),
    border_inline_start_color  => Support::Implemented("zgui-paint::lower::border"),
    border_inline_end_color    => Support::Implemented("zgui-paint::lower::border"),
    border_bottom_color        => Support::Implemented("zgui-paint::lower::border"),
    border_left_color          => Support::Implemented("zgui-paint::lower::border"),
    border_top_style           => Support::Implemented("zgui-paint::lower::border"),
    border_right_style         => Support::Implemented("zgui-paint::lower::border"),
    border_bottom_style        => Support::Implemented("zgui-paint::lower::border"),
    border_left_style          => Support::Implemented("zgui-paint::lower::border"),
    border_top_left_radius     => Support::Implemented("zgui-paint::lower::border"),
    border_top_right_radius    => Support::Implemented("zgui-paint::lower::border"),
    border_bottom_right_radius => Support::Implemented("zgui-paint::lower::border"),
    border_bottom_left_radius  => Support::Implemented("zgui-paint::lower::border"),
    border_image_source        => Support::Ignored("border images are not painted"),
    border_image_slice         => Support::Ignored("border images are not painted"),
    border_image_width         => Support::Ignored("border images are not painted"),
    border_image_outset        => Support::Ignored("border images are not painted"),
    border_image_repeat        => Support::Ignored("border images are not painted"),
}

/// How the four sides of a box's border are drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderPaint {
    /// The colours of the top, right, bottom and left sides, in that order.
    pub colors: [Color; 4],
    /// The line styles of the top, right, bottom and left sides, in that order.
    pub styles: [LineStyle; 4],
    /// Whether every side that has a width is invisible — `none`, `hidden`, or fully transparent.
    pub invisible: bool,
}

impl Default for BorderPaint {
    /// A border that draws nothing: four fully transparent sides.
    fn default() -> Self {
        Self {
            colors: [Color::TRANSPARENT; 4],
            styles: [LineStyle::Solid; 4],
            invisible: true,
        }
    }
}

impl BorderPaint {
    /// Returns the single scene border style when all visible sides can share one quad pass.
    pub fn single_quad_style(&self, widths: [f32; 4]) -> Option<BorderStyle> {
        let mut active: Option<LineStyle> = None;
        for side in 0..4 {
            if widths[side] <= 0.0 || self.colors[side].alpha() == 0.0 {
                continue;
            }
            let line = self.styles[side];
            if line == LineStyle::Double {
                return None;
            }
            match active {
                None => active = Some(line),
                Some(prev) if prev == line => {}
                Some(_) => return None,
            }
        }
        Some(active.unwrap_or(LineStyle::Solid).to_scene())
    }
}

/// How a border line is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineStyle {
    /// One continuous line.
    #[default]
    Solid,
    /// Dashes with gaps, continuing round the corners.
    Dashed,
    /// Round dots with gaps.
    Dotted,
    /// Two parallel solid lines separated by a transparent gap.
    Double,
}

impl LineStyle {
    /// Lowers a CSS border style keyword to a painted line style.
    pub fn from_css(value: BorderStyleValue) -> Self {
        match value {
            BorderStyleValue::Dashed => Self::Dashed,
            BorderStyleValue::Dotted => Self::Dotted,
            BorderStyleValue::Double => Self::Double,
            _ => Self::Solid,
        }
    }

    /// The scene's own spelling of this style for single-stroke quads.
    pub fn to_scene(self) -> BorderStyle {
        match self {
            Self::Solid | Self::Double => BorderStyle::Solid,
            Self::Dashed => BorderStyle::Dashed,
            Self::Dotted => BorderStyle::Dotted,
        }
    }
}

/// Lowers a style's border paint.
pub fn of(style: &ComputedStyle) -> BorderPaint {
    let border = style.get_border();
    let current = current(style);
    let sides = [
        (border.border_top_style, &border.border_top_color),
        (border.border_right_style, &border.border_right_color),
        (border.border_bottom_style, &border.border_bottom_color),
        (border.border_left_style, &border.border_left_color),
    ];
    let colors = sides.map(|(line, color)| side_color(line, color, current));
    let styles = sides.map(|(line, _)| LineStyle::from_css(line));
    BorderPaint {
        colors,
        styles,
        invisible: colors.iter().all(|color| color.alpha() == 0.0),
    }
}

/// One side's colour, which is fully transparent when the side draws no line at all.
///
/// The width of a `none` or `hidden` side is already zero on the fragment, so this is belt and
/// braces — but a transparent colour is what makes the *quad* stop drawing, and the quad carries
/// one stroke for four sides.
fn side_color(
    line: BorderStyleValue,
    color: &ColorValue,
    current: &zgui_css::values::color::AbsoluteColor,
) -> Color {
    match line {
        BorderStyleValue::None | BorderStyleValue::Hidden => Color::TRANSPARENT,
        _ => resolve(color, current),
    }
}

/// The four elliptical corner radii of a box's border box, in device pixels.
///
/// This is the layout stage's own resolution rather than a second one: a percentage radius is a
/// percentage of the box's extent on that axis, the two radii of a corner may differ, and radii
/// that would overlap are shrunk together rather than clamped one at a time. A second
/// implementation here would let a background curve and a clip curve disagree.
pub fn radii_of(
    style: &ComputedStyle,
    border_box: Rect<DevicePx, Device>,
    scale: f32,
) -> Corners<Vec2<DevicePx>> {
    zgui_layout::fragment::clip::radii(style, border_box, scale)
}

/// The four corner radii of the *inner* edge of a border: the outer radii less the border widths.
///
/// A curve inside a border is concentric with it rather than parallel, so the width is subtracted
/// from the radius. A radius smaller than the border it sits inside collapses to a square corner.
pub fn inner_radii(
    radii: Corners<Vec2<DevicePx>>,
    border: zgui_geom::Edges<DevicePx>,
) -> Corners<Vec2<DevicePx>> {
    let inset = |radius: Vec2<DevicePx>, horizontal: DevicePx, vertical: DevicePx| {
        Vec2::new(
            DevicePx((radius.x.0 - horizontal.0).max(0.0)),
            DevicePx((radius.y.0 - vertical.0).max(0.0)),
        )
    };
    Corners {
        top_left: inset(radii.top_left, border.left, border.top),
        top_right: inset(radii.top_right, border.right, border.top),
        bottom_right: inset(radii.bottom_right, border.right, border.bottom),
        bottom_left: inset(radii.bottom_left, border.left, border.bottom),
    }
}

#[cfg(test)]
mod tests {
    use zgui_color::Color;
    use zgui_css::values::border::BorderStyleValue;
    use zgui_geom::{Corners, DevicePx, Edges, Vec2};
    use zgui_scene::prim::quad::BorderStyle;

    use super::{BorderPaint, LineStyle, inner_radii};

    #[test]
    fn mixed_per_side_styles_and_double_disable_single_quad_collapse() {
        let mixed = BorderPaint {
            colors: [Color::BLACK; 4],
            styles: [
                LineStyle::Dashed,
                LineStyle::Dotted,
                LineStyle::Double,
                LineStyle::Solid,
            ],
            invisible: false,
        };
        assert_eq!(mixed.single_quad_style([3.0, 3.0, 5.0, 4.0]), None);
        assert_eq!(mixed.single_quad_style([3.0, 0.0, 0.0, 0.0]), Some(BorderStyle::Dashed));
        assert_eq!(mixed.single_quad_style([0.0, 3.0, 0.0, 0.0]), Some(BorderStyle::Dotted));
        assert_eq!(mixed.single_quad_style([0.0, 0.0, 5.0, 0.0]), None);
        assert_eq!(mixed.single_quad_style([0.0, 0.0, 0.0, 4.0]), Some(BorderStyle::Solid));
    }

    #[test]
    fn from_css_preserves_dashed_dotted_double_and_solid() {
        assert_eq!(LineStyle::from_css(BorderStyleValue::Dashed), LineStyle::Dashed);
        assert_eq!(LineStyle::from_css(BorderStyleValue::Dotted), LineStyle::Dotted);
        assert_eq!(LineStyle::from_css(BorderStyleValue::Double), LineStyle::Double);
        for style in [
            BorderStyleValue::Solid,
            BorderStyleValue::Groove,
            BorderStyleValue::Ridge,
            BorderStyleValue::Inset,
            BorderStyleValue::Outset,
            BorderStyleValue::None,
        ] {
            assert_eq!(LineStyle::from_css(style), LineStyle::Solid, "{style:?}");
        }
    }

    #[test]
    fn an_inner_radius_is_the_outer_one_less_the_border_and_never_negative() {
        let radii = Corners::uniform(Vec2::new(DevicePx(6.0), DevicePx(10.0)));
        let border = Edges {
            top: DevicePx(4.0),
            right: DevicePx(4.0),
            bottom: DevicePx(4.0),
            left: DevicePx(20.0),
        };
        let inner = inner_radii(radii, border);
        assert_eq!(inner.top_right, Vec2::new(DevicePx(2.0), DevicePx(6.0)));
        assert_eq!(
            inner.top_left,
            Vec2::new(DevicePx(0.0), DevicePx(6.0)),
            "a radius narrower than its border collapses rather than going negative"
        );
    }
}
