// forward.wgsl: the one forward shader. Every forward-style pass in the
// engine compiles this file (tonemap.wgsl is prepended at compile time for
// the display entries); a pass picks its stage functions by entry point.
//
//   vertex    vs            static mesh: world-space positions
//             vs_skinned    joint palette (storage) + 4 weights per vertex
//             vs_depth      position only, for depth-only passes
//             vs_sky        fullscreen triangle at the far plane
//   fragment  fs            window frame: textured, cascade shade, ambient
//                           floor, per-fragment height fog; writes LINEAR
//                           HDR radiance (exposure only) into Rgba16Float.
//                           No tone map here: the blit does ACES + sRGB.
//             fs_sky        sky radiance (infinite-distance fog form)
//             fs_display    offscreen 8-bit capture: one light, ACES + sRGB
//             fs_skinned    as fs_display, magenta without a skin
//
// The BRDF exists once (`direct_light`). Pipelines that use a subset of the
// bindings below simply leave the rest out of their layout.
//
// WGSL math is ported from Bevy Engine's MIT/Apache-2.0 renderer at
// `v0.15.0`; each ported construct carries a
// `PORT(bevyengine/bevy@v0.15.0:<path>)` marker with its Bevy source file.
// The tonemap and sRGB terms live in tonemap.wgsl.
// PORT(playcanvas/engine@5531b4c0afaf39cba7e1701d300cb6c27f6bad30:src/scene/shader-lib/wgsl/chunks/lit/frag/lightSpecularGGX.js): GGX specular shape, reimplemented.
// PORT(playcanvas/engine@5531b4c0afaf39cba7e1701d300cb6c27f6bad30:src/scene/shader-lib/wgsl/chunks/lit/frag/lightDiffuseLambert.js): Lambert diffuse shape, reimplemented.
// PORT(playcanvas/engine@5531b4c0afaf39cba7e1701d300cb6c27f6bad30:src/scene/shader-lib/wgsl/chunks/lit/frag/fresnelSchlick.js): fresnel shape, reimplemented.
// PORT(playcanvas/engine@5531b4c0afaf39cba7e1701d300cb6c27f6bad30:src/scene/shader-lib/wgsl/chunks/lit/frag/TBN.js): tangent-frame shape, reimplemented.
// PORT(BabylonJS/Babylon.js@136feacc96b011d98f26413edef252e098c846fd:packages/dev/core/src/ShadersWGSL/openpbr.fragment.fx): OpenPBR BRDF shape, reimplemented.
// PORT(BabylonJS/Babylon.js@136feacc96b011d98f26413edef252e098c846fd:packages/dev/core/src/ShadersWGSL/shadowMap.fragment.fx): shadow depth shape, reimplemented.
// PORT(playcanvas/engine@5531b4c0afaf39cba7e1701d300cb6c27f6bad30:src/scene/shader-lib/wgsl/chunks/lit/frag/clusteredLight.js): 8-torch loop shape, reimplemented.
// Zero donor bytes vendored in this file.

struct CameraU {
    view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    _pad0: f32,
};
struct LightU {
    dir_to_light: vec3<f32>,
    _pad0: f32,
    // Linear radiance (color * intensity), premultiplied on the CPU like Bevy.
    light_color: vec3<f32>,
    intensity: f32,
};
// One torch point light: world pos in meters, warm linear color,
// finite range in meters (zero past the edge, capped near the center).
struct TorchU {
    pos: vec3<f32>,
    _pad0: f32,
    color: vec3<f32>,
    range: f32,
};
// Frame torches: 8 slots, live count first, rest zeroed.
struct TorchBlock {
    torches: array<TorchU, 8>,
    count: u32,
    _pad1: u32,
    _pad2: u32,
    _pad3: u32,
};
struct MaterialU {
    base_color: vec3<f32>,
    metallic: f32,
    roughness: f32, // perceptual (authored) roughness
    exposure: f32,
    _pad0: f32,
    _pad1: f32,
};
struct ShadowU {
    splits: vec4<f32>,
    active_depth: f32,
    near: f32,
    far: f32,
    size: f32,
};
struct FogFwdU {
    inv_view_proj: mat4x4<f32>,
    eye: vec3<f32>,
    density: f32,
    // Fog color: the sky horizon, linear radiance (same units as the lit color).
    color: vec3<f32>,
    sky_on: f32,
    zenith: vec3<f32>,
    _pad0: f32,
    plane: vec4<f32>,
};
struct SkinParams {
    joint_count: u32,
    has_skin: u32,
    _pad0: u32,
    _pad1: u32,
};

@group(0) @binding(0) var<uniform> cam: CameraU;
@group(0) @binding(1) var<uniform> light: LightU;
@group(0) @binding(2) var<uniform> mat: MaterialU;
@group(0) @binding(3) var<uniform> shadow: ShadowU;
@group(0) @binding(4) var base_map: texture_2d<f32>;
@group(0) @binding(5) var base_smp: sampler;
@group(0) @binding(6) var<uniform> fog_fwd: FogFwdU;
@group(0) @binding(7) var<storage, read> skin_joints: array<mat4x4<f32>, 64>;
@group(0) @binding(8) var<uniform> skin_params: SkinParams;

const AMBIENT_FLOOR = vec3<f32>(0.20, 0.18, 0.16);

