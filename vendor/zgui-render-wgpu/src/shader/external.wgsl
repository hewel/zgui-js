// External quads sample producer pixels in place. Only CSS shape coverage lives in the atlas.
struct ExternalParams {
    bounds: vec4<f32>,
    // Clip chain, transform, opacity, and whether source colours are premultiplied.
    control: vec4<f32>,
    mask_box: vec4<f32>,
    mask_tile: vec4<f32>,
}

@group(1) @binding(0) var<uniform> foreign: ExternalParams;
@group(1) @binding(1) var external_texture: texture_2d<f32>;
@group(1) @binding(2) var external_sampler: sampler;
@group(2) @binding(0) var mask_texture: texture_2d<f32>;

struct ExternalVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_external(@builtin(vertex_index) vertex: u32) -> ExternalVarying {
    let corner = unit_corner(vertex);
    let device = foreign.bounds.xy + corner * foreign.bounds.zw;
    var out: ExternalVarying;
    out.position = to_clip_position(device, u32(foreign.control.y));
    out.uv = corner;
    return out;
}

fn shade_external(in: ExternalVarying, shape: f32) -> vec4<f32> {
    let coverage = shape * clip_coverage(device_position(in.position.xy), u32(foreign.control.x));
    if coverage <= 0.0 {
        return vec4<f32>(0.0);
    }
    var texel = textureSampleLevel(external_texture, external_sampler, in.uv, 0.0);
    if foreign.control.w == 0.0 {
        texel = vec4<f32>(texel.rgb * texel.a, texel.a);
    }
    return texel * coverage * foreign.control.z;
}

@fragment
fn fs_external(in: ExternalVarying) -> @location(0) vec4<f32> {
    return shade_external(in, 1.0);
}

@fragment
fn fs_external_masked(in: ExternalVarying) -> @location(0) vec4<f32> {
    let local = foreign.bounds.xy + in.uv * foreign.bounds.zw;
    let uv = (local - foreign.mask_box.xy) / foreign.mask_box.zw;
    let texel = clamp(floor(uv * foreign.mask_tile.zw), vec2<f32>(0.0), foreign.mask_tile.zw - 1.0);
    let coverage = textureLoad(mask_texture, vec2<i32>(foreign.mask_tile.xy + texel), 0).r;
    return shade_external(in, coverage);
}
