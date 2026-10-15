// skin.wgsl: skin, eye and hair shade slice (subsurface approx plus glint).
//
// Owner: engine-render. Linear HDR radiance in, linear radiance out; the
// blit maps to display once, never here. The forward BRDF stays in its own file;
// this text only adds the slice terms the forward fragment calls next.
// Units: directions are unit vectors, colors are linear, wrap and thickness
// are unitless 0..1. Float hygiene: every normalize guards a tiny length,
// every dot clamps before shaping.

// Wrapped diffuse: light wraps around thin tissue so shadowed skin keeps a
// soft lift instead of dropping to black.
fn skin_wrap(ndotl: f32, wrap: f32) -> f32 {
    let w = clamp(wrap, 0.0, 1.0);
    return clamp((ndotl + w) / (1.0 + w), 0.0, 1.0);
}

// Transmission: back-light bleed through thin tissue, shaped sharp and
// scaled by thickness. The caller gates it per quality tier.
fn skin_transmission(v: vec3<f32>, l: vec3<f32>, thickness: f32) -> f32 {
    let through = clamp(dot(v, -l) * 0.5 + 0.5, 0.0, 1.0);
    let t = clamp(thickness, 0.0, 1.0);
    return pow(through, 3.0) * t;
}

// Skin under one light: wrapped diffuse plus gated transmission bleed.
// subsurface_on carries the tier gate: 0 keeps bare wrapped diffuse.
fn skin_shade(
    albedo: vec3<f32>,
    n: vec3<f32>,
    l: vec3<f32>,
    v: vec3<f32>,
    wrap: f32,
    scatter: vec3<f32>,
    thickness: f32,
    subsurface_on: u32
) -> vec3<f32> {
    let nn = n / max(length(n), 1e-6);
    let ll = l / max(length(l), 1e-6);
    let vv = v / max(length(v), 1e-6);
    let ndotl = clamp(dot(nn, ll), -1.0, 1.0);
    var out = albedo * skin_wrap(ndotl, wrap);
    if (subsurface_on != 0u) {
        out += scatter * skin_transmission(vv, ll, thickness);
    }
    return out;
}

// Wet eye: soft diffuse plus a tight cornea glint. glint_on carries the
// tier gate; a collapsed half-vector keeps diffuse and skips the glint.
fn eye_shade(
    albedo: vec3<f32>,
    n: vec3<f32>,
    l: vec3<f32>,
    v: vec3<f32>,
    strength: f32,
    sharp: f32,
    glint_on: u32
) -> vec3<f32> {
    let nn = n / max(length(n), 1e-6);
    let ll = l / max(length(l), 1e-6);
    let vv = v / max(length(v), 1e-6);
    var out = albedo * clamp(dot(nn, ll), 0.0, 1.0);
    if (glint_on != 0u) {
        let h = ll + vv;
        if (dot(h, h) >= 1e-12) {
            let hn = h / max(length(h), 1e-6);
            let ndoth = clamp(dot(nn, hn), 0.0, 1.0);
            out += vec3<f32>(strength * pow(ndoth, sharp));
        }
    }
    return out;
}

// Hair strand: soft diffuse plus an anisotropic glint band around the
// tangent-perpendicular plane. shift slides the band along the strand,
// glint_on carries the tier gate.
fn hair_shade(
    albedo: vec3<f32>,
    n: vec3<f32>,
    t: vec3<f32>,
    l: vec3<f32>,
    v: vec3<f32>,
    tint: vec3<f32>,
    shift: f32,
    strength: f32,
    sharp: f32,
    glint_on: u32
) -> vec3<f32> {
    let nn = n / max(length(n), 1e-6);
    let tt = t / max(length(t), 1e-6);
    let ll = l / max(length(l), 1e-6);
    let vv = v / max(length(v), 1e-6);
    var out = albedo * clamp(dot(nn, ll), 0.0, 1.0);
    if (glint_on != 0u) {
        let h = ll + vv;
        if (dot(h, h) >= 1e-12) {
            let hn = h / max(length(h), 1e-6);
            let th = clamp(dot(tt, hn) - shift, -1.0, 1.0);
            let band = sqrt(max(0.0, 1.0 - th * th));
            out += tint * (strength * pow(band, sharp));
        }
    }
    return out;
}