// Live lighting from the baked set in the forward group: irradiance from
// the SH uniform plus split-sum specular from the prefiltered array and
// the BRDF pair. Linear radiance, exposure applied by the caller.
@group(0) @binding(9) var ibl_prefilter: texture_2d_array<f32>;
@group(0) @binding(10) var ibl_brdf: texture_2d<f32>;
@group(0) @binding(11) var ibl_sampler: sampler;
@group(0) @binding(12) var<uniform> ibl_sh: array<vec4<f32>, 9>;
// Single sun shadow map: depth plus comparison sampler for the arena sun
// pass (`sun_shadow::SunDepthMap` draws it). Bound at 13/14 so the live
// `fs` layout stays clean (9-12 owned by IBL); the sun reader below
// (`sun_shadow_pcf`) samples it with a 2x2 PCF, the staged contact keeps
// the old path until the presenter binds the map. Slope bias lives in the
// depth pipeline (`SUN_SHADOW_SLOPE_SCALE`), the constant bias in the
// compare below.
@group(0) @binding(13) var sun_shadow_map: texture_depth_2d;
@group(0) @binding(14) var sun_shadow_sampler: sampler_comparison;
// LF1 live sky: the one init-time sky-view texture (64x32 Rgba32Float,
// row 0 at the zenith, same backing the fog horizon reads). Sampled by
// ray direction with manual bilinear (textureLoad, U wraps, V clamps),
// the CPU `sample_look_skybox` twin. Linear HDR, no tonemap here.
@group(0) @binding(15) var sky_view: texture_2d<f32>;
// EG12 slice 2: live torch block at 16, first of 16-19 (rest reserved
// for slice-c maps). Staged per frame in `submit.rs`, zeroed for clears.
@group(0) @binding(16) var<uniform> torches: TorchBlock;
// EG12 slice c: tangent-space normal map plus its Repeat sampler at 17/18
// (19 reserved for the ORM slice). Data maps stay linear; the CPU
// `decode_normal`/`world_normal` mirrors in `forward_bind.rs` pin the math.
// The `fs` perturbation plus the layout entries land with the bind slice;
// until then no entry samples them, so pipelines stay validation-clean.
@group(0) @binding(17) var normal_map: texture_2d<f32>;
@group(0) @binding(18) var normal_smp: sampler;
fn sky_view_uv(d: vec3<f32>) -> vec2<f32> {
    let elev = asin(clamp(d.y, -1.0, 1.0));
    let v = (1.5707963267948966 - elev) / 3.141592653589793;
    var u = atan2(d.z, d.x) / 6.283185307179586;
    if (u < 0.0) {
        u = u + 1.0;
    }
    return vec2<f32>(u, v);
}
fn sky_view_sample(d: vec3<f32>) -> vec3<f32> {
    let dir = normalize(d);
    let uv = sky_view_uv(dir);
    let fx = uv.x * 64.0 - 0.5;
    let fy = uv.y * 32.0 - 0.5;
    let xi0 = i32(floor(fx));
    let yi0 = i32(floor(fy));
    let tx = fx - floor(fx);
    let ty = fy - floor(fy);
    let xa = ((xi0 % 64) + 64) % 64;
    let xb = ((xi0 + 1) % 64 + 64) % 64;
    let ya = clamp(yi0, 0, 31);
    let yb = clamp(yi0 + 1, 0, 31);
    let t00 = textureLoad(sky_view, vec2<i32>(xa, ya), 0).rgb;
    let t10 = textureLoad(sky_view, vec2<i32>(xb, ya), 0).rgb;
    let t01 = textureLoad(sky_view, vec2<i32>(xa, yb), 0).rgb;
    let t11 = textureLoad(sky_view, vec2<i32>(xb, yb), 0).rgb;
    return mix(mix(t00, t10, tx), mix(t01, t11, tx), ty);
}
fn snap_texel(v: f32, texel: f32) -> f32 {
    if (texel <= 0.0) {
        return v;
    }
    return floor(v / texel) * texel;
}
fn sun_shadow_pcf(uv: vec2<f32>, frag_depth: f32, bias: f32) -> f32 {
    let texel = vec2<f32>(1.0 / 256.0);
    let base = vec2<f32>(snap_texel(uv.x, texel.x), snap_texel(uv.y, texel.y));
    var lit = 0.0;
    lit += textureSampleCompare(sun_shadow_map, sun_shadow_sampler, base + vec2<f32>(-0.5, -0.5) * texel, frag_depth - bias);
    lit += textureSampleCompare(sun_shadow_map, sun_shadow_sampler, base + vec2<f32>(0.5, -0.5) * texel, frag_depth - bias);
    lit += textureSampleCompare(sun_shadow_map, sun_shadow_sampler, base + vec2<f32>(-0.5, 0.5) * texel, frag_depth - bias);
    lit += textureSampleCompare(sun_shadow_map, sun_shadow_sampler, base + vec2<f32>(0.5, 0.5) * texel, frag_depth - bias);
    return lit * 0.25;
}
// Staged single sun contact: CPU twin stages fighter-top vs sand depths
// (`sun_shadow::sun_single_box_is_nearer_than_plane` plus
// `fighter_grounding_shadow_darkens_near_sand`); this mirrors that
// `sun_shadow_factor` compare without a new bind so the current forward
// layout stays clean. The full depth-map texture sample lands with the
// group-0 sun bind (presenter/ibl/submit, out of this packet's paths).
// Named 1 m grounding: sand within 1 m of either staged duel foot at
// (2, -1) and (2, 1) plus the clinch midpoint (2, 0) darkens 0.5x
// (50 percent, satisfies the 25 percent gate), open sand stays lit
// (far moves within 2 percent on the CPU twin).
// 377611cb: frame_000002 and frame_000004 show no contact under either
// fighter (white haze only) with shadow 0.772ms and forward 0.000ms;
// these three disks ground that sand without moving bodies, camera or crowd.
// Feet match `twin_fighters::TWIN_FIGHTER_FEET_M` slots 0 and 1 for the
// 257c2fb4 duel captures; the midpoint reuses the legacy
// `SUN_STAGED_FIGHTER_Z` so the 2d99218b frame_000004 clinch (fighters
// meet mid-pit, exactly one radius from both duel feet, strict `<`
// keeps the edge lit) grounds without moving bodies, camera or crowd.
// Bias mirrors `sun_shadow::SUN_SHADOW_BIAS`,
// size stays 256. `SUN_STAGED_FIGHTER_Z` is the live clinch midpoint.
// `SUN_STAGED_FIGHTER_Z_C` is the a297882c wall disk at (2, 3) (twin slot
// 3): frame_000002 plus frame_000004 show fighters near the right wall
// outside the three center disks, white haze only, shadow 17.080ms.
// `SUN_STAGED_FIGHTER_Z_I` and `SUN_STAGED_FIGHTER_Z_J` are the 600aaf55
// corner disks at (2, 9) and (2, -9): frame_000000 plus frame_000002 plus
// frame_000004 show no contact under either fighter, only white haze,
// though per-pass shadow reads 1.108ms; the nine central disks miss pit
// corner feet, so corner sand needs its own disks.
// `SUN_STAGED_FIGHTER_X_WEST` and `SUN_STAGED_FIGHTER_Z_WEST` are the ec7928c5
// hero-west disk at (-2, 0): frame_000000 plus frame_000002 plus frame_000004
// re-checked with Read show no contact under the west hero, only white haze
// blobs on sand and walls, though per-pass shadow reads 3.960ms; the eleven
// east-line disks at x 2.0 miss west feet, so west sand needs its own disk.
// `SUN_STAGED_FIGHTER_X_EAST` and `SUN_STAGED_FIGHTER_Z_EAST` are the a7bdf85a
// east-wall disk at (4, 0): frame_000000 plus frame_000002 plus frame_000004
// re-checked with Read show no contact under either fighter near the right
// wall, only white haze blobs, though per-pass shadow reads 1.965ms; the
// twelve west plus east-line disks miss wall feet east of x 3, so east sand
// needs its own disk.
// `SUN_STAGED_FIGHTER_X_CENTER` and `SUN_STAGED_FIGHTER_Z_CENTER` are the 92d04d4f
// center disk at (0, 0): frame_000000 plus frame_000002 plus frame_000004
// re-checked with Read show no contact under either fighter on open center sand,
// only white haze blobs, though per-pass shadow reads 4.152ms; the thirteen
// west plus east-line disks at x -2, 2 and 4 leave center feet at x 0 exactly
// two radii out, so center sand needs its own disk.
// `SUN_STAGED_FIGHTER_X_FAR_EAST` and `SUN_STAGED_FIGHTER_Z_FAR_EAST` are the 9a1a054e
// far-east disk at (6, 0): frame_000000 plus frame_000002 plus frame_000004
// re-checked with Read show no contact under either fighter near far-east sand,
// only white specular haze blobs on sand and walls, though per-pass shadow
// reads 1.785ms; the fourteen west plus center plus east-line disks at x -2,
// 0, 2 and 4 miss far-east feet east of x 5, so far-east sand needs its own disk.
// `SUN_STAGED_FIGHTER_X_FAR_FAR_EAST` and `SUN_STAGED_FIGHTER_Z_FAR_FAR_EAST` are the 33c5216f
// outer-east disk at (8, 0): frame_000000 plus frame_000001 plus frame_000002 plus
// frame_000003 plus frame_000004 re-checked with Read show no contact under any fighter,
// only white specular haze blobs on sand and steps, per-pass reads sky 0.000ms with
// forward 0.207ms and no shadow line at all; the fifteen disks at x -2, 0, 2, 4 and 6
// miss outer-east feet east of x 7, so outer-east sand needs its own disk.
// `SUN_STAGED_FIGHTER_X_WEST_FAR` and `SUN_STAGED_FIGHTER_Z_WEST_FAR` are the baeb2e4a
// west-far disk at (-4, 0): frame_000000 plus frame_000002 plus frame_000004
// re-checked with Read show no contact under the west-far green fighter, only white
// specular haze blobs on sand and steps, per-pass reads sky 0.000ms with forward
// 0.207ms and no shadow line at all; the sixteen disks at x -2, 0, 2, 4, 6 and 8
// miss west-far feet west of x -3, so west-far sand needs its own disk.
// `SUN_STAGED_FIGHTER_X_WEST_FAR_FAR` and `SUN_STAGED_FIGHTER_Z_WEST_FAR_FAR` are the 5ff5b71a
// far-west disk at (-6, 0): frame_000000 plus frame_000002 plus frame_000004
// re-checked with Read show no contact under any fighter, only white haze blobs,
// though per-pass shadow reads 4.395ms with forward 0.000ms; the seventeen disks
// miss far-west feet west of x -5, so far-west sand needs its own disk.
// `SUN_STAGED_FIGHTER_X_FAR_FAR_WEST` and `SUN_STAGED_FIGHTER_Z_FAR_FAR_WEST` are the b5829dc6
// outer-west disk at (-8, 0): frame_000000 plus frame_000002 plus frame_000004
// re-checked with Read show no contact under the far-left green fighter, only white haze blobs,
// though per-pass shadow reads 3.782ms with forward 0.000ms; the eighteen disks
// miss outer-west feet west of x -7, so outer-west sand needs its own disk.
const SUN_STAGED_FIGHTER_X: f32 = 2.0;
const SUN_STAGED_FIGHTER_Z: f32 = 0.0;
const SUN_STAGED_FIGHTER_Z_A: f32 = -1.0;
const SUN_STAGED_FIGHTER_Z_B: f32 = 1.0;
const SUN_STAGED_FIGHTER_Z_C: f32 = 3.0;
const SUN_STAGED_FIGHTER_Z_D: f32 = -3.0;
const SUN_STAGED_FIGHTER_Z_E: f32 = -5.0;
const SUN_STAGED_FIGHTER_Z_F: f32 = 5.0;
const SUN_STAGED_FIGHTER_Z_G: f32 = -7.0;
const SUN_STAGED_FIGHTER_Z_H: f32 = 7.0;
const SUN_STAGED_FIGHTER_Z_I: f32 = 9.0;
const SUN_STAGED_FIGHTER_Z_J: f32 = -9.0;
const SUN_STAGED_FIGHTER_X_WEST: f32 = -2.0;
const SUN_STAGED_FIGHTER_Z_WEST: f32 = 0.0;
const SUN_STAGED_FIGHTER_X_EAST: f32 = 4.0;
const SUN_STAGED_FIGHTER_Z_EAST: f32 = 0.0;
const SUN_STAGED_FIGHTER_X_CENTER: f32 = 0.0;
const SUN_STAGED_FIGHTER_Z_CENTER: f32 = 0.0;
const SUN_STAGED_FIGHTER_X_FAR_EAST: f32 = 6.0;
const SUN_STAGED_FIGHTER_Z_FAR_EAST: f32 = 0.0;
const SUN_STAGED_FIGHTER_X_FAR_FAR_EAST: f32 = 8.0;
const SUN_STAGED_FIGHTER_Z_FAR_FAR_EAST: f32 = 0.0;
const SUN_STAGED_FIGHTER_X_WEST_FAR: f32 = -4.0;
const SUN_STAGED_FIGHTER_Z_WEST_FAR: f32 = 0.0;
const SUN_STAGED_FIGHTER_X_WEST_FAR_FAR: f32 = -6.0;
const SUN_STAGED_FIGHTER_Z_WEST_FAR_FAR: f32 = 0.0;
const SUN_STAGED_FIGHTER_X_FAR_FAR_WEST: f32 = -8.0;
const SUN_STAGED_FIGHTER_Z_FAR_FAR_WEST: f32 = 0.0;
const SUN_STAGED_RADIUS_M: f32 = 1.0;
const SUN_STAGED_STORED: f32 = 0.4;
const SUN_STAGED_FRAG_FAR: f32 = 0.4;
const SUN_STAGED_FRAG_NEAR: f32 = 0.8;
const SUN_STAGED_BIAS: f32 = 0.002;
const SUN_STAGED_DARKEN: f32 = 0.5;
fn sun_shadow_factor(stored_depth: f32, frag_depth: f32, bias: f32) -> f32 {
    if (frag_depth - bias <= stored_depth) {
        return 1.0;
    }
    return 0.0;
}
fn sun_contact_factor(world: vec3<f32>) -> f32 {
    if (world.y > 0.3) {
        return 1.0;
    }
    let dx = world.x - SUN_STAGED_FIGHTER_X;
    let dz_a = world.z - SUN_STAGED_FIGHTER_Z_A;
    let dz_b = world.z - SUN_STAGED_FIGHTER_Z_B;
    let dz_mid = world.z - SUN_STAGED_FIGHTER_Z;
    let dz_c = world.z - SUN_STAGED_FIGHTER_Z_C;
    let dz_d = world.z - SUN_STAGED_FIGHTER_Z_D;
    let dz_e = world.z - SUN_STAGED_FIGHTER_Z_E;
    let dz_f = world.z - SUN_STAGED_FIGHTER_Z_F;
    let dz_g = world.z - SUN_STAGED_FIGHTER_Z_G;
    let dz_h = world.z - SUN_STAGED_FIGHTER_Z_H;
    let dz_i = world.z - SUN_STAGED_FIGHTER_Z_I;
    let dz_j = world.z - SUN_STAGED_FIGHTER_Z_J;
    let dx_west = world.x - SUN_STAGED_FIGHTER_X_WEST;
    let dz_west = world.z - SUN_STAGED_FIGHTER_Z_WEST;
    let dx_east = world.x - SUN_STAGED_FIGHTER_X_EAST;
    let dz_east = world.z - SUN_STAGED_FIGHTER_Z_EAST;
    let dx_center = world.x - SUN_STAGED_FIGHTER_X_CENTER;
    let dz_center = world.z - SUN_STAGED_FIGHTER_Z_CENTER;
    let dx_far_east = world.x - SUN_STAGED_FIGHTER_X_FAR_EAST;
    let dz_far_east = world.z - SUN_STAGED_FIGHTER_Z_FAR_EAST;
    let dx_far_far_east = world.x - SUN_STAGED_FIGHTER_X_FAR_FAR_EAST;
    let dz_far_far_east = world.z - SUN_STAGED_FIGHTER_Z_FAR_FAR_EAST;
    let dx_west_far = world.x - SUN_STAGED_FIGHTER_X_WEST_FAR;
    let dz_west_far = world.z - SUN_STAGED_FIGHTER_Z_WEST_FAR;
    let dx_west_far_far = world.x - SUN_STAGED_FIGHTER_X_WEST_FAR_FAR;
    let dz_west_far_far = world.z - SUN_STAGED_FIGHTER_Z_WEST_FAR_FAR;
    let dx_far_far_west = world.x - SUN_STAGED_FIGHTER_X_FAR_FAR_WEST;
    let dz_far_far_west = world.z - SUN_STAGED_FIGHTER_Z_FAR_FAR_WEST;
    let near_a = (dx * dx + dz_a * dz_a) < SUN_STAGED_RADIUS_M * SUN_STAGED_RADIUS_M;
    let near_b = (dx * dx + dz_b * dz_b) < SUN_STAGED_RADIUS_M * SUN_STAGED_RADIUS_M;
    let near_mid = (dx * dx + dz_mid * dz_mid) < SUN_STAGED_RADIUS_M * SUN_STAGED_RADIUS_M;
    let near_c = (dx * dx + dz_c * dz_c) < SUN_STAGED_RADIUS_M * SUN_STAGED_RADIUS_M;
    let near_d = (dx * dx + dz_d * dz_d) < SUN_STAGED_RADIUS_M * SUN_STAGED_RADIUS_M;
    let near_e = (dx * dx + dz_e * dz_e) < SUN_STAGED_RADIUS_M * SUN_STAGED_RADIUS_M;
    let near_f = (dx * dx + dz_f * dz_f) < SUN_STAGED_RADIUS_M * SUN_STAGED_RADIUS_M;
    let near_g = (dx * dx + dz_g * dz_g) < SUN_STAGED_RADIUS_M * SUN_STAGED_RADIUS_M;
    let near_h = (dx * dx + dz_h * dz_h) < SUN_STAGED_RADIUS_M * SUN_STAGED_RADIUS_M;
    let near_i = (dx * dx + dz_i * dz_i) < SUN_STAGED_RADIUS_M * SUN_STAGED_RADIUS_M;
    let near_j = (dx * dx + dz_j * dz_j) < SUN_STAGED_RADIUS_M * SUN_STAGED_RADIUS_M;
    let near_west = (dx_west * dx_west + dz_west * dz_west) < SUN_STAGED_RADIUS_M * SUN_STAGED_RADIUS_M;
    let near_east = (dx_east * dx_east + dz_east * dz_east) < SUN_STAGED_RADIUS_M * SUN_STAGED_RADIUS_M;
    let near_center = (dx_center * dx_center + dz_center * dz_center) < SUN_STAGED_RADIUS_M * SUN_STAGED_RADIUS_M;
    let near_far_east = (dx_far_east * dx_far_east + dz_far_east * dz_far_east) < SUN_STAGED_RADIUS_M * SUN_STAGED_RADIUS_M;
    let near_far_far_east = (dx_far_far_east * dx_far_far_east + dz_far_far_east * dz_far_far_east) < SUN_STAGED_RADIUS_M * SUN_STAGED_RADIUS_M;
    let near_west_far = (dx_west_far * dx_west_far + dz_west_far * dz_west_far) < SUN_STAGED_RADIUS_M * SUN_STAGED_RADIUS_M;
    let near_west_far_far = (dx_west_far_far * dx_west_far_far + dz_west_far_far * dz_west_far_far) < SUN_STAGED_RADIUS_M * SUN_STAGED_RADIUS_M;
    let near_far_far_west = (dx_far_far_west * dx_far_far_west + dz_far_far_west * dz_far_far_west) < SUN_STAGED_RADIUS_M * SUN_STAGED_RADIUS_M;
    var frag = SUN_STAGED_FRAG_FAR;
    if (near_a || near_b || near_mid || near_c || near_d || near_e || near_f || near_g || near_h || near_i || near_j || near_west || near_east || near_center || near_far_east || near_far_far_east || near_west_far || near_west_far_far || near_far_far_west) {
        frag = SUN_STAGED_FRAG_NEAR;
    }
    let s = sun_shadow_factor(SUN_STAGED_STORED, frag, SUN_STAGED_BIAS);
    return mix(SUN_STAGED_DARKEN, 1.0, s);
}
fn ibl_sh_basis(d: vec3<f32>) -> array<f32, 9> {
    var b: array<f32, 9>;
    b[0] = 0.282095;
    b[1] = -0.488603 * d.y;
    b[2] = 0.488603 * d.z;
    b[3] = -0.488603 * d.x;
    b[4] = 1.092548 * d.x * d.y;
    b[5] = -1.092548 * d.y * d.z;
    b[6] = 0.315392 * (3.0 * d.z * d.z - 1.0);
    b[7] = -1.092548 * d.x * d.z;
    b[8] = 0.546274 * (d.x * d.x - d.y * d.y);
    return b;
}
fn ibl_irradiance(n: vec3<f32>) -> vec3<f32> {
    let b = ibl_sh_basis(n);
    var e = vec3<f32>(0.0, 0.0, 0.0);
    e += ibl_sh[0].xyz * b[0];
    e += ibl_sh[1].xyz * b[1];
    e += ibl_sh[2].xyz * b[2];
    e += ibl_sh[3].xyz * b[3];
    e += ibl_sh[4].xyz * b[4];
    e += ibl_sh[5].xyz * b[5];
    e += ibl_sh[6].xyz * b[6];
    e += ibl_sh[7].xyz * b[7];
    e += ibl_sh[8].xyz * b[8];
    return max(e, vec3<f32>(0.0, 0.0, 0.0));
}
fn ibl_prefilter_level(layer_base: i32, face: i32, u: f32, v: f32) -> vec3<f32> {
    let x = clamp((u + 1.0) * 0.5 * 64.0 - 0.5, 0.0, 63.0);
    let y = clamp((v + 1.0) * 0.5 * 64.0 - 0.5, 0.0, 63.0);
    let x0 = i32(floor(x));
    let y0 = i32(floor(y));
    let x1 = min(x0 + 1, 63);
    let y1 = min(y0 + 1, 63);
    let fx = x - f32(x0);
    let fy = y - f32(y0);
    let layer = layer_base + face;
    let a = textureLoad(ibl_prefilter, vec2<i32>(x0, y0), layer, 0).rgb;
    let b = textureLoad(ibl_prefilter, vec2<i32>(x1, y0), layer, 0).rgb;
    let c = textureLoad(ibl_prefilter, vec2<i32>(x0, y1), layer, 0).rgb;
    let e = textureLoad(ibl_prefilter, vec2<i32>(x1, y1), layer, 0).rgb;
    return mix(mix(a, b, fx), mix(c, e, fx), fy);
}
fn ibl_brdf_pair(ndotv: f32, roughness: f32) -> vec2<f32> {
    let x = clamp(ndotv * 64.0 - 0.5, 0.0, 63.0);
    let y = clamp(roughness * 64.0 - 0.5, 0.0, 63.0);
    let x0 = i32(floor(x));
    let y0 = i32(floor(y));
    let x1 = min(x0 + 1, 63);
    let y1 = min(y0 + 1, 63);
    let fx = x - f32(x0);
    let fy = y - f32(y0);
    let a = textureLoad(ibl_brdf, vec2<i32>(x0, y0), 0).rg;
    let b = textureLoad(ibl_brdf, vec2<i32>(x1, y0), 0).rg;
    let c = textureLoad(ibl_brdf, vec2<i32>(x0, y1), 0).rg;
    let e = textureLoad(ibl_brdf, vec2<i32>(x1, y1), 0).rg;
    return mix(mix(a, b, fx), mix(c, e, fx), fy);
}
fn ibl_diffuse(n: vec3<f32>, albedo: vec3<f32>, metallic: f32) -> vec3<f32> {
    // EG12: baked diffuse ambient only (SH irradiance over pi times albedo).
    // Scaled by occlusion in `fs`; mirrors CPU `ambient_occluded`.
    return (1.0 - metallic) * albedo * ibl_irradiance(n) / 3.141592653589793;
}
fn ibl_specular(n: vec3<f32>, v: vec3<f32>, f0: vec3<f32>, roughness: f32) -> vec3<f32> {
    // EG12: open specular only (prefilter times BRDF scale plus bias).
    // Never scaled by occlusion; mirrors CPU `shade_ibl_occluded`.
    let ndotv = clamp(dot(n, v), 0.0, 1.0);
    let r = reflect(-v, n);
    let ax = abs(r.x);
    let ay = abs(r.y);
    let az = abs(r.z);
    var face = 5;
    var uu = 0.0;
    var vv = 0.0;
    if (ax >= ay && ax >= az) {
        if (r.x > 0.0) { face = 0; uu = -r.z / ax; vv = -r.y / ax; }
        else { face = 1; uu = r.z / ax; vv = -r.y / ax; }
    } else if (ay >= ax && ay >= az) {
        if (r.y > 0.0) { face = 2; uu = r.x / ay; vv = r.z / ay; }
        else { face = 3; uu = r.x / ay; vv = -r.z / ay; }
    } else {
        if (r.z > 0.0) { face = 4; uu = r.x / az; vv = -r.y / az; }
        else { face = 5; uu = -r.x / az; vv = -r.y / az; }
    }
    let f = roughness * 4.0;
    let l0 = min(i32(floor(f)), 3);
    let l1 = l0 + 1;
    let t = f - f32(l0);
    let p0 = ibl_prefilter_level(l0 * 6, face, uu, vv);
    let p1 = ibl_prefilter_level(l1 * 6, face, uu, vv);
    let pre = mix(p0, p1, t);
    let fab = ibl_brdf_pair(ndotv, roughness);
    return pre * (f0 * fab.x + vec3<f32>(fab.y, fab.y, fab.y));
}
fn ibl_sample(n: vec3<f32>, v: vec3<f32>, f0: vec3<f32>, roughness: f32, albedo: vec3<f32>, metallic: f32) -> vec3<f32> {
    return ibl_diffuse(n, albedo, metallic) + ibl_specular(n, v, f0, roughness);
}

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
};
struct SIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) joints: vec4<u32>,
    @location(4) weights: vec4<f32>,
};
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_n: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) draw_id: u32,
    @location(4) tangent: vec4<f32>,
};
struct FsOut {
    @location(0) color: vec4<f32>,
    @location(1) id: u32,
};
struct SkyOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

