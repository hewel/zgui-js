use std::{io, path::Path, sync::Arc};

use resvg::{tiny_skia, usvg};
use zgui_geom::{Device, Size};
use zgui_image::{DecodeError, Decoded, Limits};

use crate::image_policy::files::FileAccess;

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
    pub(super) fn probe(self, name: &str) -> Result<Size<u32, Device>, DecodeError> {
        let probe = |bytes: &[u8]| match svg_tree(bytes) {
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
    ) -> Result<Decoded, DecodeError> {
        let decode = |bytes: &[u8]| match svg_tree(bytes) {
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

fn svg_tree(bytes: &[u8]) -> Option<Result<usvg::Tree, DecodeError>> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    (bytes.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b'<')).then(|| parse_svg(bytes))
}

fn parse_svg(bytes: &[u8]) -> Result<usvg::Tree, DecodeError> {
    use std::sync::atomic::{AtomicBool, Ordering};

    let text = std::str::from_utf8(bytes).map_err(svg_error)?;
    // usvg's convenience parser enables DTDs and accepts compressed input. Use plain XML instead.
    let document = roxmltree::Document::parse(text).map_err(svg_error)?;
    let refused = AtomicBool::new(false);
    let options = usvg::Options {
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_string: Box::new(|_, _| {
                refused.store(true, Ordering::Relaxed);
                None
            }),
            // The default data resolver recursively parses SVG or sniffs arbitrary image MIME.
            resolve_data: Box::new(|_, _, _| {
                refused.store(true, Ordering::Relaxed);
                None
            }),
        },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_xmltree(&document, &options).map_err(svg_error)?;
    if refused.load(Ordering::Relaxed) {
        return Err(svg_error(
            "SVG image references (including nested data) are unsupported",
        ));
    }
    Ok(tree)
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
