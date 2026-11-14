//! Native RGBA8 samples only: no expansion, color transfer, premultiplication, or rescaling.
use std::io::{self, Cursor, Write};

use serde_json::{Value, json};

use crate::preview::Error;
use super::policy::{self, CaptureKey, CaptureSet, Mask, Policy, Scoring, MAX_EDGE, MAX_PIXELS};

#[derive(Debug)]
pub(super) struct Image {
    pub width: u32,
    pub height: u32,
    rgba: Vec<u8>,
}

pub(super) struct Comparison {
    pub passed: bool,
    /// Equality of all decoded samples, including masked pixels and RGB under zero alpha.
    pub exact_equal: bool,
    pub metrics: Value,
    pub diff_png: Option<Vec<u8>>,
}

pub(super) fn decode(bytes: &[u8], max_pixels: u64) -> Result<Image, Error> {
    // Limit decoder-owned metadata/row buffers separately from our checked output allocation.
    let mut options = png::DecodeOptions::default();
    options.set_ignore_checksums(false);
    options.set_skip_ancillary_crc_failures(false);
    let mut decoder = png::Decoder::new_with_options(Cursor::new(bytes), options);
    decoder.set_limits(png::Limits { bytes: 64 * 1024 * 1024 });
    decoder.set_transformations(png::Transformations::IDENTITY);
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    let header = decoder.read_header_info().map_err(png_error)?;
    let (width, height) = (header.width, header.height);
    let length = image_bytes(width, height, max_pixels)?;
    if header.color_type != png::ColorType::Rgba || header.bit_depth != png::BitDepth::Eight {
        return Err(policy::invalid("comparison PNG must contain native RGBA8 samples; conversion is not allowed"));
    }
    let mut reader = decoder.read_info().map_err(png_error)?;
    reject_animation(reader.info())?;
    if reader.output_buffer_size() != Some(length) {
        return Err(policy::invalid("PNG decoded size does not match its RGBA8 extent"));
    }
    let mut rgba = zeroed(length)?;
    let frame = reader.next_frame(&mut rgba).map_err(png_error)?;
    if frame.width != width || frame.height != height || frame.color_type != png::ColorType::Rgba
        || frame.bit_depth != png::BitDepth::Eight || frame.buffer_size() != length {
        return Err(policy::invalid("PNG frame does not match its native RGBA8 extent"));
    }
    // next_frame alone does not validate chunks after IDAT, including a missing/corrupt IEND.
    reader.finish().map_err(png_error)?;
    reject_animation(reader.info())?;
    Ok(Image { width, height, rgba })
}

fn reject_animation(info: &png::Info<'_>) -> Result<(), Error> {
    if info.animation_control.is_some() || info.frame_control.is_some() {
        return Err(policy::invalid("animated PNGs are not valid comparison captures"));
    }
    Ok(())
}

fn image_bytes(width: u32, height: u32, max_pixels: u64) -> Result<usize, Error> {
    let pixels = u64::from(width).checked_mul(u64::from(height));
    if !(1..=MAX_PIXELS).contains(&max_pixels) || width == 0 || height == 0
        || width > MAX_EDGE || height > MAX_EDGE || pixels.is_none_or(|count| count > max_pixels) {
        return Err(policy::invalid("PNG dimensions exceed maxPixels or the native 16384-pixel edge limit"));
    }
    pixels.and_then(|count| count.checked_mul(4)).and_then(|length| usize::try_from(length).ok())
        .ok_or_else(|| policy::invalid("RGBA8 byte size overflow"))
}

fn zeroed(length: usize) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|error| Error::new("E_OUTPUT_LIMIT", 8, format!("cannot allocate bounded RGBA image: {error}")))?;
    bytes.resize(length, 0);
    Ok(bytes)
}

fn png_error(error: png::DecodingError) -> Error { policy::invalid(format!("invalid comparison PNG: {error}")) }

// Mask union is normalized once per horizontal band, never searched per pixel. At most
// 256 rectangles produce 513 bands, each holding at most 256 disjoint x spans.
struct Band {
    end_y: u32,
    spans: Vec<(u32, u32)>,
}