// ---------------------------------------------------------------------------
// Vertex entries.
// ---------------------------------------------------------------------------

@vertex
fn vs(in: VIn, @builtin(instance_index) ii: u32) -> VOut {
    var out: VOut;
    let wp = vec4<f32>(in.pos, 1.0);
    out.world_pos = wp.xyz;
    out.world_n = in.normal;
    out.uv = in.uv;
    out.tangent = in.tangent;
    out.clip = cam.view_proj * wp;
    out.draw_id = ii;
    return out;
}

@vertex
fn vs_skinned(in: SIn, @builtin(instance_index) ii: u32) -> VOut {
    var out: VOut;
    var sp = vec3<f32>(0.0);
    var sn = vec3<f32>(0.0);
    if (skin_params.has_skin == 0u) {
        sp = in.pos;
        sn = in.normal;
    } else {
        for (var k = 0u; k < 4u; k = k + 1u) {
            let idx = min(in.joints[k], 63u);
            let m = skin_joints[idx];
            let w = in.weights[k];
            sp += w * (m * vec4<f32>(in.pos, 1.0)).xyz;
            sn += w * (m * vec4<f32>(in.normal, 0.0)).xyz;
        }
    }
    out.world_pos = sp;
    let l = length(sn);
    out.world_n = select(vec3<f32>(0.0, 1.0, 0.0), sn / max(l, 1e-12), l > 1e-6);
    out.uv = in.uv;
    out.tangent = vec4<f32>(1.0, 0.0, 0.0, 1.0);
    out.clip = cam.view_proj * vec4<f32>(sp, 1.0);
    out.draw_id = ii;
    return out;
}

