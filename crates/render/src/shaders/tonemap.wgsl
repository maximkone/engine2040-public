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
    let toned_peak = fit_one(peak) / white;
    let scale = toned_peak / max(peak, 1e-6);
    return capped * scale;
}
// Analytic display: same one-scale shape over the analytic curve.
fn analytic_display(color: vec3<f32>) -> vec3<f32> {
    let safe = max(color, vec3<f32>(0.0));
    let capped = min(safe, vec3<f32>(1e6));
    let peak = max(capped.x, max(capped.y, capped.z));
    let white = max(analytic_one(DISPLAY_WHITE), 1e-4);
    let toned_peak = analytic_one(peak) / white;
    let scale = toned_peak / max(peak, 1e-6);
    return capped * scale;
}
// One display source for the frame: default hue-keeping, analytic when selected.
fn display_tonemap(color: vec3<f32>) -> vec3<f32> {
    if (DISPLAY_ANALYTIC) {
        return analytic_display(color);
    }
    return hue_keep_display(color);
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
