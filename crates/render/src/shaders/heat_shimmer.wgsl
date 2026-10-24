// heat_shimmer.wgsl: the half-res heat ripple over the brazier and the hot-sand
// patches (QA9).
//
// Owner: render lane. Presentation only, never hashed. Units: texels for the
// grid and the offsets, 0..1 for the mask share.
//
// One entry: `shimmer_fs` reads the full frame, builds the soft mask from the
// heat sources, nudges its uv by two out-of-phase sines scaled by the mask and
// the strength (clamped to `CAP_PX`), and writes the resampled texel. The pass
// runs at half resolution and is composited bilinearly over the frame before
// exposure, exactly where `heat_shimmer.rs` `apply` does it on the CPU.
//
// The CPU parity oracle is `crates/render/src/heat_shimmer.rs`
// (`mask_at`, `shimmer_offset_uv`): same falloff, same phase, same cap.
//
// Determinism: `phase` is the caller's frame number, never a clock.

override CAP_PX: f32 = 2.0;
override STRENGTH_PX: f32 = 1.5;
override PHASE_STEPS: f32 = 9.0;

struct ShimmerU {
    // Full frame size in texels.
    frame_w: u32,
    frame_h: u32,
    // Half-res grid size in texels.
    half_w: u32,
    half_h: u32,
    // Frame phase as the caller's frame number, taken modulo 1024.
    phase: u32,
    // Pad to the 16-byte uniform struct size (three spare words).
    pad0: u32,
    pad1: u32,
    pad2: u32,
};

struct Source {
    // Centre in half-res texels.
    center: vec2<f32>,
    // Soft radius in half-res texels.
    radius: f32,
    // 1 on a live slot, 0 on an empty one.
    live: f32,
};

struct SourcesU {
    // The brazier plus the hot-sand patches.
    items: array<Source, 4>,
};

@group(0) @binding(0) var<uniform> shine: ShimmerU;
@group(0) @binding(1) var<uniform> heat: SourcesU;
@group(0) @binding(2) var frame_tex: texture_2d<f32>;
@group(0) @binding(3) var frame_smp: sampler;

// Soft share at one half-res texel: smoothstep down to 0 at the radius, the
// strongest source wins, NaN reads 0.
fn mask_at(px: vec2<f32>) -> f32 {
    var best = 0.0;
    for (var i = 0u; i < 4u; i = i + 1u) {
        let s = heat.items[i];
        if (s.live < 0.5) { continue; }
        let t = clamp(1.0 - distance(px, s.center) / max(s.radius, 1e-4), 0.0, 1.0);
        best = max(best, t * t * (3.0 - 2.0 * t));
    }
    return best;
}

// Offset in uv: two out-of-phase sines, scaled by the mask and the strength,
// then clamped to the cap so a wild source cannot tear the frame.
fn offset_uv(px: vec2<f32>, mask: f32) -> vec2<f32> {
    if (mask <= 0.0 || STRENGTH_PX <= 0.0) { return vec2<f32>(0.0); }
    let phase = f32(shine.phase % 1024u) * PHASE_STEPS;
    let strength = min(STRENGTH_PX, CAP_PX);
    let wob = vec2<f32>(
        sin(px.x * 0.35 + phase),
        cos(px.y * 0.27 + phase * 1.31) * 0.7,
    );
    let texel = vec2<f32>(0.5 / f32(max(shine.half_w, 1u)), 0.5 / f32(max(shine.half_h, 1u)));
    return wob * strength * mask * texel;
}

@fragment
fn shimmer_fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    // Half-res texel -> the frame uv it stands for.
    let px = floor(at.xy);
    let mask = mask_at(px);
    let half_uv = (px + vec2<f32>(0.5)) / vec2<f32>(f32(shine.half_w), f32(shine.half_h));
    let uv = clamp(half_uv + offset_uv(px, mask), vec2<f32>(0.0), vec2<f32>(1.0));
    if (mask <= 0.0) { return vec4<f32>(0.0); }
    return textureSampleLevel(frame_tex, frame_smp, uv, 0.0);
}