fn mask_bands(masks: &[&Mask], width: u32, height: u32) -> Result<(Vec<Band>, u64), Error> {
    let mut edges = Vec::with_capacity(masks.len() * 2 + 2);
    edges.extend([0, height]);
    for mask in masks {
        let rect = &mask.rect;
        if rect.x.checked_add(rect.width).is_none_or(|end| end > width)
            || rect.y.checked_add(rect.height).is_none_or(|end| end > height) {
            return Err(policy::invalid("mask rectangle is outside the selected capture's physical extent"));
        }
        edges.extend([rect.y, rect.y + rect.height]);
    }
    edges.sort_unstable();
    edges.dedup();
    let mut bands = Vec::with_capacity(edges.len() - 1);
    let mut masked = 0_u64;
    for edge in edges.windows(2) {
        let (start_y, end_y) = (edge[0], edge[1]);
        let mut spans = Vec::new();
        for mask in masks {
            let rect = &mask.rect;
            if rect.y <= start_y && start_y < rect.y + rect.height {
                spans.push((rect.x, rect.x + rect.width));
            }
        }
        spans.sort_unstable();
        let mut count = 0;
        for index in 0..spans.len() {
            let (start, end) = spans[index];
            if count != 0 && start <= spans[count - 1].1 {
                spans[count - 1].1 = spans[count - 1].1.max(end);
            } else {
                spans[count] = (start, end);
                count += 1;
            }
        }
        spans.truncate(count);
        let row_masked: u64 = spans.iter().map(|(start, end)| u64::from(end - start)).sum();
        masked += row_masked * u64::from(end_y - start_y);
        bands.push(Band { end_y, spans });
    }
    if masked == u64::from(width) * u64::from(height) {
        return Err(policy::invalid("mask union covers the entire image; no pixels would be scored"));
    }
    Ok((bands, masked))
}

#[derive(Default)]
struct Bounds(Option<(u32, u32, u32, u32)>);
impl Bounds {
    fn include(&mut self, x: u32, y: u32) {
        self.0 = Some(match self.0 {
            None => (x, y, x, y),
            Some((left, top, right, bottom)) => (left.min(x), top.min(y), right.max(x), bottom.max(y)),
        });
    }
    fn json(&self) -> Value {
        self.0.map_or(Value::Null, |(left, top, right, bottom)| {
            json!({"x":left,"y":top,"width":right-left+1,"height":bottom-top+1})
        })
    }
}

