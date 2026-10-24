// tonemap.wgsl: the display transform, one copy for every pass that ends a
// frame. Linear HDR radiance in, display value out: one hue-keeping tone
// map, then the sRGB transfer (found2 5.3, gems1 15.4). The window frame
// runs it once, in the blit, never in the forward fragment. Prepended
// (compile-time concat) to forward.wgsl, to blit.wgsl and to the two post
// shaders, so this text exists once.
//
// One source, one curve: the tone map below is the crate's only ACES
// definition, and every consumer (the live blit, the offscreen capture, the
// post chain and its compute twin) calls it. A torch flame keeps its hue
// through it: the fit is evaluated at the peak and the result is one scale
// for all three channels, so the input ratios survive.
// PORT(BabylonJS/Babylon.js@136feacc96b011d98f26413edef252e098c846fd:packages/dev/core/src/ShadersWGSL/tonemap.fragment.fx): ACES shape, reimplemented.
// PORT(BabylonJS/Babylon.js@136feacc96b011d98f26413edef252e098c846fd:packages/dev/core/src/ShadersWGSL/imageProcessing.fragment.fx): grade shape, reimplemented.
// Zero donor bytes vendored in this file.

// Display white for the hue-keeping path, same units as the post white.
const DISPLAY_WHITE: f32 = 11.2;
// Display exposure anchor: the arena rig lights the sand and the stands
// about 0.6 stops above the display mid, so the fit maps both onto its flat
// shoulder and the floor/stands value split collapses (measured on the
// 3c2d7b1a capture: sand luma 0.70, one stand tier luma 0.79). Pulling the
// display radiance back onto the steep part of the fit restores the split;
// the camera, the bodies, the crowd and the sky are untouched.
const DISPLAY_EXPOSURE: f32 = 0.6;
// Highlight contrast knee: display level at or below the knee keeps the
// plain fit value (the dim sky band and the black level), above it the one
// hue-keeping scale grows by DISPLAY_KNEE_GAIN per unit of display level,
// so the lit floor and wall base separate in value. Measured on the
// 44fd1088 capture (2026-10-17, floor window x 380-900 y 600-700, tier
// window x 80-330 y 80-150, every second pixel): at gain 0.30 the sand
// floors at luma 156.6 and the stone tiers at 188.0 - a 32-code split that
// reads as one tan family. Both sit above the knee, so the stone (toned
// peak 0.66, knee excess 0.31) takes 2.5x the sand's lift (toned peak
// 0.45, excess 0.10); gain 0.80 widens the split to 164 / 204 (40 codes)
// and leaves the sky band (toned peak 0.21, below the knee) untouched.
const DISPLAY_KNEE: f32 = 0.35;
const DISPLAY_KNEE_GAIN: f32 = 0.80;
// The knee is a display look, not part of the tone map: the post chain and
// the capture path carry no knee, so it lives beside the tone map and the
// blit applies it as its own step. One scale for every channel, so the hue
// survives it too.
fn display_knee(toned: vec3<f32>) -> vec3<f32> {
    let peak = max(toned.x, max(toned.y, toned.z));
    if (peak <= 1e-6) {
        return toned;
    }
    let knee = max(peak - DISPLAY_KNEE, 0.0);
    let lifted = peak + knee * DISPLAY_KNEE_GAIN;
    return toned * (lifted / peak);
}
// Stone drain: above DISPLAY_DRAIN_KNEE the display colour drains toward its
// own luma, so the lit stands read pale travertine instead of the sand's warm
// tan; at or below the knee the hue is kept, so the sand stays warm ochre.
// Measured on the 3c2d7b1a recheck captures (2026-10-17, identical frame-0
// camera): sand peak channel 0.48 linear (byte 186, R-B 70), one stand tier
// 0.74..0.79 (byte 223..230, R-B 89) - the two share one hue ratio, so value
// alone cannot separate the materials and the drain knee sits between their
// peaks. The mix is luma-preserving in linear light: it moves chroma only,
// never the value the knee lifted, and the dim sky band (peak 0.11) stays
// untouched.
const DISPLAY_DRAIN_KNEE: f32 = 0.55;
const DISPLAY_DRAIN_SPAN: f32 = 0.20;
const DISPLAY_DRAIN: f32 = 0.60;
fn stone_drain(d: vec3<f32>) -> vec3<f32> {
    let level = max(d.x, max(d.y, d.z));
    let t = clamp((level - DISPLAY_DRAIN_KNEE) / DISPLAY_DRAIN_SPAN, 0.0, 1.0);
    let luma = vec3<f32>(dot(d, vec3<f32>(0.2126, 0.7152, 0.0722)));
    return mix(d, luma, DISPLAY_DRAIN * t);
}
// Per-channel compressive fit, denominator guarded so finite input stays finite.
fn fit_one(x: f32) -> f32 {
    let v = max(x, 0.0);
    let num = v * (2.51 * v + 0.03);
    let den = max(v * (2.43 * v + 0.59) + 0.14, 1e-4);
    return clamp(num / den, 0.0, 1.0);
}
// Analytic log-poly fit for the selectable branch, finite for finite input.
fn analytic_one(x: f32) -> f32 {
    let v = max(x, 0.0);
    let ev = log2(v + 0.001953125);
    let t = clamp((ev + 6.0) / 8.0, 0.0, 1.0);
    let s = t * t * (3.0 - 2.0 * t);
    return clamp(s, 0.0, 1.0);
}
// Pipeline-selectable branch: false keeps the default hue-keeping path.
override DISPLAY_ANALYTIC: bool = false;
// Pipeline-selectable curve: true swaps the ACES fit for Khronos PBR Neutral
// (below), the same exposure, knee and drain after it, so a capture pair
// differs in the curve alone. false keeps the ACES fit.
override DISPLAY_NEUTRAL: bool = false;
// The one tone map of the crate: hue-keeping by construction. The neutral
// fit is evaluated at the peak of the frame, the white point normalises it,
// and the result is a single scale applied to all three channels. A per
// channel fit would compress the channels by different amounts and turn a
// torch flame's orange into a different hue (measured +32.3 degrees on the
// old post-chain copy); one scale keeps every ratio, so the hue survives.
// The exposure rides the peak and the white point is the display white the
// peak is normalised against. The knee is NOT here: it is the blit's own
// display look, so this curve is the same for every consumer.
fn aces_tonemap(color: vec3<f32>, exposure: f32, white: f32) -> vec3<f32> {
    let safe = max(color, vec3<f32>(0.0));
    let capped = min(safe, vec3<f32>(1e6));
    let peak = max(capped.x, max(capped.y, capped.z));
    let w = max(fit_one(white), 1e-4);
    let toned_peak = fit_one(peak * exposure) / w;
    return capped * (toned_peak / max(peak, 1e-6));
}
// The hue-keeping default branch of the one tone map: the same one-scale
// shape as `aces_tonemap`, at the live display's own exposure and white.
fn hue_keep_display(color: vec3<f32>) -> vec3<f32> {
    return aces_tonemap(color, DISPLAY_EXPOSURE, DISPLAY_WHITE);
}
// Analytic display: same one-scale shape over the analytic curve.
fn analytic_display(color: vec3<f32>) -> vec3<f32> {
    let safe = max(color, vec3<f32>(0.0));
    let capped = min(safe, vec3<f32>(1e6));
    let peak = max(capped.x, max(capped.y, capped.z));
    let white = max(analytic_one(DISPLAY_WHITE), 1e-4);
    let toned_peak = analytic_one(peak * DISPLAY_EXPOSURE) / white;
    return capped * (toned_peak / max(peak, 1e-6));
}
// Khronos PBR Neutral (the commerce neutral curve): a subtractive black
// offset, then a highlight shoulder from NEUTRAL_START that eases the peak
// toward display white and desaturates it toward white. The offset is the same
// for every channel and the shoulder is one scale, so the hue survives below
// the shoulder. Output stays in 0..1 for any finite input.
const NEUTRAL_START: f32 = 0.76;
const NEUTRAL_DESAT: f32 = 0.15;
fn pbr_neutral(color: vec3<f32>) -> vec3<f32> {
    let x = min(color.x, min(color.y, color.z));
    var offset = 0.04;
    if (x < 0.08) {
        offset = x - 6.25 * x * x;
    }
    var c = color - vec3<f32>(offset);
    let peak = max(c.x, max(c.y, c.z));
    if (peak < NEUTRAL_START) {
        return c;
    }
    let d = 1.0 - NEUTRAL_START;
    let new_peak = 1.0 - d * d / (peak + d - NEUTRAL_START);
    c = c * (new_peak / peak);
    let g = 1.0 - 1.0 / (NEUTRAL_DESAT * (peak - new_peak) + 1.0);
    return mix(c, vec3<f32>(new_peak), g);
}
// The Neutral option at the display's own exposure, the ACES fit's twin.
fn neutral_display(color: vec3<f32>) -> vec3<f32> {
    let safe = max(color, vec3<f32>(0.0));
    let capped = min(safe, vec3<f32>(1e6));
    return pbr_neutral(capped * DISPLAY_EXPOSURE);
}
// SW50: the AgX arm, beside the ACES and PBR Neutral curves. A power toe under
// the 0.18 display crossover, then a Reinhard-like shoulder that eases the top
// of the range toward white instead of clipping it, per channel: the sand at
// 4.0 lands near 0.9 display and keeps more chroma than the ACES fit. The CPU
// twin is `crate::tonemap::agx`.
override DISPLAY_AGX: bool = false;
const AGX_SLOPE: f32 = 5.30;
const AGX_SHOULDER_MAX: f32 = 0.1815;
const AGX_CROSSOVER: f32 = 0.18;
const AGX_TOE_POWER: f32 = 1.35;
fn agx_curve(x: f32) -> f32 {
    let v = max(x, 0.0);
    if (v < AGX_CROSSOVER) {
        return AGX_CROSSOVER * pow(v / AGX_CROSSOVER, AGX_TOE_POWER);
    }
    let num = AGX_SLOPE * v * (1.0 + v);
    let den = 1.0 + AGX_SLOPE * v / AGX_SHOULDER_MAX;
    return clamp(num / den, 0.0, 1.0);
}
fn agx_display(color: vec3<f32>) -> vec3<f32> {
    let capped = min(max(color, vec3<f32>(0.0)), vec3<f32>(1e6));
    return vec3<f32>(agx_curve(capped.x), agx_curve(capped.y), agx_curve(capped.z));
}
// Radial grade vignette, applied to the display value after the tone map and
// before the grade: 1 inside GRADE_VIGNETTE_RADIUS, falling to
// 1 - GRADE_VIGNETTE_STRENGTH at the corners. r2 is the squared distance from
// the frame centre, scaled so a corner is 1, and the falloff is a smoothstep
// from RADIUS^2 to 1. One multiply per pixel on all three channels; strength 0
// is the identity. RADIUS is a fraction of the centre-to-corner distance, 0..1.
const GRADE_VIGNETTE_STRENGTH: f32 = 0.20;
const GRADE_VIGNETTE_RADIUS: f32 = 0.60;
fn grade_vignette(c: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    if (GRADE_VIGNETTE_STRENGTH <= 0.0) {
        return c;
    }
    let d = uv - vec2<f32>(0.5);
    let r2 = dot(d, d) * 2.0;
    let t = smoothstep(GRADE_VIGNETTE_RADIUS * GRADE_VIGNETTE_RADIUS, 1.0, r2);
    return c * (1.0 - GRADE_VIGNETTE_STRENGTH * t);
}
// One display source for the frame: default hue-keeping, analytic when
// selected, then the display look (the highlight knee, then the value-gated
// stone drain) for every branch. DISPLAY_NEUTRAL swaps only the curve.
fn display_tonemap(color: vec3<f32>) -> vec3<f32> {
    if (DISPLAY_ANALYTIC) {
        return stone_drain(display_knee(analytic_display(color)));
    }
    if (DISPLAY_NEUTRAL) {
        return stone_drain(display_knee(neutral_display(color)));
    }
    if (DISPLAY_AGX) {
        return stone_drain(display_knee(agx_display(color)));
    }
    return stone_drain(display_knee(hue_keep_display(color)));
}
// Fixed-pattern 4x4 dither scaled to one 8-bit step, signed offset.
fn dither4(x: u32, y: u32) -> f32 {
    let xi = x & 3u;
    let yi = y & 3u;
    let idx = yi * 4u + xi;
    var m: u32 = 0u;
    switch (idx) {
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
    return (f32(m) + 0.5) / 16.0 / 255.0 - 0.5 / 255.0;
}

// PORT(bevyengine/bevy@v0.15.0:crates/bevy_render/src/color_operations.wgsl): inverse_gamma (sRGB EOTF).
fn linear_to_srgb_term(color: vec3<f32>) -> vec3<f32> {
    var out = color;
    out = select(out, vec3<f32>(0.0), out < vec3<f32>(0.0));
    let lo = out * 12.92;
    let hi = 1.055 * pow(out, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, out <= vec3<f32>(0.0031308));
}
