// forward_maps.wgsl: EG12 staged chunk — ambient, torches, normal and ORM maps.
//
// Staged, not yet in the frame: no pipeline concatenates this file today.
// `present/gpu_present/forward_bind.rs` holds it as FORWARD_MAPS_WGSL and
// pins the markers below. The slice-2 bind uses group 0 bindings 16-19,
// free in shaders/forward.wgsl today (0-15 taken), so this chunk declares
// them here and nowhere else. The BRDF lives once in forward.wgsl: this
// file never redefines it (torch terms reuse Lambert only).

// One torch point light: world pos in meters, warm linear color, finite
// range in meters (zero at and past the edge, capped near the center).
struct Eg12Torch {
    pos: vec3<f32>,
    _pad0: f32,
    color: vec3<f32>,
    range: f32,
}
// Frame torches: 8 slots, live count first, rest zeroed. Same 272-byte
// image as `TorchBlock` in forward.wgsl (8 x 32 B slots plus the count).
struct Eg12TorchBlock {
    torches: array<Eg12Torch, 8>,
    count: u32,
    _pad1: u32,
    _pad2: u32,
    _pad3: u32,
}
// ORM texel split: occlusion in R, perceptual roughness in G, metalness in B.
struct Eg12Orm {
    occlusion: f32,
    roughness: f32,
    metallic: f32,
}

// Sand and stone maps: tangent-space normals plus packed ORM, one shared
// Repeat sampler (tiling readers, never Clamp).
@group(0) @binding(16) var eg12_normal_map: texture_2d<f32>;
@group(0) @binding(17) var eg12_orm_map: texture_2d<f32>;
@group(0) @binding(18) var eg12_map_sampler: sampler;

// Live torch frame at 19, last of 16-19: 8 warm slots plus count, staged
// per frame from the placement torch actors (west plus east heads, 12 m
// range, live count first, rest zeroed). Lambert only, no BRDF copy.
@group(0) @binding(19) var<uniform> eg12_torches: Eg12TorchBlock;

// Baked ambient scaled by occlusion: AO dims the ambient term only, never
// the direct sun or the final color (pipeline rule 4). `irradiance` is the
// SH diffuse term from forward.wgsl (`ibl_irradiance`); occlusion is the
// frame AO in 0..=1 (1 open, 0 fully shadowed).
fn eg12_ambient(irradiance: vec3<f32>, occlusion: f32) -> vec3<f32> {
    return irradiance * clamp(occlusion, 0.0, 1.0);
}

// One torch falloff: inverse-square capped near the center, squared range
// window to zero at the edge. Zero past the edge or on a dead range.
fn eg12_torch_falloff(dist: f32, range: f32) -> f32 {
    if (range <= 0.0 || dist >= range) {
        return 0.0;
    }
    let att = 1.0 / (1.0 + dist * dist);
    let edge = 1.0 - dist / range;
    return att * edge * edge;
}

// All live torches over one fragment: Lambert diffuse under the falloff
// above, summed over the bound count only. Backfaces stay unlit.
fn eg12_torch_sum(
    world_pos: vec3<f32>,
    geom_n: vec3<f32>,
    albedo: vec3<f32>,
    block: Eg12TorchBlock
) -> vec3<f32> {
    let n = normalize(geom_n);
    var out = vec3<f32>(0.0);
    for (var i = 0u; i < 8u; i = i + 1u) {
        if (i >= block.count) {
            break;
        }
        let torch = block.torches[i];
        let delta = torch.pos - world_pos;
        let dist = length(delta);
        let ndot_l_in = dot(n, delta / max(dist, 0.0001));
        if (torch.range <= 0.0 || dist >= torch.range || ndot_l_in <= 0.0) {
            continue;
        }
        let l = delta / max(dist, 0.0001);
        let ndot_l = clamp(dot(n, l), 0.0, 1.0);
        out += albedo * ndot_l * torch.color * eg12_torch_falloff(dist, torch.range);
    }
    return out;
}

// Fill one torch slot from caller values: the CPU `from_list` mirror for
// staging a single actor light into the uniform array.
fn eg12_make_torch(pos: vec3<f32>, color: vec3<f32>, range: f32) -> Eg12Torch {
    var torch: Eg12Torch;
    torch.pos = pos;
    torch.color = color;
    torch.range = range;
    return torch;
}

// Read one torch back from the block with a bounds guard: out-of-range
// indices return a dark slot, never an out-of-bounds read. Mirrors the CPU
// `unpack_slot` read-back for the pack/unpack parity test.
fn eg12_torch_at(block: Eg12TorchBlock, index: u32) -> Eg12Torch {
    if (index >= 8u || index >= block.count) {
        var dark: Eg12Torch;
        dark.pos = vec3<f32>(0.0);
        dark.color = vec3<f32>(0.0);
        dark.range = 0.0;
        return dark;
    }
    return block.torches[index];
}

// Packed ORM split: R occlusion, G perceptual roughness, B metalness.
fn eg12_split_orm(texel: vec3<f32>) -> Eg12Orm {
    var orm: Eg12Orm;
    orm.occlusion = clamp(texel.x, 0.0, 1.0);
    orm.roughness = clamp(texel.y, 0.0, 1.0);
    orm.metallic = clamp(texel.z, 0.0, 1.0);
    return orm;
}

// Tangent-space normal to [-1, 1]: guarded normalize, zero length refuses
// with the geometric normal so a flat texel never writes NaN.
fn eg12_decode_normal(texel: vec3<f32>, geom_n: vec3<f32>) -> vec3<f32> {
    let v = texel * 2.0 - vec3<f32>(1.0);
    let l2 = dot(v, v);
    if (l2 < 1e-12) {
        return normalize(geom_n);
    }
    return v / sqrt(l2);
}

// Mapped world normal: orthonormalize the interpolated tangent against the
// geometric normal, handedness from tangent.w, bitangent = w * (n x t).
// The two maps blend as slopes by the caller, never averaged raw.
fn eg12_world_normal(
    geom_n: vec3<f32>,
    tangent: vec4<f32>,
    map_n: vec3<f32>
) -> vec3<f32> {
    let n = normalize(geom_n);
    let tx = tangent.xyz - n * dot(tangent.xyz, n);
    let tl2 = dot(tx, tx);
    // A degenerate tangent keeps the geometric normal: no NaN downstream.
    if (tl2 < 1e-12) {
        return n;
    }
    let t = tx / sqrt(tl2);
    let w = select(-1.0, 1.0, tangent.w >= 0.0);
    let b = w * cross(n, t);
    let out = t * map_n.x + b * map_n.y + n * map_n.z;
    let ol2 = dot(out, out);
    if (ol2 < 1e-12) {
        return n;
    }
    return out / sqrt(ol2);
}

// Sand and stone sample: Repeat-sampled normal plus ORM at one UV. One
// call site per fragment; the caller feeds `normal` into
// `eg12_world_normal` and scales `eg12_ambient` by `orm.occlusion`.
struct Eg12Maps {
    normal_texel: vec3<f32>,
    orm: Eg12Orm,
}
fn eg12_sample_maps(uv: vec2<f32>) -> Eg12Maps {
    var maps: Eg12Maps;
    maps.normal_texel = textureSample(eg12_normal_map, eg12_map_sampler, uv).rgb;
    let orm_texel = textureSample(eg12_orm_map, eg12_map_sampler, uv).rgb;
    maps.orm = eg12_split_orm(orm_texel);
    return maps;
}
