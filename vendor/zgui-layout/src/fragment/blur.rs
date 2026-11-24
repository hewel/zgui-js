//! Pure geometric scan-conversion of rounded placed border boxes into integer horizontal blur
//! strips.
//!
//! This module contains zero Wayland, JavaScript, or shell concepts. It resolves eligible
//! fragment border boxes and their full ancestor clip chains through a caller-provided spatial
//! resolver, maps device pixels into surface-local logical pixels via exact per-axis
//! `destination / source` viewport ratios, samples every participating rounded/superellipse shape
//! at logical pixel-center rows `s = y + 0.5` using the same four-quadrant selection as
//! `sdf.wgsl`, coalesces identical horizontal spans across consecutive rows, and degrades safely
//! within [`MAX_BLUR_RECTS`] while preserving `output ⊆ step1`.

use zgui_css::ComputedStyle;
use zgui_geom::{Corners, Css, CssPx, Device, DevicePx, Matrix4, Point, Rect, Size, Vec2};
use zgui_scene::{ClipId, ClipLink, ClipNode, ClipTable, CornerShape, SpatialId};

use crate::fragment::custom_properties::CustomProperties;
use crate::fragment::{Fragment, FragmentKind, clip, corner};

/// Maximum number of output rectangles emitted for one surface blur region.
pub const MAX_BLUR_RECTS: usize = 256;

/// One candidate fragment piece eligible for native rounded blur rasterization.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlurPiece {
    /// Untransformed border box in the fragment's own coordinate space (device pixels).
    pub border_box: Rect<DevicePx, Device>,
    /// Fitted elliptical corner radii in device pixels.
    pub radii: Corners<Vec2<DevicePx>>,
    /// Corner shape exponent; non-finite values are rejected before clamping.
    pub shape: CornerShape,
    /// Optional transform coordinate system of the fragment (`None` means untransformed).
    pub transform: Option<SpatialId>,
    /// Ancestor clip chain applied to this fragment.
    pub clip: ClipId,
}

impl BlurPiece {
    /// Builds a [`BlurPiece`] from a laid-out fragment and its box's computed style.
    ///
    /// Returns `None` for non-eligible [`FragmentKind`]s or non-positive/non-finite scale.
    #[must_use]
    pub fn from_fragment(
        fragment: &Fragment,
        style: &ComputedStyle,
        declared: CustomProperties,
        scale: f32,
    ) -> Option<Self> {
        if !is_blur_eligible_kind(fragment.kind) || !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        Some(Self {
            border_box: fragment.border_box,
            radii: clip::radii(style, fragment.border_box, scale),
            shape: corner::shape_with(style, declared),
            transform: fragment.transform,
            clip: fragment.clip,
        })
    }
}

/// Mapping from physical buffer/device coordinates to surface-local logical coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlurSurfaceMapping {
    /// Physical buffer extent in device pixels (`source`).
    pub source: Size<DevicePx, Device>,
    /// Surface-local logical viewport extent in CSS pixels (`destination`).
    pub destination: Size<CssPx, Css>,
}

/// Returns whether a fragment kind may contribute its border box to native node blur.
#[must_use]
pub const fn is_blur_eligible_kind(kind: FragmentKind) -> bool {
    matches!(
        kind,
        FragmentKind::Box
            | FragmentKind::Replaced { .. }
            | FragmentKind::Vector
            | FragmentKind::Custom
    )
}

#[derive(Clone, Copy, Debug)]
struct LogicalRoundedRect {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
    tl: (f64, f64),
    tr: (f64, f64),
    br: (f64, f64),
    bl: (f64, f64),
    exponent: f64,
    is_square: bool,
}

impl LogicalRoundedRect {
    fn row_interval(&self, s: f64) -> Option<(f64, f64)> {
        if s < self.top || s >= self.bottom {
            return None;
        }
        if self.is_square {
            return Some((self.left, self.right));
        }
        let cx = (self.left + self.right) * 0.5;
        let cy = (self.top + self.bottom) * 0.5;
        let (left_r, right_r, is_top) = if s < cy {
            (self.tl, self.tr, true)
        } else {
            (self.bl, self.br, false)
        };

        let left_half = {
            let dy = if is_top {
                (self.top + left_r.1) - s
            } else {
                s - (self.bottom - left_r.1)
            };
            if dy <= 0.0 || left_r.0 <= 0.0 || left_r.1 <= 0.0 {
                Some((self.left, cx))
            } else {
                let u = (dy / left_r.1).clamp(0.0, 1.0);
                let dx = left_r.0 * superellipse_x(u, self.exponent);
                let x_left = self.left + (left_r.0 - dx);
                (x_left < cx).then_some((x_left, cx))
            }
        };

        let right_half = {
            let dy = if is_top {
                (self.top + right_r.1) - s
            } else {
                s - (self.bottom - right_r.1)
            };
            if dy <= 0.0 || right_r.0 <= 0.0 || right_r.1 <= 0.0 {
                Some((cx, self.right))
            } else {
                let u = (dy / right_r.1).clamp(0.0, 1.0);
                let dx = right_r.0 * superellipse_x(u, self.exponent);
                let x_right = self.right - (right_r.0 - dx);
                (x_right >= cx).then_some((cx, x_right))
            }
        };

        match (left_half, right_half) {
            (Some((l, _)), Some((_, r))) => Some((l, r)),
            (Some(interval), None) | (None, Some(interval)) => Some(interval),
            (None, None) => None,
        }
    }
}

