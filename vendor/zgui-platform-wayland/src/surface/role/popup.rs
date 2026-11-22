//! A pop-up: where the compositor is asked to put it, and what it may do to fit it on screen.

use wayland_client::Proxy;
use wayland_protocols::xdg::shell::client::xdg_positioner::{
    Anchor, ConstraintAdjustment, Gravity, XdgPositioner,
};
use zgui_geom::{Css, CssPx, Point, Rect};
use zgui_platform::{Constrain, PlatformError, PopupPlacement};

/// Which point of the anchor rectangle the pop-up hangs from.
pub const fn anchor(anchor: zgui_platform::Anchor) -> Anchor {
    match anchor {
        zgui_platform::Anchor::Top => Anchor::Top,
        zgui_platform::Anchor::Bottom => Anchor::Bottom,
        zgui_platform::Anchor::Left => Anchor::Left,
        zgui_platform::Anchor::Right => Anchor::Right,
        zgui_platform::Anchor::TopLeft => Anchor::TopLeft,
        zgui_platform::Anchor::TopRight => Anchor::TopRight,
        zgui_platform::Anchor::BottomLeft => Anchor::BottomLeft,
        zgui_platform::Anchor::BottomRight => Anchor::BottomRight,
        _ => Anchor::None,
    }
}

/// Which way the pop-up extends from the anchor point.
pub const fn gravity(anchor: zgui_platform::Anchor) -> Gravity {
    match anchor {
        zgui_platform::Anchor::Top => Gravity::Top,
        zgui_platform::Anchor::Bottom => Gravity::Bottom,
        zgui_platform::Anchor::Left => Gravity::Left,
        zgui_platform::Anchor::Right => Gravity::Right,
        zgui_platform::Anchor::TopLeft => Gravity::TopLeft,
        zgui_platform::Anchor::TopRight => Gravity::TopRight,
        zgui_platform::Anchor::BottomLeft => Gravity::BottomLeft,
        zgui_platform::Anchor::BottomRight => Gravity::BottomRight,
        _ => Gravity::None,
    }
}

/// What the compositor may do to a pop-up that would not fit.
pub fn constraint(constrain: Constrain) -> ConstraintAdjustment {
    let mut adjustment = ConstraintAdjustment::None;
    if constrain.slide {
        adjustment |= ConstraintAdjustment::SlideX | ConstraintAdjustment::SlideY;
    }
    if constrain.flip {
        adjustment |= ConstraintAdjustment::FlipX | ConstraintAdjustment::FlipY;
    }
    if constrain.resize {
        adjustment |= ConstraintAdjustment::ResizeX | ConstraintAdjustment::ResizeY;
    }
    adjustment
}

/// The anchor rectangle, translated into the parent's window-geometry coordinates and clamped into
/// the parent's visual geometry and to a non-empty extent.
///
/// Both are protocol errors rather than mistakes the compositor forgives: a rectangle of zero
/// extent, and one that leaves the parent's window geometry, each disconnect the client. The
/// rectangle a menu is anchored to is computed from a layout in the parent's surface-local
/// coordinates that knows nothing about either, so it is translated by `-parent.origin` and pulled
/// inward at the edges rather than refused, because a menu placed a pixel from where it was asked
/// for is a better answer than no menu.
pub fn anchor_rect(
    rect: Rect<CssPx, Css>,
    parent: Rect<CssPx, Css>,
) -> (i32, i32, i32, i32) {
    let origin_x = parent.origin.x.0.round() as i32;
    let origin_y = parent.origin.y.0.round() as i32;
    let limit_x = parent.size.width.0.max(1.0).round() as i32;
    let limit_y = parent.size.height.0.max(1.0).round() as i32;
    let x = (rect.origin.x.0.round() as i32)
        .saturating_sub(origin_x)
        .clamp(0, limit_x - 1);
    let y = (rect.origin.y.0.round() as i32)
        .saturating_sub(origin_y)
        .clamp(0, limit_y - 1);
    let width = (rect.size.width.0.round() as i32).clamp(1, limit_x - x);
    let height = (rect.size.height.0.round() as i32).clamp(1, limit_y - y);
    (x, y, width, height)
}

/// A popup translation quantized to signed Wayland logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Offset {
    x: i32,
    y: i32,
}

impl Offset {
    /// Rounds each axis to the nearest logical pixel, with ties away from zero.
    ///
    /// # Errors
    ///
    /// Returns [`PlatformError::SurfaceCreation`] if either axis is nonfinite or outside the
    /// signed 32-bit range.
    pub fn new(offset: Point<CssPx, Css>) -> Result<Self, PlatformError> {
        fn axis(value: CssPx) -> Option<i32> {
            // Widen before comparing: i32::MAX rounds up to 2^31 as an f32.
            let value = f64::from(value.0);
            if (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&value) {
                // Rounding stays within the checked bounds, so the cast is exact.
                Some(value.round() as i32)
            } else {
                None
            }
        }

        match (axis(offset.x), axis(offset.y)) {
            (Some(x), Some(y)) => Ok(Self { x, y }),
            _ => Err(PlatformError::SurfaceCreation(
                "pop-up offset must be finite and fit the signed 32-bit range".to_owned(),
            )),
        }
    }
}

