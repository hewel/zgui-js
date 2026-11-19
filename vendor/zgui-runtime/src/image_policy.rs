//! Image access policy and owned readiness evidence for the native loader.

use std::{collections::BTreeMap, sync::Arc};

mod blobs;
pub(crate) mod files;

pub use blobs::ImageBlobs;
use files::FileAccess;

/// Source authority and publication, selected before mounting or at a stopped-owner replacement.
#[derive(Clone, Debug, Default)]
pub struct ImagePolicy {
    pub(crate) assets: Option<Arc<BTreeMap<String, Arc<[u8]>>>>,
    pub(crate) strict: bool,
    pub(crate) files: FileAccess,
    pub(crate) blobs: Option<ImageBlobs>,
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
    #[must_use]
    pub fn with_file_prefixes(mut self, prefixes: &[String]) -> Self {
        self.files = FileAccess::live(prefixes);
        self
    }

    /// Resolves files entirely from owned bytes, requiring both an exact URI and a granted prefix.
    ///
    /// Unlike [`Self::with_file_prefixes`], this never opens a filesystem path.
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
