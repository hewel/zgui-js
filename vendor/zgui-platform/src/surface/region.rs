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
}
