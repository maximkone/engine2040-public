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
// The dual-filter bloom halo (RL36): half the frame size, written by the
// compute chain before this pass, sampled at the pixel centre.
@group(0) @binding(5) var bloom_tex: texture_2d<f32>;
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
// Dual-filter bloom before the one tone map (RL36). The halo is built by the
// compute chain in `shaders/post/bloom_dual_*.wgsl`: a Karis-weighted bright
// mip at half the frame size, a 13-tap downsample ladder and a 3x3 tent
// upsample ladder, so a glint a pixel wide spreads over many rungs while a
// frame with no texel above the knee adds exactly zero. This pass samples
// rung 0 at the pixel centre and adds it at the Kino intensity on linear
// radiance; both sides share one exposure, so the halo rides the same meter
// as the frame. `BLIT_BLOOM_INTENSITY` mirrors `BloomConfig::default`
// (threshold 1.0, knee 0.5, intensity 0.6). Live flag rides `FogU::_pad0`
// (0.0 off returns `c` unchanged, 1.0 on adds the halo); the live frame
// writes it per frame, the probe path keeps it 0.
const BLIT_BLOOM_INTENSITY: f32 = 0.6;
fn blit_bloom(c: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    if (fog._pad0 < 0.5) {
        return c;
    }
    let halo = textureSampleLevel(bloom_tex, hdr_smp, uv, 0.0).rgb;
    return c + halo * (BLIT_BLOOM_INTENSITY * fog.exposure);
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
// Live entries (GS34): the window frame leaves the fog pair neutral (factor
// 0), and mix(hdr, color, 0) is exactly hdr, so the composite is hdr times
// exposure and the fog texel is never read. A caller picks these only while
// the pair is neutral; on those frames they write the same bytes as `fs` and
// `fs_hw_srgb`. The rest of the chain is shared with those entries.
fn live_lit(in: BOut) -> vec4<f32> {
    let hdr = textureSample(hdr_tex, hdr_smp, in.uv);
    return vec4<f32>(hdr.rgb * fog.exposure, hdr.a);
}
@fragment
fn fs_live(in: BOut) -> @location(0) vec4<f32> {
    let lit = live_lit(in);
    let bloomed = blit_bloom(lit.rgb, in.uv);
    let toned = display_tonemap(bloomed);
    let graded = blit_grade(toned);
    let px = vec2<u32>(u32(in.pos.x), u32(in.pos.y));
    let dithered = graded + vec3<f32>(dither4(px.x, px.y));
    return vec4<f32>(linear_to_srgb_term(dithered), lit.a);
}
@fragment
fn fs_hw_srgb_live(in: BOut) -> @location(0) vec4<f32> {
    let lit = live_lit(in);
    let bloomed = blit_bloom(lit.rgb, in.uv);
    let toned = display_tonemap(bloomed);
    let graded = blit_grade(toned);
    let px = vec2<u32>(u32(in.pos.x), u32(in.pos.y));
    return vec4<f32>(graded + vec3<f32>(dither4(px.x, px.y)), lit.a);
}
