//! Turning one glyph into pixels.

use core::ops::Range;
use std::sync::{Arc, Mutex};

use smallvec::SmallVec;
use swash::FontRef;
use swash::scale::image::{Content, Image};
use swash::scale::outline::Outline;
use swash::scale::{Render, ScaleContext, Scaler, Source, StrikeWith};
use swash::zeno::{Angle, Format, Mask, Origin, Point, Style, Transform, Vector, Verb};
use zgui_text::{
    FaceId, GlyphImage, GlyphKey, GlyphOutline, GlyphRaster, OutlineKey, RasterStyle,
    VariationInstance,
};

use crate::raster::faces::FaceBytes;
use crate::raster::image::convert;
use crate::raster::outline::Outlines;
use crate::system::FontSystem;

/// The sources a monochrome or subpixel request draws from, in order of preference.
///
/// An outline first, then an alpha bitmap strike for the faces that ship one instead — a bitmap-only
/// face would otherwise rasterise to nothing at all.
const MASK_SOURCES: [Source; 2] = [Source::Outline, Source::Bitmap(StrikeWith::BestFit)];

/// The sources a colour request draws from, in order of preference.
///
/// Layered colour outlines first, then a colour bitmap strike, then the plain outline. The last is
/// what makes a colour request against an ordinary text face produce the glyph in a single colour
/// rather than produce nothing.
const COLOR_SOURCES: [Source; 3] = [
    Source::ColorOutline(0),
    Source::ColorBitmap(StrikeWith::BestFit),
    Source::Outline,
];

/// Color-only sources checked before falling back to a plain emboldened outline.
const COLOR_ONLY_SOURCES: [Source; 2] = [
    Source::ColorOutline(0),
    Source::ColorBitmap(StrikeWith::BestFit),
];

/// Alpha bitmap fallback for bitmap-only faces when synthetic bolding is requested.
const BITMAP_ONLY_SOURCES: [Source; 1] = [Source::Bitmap(StrikeWith::BestFit)];

/// How far the outline moves along x for the red, the green and the blue coverage, in pixels.
///
/// The red stripe is a third of a pixel to the left of the centre of its pixel. The outline moves
/// to the right by that distance, so the centre of the pixel reads the coverage at the red stripe.
/// The blue stripe is the mirror of that.
const STRIPE_OFFSETS: [f32; 3] = [1.0 / 3.0, 0.0, -1.0 / 3.0];

/// Rasterises glyphs from the faces one font system holds.
///
/// Hinted, and capable of subpixel coverage and of the face's own colour glyphs. Which of those a
/// glyph gets is entirely in the key it is asked for, never in state held here — a hinting setting
/// kept on the side would silently change what an already cached key meant.
///
/// ```
/// use std::sync::Arc;
/// use zgui_text::{GlyphKey, GlyphRaster, FaceId, RasterStyle, SubpixelOffset};
/// use zgui_text_parley::{FontSystem, FontSystemOptions, Rasteriser};
///
/// let fonts = Arc::new(FontSystem::new(FontSystemOptions::registered_only()));
/// let raster = Rasteriser::new(fonts);
///
/// // A handle no system issued has no glyph, which is not the same as a blank one.
/// let key = GlyphKey::new(FaceId(7), 1, 16.0, SubpixelOffset(0), RasterStyle::Grayscale);
/// assert!(raster.raster(&key).is_none());
/// ```
pub struct Rasteriser {
    /// The system the faces come from.
    fonts: Arc<FontSystem>,
    /// Face bytes already looked up.
    faces: FaceBytes,
    /// The rasteriser's own caches and scratch buffers.
    scale: Mutex<ScaleContext>,
    /// The curves already extracted, for the runs the atlas cannot serve.
    outlines: Outlines,
}

impl Rasteriser {
    /// A rasteriser over one font system's faces.
    pub fn new(fonts: Arc<FontSystem>) -> Self {
        Self {
            fonts,
            faces: FaceBytes::default(),
            scale: Mutex::new(ScaleContext::new()),
            outlines: Outlines::default(),
        }
    }
}

impl Rasteriser {
    /// Rasterises one glyph through `context`.
    fn raster_with(&self, context: &mut ScaleContext, key: &GlyphKey) -> Option<GlyphImage> {
        let (blob, index) = self.faces.get(&self.fonts, key.face)?;
        let font = FontRef::from_index(blob.data(), index as usize)?;
        let coords = self.fonts.variation_coords(key.instance);
        let mut scaler = context
            .builder_with_id(font, identity(key.face, index, key.instance))
            .size(key.size())
            .hint(true)
            .normalized_coords(coords.as_deref().unwrap_or_default())
            .build();
        let bold = f32::from_bits(key.synthetic_bold_bits);
        let image = if bold != 0.0 {
            render_emboldened(key, bold, &mut scaler)?
        } else {
            renderer(key, sources_for(key.style)).render(&mut scaler, key.glyph)?
        };
        Some(convert(&image))
    }
}