/// Fills `positioner` in from `placement`, measured against a parent whose window geometry is
/// `parent` in logical pixels.
pub fn describe(
    positioner: &XdgPositioner,
    placement: &PopupPlacement,
    offset: Offset,
    parent: Rect<CssPx, Css>,
    size: zgui_geom::Size<CssPx, Css>,
) {
    let (x, y, width, height) = anchor_rect(placement.anchor_rect, parent);
    positioner.set_anchor_rect(x, y, width, height);
    positioner.set_anchor(anchor(placement.anchor));
    positioner.set_gravity(gravity(placement.gravity));
    positioner.set_constraint_adjustment(constraint(placement.constrain));
    // set_offset is part of the original positioner protocol; reactive placement needs v3.
    positioner.set_offset(offset.x, offset.y);
    if placement.reactive && positioner.version() >= 3 {
        positioner.set_reactive();
    }
    positioner.set_size(
        (size.width.0.round() as i32).max(1),
        (size.height.0.round() as i32).max(1),
    );
}

#[cfg(test)]
mod tests {
    use super::{Offset, anchor, anchor_rect, constraint, gravity};
    use wayland_protocols::xdg::shell::client::xdg_positioner::{
        Anchor, ConstraintAdjustment, Gravity,
    };
    use zgui_geom::{CssPx, Point, Rect, Size};
    use zgui_platform::Constrain;

    fn parent() -> Rect<CssPx, zgui_geom::Css> {
        Rect::new(
            Point::new(CssPx(0.0), CssPx(0.0)),
            Size::new(CssPx(400.0), CssPx(300.0)),
        )
    }

    #[test]
    fn popup_offset_uses_the_signed_integer_range() {
        let offset = |x, y| Offset::new(Point::new(CssPx(x), CssPx(y)));
        assert_eq!(
            offset(-2_147_483_648.0, 2_147_483_520.0),
            Ok(Offset { x: i32::MIN, y: 2_147_483_520 }),
            "the extreme representable in-range f32 values remain exact"
        );
        for invalid in [
            2_147_483_648.0,
            -2_147_483_904.0,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ] {
            assert!(offset(invalid, 0.0).is_err(), "invalid x offset: {invalid}");
            assert!(offset(0.0, invalid).is_err(), "invalid y offset: {invalid}");
        }
    }

    #[test]
    fn a_drop_down_hangs_from_the_bottom_left_and_extends_down_and_right() {
        assert_eq!(
            anchor(zgui_platform::Anchor::BottomLeft),
            Anchor::BottomLeft
        );
        assert_eq!(
            gravity(zgui_platform::Anchor::BottomRight),
            Gravity::BottomRight
        );
        assert_eq!(anchor(zgui_platform::Anchor::Center), Anchor::None);
    }

    #[test]
    fn each_freedom_the_compositor_is_given_covers_both_axes() {
        assert_eq!(constraint(Constrain::NONE), ConstraintAdjustment::None);
        assert_eq!(
            constraint(Constrain::SLIDE),
            ConstraintAdjustment::SlideX | ConstraintAdjustment::SlideY
        );
        assert!(constraint(Constrain::ANY).contains(ConstraintAdjustment::FlipY));
        assert!(constraint(Constrain::ANY).contains(ConstraintAdjustment::ResizeX));
    }

    #[test]
    fn a_rectangle_inside_the_parent_crosses_unchanged() {
        let rect = Rect::new(
            Point::new(CssPx(10.0), CssPx(20.0)),
            Size::new(CssPx(100.0), CssPx(24.0)),
        );
        assert_eq!(anchor_rect(rect, parent()), (10, 20, 100, 24));
    }

    #[test]
    fn a_rectangle_of_no_extent_is_grown_rather_than_sent_as_a_protocol_error() {
        // A zero-sized anchor rectangle disconnects the client. A caret is exactly that.
        let caret = Rect::new(
            Point::new(CssPx(40.0), CssPx(40.0)),
            Size::new(CssPx(0.0), CssPx(0.0)),
        );
        assert_eq!(anchor_rect(caret, parent()), (40, 40, 1, 1));
    }

    #[test]
    fn a_rectangle_hanging_off_the_parent_is_pulled_back_inside_it() {
        let hanging = Rect::new(
            Point::new(CssPx(380.0), CssPx(290.0)),
            Size::new(CssPx(200.0), CssPx(200.0)),
        );
        let (x, y, width, height) = anchor_rect(hanging, parent());
        assert_eq!((x, y), (380, 290));
        assert_eq!((width, height), (20, 10));
        assert!(x + width <= 400 && y + height <= 300);
    }

    #[test]
    fn a_rectangle_starting_before_the_parent_is_moved_to_its_origin() {
        let before = Rect::new(
            Point::new(CssPx(-30.0), CssPx(-30.0)),
            Size::new(CssPx(10.0), CssPx(10.0)),
        );
        assert_eq!(anchor_rect(before, parent()), (0, 0, 10, 10));
    }

    #[test]
    fn an_anchor_on_an_inset_parent_subtracts_parent_window_geometry_origin() {
        let inset_parent = Rect::new(
            Point::new(CssPx(28.0), CssPx(16.0)),
            Size::new(CssPx(360.0), CssPx(200.0)),
        );
        let trigger = Rect::new(
            Point::new(CssPx(40.0), CssPx(46.0)),
            Size::new(CssPx(100.0), CssPx(24.0)),
        );
        assert_eq!(anchor_rect(trigger, inset_parent), (12, 30, 100, 24));

        // A trigger near the right/bottom visual edge clamps to the parent's visual extent.
        let near_edge = Rect::new(
            Point::new(CssPx(368.0), CssPx(206.0)),
            Size::new(CssPx(80.0), CssPx(40.0)),
        );
        assert_eq!(anchor_rect(near_edge, inset_parent), (340, 190, 20, 10));
    }
}
