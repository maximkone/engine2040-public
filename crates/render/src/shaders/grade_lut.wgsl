// grade_lut.wgsl: the film grade as one 16-cube colour LUT (SW152).
//
// Compiled beside `shaders/tonemap.wgsl` (it takes the linear->sRGB encode from
// there), bound with the display stage's 3D sampler and applied after the tone
// map, before the 8-bit store. The CPU twin is `crate::grade_lut`: the same
// trilinear read over 4096 three-byte entries and the same 4x4 dither.
//
// Why a LUT: dust, torch flame and blood are three hand-written constants
// today; one cube puts them in one palette and drops the shader to one texture
// fetch. The neutral cube is built from the display stage's own PBR Neutral
// curve, so an ungraded frame keeps its bytes.

@group(2) @binding(0) var grade_lut_tex: texture_3d<f32>;
@group(2) @binding(1) var grade_lut_smp: sampler;

// Half an 8-bit step each way: the LUT stage's small offset, which cannot
// invert a one-unit ramp (`crate::grade_lut::dither_4x4`).
const DITHER_4X4_HALF_STEP: f32 = 0.5 / 255.0;

// The Bayer 4x4 cell index for one pixel, 0..15.
fn bayer_cell(px: vec2<f32>) -> u32 {
    let x = u32(px.x) & 3u;
    let y = u32(px.y) & 3u;
    return y * 4u + x;
}

// A +0..1 dither value in `-0.5..0.5` of the cell's order, scaled to a step.
fn dither_8bit(px: vec2<f32>) -> f32 {
    var m: u32 = 0u;
    switch (bayer_cell(px)) {
        case 0u: { m = 0u; }
        case 1u: { m = 8u; }
        case 2u: { m = 2u; }
        case 3u: { m = 10u; }
        case 4u: { m = 12u; }
        case 5u: { m = 4u; }
        case 6u: { m = 14u; }
        case 7u: { m = 6u; }
        case 8u: { m = 3u; }
        case 9u: { m = 11u; }
        case 10u: { m = 1u; }
        case 11u: { m = 9u; }
        case 12u: { m = 15u; }
        case 13u: { m = 7u; }
        case 14u: { m = 13u; }
        default: { m = 5u; }
    }
    return ((f32(m) + 0.5) / 16.0 - 0.5) * DITHER_4X4_HALF_STEP * 2.0;
}

// One trilinear read of the cube: display `0..1` in, display `0..1` out.
fn grade_lut_sample(rgb: vec3<f32>) -> vec3<f32> {
    let uv = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    // `textureSampleLevel` in a fragment-free helper: the LUT has no mips, so
    // level 0 is the only level and the read needs no derivatives.
    return textureSampleLevel(grade_lut_tex, grade_lut_smp, uv, 0.0).rgb;
}

// The graded store value: tone-mapped display colour in, dithered sRGB out.
fn grade_lut_store(display: vec3<f32>, px: vec2<f32>) -> vec3<f32> {
    let graded = grade_lut_sample(display);
    let encoded = linear_to_srgb_term(graded);
    let step = dither_8bit(px);
    return clamp(encoded + vec3<f32>(step), vec3<f32>(0.0), vec3<f32>(1.0));
}