fn superellipse_x(u: f64, exponent: f64) -> f64 {
    if u >= 1.0 {
        return 0.0;
    }
    if u <= 0.0 {
        return 1.0;
    }
    if exponent == 2.0 {
        (1.0 - u * u).max(0.0).sqrt()
    } else if exponent == 1.0 {
        (1.0 - u).max(0.0)
    } else {
        (1.0 - u.powf(exponent)).max(0.0).powf(1.0 / exponent)
    }
}

fn axis_aligned_positive_affine(matrix: Matrix4) -> Option<(f64, f64, f64, f64)> {
    let affine = matrix.to_affine2()?;
    let coeffs = [affine.a, affine.b, affine.c, affine.d, affine.tx, affine.ty];
    if !coeffs.iter().all(|c| c.is_finite()) {
        return None;
    }
    if affine.b != 0.0 || affine.c != 0.0 || affine.a <= 0.0 || affine.d <= 0.0 {
        return None;
    }
    Some((
        f64::from(affine.a),
        f64::from(affine.d),
        f64::from(affine.tx),
        f64::from(affine.ty),
    ))
}

fn transform_rounded_rect<F>(
    rect: Rect<DevicePx, Device>,
    radii: Corners<Vec2<DevicePx>>,
    shape: CornerShape,
    space: Option<SpatialId>,
    resolve_space: &F,
    sx: f64,
    sy: f64,
) -> Option<LogicalRoundedRect>
where
    F: Fn(SpatialId) -> Option<Matrix4>,
{
    let raw_exponent = f64::from(shape.get());
    if !raw_exponent.is_finite() {
        return None;
    }
    let exponent = raw_exponent.clamp(0.01, f64::from(CornerShape::NOTCH.get()));

    let ox = f64::from(rect.origin.x.0);
    let oy = f64::from(rect.origin.y.0);
    let rw = f64::from(rect.size.width.0);
    let rh = f64::from(rect.size.height.0);
    if !ox.is_finite() || !oy.is_finite() || !rw.is_finite() || !rh.is_finite() || rw <= 0.0 || rh <= 0.0
    {
        return None;
    }

    let raw_radii = [
        (f64::from(radii.top_left.x.0), f64::from(radii.top_left.y.0)),
        (f64::from(radii.top_right.x.0), f64::from(radii.top_right.y.0)),
        (f64::from(radii.bottom_right.x.0), f64::from(radii.bottom_right.y.0)),
        (f64::from(radii.bottom_left.x.0), f64::from(radii.bottom_left.y.0)),
    ];
    for (rx, ry) in raw_radii {
        if !rx.is_finite() || !ry.is_finite() || rx < 0.0 || ry < 0.0 {
            return None;
        }
    }

    let (a, d, tx, ty) = match space {
        None => (1.0, 1.0, 0.0, 0.0),
        Some(id) => axis_aligned_positive_affine(resolve_space(id)?)?,
    };

    // Fit radii within the untransformed device border box per CSS Backgrounds 3 §5.5.
    let mut fit = 1.0_f64;
    let check_fit = |sum: f64, extent: f64, factor: &mut f64| {
        if sum > extent && sum > 0.0 {
            *factor = factor.min(extent / sum);
        }
    };
    check_fit(raw_radii[0].0 + raw_radii[1].0, rw, &mut fit);
    check_fit(raw_radii[3].0 + raw_radii[2].0, rw, &mut fit);
    check_fit(raw_radii[0].1 + raw_radii[3].1, rh, &mut fit);
    check_fit(raw_radii[1].1 + raw_radii[2].1, rh, &mut fit);

    let fx = a * sx;
    let fy = d * sy;
    let left = (ox * a + tx) * sx;
    let top = (oy * d + ty) * sy;
    let width = rw * fx;
    let height = rh * fy;
    let right = left + width;
    let bottom = top + height;
    if !left.is_finite()
        || !top.is_finite()
        || !right.is_finite()
        || !bottom.is_finite()
        || width <= 0.0
        || height <= 0.0
    {
        return None;
    }

    let scale_corner = |(rx, ry): (f64, f64)| -> Option<(f64, f64)> {
        let sx_r = rx * fit * fx;
        let sy_r = ry * fit * fy;
        if !sx_r.is_finite() || !sy_r.is_finite() || sx_r < 0.0 || sy_r < 0.0 {
            return None;
        }
        Some((sx_r, sy_r))
    };
    let tl = scale_corner(raw_radii[0])?;
    let tr = scale_corner(raw_radii[1])?;
    let br = scale_corner(raw_radii[2])?;
    let bl = scale_corner(raw_radii[3])?;
    let is_square = [tl, tr, br, bl].iter().all(|&(rx, ry)| rx <= 0.0 || ry <= 0.0);

    Some(LogicalRoundedRect {
        left,
        top,
        right,
        bottom,
        tl,
        tr,
        br,
        bl,
        exponent,
        is_square,
    })
}

/// Half-open pixel-center coordinate quantizer `q(v) = ceil(v - 0.5)`.
///
/// An integer pixel index `i` (with center `i + 0.5`) lies in `[q(left), q(right))` iff
/// `left <= i + 0.5 < right`.
fn quantize_center(v: f64, max_bound: i32) -> i32 {
    let q = (v - 0.5).ceil();
    if !q.is_finite() || q <= 0.0 {
        0
    } else if q >= f64::from(max_bound) {
        max_bound
    } else {
        q as i32
    }
}

