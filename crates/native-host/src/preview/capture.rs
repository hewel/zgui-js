use std::{cell::RefCell, rc::Rc, time::Duration};

use zgui::{
    atlas::TextureSink,
    bits::DamageSet,
    render::{self, FrameOutcome, RenderTarget, Renderer},
    scene::Scene,
};
use zgui_render_wgpu::{Builder, Pixels, WgpuRenderer, wgpu};

use super::Error;

/// A readback belongs to a successful presentation, not merely the latest draw attempt.
pub struct PresentedPixels {
    pub pixels: Pixels,
    pub draw_id: u64,
    pub presentation_id: u64,
    pub configuration_id: u64,
    pub target: RenderTarget,
}

#[derive(Clone, Copy)]
pub struct CaptureState {
    pub draw_id: u64,
    pub presentation_id: u64,
    pub configuration_id: u64,
    pub presented_draw: Option<u64>,
}

#[derive(Default)]
struct State {
    requested: bool,
    draw_id: u64,
    presentation_id: u64,
    configuration_id: u64,
    last_outcome: Option<FrameOutcome>,
    presented_draw: Option<u64>,
    target: Option<RenderTarget>,
    device_identity: usize,
    pixels: Option<PresentedPixels>,
    metadata: serde_json::Value,
}

/// UI-thread evidence sink. Requesting pixels never requests a redraw or advances time.
#[derive(Clone, Default)]
pub struct CaptureHandle(Rc<RefCell<State>>);

impl CaptureHandle {
    /// Release all presentation ownership before a native document is replaced.
    /// Sequence counters stay monotonic; a new renderer must present before readback is valid.
    pub fn retire(&self) {
        let mut state = self.0.borrow_mut();
        state.requested = false;
        state.last_outcome = None;
        state.presented_draw = None;
        state.target = None;
        state.device_identity = 0;
        state.pixels = None;
        state.metadata = serde_json::Value::Null;
    }
    pub fn request(&self) {
        let mut state = self.0.borrow_mut();
        state.requested = true;
        state.pixels = None;
    }

    pub fn take(&self) -> Option<PresentedPixels> {
        self.0.borrow_mut().pixels.take()
    }

    pub fn last_outcome(&self) -> Option<FrameOutcome> {
        self.0.borrow().last_outcome
    }

    pub fn metadata(&self) -> serde_json::Value {
        self.0.borrow().metadata.clone()
    }

    pub fn state(&self) -> CaptureState {
        let state = self.0.borrow();
        CaptureState {
            draw_id: state.draw_id, presentation_id: state.presentation_id,
            configuration_id: state.configuration_id, presented_draw: state.presented_draw,
        }
    }

    /// Reads an already-presented idle frame without drawing it again. The caller must supply
    /// this handle's window renderer; its forwarded backend identity is checked before reading.
    pub fn read_current(&self, renderer: &dyn Renderer) -> Result<PresentedPixels, Error> {
        let inner = renderer.as_any().and_then(|any| any.downcast_ref::<WgpuRenderer>())
            .ok_or_else(|| capture_error("the window has no offscreen wgpu renderer"))?;
        let state = self.0.borrow();
        let draw_id = state.presented_draw
            .ok_or_else(|| capture_error("no frame has been presented at the current configuration"))?;
        if inner.gpu().as_ref() as *const _ as usize != state.device_identity {
            return Err(capture_error("the renderer does not belong to this capture handle"));
        }
        let target = state.target.ok_or_else(|| capture_error("the target is not configured"))?;
        let pixels = inner.read_presented().ok_or_else(|| capture_error("the presented target is not readable"))?;
        Ok(PresentedPixels {
            pixels, draw_id, presentation_id: state.presentation_id,
            configuration_id: state.configuration_id, target,
        })
    }
}

pub fn renderer_factory(handle: CaptureHandle) -> zgui::runtime::RendererFactory {
    Box::new(move |_surface, target| {
        // Runtime owns the extent, scale and opacity policy. Do not reconstruct this target
        // from surface dimensions: those may still describe the preceding configure.
        let mut inner = Builder::new().offscreen(target, wgpu::TextureFormat::Bgra8Unorm, false)
            .map_err(zgui::runtime::AppError::GpuUnavailable)?;
        zgui_render_vector_vello::attach(&mut inner, target.size);
        let mut renderer = CaptureRenderer { inner, handle: handle.clone() };
        renderer.update_metadata();
        {
            let mut state = renderer.handle.0.borrow_mut();
            state.target = Some(target);
            state.configuration_id += 1;
            state.presented_draw = None;
        }
        Ok(Box::new(renderer) as Box<dyn Renderer>)
    })
}