impl GlyphRaster for Rasteriser {
    fn raster(&self, key: &GlyphKey) -> Option<GlyphImage> {
        let mut context = self
            .scale
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.raster_with(&mut context, key)
    }

    /// Rasterises the batch through a context of its own, so batches on other threads do not wait
    /// for the shared one.
    ///
    /// The context lives for the batch only. A batch sorted by face and size prepares each face's
    /// hinting state once.
    fn raster_many(&self, keys: &[GlyphKey]) -> Vec<Option<GlyphImage>> {
        let mut context = ScaleContext::new();
        keys.iter()
            .map(|key| self.raster_with(&mut context, key))
            .collect()
    }

    fn outline(&self, key: &OutlineKey) -> Option<GlyphOutline> {
        let (blob, index) = self.faces.get(&self.fonts, key.face)?;
        // Read through a second reader rather than the one the bitmap path uses: the curves are
        // wanted unhinted and unshifted, and the scaler that produces tiles is configured for
        // neither.
        let font = skrifa::FontRef::from_index(blob.data(), index).ok()?;
        let coords = self.fonts.variation_coords(key.instance);
        self.outlines
            .get(&font, key, coords.as_deref().unwrap_or_default())
    }
}

/// What the scaler caches one face's prepared state under.
///
/// It has to be *stable* across calls and *distinct* between faces, and it is neither by default:
/// a font reference built from bytes mints a fresh identity every time it is constructed, so a
/// scaler built per glyph shares nothing with the one before it and runs the face's `fpgm` and
/// `prep` control programs again for every single glyph. A face handle and the index within its
/// collection are exactly the pair that names a face, so they are what the identity is made of.
/// The variation instance joins them, so the state prepared for one instance is never reused for
/// another.
fn identity(face: FaceId, index: u32, instance: VariationInstance) -> [u64; 2] {
    [
        u64::from(face.0),
        u64::from(index) | (u64::from(instance.0) << 32),
    ]
}

/// The sources a request draws from.
fn sources_for(style: RasterStyle) -> &'static [Source] {
    match style {
        RasterStyle::Color => &COLOR_SOURCES,
        RasterStyle::Grayscale | RasterStyle::Subpixel => &MASK_SOURCES,
    }
}

