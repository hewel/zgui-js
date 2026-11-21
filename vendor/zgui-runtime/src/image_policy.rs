//! Image access policy and owned readiness evidence for the native loader.

use std::{collections::BTreeMap, sync::{Arc, OnceLock}};

mod blobs;
pub(crate) mod files;

pub use blobs::ImageBlobs;
use files::FileAccess;

/// Font database bound to an [`ImagePolicy`] for rasterizing SVG `<text>` elements.
#[derive(Debug)]
pub(crate) struct SvgFonts {
    pub(crate) db: Arc<resvg::usvg::fontdb::Database>,
    pub(crate) default_family: String,
}

impl SvgFonts {
    pub(crate) fn new(
        include_system_fonts: bool,
        faces: impl IntoIterator<Item = zgui_text::FontData>,
    ) -> Arc<Self> {
        let faces: Vec<_> = faces.into_iter().collect();
        if include_system_fonts {
            static SYSTEM: OnceLock<Arc<SvgFonts>> = OnceLock::new();
            let base = SYSTEM.get_or_init(|| {
                let mut db = resvg::usvg::fontdb::Database::new();
                db.load_system_fonts();
                let default_family = ensure_generic_families(&mut db, false);
                Arc::new(SvgFonts { db: Arc::new(db), default_family })
            });
            if faces.is_empty() {
                return Arc::clone(base);
            }
            let mut db = (*base.db).clone();
            for data in faces {
                db.load_font_source(resvg::usvg::fontdb::Source::Binary(data));
            }
            let default_family = ensure_generic_families(&mut db, false);
            return Arc::new(SvgFonts { db: Arc::new(db), default_family });
        }
        let mut db = resvg::usvg::fontdb::Database::new();
        for data in faces {
            db.load_font_source(resvg::usvg::fontdb::Source::Binary(data));
        }
        let default_family = ensure_generic_families(&mut db, true);
        Arc::new(SvgFonts { db: Arc::new(db), default_family })
    }
}

fn ensure_generic_families(db: &mut resvg::usvg::fontdb::Database, force_assign: bool) -> String {
    use resvg::usvg::fontdb::{Family, Query};
    let mut first_any: Option<String> = None;
    let mut sans: Option<String> = None;
    let mut serif: Option<String> = None;
    let mut mono: Option<String> = None;
    for face in db.faces() {
        let Some((family, _)) = face.families.first() else { continue; };
        if first_any.is_none() {
            first_any = Some(family.clone());
        }
        let lower = family.to_ascii_lowercase();
        if (face.monospaced || lower.contains("mono") || lower.contains("code")) && mono.is_none() {
            mono = Some(family.clone());
        } else if lower.contains("serif") && !lower.contains("sans") && serif.is_none() {
            serif = Some(family.clone());
        } else if sans.is_none() {
            sans = Some(family.clone());
        }
    }
    let fallback_sans = sans.or(first_any).unwrap_or_else(|| "sans-serif".to_owned());
    let fallback_serif = serif.unwrap_or_else(|| fallback_sans.clone());
    let fallback_mono = mono.unwrap_or_else(|| fallback_sans.clone());
    let has_family = |db: &resvg::usvg::fontdb::Database, family: Family<'_>| {
        db.query(&Query { families: &[family], ..Query::default() }).is_some()
    };
    if force_assign || !has_family(db, Family::SansSerif) {
        db.set_sans_serif_family(fallback_sans.clone());
    }
    if force_assign || !has_family(db, Family::Serif) {
        db.set_serif_family(fallback_serif);
    }
    if force_assign || !has_family(db, Family::Monospace) {
        db.set_monospace_family(fallback_mono);
    }
    if force_assign || !has_family(db, Family::Cursive) {
        db.set_cursive_family(fallback_sans.clone());
    }
    if force_assign || !has_family(db, Family::Fantasy) {
        db.set_fantasy_family(fallback_sans.clone());
    }
    let resolved = db.family_name(&Family::SansSerif);
    if resolved.is_empty() { fallback_sans } else { resolved.to_owned() }
}

/// Source authority and publication, selected before mounting or at a stopped-owner replacement.
#[derive(Clone, Debug, Default)]
pub struct ImagePolicy {
    pub(crate) assets: Option<Arc<BTreeMap<String, Arc<[u8]>>>>,
    pub(crate) strict: bool,
    pub(crate) files: FileAccess,
    pub(crate) blobs: Option<ImageBlobs>,
    pub(crate) svg_fonts: Option<Arc<SvgFonts>>,
}

impl ImagePolicy {
    /// Restricts sources to exact declared package-relative names backed by verified owned bytes.
    /// Strict publications happen only at `Window::poll_image_barrier`, in launch order.
    pub fn package_only(assets: impl IntoIterator<Item = (String, Arc<[u8]>)>) -> Self {
        Self { strict: true, ..Self::package_assets(assets) }
    }

    /// Uses immutable declared package bytes with ordinary native asynchronous publication.
    /// No working-directory or registered-byte fallback is permitted.
    pub fn package_assets(assets: impl IntoIterator<Item = (String, Arc<[u8]>)>) -> Self {
        Self { assets: Some(Arc::new(assets.into_iter().collect())), ..Self::default() }
    }