struct CaptureRenderer {
    inner: WgpuRenderer,
    handle: CaptureHandle,
}

impl CaptureRenderer {
    fn update_metadata(&mut self) {
        let info = self.inner.gpu().adapter().get_info();
        let formats = self.inner.formats();
        let mut state = self.handle.0.borrow_mut();
        state.device_identity = self.inner.gpu().as_ref() as *const _ as usize;
        state.metadata = serde_json::json!({
            "adapter": info.name,
            "backend": format!("{:?}", info.backend),
            "deviceType": format!("{:?}", info.device_type),
            "vendor": info.vendor,
            "device": info.device,
            "driver": info.driver,
            "driverInfo": info.driver_info,
            "targetFormat": format!("{:?}", formats.surface),
            "sceneFormat": format!("{:?}", formats.scene),
            "scratchFormat": format!("{:?}", formats.scratch),
            "srgbTier": format!("{:?}", formats.tier),
            "alphaMode": format!("{:?}", formats.alpha_mode),
            "storedColor": "premultiplied-gamma-encoded",
            "pngColor": "straight-alpha-gamma-encoded",
            "conversionRevision": 1,
            "additionalTransferConversion": false,
            "compositorTransparencyValidated": false,
        });
    }

    fn observe(&mut self, outcome: FrameOutcome) {
        if matches!(outcome, FrameOutcome::Recovered) {
            self.update_metadata();
        }
        let mut state = self.handle.0.borrow_mut();
        state.draw_id += 1;
        state.last_outcome = Some(outcome);
        if matches!(outcome, FrameOutcome::Recovered) {
            state.presented_draw = None;
            state.pixels = None;
        }
        if matches!(outcome, FrameOutcome::Presented(_)) {
            state.presentation_id += 1;
            state.presented_draw = Some(state.draw_id);
            if state.requested {
                state.requested = false;
                state.pixels = self.inner.read_presented().map(|pixels| PresentedPixels {
                    pixels,
                    draw_id: state.draw_id,
                    presentation_id: state.presentation_id,
                    configuration_id: state.configuration_id,
                    target: state.target.expect("a presented renderer has a target"),
                });
            }
        }
    }
}

impl Renderer for CaptureRenderer {
    fn capabilities(&self) -> render::RenderCapabilities { self.inner.capabilities() }
    fn configure(&mut self, target: RenderTarget) {
        self.inner.configure(target);
        let mut state = self.handle.0.borrow_mut();
        state.target = Some(target);
        state.configuration_id += 1;
        state.presented_draw = None;
        state.pixels = None;
    }
    fn target(&self) -> Option<RenderTarget> { self.inner.target() }
    fn draw(&mut self, scene: &Scene, damage: &DamageSet) -> FrameOutcome {
        let outcome = self.inner.draw(scene, damage);
        self.observe(outcome);
        outcome
    }
    fn shifts_composed_pixels(&self) -> bool { self.inner.shifts_composed_pixels() }
    fn composed_survives_configure(&self) -> bool { self.inner.composed_survives_configure() }
    fn shift_composed(&mut self, shift: render::ScrollShift) { self.inner.shift_composed(shift); }
    fn register_external(&mut self, texture: render::ExternalTexture) -> render::TextureHandle { self.inner.register_external(texture) }
    fn release_external(&mut self, handle: render::TextureHandle) { self.inner.release_external(handle); }
    fn memory(&self) -> render::MemoryReport { self.inner.memory() }
    fn vector_status(&self) -> render::VectorStatus { self.inner.vector_status() }
    fn target_pool(&self) -> render::TargetPoolReport { self.inner.target_pool() }
    fn release_cached_targets(&mut self) -> u64 { self.inner.release_cached_targets() }
    fn release_idle_resources(&mut self) -> u64 { self.inner.release_idle_resources() }
    fn acquire_block(&self) -> Duration { self.inner.acquire_block() }
    fn present_held(&mut self) -> FrameOutcome {
        let outcome = self.inner.present_held();
        self.observe(outcome);
        outcome
    }
    fn texture_sink(&mut self) -> &mut dyn TextureSink { self.inner.texture_sink() }
    fn as_any_mut(&mut self) -> Option<&mut dyn core::any::Any> { self.inner.as_any_mut() }
    fn as_any(&self) -> Option<&dyn core::any::Any> { self.inner.as_any() }
}

