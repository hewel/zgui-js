//! Pointer input in surface-local logical pixels.

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
pub enum InputRegion {
    /// Use the compositor default (the entire surface).
    #[default]
    Full,
    /// Pass through all pointer input.
    Empty,
    /// Accept input in the union of the rectangles.
    Rects(Vec<InputRect>),
}
