//! Standalone textures for streamed frames, independent of the sprite atlas.

use crate::{Gpu, WgpuRenderer};
use zgui_geom::{Device, Size};
use zgui_render::{ExternalTexture, Renderer, TextureHandle};
use zgui_scene::ExternalTextureId;

#[cfg(all(target_os = "linux", vulkan_hal))]
mod nv12;
#[cfg(all(target_os = "linux", vulkan_hal))]
use crate::ImportedFrame;
#[cfg(all(target_os = "linux", vulkan_hal))]
pub(crate) use nv12::Nv12Pipelines;
#[cfg(all(target_os = "linux", vulkan_hal))]
pub use nv12::Nv12Target;

/// A renderer-local binding for a streamed frame.
#[derive(Debug)]
pub struct FrameTexture {
    texture: wgpu::Texture,
    handle: TextureHandle,
    #[cfg(all(target_os = "linux", vulkan_hal))]
    _imported: Option<ImportedFrame>,
}

impl FrameTexture {
    /// The texture's current pixel extent.
    pub fn size(&self) -> Size<u32, Device> {
        Size::new(self.texture.width(), self.texture.height())
    }
}

impl WgpuRenderer {
    /// Uploads tightly packed straight-alpha RGBA8, reusing the binding at the same extent.
    ///
    /// `slot` belongs to this renderer and `id` for its entire lifetime. Returns false for
    /// unrepresentable dimensions or an incorrect byte count without changing the slot.
    pub fn write_frame(
        &mut self,
        id: ExternalTextureId,
        slot: &mut Option<FrameTexture>,
        size: Size<u32, Device>,
        bytes: &[u8],
    ) -> bool {
        let gpu = self.gpu();
        let limit = gpu.device().limits().max_texture_dimension_2d;
        if size.width == 0
            || size.height == 0
            || size.width > limit
            || size.height > limit
            || u64::from(size.width) * u64::from(size.height) * 4 != bytes.len() as u64
        {
            return false;
        }
        if slot.as_ref().is_none_or(|slot| {
            slot.size() != size || !slot.texture.usage().contains(wgpu::TextureUsages::COPY_DST)
        }) {
            let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
                label: Some("zgui.software-frame"),
                size: extent(size),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            if let Some(previous) = slot.take() {
                self.remove_frame(previous);
            }
            *slot = Some(self.bind_frame(
                id,
                texture,
                #[cfg(all(target_os = "linux", vulkan_hal))]
                None,
            ));
        }
        let slot = slot
            .as_ref()
            .expect("the validated frame has a texture slot");
        upload(self.gpu(), &slot.texture, size, bytes);
        true
    }

    /// Detaches a frame texture and returns its binding to the renderer.
    pub fn remove_frame(&mut self, slot: FrameTexture) {
        self.release_external(slot.handle);
    }

    pub(crate) fn bind_frame(
        &mut self,
        id: ExternalTextureId,
        texture: wgpu::Texture,
        #[cfg(all(target_os = "linux", vulkan_hal))] imported: Option<ImportedFrame>,
    ) -> FrameTexture {
        let handle = self.register_external(ExternalTexture {
            id,
            handle: TextureHandle(0),
            size: Size::new(texture.width() as i32, texture.height() as i32),
            premultiplied: false,
        });
        self.attach_external(id, &texture);
        FrameTexture {
            texture,
            handle,
            #[cfg(all(target_os = "linux", vulkan_hal))]
            _imported: imported,
        }
    }
}

pub(crate) fn extent(size: Size<u32, Device>) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: size.width,
        height: size.height,
        depth_or_array_layers: 1,
    }
}

fn upload(gpu: &Gpu, texture: &wgpu::Texture, size: Size<u32, Device>, bytes: &[u8]) {
    gpu.queue().write_texture(
        texture.as_image_copy(),
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size.width * 4),
            rows_per_image: Some(size.height),
        },
        extent(size),
    );
}
