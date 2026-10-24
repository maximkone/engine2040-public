// dof.wgsl: the finisher beat's depth of field (SW173), half resolution.
// Mirrors crates/render/src/post_dof.rs, the CPU twin its tests pin.
//
// Two dispatches, 8x8 each, over the half target (ceil(w/2) x ceil(h/2)):
// `dof_coc` writes one radius per half texel from the full-res depth, and
// `dof_gather` blends a fixed disc of taps whose own CoC reaches the sample
// distance. Scatter-as-gather keeps a sharp foreground sharp: a neighbour
// joins only when its own radius covers the distance to this texel.
//
// Thin-lens CoC shape (textbook, no donor bytes): radius = aperture * |d - f| / d.

struct DofU {
    // full width, full height, half width, half height
    dims: vec4<u32>,
    // focus depth (m), aperture (half-res px at 1 m), max CoC (px), unused
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: DofU;
// Full-res view depth in metres, row-major over `dims.xy`.
@group(0) @binding(1) var<storage, read> depth: array<f32>;
// Half-res CoC radius in pixels, row-major over `dims.zw`.
@group(0) @binding(2) var<storage, read_write> coc: array<f32>;
// Full-res RGBA working colour (linear, pre-tonemap).
@group(0) @binding(3) var<storage, read> colour: array<vec4<f32>>;
// Half-res RGB the gather wrote, alpha carries coverage (0 = keep the frame).
@group(0) @binding(4) var<storage, read_write> out_dof: array<vec4<f32>>;

fn full_depth(x: u32, y: u32) -> f32 {
    let cx = min(x, u.dims.x - 1u);
    let cy = min(y, u.dims.y - 1u);
    return max(depth[cy * u.dims.x + cx], 0.0);
}

fn coc_px(depth_m: f32) -> f32 {
    if (depth_m <= 0.0) {
        return u.params.z;
    }
    return min(u.params.y * abs(depth_m - u.params.x) / depth_m, u.params.z);
}

@compute @workgroup_size(8, 8, 1)
fn dof_coc(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= u.dims.z || gid.y >= u.dims.w) {
        return;
    }
    let fx = min(gid.x * 2u, u.dims.x - 1u);
    let fy = min(gid.y * 2u, u.dims.y - 1u);
    coc[gid.y * u.dims.z + gid.x] = coc_px(full_depth(fx, fy));
}

// Fixed disc: one centre, six taps at half the radius, six at the radius, the
// same table the CPU twin walks.
const TAP_HALF: f32 = 0.4330127;
const TAP_FULL: f32 = 0.8660254;

fn tap_weight(index: u32, hx: u32, hy: u32, radius: f32) -> vec4<f32> {
    var offset = vec2<f32>(0.0, 0.0);
    switch index {
        case 0u: { offset = vec2<f32>(0.0, 0.0); }
        case 1u: { offset = vec2<f32>(0.5, 0.0); }
        case 2u: { offset = vec2<f32>(0.25, TAP_HALF); }
        case 3u: { offset = vec2<f32>(-0.25, TAP_HALF); }
        case 4u: { offset = vec2<f32>(-0.5, 0.0); }
        case 5u: { offset = vec2<f32>(-0.25, -TAP_HALF); }
        case 6u: { offset = vec2<f32>(0.25, -TAP_HALF); }
        case 7u: { offset = vec2<f32>(1.0, 0.0); }
        case 8u: { offset = vec2<f32>(0.5, TAP_FULL); }
        case 9u: { offset = vec2<f32>(-0.5, TAP_FULL); }
        case 10u: { offset = vec2<f32>(-1.0, 0.0); }
        case 11u: { offset = vec2<f32>(-0.5, -TAP_FULL); }
        default: { offset = vec2<f32>(0.5, -TAP_FULL); }
    }
    let sx = i32(hx) + i32(round(offset.x * radius));
    let sy = i32(hy) + i32(round(offset.y * radius));
    if (sx < 0 || sy < 0 || u32(sx) >= u.dims.z || u32(sy) >= u.dims.w) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    // Scatter-as-gather: the neighbour joins only when its own CoC reaches this
    // sample, so a sharp edge never smears.
    let reach = coc[u32(sy) * u.dims.z + u32(sx)];
    let dist = length(offset * radius);
    if (reach < dist) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let fx = min(u32(sx) * 2u, u.dims.x - 1u);
    let fy = min(u32(sy) * 2u, u.dims.y - 1u);
    return vec4<f32>(colour[fy * u.dims.x + fx].rgb, 1.0);
}

@compute @workgroup_size(8, 8, 1)
fn dof_gather(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= u.dims.z || gid.y >= u.dims.w) {
        return;
    }
    let idx = gid.y * u.dims.z + gid.x;
    let fx = min(gid.x * 2u, u.dims.x - 1u);
    let fy = min(gid.y * 2u, u.dims.y - 1u);
    let centre = colour[fy * u.dims.x + fx].rgb;
    let radius = coc[idx];
    if (radius < 0.5) {
        // The focus plane is untouched, so a sharp pixel copies through.
        out_dof[idx] = vec4<f32>(centre, 1.0);
        return;
    }
    var acc = vec3<f32>(0.0, 0.0, 0.0);
    var weight = 0.0;
    for (var i = 0u; i < 13u; i = i + 1u) {
        let t = tap_weight(i, gid.x, gid.y, radius);
        acc = acc + t.rgb * t.w;
        weight = weight + t.w;
    }
    if (weight < 0.5) {
        out_dof[idx] = vec4<f32>(centre, 1.0);
    } else {
        out_dof[idx] = vec4<f32>(acc / weight, 1.0);
    }
}