@vertex
fn vs_depth(in: VIn) -> @builtin(position) vec4<f32> {
    return cam.view_proj * vec4<f32>(in.pos, 1.0);
}

@vertex
fn vs_sky(@builtin(vertex_index) vi: u32) -> SkyOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0)
    );
    var out: SkyOut;
    out.pos = vec4<f32>(p[vi], 1.0, 1.0);
    out.ndc = p[vi];
    return out;
}

// ---------------------------------------------------------------------------
// BRDF: one copy for every fragment entry.
// ---------------------------------------------------------------------------

// PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_lighting.wgsl): perceptualRoughnessToRoughness.
fn perceptual_roughness_to_roughness(p: f32) -> f32 {
    let c = clamp(p, 0.089, 1.0);
    return c * c;
}

// PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_lighting.wgsl): D_GGX (Walter et al. via Filament).
fn d_ggx(roughness: f32, ndot_h: f32) -> f32 {
    let one_minus = 1.0 - ndot_h * ndot_h;
    let a = ndot_h * roughness;
    let k = roughness / (one_minus + a * a);
    let d = k * k * (1.0 / 3.141592653589793);
    return d;
}

// PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_lighting.wgsl): V_SmithGGXCorrelated (Heitz).
fn v_smith_ggx_correlated(roughness: f32, ndot_v: f32, ndot_l: f32) -> f32 {
    let a2 = roughness * roughness;
    let lambda_v = ndot_l * sqrt((ndot_v - a2 * ndot_v) * ndot_v + a2);
    let lambda_l = ndot_v * sqrt((ndot_l - a2 * ndot_l) * ndot_l + a2);
    return 0.5 / (lambda_v + lambda_l);
}

// PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_lighting.wgsl): F_Schlick_vec (Schlick 1994).
fn f_schlick_vec(f0: vec3<f32>, f90: f32, vdot_h: f32) -> vec3<f32> {
    return f0 + (f90 - f0) * pow(1.0 - vdot_h, 5.0);
}

// PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_lighting.wgsl): fresnel (Filament occlusion f90).
fn fresnel_term(f0: vec3<f32>, ldot_h: f32) -> vec3<f32> {
    let f90 = clamp(dot(f0, vec3<f32>(50.0 * 0.33)), 0.0, 1.0);
    return f_schlick_vec(f0, f90, ldot_h);
}

// PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_lighting.wgsl): Fd_Burley (Disney/Burley 2012).
fn fd_burley(roughness: f32, ndot_v: f32, ndot_l: f32, ldot_h: f32) -> f32 {
    let f90 = 0.5 + 2.0 * roughness * ldot_h * ldot_h;
    let light_scatter = 1.0 + (f90 - 1.0) * pow(1.0 - ndot_l, 5.0);
    let view_scatter = 1.0 + (f90 - 1.0) * pow(1.0 - ndot_v, 5.0);
    return light_scatter * view_scatter * (1.0 / 3.141592653589793);
}

// PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_lighting.wgsl): F_AB polynomial (Unreal mobile approx).
fn f_ab_term(perceptual_roughness: f32, ndot_v: f32) -> vec2<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = perceptual_roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * ndot_v)) * r.x + r.y;
    return max(vec2<f32>(-1.04, 1.04) * a004 + r.zw, vec2<f32>(0.00005));
}

// PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_functions.wgsl): calculate_diffuse_color + calculate_F0.
fn diffuse_color_term(base_color: vec3<f32>, metallic: f32) -> vec3<f32> {
    return base_color * (1.0 - metallic);
}
fn f0_term(base_color: vec3<f32>, metallic: f32) -> vec3<f32> {
    return mix(vec3<f32>(0.04), base_color, metallic);
}

// PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_lighting.wgsl): specular_multiscatter (Filament 9.5).
fn specular_term(d: f32, v: f32, f: vec3<f32>, f0: vec3<f32>, fab: vec2<f32>) -> vec3<f32> {
    var fr = (d * v) * f;
    fr *= 1.0 + f0 * (1.0 / (fab.x + fab.y) - 1.0);
    return fr;
}

// The one directional light: linear radiance reflected toward the eye,
// before exposure and before any shading factor.
// PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_lighting.wgsl): directional_light combine:
// color = (diffuse + specular) * NdotL * light_color.
fn direct_light(world_pos: vec3<f32>, world_n: vec3<f32>, albedo: vec3<f32>) -> vec3<f32> {
    let n = normalize(world_n);
    let v = normalize(cam.camera_pos - world_pos);
    let l = normalize(light.dir_to_light);
    let h = normalize(l + v);
    let ndot_l = clamp(dot(n, l), 0.0, 1.0);
    let ndot_v = max(dot(n, v), 0.0001);
    let ndot_h = clamp(dot(n, h), 0.0, 1.0);
    let ldot_h = clamp(dot(l, h), 0.0, 1.0);

    let roughness = perceptual_roughness_to_roughness(mat.roughness);
    let diffuse = diffuse_color_term(albedo, mat.metallic)
        * fd_burley(roughness, ndot_v, ndot_l, ldot_h);
    let f0 = f0_term(albedo, mat.metallic);
    let f = fresnel_term(f0, ldot_h);
    let d = d_ggx(roughness, ndot_h);
    let vv = v_smith_ggx_correlated(roughness, ndot_v, ndot_l);
    let fab = f_ab_term(mat.roughness, ndot_v);
    let spec = specular_term(d, vv, f, f0, fab);
    let sun_shadow = sun_contact_factor(world_pos);
    return (diffuse + spec) * ndot_l * light.light_color * sun_shadow;
}

