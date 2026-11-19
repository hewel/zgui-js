use std::{io::Cursor, time::{Duration, Instant}};

use serde_json::{Value, json};
use zgui::prelude::*;

use super::{Error, pipeline::{self, StaticFrame}, profile::LoadedProfile};

pub struct Calibration {
    pub png: Vec<u8>,
    pub metadata: Value,
}

/// Exercises the native pipeline, then asserts its presented bytes before encoding them.
/// Native GPU calls are synchronous upstream; the doctor's supervising process must enforce
/// the hard wall deadline as well as the cooperative pump limits used here.
pub fn calibrate(profile: &LoadedProfile) -> Result<Calibration, Error> {
    let started = Instant::now();
    let run_budget = Duration::from_millis(profile.profile.limits.max_run_wall_ms);
    let step_budget = Duration::from_millis(profile.profile.limits.max_step_wall_ms);
    let deadline = started + run_budget;
    let surface = &profile.profile.surface;
    let opaque = surface.opaque;
    let background = if opaque {
        None
    } else {
        let rgba = surface.background.strip_prefix('#')
            .filter(|hex| hex.len() == 8 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .ok_or_else(|| calibration_error("surface.background must be a #RRGGBBAA hexadecimal color"))?;
        Some(rgba.to_be_bytes())
    };
    pipeline::render_static(
        profile,
        "zgui preview calibration",
        stylesheet(profile),
        runtime_error,
        move || calibration_view(opaque),
        |session| {
            session.check_runtime()?;
            if session.window_count() == 0 {
                return Err(calibration_error("the native runtime created no calibration window"));
            }
            session.configure(profile.physical_width, profile.physical_height, surface.scale);
            let frame = session.await_frame(deadline.min(Instant::now() + step_budget))?;
            let mut checks = verify(&frame, profile.physical_width, profile.physical_height, opaque)?;
            let png = session.encode_png(&frame, opaque)?;
            if let Some(background) = background {
                checks["transparent"] = verify_transparency(&frame, background, Some(&png))?;
            }
            let original_metadata = session.metadata();

            // Widths not divisible by the GPU's copy alignment must be tested on the real readback,
            // not merely by constructing tightly-packed CPU bytes. Keep the requested PNG unchanged.
            let odd_width = if profile.physical_width % 2 == 1 {
                profile.physical_width
            } else {
                profile.physical_width - 1
            };
            let odd_checks = if odd_width == profile.physical_width {
                checks.clone()
            } else {
                session.configure(odd_width, profile.physical_height, surface.scale);
                let odd = session.await_frame(deadline.min(Instant::now() + step_budget))?;
                let mut checks = verify(&odd, odd_width, profile.physical_height, opaque)?;
                if let Some(background) = background {
                    checks["transparent"] = verify_transparency(&odd, background, None)?;
                }
                checks
            };
            if Instant::now() >= deadline {
                return Err(timeout_error());
            }
            Ok(Calibration {
                png,
                metadata: json!({
                    "renderer": original_metadata,
                    "physicalWidth": frame.pixels().size().width,
                    "physicalHeight": frame.pixels().size().height,
                    "logicalWidth": surface.width,
                    "logicalHeight": surface.height,
                    "scale": surface.scale,
                    "opaque": surface.opaque,
                    "background": surface.background,
                    "drawId": frame.draw_id(),
                    "presentationId": frame.presentation_id(),
                    "configurationId": frame.configuration_id(),
                    "checks": checks,
                    "oddWidth": odd_width,
                    "oddWidthChecks": odd_checks,
                    "nativeClock": "explicit-harness-virtual",
                    "sharedJsVirtualTimeValidated": false,
                    "compositorTransparencyValidated": false,
                }),
            })
        },
    )
}

fn calibration_view(opaque: bool) -> impl IntoView {
    let fixture = view! {
        box(class = "calibration") {
            box(class = "swatch red")
            box(class = "swatch green")
            box(class = "swatch blue")
            box(class = "swatch white")
            box(class = "swatch gray")
            box(class = "swatch half-base") {
                box(class = "half")
            }
            box(class = "text-panel") {
                text(class = "calibration-text") {"Zgui Aa ffi 0123456789"}
            }
        }
    };
    if opaque {
        fixture
    } else {
        fixture.child(view! { box(class = "swatch half-red") })
    }
}

fn stylesheet(profile: &LoadedProfile) -> String {
    // Both the background and family CSS have already been validated/escaped by the loader.
    let font_size = (profile.profile.surface.height as f32 / 12.0).clamp(8.0, 28.0);
    let mut css = format!(r#"
        :root {{ display: block; width: 100%; height: 100%; background-color: {}; font-family: {}; }}
        .calibration {{ display: block; position: relative; width: 100%; height: 100%; overflow: hidden; }}
        .swatch {{ display: block; position: absolute; width: 25%; height: 25%; }}
        .red {{ left: 0; top: 0; background-color: #ff0000; }}
        .green {{ right: 0; top: 0; background-color: #00ff00; }}
        .blue {{ left: 0; bottom: 0; background-color: #0000ff; }}
        .white {{ right: 0; bottom: 0; background-color: #ffffff; }}
        .gray {{ left: 25%; top: 0; background-color: #808080; }}
        .half-base {{ left: 50%; top: 0; background-color: #ffffff; }}
        .half {{ display: block; width: 100%; height: 100%; background-color: rgba(128, 128, 128, 0.5); }}
        .text-panel {{ display: block; position: absolute; left: 0; top: 25%; width: 100%; height: 50%; background-color: #ffffff; overflow: hidden; }}
        .calibration-text {{ display: block; color: #000000; font-size: {}px; line-height: 1.25; }}
    "#, profile.profile.surface.background, profile.font_family_css, font_size);
    if !profile.profile.surface.opaque {
        css.push_str(".half-red { left: 50%; bottom: 0; background-color: rgba(255, 0, 0, 0.5); }");
    }
    css
}

fn verify(frame: &StaticFrame, width: u32, height: u32, opaque: bool) -> Result<Value, Error> {
    let pixels = frame.pixels();
    let size = pixels.size();
    if size.width != width as i32 || size.height != height as i32 || frame.target_size() != size {
        return Err(calibration_error("presented extent does not match the configured native target"));
    }
    if pixels.bytes().len() != width as usize * height as usize * 4 {
        return Err(calibration_error("readback contains padding or truncated rows"));
    }
    if opaque && pixels.bytes().as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 255) {
        return Err(calibration_error("opaque calibration contains alpha other than 255"));
    }
    let samples = [
        ("topLeftRed", 1, 1, [255, 0, 0, 255]),
        ("topRightGreen", 7, 1, [0, 255, 0, 255]),
        ("bottomLeftBlue", 1, 7, [0, 0, 255, 255]),
        ("bottomRightWhite", 7, 7, [255, 255, 255, 255]),
        ("gray", 3, 1, [128, 128, 128, 255]),
        ("halfGrayOverWhite", 5, 1, [191, 191, 191, 255]),
    ];
    let mut measured = serde_json::Map::new();
    for (name, x, y, expected) in samples {
        let x = ((u64::from(width) * x) / 8).min(u64::from(width - 1)) as i32;
        let y = ((u64::from(height) * y) / 8).min(u64::from(height - 1)) as i32;
        let actual = pixels.rgba(x, y);
        // A single quantization unit is allowed for half-alpha blending, never a transfer curve.
        if actual.iter().zip(expected).any(|(&actual, expected)| actual.abs_diff(expected) > 1) {
            return Err(calibration_error(format!("{name} at ({x},{y}): expected {expected:?}, got {actual:?}")));
        }
        measured.insert(name.into(), json!({"x": x, "y": y, "rgba": actual}));
    }
    // The fixture's center contains only white and real shaped/rasterized black text. Missing
    // glyph rasterization cannot pass this check merely because layout produced text geometry.
    let row_bytes = width as usize * 4;
    let ink = pixels.bytes().chunks_exact(row_bytes)
        .skip((height / 4 + 1) as usize)
        .take((height * 3 / 4).saturating_sub(height / 4 + 1) as usize)
        .flat_map(|row| row.as_chunks::<4>().0)
        .filter(|pixel| pixel[0] < 240 && pixel[1] < 240 && pixel[2] < 240)
        .count();
    if ink == 0 { return Err(calibration_error("no rasterized calibration text was present")); }
    Ok(json!({
        "extent": true,
        "packedByteLength": pixels.bytes().len(),
        "topDownOrientation": true,
        "channelOrder": if pixels.is_bgra() { "BGRA" } else { "RGBA" },
        "samples": measured,
        "textInkPixels": ink,
        "opaqueAlpha255": if opaque { Some(true) } else { None },
    }))
}

struct TransparentSample {
    name: &'static str,
    premultiplied: [u8; 4],
    rgb_tolerance: u8,
    alpha_tolerance: u8,
}

fn transparent_swatches(background: [u8; 4]) -> [TransparentSample; 2] {
    let [r, g, b, a] = background;
    let alpha = u32::from(a);
    let premultiplied = [r, g, b].map(|channel| u32::from(channel) * alpha);
    let [r, g, b] = premultiplied.map(|channel| pipeline::round_ratio_to_byte(channel, 255));
    let [over_r, over_g, over_b] = std::array::from_fn(|i| {
        // Source-over with half-alpha red, before quantizing the root's channels.
        pipeline::round_ratio_to_byte(premultiplied[i] + [255 * 255, 0, 0][i], 2 * 255)
    });
    [
        TransparentSample {
            name: "root",
            premultiplied: [r, g, b, a],
            rgb_tolerance: u8::from(!matches!(a, 0 | 255)),
            alpha_tolerance: 0,
        },
        TransparentSample {
            name: "halfRedOverRoot",
            premultiplied: [over_r, over_g, over_b, pipeline::round_ratio_to_byte(255 + alpha, 2)],
            rgb_tolerance: 1,
            alpha_tolerance: 1,
        },
    ]
}

impl TransparentSample {
    fn matches_premultiplied(&self, actual: [u8; 4]) -> bool {
        actual[3].abs_diff(self.premultiplied[3]) <= self.alpha_tolerance
            && (0..3).all(|i| actual[i].abs_diff(self.premultiplied[i]) <= self.rgb_tolerance)
    }

    fn matches_straight(&self, actual: [u8; 4]) -> bool {
        let alpha = actual[3];
        alpha.abs_diff(self.premultiplied[3]) <= self.alpha_tolerance
            && (0..3).all(|i| {
                // One premultiplied byte can span many straight RGB values at low alpha.
                let channel = self.premultiplied[i];
                let min = channel.saturating_sub(self.rgb_tolerance);
                let max = channel.saturating_add(self.rgb_tolerance);
                (min..=max).any(|candidate| pipeline::straight_channel(candidate, alpha) == actual[i])
            })
    }
}

fn verify_transparency(frame: &StaticFrame, background: [u8; 4], png: Option<&[u8]>) -> Result<Value, Error> {
    let pixels = frame.pixels();
    let size = pixels.size();
    // Both samples are on the same row, below the text panel and away from swatch edges.
    let y = size.height * 7 / 8;
    let xs = [size.width * 3 / 8, size.width * 5 / 8];
    let raw = xs.map(|x| pixels.rgba(x, y));
    let samples = transparent_swatches(background);
    let mut checks = serde_json::Map::new();
    for (i, sample) in samples.iter().enumerate() {
        let x = xs[i];
        let actual = raw[i];
        let name = sample.name;
        if !sample.matches_premultiplied(actual) {
            return Err(calibration_error(format!("{name} at ({x},{y}): expected premultiplied {:?}, got {actual:?}", sample.premultiplied)));
        }
        checks.insert(name.into(), json!({"x": x, "y": y, "premultipliedRgba": actual}));
    }
    if let Some(png) = png {
        let decoded = decode_alpha_samples(png, (size.width, size.height), xs, y)?;
        for (i, sample) in samples.iter().enumerate() {
            let name = sample.name;
            if !sample.matches_straight(decoded[i]) {
                return Err(calibration_error(format!("{name} PNG: expected straight alpha for premultiplied {:?}, got {:?}", sample.premultiplied, decoded[i])));
            }
            checks[name]["straightRgba"] = json!(decoded[i]);
        }
    }
    Ok(Value::Object(checks))
}

fn decode_alpha_samples(
    png: &[u8], (width, height): (i32, i32), xs: [i32; 2], y: i32,
) -> Result<[[u8; 4]; 2], Error> {
    let png_error = |error: png::DecodingError| calibration_error(format!("calibration PNG: {error}"));
    let mut reader = png::Decoder::new(Cursor::new(png)).read_info().map_err(png_error)?;
    let info = reader.info();
    if i32::try_from(info.width) != Ok(width) || i32::try_from(info.height) != Ok(height)
        || info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight || info.interlaced
    {
        return Err(calibration_error("expected a packed RGBA8 calibration PNG at the readback extent"));
    }
    let missing_row = || calibration_error("calibration PNG ended before the alpha sample row");
    for _ in 0..y {
        reader.next_row().map_err(png_error)?.ok_or_else(missing_row)?;
    }
    let row = reader.next_row().map_err(png_error)?.ok_or_else(missing_row)?;
    let rgba = row.data().as_chunks::<4>().0;
    let [left, right] = xs.map(|x| usize::try_from(x).ok().and_then(|x| rgba.get(x)).copied());
    let samples = left.zip(right).ok_or_else(|| calibration_error("calibration PNG alpha sample fell outside its row"))?;
    reader.finish().map_err(png_error)?;
    Ok([samples.0, samples.1])
}

fn runtime_error(error: zgui::runtime::AppError) -> Error {
    match error {
        zgui::runtime::AppError::GpuUnavailable(_) => Error::new("E_GRAPHICS_UNAVAILABLE", 7, error.to_string()),
        _ => calibration_error(error.to_string()),
    }
}

fn calibration_error(message: impl Into<String>) -> Error { Error::new("E_CALIBRATION", 7, message) }
fn timeout_error() -> Error { Error::new("E_READINESS_TIMEOUT", 5, "calibration exceeded its real wall-clock budget") }

#[cfg(test)]
mod tests {
    use super::transparent_swatches;

    #[test]
    fn transparent_swatches_use_premultiplied_source_over_and_keep_alpha() {
        let [root, half_red] = transparent_swatches([0, 0, 0, 0]);
        assert_eq!(root.premultiplied, [0; 4]);
        assert_eq!(half_red.premultiplied, [128, 0, 0, 128]);
        let [root, half_red] = transparent_swatches([17, 34, 51, 128]); // #11223380
        assert_eq!(root.premultiplied, [9, 17, 26, 128]);
        assert_eq!(half_red.premultiplied, [132, 9, 13, 192]);
        assert!(half_red.matches_straight([175, 12, 17, 192]));
        assert!(!half_red.matches_straight([175, 20, 17, 192]));
    }

    #[test]
    fn calibration_root_does_not_tolerate_losing_one_alpha_byte() {
        let [root, _] = transparent_swatches([0, 0, 0, 1]); // #00000001
        assert!(!root.matches_premultiplied([0, 0, 0, 0]));
        assert!(!root.matches_straight([0, 0, 0, 0]));
        assert!(root.matches_premultiplied([0, 0, 0, 1]));
        assert!(root.matches_straight([0, 0, 0, 1]));
    }
}
