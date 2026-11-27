//! What one rectangle showing a texture the renderer did not draw is told.

use bytemuck::{Pod, Zeroable};
use zgui_render::ExternalTexture;
use zgui_scene::ExternalQuad;

/// The block one external quad reads.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ExternalParams {
    /// The quad, in device pixels: origin then extent.
    pub bounds: [f32; 4],
    /// The clip chain, the transform, the opacity, and whether the texture is already
    /// premultiplied.
    pub control: [f32; 4],
    /// The mask's local destination: origin then extent.
    pub mask_box: [f32; 4],
    /// The mask's atlas rectangle: origin then extent, in texels.
    pub mask_tile: [f32; 4],
}

impl ExternalParams {
    /// The block for `quad` showing `texture`.
    pub fn of(quad: &ExternalQuad, texture: &ExternalTexture) -> Self {
        Self {
            bounds: [
                quad.bounds.origin.x.0,
                quad.bounds.origin.y.0,
                quad.bounds.size.width.0,
                quad.bounds.size.height.0,
            ],
            control: [
                quad.clip.0 as f32,
                quad.transform.index() as f32,
                quad.opacity.clamp(0.0, 1.0),
                f32::from(texture.premultiplied),
            ],
            mask_box: [0.0; 4],
            mask_tile: [0.0; 4],
        }
    }

    /// Samples the coverage tile at `rect`, in the quad's local space.
    pub fn mask(
        &mut self,
        analytic_clip: zgui_scene::ClipId,
        rect: zgui_geom::Rect<zgui_geom::DevicePx, zgui_geom::Device>,
        tile: zgui_atlas::AtlasTile,
    ) {
        self.control[0] = analytic_clip.0 as f32;
        self.mask_box = [
            rect.origin.x.0,
            rect.origin.y.0,
            rect.size.width.0,
            rect.size.height.0,
        ];
        let bounds = tile.bounds;
        self.mask_tile = [
            bounds.origin.x as f32,
            bounds.origin.y as f32,
            bounds.size.width as f32,
            bounds.size.height as f32,
        ];
    }
}

#[cfg(test)]
mod tests {
    use super::ExternalParams;
    use zgui_geom::{Device, DevicePx, Point, Rect, Size};
    use zgui_render::{ExternalTexture, TextureHandle};
    use zgui_scene::{ExternalQuad, ExternalTextureId};

    fn texture(premultiplied: bool) -> ExternalTexture {
        ExternalTexture {
            id: ExternalTextureId(1),
            handle: TextureHandle(1),
            size: Size::new(8, 8),
            premultiplied,
        }
    }

    #[test]
    fn a_texture_that_is_not_premultiplied_says_so_where_the_shader_can_see_it() {
        let bounds: Rect<DevicePx, Device> = Rect::new(
            Point::new(DevicePx(0.0), DevicePx(0.0)),
            Size::new(DevicePx(8.0), DevicePx(8.0)),
        );
        let quad = ExternalQuad::new(bounds, ExternalTextureId(1));
        assert_eq!(ExternalParams::of(&quad, &texture(false)).control[3], 0.0);
        assert_eq!(ExternalParams::of(&quad, &texture(true)).control[3], 1.0);
    }
}
