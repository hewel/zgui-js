//! Replaced content into atlas tiles and external textures.
//!
//! A replaced node names its content and says nothing about it; what that name resolves to is
//! decided here. There are exactly two answers, because there are exactly two places pixels can
//! be: inside this framework's own atlas, which is where a decoded still image goes, and inside
//! somebody else's texture, which is where a video frame or another process's surface stays.
//!
//! # Decoding is not done here, and cannot be
//!
//! What arrives is already-decoded, already-premultiplied texels. Decoding a file format is a
//! decision about which codecs an application links, and it happens off the frame thread; this
//! module is reached while a frame is being built and only ever moves bytes into a tile.

use zgui_atlas::{Atlas, AtlasError, AtlasKey, TextureKind};
use zgui_dom::host::ReplacedId;
use zgui_geom::{Device, Size};
use zgui_scene::ExternalTextureId;

use crate::emit::replaced::Source;

/// What went wrong attaching content to a replaced node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ImageError {
    /// The byte count does not match the extent: four premultiplied bytes per texel are needed.
    #[error("an image of {size:?} needs {expected} bytes and {actual} were given")]
    WrongByteCount {
        /// The extent that was declared.
        size: Size<u32, Device>,
        /// How many bytes that extent needs.
        expected: usize,
        /// How many were given.
        actual: usize,
    },
}

/// One level of detail below an image's own resolution.
#[derive(Clone, Debug)]
pub struct MipLevel {
    /// The level's extent: half the previous one's, rounded down, never below one.
    pub size: Size<u32, Device>,
    /// The level's texels, in the same format as the image's own.
    pub texels: std::sync::Arc<Vec<u8>>,
}

/// The handle namespace of tiles a loader shares between nodes.
///
/// Set on a loader's counter and never on a node's packed name, so the two vocabularies cannot
/// meet in the atlas and serve one node's pixels for another's.
const SHARED: u64 = 1 << 63;

/// The handle namespace of `clip-path`-masked image tiles.
///
/// Any derived shared-image namespace must uphold the residency and cleanup contract documented on
/// [`crate::content::cache::ContentCache::image_tile_resident`].
const CLIPPED: u64 = 1 << 62;

/// The atlas key a loader's shared handle resolves to.
pub(crate) fn shared_key(handle: u64) -> AtlasKey {
    AtlasKey::new(SHARED | handle, TextureKind::Image)
}

/// One piece of replaced content, as the frame draws it.
#[derive(Clone, Debug)]
pub(crate) enum Content {
    /// Texels this framework owns, cached in its own atlas under `key`.
    Decoded {
        /// What the atlas caches the texels under.
        key: AtlasKey,
        /// The extent of the texels.
        size: Size<u32, Device>,
        /// The image's own extent, which the texels may be a downscale of.
        natural: Size<u32, Device>,
        /// The texels, premultiplied and tightly packed, held so the tile can be rebuilt after an
        /// eviction or a lost device without asking the application to decode again.
        texels: std::sync::Arc<Vec<u8>>,
        /// Levels of detail below `texels`, for an image large enough to have its own texture.
        ///
        /// Empty for content that shares an atlas page: a page holds many tiles and can have no
        /// level of detail of its own. An image that arrives with levels is cached standalone —
        /// the texture is the tile — and minified sampling reads them.
        mips: Vec<MipLevel>,
    },
    /// Texels whose tile is resident and whose host copy has been given back.
    ///
    /// The state a shared attachment settles into once its upload has flushed: the atlas serves
    /// the tile, and nothing on the host doubles it. A frame that finds the tile gone — evicted,
    /// or lost with the device — reports the node as missing rather than drawing, and whoever
    /// owns the source decodes it again.
    Uploaded {
        /// What the atlas caches the texels under; the tile itself carries the extent.
        key: AtlasKey,
        /// The image's own extent, which the tile's texels may be a downscale of.
        natural: Size<u32, Device>,
    },
    /// A texture somebody else owns, drawn from where it already is.
    External(ExternalTextureId, Option<Size<u32, Device>>),
}