/// The render configured for one key without swash's built-in outline emboldening.
fn renderer(key: &GlyphKey, sources: &'static [Source]) -> Render<'static> {
    let mut render = Render::new(sources);
    render.format(format_for(key.style));
    // Positioning is in fractions of a pixel and the key carries the phase, so the same glyph at
    // four phases is four cache entries rather than an unbounded set.
    render.offset(Vector::new(key.offset.to_pixels(), 0.0));
    let slant = f32::from_bits(key.synthetic_slant_bits);
    if slant != 0.0 {
        // A synthesised italic leans the glyph forward, which is a shear along x by the angle the
        // face would have been drawn at.
        render.transform(Some(Transform::skew(
            Angle::from_degrees(slant),
            Angle::ZERO,
        )));
    }
    render
}

/// The mask format matching a raster style.
fn format_for(style: RasterStyle) -> Format {
    match style {
        RasterStyle::Subpixel => Format::CustomSubpixel(STRIPE_OFFSETS),
        RasterStyle::Grayscale | RasterStyle::Color => Format::Alpha,
    }
}

/// Renders one glyph with synthetic bolding, bypassing swash 0.2.10's broken layer-wide winding.
///
/// `swash::scale::outline::LayerMut::embolden` computes winding once over the flattened
/// `self.points` slice without splitting contours. Cross-contour jumps from the end of contour `k`
/// to the start of contour `k + 1` inject bogus signed-area terms that can invert the winding sign
/// on multi-contour glyphs (notably CJK), eroding strokes instead of expanding them.
///
/// This path extracts the scaled outline, computes the true layer winding strictly by summing each
/// closed contour's individual signed area, emboldens each contour with that winding sign, applies
/// any synthetic slant, and rasterises with `zeno::Mask`. Color-layer glyphs and bitmap-only faces
/// retain their existing fallbacks.
fn render_emboldened(key: &GlyphKey, bold: f32, scaler: &mut Scaler<'_>) -> Option<Image> {
    if key.style == RasterStyle::Color
        && let Some(image) = renderer(key, &COLOR_ONLY_SOURCES).render(scaler, key.glyph)
    {
        return Some(image);
    }

    if scaler.has_outlines()
        && let Some(mut outline) = scaler.scale_outline(key.glyph)
    {
        embolden_outline(&mut outline, bold, bold);
        let slant = f32::from_bits(key.synthetic_slant_bits);
        if slant != 0.0 {
            outline.transform(&Transform::skew(Angle::from_degrees(slant), Angle::ZERO));
        }
        let format = format_for(key.style);
        let offset = Vector::new(key.offset.to_pixels(), 0.0);
        let mut image = Image::new();
        let placement = Mask::new(outline.path())
            .format(format)
            .origin(Origin::BottomLeft)
            .style(Style::default())
            .offset(offset)
            .render_offset(offset)
            .inspect(|fmt, w, h| {
                image.data.resize(fmt.buffer_size(w, h), 0);
            })
            .render_into(&mut image.data[..], None);
        image.placement = placement;
        image.content = if format == Format::Alpha {
            Content::Mask
        } else {
            Content::SubpixelMask
        };
        image.source = Source::Outline;
        return Some(image);
    }

    renderer(key, &BITMAP_ONLY_SOURCES).render(scaler, key.glyph)
}

/// Applies faux-bold to every layer of `outline` using per-contour signed-area winding.
fn embolden_outline(outline: &mut Outline, x_strength: f32, y_strength: f32) {
    for index in 0..outline.len() {
        if let Some(mut layer) = outline.get_mut(index) {
            let verbs = layer.verbs();
            let points = layer.points_mut();
            embolden_layer(points, verbs, x_strength, y_strength);
        }
    }
}

/// Emboldens one layer's contours in place using the true summed contour winding.
fn embolden_layer(points: &mut [Point], verbs: &[Verb], x_strength: f32, y_strength: f32) {
    let contours = contour_ranges(verbs, points.len());
    if contours.is_empty() {
        return;
    }
    let winding = layer_winding(points, &contours);
    for range in contours {
        if let Some(contour) = points.get_mut(range) {
            embolden_contour(contour, winding, x_strength, y_strength);
        }
    }
}

/// Splits a layer's point slice into its individual contour point ranges from its verb stream.
fn contour_ranges(verbs: &[Verb], point_count: usize) -> SmallVec<[Range<usize>; 8]> {
    let mut ranges = SmallVec::new();
    let mut start = 0usize;
    let mut pos = 0usize;
    for verb in verbs {
        match verb {
            Verb::MoveTo | Verb::Close => {
                if pos > point_count {
                    return ranges;
                }
                if pos > start {
                    ranges.push(start..pos);
                }
                start = pos;
                if *verb == Verb::MoveTo {
                    pos += 1;
                }
            }
            Verb::LineTo => pos += 1,
            Verb::QuadTo => pos += 2,
            Verb::CurveTo => pos += 3,
        }
    }
    if pos > start && pos <= point_count {
        ranges.push(start..pos);
    }
    ranges
}

/// Computes the outer winding orientation (`1` for positive signed area, `0` otherwise) strictly by
/// summing each closed contour's individual signed area without cross-contour jumps.
fn layer_winding(points: &[Point], contours: &[Range<usize>]) -> u8 {
    let mut total_area = 0.0_f64;
    for range in contours {
        let Some(contour) = points.get(range.clone()) else {
            continue;
        };
        if contour.len() < 2 {
            continue;
        }
        let mut prev = contour[contour.len() - 1];
        for &cur in contour {
            total_area += (f64::from(cur.y) - f64::from(prev.y))
                * (f64::from(cur.x) + f64::from(prev.x));
            prev = cur;
        }
    }
    u8::from(total_area > 0.0)
}

/// Expands one closed contour along its outward normals using the layer's true winding sign.
///
/// Mirrors FreeType's `FT_Outline_Embolden` / `swash`'s corner-miter algorithm so outer contours
/// dilate outward and opposite-wound hole contours contract inward, thickening every stroke.
fn embolden_contour(points: &mut [Point], winding: u8, x_strength: f32, y_strength: f32) {
    if points.is_empty() {
        return;
    }
    let last = points.len() - 1;
    let mut i = last;
    let mut j = 0;
    let mut k = usize::MAX;
    let mut out_len;
    let mut in_len = 0.0_f32;
    let mut anchor_len = 0.0_f32;
    let mut anchor = Point::ZERO;
    let mut out;
    let mut in_ = Point::ZERO;
    while j != i && i != k {
        if j != k {
            out = points[j] - points[i];
            out_len = out.length();
            if out_len == 0.0 {
                j = if j < last { j + 1 } else { 0 };
                continue;
            }
            let s = 1.0 / out_len;
            out.x *= s;
            out.y *= s;
        } else {
            out = anchor;
            out_len = anchor_len;
        }
        if in_len != 0.0 {
            if k == usize::MAX {
                k = i;
                anchor = in_;
                anchor_len = in_len;
            }
            let mut d = (in_.x * out.x) + (in_.y * out.y);
            let shift = if d > -0.9396 {
                d += 1.0;
                let mut sx = in_.y + out.y;
                let mut sy = in_.x + out.x;
                if winding == 0 {
                    sx = -sx;
                } else {
                    sy = -sy;
                }
                let mut q = (out.x * in_.y) - (out.y * in_.x);
                if winding == 0 {
                    q = -q;
                }
                let l = in_len.min(out_len);
                if x_strength * q <= l * d {
                    sx = sx * x_strength / d;
                } else {
                    sx = sx * l / q;
                }
                if y_strength * q <= l * d {
                    sy = sy * y_strength / d;
                } else {
                    sy = sy * l / q;
                }
                Point::new(sx, sy)
            } else {
                Point::ZERO
            };

            while i != j {
                points[i].x += x_strength + shift.x;
                points[i].y += y_strength + shift.y;
                i = if i < last { i + 1 } else { 0 };
            }
        } else {
            i = j;
        }
        in_ = out;
        in_len = out_len;
        j = if j < last { j + 1 } else { 0 };
    }
}

#[cfg(test)]
mod tests {
    use swash::zeno::{Format, Mask, Origin, Point, Style, Verb};

    use super::{contour_ranges, embolden_layer, layer_winding};

    /// Replicates swash 0.2.10's defective flattened `compute_winding(self.points)`.
    fn buggy_flattened_winding(points: &[Point]) -> u8 {
        if points.is_empty() {
            return 0;
        }
        let mut area = 0.0_f32;
        let mut prev = points[points.len() - 1];
        for &cur in points {
            area += (cur.y - prev.y) * (cur.x + prev.x);
            prev = cur;
        }
        u8::from(area > 0.0)
    }

    fn raster_coverage(points: &[Point], verbs: &[Verb]) -> u64 {
        let mut data = Vec::new();
        Mask::new((points, verbs))
            .format(Format::Alpha)
            .origin(Origin::BottomLeft)
            .style(Style::default())
            .inspect(|fmt, w, h| data.resize(fmt.buffer_size(w, h), 0))
            .render_into(&mut data[..], None);
        data.into_iter().map(u64::from).sum()
    }

    fn push_cw_rect(points: &mut Vec<Point>, verbs: &mut Vec<Verb>, x0: f32, y0: f32, x1: f32, y1: f32) {
        points.push(Point::new(x0, y0));
        verbs.push(Verb::MoveTo);
        points.push(Point::new(x0, y1));
        verbs.push(Verb::LineTo);
        points.push(Point::new(x1, y1));
        verbs.push(Verb::LineTo);
        points.push(Point::new(x1, y0));
        verbs.push(Verb::LineTo);
        verbs.push(Verb::Close);
    }

    fn push_ccw_rect(points: &mut Vec<Point>, verbs: &mut Vec<Verb>, x0: f32, y0: f32, x1: f32, y1: f32) {
        points.push(Point::new(x0, y0));
        verbs.push(Verb::MoveTo);
        points.push(Point::new(x1, y0));
        verbs.push(Verb::LineTo);
        points.push(Point::new(x1, y1));
        verbs.push(Verb::LineTo);
        points.push(Point::new(x0, y1));
        verbs.push(Verb::LineTo);
        verbs.push(Verb::Close);
    }

    /// Reproduces the multi-contour CJK winding-inversion bug in swash 0.2.10:
    /// two CW strokes positioned so the cross-contour jump produces a large positive signed area,
    /// flipping swash's flattened winding from 0 (CW) to 1 (CCW). Per-contour winding must detect
    /// CW (`0`) and strictly increase rasterised pixel coverage instead of eroding strokes.
    #[test]
    fn multi_contour_embolden_never_erodes_when_flattened_winding_flips() {
        let mut points = Vec::new();
        let mut verbs = Vec::new();
        // Three CW strokes placed in a CCW triangle (bottom-left -> bottom-right -> top-right):
        // each stroke has negative CW signed area (-48, total -144), while the bogus cross-contour
        // closing edges trace a large positive CCW loop (+572) that flips swash's flattened
        // winding from 0 (CW) to 1 (CCW).
        push_cw_rect(&mut points, &mut verbs, 2.0, 2.0, 6.0, 8.0);
        push_cw_rect(&mut points, &mut verbs, 28.0, 2.0, 32.0, 8.0);
        push_cw_rect(&mut points, &mut verbs, 28.0, 24.0, 32.0, 30.0);

        let contours = contour_ranges(&verbs, points.len());
        assert_eq!(contours.len(), 3);
        // Confirm the fixture genuinely triggers swash 0.2.10's winding inversion bug.
        assert_eq!(
            buggy_flattened_winding(&points),
            1,
            "fixture must trigger swash flattened winding inversion"
        );
        // Our per-contour winding ignores cross-contour jumps and reports true CW winding (0).
        assert_eq!(layer_winding(&points, &contours), 0);

        let base_coverage = raster_coverage(&points, &verbs);
        let mut emboldened = points.clone();
        embolden_layer(&mut emboldened, &verbs, 0.8, 0.8);
        let bold_coverage = raster_coverage(&emboldened, &verbs);

        assert!(
            bold_coverage > base_coverage,
            "expected emboldened coverage ({bold_coverage}) > base ({base_coverage})"
        );
    }

    /// Verifies an outer contour with an opposite-wound inner hole thickens every wall instead of
    /// expanding the hole.
    #[test]
    fn embolden_thickens_outer_contour_with_inner_hole() {
        let mut points = Vec::new();
        let mut verbs = Vec::new();
        // TrueType-style CW outer square with CCW inner hole.
        push_cw_rect(&mut points, &mut verbs, 4.0, 4.0, 28.0, 28.0);
        push_ccw_rect(&mut points, &mut verbs, 10.0, 10.0, 22.0, 22.0);

        let contours = contour_ranges(&verbs, points.len());
        assert_eq!(contours.len(), 2);
        assert_eq!(layer_winding(&points, &contours), 0);

        let base_coverage = raster_coverage(&points, &verbs);
        let mut emboldened = points.clone();
        embolden_layer(&mut emboldened, &verbs, 1.0, 1.0);
        let bold_coverage = raster_coverage(&emboldened, &verbs);

        assert!(
            bold_coverage > base_coverage,
            "expected emboldened hollow square coverage ({bold_coverage}) > base ({base_coverage})"
        );
    }

    /// Verifies end-to-end `Rasteriser` synthetic bolding on multi-contour CJK glyphs (`能`, `清`,
    /// `单`) using the repository's shipped Regular CJK face: every glyph's pixel alpha coverage
    /// strictly increases and never erodes.
    #[test]
    fn cjk_multi_contour_glyphs_strictly_thicken_under_rasteriser() {
        use std::sync::Arc;

        use zgui_text::{FontSource, GlyphKey, GlyphRaster, RasterStyle, SubpixelOffset};

        use crate::{FontSystem, FontSystemOptions, Rasteriser};

        let fonts = Arc::new(FontSystem::new(FontSystemOptions::registered_only()));
        let path = format!(
            "{}/../../preview/fonts/NotoSansCJKsc-Regular.otf",
            env!("CARGO_MANIFEST_DIR")
        );
        let raw = std::fs::read(&path).expect("shipped CJK Regular face");
        let face_ref = swash::FontRef::from_index(&raw, 0).expect("parse CJK face");
        let gids: Vec<(char, u16)> = ['能', '力', '展', '示', '清', '单']
            .into_iter()
            .map(|ch| {
                let gid = face_ref.charmap().map(ch);
                assert_ne!(gid, 0, "glyph for {ch} must exist");
                (ch, gid)
            })
            .collect();

        let bytes: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(raw);
        let faces = fonts.register(bytes, None).expect("register CJK face");
        let face = faces[0];
        let raster = Rasteriser::new(fonts);

        for (ch, gid) in gids {
            let base_key = GlyphKey::new(face, gid, 24.0, SubpixelOffset(0), RasterStyle::Grayscale);
            let mut bold_key = base_key;
            bold_key.synthetic_bold_bits = (24.0_f32 * zgui_text::SYNTHETIC_BOLD_RATIO).to_bits();

            let base_img = raster.raster(&base_key).expect("base glyph raster");
            let bold_img = raster.raster(&bold_key).expect("bold glyph raster");
            let base_cov: u64 = base_img.bytes.iter().copied().map(u64::from).sum();
            let bold_cov: u64 = bold_img.bytes.iter().copied().map(u64::from).sum();
            assert!(
                bold_cov > base_cov,
                "CJK glyph {ch} (gid={gid}) eroded: bold_cov={bold_cov} <= base_cov={base_cov}"
            );
        }
    }
}
