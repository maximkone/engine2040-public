// SW150 edge pass over the half-res mask, then the two separable blurs and the
// additive composite, one entry point each in the order the donor renders them.
// Port of three.js OutlinePass `_getEdgeDetectionMaterial`,
// `_getSeparableBlurMaterial` and `_getOverlayMaterial`. The edge colour comes
// from the mask's visibility channel and never from the frame, so a locked
// fighter behind the sand berm still reads as a dim warm outline.

struct EdgeGlobals {
    // Mask or edge target size in texels.
    tex_size: vec2<f32>,
    // Blur direction: (1, 0) across the row, (0, 1) down the column.
    direction: vec2<f32>,
    // Thickness kernel radius in texels, capped by the CPU plan.
    kernel_radius: f32,
    // Multiplier on the composite band, the donor's edgeStrength.
    edge_strength: f32,
    // Weight of the glow band in the composite, the donor's edgeGlow.
    edge_glow: f32,
    // Cosine breathing of both colours, the donor's pulsePeriod.
    pulse: f32,
}

@group(0) @binding(0) var<uniform> globals: EdgeGlobals;
@group(0) @binding(1) var mask_tex: texture_2d<f32>;
@group(0) @binding(2) var edge_tex: texture_2d<f32>;
@group(0) @binding(3) var edge_tex2: texture_2d<f32>;
@group(0) @binding(4) var edge_samp: sampler;

// The donor's colours: white in front, a dim warm ember behind the berm.
const VISIBLE_EDGE_COLOR: vec3<f32> = vec3(1.0, 1.0, 1.0);
const HIDDEN_EDGE_COLOR: vec3<f32> = vec3(0.1, 0.04, 0.02);
// Multiplier on the Sobel gradient; the donor's edgeStrength default.
override EDGE_STRENGTH: f32 = 3.0;
// Sobel taps out to this many texels; the donor's MAX_EDGE_THICKNESS.
const MAX_EDGE_THICKNESS: u32 = 4u;
// The donor's MAX_EDGE_GLOW: the icon glow kernel radius.
const MAX_EDGE_GLOW: i32 = 4;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// One full-screen triangle; uv row 0 is the top row of the target.
@vertex
fn vs_full(@builtin(vertex_index) index: u32) -> VsOut {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: VsOut;
    out.clip = vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(corner.x, 1.0 - corner.y);
    return out;
}

// The mask's red channel: 1 where a selected target was drawn.
fn mask_coverage(uv: vec2<f32>) -> f32 {
    return textureSampleLevel(mask_tex, edge_samp, uv, 0.0).r;
}

// The mask's green channel: 1 in front of the prefill, 0 behind it.
fn mask_visibility(uv: vec2<f32>) -> f32 {
    return textureSampleLevel(mask_tex, edge_samp, uv, 0.0).g;
}

// The 3x3 Sobel gradient of the mask coverage at `uv`, magnitude in coverage
// units per texel: a hard coverage step reads 1. Taps clamp to the mask's edge
// through the sampler, so the mask border never draws a false edge.
fn sobel_edge(uv: vec2<f32>) -> f32 {
    let texel = 1.0 / globals.tex_size;
    let left = uv.x - texel.x;
    let right = uv.x + texel.x;
    let up = uv.y - texel.y;
    let down = uv.y + texel.y;
    let tl = mask_coverage(vec2<f32>(left, up));
    let tc = mask_coverage(vec2<f32>(uv.x, up));
    let tr = mask_coverage(vec2<f32>(right, up));
    let ml = mask_coverage(vec2<f32>(left, uv.y));
    let mr = mask_coverage(vec2<f32>(right, uv.y));
    let bl = mask_coverage(vec2<f32>(left, down));
    let bc = mask_coverage(vec2<f32>(uv.x, down));
    let br = mask_coverage(vec2<f32>(right, down));
    let gx = (tr + 2.0 * mr + br) - (tl + 2.0 * ml + bl);
    let gy = (bl + 2.0 * bc + br) - (tl + 2.0 * tc + tr);
    // /4 makes a one-texel coverage step read 1; the strength is the
    // composite's, as in the donor.
    return length(vec2<f32>(gx, gy)) * 0.25;
}

// The least visibility over the 3x3 window the gradient reads: one neighbour
// behind the prefill makes the whole edge hidden, so the outline shows through
// the sand. The donor samples only the four central taps, which misses the
// diagonal neighbour a round silhouette's gradient comes from.
fn window_visibility(uv: vec2<f32>) -> f32 {
    let texel = 1.0 / globals.tex_size;
    var least = 1.0;
    for (var row = -1; row <= 1; row = row + 1) {
        for (var col = -1; col <= 1; col = col + 1) {
            let offset = vec2<f32>(f32(col), f32(row)) * texel;
            least = min(least, mask_visibility(uv + offset));
        }
    }
    return least;
}

@fragment
fn fs_edge(in: VsOut) -> @location(0) vec4<f32> {
    let magnitude = sobel_edge(in.uv);
    if (magnitude <= 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let base = select(HIDDEN_EDGE_COLOR, VISIBLE_EDGE_COLOR, window_visibility(in.uv) > 0.999);
    return vec4<f32>(base * magnitude, magnitude);
}

// The donor's gaussianPdf: one tap weight at `x` texels, sigma in texels.
fn gaussian_pdf(x: f32, sigma: f32) -> f32 {
    return 0.39894 * exp(-0.5 * x * x / (sigma * sigma)) / sigma;
}

// One separable gaussian blur pass, sigma = radius / 2, taps one texel apart
// out to the kernel radius. Every tap reads edge_tex, so X then Y repeated
// gives the square kernel.
@fragment
fn fs_blur(in: VsOut) -> @location(0) vec4<f32> {
    let sigma = max(globals.kernel_radius, 1.0) * 0.5;
    let step = globals.direction / globals.tex_size;
    var weight_sum = gaussian_pdf(0.0, sigma);
    var sum = textureSampleLevel(edge_tex, edge_samp, in.uv, 0.0) * weight_sum;
    let taps = i32(globals.kernel_radius);
    for (var i = 1; i <= MAX_EDGE_GLOW; i = i + 1) {
        if (i > taps) {
            break;
        }
        let w = 2.0 * gaussian_pdf(f32(i), sigma);
        let offset = step * (f32(i) / max(globals.kernel_radius, 1.0));
        sum = sum + (textureSampleLevel(edge_tex, edge_samp, in.uv + offset, 0.0)
            + textureSampleLevel(edge_tex, edge_samp, in.uv - offset, 0.0)) * w;
        weight_sum = weight_sum + w;
    }
    return sum / max(weight_sum, 1e-6);
}

// The additive composite: strength * pulse * (thickness + glow * edgeGlow),
// saturating per channel. The frame colour is not sampled here; the caller adds
// the result over the graded frame. A zero strength in the uniform means "use
// the compiled default", so the override below is the donor's edgeStrength.
@fragment
fn fs_composite(in: VsOut) -> @location(0) vec4<f32> {
    let thickness = textureSampleLevel(edge_tex, edge_samp, in.uv, 0.0);
    let glow = textureSampleLevel(edge_tex2, edge_samp, in.uv, 0.0);
    let band = max(thickness.rgb + glow.rgb * globals.edge_glow, vec3<f32>(0.0));
    let strength = select(EDGE_STRENGTH, globals.edge_strength, globals.edge_strength > 0.0);
    return vec4<f32>(clamp(band * strength * globals.pulse, vec3<f32>(0.0),
        vec3<f32>(1.0)), 1.0);
}
