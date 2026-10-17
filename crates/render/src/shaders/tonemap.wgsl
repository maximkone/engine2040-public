// tonemap.wgsl: the display transform, one copy for every pass that ends a
// frame. Linear HDR radiance in, display value out: hue-keeping display curve,
// then the sRGB transfer (found2 5.3, gems1 15.4). The window frame runs it once,
// in the blit, never in the forward fragment. Prepended (compile-time
// concat) to forward.wgsl and to blit.wgsl, so this text exists once.
// PORT(BabylonJS/Babylon.js@136feacc96b011d98f26413edef252e098c846fd:packages/dev/core/src/ShadersWGSL/tonemap.fragment.fx): ACES shape, reimplemented.
// PORT(BabylonJS/Babylon.js@136feacc96b011d98f26413edef252e098c846fd:packages/dev/core/src/ShadersWGSL/imageProcessing.fragment.fx): grade shape, reimplemented.
// Zero donor bytes vendored in this file.

// PORT(bevyengine/bevy@v0.15.0:crates/bevy_core_pipeline/src/tonemapping/tonemapping_shared.wgsl): RRTAndODTFit + ACESFitted.
fn rrt_odt_fit(v: vec3<f32>) -> vec3<f32> {
    let a = v * (v + 0.0245786) - 0.000090537;
    let b = v * (0.983729 * v + 0.4329510) + 0.238081;
    return a / b;
}
fn aces_fitted(color: vec3<f32>) -> vec3<f32> {
    let rgb_to_rrt = mat3x3<f32>(
        vec3<f32>(0.59719, 0.35458, 0.04823),
        vec3<f32>(0.07600, 0.90834, 0.01566),
        vec3<f32>(0.02840, 0.13383, 0.83777)
    );
    let odt_to_rgb = mat3x3<f32>(
        vec3<f32>(1.60475, -0.53108, -0.07367),
        vec3<f32>(-0.10208, 1.10813, -0.00605),
        vec3<f32>(-0.00327, -0.07276, 1.07602)
    );
    var c = color * rgb_to_rrt;
    c = rrt_odt_fit(c);
    c = c * odt_to_rgb;
    return clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
}

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
// One hue-preserving display scale for a peak: the fit of the anchored peak
// plus the knee expansion, divided back by the peak so every channel keeps
// its ratio (the same shape as the unanchored path, anchor applied once).
fn display_scale(peak: f32, toned_peak: f32) -> f32 {
    let knee = max(toned_peak - DISPLAY_KNEE, 0.0);
    let lifted = toned_peak + knee * DISPLAY_KNEE_GAIN;
    return lifted / max(peak, 1e-6);
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
// Hue-keeping display: one scale from the per-channel curve keeps input ratios.
fn hue_keep_display(color: vec3<f32>) -> vec3<f32> {
    let safe = max(color, vec3<f32>(0.0));
    let capped = min(safe, vec3<f32>(1e6));
    let peak = max(capped.x, max(capped.y, capped.z));
    let white = max(fit_one(DISPLAY_WHITE), 1e-4);
    let toned_peak = fit_one(peak * DISPLAY_EXPOSURE) / white;
    return capped * display_scale(peak, toned_peak);
}
// Analytic display: same one-scale shape over the analytic curve.
fn analytic_display(color: vec3<f32>) -> vec3<f32> {
    let safe = max(color, vec3<f32>(0.0));
    let capped = min(safe, vec3<f32>(1e6));
    let peak = max(capped.x, max(capped.y, capped.z));
    let white = max(analytic_one(DISPLAY_WHITE), 1e-4);
    let toned_peak = analytic_one(peak * DISPLAY_EXPOSURE) / white;
    return capped * display_scale(peak, toned_peak);
}
// One display source for the frame: default hue-keeping, analytic when
// selected, then the value-gated stone drain (one grade for both branches).
fn display_tonemap(color: vec3<f32>) -> vec3<f32> {
    if (DISPLAY_ANALYTIC) {
        return stone_drain(analytic_display(color));
    }
    return stone_drain(hue_keep_display(color));
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