impl Content {
    /// Builds a decoded entry, checking the bytes against the extent.
    pub(crate) fn decoded(
        id: ReplacedId,
        size: Size<u32, Device>,
        texels: Vec<u8>,
    ) -> Result<Self, ImageError> {
        Self::under_key(
            AtlasKey::new(handle(id), TextureKind::Image),
            size,
            size,
            std::sync::Arc::new(texels),
            Vec::new(),
        )
    }

    /// Builds a decoded entry under a caller-chosen handle, so nodes can share one tile.
    ///
    /// The handle is namespaced away from the per-node ones: a loader's counter and a node's
    /// packed name must never meet in the atlas, because the atlas would then serve one node's
    /// pixels for the other's.
    pub(crate) fn shared(
        handle: u64,
        size: Size<u32, Device>,
        natural: Size<u32, Device>,
        texels: std::sync::Arc<Vec<u8>>,
        mips: Vec<MipLevel>,
    ) -> Result<Self, ImageError> {
        Self::under_key(shared_key(handle), size, natural, texels, mips)
    }

    /// Builds an entry for a shared tile that is already resident, carrying no texels at all.
    pub(crate) fn uploaded_shared(handle: u64, natural: Size<u32, Device>) -> Self {
        Self::Uploaded {
            key: shared_key(handle),
            natural,
        }
    }

    /// Whether this entry's key is in the loader-shared namespace.
    ///
    /// A shared entry has an owner that can decode it again, which is what makes dropping its
    /// host texels safe; a per-node entry was attached directly and its texels are the only copy.
    pub(crate) fn is_shared(&self) -> bool {
        match self {
            Self::Decoded { key, .. } | Self::Uploaded { key, .. } => key.handle() & SHARED != 0,
            Self::External(..) => false,
        }
    }

    /// The entry both constructors build, once the bytes are checked against the extent.
    fn under_key(
        key: AtlasKey,
        size: Size<u32, Device>,
        natural: Size<u32, Device>,
        texels: std::sync::Arc<Vec<u8>>,
        mips: Vec<MipLevel>,
    ) -> Result<Self, ImageError> {
        let expected = size.width as usize * size.height as usize * 4;
        if texels.len() != expected {
            return Err(ImageError::WrongByteCount {
                size,
                expected,
                actual: texels.len(),
            });
        }
        for level in &mips {
            let expected = level.size.width as usize * level.size.height as usize * 4;
            if level.texels.len() != expected {
                return Err(ImageError::WrongByteCount {
                    size: level.size,
                    expected,
                    actual: level.texels.len(),
                });
            }
        }
        Ok(Self::Decoded {
            key,
            size,
            natural,
            texels,
            mips,
        })
    }

    /// How many bytes of decoded texels this entry is holding outside the atlas.
    ///
    /// Zero for external content, which is somebody else's memory and would be counted twice by a
    /// budget that claimed it.
    pub(crate) fn held_bytes(&self) -> u64 {
        match self {
            Self::External(..) | Self::Uploaded { .. } => 0,
            Self::Decoded { texels, mips, .. } => {
                texels.len() as u64
                    + mips
                        .iter()
                        .map(|level| level.texels.len() as u64)
                        .sum::<u64>()
            }
        }
    }
}

