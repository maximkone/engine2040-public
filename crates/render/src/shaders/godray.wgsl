// godray.wgsl: the quarter-res sun-shaft pass (SW91), two entries.
//
// Owner: render lane. Presentation only, never hashed. Units: texels for the
// mask, 0..1 shares, linear light for the added colour.
//
// `mask_fs` bright-passes one quarter-res texel: a frame luma over
// `MASK_THRESHOLD` becomes a share, everything else 0. `ray_fs` sums
// `NUM_SAMPLES` taps from the pixel toward the sun's mask position and adds the
// result over the frame after fog, before tonemap.
//
// The CPU parity oracle is `crates/render/src/godray.rs` (`sum_along_ray`,
// `mask_share`): same loop, same field order, so a frame checks headless.
// Donor shape: GPU Gems 3 ch. 13 Listing 13-1 (`deltaTexCoord *=
// 1.0/NUM_SAMPLES * Density`, `illuminationDecay *= Decay`, `sample *=
// illuminationDecay * Weight`, `return color * Exposure`).
//
// One pipeline per tier through `@override`: potato 8 taps, the rest 16.

override NUM_SAMPLES: u32 = 8u;
override DENSITY: f32 = 1.0;
override DECAY: f32 = 0.96;
override WEIGHT: f32 = 0.5;
override EXPOSURE: f32 = 1.0;
override MASK_THRESHOLD: f32 = 0.75;

struct RayU {
    // Sun position in mask texels.
    sun_px: vec2<f32>,
    // Mask size in texels, and whether the pass records at all.
    mask_w: u32,
    mask_h: u32,
    runs: u32,
    _pad: u32,
};

@group(0) @binding(0) var<uniform> ray: RayU;
@group(0) @binding(1) var mask_tex: texture_2d<f32>;
@group(0) @binding(2) var hdr_tex: texture_2d<f32>;

// Bright-pass share of one luma: 0 at or under the threshold, 1 at white.
fn mask_share(luma: f32) -> f32 {
    if (luma <= 0.0 || !(luma == luma)) { return 0.0; }
    let span = max(1.0 - MASK_THRESHOLD, 1e-5);
    return clamp((luma - MASK_THRESHOLD) / span, 0.0, 1.0);
}

// Perceived luma of one linear RGB texel.
fn luma_of(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

@fragment
fn mask_fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    // One quarter-res texel stands for a 4x4 block: read its centre.
    let px = vec2<i32>(at.xy) * 4 + vec2<i32>(2, 2);
    let dims = vec2<i32>(textureDimensions(hdr_tex));
    let clamped = clamp(px, vec2<i32>(0), max(dims - vec2<i32>(1), vec2<i32>(0)));
    let c = textureLoad(hdr_tex, clamped, 0).rgb;
    let share = mask_share(luma_of(c));
    return vec4<f32>(share, share, share, 1.0);
}

// One mask texel, clamped to the grid (a ray leaves the frame as empty air).
fn mask_tap(p: vec2<i32>) -> f32 {
    let dims = vec2<i32>(i32(ray.mask_w), i32(ray.mask_h));
    if (any(p < vec2<i32>(0)) || any(p >= dims)) { return 0.0; }
    return textureLoad(mask_tex, p, 0).r;
}

@fragment
fn ray_fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    if (ray.runs == 0u || NUM_SAMPLES == 0u) { return vec4<f32>(0.0); }
    let step = (at.xy - ray.sun_px) * (1.0 / f32(NUM_SAMPLES)) * DENSITY;
    var coord = at.xy;
    var decay = 1.0;
    var sum = 0.0;
    for (var i = 0u; i < NUM_SAMPLES; i = i + 1u) {
        coord = coord - step;
        let share = mask_tap(vec2<i32>(round(coord)));
        sum = sum + share * decay * WEIGHT;
        decay = decay * DECAY;
    }
    let add = sum * EXPOSURE;
    return vec4<f32>(vec3<f32>(add), 0.0);
}
