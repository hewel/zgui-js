//! Surface-local logical regions for pointer input and background effects.

use zgui_geom::{Css, CssPx, Rect};

/// One rectangle in surface-local logical coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputRect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width; a non-positive extent contributes no input area.
    pub width: i32,
    /// Height; a non-positive extent contributes no input area.
    pub height: i32,
}

/// The part of a surface that receives input; the rest passes through.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum InputRegion {
    /// Use the compositor default (the entire surface).
    #[default]
    Full,
    /// Pass through all pointer input.
    Empty,
    /// Accept input in the union of the rectangles.
    Rects(Vec<InputRect>),
}

/// A surface-local rounded or superellipse blur mask in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlurShape {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Horizontal extent.
    pub width: f64,
    /// Vertical extent.
    pub height: f64,
    /// Scalar radii in top-left, top-right, bottom-right, bottom-left order.
    pub radii: [f64; 4],
    /// Superellipse exponent (2 for circular corners).
    pub corner_exponent: f64,
    /// Blur mask opacity, from zero to one.
    pub opacity: f64,
}

impl BlurShape {
    /// Maximum number of shapes accepted in one region by the shape protocol.
    pub const MAX_COUNT: usize = 256;

    /// Normalizes a shape to signed 24.8 fixed point, rounding shared edges rather than
    /// independent widths. Invalid geometry and zero wire opacity return `None`.
    #[must_use]
    pub fn wire_canonical(self) -> Option<Self> {
        fn ticks(value: f64) -> Option<i32> {
            let ticks = (value * 256.0).round();
            (f64::from(i32::MIN)..=f64::from(i32::MAX))
                .contains(&ticks)
                .then_some(ticks as i32)
        }
        let fixed = |ticks| f64::from(ticks) / 256.0;
        let x = ticks(self.x)?;
        let y = ticks(self.y)?;
        let width = ticks(self.x + self.width)?.checked_sub(x)?;
        let height = ticks(self.y + self.height)?.checked_sub(y)?;
        if width <= 0 || height <= 0 || !self.opacity.is_finite() || !self.corner_exponent.is_finite()
        {
            return None;
        }
        let opacity = ticks(self.opacity.clamp(0.0, 1.0))?;
        if opacity == 0 {
            return None;
        }
        let mut radii = self.radii;
        for radius in &mut radii {
            if !radius.is_finite() {
                return None;
            }
            *radius = fixed(ticks(radius.max(0.0))?);
        }
        let exponent = if self.corner_exponent <= 0.0 {
            2.0
        } else {
            self.corner_exponent.clamp(3.0 / 256.0, 64.0)
        };
        Some(Self {
            x: fixed(x),
            y: fixed(y),
            width: fixed(width),
            height: fixed(height),
            radii,
            corner_exponent: fixed(ticks(exponent)?),
            opacity: fixed(opacity),
        })
    }
}

/// The part of a surface that requests compositor background blur, in CSS pixels.
///
/// An empty [`BlurRegion::Rects`] slice normalizes to [`BlurRegion::None`]. Keeping
/// [`BlurRegion::Full`] distinct lets full-surface blur survive resizes without round-tripping
/// through application code, while [`BlurRegion::None`] clears any active blur region.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum BlurRegion<'a> {
    /// No background blur is requested; clears any active blur region.
    #[default]
    None,
    /// Request background blur across the full surface extent.
    Full,
    /// Request background blur behind the union of these surface-local CSS rectangles.
    Rects(&'a [Rect<CssPx, Css>]),
    /// Request these non-overlapping rounded masks on a shape-capable surface.
    Shapes(&'a [BlurShape]),
}
