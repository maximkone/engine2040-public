// impostor_bake.wgsl: the offline half of the octahedral impostors (RL64).
//
// `vs_mesh` / `fs_mesh` render one mesh part from one frame direction with an
// orthographic camera fitted to the bounding sphere (the view-projection is
// built on the CPU from `impostor::bake_basis`, the matrix the draw's quad
// shares) into two targets: world-space albedo (alpha = coverage) and the
// world normal (n * 0.5 + 0.5) with the tunic mask in alpha.
// `vs_blit` / `fs_blit` box-filter the 2x supersampled bake down into the
// atlas, so the stored colour, normal and mask are premultiplied by coverage.

struct FrameU {
    view_proj: mat4x4<f32>,
};

struct PartU {
    base_color: vec3<f32>,
    // 1 when the part's pixels take the instance tunic tint.
    mask: f32,
    // 1 when the part carries a base-colour texture.
    has_tex: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var<uniform> frame: FrameU;
@group(1) @binding(0) var<uniform> part: PartU;
@group(1) @binding(1) var base_map: texture_2d<f32>;
@group(1) @binding(2) var base_smp: sampler;

struct MeshIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct MeshOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn vs_mesh(in: MeshIn) -> MeshOut {
    var out: MeshOut;
    out.clip = frame.view_proj * vec4<f32>(in.pos, 1.0);
    out.normal = in.normal;
    out.uv = in.uv;
    return out;
}

struct BakeOut {
    @location(0) albedo: vec4<f32>,
    @location(1) normal: vec4<f32>,
};

@fragment
fn fs_mesh(in: MeshOut) -> BakeOut {
    var rgb = part.base_color;
    if (part.has_tex > 0.5) {
        rgb = rgb * textureSample(base_map, base_smp, in.uv).rgb;
    }
    var out: BakeOut;
    out.albedo = vec4<f32>(rgb, 1.0);
    let n = normalize(in.normal);
    out.normal = vec4<f32>(n * 0.5 + vec3<f32>(0.5), part.mask);
    return out;
}

// Downsample: one fullscreen triangle per target pair; the bilinear tap at
// the centre of a 2 by 2 block is exactly its box average.
@group(0) @binding(1) var hi_albedo: texture_2d<f32>;
@group(0) @binding(2) var hi_normal: texture_2d<f32>;
@group(0) @binding(3) var hi_smp: sampler;

struct BlitOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_blit(@builtin(vertex_index) vi: u32) -> BlitOut {
    // 0 -> (-1,-1) 1 -> (3,-1) 2 -> (-1,3), uv y down.
    let x = f32(i32(vi & 1u) * 4 - 1);
    let y = f32(i32(vi >> 1u) * 4 - 1);
    var out: BlitOut;
    out.clip = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

@fragment
fn fs_blit(in: BlitOut) -> BakeOut {
    var out: BakeOut;
    out.albedo = textureSampleLevel(hi_albedo, hi_smp, in.uv, 0.0);
    out.normal = textureSampleLevel(hi_normal, hi_smp, in.uv, 0.0);
    return out;
}
