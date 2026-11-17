use std::time::{Duration, Instant};

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
    pipeline::render_static(
        profile,
        "zgui preview calibration",
        stylesheet(profile),
        runtime_error,
        calibration_view,
        |session| {
            session.check_runtime()?;
            if session.window_count() == 0 {
                return Err(calibration_error("the native runtime created no calibration window"));
            }
            session.configure(profile.physical_width, profile.physical_height, surface.scale);
            let frame = session.await_frame(deadline.min(Instant::now() + step_budget))?;
            let checks = verify(&frame, profile.physical_width, profile.physical_height, surface.opaque)?;
            let png = session.encode_png(&frame, surface.opaque)?;
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
                verify(&odd, odd_width, profile.physical_height, surface.opaque)?
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

fn calibration_view() -> impl IntoView {
    view! {
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
    }
}

fn stylesheet(profile: &LoadedProfile) -> String {
    // Both the background and family CSS have already been validated/escaped by the loader.
    let font_size = (profile.profile.surface.height as f32 / 12.0).clamp(8.0, 28.0);
    format!(r#"
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
    "#, profile.profile.surface.background, profile.font_family_css, font_size)
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

fn runtime_error(error: zgui::runtime::AppError) -> Error {
    match error {
        zgui::runtime::AppError::GpuUnavailable(_) => Error::new("E_GRAPHICS_UNAVAILABLE", 7, error.to_string()),
        _ => calibration_error(error.to_string()),
    }
}

fn calibration_error(message: impl Into<String>) -> Error { Error::new("E_CALIBRATION", 7, message) }
fn timeout_error() -> Error { Error::new("E_READINESS_TIMEOUT", 5, "calibration exceeded its real wall-clock budget") }
