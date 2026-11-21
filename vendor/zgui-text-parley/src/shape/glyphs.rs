//! Reading positioned glyphs out of a broken layout.
//!
//! # Why the positions are shifted
//!
//! The engine positions every glyph relative to the *paragraph's* origin, and folds the line's
//! alignment offset and its inline minimum coordinate into that position. A line box, on the other
//! hand, starts exactly where those two put it. Subtracting them therefore moves the glyphs into
//! the line box's own space, which is the space everything downstream places a line in — and it is
//! the only conversion here, because the engine is already working in device pixels.

use zgui_text::{Brush, FontSource, SYNTHETIC_BOLD_RATIO, ShapedGlyph, ShapedRun};

use crate::shape::brush::SlotBrush;
use crate::shape::engine::{Form, ShapedLayout};
use crate::system::FontSystem;

/// Visits the style-uniform runs of one line, positioned against the line box's top-left corner.
pub(crate) fn visit_line(
    fonts: &FontSystem,
    shaped: &ShapedLayout,
    line: u16,
    visit: &mut dyn FnMut(ShapedRun<'_>),
) {
    let layout = match &shaped.form {
        Form::Live(live) => &live.layout,
        Form::Frozen(frozen) => return frozen.visit_line(fonts, line, visit),
    };
    let Some(line) = layout.get(line as usize) else {
        return;
    };
    let metrics = *line.metrics();
    let left = metrics.offset + metrics.inline_min_coord;
    let top = crate::shape::lines::top(&metrics);

    // One buffer for the whole line rather than one per run: a line of prose is a handful of runs
    // and every one of them would otherwise allocate.
    let mut glyphs: Vec<ShapedGlyph> = Vec::new();
    for item in line.items() {
        let parley::PositionedLayoutItem::GlyphRun(run) = item else {
            continue;
        };
        glyphs.clear();
        glyphs.extend(run.positioned_glyphs().map(|glyph| ShapedGlyph {
            glyph: glyph.id as u16,
            x: glyph.x - left,
            y: glyph.y - top,
        }));
        if glyphs.is_empty() {
            continue;
        }
        visit(described(fonts, &run, &glyphs, shaped.prefix));
    }
}

/// Minimum requested CSS font weight eligible for synthetic bolding.
///
/// Matches CSS Fonts 4 / browser conventions (Blink, Gecko): requesting `500` (Medium) against a
/// `400` Regular face does not trigger heavy faux-bold; only `>= 600` against a face `< 600` does.
const SYNTHETIC_BOLD_THRESHOLD: f32 = 600.0;

/// Resolves the synthetic bold ratio for one shaped run.
///
/// Only applies faux-bold when the shaper requested synthesis, the author asked for at least
/// SemiBold (`>= 600`), and the matched face itself is lighter than `600`.
pub(crate) fn synthetic_bold(embolden: bool, requested_weight: f32, face_weight: f32) -> f32 {
    if embolden
        && requested_weight >= SYNTHETIC_BOLD_THRESHOLD
        && face_weight < SYNTHETIC_BOLD_THRESHOLD
    {
        SYNTHETIC_BOLD_RATIO
    } else {
        0.0
    }
}

/// One engine run, described the way a rasteriser needs it.
///
/// `prefix` is how many bytes of directional control lead the engine's string; the run's text
/// offset is reported in the caller's string, which does not hold them.
fn described<'a>(
    fonts: &FontSystem,
    run: &parley::GlyphRun<'_, SlotBrush>,
    glyphs: &'a [ShapedGlyph],
    prefix: usize,
) -> ShapedRun<'a> {
    let text = run.run();
    let text_start = text.text_range().start.saturating_sub(prefix);
    let synthesis = text.synthesis();
    run_of(
        fonts,
        text.font(),
        text.normalized_coords(),
        text.font_size(),
        synthesis.embolden(),
        text.font_attrs().weight.value(),
        synthesis.skew().unwrap_or(0.0),
        run.style().brush.0,
        glyphs,
        u32::try_from(text_start).unwrap_or(u32::MAX),
    )
}

