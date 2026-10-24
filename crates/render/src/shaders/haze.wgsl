// haze: hot air over the sand refracts the frame by a few pixels (SW143).
//
// The GPU twin of `post/haze.rs`: five octaves of value noise (8/16/32/64/128
// cells across the frame) from two counter-scrolling uv sets, one screen tap
// per output texel, and a depth mask `max(depth - MIN_DEPTH, 0)` so the air over
// the sand wobbles and nothing near the lens does. The shift is capped at
// MAX_SHIFT_PX at 1080p and scales with the frame height; a strength of 0 leaves
// the frame untouched and the pass is skipped by the caller.
//
// PORT(godotshaders.com/shader/heat-haze-shader by arlez80, MIT): `value_noise`,
// `noise`, `random` and `fragment` shapes, `max_shift` 2.5, `speed` 0.03 and
// `min_heathaze_dist` 0.99. Zero donor bytes; the octave table and the depth
// mask are this crate's.

const CELLS: array<u32, 5> = array<u32, 5>(8u, 16u, 32u, 64u, 128u);
const OCTAVE_WEIGHT: array<f32, 5> = array<f32, 5>(0.32, 0.26, 0.20, 0.14, 0.08);

/// Largest shift at the reference height, texels.
const MAX_SHIFT_PX: f32 = 2.5;
/// Frame height the cap is quoted at, texels.
const REFERENCE_H: f32 = 1080.0;
/// Shallowest depth that counts as hot air.
const MIN_DEPTH: f32 = 0.99;
/// Scroll of the two uv sets, screens per second.
const SPEED: f32 = 0.03;

struct HazeParams {
    seconds: f32,
    strength: f32,
    width: f32,
    height: f32,
}

@group(0) @binding(0) var frame_tex: texture_2d<f32>;
@group(0) @binding(1) var frame_sampler: sampler;
@group(0) @binding(2) var depth_tex: texture_depth_2d;
@group(0) @binding(3) var<uniform> params: HazeParams;

/// A hashed corner value in 0..1, from the cell and a salt.
fn corner(cell: vec2<i32>, salt: u32) -> f32 {
    var h = bitcast<u32>(cell.x) * 0x9E3779B1u
        + bitcast<u32>(cell.y) * 0x85EBCA6Bu
        + salt * 0xC2B2AE35u;
    h = h ^ (h >> 15u);
    h = h * 0x2545F491u;
    h = h ^ (h >> 13u);
    h = h * 0x27D4EB2Du;
    h = h ^ (h >> 16u);
    return f32((h >> 8u) & 0xffffu) / 65535.0;
}

fn smooth01(t: f32) -> f32 {
    return t * t * (3.0 - 2.0 * t);
}

/// One octave of value noise over `uv`, `cells` cells across.
fn value_noise(uv: vec2<f32>, cells: u32, salt: u32) -> f32 {
    let n = f32(cells);
    let f = uv * n;
    let base = floor(f);
    let cell = vec2<i32>(base);
    let within = f - base;
    let s = vec2<f32>(smooth01(within.x), smooth01(within.y));
    let west = mix(corner(cell, salt), corner(cell + vec2<i32>(1, 0), salt), s.x);
    let east = mix(
        corner(cell + vec2<i32>(0, 1), salt),
        corner(cell + vec2<i32>(1, 1), salt),
        s.x,
    );
    return mix(west, east, s.y);
}

/// The five-octave noise at `uv` and `seconds`, from two counter-scrolling uv sets: the first
/// drifts one way, the second the other. Returns both fields in 0..1, x from the first set and y
/// from the second.
fn octaves(uv: vec2<f32>, seconds: f32) -> vec2<f32> {
    let a = vec2<f32>(uv.x + seconds * SPEED, uv.y - seconds * SPEED * 0.5);
    let b = vec2<f32>(uv.x - seconds * SPEED * 0.5, uv.y + seconds * SPEED);
    var sum = vec2<f32>(0.0, 0.0);
    var salt = 1u;
    for (var index = 0u; index < 5u; index = index + 1u) {
        sum = sum + vec2<f32>(
            OCTAVE_WEIGHT[index] * value_noise(a, CELLS[index], salt),
            OCTAVE_WEIGHT[index] * value_noise(b, CELLS[index], salt),
        );
        salt = salt + 0x9E37u;
    }
    return clamp(sum, vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0));
}

@fragment
fn frag(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(params.width, params.height);
    let texel = vec2<i32>(uv * size);
    let depth = textureLoad(depth_tex, texel, 0);
    let mask = max(depth - MIN_DEPTH, 0.0) * clamp(params.strength, 0.0, 1.0);
    if (mask <= 0.0) {
        return textureSampleLevel(frame_tex, frame_sampler, uv, 0.0);
    }
    let field = octaves(uv, params.seconds);
    let cap = MAX_SHIFT_PX * (params.height / REFERENCE_H) * mask;
    let shift = (field - vec2<f32>(0.5, 0.5)) * 2.0 * cap;
    return textureSampleLevel(frame_tex, frame_sampler, uv + shift / size, 0.0);
}