struct PreparedPiece {
    target: LogicalRoundedRect,
    rounded_clips: Vec<LogicalRoundedRect>,
    aabb_l: f64,
    aabb_r: f64,
    row_start: i32,
    row_end: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IntRect {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl IntRect {
    fn area(self) -> u64 {
        let w = u64::try_from(self.x1.saturating_sub(self.x0)).unwrap_or(0);
        let h = u64::try_from(self.y1.saturating_sub(self.y0)).unwrap_or(0);
        w.saturating_mul(h)
    }

    fn to_css_rect(self) -> Rect<CssPx, Css> {
        Rect::new(
            Point::new(CssPx(self.x0 as f32), CssPx(self.y0 as f32)),
            Size::new(
                CssPx((self.x1 - self.x0) as f32),
                CssPx((self.y1 - self.y0) as f32),
            ),
        )
    }
}

fn sort_scan_order(rects: &mut [IntRect]) {
    rects.sort_unstable_by_key(|r| (r.y0, r.x0, r.y1, r.x1));
}

fn union_spans(spans: &mut Vec<(i32, i32)>) {
    if spans.len() <= 1 {
        return;
    }
    spans.sort_unstable_by_key(|&(a, b)| (a, b));
    let mut write = 0;
    for read in 1..spans.len() {
        let next = spans[read];
        if next.0 <= spans[write].1 {
            spans[write].1 = spans[write].1.max(next.1);
        } else {
            write += 1;
            spans[write] = next;
        }
    }
    spans.truncate(write + 1);
}

fn intersect_span_lists(a: &[(i32, i32)], b: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let lo = a[i].0.max(b[j].0);
        let hi = a[i].1.min(b[j].1);
        if lo < hi {
            out.push((lo, hi));
        }
        if a[i].1 < b[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

/// Coalesces `(y0, y1, spans)` bands into disjoint vertically-merged integer rectangles.
fn coalesce_bands(bands: &[(i32, i32, Vec<(i32, i32)>)]) -> Vec<IntRect> {
    let mut finalized = Vec::new();
    // Open spans from the immediately preceding band: (x0, x1, y0, y1).
    let mut open: Vec<IntRect> = Vec::new();
    let mut next_open: Vec<IntRect> = Vec::new();

    for (y0, y1, spans) in bands {
        next_open.clear();
        let contiguous = open.first().is_some_and(|r| r.y1 == *y0);
        if !contiguous {
            finalized.append(&mut open);
            for &(x0, x1) in spans {
                next_open.push(IntRect {
                    x0,
                    y0: *y0,
                    x1,
                    y1: *y1,
                });
            }
            std::mem::swap(&mut open, &mut next_open);
            continue;
        }
        // Both `open` and `spans` are sorted by `(x0, x1)`.
        let (mut oi, mut si) = (0, 0);
        while oi < open.len() && si < spans.len() {
            let o = open[oi];
            let s = spans[si];
            match (o.x0, o.x1).cmp(&s) {
                std::cmp::Ordering::Equal => {
                    next_open.push(IntRect { y1: *y1, ..o });
                    oi += 1;
                    si += 1;
                }
                std::cmp::Ordering::Less => {
                    finalized.push(o);
                    oi += 1;
                }
                std::cmp::Ordering::Greater => {
                    next_open.push(IntRect {
                        x0: s.0,
                        y0: *y0,
                        x1: s.1,
                        y1: *y1,
                    });
                    si += 1;
                }
            }
        }
        while oi < open.len() {
            finalized.push(open[oi]);
            oi += 1;
        }
        while si < spans.len() {
            let s = spans[si];
            next_open.push(IntRect {
                x0: s.0,
                y0: *y0,
                x1: s.1,
                y1: *y1,
            });
            si += 1;
        }
        std::mem::swap(&mut open, &mut next_open);
    }
    finalized.append(&mut open);
    sort_scan_order(&mut finalized);
    finalized
}

fn degrade_to_budget(rows: &[(i32, Vec<(i32, i32)>)], step1: Vec<IntRect>) -> Vec<IntRect> {
    if step1.len() <= MAX_BLUR_RECTS {
        return step1;
    }

    // Group non-empty rows into contiguous runs so empty rows are never bridged.
    let mut runs: Vec<&[(i32, Vec<(i32, i32)>)]> = Vec::new();
    let mut run_start = 0;
    for i in 1..rows.len() {
        if rows[i].0 != rows[i - 1].0 + 1 {
            runs.push(&rows[run_start..i]);
            run_start = i;
        }
    }
    if run_start < rows.len() {
        runs.push(&rows[run_start..]);
    }

    let max_run_len = runs.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut best_fallback = rank_and_truncate(&step1);
    let mut best_area: u64 = best_fallback.iter().map(|r| r.area()).sum();

    let mut band = 2usize;
    while band <= max_run_len {
        let mut bands = Vec::new();
        for run in &runs {
            let mut offset = 0;
            while offset < run.len() {
                let end = (offset + band).min(run.len());
                let slice = &run[offset..end];
                let mut acc = slice[0].1.clone();
                for row in &slice[1..] {
                    acc = intersect_span_lists(&acc, &row.1);
                    if acc.is_empty() {
                        break;
                    }
                }
                if !acc.is_empty() {
                    let y0 = slice[0].0;
                    let y1 = slice[slice.len() - 1].0 + 1;
                    bands.push((y0, y1, acc));
                }
                offset = end;
            }
        }
        let coalesced = coalesce_bands(&bands);
        if coalesced.len() <= MAX_BLUR_RECTS {
            return coalesced;
        }
        let truncated = rank_and_truncate(&coalesced);
        let area: u64 = truncated.iter().map(|r| r.area()).sum();
        if area > best_area {
            best_area = area;
            best_fallback = truncated;
        }
        match band.checked_mul(2) {
            Some(next) => band = next,
            None => break,
        }
    }

    best_fallback
}

fn rank_and_truncate(rects: &[IntRect]) -> Vec<IntRect> {
    let mut ranked = rects.to_vec();
    ranked.sort_unstable_by(|a, b| {
        b.area()
            .cmp(&a.area())
            .then_with(|| (a.y0, a.x0, a.y1, a.x1).cmp(&(b.y0, b.x0, b.y1, b.x1)))
    });
    ranked.truncate(MAX_BLUR_RECTS);
    sort_scan_order(&mut ranked);
    ranked
}

/// Resolves a slice of [`BlurPiece`]s into at most [`MAX_BLUR_RECTS`] pixel-center-covered
/// integer logical blur rectangles, clipped to `mapping.destination` and all ancestor clips.
pub fn resolve_blur_rects<F>(
    pieces: &[BlurPiece],
    clips: &ClipTable,
    resolve_space: F,
    mapping: BlurSurfaceMapping,
) -> Vec<Rect<CssPx, Css>>
where
    F: Fn(SpatialId) -> Option<Matrix4>,
{
    if pieces.is_empty() {
        return Vec::new();
    }
    let src_w = f64::from(mapping.source.width.0);
    let src_h = f64::from(mapping.source.height.0);
    let dst_w = f64::from(mapping.destination.width.0);
    let dst_h = f64::from(mapping.destination.height.0);
    if !src_w.is_finite()
        || !src_h.is_finite()
        || !dst_w.is_finite()
        || !dst_h.is_finite()
        || src_w <= 0.0
        || src_h <= 0.0
        || dst_w <= 0.0
        || dst_h <= 0.0
        || dst_w > f64::from(i32::MAX)
        || dst_h > f64::from(i32::MAX)
    {
        return Vec::new();
    }

    let max_x = quantize_center(dst_w, i32::MAX);
    let max_y = quantize_center(dst_h, i32::MAX);
    if max_x <= 0 || max_y <= 0 {
        return Vec::new();
    }

    let sx = dst_w / src_w;
    let sy = dst_h / src_h;
    if !sx.is_finite() || !sy.is_finite() || sx <= 0.0 || sy <= 0.0 {
        return Vec::new();
    }

    let mut prepared = Vec::with_capacity(pieces.len());
    'pieces: for piece in pieces {
        let Some(target) = transform_rounded_rect(
            piece.border_box,
            piece.radii,
            piece.shape,
            piece.transform,
            &resolve_space,
            sx,
            sy,
        ) else {
            continue;
        };

        let mut aabb_l = target.left.max(0.0);
        let mut aabb_t = target.top.max(0.0);
        let mut aabb_r = target.right.min(dst_w);
        let mut aabb_b = target.bottom.min(dst_h);
        let mut rounded_clips = Vec::new();

        // Walk the full clip chain via `ClipTable::get` (never flattened `resolve_placed`).
        let mut cursor = piece.clip;
        let mut steps = 0usize;
        let limit = clips.len().saturating_add(1);
        loop {
            if steps > limit {
                continue 'pieces;
            }
            steps += 1;
            let Some(node) = clips.get(cursor) else {
                continue 'pieces;
            };
            match node {
                ClipNode::Root => break,
                ClipNode::Link { link, parent, .. } => {
                    match link {
                        ClipLink::RoundedRect {
                            rect,
                            radii,
                            shape,
                            space,
                        } => {
                            let Some(c) = transform_rounded_rect(
                                *rect,
                                *radii,
                                *shape,
                                Some(*space),
                                &resolve_space,
                                sx,
                                sy,
                            ) else {
                                continue 'pieces;
                            };
                            aabb_l = aabb_l.max(c.left);
                            aabb_t = aabb_t.max(c.top);
                            aabb_r = aabb_r.min(c.right);
                            aabb_b = aabb_b.min(c.bottom);
                            if !c.is_square {
                                rounded_clips.push(c);
                            }
                        }
                        ClipLink::Mask { .. } => continue 'pieces,
                    }
                    cursor = *parent;
                }
            }
        }

        if aabb_l >= aabb_r || aabb_t >= aabb_b {
            continue;
        }
        let row_start = quantize_center(aabb_t, max_y);
        let row_end = quantize_center(aabb_b, max_y);
        if row_start >= row_end {
            continue;
        }
        prepared.push(PreparedPiece {
            target,
            rounded_clips,
            aabb_l,
            aabb_r,
            row_start,
            row_end,
        });
    }

    if prepared.is_empty() {
        return Vec::new();
    }

    // Unclipped (or square-clipped) single square-cornered box fast path.
    if prepared.len() == 1
        && prepared[0].target.is_square
        && prepared[0].rounded_clips.is_empty()
    {
        let p = &prepared[0];
        let x0 = quantize_center(p.aabb_l, max_x);
        let x1 = quantize_center(p.aabb_r, max_x);
        if x0 < x1 && p.row_start < p.row_end {
            return vec![
                IntRect {
                    x0,
                    y0: p.row_start,
                    x1,
                    y1: p.row_end,
                }
                .to_css_rect(),
            ];
        }
        return Vec::new();
    }

    // Collect merged active row ranges across all prepared pieces.
    let mut row_ranges: Vec<(i32, i32)> = prepared
        .iter()
        .map(|p| (p.row_start, p.row_end))
        .collect();
    union_spans(&mut row_ranges);

    let mut rows: Vec<(i32, Vec<(i32, i32)>)> = Vec::new();
    let mut row_spans = Vec::new();

    for (range_start, range_end) in row_ranges {
        for y in range_start..range_end {
            let s = f64::from(y) + 0.5;
            row_spans.clear();
            for piece in &prepared {
                if y < piece.row_start || y >= piece.row_end {
                    continue;
                }
                let Some((tl, tr)) = piece.target.row_interval(s) else {
                    continue;
                };
                let mut left = piece.aabb_l.max(tl);
                let mut right = piece.aabb_r.min(tr);
                if left >= right {
                    continue;
                }
                let mut empty = false;
                for c in &piece.rounded_clips {
                    let Some((cl, cr)) = c.row_interval(s) else {
                        empty = true;
                        break;
                    };
                    left = left.max(cl);
                    right = right.min(cr);
                    if left >= right {
                        empty = true;
                        break;
                    }
                }
                if empty {
                    continue;
                }
                let x0 = quantize_center(left, max_x);
                let x1 = quantize_center(right, max_x);
                if x0 < x1 {
                    row_spans.push((x0, x1));
                }
            }
            if !row_spans.is_empty() {
                union_spans(&mut row_spans);
                rows.push((y, row_spans.clone()));
            }
        }
    }

    if rows.is_empty() {
        return Vec::new();
    }

    let step1_bands: Vec<(i32, i32, Vec<(i32, i32)>)> = rows
        .iter()
        .map(|(y, spans)| (*y, *y + 1, spans.clone()))
        .collect();
    let step1 = coalesce_bands(&step1_bands);
    let final_rects = degrade_to_budget(&rows, step1);
    final_rects.into_iter().map(IntRect::to_css_rect).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zgui_geom::Affine2;
    use zgui_scene::{MaskSource, OwnSpace, PropertyOwner, SpatialTree};

    fn drect(x: f32, y: f32, w: f32, h: f32) -> Rect<DevicePx, Device> {
        Rect::new(
            Point::new(DevicePx(x), DevicePx(y)),
            Size::new(DevicePx(w), DevicePx(h)),
        )
    }

    fn map_1x(w: f32, h: f32) -> BlurSurfaceMapping {
        BlurSurfaceMapping {
            source: Size::new(DevicePx(w), DevicePx(h)),
            destination: Size::new(CssPx(w), CssPx(h)),
        }
    }

    /// Completely independent brute-force pixel-center point-membership oracle matching `sdf.wgsl`
    /// quadrant rules and half-open `[left, right) x [top, bottom)` boundaries.
    fn oracle_point_in_shape(shape: &LogicalRoundedRect, px: f64, py: f64) -> bool {
        if px < shape.left || px >= shape.right || py < shape.top || py >= shape.bottom {
            return false;
        }
        if shape.is_square {
            return true;
        }
        let cx = (shape.left + shape.right) * 0.5;
        let cy = (shape.top + shape.bottom) * 0.5;
        let hx = (shape.right - shape.left) * 0.5;
        let hy = (shape.bottom - shape.top) * 0.5;
        let dx_c = px - cx;
        let dy_c = py - cy;
        let (rx, ry) = if dx_c < 0.0 {
            if dy_c < 0.0 { shape.tl } else { shape.bl }
        } else if dy_c < 0.0 {
            shape.tr
        } else {
            shape.br
        };
        let ccp_x = dx_c.abs() - hx + rx;
        let ccp_y = dy_c.abs() - hy + ry;
        if ccp_x <= 0.0 || ccp_y <= 0.0 || rx <= 0.0 || ry <= 0.0 {
            return true;
        }
        let norm = (ccp_x / rx).powf(shape.exponent) + (ccp_y / ry).powf(shape.exponent);
        // Half-open boundary: left edge includes exact curve (`<= 1`), right edge excludes (`< 1`).
        if dx_c < 0.0 { norm <= 1.0 } else { norm < 1.0 }
    }

    fn oracle_contains_pixel<F>(
        pieces: &[BlurPiece],
        clips: &ClipTable,
        resolve_space: &F,
        mapping: BlurSurfaceMapping,
        x: i32,
        y: i32,
    ) -> bool
    where
        F: Fn(SpatialId) -> Option<Matrix4>,
    {
        let px = f64::from(x) + 0.5;
        let py = f64::from(y) + 0.5;
        let dst_w = f64::from(mapping.destination.width.0);
        let dst_h = f64::from(mapping.destination.height.0);
        if px < 0.0 || px >= dst_w || py < 0.0 || py >= dst_h {
            return false;
        }
        let sx = dst_w / f64::from(mapping.source.width.0);
        let sy = dst_h / f64::from(mapping.source.height.0);

        'pieces: for piece in pieces {
            let Some(target) = transform_rounded_rect(
                piece.border_box,
                piece.radii,
                piece.shape,
                piece.transform,
                resolve_space,
                sx,
                sy,
            ) else {
                continue;
            };
            if !oracle_point_in_shape(&target, px, py) {
                continue;
            }
            let mut cursor = piece.clip;
            loop {
                let Some(node) = clips.get(cursor) else {
                    continue 'pieces;
                };
                match node {
                    ClipNode::Root => return true,
                    ClipNode::Link { link, parent, .. } => {
                        match link {
                            ClipLink::RoundedRect {
                                rect,
                                radii,
                                shape,
                                space,
                            } => {
                                let Some(c) = transform_rounded_rect(
                                    *rect,
                                    *radii,
                                    *shape,
                                    Some(*space),
                                    resolve_space,
                                    sx,
                                    sy,
                                ) else {
                                    continue 'pieces;
                                };
                                if !oracle_point_in_shape(&c, px, py) {
                                    continue 'pieces;
                                }
                            }
                            ClipLink::Mask { .. } => continue 'pieces,
                        }
                        cursor = *parent;
                    }
                }
            }
        }
        false
    }

    fn rects_cover_pixel(rects: &[Rect<CssPx, Css>], x: i32, y: i32) -> bool {
        let px = f64::from(x) + 0.5;
        let py = f64::from(y) + 0.5;
        rects.iter().any(|r| {
            let l = f64::from(r.origin.x.0);
            let t = f64::from(r.origin.y.0);
            let rr = l + f64::from(r.size.width.0);
            let bb = t + f64::from(r.size.height.0);
            px >= l && px < rr && py >= t && py < bb
        })
    }

    fn assert_matches_oracle<F>(
        pieces: &[BlurPiece],
        clips: &ClipTable,
        resolve_space: F,
        mapping: BlurSurfaceMapping,
        label: &str,
    ) -> Vec<Rect<CssPx, Css>>
    where
        F: Fn(SpatialId) -> Option<Matrix4>,
    {
        let rects = resolve_blur_rects(pieces, clips, &resolve_space, mapping);
        assert!(rects.len() <= MAX_BLUR_RECTS, "{label}: exceeded budget");
        let w = mapping.destination.width.0.ceil() as i32;
        let h = mapping.destination.height.0.ceil() as i32;
        for y in 0..h {
            for x in 0..w {
                let expected = oracle_contains_pixel(pieces, clips, &resolve_space, mapping, x, y);
                let actual = rects_cover_pixel(&rects, x, y);
                assert_eq!(
                    actual, expected,
                    "{label}: mismatch at pixel ({x}, {y}) (rects={rects:?})"
                );
            }
        }
        rects
    }

    #[test]
    fn half_pixel_ties_and_adjacent_tiling_match_oracle() {
        let clips = ClipTable::rooted();
        let a = BlurPiece {
            border_box: drect(10.5, 5.5, 10.0, 8.0),
            radii: Corners::uniform(Vec2::splat(DevicePx(0.0))),
            shape: CornerShape::ROUND,
            transform: None,
            clip: ClipId::ROOT,
        };
        let b = BlurPiece {
            border_box: drect(20.5, 5.5, 10.0, 8.0),
            radii: Corners::uniform(Vec2::splat(DevicePx(0.0))),
            shape: CornerShape::ROUND,
            transform: None,
            clip: ClipId::ROOT,
        };
        let r_a = assert_matches_oracle(&[a], &clips, |_| Some(Matrix4::IDENTITY), map_1x(40.0, 20.0), "tie_a");
        assert_eq!(r_a.len(), 1);
        assert_eq!(r_a[0].origin, Point::new(CssPx(10.0), CssPx(5.0)));
        assert_eq!(r_a[0].size, Size::new(CssPx(10.0), CssPx(8.0)));

        let r_ab = assert_matches_oracle(&[a, b], &clips, |_| Some(Matrix4::IDENTITY), map_1x(40.0, 20.0), "tie_ab");
        assert_eq!(r_ab.len(), 1, "adjacent .5-tied boxes merge cleanly");
        assert_eq!(r_ab[0].origin, Point::new(CssPx(10.0), CssPx(5.0)));
        assert_eq!(r_ab[0].size, Size::new(CssPx(20.0), CssPx(8.0)));
    }

    #[test]
    fn all_corner_shapes_and_asymmetric_large_radii_match_oracle() {
        let clips = ClipTable::rooted();
        // Single corner exceeding half width and half height (tl = 70x45 on 100x60 box).
        let asymmetric = Corners::new(
            Vec2::new(DevicePx(70.0), DevicePx(45.0)),
            Vec2::new(DevicePx(0.0), DevicePx(0.0)),
            Vec2::new(DevicePx(18.0), DevicePx(12.0)),
            Vec2::new(DevicePx(10.0), DevicePx(15.0)),
        );
        for (name, shape) in [
            ("round", CornerShape::ROUND),
            ("squircle", CornerShape::SQUIRCLE),
            ("superellipse_2_83", CornerShape::new(2.83)),
            ("bevel", CornerShape::BEVEL),
            ("scoop", CornerShape::SCOOP),
            ("notch", CornerShape::NOTCH),
        ] {
            let piece = BlurPiece {
                border_box: drect(10.0, 8.0, 100.0, 60.0),
                radii: asymmetric,
                shape,
                transform: None,
                clip: ClipId::ROOT,
            };
            let rects = assert_matches_oracle(
                &[piece],
                &clips,
                |_| Some(Matrix4::IDENTITY),
                map_1x(120.0, 80.0),
                name,
            );
            assert!(!rects.is_empty(), "{name} should emit blur strips");
        }
    }

    #[test]
    fn superellipse_2_83_r22_has_no_corner_l_notch() {
        let clips = ClipTable::rooted();
        let piece = BlurPiece {
            border_box: drect(0.0, 0.0, 100.0, 80.0),
            radii: Corners::uniform(Vec2::splat(DevicePx(22.0))),
            shape: CornerShape::new(2.83),
            transform: None,
            clip: ClipId::ROOT,
        };
        let rects = assert_matches_oracle(
            &[piece],
            &clips,
            |_| Some(Matrix4::IDENTITY),
            map_1x(100.0, 80.0),
            "superellipse_2_83_r22",
        );
        // Row y=0 (s=0.5) should cover flat tangent pixels near the top edge (not carved out by
        // conservative outer-edge ceil).
        assert!(rects_cover_pixel(&rects, 16, 0));
        assert!(!rects_cover_pixel(&rects, 0, 0));
    }

    #[test]
    fn non_finite_corner_shape_on_target_or_clip_is_rejected() {
        let mut clips = ClipTable::rooted();
        let valid_piece = BlurPiece {
            border_box: drect(10.0, 10.0, 40.0, 40.0),
            radii: Corners::uniform(Vec2::splat(DevicePx(8.0))),
            shape: CornerShape(f32::NAN),
            transform: None,
            clip: ClipId::ROOT,
        };
        assert!(resolve_blur_rects(&[valid_piece], &clips, |_| Some(Matrix4::IDENTITY), map_1x(60.0, 60.0)).is_empty());

        let inf_piece = BlurPiece {
            shape: CornerShape(f32::INFINITY),
            ..valid_piece
        };
        assert!(resolve_blur_rects(&[inf_piece], &clips, |_| Some(Matrix4::IDENTITY), map_1x(60.0, 60.0)).is_empty());

        let bad_clip = clips.push(
            ClipId::ROOT,
            ClipLink::RoundedRect {
                rect: drect(0.0, 0.0, 60.0, 60.0),
                radii: Corners::uniform(Vec2::splat(DevicePx(10.0))),
                shape: CornerShape(f32::NEG_INFINITY),
                space: SpatialId::VIEWPORT,
            },
        );
        let clipped_piece = BlurPiece {
            shape: CornerShape::ROUND,
            clip: bad_clip,
            ..valid_piece
        };
        assert!(resolve_blur_rects(&[clipped_piece], &clips, |_| Some(Matrix4::IDENTITY), map_1x(60.0, 60.0)).is_empty());
    }

    #[test]
    fn three_differently_placed_clip_links_and_fractional_scale_match_oracle() {
        let mut tree = SpatialTree::with_viewport();
        let vp = tree.viewport();
        let s1 = tree.space_of(
            vp,
            PropertyOwner::new(1).unwrap(),
            OwnSpace::of(Some(Matrix4::translation(4.5, 3.0, 0.0)), None, false),
        );
        let s2 = tree.space_of(
            s1,
            PropertyOwner::new(2).unwrap(),
            OwnSpace::of(Some(Affine2::scale(0.97, 0.97).to_matrix4()), None, false),
        );
        let s3 = tree.space_of(
            s2,
            PropertyOwner::new(3).unwrap(),
            OwnSpace::of(Some(Matrix4::translation(6.25, -2.5, 0.0)), None, false),
        );

        let mut clips = ClipTable::rooted();
        let c1 = clips.push(
            ClipId::ROOT,
            ClipLink::shaped(
                drect(5.0, 5.0, 110.0, 75.0),
                Corners::uniform(Vec2::splat(DevicePx(14.0))),
                CornerShape::ROUND,
                s1,
            ),
        );
        let c2 = clips.push(
            c1,
            ClipLink::shaped(
                drect(10.0, 8.0, 95.0, 65.0),
                Corners::uniform(Vec2::splat(DevicePx(12.0))),
                CornerShape::SQUIRCLE,
                s2,
            ),
        );
        let c3 = clips.push(
            c2,
            ClipLink::shaped(
                drect(4.0, 10.0, 85.0, 60.0),
                Corners::uniform(Vec2::splat(DevicePx(10.0))),
                CornerShape::BEVEL,
                s3,
            ),
        );

        let piece = BlurPiece {
            border_box: drect(0.0, 0.0, 100.0, 70.0),
            radii: Corners::uniform(Vec2::splat(DevicePx(16.0))),
            shape: CornerShape::new(2.83),
            transform: Some(s3),
            clip: c3,
        };

        // Odd logical extent 101x81 at 1.25x -> 126x101 physical buffer.
        let mapping = BlurSurfaceMapping {
            source: Size::new(DevicePx(126.0), DevicePx(101.0)),
            destination: Size::new(CssPx(101.0), CssPx(81.0)),
        };
        let rects = assert_matches_oracle(
            &[piece],
            &clips,
            |id| tree.resolve(id),
            mapping,
            "three_clips_097_125x",
        );
        assert!(!rects.is_empty());
    }

    #[test]
    fn sub_1x_scale_distinguishes_101_vs_102_logical_width_for_same_51px_buffer() {
        let clips = ClipTable::rooted();
        let piece = BlurPiece {
            border_box: drect(0.0, 0.0, 51.0, 20.0),
            radii: Corners::uniform(Vec2::splat(DevicePx(0.0))),
            shape: CornerShape::ROUND,
            transform: None,
            clip: ClipId::ROOT,
        };
        let r101 = assert_matches_oracle(
            &[piece],
            &clips,
            |_| Some(Matrix4::IDENTITY),
            BlurSurfaceMapping {
                source: Size::new(DevicePx(51.0), DevicePx(20.0)),
                destination: Size::new(CssPx(101.0), CssPx(40.0)),
            },
            "0.5x_101",
        );
        let r102 = assert_matches_oracle(
            &[piece],
            &clips,
            |_| Some(Matrix4::IDENTITY),
            BlurSurfaceMapping {
                source: Size::new(DevicePx(51.0), DevicePx(20.0)),
                destination: Size::new(CssPx(102.0), CssPx(40.0)),
            },
            "0.5x_102",
        );
        assert_eq!(r101[0].size.width, CssPx(101.0));
        assert_eq!(r102[0].size.width, CssPx(102.0));
    }

    #[test]
    fn unsupported_transforms_masks_and_missing_spaces_are_conservatively_omitted() {
        let mut tree = SpatialTree::with_viewport();
        let vp = tree.viewport();
        let rotated = tree.space_of(
            vp,
            PropertyOwner::new(10).unwrap(),
            OwnSpace::of(Some(Affine2::rotation(0.25).to_matrix4()), None, false),
        );
        let skewed = tree.space_of(
            vp,
            PropertyOwner::new(11).unwrap(),
            OwnSpace::of(Some(Affine2::skew(0.2, 0.0).to_matrix4()), None, false),
        );
        let mirrored = tree.space_of(
            vp,
            PropertyOwner::new(12).unwrap(),
            OwnSpace::of(Some(Affine2::scale(-1.0, 1.0).to_matrix4()), None, false),
        );
        let persp = tree.space_of(
            vp,
            PropertyOwner::new(13).unwrap(),
            OwnSpace::of(Some(Matrix4::perspective(400.0)), None, false),
        );
        let mut other_tree = SpatialTree::with_viewport();
        let missing = other_tree.space_of(
            other_tree.viewport(),
            PropertyOwner::new(99).unwrap(),
            OwnSpace::of(Some(Matrix4::translation(1.0, 2.0, 0.0)), None, false),
        );

        let mut clips = ClipTable::rooted();
        let base = BlurPiece {
            border_box: drect(10.0, 10.0, 40.0, 40.0),
            radii: Corners::uniform(Vec2::splat(DevicePx(6.0))),
            shape: CornerShape::ROUND,
            transform: None,
            clip: ClipId::ROOT,
        };
        for space in [rotated, skewed, mirrored, persp, missing] {
            let p = BlurPiece {
                transform: Some(space),
                ..base
            };
            assert!(
                resolve_blur_rects(&[p], &clips, |id| tree.resolve(id), map_1x(100.0, 100.0)).is_empty(),
                "transform {space:?} must be omitted"
            );
        }

        let mask_clip = clips.push(
            ClipId::ROOT,
            ClipLink::Mask {
                tile: zgui_atlas::AtlasTile {
                    texture: zgui_atlas::TextureId {
                        kind: zgui_atlas::TextureKind::Color,
                        index: 0,
                    },
                    tile: zgui_atlas::TileId(0),
                    bounds: Rect::new(Point::new(0, 0), Size::new(10, 10)),
                },
                transform: vp,
                source: MaskSource::Path,
            },
        );
        let masked = BlurPiece {
            clip: mask_clip,
            ..base
        };
        assert!(resolve_blur_rects(&[masked], &clips, |id| tree.resolve(id), map_1x(100.0, 100.0)).is_empty());

        // Fragment kind whitelist
        assert!(is_blur_eligible_kind(FragmentKind::Box));
        assert!(is_blur_eligible_kind(FragmentKind::Vector));
        assert!(is_blur_eligible_kind(FragmentKind::Custom));
        assert!(!is_blur_eligible_kind(FragmentKind::Line {
            paragraph: crate::fragment::ParagraphId(0),
            line: 0,
        }));
        assert!(!is_blur_eligible_kind(FragmentKind::Scrollbar {
            axis: crate::Axis::Vertical,
            part: crate::fragment::ScrollbarPart::Thumb,
        }));
    }

    #[test]
    fn degradation_guarantees_subset_of_step1_zero_bridged_holes_and_budget_cap() {
        let clips = ClipTable::rooted();
        // Two tall curved circles with a clear empty horizontal gap between them (total > 256
        // step-1 strips).
        let top_circle = BlurPiece {
            border_box: drect(10.0, 0.0, 300.0, 300.0),
            radii: Corners::uniform(Vec2::splat(DevicePx(150.0))),
            shape: CornerShape::ROUND,
            transform: None,
            clip: ClipId::ROOT,
        };
        let bottom_circle = BlurPiece {
            border_box: drect(10.0, 310.0, 300.0, 300.0),
            radii: Corners::uniform(Vec2::splat(DevicePx(150.0))),
            shape: CornerShape::ROUND,
            transform: None,
            clip: ClipId::ROOT,
        };
        let mapping = map_1x(320.0, 620.0);
        let rects = resolve_blur_rects(
            &[top_circle, bottom_circle],
            &clips,
            |_| Some(Matrix4::IDENTITY),
            mapping,
        );
        assert!(rects.len() <= MAX_BLUR_RECTS);
        assert!(!rects.is_empty());

        // Every output pixel must be accepted by the exact Step-1 oracle (output ⊆ step1).
        for y in 0..620 {
            for x in 0..320 {
                if rects_cover_pixel(&rects, x, y) {
                    assert!(
                        oracle_contains_pixel(
                            &[top_circle, bottom_circle],
                            &clips,
                            &|_| Some(Matrix4::IDENTITY),
                            mapping,
                            x,
                            y,
                        ),
                        "degraded pixel ({x}, {y}) escaped step-1 interior"
                    );
                }
            }
        }
        // Gap rows 300..310 must remain completely empty (zero bridged holes).
        for y in 300..310 {
            for x in 0..320 {
                assert!(!rects_cover_pixel(&rects, x, y), "gap row {y} bridged at x={x}");
            }
        }
    }

    #[test]
    fn degradation_fallback_caps_over_256_disjoint_islands_deterministically() {
        let clips = ClipTable::rooted();
        // 300 disjoint 4x4 squares with 2px gaps vertically and horizontally.
        let mut pieces = Vec::new();
        for row in 0..30 {
            for col in 0..10 {
                pieces.push(BlurPiece {
                    border_box: drect((col * 6) as f32, (row * 6) as f32, 4.0, 4.0),
                    radii: Corners::uniform(Vec2::splat(DevicePx(0.0))),
                    shape: CornerShape::ROUND,
                    transform: None,
                    clip: ClipId::ROOT,
                });
            }
        }
        let mapping = map_1x(64.0, 184.0);
        let first = resolve_blur_rects(&pieces, &clips, |_| Some(Matrix4::IDENTITY), mapping);
        let second = resolve_blur_rects(&pieces, &clips, |_| Some(Matrix4::IDENTITY), mapping);
        assert_eq!(first, second, "degradation must be deterministic");
        assert_eq!(first.len(), MAX_BLUR_RECTS);
        for y in 0..184 {
            for x in 0..64 {
                if rects_cover_pixel(&first, x, y) {
                    assert!(
                        oracle_contains_pixel(&pieces, &clips, &|_| Some(Matrix4::IDENTITY), mapping, x, y),
                        "fallback pixel ({x}, {y}) escaped step-1 interior"
                    );
                }
            }
        }
    }
}
