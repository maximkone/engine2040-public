// SW150 mask pass: the locked target ids drawn white over a depth prefill of
// everything else. One draw per selected fighter (the plan's `draws`), the
// model matrix in the per-draw uniform. The fragment writes
// (coverage, visibility), where visibility is the depth test against the
// prefill sampled through the donor's 0.5-biased texture matrix: 1 when the
// fighter is in front (a visible edge, white afterwards), 0 when the sand berm
// covers it (the hidden edge colour). Port of three.js OutlinePass
// `_getPrepareMaskMaterial` plus `_updateTextureMatrix`.

struct MaskGlobals {
    // 0.5 bias * projection * view, the donor's texture matrix.
    texture_matrix: mat4x4<f32>,
    view_proj: mat4x4<f32>,
    // Depth-target size in texels, for the prefill texel lookup.
    prefill_size: vec2<f32>,
    // Near and far plane in metres, the prefill's linearization.
    near_far: vec2<f32>,
}

struct MaskDraw {
    model: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> globals: MaskGlobals;
@group(1) @binding(0) var<uniform> draw: MaskDraw;
@group(1) @binding(1) var prefill_depth: texture_depth_2d;

// The white the mask writes: an override so a debug capture can set it to 0.5
// and see the coverage without the bloom that a full white adds.
override MASK_WHITE: f32 = 1.0;
// How far behind the prefill a fragment may sit and stay visible, metres.
const DEPTH_BIAS_M: f32 = 0.02;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    // The world position through the texture matrix, divided by w in the
    // fragment for the prefill uv.
    @location(0) proj_tex_coord: vec4<f32>,
    // Eye distance along the camera forward, metres, positive in front.
    @location(1) view_depth: f32,
}

@vertex
fn vs_mask(@location(0) position: vec3<f32>) -> VsOut {
    let world = draw.model * vec4<f32>(position, 1.0);
    var out: VsOut;
    out.clip = globals.view_proj * world;
    out.proj_tex_coord = globals.texture_matrix * world;
    // The camera looks down -Z, so a positive view depth is in front of it.
    out.view_depth = -out.clip.w;
    return out;
}

// A stored depth `0..=1` back to metres along the view ray, the wgpu
// convention (0 at the near plane, 1 at the far plane).
fn depth_to_view_z(depth: f32, near: f32, far: f32) -> f32 {
    return near * far / max(far - depth * (far - near), 1e-6);
}

@fragment
fn fs_mask(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.proj_tex_coord.xy / max(in.proj_tex_coord.w, 1e-6);
    var occluder = 0.0;
    if (all(uv >= vec2<f32>(0.0)) && all(uv < vec2<f32>(1.0))) {
        let texel = vec2<i32>(uv * globals.prefill_size);
        occluder = textureLoad(prefill_depth, texel, 0);
    }
    let scene_z = depth_to_view_z(occluder, globals.near_far.x, globals.near_far.y);
    // The donor's test: `(-vPosition.z > viewZ) ? 0 : 1`.
    let visible = select(0.0, 1.0, in.view_depth <= scene_z + DEPTH_BIAS_M);
    return vec4<f32>(MASK_WHITE, visible, 0.0, 1.0);
}
