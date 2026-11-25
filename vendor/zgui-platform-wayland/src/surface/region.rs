//! The parts of a surface that take input or are opaque, in the compositor's whole pixels.
//!
//! A region is a set of rectangles in surface-local logical pixels, which is the space a CSS pixel
//! is drawn in here. The compositor measures them in whole pixels, so a rectangle that a
//! fractional scale left between pixels is rounded: outwards for input, so no pixel that shows a
//! control lets a press through, and inwards for opacity, so no pixel that is partly transparent
//! hides what is behind it.

use zgui_geom::{Css, CssPx, Rect, Size};
use zgui_platform::{BlurRegion, BlurShape};

/// A rectangle in whole logical pixels: left, top, width, height.
pub type Whole = (i32, i32, i32, i32);

/// What a surface was asked to change about one of its regions, and has not sent yet.
///
/// The outer absence is "nothing to send". The inner one is the region the protocol spells as no
/// region at all: the whole surface for input, nothing for opacity.
pub type Owed = Option<Option<Vec<Rect<CssPx, Css>>>>;

/// Owned normalized background blur intent retained by a Wayland surface across capability changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum OwnedBlurRegion {
    /// No background blur is requested.
    #[default]
    None,
    /// Full-surface background blur (`wl_region.add(0, 0, i32::MAX, i32::MAX)`).
    Full,
    /// Validated non-empty union of CSS rectangles.
    Rects(Vec<Rect<CssPx, Css>>),
    /// Validated wire-canonical rounded masks.
    Shapes(Vec<BlurShape>),
}

impl OwnedBlurRegion {
    /// Whether no background blur is desired.
    #[must_use]
    pub const fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

/// What the most recent buffer or bare commit actually submitted.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum CommittedBlurSnapshot {
    #[default]
    Unbound,
    Ext(OwnedBlurRegion),
    NiriCleared,
    NiriActive {
        region: OwnedBlurRegion,
        full_extent: Option<Size<CssPx, Css>>,
    },
}

/// State displaced while handing a frame to the presenter but before it confirms a commit.
#[derive(Debug)]
pub(crate) struct BlurBeforePresent {
    pub(crate) desired: Option<OwnedBlurRegion>,
    pub(crate) niri_ever_bound: bool,
}

/// Normalizes a [`BlurRegion`] into owned validated intent, dropping non-finite, non-positive,
/// or out-of-protocol-range rectangles and collapsing an empty list to [`OwnedBlurRegion::None`].
#[must_use]
pub fn normalize_blur(region: BlurRegion<'_>) -> OwnedBlurRegion {
    match region {
        BlurRegion::None => OwnedBlurRegion::None,
        BlurRegion::Full => OwnedBlurRegion::Full,
        BlurRegion::Rects(rects) => {
            let valid: Vec<_> = rects
                .iter()
                .copied()
                .filter(|&rect| valid_blur_rect(rect))
                .collect();
            if valid.is_empty() {
                OwnedBlurRegion::None
            } else {
                OwnedBlurRegion::Rects(valid)
            }
        }
        BlurRegion::Shapes(shapes) => {
            let valid: Vec<_> = shapes.iter().copied().filter_map(BlurShape::wire_canonical)
                .take(BlurShape::MAX_COUNT).collect();
            if valid.is_empty() { OwnedBlurRegion::None } else { OwnedBlurRegion::Shapes(valid) }
        }
        _ => OwnedBlurRegion::None,
    }
}

fn valid_blur_rect(rect: Rect<CssPx, Css>) -> bool {
    let x = f64::from(rect.origin.x.0);
    let y = f64::from(rect.origin.y.0);
    let w = f64::from(rect.size.width.0);
    let h = f64::from(rect.size.height.0);
    if !x.is_finite() || !y.is_finite() || !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
        return false;
    }
    let right_sum = x + w;
    let bottom_sum = y + h;
    if !right_sum.is_finite() || !bottom_sum.is_finite() {
        return false;
    }
    let left = x.floor();
    let top = y.floor();
    let right = right_sum.ceil();
    let bottom = bottom_sum.ceil();
    let range = f64::from(i32::MIN)..=f64::from(i32::MAX);
    if !range.contains(&left)
        || !range.contains(&top)
        || !range.contains(&right)
        || !range.contains(&bottom)
    {
        return false;
    }
    let Some(width) = (right as i32).checked_sub(left as i32) else {
        return false;
    };
    let Some(height) = (bottom as i32).checked_sub(top as i32) else {
        return false;
    };
    width > 0
        && height > 0
        && outward(rect).is_some_and(|(_, _, rw, rh)| rw > 0 && rh > 0)
}

/// `rect` grown to the whole pixels it touches.
pub fn outward(rect: Rect<CssPx, Css>) -> Option<Whole> {
    whole(
        rect.left().0.floor(),
        rect.top().0.floor(),
        rect.right().0.ceil(),
        rect.bottom().0.ceil(),
    )
}

/// `rect` shrunk to the whole pixels it covers completely.
pub fn inward(rect: Rect<CssPx, Css>) -> Option<Whole> {
    whole(
        rect.left().0.ceil(),
        rect.top().0.ceil(),
        rect.right().0.floor(),
        rect.bottom().0.floor(),
    )
}

/// The rectangle between two corners, when it covers any pixel at all.
fn whole(left: f32, top: f32, right: f32, bottom: f32) -> Option<Whole> {
    let finite = [left, top, right, bottom]
        .iter()
        .all(|edge| edge.is_finite());
    if !finite || right <= left || bottom <= top {
        return None;
    }
    let clamp = |value: f32| value.clamp(i32::MIN as f32, i32::MAX as f32) as i32;
    let (left, top) = (clamp(left), clamp(top));
    Some((
        left,
        top,
        clamp(right).saturating_sub(left),
        clamp(bottom).saturating_sub(top),
    ))
}

#[cfg(test)]
mod tests {
    use super::{inward, outward};
    use zgui_geom::{CssPx, Point, Rect, Size};

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect<CssPx, zgui_geom::Css> {
        Rect::new(
            Point::new(CssPx(x), CssPx(y)),
            Size::new(CssPx(width), CssPx(height)),
        )
    }

    #[test]
    fn a_rectangle_on_whole_pixels_is_sent_as_it_is() {
        assert_eq!(
            outward(rect(8.0, 4.0, 320.0, 200.0)),
            Some((8, 4, 320, 200))
        );
        assert_eq!(inward(rect(8.0, 4.0, 320.0, 200.0)), Some((8, 4, 320, 200)));
    }

    #[test]
    fn input_grows_to_every_pixel_it_touches() {
        // A card at 1.25 scale whose border box is 101 device pixels from the left edge.
        assert_eq!(
            outward(rect(80.8, 0.4, 100.0, 10.0)),
            Some((80, 0, 101, 11))
        );
    }

    #[test]
    fn opacity_shrinks_to_the_pixels_it_covers() {
        assert_eq!(inward(rect(80.8, 0.4, 100.0, 10.0)), Some((81, 1, 99, 9)));
    }

    #[test]
    fn a_rectangle_with_no_pixel_left_is_dropped() {
        assert_eq!(inward(rect(0.2, 0.2, 0.6, 5.0)), None);
        assert_eq!(outward(rect(3.0, 3.0, 0.0, 5.0)), None);
        assert_eq!(outward(rect(f32::NAN, 0.0, 5.0, 5.0)), None);
    }
}