/// Where one piece of content is right now, putting it in the atlas if it is not there yet.
///
/// The key comes back with the source, and is `None` for content this framework does not own: an
/// external texture is somebody else's to free, so there is nothing here to hold it by. A caller
/// that is going to replay the primitive rather than emit it again needs the key — see
/// [`hold`](crate::walk::replay::hold) — and cannot recover it from the tile.
///
/// `Ok(None)` is an [`Uploaded`](Content::Uploaded) entry whose tile is gone: there are no texels
/// here to rebuild it from, so the node draws nothing this frame and the caller reports it for a
/// re-decode.
///
/// # Errors
///
/// Whatever the atlas refused the insertion with. [`AtlasError::OutOfSpace`] is a signal to evict
/// and call again; the rest are final for this content.
pub(crate) fn source_of(
    atlas: &mut Atlas,
    content: &Content,
) -> Result<Option<(Source, Option<AtlasKey>)>, AtlasError> {
    match content {
        Content::External(texture, natural) => {
            Ok(Some((Source::External(*texture, *natural), None)))
        }
        Content::Uploaded { key, natural, .. } => Ok(atlas.get(*key).map(|tile| {
            (
                Source::Decoded {
                    resource: tile.into(),
                    natural: *natural,
                },
                Some(*key),
            )
        })),
        Content::Decoded {
            key,
            size,
            natural,
            texels,
            mips,
        } => {
            let extent = Size::new(size.width as i32, size.height as i32);
            let tile = if mips.is_empty() {
                atlas.get_or_insert(*key, extent, || std::sync::Arc::clone(texels))?
            } else {
                atlas.insert_standalone(*key, extent, || {
                    std::iter::once(std::sync::Arc::clone(texels))
                        .chain(
                            mips.iter()
                                .map(|level| std::sync::Arc::clone(&level.texels)),
                        )
                        .collect::<Vec<_>>()
                })?
            };
            Ok(Some((
                Source::Decoded {
                    resource: tile.into(),
                    natural: *natural,
                },
                Some(*key),
            )))
        }
    }
}

/// Maximum device-pixel dimension per axis for a CPU-masked `clip-path` image tile.
const MAX_CLIPPED_SIDE: u32 = 2048;

/// Produces a cached, `clip-path`-masked tile for `content` fitted to `placement.content_box`.
///
/// Returns `Ok(None)` when `content` is [`Content::Uploaded`] and the masked variant is not in
/// the atlas, signalling the caller to request a re-decode.
pub(crate) fn clipped_source_of(
    atlas: &mut Atlas,
    content: &Content,
    revision: u64,
    placement: &crate::emit::replaced::ReplacedPlacement,
    clip_path: &crate::lower::ClipPathShape,
    reference_box: zgui_geom::Rect<zgui_geom::DevicePx, Device>,
) -> Result<Option<(Source, Option<AtlasKey>)>, AtlasError> {
    let (base_key, natural, decoded) = match content {
        Content::External(texture, natural) => {
            return match external_mask(atlas, placement, clip_path, reference_box)? {
                Some((mask, key)) => Ok(Some((
                    Source::ExternalMasked {
                        texture: *texture,
                        natural: *natural,
                        mask,
                    },
                    Some(key),
                ))),
                None => source_of(atlas, content),
            };
        }
        Content::Uploaded { key, natural } => (*key, *natural, None),
        Content::Decoded {
            key,
            size,
            natural,
            texels,
            ..
        } => (*key, *natural, Some((*size, texels))),
    };
    let out_w = (placement.content_box.size.width.0.round() as u32).clamp(1, MAX_CLIPPED_SIDE);
    let out_h = (placement.content_box.size.height.0.round() as u32).clamp(1, MAX_CLIPPED_SIDE);
    let dest = crate::emit::replaced::destination(placement, natural);
    let dest_x = dest.origin.x.0 - placement.content_box.origin.x.0;
    let dest_y = dest.origin.y.0 - placement.content_box.origin.y.0;
    let dest_w = dest.size.width.0;
    let dest_h = dest.size.height.0;
    let ref_x = reference_box.origin.x.0 - placement.content_box.origin.x.0;
    let ref_y = reference_box.origin.y.0 - placement.content_box.origin.y.0;
    let hash = zgui_scene::ContentHash::new()
        .u64(base_key.handle())
        .u64(revision)
        .u64(clip_path.hash())
        .u32(out_w)
        .u32(out_h)
        .f32s(&[
            dest_x,
            dest_y,
            dest_w,
            dest_h,
            ref_x,
            ref_y,
            reference_box.size.width.0,
            reference_box.size.height.0,
            placement.scale,
        ])
        .finish();
    let masked_key = AtlasKey::new((hash & !(SHARED | CLIPPED)) | CLIPPED, TextureKind::Image);
    let out_natural = Size::new(out_w, out_h);
    if let Some(tile) = atlas.get(masked_key) {
        return Ok(Some((
            Source::Decoded {
                resource: tile.into(),
                natural: out_natural,
            },
            Some(masked_key),
        )));
    }
    let Some((src_size, texels)) = decoded else {
        return Ok(None);
    };
    let Some(mask) = clip_path.rasterize_mask(
        reference_box,
        placement.content_box,
        placement.scale,
        out_w,
        out_h,
    ) else {
        return source_of(atlas, content);
    };
    let masked_bytes = sample_and_mask(
        texels,
        src_size.width,
        src_size.height,
        out_w,
        out_h,
        (dest_x, dest_y, dest_w, dest_h),
        &mask,
    );
    let extent = Size::new(out_w as i32, out_h as i32);
    let tile = atlas.get_or_insert(masked_key, extent, || std::sync::Arc::new(masked_bytes))?;
    Ok(Some((
        Source::Decoded {
            resource: tile.into(),
            natural: out_natural,
        },
        Some(masked_key),
    )))
}