pub(super) fn compare(key: &CaptureKey, left: &Image, right: &Image, policy: &Policy) -> Result<Comparison, Error> {
    if left.width != right.width || left.height != right.height {
        return Err(Error::new("E_BASELINE_INCOMPATIBLE", 2, "comparison image dimensions differ; rescaling is forbidden"));
    }
    let length = image_bytes(left.width, left.height, policy.max_pixels)?;
    if left.rgba.len() != length || right.rgba.len() != length {
        return Err(policy::invalid("comparison image buffer does not match its RGBA8 extent"));
    }
    if let CaptureSet::Subset { captures, .. } = &policy.capture_set
        && !captures.contains(key) {
        return Err(policy::invalid("cannot score a capture excluded by the policy subset"));
    }
    let masks: Vec<_> = policy.masks.iter()
        .filter(|mask| mask.capture_id == key.capture_id && mask.surface == key.surface).collect();
    let (bands, masked_pixels) = mask_bands(&masks, left.width, left.height)?;
    let scored_pixels = u64::from(left.width) * u64::from(left.height) - masked_pixels;
    let (channel_threshold, count_budget, ratio_budget, review_reason, mode) = match &policy.scoring {
        Scoring::Exact => (0, 0, 0.0, None, "exact"),
        Scoring::Tolerant { thresholds, review_reason } => (
            thresholds.channel, thresholds.max_changed_pixels, thresholds.max_changed_ratio,
            Some(review_reason.as_str()), "tolerant",
        ),
    };
    // Policy geometry is validated even for identical images. Exact equality needs no diff buffer.
    let exact_equal = left.rgba == right.rgba;
    let mut raw_different = 0_u64;
    let mut scored_different = 0_u64;
    let mut changed = 0_u64;
    let mut max_raw_delta = 0_u8;
    let mut max_delta = 0_u8;
    let mut raw_bounds = Bounds::default();
    let mut changed_bounds = Bounds::default();
    let diff_png = if exact_equal {
        None
    } else {
        let mut diff = zeroed(length)?;
        let mut band_index = 0;
        for y in 0..left.height {
            while y >= bands[band_index].end_y { band_index += 1; }
            let spans = &bands[band_index].spans;
            let mut span_index = 0;
            let row = y as usize * left.width as usize * 4;
            for x in 0..left.width {
                while span_index < spans.len() && x >= spans[span_index].1 { span_index += 1; }
                let masked = span_index < spans.len() && x >= spans[span_index].0;
                let offset = row + x as usize * 4;
                let mut deltas = [0_u8; 4];
                for channel in 0..4 { deltas[channel] = left.rgba[offset + channel].abs_diff(right.rgba[offset + channel]); }
                let delta = *deltas.iter().max().expect("four RGBA channels");
                // Opaque raw differences: alpha is magenta, including alpha-only and masked changes.
                diff[offset..offset + 4].copy_from_slice(&[deltas[0].max(deltas[3]), deltas[1], deltas[2].max(deltas[3]), 255]);
                max_raw_delta = max_raw_delta.max(delta);
                if delta != 0 { raw_different += 1; raw_bounds.include(x, y); }
                if !masked {
                    max_delta = max_delta.max(delta);
                    if delta != 0 { scored_different += 1; }
                    if delta > channel_threshold { changed += 1; changed_bounds.include(x, y); }
                }
            }
        }
        Some(encode_diff(&diff, left.width, left.height, policy.max_output_bytes)?)
    };
    let changed_ratio = changed as f64 / scored_pixels as f64;
    let passed = changed <= count_budget && changed_ratio <= ratio_budget;
    Ok(Comparison {
        passed, exact_equal, diff_png,
        metrics: json!({
            "mode":mode,"alpha":"include","width":left.width,"height":left.height,
            "channelThreshold":channel_threshold,"maxChangedPixels":count_budget,"maxChangedRatio":ratio_budget,
            "reviewReason":review_reason,"masks":masks,
            "maskedPixels":masked_pixels,"scoredPixels":scored_pixels,
            "rawDifferentPixels":raw_different,"scoredDifferentPixels":scored_different,
            "changedPixels":changed,"changedRatio":changed_ratio,
            "maxChannelDelta":max_delta,"maxRawChannelDelta":max_raw_delta,
            "changedBounds":changed_bounds.json(),"rawDifferenceBounds":raw_bounds.json(),
            "definitions":{
                "rawDifferentPixels":"All pixels with any unequal RGBA sample, including masked pixels.",
                "scoredDifferentPixels":"Unmasked pixels with any unequal RGBA sample.",
                "changedPixels":"Unmasked pixels whose largest absolute RGBA channel delta is strictly greater than channelThreshold.",
                "changedRatio":"changedPixels / scoredPixels; both count and ratio budgets must pass inclusively.",
                "maxChannelDelta":"Largest absolute RGBA channel delta over unmasked pixels; maxRawChannelDelta includes masks.",
                "bounds":"Smallest physical-pixel rectangle containing the named differences, or null for none.",
                "diffPng":"Opaque raw absolute RGB difference with alpha delta added via max to red and blue; black means equal. Includes masks and below-threshold changes."
            }
        }),
    })
}

