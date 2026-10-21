// lens: the bodycam lens in one pass. Barrel distortion (k1), a radial
// red/blue split (chromatic aberration), a vignette and film grain.
//
// The steps and their order are the CPU oracle's in `lens.rs`: the barrel
// moves the place the pass reads, the split reads red and blue beside that
// place, the vignette darkens the corners and the grain adds the same noise
// to every channel. The uniform carries the four knobs and the frame index
// that seeds the grain, so the same frame index gives the same grain twice.

struct LensParams {
    // frame width, frame height, frame index, unused.
    dims: vec4<u32>,
    // barrel k1, chromatic aberration, vignette, grain.
    knobs: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: LensParams;
@group(0) @binding(1) var<storage, read> source: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> lens_out: array<vec4<f32>>;

const CENTER: vec2<f32> = vec2<f32>(0.5, 0.5);
const CORNER_LEN: f32 = 0.70710678;

// The barrel map: the uv the pass reads for one output uv. A positive k1
// pulls the source toward the frame center, so the corner of the picture
// moves inward.
fn barrel_uv(uv: vec2<f32>, k1: f32) -> vec2<f32> {
    let c = uv * 2.0 - 1.0;
    let f = 1.0 + k1 * dot(c, c);
    return c * f * 0.5 + 0.5;
}

// The chromatic aberration shift: the red and the blue sample sit this far
// apart along the radial direction.
fn ca_shift(uv: vec2<f32>, amount: f32) -> vec2<f32> {
    let d = uv - CENTER;
    let len = length(d);
    if (len < 1e-6) {
        return vec2<f32>(0.0, 0.0);
    }
    let k = 0.5 * amount / len;
    return d * k;
}

// The vignette factor: 1 at the center, `1 - strength` at the corner.
fn vignette_at(uv: vec2<f32>, strength: f32) -> f32 {
    let d = length(uv - CENTER) / CORNER_LEN;
    return clamp(1.0 - strength * d * d, 0.0, 1.0);
}

// The grain noise: an integer hash of (x, y, frame index), so the same frame
// index gives the same noise twice and no backend drifts. The lanes are 16
// bits wide, so every multiply stays inside u32.
fn grain_at(x: u32, y: u32, frame: u32) -> f32 {
    var h: u32 = (x & 0xffffu) * 0x9e37u;
    h = h ^ ((y & 0xffffu) * 0x85ebu);
    h = h ^ ((frame & 0xffffu) * 0xc2b2u);
    h = h ^ ((frame >> 16u) * 0x2545u);
    h = h ^ (h >> 16u);
    h = (h & 0xffffu) * 0x2545u;
    h = h ^ (h >> 13u);
    h = (h & 0xffffu) * 0x1f35u;
    h = h ^ (h >> 11u);
    return f32(h >> 16u) * (1.0 / 65536.0) - 0.5;
}

// One texel, clamped to the frame edge.
fn fetch(ix: i32, iy: i32) -> vec3<f32> {
    let w = i32(params.dims.x);
    let h = i32(params.dims.y);
    let cx = u32(clamp(ix, 0, w - 1));
    let cy = u32(clamp(iy, 0, h - 1));
    return source[cy * params.dims.x + cx].rgb;
}

// The color at one uv: bilinear over the four neighbouring texels.
fn sample_at(uv: vec2<f32>) -> vec3<f32> {
    let size = vec2<f32>(f32(params.dims.x), f32(params.dims.y));
    let t = uv * size - 0.5;
    let base = floor(t);
    let frac = t - base;
    let x0 = i32(base.x);
    let y0 = i32(base.y);
    let top = mix(fetch(x0, y0), fetch(x0 + 1, y0), frac.x);
    let bottom = mix(fetch(x0, y0 + 1), fetch(x0 + 1, y0 + 1), frac.x);
    return mix(top, bottom, frac.y);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = params.dims.x;
    let h = params.dims.y;
    if (gid.x >= w || gid.y >= h) {
        return;
    }
    let uv = (vec2<f32>(f32(gid.x), f32(gid.y)) + 0.5) / vec2<f32>(f32(w), f32(h));
    let base = barrel_uv(uv, params.knobs.x);
    let shift = ca_shift(base, params.knobs.y);
    let red = sample_at(base + shift);
    let green = sample_at(base);
    let blue = sample_at(base - shift);
    var color = vec3<f32>(red.r, green.g, blue.b);
    color = color * vignette_at(uv, params.knobs.z);
    let noise = grain_at(gid.x, gid.y, params.dims.z) * params.knobs.w;
    color = color + vec3<f32>(noise);
    let index = gid.y * w + gid.x;
    lens_out[index] = vec4<f32>(clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