/// One run of glyphs in one face, described the way a rasteriser needs it.
///
/// `embolden` and `skew` are the engine's synthesis. `text_start` is in the caller's string.
#[expect(
    clippy::too_many_arguments,
    reason = "the fields of one engine run, read once"
)]
pub(crate) fn run_of<'a>(
    fonts: &FontSystem,
    font: &parley::FontData,
    coords: &[i16],
    size: f32,
    embolden: bool,
    requested_weight: f32,
    skew: f32,
    brush: Brush,
    glyphs: &'a [ShapedGlyph],
    text_start: u32,
) -> ShapedRun<'a> {
    let face = fonts.face_for(font);
    let record = fonts.face(face);
    let face_weight = record.as_ref().map_or(400.0, |record| record.weight);
    ShapedRun {
        face,
        size,
        instance: fonts.variation_instance(coords),
        // The engine reports *whether* to embolden; apply faux-bold only for SemiBold+ requests
        // matched against lighter faces, matching CSS Fonts 4 browser conventions.
        synthetic_bold: synthetic_bold(embolden, requested_weight, face_weight),
        synthetic_slant: skew,
        has_color: record.is_some_and(|record| record.has_color),
        brush,
        glyphs,
        text_start,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use zgui_geom::CssPx;
    use zgui_interned::Ident;
    use zgui_scene::PaintSlot;
    use zgui_text::{
        FontSource, ParagraphContent, ParagraphShaper, SYNTHETIC_BOLD_RATIO, ShapedRun, StyledRun,
        TextMap,
    };
    use zgui_text_style::{
        FamilyName, FontFamilyList, ParagraphStyle, TextStyle, WrapMode,
    };

    use super::synthetic_bold;
    use crate::{FontSystem, FontSystemOptions, LineRequest, Shaper};

    fn regular_only_fonts() -> Arc<FontSystem> {
        let fonts = Arc::new(FontSystem::new(FontSystemOptions::registered_only()));
        let path = format!(
            "{}/../../preview/fonts/NotoSans-Regular.ttf",
            env!("CARGO_MANIFEST_DIR")
        );
        let bytes: Arc<dyn AsRef<[u8]> + Send + Sync> =
            Arc::new(std::fs::read(&path).expect("shipped Regular test face"));
        fonts.register(bytes, None).expect("register Regular face");
        fonts
    }

    #[test]
    fn synthetic_bold_threshold_matches_css_conventions() {
        assert_eq!(synthetic_bold(true, 400.0, 400.0), 0.0);
        assert_eq!(synthetic_bold(true, 500.0, 400.0), 0.0);
        assert_eq!(synthetic_bold(true, 599.0, 400.0), 0.0);
        assert_eq!(synthetic_bold(true, 600.0, 400.0), SYNTHETIC_BOLD_RATIO);
        assert_eq!(synthetic_bold(true, 700.0, 400.0), SYNTHETIC_BOLD_RATIO);
        assert_eq!(synthetic_bold(true, 700.0, 600.0), 0.0);
        assert_eq!(synthetic_bold(false, 700.0, 400.0), 0.0);
    }

    #[test]
    fn shaping_paths_suppress_faux_bold_at_500_and_enable_at_700() {
        let fonts = regular_only_fonts();
        let mut shaper = Shaper::new(Arc::clone(&fonts));
        let family = Ident::new("Noto Sans");
        let text = "Weight";

        for (weight, expected) in [
            (400_u16, 0.0_f32),
            (500_u16, 0.0_f32),
            (600_u16, SYNTHETIC_BOLD_RATIO),
            (700_u16, SYNTHETIC_BOLD_RATIO),
        ] {
            // 1. Live paragraph path (Wrap)
            let mut map = TextMap::new();
            map.push(0..text.len(), 0, 0);
            let live_style = Arc::new(TextStyle {
                family: FontFamilyList::from_iter([FamilyName::Named(family)]),
                size: CssPx(16.0),
                weight: f32::from(weight),
                wrap_mode: WrapMode::Wrap,
                ..TextStyle::initial()
            });
            let live_runs = [StyledRun {
                text: 0..text.len(),
                style: live_style,
                brush: PaintSlot(0),
            }];
            let paragraph_style = ParagraphStyle::default();
            let live_content = ParagraphContent {
                text,
                map: &map,
                runs: &live_runs,
                boxes: &[],
                paragraph: &paragraph_style,
                scale: 1.0,
            };
            let live = shaper.shape(&live_content);
            assert!(!live.engine.is_frozen());
            let mut live_bolds = Vec::new();
            shaper.visit_line(&live, 0, &mut |run: ShapedRun<'_>| {
                live_bolds.push(run.synthetic_bold);
            });
            assert!(!live_bolds.is_empty());
            assert!(
                live_bolds.iter().all(|&b| b == expected),
                "live weight={weight} got {live_bolds:?}, expected {expected}"
            );

            // 2. Frozen paragraph path (NoWrap)
            let frozen_style = Arc::new(TextStyle {
                family: FontFamilyList::from_iter([FamilyName::Named(family)]),
                size: CssPx(16.0),
                weight: f32::from(weight),
                wrap_mode: WrapMode::NoWrap,
                ..TextStyle::initial()
            });
            let frozen_runs = [StyledRun {
                text: 0..text.len(),
                style: frozen_style,
                brush: PaintSlot(0),
            }];
            let frozen_content = ParagraphContent {
                text,
                map: &map,
                runs: &frozen_runs,
                boxes: &[],
                paragraph: &paragraph_style,
                scale: 1.0,
            };
            let frozen = shaper.shape(&frozen_content);
            assert!(frozen.engine.is_frozen());
            let mut frozen_bolds = Vec::new();
            shaper.visit_line(&frozen, 0, &mut |run: ShapedRun<'_>| {
                frozen_bolds.push(run.synthetic_bold);
            });
            assert!(!frozen_bolds.is_empty());
            assert!(
                frozen_bolds.iter().all(|&b| b == expected),
                "frozen weight={weight} got {frozen_bolds:?}, expected {expected}"
            );

            // 3. Single-line shaping path
            let families = [family];
            let line_runs = shaper.shape_line(
                text,
                &LineRequest {
                    families: &families,
                    weight,
                    italic: false,
                    size_device_px: 16.0,
                    letter_spacing: 0.0,
                    ligatures: true,
                },
            );
            assert!(!line_runs.is_empty());
            assert!(
                line_runs.iter().all(|run| run.synthetic_bold == expected),
                "line weight={weight} got {:?}, expected {expected}",
                line_runs.iter().map(|r| r.synthetic_bold).collect::<Vec<_>>()
            );
        }
    }
}