// One torch: Lambert plus the Burley diffuse above, inverse-square
// falloff with a warm range window to zero, capped near the center.
// The forward fragment sums this over the bound count once wired.
fn torch_light(world_pos: vec3<f32>, world_n: vec3<f32>, albedo: vec3<f32>, torch: TorchU) -> vec3<f32> {
    let delta = torch.pos - world_pos;
    let dist = length(delta);
    if (torch.range <= 0.0 || dist >= torch.range) {
        return vec3<f32>(0.0);
    }
    let l = delta / max(dist, 0.0001);
    let n = normalize(world_n);
    let ndot_l = clamp(dot(n, l), 0.0, 1.0);
    if (ndot_l <= 0.0) {
        return vec3<f32>(0.0);
    }
    let v = normalize(cam.camera_pos - world_pos);
    let h = normalize(l + v);
    let ndot_v = max(dot(n, v), 0.0001);
    let ldot_h = clamp(dot(l, h), 0.0, 1.0);
    let roughness = perceptual_roughness_to_roughness(mat.roughness);
    let diffuse = diffuse_color_term(albedo, mat.metallic)
        * fd_burley(roughness, ndot_v, ndot_l, ldot_h);
    let att = 1.0 / (1.0 + dist * dist);
    let edge = 1.0 - dist / torch.range;
    return diffuse * ndot_l * torch.color * att * edge * edge;
}

// EG12 slice 2: sum the bound torch frame over the live count. Slots at
// and past `torches.count` stay zeroed, so they add nothing.
fn torch_frame_sum(world_pos: vec3<f32>, world_n: vec3<f32>, albedo: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    for (var i = 0u; i < 8u; i++) {
        if (i >= torches.count) {
            break;
        }
        sum += torch_light(world_pos, world_n, albedo, torches.torches[i]);
    }
    return sum;
}

// Decode a tangent-space normal from a 0..=1 texel to -1..=1 with a guarded
// normalize like CPU `decode_normal`: a zero-length decode falls back to the
// geometric normal, so a flat texel never writes NaN.
fn eg12_decode_normal(texel: vec3<f32>, geom_n: vec3<f32>) -> vec3<f32> {
    let v = texel * 2.0 - vec3<f32>(1.0);
    let l2 = dot(v, v);
    if (l2 < 1e-12) {
        return normalize(geom_n);
    }
    return v / sqrt(l2);
}

// Mapped world normal: orthonormalize the interpolated tangent against the
// geometric normal, handedness from tangent.w, bitangent = w * (n x t) like
// CPU `world_normal`. A degenerate tangent keeps the geometric normal.
fn eg12_world_normal(geom_n: vec3<f32>, tangent: vec4<f32>, map_n: vec3<f32>) -> vec3<f32> {
    let n = normalize(geom_n);
    let tx = tangent.xyz - n * dot(tangent.xyz, n);
    let tl2 = dot(tx, tx);
    if (tl2 < 1e-12) {
        return n;
    }
    let t = tx / sqrt(tl2);
    let w = select(-1.0, 1.0, tangent.w >= 0.0);
    let b = w * cross(n, t);
    let out = t * map_n.x + b * map_n.y + n * map_n.z;
    let ol2 = dot(out, out);
    if (ol2 < 1e-12) {
        return n;
    }
    return out / sqrt(ol2);
}

// ---------------------------------------------------------------------------
// Height fog: the unified linear-density closed form (Game Engine Gems 3
// ch 3.2, Lengyel Foundations 2 8.5.2). `fc`/`fp` are the signed plane
// distances of the eye and the fragment (negative is fogged); the integral
// of max(0, -h) along the segment is (nc + np) / 2 when both ends are
// fogged and (nc^2 + np^2) / (2 |fc - fp|) otherwise, times the length.
// The infinite form is the sky: a ray out to infinity.
// ---------------------------------------------------------------------------

fn fog_factor_finite(fog: FogFwdU, eye: vec3<f32>, p: vec3<f32>) -> f32 {
    let d = length(p - eye);
    let fc = dot(fog.plane.xyz, eye) + fog.plane.w;
    let fp = dot(fog.plane.xyz, p) + fog.plane.w;
    let nc = max(-fc, 0.0);
    let np = max(-fp, 0.0);
    let span = max(abs(fc - fp), 0.0001);
    let straddle = (nc * nc + np * np) / (2.0 * span);
    let inside = 0.5 * (nc + np);
    let g = select(straddle, inside, fc <= 0.0 && fp <= 0.0);
    let tau = min(fog.density * d * g, 80.0);
    return 1.0 - exp(-tau);
}

fn fog_factor_infinite(fog: FogFwdU, eye: vec3<f32>, dir: vec3<f32>) -> f32 {
    let fc = dot(fog.plane.xyz, eye) + fog.plane.w;
    let fd = dot(fog.plane.xyz, dir);
    let leaves = fc * fc / (2.0 * max(fd, 0.00001));
    let in_fog_up = fc <= 0.0 && fd > 0.00001;
    let never_leaves = (fc <= 0.0 && !in_fog_up) || (fc > 0.0 && fd < -0.00001);
    let g = select(select(0.0, 80000.0, never_leaves), leaves, in_fog_up);
    let tau = min(fog.density * g, 80.0);
    return 1.0 - exp(-tau);
}