/// Cache only the shape, not the foreign pixels or their per-frame revision.
fn external_mask(
    atlas: &mut Atlas,
    placement: &crate::emit::replaced::ReplacedPlacement,
    shape: &crate::lower::ClipPathShape,
    reference: zgui_geom::Rect<zgui_geom::DevicePx, Device>,
) -> Result<Option<(zgui_atlas::AtlasTile, AtlasKey)>, AtlasError> {
    use zgui_geom::{DevicePx, Point, Rect};
    let target = placement.content_box;
    let factor = (MAX_CLIPPED_SIDE as f32 / target.size.width.0.max(target.size.height.0)).min(1.0);
    let w = (target.size.width.0 * factor).round().max(1.0) as u32;
    let h = (target.size.height.0 * factor).round().max(1.0) as u32;
    let rx = reference.origin.x.0 - target.origin.x.0;
    let ry = reference.origin.y.0 - target.origin.y.0;
    let hash = zgui_scene::ContentHash::new()
        .u64(shape.hash())
        .u32(w)
        .u32(h)
        .f32s(&[
            rx,
            ry,
            reference.size.width.0,
            reference.size.height.0,
            placement.scale,
            factor,
        ])
        .finish();
    let key = AtlasKey::new((hash & !(SHARED | CLIPPED)) | CLIPPED, TextureKind::Mono);
    if let Some(tile) = atlas.get(key) {
        return Ok(Some((tile, key)));
    }
    let Some(mask) = shape.rasterize_mask(
        Rect::new(
            Point::new(DevicePx(rx * factor), DevicePx(ry * factor)),
            Size::new(
                DevicePx(reference.size.width.0 * factor),
                DevicePx(reference.size.height.0 * factor),
            ),
        ),
        Rect::new(
            Point::new(DevicePx(0.0), DevicePx(0.0)),
            Size::new(DevicePx(w as f32), DevicePx(h as f32)),
        ),
        placement.scale * factor,
        w,
        h,
    ) else {
        return Ok(None);
    };
    let tile = atlas.get_or_insert(key, Size::new(w as i32, h as i32), || {
        std::sync::Arc::new(mask)
    })?;
    Ok(Some((tile, key)))
}

