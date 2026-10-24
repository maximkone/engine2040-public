// ssr.wgsl: screen-space reflections for wet and polished stone (SW90).
// Mirrors crates/render/src/ssr.rs, the CPU twin its tests pin.
// PORT(godotengine/godot@master:servers/rendering/renderer_rd/shaders/effects/screen_space_reflection.glsl):
// the parametric-t Hi-Z march, `mip_offset`, depth_tolerance, validity in alpha.
// PORT(mrdoob/three.js@dev:examples/jsm/shaders/SSRShader.js): view position
// from screen xy and depth, the thickness test, distance attenuation.
//
// One invocation per target texel (8x8 workgroup). The half target is
// ceil(w/2) x ceil(h/2) on Potato/Web/Mobile, full on the other tiers; the
// half image goes through gtao::upsample_bilateral, which already blends on
// depth, so a reflection never bleeds across a silhouette.

struct SsrU {
    // target width, target height, full width, full height
    dims: vec4<u32>,
    // depth_tolerance (m), distance_fade (t), curve_fade_in (t), roughness_cut
    params: vec4<f32>,
    // projection scale x, projection scale y, ray steps, Hi-Z mip count
    ray: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: SsrU;
// Depth pyramid, level 0 is the full-res view depth in meters; each level
// holds the minimum of its four children (closest surface, so a ray in front
// of the mip is in front of everything inside it). Cheapest tiers bind one
// level, so the march degrades to the fixed-step three.js fallback.
@group(0) @binding(1) var<storage, read> hi_z: array<f32>;
// Forward HDR colour, the source of the reflected texel.
@group(0) @binding(2) var<storage, read> hdr: array<vec4<f32>>;
// G-buffer: x roughness, y metalness, z unused, w view depth in meters.
@group(0) @binding(3) var<storage, read> material: array<vec4<f32>>;
// Reflection colour plus validity in alpha (0 = keep the forward colour).
@group(0) @binding(4) var<storage, read_write> out_ssr: array<vec4<f32>>;

fn texel_uv(x: u32, y: u32) -> vec2<f32> {
    return vec2<f32>((f32(x) + 0.5) / f32(max(u.dims.x, 1u)),
                     (f32(y) + 0.5) / f32(max(u.dims.y, 1u)));
}

// Full-res depth alphas are meters; a cleared texel decodes at the far plane.
fn depth_at(x: u32, y: u32) -> f32 {
    let fx = u.dims.z;
    let fy = u.dims.w;
    let cx = min(x, fx - 1u);
    let cy = min(y, fy - 1u);
    return max(material[cy * fx + cx].w, 0.0);
}

// View position from the target texel and its depth: x to the right, y up,
// z away from the camera in meters (three.js getViewPosition).
fn view_pos(x: u32, y: u32, depth_m: f32) -> vec3<f32> {
    let uv = texel_uv(x, y);
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    return vec3<f32>(ndc * u.ray.xy * depth_m, depth_m);
}

// Screen position of a view position, the inverse of view_pos.
fn view_to_target(p: vec3<f32>) -> vec2<f32> {
    let z = max(p.z, 1e-4);
    let ndc = p.xy / (u.ray.xy * z);
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}

// Geometric normal from depth derivatives (Godot compute_geometric_normal):
// the surface normal, no G-buffer normal channel needed.
fn geometric_normal(x: u32, y: u32) -> vec3<f32> {
    let center = view_pos(x, y, depth_at(x, y));
    let right = view_pos(x + 1u, y, depth_at(x + 1u, y));
    let up = view_pos(x, y + 1u, depth_at(x, y + 1u));
    let n = cross(right - center, up - center);
    let len = length(n);
    if (len < 1e-6) {
        return vec3<f32>(0.0, 0.0, 1.0);
    }
    return n / len;
}

// Offset of Hi-Z level `level` in the packed pyramid: sum of the texels of the
// finer levels.
fn level_base(level: u32) -> u32 {
    var base = 0u;
    var lw = u.dims.z;
    var lh = u.dims.w;
    var l = 0u;
    loop {
        if (l >= level) {
            break;
        }
        base = base + lw * lh;
        lw = max(lw / 2u, 1u);
        lh = max(lh / 2u, 1u);
        l = l + 1u;
    }
    return base;
}

fn hiz_depth(level: u32, uv: vec2<f32>) -> f32 {
    var lw = u.dims.z;
    var lh = u.dims.w;
    var l = 0u;
    loop {
        if (l >= level) {
            break;
        }
        lw = max(lw / 2u, 1u);
        lh = max(lh / 2u, 1u);
        l = l + 1u;
    }
    let x = min(u32(clamp(uv.x, 0.0, 1.0) * f32(lw)), lw - 1u);
    let y = min(u32(clamp(uv.y, 0.0, 1.0) * f32(lh)), lh - 1u);
    return hi_z[level_base(level) + y * lw + x];
}

fn fetch_hdr(x: u32, y: u32) -> vec3<f32> {
    let cx = min(x, u.dims.x - 1u);
    let cy = min(y, u.dims.y - 1u);
    return hdr[cy * u.dims.x + cx].rgb;
}

// March one reflected ray. Steps by parametric t (Z normalised to +-1), stops
// at the screen edge or after u.ray.z taps, and steps the Hi-Z level with
// mip_offset = inside ? -1 : +1 (coarser while the ray is in front of the
// geometry, back to level 0 the moment it is inside). A hit returns its colour
// with validity 1.0; every miss returns alpha 0.0 rather than being rejected.
fn march(origin: vec3<f32>, dir: vec3<f32>, steps: u32, t_max: f32) -> vec4<f32> {
    let mip_count = max(u32(u.ray.w), 1u);
    var level = 0u;
    var step = 0u;
    loop {
        if (step >= steps) {
            break;
        }
        let t = f32(step + 1u) / f32(max(steps, 1u));
        if (t > t_max) {
            break;
        }
        let p = origin + dir * t;
        let uv = view_to_target(p);
        let d = hiz_depth(level, uv);
        let inside = p.z >= d && p.z <= d + u.params.x;
        if (inside && level == 0u) {
            let x = min(u32(clamp(uv.x, 0.0, 1.0) * f32(u.dims.x)), u.dims.x - 1u);
            let y = min(u32(clamp(uv.y, 0.0, 1.0) * f32(u.dims.y)), u.dims.y - 1u);
            // Distance attenuation (three.js ratio * ratio) times the fade-in
            // off the reflecting surface, so the mirror never smears.
            let fade_in = select(1.0, clamp(t / u.params.z, 0.0, 1.0), u.params.z > 0.0);
            let run = clamp(t / u.params.y, 0.0, 1.0);
            return vec4<f32>(fetch_hdr(x, y), fade_in * run * run);
        }
        // mip_offset = hit ? -1 : +1, clamped to the packed chain.
        if (inside) {
            if (level > 0u) {
                level = level - 1u;
            }
        } else if (level + 1u < mip_count) {
            level = level + 1u;
        }
        step = step + 1u;
    }
    return vec4<f32>(0.0, 0.0, 0.0, 0.0);
}

@compute @workgroup_size(8, 8, 1)
fn ssr_trace(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= u.dims.x || gid.y >= u.dims.y) {
        return;
    }
    let idx = gid.y * u.dims.x + gid.x;
    out_ssr[idx] = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    // (a) rough stone and pure dielectrics trace nothing.
    let mat = material[idx];
    if (mat.x >= u.params.w || mat.y <= 0.0) {
        return;
    }
    let depth_m = max(mat.w, 0.0);
    let origin = view_pos(gid.x, gid.y, depth_m);
    let n = geometric_normal(gid.x, gid.y);
    var dir = reflect(normalize(origin), n);
    // (c) normalise the ray so Z is +-1: the march then advances one meter of
    // depth per unit of t. A ray that leaves along the screen has a tiny Z and
    // is clamped, so the step never explodes.
    dir = dir / max(abs(dir.z), 0.05);
    let end = origin + dir * max(depth_m, 0.1);
    let a = view_to_target(origin);
    let b = view_to_target(end);
    // Clip the ray at the screen edge: the last tap is the last texel inside.
    let span = b - a;
    var t_max = 1.0;
    if (span.x > 0.0) {
        t_max = min(t_max, (1.0 - a.x) / span.x);
    } else if (span.x < 0.0) {
        t_max = min(t_max, a.x / -span.x);
    }
    if (span.y > 0.0) {
        t_max = min(t_max, (1.0 - a.y) / span.y);
    } else if (span.y < 0.0) {
        t_max = min(t_max, a.y / -span.y);
    }
    let steps = u32(max(u.ray.z, 1.0));
    let travel = dir * max(depth_m, 0.1);
    out_ssr[idx] = march(origin, travel, steps, clamp(t_max, 0.0, 1.0));
}
