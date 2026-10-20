// blit.wgsl: the last pass of the frame. Samples the linear HDR target into
// the LDR twin or the surface view, applies the opt-in probe composite (the
// fog factor and contact AO pair, both acting on linear radiance), then
// does the one tone map of the frame: hue-keeping display plus sRGB (tonemap.wgsl is
// prepended at compile time). The live window frame leaves the pair neutral
// (factor 0, occlusion 1): its fog is a per-fragment function of the
// forward shader. Row convention: one UV addresses the HDR, fog and AO
// texels alike. Clip +y is up while texture row 0 is the top row, so the
// vertex stage flips V: presented row 0 holds view-top.
// Two fragment entries: `fs` encodes sRGB itself (the LDR twin and any
// non-sRGB surface); `fs_hw_srgb` leaves the encode to the hardware (a
// `*Srgb` surface view encodes on write, so encoding here too would wash
// the frame out).
@group(0) @binding(0) var hdr_tex: texture_2d<f32>;
@group(0) @binding(1) var hdr_smp: sampler;
@group(0) @binding(2) var fog_tex: texture_2d<f32>;
@group(0) @binding(3) var ao_tex: texture_2d<f32>;
struct FogU {
    // Fog color as linear radiance (the sky horizon), same units as the HDR.
    color: vec3<f32>,
    width_f: f32,
    height_f: f32,
    // Bloom switch (LF4), 0 off.
    _pad0: f32,
    // Camera exposure as a linear multiplier (GS44): the meter's stops,
    // written per frame. 1.0 is neutral.
    exposure: f32,
    _pad2: f32,
};
@group(0) @binding(4) var<uniform> fog: FogU;
struct BOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};
@vertex
fn vs(@builtin(vertex_index) vi: u32) -> BOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0)
    );
    var out: BOut;
    out.pos = vec4<f32>(p[vi], 0.0, 1.0);
    out.uv = vec2<f32>(p[vi].x * 0.5 + 0.5, 0.5 - p[vi].y * 0.5);
    return out;
}
fn composite(in: BOut) -> vec4<f32> {
    let hdr = textureSample(hdr_tex, hdr_smp, in.uv);
    let dims = vec2<f32>(fog.width_f, fog.height_f);
    let px = vec2<i32>(clamp(in.uv * dims, vec2<f32>(0.0), dims - vec2<f32>(1.0)));
    let f = clamp(textureLoad(fog_tex, px, 0).x, 0.0, 1.0);
    // LF3: AO does not multiply the whole frame. The forward `fs` entry
    // already scales the baked diffuse by the ORM occlusion and contact, so
    // multiplying here darkened direct sun too (open sand lost its sun).
    // The live GTAO field stays declared for a later ambient-only composite.
    // One camera exposure for the whole frame, applied to linear radiance
    // before the one tone map (GS44). 1.0 leaves the frame exactly as it was.
    let lit = mix(hdr.rgb, fog.color, f) * fog.exposure;
    return vec4<f32>(lit, hdr.a);
}
// Single-pass bloom before the one tone map (LF4 slice 2). Every tap is
// soft-knee extracted first, then the extracts are blurred: a glint a pixel
// or two wide still haloes though its neighbours are dark, and a frame with
// no texel above the knee adds exactly zero. The knee curve is
// `post.rs` `kino_prefilter_weight`; threshold 1.0, knee 0.5, intensity 0.6
// mirror `BloomConfig::default`. The mip pyramid in `post.rs`
// `apply_bloom` stays the CPU oracle. Live flag rides `FogU::_pad0` (0.0
// off returns `c` unchanged, 1.0 on runs the halo); the live frame writes it
// per frame after forward.
const BLIT_BLOOM_THRESHOLD: f32 = 1.0;
const BLIT_BLOOM_KNEE: f32 = 0.5;
const BLIT_BLOOM_INTENSITY: f32 = 0.6;
// Per-channel cap on one tap before the knee, so a single hot texel cannot
// own the halo (the oracle weights its first mip for the same reason).
const BLIT_BLOOM_TAP_MAX: f32 = 16.0;
// Kernel weights, summing to 1: the centre texel, four axis taps (two texels
// each through the bilinear fetch) and four diagonal taps (four texels each).
// Together they cover every texel of the 5x5 block around the pixel.
const BLIT_BLOOM_W_CENTER: f32 = 0.28;
const BLIT_BLOOM_W_AXIS: f32 = 0.12;
const BLIT_BLOOM_W_DIAG: f32 = 0.06;
fn blit_bloom_extract(s: vec3<f32>) -> vec3<f32> {
    let rgb = clamp(s, vec3<f32>(0.0), vec3<f32>(BLIT_BLOOM_TAP_MAX));
    let br = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    let knee = BLIT_BLOOM_THRESHOLD * BLIT_BLOOM_KNEE + 0.00001;
    let foot = BLIT_BLOOM_THRESHOLD - knee;
    if (br <= foot) {
        return vec3<f32>(0.0);
    }
    if (br >= foot + 2.0 * knee) {
        return rgb;
    }
    let rq = clamp(br - foot, 0.0, 2.0 * knee);
    let soft = max(0.25 / knee * rq * rq, br - BLIT_BLOOM_THRESHOLD);
    return rgb * (soft / max(br, 0.00001));
}
fn blit_bloom_tap(uv: vec2<f32>, texel: vec2<f32>, off: vec2<f32>) -> vec3<f32> {
    return blit_bloom_extract(textureSampleLevel(hdr_tex, hdr_smp, uv + off * texel, 0.0).rgb);
}
fn blit_bloom(c: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    if (fog._pad0 < 0.5) {
        return c;
    }
    let texel = vec2<f32>(1.0 / max(fog.width_f, 1.0), 1.0 / max(fog.height_f, 1.0));
    var halo = blit_bloom_extract(c) * BLIT_BLOOM_W_CENTER;
    var axis = blit_bloom_tap(uv, texel, vec2<f32>(1.5, 0.0));
    axis += blit_bloom_tap(uv, texel, vec2<f32>(-1.5, 0.0));
    axis += blit_bloom_tap(uv, texel, vec2<f32>(0.0, 1.5));
    axis += blit_bloom_tap(uv, texel, vec2<f32>(0.0, -1.5));
    var diag = blit_bloom_tap(uv, texel, vec2<f32>(1.5, 1.5));
    diag += blit_bloom_tap(uv, texel, vec2<f32>(-1.5, 1.5));
    diag += blit_bloom_tap(uv, texel, vec2<f32>(1.5, -1.5));
    diag += blit_bloom_tap(uv, texel, vec2<f32>(-1.5, -1.5));
    halo += axis * BLIT_BLOOM_W_AXIS + diag * BLIT_BLOOM_W_DIAG;
    return c + halo * BLIT_BLOOM_INTENSITY;
}
// Display grade after the one tone map, before the sRGB encode (LF4 slice
// 1): contrast and saturation around mid-grey, then gain plus lift.
const BLIT_GRADE_MID: f32 = 0.18;
const BLIT_GRADE_CONTRAST: f32 = 1.08;
const BLIT_GRADE_SATURATION: f32 = 1.12;
const BLIT_GRADE_GAIN: vec3<f32> = vec3<f32>(1.03, 1.0, 0.98);
const BLIT_GRADE_LIFT: vec3<f32> = vec3<f32>(0.004, 0.002, 0.0);
fn blit_grade(c: vec3<f32>) -> vec3<f32> {
    var out = (c - vec3<f32>(BLIT_GRADE_MID)) * BLIT_GRADE_CONTRAST + vec3<f32>(BLIT_GRADE_MID);
    let luma = dot(out, vec3<f32>(0.2126, 0.7152, 0.0722));
    let s = clamp(BLIT_GRADE_SATURATION, 0.0, 4.0);
    out = vec3<f32>(luma) + (out - vec3<f32>(luma)) * s;
    out = out * BLIT_GRADE_GAIN + BLIT_GRADE_LIFT;
    return max(out, vec3<f32>(0.0));
}
@fragment
fn fs(in: BOut) -> @location(0) vec4<f32> {
    let lit = composite(in);
    let bloomed = blit_bloom(lit.rgb, in.uv);
    let toned = display_tonemap(bloomed);
    let graded = blit_grade(toned);
    let px = vec2<u32>(u32(in.pos.x), u32(in.pos.y));
    let dithered = graded + vec3<f32>(dither4(px.x, px.y));
    return vec4<f32>(linear_to_srgb_term(dithered), lit.a);
}
@fragment
fn fs_hw_srgb(in: BOut) -> @location(0) vec4<f32> {
    let lit = composite(in);
    let bloomed = blit_bloom(lit.rgb, in.uv);
    let toned = display_tonemap(bloomed);
    let graded = blit_grade(toned);
    let px = vec2<u32>(u32(in.pos.x), u32(in.pos.y));
    return vec4<f32>(graded + vec3<f32>(dither4(px.x, px.y)), lit.a);
}