struct LimitedOutput {
    bytes: Vec<u8>,
    limit: u64,
    limited: bool,
}
impl Write for LimitedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if (self.bytes.len() as u64).checked_add(bytes.len() as u64).is_none_or(|size| size > self.limit)
            || self.bytes.try_reserve(bytes.len()).is_err() {
            self.limited = true;
            return Err(io::Error::other("diff PNG exceeds its output allocation/byte limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

fn encode_diff(rgba: &[u8], width: u32, height: u32, limit: u64) -> Result<Vec<u8>, Error> {
    let mut output = LimitedOutput { bytes:Vec::new(), limit, limited:false };
    let result = (|| -> Result<(), png::EncodingError> {
        let mut encoder = png::Encoder::new(&mut output, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(rgba)?;
        writer.finish()
    })();
    if let Err(error) = result {
        return Err(Error::new(if output.limited { "E_OUTPUT_LIMIT" } else { "E_OUTPUT_IO" }, 8, format!("cannot encode comparison diff PNG: {error}")));
    }
    Ok(output.bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> CaptureKey { CaptureKey { capture_id:"shot".into(), surface:"main".into() } }
    fn policy(masks: Value, tolerant: Option<Value>) -> Policy {
        let mut value = json!({"schemaVersion":1,"mode":"exact","alpha":"include","captureSet":{"mode":"all"},"masks":masks});
        if let Some(thresholds) = tolerant {
            value["mode"] = json!("tolerant"); value["thresholds"] = thresholds;
            value["reviewReason"] = json!("reviewed deterministic rasterizer tolerance");
        }
        Policy::parse(&serde_json::to_vec(&value).unwrap()).unwrap()
    }
    fn image(width: u32, pixels: &[[u8; 4]]) -> Image {
        Image { width, height:pixels.len() as u32 / width, rgba:pixels.iter().flatten().copied().collect() }
    }
    fn mask(x: u32, y: u32, width: u32, height: u32) -> Value {
        json!({"captureId":"shot","surface":"main","rect":{"x":x,"y":y,"width":width,"height":height},"reason":"reviewed clock region"})
    }
    fn png(width: u32, height: u32, color: png::ColorType, depth: png::BitDepth, samples: &[u8], animated: bool) -> Vec<u8> {
        let mut output = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut output, width, height);
            encoder.set_color(color); encoder.set_depth(depth);
            if animated { encoder.set_animated(1, 0).unwrap(); }
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(samples).unwrap(); writer.finish().unwrap();
        }
        output
    }

    #[test]
    fn real_decode_preserves_all_samples_and_checks_complete_png() {
        let samples = [1,2,3,0, 4,5,6,127, 7,8,9,255, 10,11,12,1];
        let bytes = png(2, 2, png::ColorType::Rgba, png::BitDepth::Eight, &samples, false);
        let decoded = decode(&bytes, 4).unwrap();
        assert_eq!((decoded.width, decoded.height), (2,2)); assert_eq!(decoded.rgba, samples);
        assert!(decode(&bytes, 3).is_err());
        assert!(decode(&bytes, 0).is_err());
        assert!(decode(&bytes[..bytes.len()-12], 4).is_err());
        let mut corrupt = bytes.clone(); let last = corrupt.len()-1; corrupt[last] ^= 1;
        assert!(decode(&corrupt, 4).is_err());
        assert!(decode(b"not a PNG", 4).is_err());
    }

    #[test]
    fn decode_refuses_color_conversion_animation_and_native_extent_overflow() {
        for (color, depth, samples) in [
            (png::ColorType::Rgb, png::BitDepth::Eight, vec![1,2,3]),
            (png::ColorType::GrayscaleAlpha, png::BitDepth::Eight, vec![1,255]),
            (png::ColorType::Rgba, png::BitDepth::Sixteen, vec![0;8]),
        ] {
            assert!(decode(&png(1,1,color,depth,&samples,false),1).is_err());
        }
        assert!(decode(&png(1,1,png::ColorType::Rgba,png::BitDepth::Eight,&[1,2,3,4],true),1).is_err());
        let wide = png(MAX_EDGE+1,1,png::ColorType::Rgba,png::BitDepth::Eight,&vec![0; (MAX_EDGE as usize+1)*4],false);
        assert!(decode(&wide,MAX_PIXELS).is_err());
        assert!(image_bytes(u32::MAX,u32::MAX,MAX_PIXELS).is_err());
    }

    #[test]
    fn alpha_and_hidden_rgb_are_scored_and_alpha_only_diff_is_visible() {
        let left = image(2,&[[10,20,30,0],[0,0,0,0]]);
        let right = image(2,&[[10,20,30,7],[1,0,0,0]]);
        let result = compare(&key(),&left,&right,&policy(json!([]),None)).unwrap();
        assert!(!result.passed); assert!(!result.exact_equal);
        assert_eq!(result.metrics["changedPixels"],2); assert_eq!(result.metrics["maxChannelDelta"],7);
        assert_eq!(result.metrics["changedBounds"],json!({"x":0,"y":0,"width":2,"height":1}));
        let diff = decode(result.diff_png.as_ref().unwrap(),2).unwrap();
        assert_eq!(diff.rgba,[7,0,7,255, 1,0,0,255]);
        let same = compare(&key(),&left,&left,&policy(json!([]),None)).unwrap();
        assert!(same.passed && same.exact_equal && same.diff_png.is_none());
        assert_eq!(same.metrics["changedBounds"],Value::Null);
    }

    #[test]
    fn thresholds_are_strict_per_channel_and_both_budgets_are_inclusive() {
        let left = image(4,&[[0,0,0,0];4]);
        let right = image(4,&[[2,0,0,0],[0,3,0,0],[0,0,0,3],[0,0,0,0]]);
        let budgets = |count,ratio| policy(json!([]),Some(json!({"channel":2,"maxChangedPixels":count,"maxChangedRatio":ratio})));
        let result = compare(&key(),&left,&right,&budgets(2,0.5)).unwrap();
        assert!(result.passed && !result.exact_equal && result.diff_png.is_some());
        assert_eq!(result.metrics["rawDifferentPixels"],3); assert_eq!(result.metrics["changedPixels"],2);
        assert!(!compare(&key(),&left,&right,&budgets(1,0.5)).unwrap().passed);
        assert!(!compare(&key(),&left,&right,&budgets(2,0.499)).unwrap().passed);
        assert!(compare(&key(),&left,&image(4,&[[2,0,0,0];4]),&budgets(0,0.0)).unwrap().passed);
    }

    #[test]
    fn overlapping_mask_union_counts_once_and_uses_unmasked_denominator() {
        let left = image(4,&[[0,0,0,0];8]);
        let right = image(4,&[[0,0,0,4];8]);
        let policy = policy(json!([mask(0,0,2,2),mask(1,0,2,1)]),Some(json!({"channel":0,"maxChangedPixels":3,"maxChangedRatio":1.0})));
        let result = compare(&key(),&left,&right,&policy).unwrap();
        assert!(result.passed && !result.exact_equal);
        assert_eq!(result.metrics["maskedPixels"],5); assert_eq!(result.metrics["scoredPixels"],3);
        assert_eq!(result.metrics["changedPixels"],3); assert_eq!(result.metrics["changedRatio"],1.0);
        assert_eq!(result.metrics["rawDifferentPixels"],8);
        let diff = decode(result.diff_png.as_ref().unwrap(),8).unwrap();
        assert!(diff.rgba.chunks_exact(4).all(|pixel| pixel == [4,0,4,255]));
    }

    #[test]
    fn masked_changes_remain_raw_evidence_but_do_not_fail_exact_scoring() {
        let left = image(2,&[[0,0,0,0];2]); let right = image(2,&[[255,0,0,0],[0,0,0,0]]);
        let result = compare(&key(),&left,&right,&policy(json!([mask(0,0,1,1)]),None)).unwrap();
        assert!(result.passed && !result.exact_equal && result.diff_png.is_some());
        assert_eq!(result.metrics["rawDifferentPixels"],1); assert_eq!(result.metrics["changedPixels"],0);
        assert_eq!(result.metrics["maxRawChannelDelta"],255); assert_eq!(result.metrics["maxChannelDelta"],0);
    }

    #[test]
    fn invalid_mask_union_and_dimensions_fail_even_for_equal_images() {
        let equal = image(4,&[[0,0,0,0];8]);
        for masks in [json!([mask(0,0,3,2),mask(2,0,2,2)]),json!([mask(3,0,2,1)])] {
            let error = compare(&key(),&equal,&equal,&policy(masks,None)).err().unwrap();
            assert_eq!((error.code,error.exit_code),("E_SCHEMA",2));
        }
        let error = compare(&key(),&equal,&image(2,&[[0,0,0,0];8]),&policy(json!([]),None)).err().unwrap();
        assert_eq!(error.code,"E_BASELINE_INCOMPATIBLE");
    }

    #[test]
    fn diff_output_limit_fails_instead_of_omitting_review_evidence() {
        let mut limited = policy(json!([]),None); limited.max_output_bytes = 1;
        let error = compare(&key(),&image(1,&[[0,0,0,0]]),&image(1,&[[0,0,0,1]]),&limited).err().unwrap();
        assert_eq!((error.code,error.exit_code),("E_OUTPUT_LIMIT",8));
    }
}
