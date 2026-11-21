use std::{
    io,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

use resvg::{tiny_skia, usvg};
use zgui_geom::{Device, Size};
use zgui_image::{DecodeError, Decoded, Limits};

use crate::image_policy::{SvgFonts, files::FileAccess};

/// Maximum decoded byte length of one embedded raster data URI inside an SVG.
const MAX_SVG_EMBEDDED_BYTES: usize = 4 * 1024 * 1024;
/// Maximum number of embedded raster images permitted in a single SVG document.
const MAX_SVG_EMBEDDED_IMAGES: usize = 32;

/// An owned worker job input; preparing it never touches the filesystem.
pub(super) enum Input {
    Dynamic(Arc<[u8]>),
    Package(Arc<[u8]>),
    Registered(Arc<Vec<u8>>),
    External(FileAccess),
    UnrestrictedPath,
    Denied(io::Error),
}

impl Input {
    pub(super) fn probe(
        self,
        name: &str,
        limits: Limits,
        svg_fonts: Option<&SvgFonts>,
    ) -> Result<Size<u32, Device>, DecodeError> {
        let probe = |bytes: &[u8]| match svg_tree(bytes, limits, svg_fonts) {
            Some(tree) => {
                let native = tree?.size().to_int_size();
                Ok(Size::new(native.width(), native.height()))
            }
            None => zgui_image::probe(bytes),
        };
        match self {
            Self::UnrestrictedPath => zgui_image::probe_file(Path::new(name)),
            Self::Package(bytes) => zgui_image::probe(&bytes),
            Self::Registered(bytes) => zgui_image::probe(&bytes),
            input => input.with_bytes(name, probe),
        }
    }

    pub(super) fn decode(
        self,
        name: &str,
        class: u32,
        limits: Limits,
        svg_fonts: Option<&SvgFonts>,
    ) -> Result<Decoded, DecodeError> {
        let decode = |bytes: &[u8]| match svg_tree(bytes, limits, svg_fonts) {
            Some(tree) => rasterize(&tree?, class.min(limits.max_dimension).max(1)),
            None => zgui_image::decode_scaled(bytes, class, limits),
        };
        match self {
            Self::UnrestrictedPath => {
                zgui_image::decode_file_scaled(Path::new(name), class, limits)
            }
            Self::Package(bytes) => zgui_image::decode_scaled(&bytes, class, limits),
            Self::Registered(bytes) => zgui_image::decode_scaled(&bytes, class, limits),
            input => input.with_bytes(name, decode),
        }
    }

    fn with_bytes<T>(
        self,
        name: &str,
        work: impl FnOnce(&[u8]) -> Result<T, DecodeError>,
    ) -> Result<T, DecodeError> {
        match self {
            Self::Dynamic(bytes) => work(&bytes),
            Self::External(files) => work(&files.read_live(name)?),
            Self::Denied(error) => Err(error.into()),
            Self::Package(_) | Self::Registered(_) | Self::UnrestrictedPath => {
                unreachable!("only dynamic sources use the SVG decoder")
            }
        }
    }
}

fn svg_tree(
    bytes: &[u8],
    limits: Limits,
    svg_fonts: Option<&SvgFonts>,
) -> Option<Result<usvg::Tree, DecodeError>> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    (bytes.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b'<'))
        .then(|| parse_svg(bytes, limits, svg_fonts))
}