pub fn rgba(pixels: &Pixels, opaque: bool) -> Result<Vec<u8>, Error> {
    let size = pixels.size();
    let width = u32::try_from(size.width).map_err(|_| capture_error("invalid readback width"))?;
    let height = u32::try_from(size.height).map_err(|_| capture_error("invalid readback height"))?;
    convert(pixels.bytes(), width, height, pixels.is_bgra(), opaque)
}

fn convert(bytes: &[u8], width: u32, height: u32, bgra: bool, opaque: bool) -> Result<Vec<u8>, Error> {
    let length = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(4))
        .filter(|_| width > 0 && height > 0)
        .ok_or_else(|| capture_error("invalid readback extent"))?;
    if bytes.len() != length {
        return Err(capture_error("readback rows are not tightly packed at the reported extent"));
    }
    let mut output = Vec::with_capacity(length);
    for pixel in bytes.chunks_exact(4) {
        let [r, g, b, a] = if bgra { [pixel[2], pixel[1], pixel[0], pixel[3]] } else { [pixel[0], pixel[1], pixel[2], pixel[3]] };
        if opaque && a != 255 {
            return Err(capture_error("opaque capture contains a pixel whose alpha is not 255"));
        }
        // Integer round-to-nearest preserves exact opaque values and handles low-alpha
        // quantization. RGB at alpha zero is deliberately canonicalized to black.
        let straight = |channel: u8| -> u8 {
            if a == 0 { 0 } else { ((u32::from(channel) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8 }
        };
        output.extend_from_slice(&[straight(r), straight(g), straight(b), a]);
    }
    Ok(output)
}

pub fn png(pixels: &Pixels, opaque: bool) -> Result<Vec<u8>, Error> {
    let rgba = rgba(pixels, opaque)?;
    let size = pixels.size();
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut output, size.width as u32, size.height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|error| capture_error(error.to_string()))?;
        writer.write_image_data(&rgba).map_err(|error| capture_error(error.to_string()))?;
        writer.finish().map_err(|error| capture_error(error.to_string()))?;
    }
    Ok(output)
}

fn capture_error(message: impl Into<String>) -> Error { Error::new("E_CAPTURE", 7, message) }

#[cfg(test)]
mod tests {
    use super::convert;

    #[test]
    fn an_empty_backend_selection_really_has_no_adapter() {
        let target = zgui::render::RenderTarget::new(
            zgui::geom::Size::new(257, 193),
            zgui::geom::Scale::new(1.0),
        );
        assert!(zgui_render_wgpu::Builder::with_backends(zgui_render_wgpu::wgpu::Backends::empty())
            .offscreen(target, zgui_render_wgpu::wgpu::TextureFormat::Bgra8Unorm, false)
            .is_err());
    }

    #[test]
    fn packed_odd_width_rows_keep_channels_and_orientation() {
        for width in [1_u32, 3, 65, 257] {
            let mut bytes = vec![0; width as usize * 2 * 4];
            for (index, pixel) in bytes.chunks_exact_mut(4).enumerate() {
                pixel.copy_from_slice(&[(index % 251) as u8, (index / width as usize) as u8, 93, 255]);
            }
            let rgba = convert(&bytes, width, 2, true, true).unwrap();
            for (index, pixel) in rgba.chunks_exact(4).enumerate() {
                assert_eq!(pixel, [93, (index / width as usize) as u8, (index % 251) as u8, 255]);
            }
        }
        assert!(convert(&[0; 32], 3, 2, true, false).is_err());
    }

    #[test]
    fn straight_alpha_handles_zero_low_and_opaque_alpha_without_transfer_conversion() {
        let bytes = [99, 2, 1, 0, 1, 0, 1, 1, 64, 32, 128, 128, 128, 128, 128, 255];
        assert_eq!(convert(&bytes, 4, 1, false, false).unwrap(), [0, 0, 0, 0, 255, 0, 255, 1, 128, 64, 255, 128, 128, 128, 128, 255]);
        assert!(convert(&bytes, 4, 1, false, true).is_err());
        assert!(convert(&[], 0, 1, false, false).is_err());
    }
}