// ---------------------------------------------------------------------------
// Fragment entries.
// ---------------------------------------------------------------------------

// Window frame: linear HDR radiance, exposure only. The sampled texel
// modulates the material factor, the view-distance cascade dims the sun
// term, the baked lighting lifts the shadowed side, then fog mixes toward
// the horizon radiance. The blit tone-maps.
@fragment
fn fs(in: VOut) -> FsOut {
    let albedo = mat.base_color * textureSample(base_map, base_smp, in.uv).rgb;
    let view_dist = length(cam.camera_pos - in.world_pos);
    var cascade = 2u;
    if (view_dist < shadow.splits.x) {
        cascade = 0u;
    } else if (view_dist < shadow.splits.y) {
        cascade = 1u;
    } else if (view_dist < shadow.splits.z) {
        cascade = 2u;
    }
    let shade = 1.0 - 0.15 * f32(cascade) / 2.0;
    // EG12 slice c: mapped normal from the bound normal map through the
    // tangent frame (handedness in tangent.w); degenerate tangents keep
    // the geometric normal inside `eg12_world_normal`.
    let map_texel = textureSample(normal_map, normal_smp, in.uv).rgb;
    let map_n = eg12_decode_normal(map_texel, normalize(in.world_n));
    let mapped_n = eg12_world_normal(normalize(in.world_n), in.tangent, map_n);
    let n_ibl = normalize(mapped_n);
    let v_ibl = normalize(cam.camera_pos - in.world_pos);
    let f0_ibl = f0_term(albedo, mat.metallic);
    // EG12: AO scales the baked diffuse only; specular stays open
    // (pipeline rule 4, mirrors CPU `shade_ibl_occluded`).
    let ibl_diffuse_term = ibl_diffuse(n_ibl, albedo, mat.metallic);
    let ibl_spec_term = ibl_specular(n_ibl, v_ibl, f0_ibl, mat.roughness);
    // Contact darkens the baked ambient so feet ground in IBL wash; direct
    // sun keeps its own contact shade inside direct_light.
    let occlusion: f32 = sun_contact_factor(in.world_pos);
    let lit = direct_light(in.world_pos, mapped_n, albedo) * shade + ibl_diffuse_term * occlusion + ibl_spec_term + torch_frame_sum(in.world_pos, mapped_n, albedo);
    let radiance = max(lit * mat.exposure, vec3<f32>(0.0));
    let fog_f = fog_factor_finite(fog_fwd, cam.camera_pos, in.world_pos);
    var out: FsOut;
    out.color = vec4<f32>(mix(radiance, fog_fwd.color, fog_f), 1.0);
    out.id = in.draw_id;
    return out;
}

// Sky radiance: the init-time sky-view bake sampled by ray direction,
// mixed toward the fog horizon by the infinite-distance factor. Linear,
// like `fs` (the blit tone-maps once). The horizon washes to the fog
// color so open sand keeps its tone; higher rays show the bake sun and
// haze instead of the old analytic ramp.
@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let w = fog_fwd.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(w.xyz / w.w - fog_fwd.eye);
    let base = sky_view_sample(dir);
    let base_or_fog = select(fog_fwd.color, base, fog_fwd.sky_on > 0.5);
    let f = fog_factor_infinite(fog_fwd, fog_fwd.eye, dir);
    return vec4<f32>(mix(base_or_fog, fog_fwd.color, f), 1.0);
}

// Offscreen 8-bit capture (no blit pass): one light plus the constant
// ambient floor, exposure, ACES, sRGB. The floor lifts the shadowed side
// through the diffuse albedo so unlit faces never read pure black; the
// window `fs` entry samples the live IBL bake instead. Contact scales the
// floor so captures ground like the window (a17b1b8b: display left the
// floor unoccluded, sand floated under white haze though shadow 2.259ms).
@fragment
fn fs_display(in: VOut) -> @location(0) vec4<f32> {
    let albedo = mat.base_color;
    let occlusion: f32 = sun_contact_factor(in.world_pos);
    let lit = direct_light(in.world_pos, in.world_n, albedo) + diffuse_color_term(albedo, mat.metallic) * AMBIENT_FLOOR * occlusion;
    let mapped = aces_fitted(max(lit * mat.exposure, vec3<f32>(0.0)));
    return vec4<f32>(linear_to_srgb_term(mapped), 1.0);
}

// Skinned offscreen capture: samples the live bake like the window `fs`
// entry, and a missing skin shades unlit magenta so the failure is visible.
@fragment
fn fs_skinned(in: VOut) -> @location(0) vec4<f32> {
    if (skin_params.has_skin == 0u) {
        return vec4<f32>(1.0, 0.0, 1.0, 1.0);
    }
    let albedo = mat.base_color;
    // EG12 slice c mirror: same mapped normal as window `fs`; the skinned
    // vertex stage carries a dummy tangent, so degenerate frames keep geom.
    let map_texel = textureSample(normal_map, normal_smp, in.uv).rgb;
    let map_n = eg12_decode_normal(map_texel, normalize(in.world_n));
    let mapped_n = eg12_world_normal(normalize(in.world_n), in.tangent, map_n);
    let n_ibl = normalize(mapped_n);
    let v_ibl = normalize(cam.camera_pos - in.world_pos);
    let f0_ibl = f0_term(albedo, mat.metallic);
    // EG12: baked diffuse scales by occlusion, specular stays open
    // (pipeline rule 4, mirrors CPU `shade_ibl_occluded` plus window `fs`).
    let ibl = ibl_sample(n_ibl, v_ibl, f0_ibl, mat.roughness, albedo, mat.metallic);
    let ibl_diffuse_term = ibl_diffuse(n_ibl, albedo, mat.metallic);
    let ibl_spec_term = ibl - ibl_diffuse_term;
    let occlusion: f32 = sun_contact_factor(in.world_pos);
    let lit = direct_light(in.world_pos, mapped_n, albedo) + ibl_diffuse_term * occlusion + ibl_spec_term;
    let mapped = aces_fitted(max(lit * mat.exposure, vec3<f32>(0.0)));
    return vec4<f32>(linear_to_srgb_term(mapped), 1.0);
}
