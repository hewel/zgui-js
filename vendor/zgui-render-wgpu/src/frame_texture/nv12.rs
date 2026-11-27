//! GPU conversion of studio-range NV12 planes into the renderer's RGB external texture.

use std::sync::{Arc, Weak};

use crate::Gpu;

#[derive(Debug)]
pub(crate) struct Nv12Pipelines([wgpu::RenderPipeline; 2]);

const SHADER: &str = r#"
@group(0) @binding(0) var luma: texture_2d<f32>;
@group(0) @binding(1) var chroma: texture_2d<f32>;

@vertex fn vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0, 0.0, 1.0);
}

fn convert(p: vec4<f32>, coefficients: vec4<f32>) -> vec4<f32> {
    let ij = vec2<i32>(p.xy);
    let y = (round(textureLoad(luma, ij, 0).r * 255.0) - 16.0) * 298.0;
    let uv = round(textureLoad(chroma, ij / 2, 0).rg * 255.0) - vec2<f32>(128.0);
    let rgb = vec3<f32>(y + coefficients.x * uv.y,
                        y - coefficients.y * uv.x - coefficients.z * uv.y,
                        y + coefficients.w * uv.x);
    return vec4<f32>(clamp(floor((rgb + 128.0) / 256.0) / 255.0, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}

@fragment fn bt601(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return convert(p, vec4<f32>(409.0, 100.0, 208.0, 516.0));
}

@fragment fn bt709(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return convert(p, vec4<f32>(459.0, 55.0, 136.0, 541.0));
}
"#;

/// A stream's reusable RGB render target, replaced on extent or device changes.
#[derive(Debug, Default)]
pub struct Nv12Target(Option<Target>);

#[derive(Debug)]
struct Target {
    gpu: Weak<Gpu>,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Nv12Target {
    pub(crate) fn convert(
        &mut self,
        gpu: &Arc<Gpu>,
        source: &wgpu::Texture,
        bt709: bool,
    ) -> wgpu::Texture {
        let size = source.size();
        if self.0.as_ref().is_none_or(|target| {
            target.texture.size() != size || target.gpu.as_ptr() != Arc::as_ptr(gpu)
        }) {
            let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
                label: Some("zgui.nv12.rgb"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            self.0 = Some(Target {
                gpu: Arc::downgrade(gpu),
                texture,
                view,
            });
        }
        let target = self.0.as_ref().expect("NV12 target allocated");
        convert(gpu, source, &target.view, bt709);
        target.texture.clone()
    }
}

fn convert(gpu: &Gpu, texture: &wgpu::Texture, target: &wgpu::TextureView, bt709: bool) {
    let device = gpu.device();
    let Nv12Pipelines(pipelines) = gpu.nv12_pipelines.get_or_init(|| {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("zgui.nv12"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        Nv12Pipelines(["bt601", "bt709"].map(|name| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("zgui.nv12"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(name),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
                }),
                multiview_mask: None,
                cache: None,
            })
        }))
    });
    let pipeline = &pipelines[usize::from(bt709)];
    let [y, uv] = [wgpu::TextureAspect::Plane0, wgpu::TextureAspect::Plane1].map(|aspect| {
        texture.create_view(&wgpu::TextureViewDescriptor {
            aspect,
            ..Default::default()
        })
    });
    let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("zgui.nv12.planes"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&y),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&uv),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("zgui.nv12.convert"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bindings, &[]);
        pass.draw(0..3, 0..1);
    }
    gpu.queue().submit([encoder.finish()]);
}