fn parse_svg(
    bytes: &[u8],
    limits: Limits,
    svg_fonts: Option<&SvgFonts>,
) -> Result<usvg::Tree, DecodeError> {
    let text = std::str::from_utf8(bytes).map_err(svg_error)?;
    // usvg's convenience parser enables DTDs and accepts compressed input. Use plain XML instead.
    let document = roxmltree::Document::parse(text).map_err(svg_error)?;
    let refused = AtomicBool::new(false);
    let embedded_count = AtomicUsize::new(0);
    let embedded_pixels = AtomicU64::new(0);
    let mut options = usvg::Options {
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_string: Box::new(|_, _| {
                refused.store(true, Ordering::Relaxed);
                None
            }),
            // Allow bounded embedded PNG/JPEG/WebP data URIs (used by Linux icon themes for
            // raster shadow/glow layers) while refusing nested SVG and unverified formats.
            resolve_data: Box::new(|mime, data, _| {
                let resolved = resolve_embedded_raster(
                    mime,
                    data,
                    limits,
                    &embedded_count,
                    &embedded_pixels,
                );
                if resolved.is_none() {
                    refused.store(true, Ordering::Relaxed);
                }
                resolved
            }),
        },
        ..usvg::Options::default()
    };
    if let Some(fonts) = svg_fonts {
        options.fontdb = Arc::clone(&fonts.db);
        options.font_family = fonts.default_family.clone();
    }
    let tree = usvg::Tree::from_xmltree(&document, &options).map_err(svg_error)?;
    if refused.load(Ordering::Relaxed) {
        return Err(svg_error(
            "SVG image references (including nested data) are unsupported",
        ));
    }
    Ok(tree)
}

fn resolve_embedded_raster(
    mime: &str,
    data: Arc<Vec<u8>>,
    limits: Limits,
    count: &AtomicUsize,
    total_pixels: &AtomicU64,
) -> Option<usvg::ImageKind> {
    if data.is_empty() || data.len() > MAX_SVG_EMBEDDED_BYTES {
        return None;
    }
    if count.fetch_add(1, Ordering::Relaxed) >= MAX_SVG_EMBEDDED_IMAGES {
        return None;
    }
    let kind = match mime.to_ascii_lowercase().as_str() {
        "image/png" if data.starts_with(b"\x89PNG\r\n\x1a\n") => usvg::ImageKind::PNG(data),
        "image/jpeg" | "image/jpg" if data.starts_with(&[0xff, 0xd8, 0xff]) => {
            usvg::ImageKind::JPEG(data)
        }
        "image/webp" if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" => {
            usvg::ImageKind::WEBP(data)
        }
        _ => return None,
    };
    let raw = match &kind {
        usvg::ImageKind::PNG(bytes)
        | usvg::ImageKind::JPEG(bytes)
        | usvg::ImageKind::WEBP(bytes) => bytes.as_slice(),
        _ => return None,
    };
    let probed = zgui_image::probe(raw).ok()?;
    if probed.width == 0
        || probed.height == 0
        || probed.width > limits.max_dimension
        || probed.height > limits.max_dimension
    {
        return None;
    }
    let pixels = u64::from(probed.width).checked_mul(u64::from(probed.height))?;
    let max_total_pixels =
        u64::from(limits.max_dimension).saturating_mul(u64::from(limits.max_dimension));
    let next = total_pixels
        .fetch_add(pixels, Ordering::Relaxed)
        .saturating_add(pixels);
    if next > max_total_pixels {
        return None;
    }
    Some(kind)
}

fn svg_error(error: impl std::fmt::Display) -> DecodeError {
    io::Error::new(io::ErrorKind::InvalidData, format!("invalid SVG: {error}")).into()
}