    /// Grants Linux worker reads of regular files beneath canonical absolute directory prefixes.
    ///
    /// Invalid prefixes grant nothing. Symlinked prefixes are refused; relative symlinks inside a
    /// granted directory remain usable only if they resolve beneath it.
    /// Callers resolve any package path templates before constructing this policy.
    #[must_use]
    pub fn with_file_prefixes(mut self, prefixes: &[String]) -> Self {
        self.files = FileAccess::live(prefixes);
        self
    }

    /// Resolves files entirely from owned bytes, requiring both an exact URI and a granted prefix.
    ///
    /// Unlike [`Self::with_file_prefixes`], this never opens a filesystem path.
    /// Both the prefixes and fixture paths are literal, not environment templates.
    #[must_use]
    pub fn with_fixture_files(
        mut self,
        prefixes: &[String],
        files: impl IntoIterator<Item = (String, Arc<[u8]>)>,
    ) -> Self {
        self.files = FileAccess::fixtures(prefixes, files);
        self
    }

    /// Makes this session's registered handles available without consulting the legacy byte registry.
    #[must_use]
    pub fn with_blobs(mut self, blobs: ImageBlobs) -> Self {
        self.blobs = Some(blobs);
        self
    }

    /// Binds fonts used to render `<text>` elements inside decoded SVG images.
    ///
    /// Live desktop sessions pass `include_system_fonts = true`; deterministic preview sessions
    /// pass `include_system_fonts = false` with only the shipped/package font faces.
    #[must_use]
    pub fn with_svg_fonts(
        mut self,
        include_system_fonts: bool,
        faces: impl IntoIterator<Item = zgui_text::FontData>,
    ) -> Self {
        self.svg_fonts = Some(SvgFonts::new(include_system_fonts, faces));
        self
    }
}

/// Audits a stylesheet for CSS resource constructs forbidden in packaged applications.
///
/// # Errors
///
/// Returns a diagnostic for URL tokens, `@import`, or `image-set`, including in nested values.
pub fn audit_stylesheet_image_urls(css: &str) -> Result<(), &'static str> {
    let mut input = cssparser::ParserInput::new(css);
    if forbidden_css(&mut cssparser::Parser::new(&mut input)) {
        return Err("CSS resource URLs, @import, and image-set are unsupported by the native painter; use an image element with a declared package src");
    }
    Ok(())
}

fn forbidden_css(parser: &mut cssparser::Parser<'_, '_>) -> bool {
    use cssparser::Token;
    while let Ok(token) = parser.next_including_whitespace_and_comments().cloned() {
        match token {
            Token::UnquotedUrl(_) | Token::BadUrl(_) => return true,
            Token::AtKeyword(name) if name.eq_ignore_ascii_case("import") => return true,
            Token::Function(name) if name.eq_ignore_ascii_case("url") || name.eq_ignore_ascii_case("image-set") || name.eq_ignore_ascii_case("-webkit-image-set") => return true,
            Token::Function(_) | Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                let nested: Result<bool, cssparser::ParseError<'_, ()>> = parser.parse_nested_block(|parser| Ok(forbidden_css(parser)));
                if nested.unwrap_or(true) { return true; }
            }
            _ => {}
        }
    }
    false
}

/// Current state of one native image request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageState {
    /// The request has not yet acquired layout demand.
    Requested,
    /// A probe/decode/publication is outstanding, or the attached pixels are not uploaded yet.
    Loading,
    /// The attached, successfully uploaded resident variant satisfies current device-pixel demand.
    Ready,
    /// Source authority, header, decode, or actual native image allocation failed.
    Failed,
    /// The source was replaced, cleared, or its node retired.
    Canceled,
}

/// Owned evidence for one source assignment, never a JavaScript or document borrow.
#[derive(Clone, Debug)]
pub struct ImageResource {
    /// Stable request identity, unique within a window.
    pub id: String,
    /// Exact application source string.
    pub source: String,
    /// Monotonic source-assignment generation.
    pub generation: u64,
    /// Whether current placed/clipped geometry can contribute pixels to the surface.
    pub visible: bool,
    /// Long-edge current decode target in device pixels.
    pub requested_variant: Option<u32>,
    /// Long-edge variant currently attached (may be insufficient).
    pub attached_variant: Option<u32>,
    /// The attached variant has successfully reached a resident native texture.
    pub uploaded: bool,
    /// Current lifecycle state.
    pub state: ImageState,
    /// Failure diagnostic, if any.
    pub failure: Option<String>,
}

/// Bounded readiness evidence. Truncation must never certify capture readiness.
#[derive(Clone, Debug, Default)]
pub struct ImageResources {
    /// Active requests followed by bounded retirement history, in generation order.
    pub resources: Vec<ImageResource>,
    /// Number of retired records omitted from the bounded history.
    pub dropped: u64,
    /// Whether active requests exceeded the evidence limit.
    pub truncated: bool,
    /// Sticky policy failures, including unsupported CSS resource references.
    pub failures: Vec<String>,
    /// A graphics-device upload refusal, distinct from an asset decode/allocation failure.
    pub graphics_failure: Option<String>,
    /// Manifest-declared sources, bounded separately from active request evidence.
    pub declared_assets: Vec<String>,
    /// Number of declarations omitted from this snapshot.
    pub dropped_declarations: u64,
}
