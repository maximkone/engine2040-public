// crowd_vat.wgsl: instanced VAT crowd plus far-row impostor branch.
//
// Owner: render lane. Units: meters, frames and verts as indices.
// Determinism: CPU staging is the authority; this shader is presentation
// only. Budget: far rows cost one quad, not one body.
// PORT(Mefodei/Render-Crowd-Of-Animated-Characters@HEAD-verified-2026-10-15:
// baked position texture with per-instance frame offset and phase;
// reimplemented against crates/assets/src/vat.rs, zero bytes vendored).
// PORT(wojtekpil/Godot-Octahedral-Impostors@HEAD-verified-2026-10-15:
// octahedral bake plus single-plane sampling; same terms, reimplemented).

struct CameraU {
    view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    _pad0: f32,
};
struct VatU {
    box_min_m: vec3<f32>,
    _pad0: f32,
    box_ext_m: vec3<f32>,
    frames: u32,
    verts: u32,
    _pad1: u32,
    _pad2: u32,
    _pad3: u32,
};
struct CrowdIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) packed0: vec4<u32>,
    @location(4) packed1: u32,
};
struct CrowdOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) tint: u32,
    @location(2) @interpolate(flat) cell: u32,
};

@group(0) @binding(0) var<uniform> cam: CameraU;
@group(0) @binding(1) var<uniform> vat: VatU;
@group(0) @binding(2) var vat_tex: texture_2d<f32>;
@group(0) @binding(3) var impostor_atlas: texture_2d<f32>;
@group(0) @binding(4) var vat_smp: sampler;

fn vat_frame_of(phase_permille: u32, tick: u32) -> u32 {
    let frames = max(vat.frames, 1u);
    return (tick + phase_permille % 1000u) % frames;
}

fn vat_pos(frame: u32, vert: u32) -> vec3<f32> {
    let w = max(vat.verts, 1u);
    let x = i32(vert % w);
    let y = i32(frame);
    let texel = textureLoad(vat_tex, vec2<i32>(x, y), 0).rgb;
    return vat.box_min_m + texel * vat.box_ext_m;
}

fn impostor_uv(cell: u32, uv: vec2<f32>) -> vec2<f32> {
    let edge = 4.0;
    let col = f32(cell % 4u);
    let row = f32((cell / 4u) % 4u);
    return (vec2<f32>(col, row) + uv) / edge;
}

@vertex
fn vs_crowd_vat(in: CrowdIn, @builtin(instance_index) ii: u32, @builtin(vertex_index) vi: u32) -> CrowdOut {
    var out: CrowdOut;
    let lod = (in.packed0.w >> 24u) & 255u;
    let tint = (in.packed0.w >> 18u) & 63u;
    let phase = in.packed0.w & 1023u;
    let tick = in.packed1 & 65535u;
    let is_far = lod >= 2u;
    var world = in.pos;
    var uv = in.uv;
    var cell = 0u;
    if (is_far) {
        cell = (tint * 7u + phase) % 16u;
        uv = impostor_uv(cell, in.uv);
    } else {
        let frame = vat_frame_of(phase, tick);
        let baked = vat_pos(frame, vi);
        world = in.pos + baked * 0.15;
    }
    out.clip = cam.view_proj * vec4<f32>(world, 1.0);
    out.uv = uv;
    out.tint = tint;
    out.cell = cell;
    _ = ii;
    return out;
}

@fragment
fn fs_crowd(in: CrowdOut) -> @location(0) vec4<f32> {
    let tint_f = f32(in.tint) / 63.0;
    let base = textureSample(impostor_atlas, vat_smp, in.uv).rgb;
    let shade = vec3<f32>(0.75 + 0.5 * tint_f, 0.72 + 0.4 * tint_f, 0.68 + 0.3 * tint_f);
    return vec4<f32>(base * shade, 1.0);
}