fn rasterize(tree: &usvg::Tree, max_dimension: u32) -> Result<Decoded, DecodeError> {
    let native = tree.size();
    let scale = (max_dimension as f32 / native.width().max(native.height())).min(1.0);
    let width = (native.width() * scale).round().max(1.0) as u32;
    let height = (native.height() * scale).round().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| svg_error("could not allocate raster surface"))?;
    resvg::render(
        tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny-skia already produces tightly packed, premultiplied RGBA.
    Ok(Decoded {
        size: Size::new(width, height),
        texels: Arc::new(pixmap.take()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED_PNG: &[u8] = include_bytes!("../../tests/fixtures/red2x2.png");

    fn encode_base64(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let b0 = u32::from(chunk[0]);
            let b1 = chunk.get(1).copied().map_or(0, u32::from);
            let b2 = chunk.get(2).copied().map_or(0, u32::from);
            let triple = (b0 << 16) | (b1 << 8) | b2;
            out.push(TABLE[((triple >> 18) & 0x3f) as usize] as char);
            out.push(TABLE[((triple >> 12) & 0x3f) as usize] as char);
            out.push(if chunk.len() > 1 {
                TABLE[((triple >> 6) & 0x3f) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                TABLE[(triple & 0x3f) as usize] as char
            } else {
                '='
            });
        }
        out
    }

    #[test]
    fn embedded_png_data_uri_renders_non_blank_pixels() {
        let encoded = encode_base64(RED_PNG);
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="8" height="8">
                <image width="8" height="8" xlink:href="data:image/png;base64,{encoded}"/>
            </svg>"#
        );
        let bytes: Arc<[u8]> = Arc::from(svg.into_bytes());
        let probed = Input::Dynamic(Arc::clone(&bytes))
            .probe("icon.svg", Limits::default(), None)
            .expect("probing SVG with embedded PNG succeeds");
        assert_eq!(probed, Size::new(8, 8));

        let decoded = Input::Dynamic(bytes)
            .decode("icon.svg", 8, Limits::default(), None)
            .expect("decoding SVG with embedded PNG succeeds");
        assert_eq!(decoded.size, Size::new(8, 8));
        assert!(
            decoded
                .texels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|px| px[0] > 200 && px[1] < 20 && px[2] < 20 && px[3] > 200),
            "embedded PNG must actually rasterize red pixels into the SVG pixmap"
        );
    }

    #[test]
    fn embedded_raster_enforces_limits_and_magic_bytes() {
        let encoded = encode_base64(RED_PNG);
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8">
                <image width="8" height="8" href="data:image/png;base64,{encoded}"/>
            </svg>"#
        );
        let err = Input::Dynamic(Arc::from(svg.into_bytes()))
            .decode("icon.svg", 8, Limits { max_dimension: 1 }, None)
            .expect_err("2x2 embedded PNG exceeds max_dimension=1");
        assert!(err.to_string().contains("SVG image references"), "{err}");

        let spoofed = encode_base64(b"not a png payload");
        let spoofed_svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8">
                <image width="8" height="8" href="data:image/png;base64,{spoofed}"/>
            </svg>"#
        );
        let err = Input::Dynamic(Arc::from(spoofed_svg.into_bytes()))
            .decode("icon.svg", 8, Limits::default(), None)
            .expect_err("spoofed PNG magic bytes must be rejected");
        assert!(err.to_string().contains("SVG image references"), "{err}");

        let gif_svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8">
                <image width="8" height="8" href="data:image/gif;base64,{}"/>
            </svg>"#,
            encode_base64(b"GIF89a\x01\x00\x01\x00")
        );
        let err = Input::Dynamic(Arc::from(gif_svg.into_bytes()))
            .decode("icon.svg", 8, Limits::default(), None)
            .expect_err("GIF data URIs remain rejected");
        assert!(err.to_string().contains("SVG image references"), "{err}");

        let mut many_images = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8">"#);
        for _ in 0..=MAX_SVG_EMBEDDED_IMAGES {
            many_images.push_str(&format!(
                r#"<image width="2" height="2" href="data:image/png;base64,{encoded}"/>"#
            ));
        }
        many_images.push_str("</svg>");
        let err = Input::Dynamic(Arc::from(many_images.into_bytes()))
            .decode("icon.svg", 8, Limits::default(), None)
            .expect_err("exceeding embedded image count cap must be rejected");
        assert!(err.to_string().contains("SVG image references"), "{err}");
    }

    #[test]
    fn svg_text_renders_glyphs_using_bound_font_database() {
        let noto: zgui_text::FontData =
            Arc::new(include_bytes!("../../../../preview/fonts/NotoSans-Regular.ttf").as_slice());
        let fonts = SvgFonts::new(false, [noto]);
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="40">
            <rect width="120" height="40" fill="#000000"/>
            <text x="10" y="28" font-family="sans-serif" font-size="22" fill="#00ff88">Hello</text>
        </svg>"##;
        let decoded = Input::Dynamic(Arc::from(svg.as_slice()))
            .decode("diagram.svg", 120, Limits::default(), Some(&fonts))
            .expect("SVG with <text> decodes cleanly");
        assert_eq!(decoded.size, Size::new(120, 40));
        let green_pixels = decoded
            .texels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[1] > 180 && px[0] < 40 && px[3] > 200)
            .count();
        assert!(
            green_pixels > 40,
            "SVG <text> must render visible glyph pixels using the session fontdb, got {green_pixels}",
        );
    }
}