/// Bilinearly resamples premultiplied RGBA8 `src` into `out_w x out_h` fitted to `dest`,
/// multiplying each output channel by the 8-bit `mask` coverage.
fn sample_and_mask(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    out_w: u32,
    out_h: u32,
    dest: (f32, f32, f32, f32),
    mask: &[u8],
) -> Vec<u8> {
    let mut out = vec![0u8; (out_w as usize) * (out_h as usize) * 4];
    let (dest_x, dest_y, dest_w, dest_h) = dest;
    if src_w == 0 || src_h == 0 || dest_w <= 0.0 || dest_h <= 0.0 {
        return out;
    }
    let src_wf = src_w as f32;
    let src_hf = src_h as f32;
    let max_sx = (src_w - 1) as i32;
    let max_sy = (src_h - 1) as i32;
    let row_bytes = (src_w as usize) * 4;
    for y in 0..out_h {
        let py = y as f32 + 0.5;
        let v_norm = (py - dest_y) / dest_h;
        if !(0.0..=1.0).contains(&v_norm) {
            continue;
        }
        let sy = v_norm * src_hf - 0.5;
        let y0 = (sy.floor() as i32).clamp(0, max_sy);
        let y1 = (y0 + 1).min(max_sy);
        let fy = (sy - y0 as f32).clamp(0.0, 1.0);
        let row0 = (y0 as usize) * row_bytes;
        let row1 = (y1 as usize) * row_bytes;
        let mask_row = (y as usize) * (out_w as usize);
        let out_row = mask_row * 4;
        for x in 0..out_w {
            let cov = mask[mask_row + x as usize];
            if cov == 0 {
                continue;
            }
            let px = x as f32 + 0.5;
            let u_norm = (px - dest_x) / dest_w;
            if !(0.0..=1.0).contains(&u_norm) {
                continue;
            }
            let sx = u_norm * src_wf - 0.5;
            let x0 = (sx.floor() as i32).clamp(0, max_sx);
            let x1 = (x0 + 1).min(max_sx);
            let fx = (sx - x0 as f32).clamp(0.0, 1.0);
            let i00 = row0 + (x0 as usize) * 4;
            let i10 = row0 + (x1 as usize) * 4;
            let i01 = row1 + (x0 as usize) * 4;
            let i11 = row1 + (x1 as usize) * 4;
            let w00 = (1.0 - fx) * (1.0 - fy);
            let w10 = fx * (1.0 - fy);
            let w01 = (1.0 - fx) * fy;
            let w11 = fx * fy;
            let dst = out_row + (x as usize) * 4;
            for c in 0..4 {
                let sample = f32::from(src[i00 + c]) * w00
                    + f32::from(src[i10 + c]) * w10
                    + f32::from(src[i01 + c]) * w01
                    + f32::from(src[i11 + c]) * w11;
                let rounded = sample.round().clamp(0.0, 255.0) as u32;
                let prod = rounded * u32::from(cov) + 128;
                out[dst + c] = ((prod + (prod >> 8)) >> 8) as u8;
            }
        }
    }
    out
}

/// The atlas handle one replaced node's content is cached under.
///
/// A node's own generation-checked name, which is already unique for the life of the document and
/// is reissued to nobody: content attached to a node that has been destroyed cannot be served to
/// whatever takes the slot over.
fn handle(id: ReplacedId) -> u64 {
    let key = id.node();
    u64::from(key.index()) << 32 | u64::from(key.generation().get())
}

#[cfg(test)]
mod tests {
    use zgui_geom::Size;

    use super::{Content, ImageError};

    /// A replaced identifier for a node of the first domain.
    fn id() -> zgui_dom::host::ReplacedId {
        use zgui_arena::{DomainId, Generation};
        zgui_dom::host::ReplacedId::new(zgui_dom::NodeKey::new(
            3,
            Generation::FIRST,
            DomainId::FIRST,
        ))
    }

    #[test]
    fn an_image_whose_bytes_do_not_match_its_extent_is_refused() {
        let refused = Content::decoded(id(), Size::new(2, 2), vec![0; 8]);
        assert_eq!(
            refused.unwrap_err(),
            ImageError::WrongByteCount {
                size: Size::new(2, 2),
                expected: 16,
                actual: 8,
            },
            "a short buffer would have the upload read past the end of a row"
        );
        assert!(Content::decoded(id(), Size::new(2, 2), vec![0; 16]).is_ok());
    }
}
