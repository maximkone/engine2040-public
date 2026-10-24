// forward.wgsl: the one forward shader. Every forward-style pass in the
// engine compiles this file (tonemap.wgsl is prepended at compile time for
// the display entries); a pass picks its stage functions by entry point.
//
//   vertex    vs            static mesh: world-space positions
//             vs_skinned    joint palette (storage) + 4 weights per vertex
//                           + the mesh tangent, skinned with the joints
//             vs_depth      position only, for depth-only passes
//             vs_sky        fullscreen triangle at the far plane
//   fragment  fs            window frame: textured, live sun-map shadow and
//                           contact shade, ambient
//                           floor, per-fragment height fog; writes LINEAR
//                           HDR radiance (exposure only) into Rgba16Float.
//                           No tone map here: the blit does ACES + sRGB.
//             fs_sky        sky radiance (infinite-distance fog form)
//             fs_display    offscreen 8-bit capture: one light, ACES + sRGB
//             fs_skinned    as fs_display, magenta without a skin
//             fs_gbuffer    A3 debug channels: normal, albedo, material id
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
    // LF8 shading family (bytes 24..28): 0 stone, 1 skin, 2 metal, 3 cloth.
    // The Rust owner is `crate::material_kind::MaterialKind`.
    kind: u32,
    // LF8 parameter (bytes 28..32): skin thickness or metal anisotropy, both
    // in 0..=1. Stone and cloth read none, so their draws keep both words 0.
    param: f32,
};
struct ShadowU {
    splits: vec4<f32>,
    active_depth: f32,
    near: f32,
    far: f32,
    size: f32,
    // Live contact feet (see `sun_contact_factor`): x, z, radius, strength.
    feet: array<vec4<f32>, 8>,
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

// Live lighting from the baked set in the forward group: irradiance from
// the SH uniform plus split-sum specular from the prefiltered array and
// the BRDF pair. Linear radiance, exposure applied by the caller.
@group(0) @binding(9) var ibl_prefilter: texture_2d_array<f32>;
@group(0) @binding(10) var ibl_brdf: texture_2d<f32>;
@group(0) @binding(11) var ibl_sampler: sampler;
@group(0) @binding(12) var<uniform> ibl_sh: array<vec4<f32>, 9>;
// Single sun shadow map: depth plus comparison sampler for the arena sun
// pass (`sun_shadow::SunShadowSubmitter` draws it). Bound at 13/14 so the
// live `fs` layout stays clean (9-12 owned by IBL); `sun_shadow_at` below
// reads it with a 3x3 `textureSampleCompare` PCF, and the pass frame at 20
// carries the light view-projection so a fragment finds its texel. The
// grounding shade under the fighters (`sun_contact_factor`) is the live feet
// block of `shadow`, not this map. The compare bias is the 2^-19 constant
// plus the slope term `sun_bias` adds; the normal offset moves the sample,
// never the compare. `SUN_SHADOW_BIAS`, `SUN_SHADOW_SLOPE_SCALE` and
// `SUN_SHADOW_NORMAL_OFFSET_TEXELS` are the CPU owners.
// SW1: the map is a layer array, near to far (`SUN_LAYERS`); `sun_shadow_at`
// samples the nearest layer that holds the point.
@group(0) @binding(13) var sun_shadow_map: texture_depth_2d_array;
@group(0) @binding(14) var sun_shadow_sampler: sampler_comparison;
// EG11b: the live sun pass frame. `view_proj` is the matrix the depth pass
// drew with, `enabled` is 0 on a frame that skipped the pass (the reader
// then returns fully lit), `texel` is 1/edge of the map in texels,
// `normal_offset` is the sample shift in meters and `slope_scale` the slope
// bias per unit tan(theta) in normalized depth.
struct SunFwdU {
    view_proj: mat4x4<f32>,
    bias: f32,
    enabled: f32,
    texel: f32,
    normal_offset: f32,
    slope_scale: f32,
    uv_scale: f32,
    _pad1: f32,
    _pad2: f32,
    // SW1: the matrices layers 1 and 2 were last drawn with (`view_proj` is
    // layer 0), then x = layers drawn (1..3), y and z = the span of layers 1
    // and 2 over layer 0 (`SUN_LAYER_SCALE`).
    far_vp: array<mat4x4<f32>, 2>,
    layers: vec4<f32>,
};
@group(0) @binding(20) var<uniform> sun_fwd: SunFwdU;
// LF5 motion: the previous frame's view-projection and the previous joint
// palette. The forward vertex entries read them to write where this
// fragment stood a frame ago; every other pass leaves 21/22 unread.
struct PrevCameraU {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(21) var<uniform> prev_cam: PrevCameraU;
@group(0) @binding(22) var<storage, read> prev_skin_joints: array<mat4x4<f32>, 64>;
// LF5b: this frame's un-jittered view-projection. The forward entries rasterize
// with the jittered `cam` above and build motion from `steady_cam`, so the
// Halton offset never enters a motion vector.
@group(0) @binding(23) var<uniform> steady_cam: PrevCameraU;
// LF1 live sky: the one init-time sky-view texture (64x32 Rgba32Float,
// row 0 at the zenith, same backing the fog horizon reads). Sampled by
// ray direction with manual bilinear (textureLoad, U wraps, V clamps),
// the CPU `sample_look_skybox` twin. Linear HDR, no tonemap here.
@group(0) @binding(15) var sky_view: texture_2d<f32>;
// EG12 slice 2: live torch block at 16, first of 16-19 (rest reserved
// for slice-c maps). Staged per frame in `submit.rs`, zeroed for clears.
@group(0) @binding(16) var<uniform> torches: TorchBlock;
// EG12 slice c: tangent-space normal map plus its Repeat sampler at 17/18
// (19 is the VR2 ORM map). Data maps stay linear; the CPU
// `decode_normal`/`world_normal` mirrors in `forward_bind.rs` pin the math.
// The `fs` perturbation plus the layout entries land with the bind slice;
// until then no entry samples them, so pipelines stay validation-clean.
@group(0) @binding(17) var normal_map: texture_2d<f32>;
@group(0) @binding(18) var normal_smp: sampler;
// VR2: packed ORM map (R occlusion, G perceptual roughness, B metallic, linear)
// at 19, sampled with the normal map's Repeat sampler. The texel scales the
// material factors; a draw with no set binds a 1x1 white texel (identity).
// `orm_scale` carries it into the BRDF helpers below; `fs` writes it once,
// every other entry keeps the identity default.
@group(0) @binding(19) var orm_map: texture_2d<f32>;
// LF3 landing 2: the screen-space occlusion field the forward pass reads
// for its ambient. It sits in group 1 because it belongs to the frame, not
// to a draw: the live frame binds it after a depth prepass feeds the GTAO
// kernel, and the field is the same for every draw of the frame.
@group(1) @binding(0) var ao_tex: texture_2d<f32>;
var<private> orm_scale: vec3<f32> = vec3<f32>(1.0, 1.0, 1.0);
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
// Snap to the centre of the texel that holds `v`, so the taps below are that
// texel and its neighbours: the compare grid stays on whole texels as the
// camera moves and does not slide by a texel inside one.
fn snap_texel(v: f32, texel: f32) -> f32 {
    if (texel <= 0.0) {
        return v;
    }
    return (floor(v / texel) + 0.5) * texel;
}
// Constant plus slope-scaled compare bias. A surface tilted against the light
// spans more depth per texel, so its bias grows with tan(theta); the clamp
// caps a face-on or back-facing surface instead of letting it diverge.
fn sun_bias(n: vec3<f32>) -> f32 {
    let no_l = clamp(dot(n, normalize(light.dir_to_light)), 0.0, 1.0);
    let tan_theta = sqrt(max(1.0 - no_l * no_l, 0.0)) / max(no_l, 0.05);
    return sun_fwd.bias + sun_fwd.slope_scale * tan_theta;
}
// Sun-shadow compare filter, one pipeline per tier (SW92): 0 = 3x3 grid,
// 1 = four corners rotated by the 4x4 Bayer angle, 2 = two taps on one fixed
// diagonal (the web arm: no per-pixel noise to survive a missing TAA pass).
override SHADOW_FILTER: u32 = 0u;
// 4x4 Bayer matrix, the same table `lod_dither.wgsl` carries: its value turns
// the rotated kernel by a stable, TAA-friendly angle per texel.
const SHADOW_BAYER4: array<u32, 16> = array<u32, 16>(
    0u, 8u, 2u, 10u, 12u, 4u, 14u, 6u, 3u, 11u, 1u, 9u, 15u, 7u, 13u, 5u,
);
// One compare tap at `off` texels from `base`, sharing the caller's bias.
fn sun_compare(base: vec2<f32>, off: vec2<f32>, frag_depth: f32, bias: f32, layer: u32) -> f32 {
    return textureSampleCompare(
        sun_shadow_map, sun_shadow_sampler, (base + off) * sun_fwd.uv_scale, layer, frag_depth - bias,
    );
}
fn sun_shadow_pcf(uv: vec2<f32>, frag_depth: f32, bias: f32, layer: u32) -> f32 {
    // `uv` and `texel` live in the live map's own space (0..1 over `edge`
    // texels); a tier below the texture's size draws into its low corner, so
    // the sample position scales by `uv_scale` (1 when the map fills it).
    let texel = vec2<f32>(max(sun_fwd.texel, 1.0 / 4096.0));
    let base = vec2<f32>(snap_texel(uv.x, texel.x), snap_texel(uv.y, texel.y));
    // Bayer cell of the base texel: the rotation is a property of the texel,
    // not the frame, so the pattern never crawls.
    let cell = vec2<u32>(
        u32(max(floor(base.x / texel.x), 0.0)) & 3u,
        u32(max(floor(base.y / texel.y), 0.0)) & 3u,
    );
    let bayer = f32(SHADOW_BAYER4[cell.y * 4u + cell.x]);
    let angle = (bayer + 0.5) * 0.39269908; // 2*PI/16 per row
    let c = cos(angle);
    let s = sin(angle);
    if (SHADOW_FILTER == 2u) {
        // Two taps on the fixed diagonal: a gradient across the contact, no
        // per-pixel rotation (the web arm has no history to average noise).
        let a = sun_compare(base, vec2<f32>(-0.5, -0.5) * texel, frag_depth, bias, layer);
        let b = sun_compare(base, vec2<f32>(0.5, 0.5) * texel, frag_depth, bias, layer);
        return (a + b) * 0.5;
    }
    if (SHADOW_FILTER == 1u) {
        // Four corners of a 1x1 texel box, rotated: 9 compares become 4.
        var lit = 0.0;
        for (var i = 0; i < 4; i = i + 1) {
            let cx = select(-0.5, 0.5, (i & 1) == 1);
            let cy = select(-0.5, 0.5, (i & 2) == 2);
            let off = vec2<f32>(cx * c - cy * s, cx * s + cy * c) * texel;
            lit += sun_compare(base, off, frag_depth, bias, layer);
        }
        return lit * 0.25;
    }
    var lit = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let off = vec2<f32>(f32(x), f32(y)) * texel;
            lit += sun_compare(base, off, frag_depth, bias, layer);
        }
    }
    return lit / 9.0;
}
// Sun visibility at `world_pos` from the live depth map: 1 lit, 0 shadowed.
// A frame that skipped the pass reads fully lit, and so does a point whose
// ortho depth falls outside the map, so the term never leaks past the pit
// the map covers. The sample stays in uniform control flow: the outside case
// is a select, not a branch around the compare. The sample moves along the
// surface normal by one texel (`normal_offset`) before it projects, so a face
// does not read the depth it wrote itself.
// SW1: of the `layers.x` drawn layers the nearest one that holds the point,
// two texels inside its edge so the PCF taps stay on the map, is the one
// sampled; a far layer's texel is wider by its span, so is its normal offset.
// The pick branches on values only; every compare runs after it, in uniform
// control flow.
fn sun_layer_vp(layer: u32) -> mat4x4<f32> {
    if (layer == 0u) {
        return sun_fwd.view_proj;
    }
    return sun_fwd.far_vp[min(layer, 2u) - 1u];
}
fn sun_layer_holds(layer: u32, p: vec3<f32>) -> bool {
    let clip = sun_layer_vp(layer) * vec4<f32>(p, 1.0);
    let w = max(clip.w, 0.0001);
    let uv = vec2<f32>(clip.x / w * 0.5 + 0.5, 0.5 - clip.y / w * 0.5);
    let depth = clip.z / w;
    let m = 2.0 * max(sun_fwd.texel, 1.0 / 4096.0);
    return all(uv >= vec2<f32>(m)) && all(uv <= vec2<f32>(1.0 - m)) && depth >= 0.0
        && depth <= 1.0;
}
fn sun_layer_for(p: vec3<f32>) -> u32 {
    let count = u32(clamp(sun_fwd.layers.x, 1.0, 3.0));
    var layer = 0u;
    if (count > 1u && !sun_layer_holds(0u, p)) {
        layer = 1u;
        if (count > 2u && !sun_layer_holds(1u, p)) {
            layer = 2u;
        }
    }
    return layer;
}
fn sun_shadow_at(world_pos: vec3<f32>, normal: vec3<f32>) -> f32 {
    if (sun_fwd.enabled < 0.5) {
        return 1.0;
    }
    let layer = sun_layer_for(world_pos);
    let span = select(1.0, select(sun_fwd.layers.y, sun_fwd.layers.z, layer == 2u), layer > 0u);
    let n_len = max(length(normal), 0.000001);
    let sample_pos = world_pos + normal * (sun_fwd.normal_offset * span / n_len);
    let clip = sun_layer_vp(layer) * vec4<f32>(sample_pos, 1.0);
    let w = max(clip.w, 0.0001);
    // Texture v runs down and NDC y runs up; the depth pass drew through the
    // same viewport, so the flip is the whole difference.
    let uv = vec2<f32>(clip.x / w * 0.5 + 0.5, 0.5 - clip.y / w * 0.5);
    let depth = clamp(clip.z / w, 0.0, 1.0);
    let inside = step(0.0, uv.x) * step(uv.x, 1.0) * step(0.0, uv.y) * step(uv.y, 1.0);
    let uv_in = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0));
    let lit = sun_shadow_pcf(uv_in, depth, sun_bias(normal), layer);
    return select(1.0, lit, inside > 0.5);
}
// Live contact shade (presentation only). Sand under a fighter keeps less of
// the sun and the baked ambient so the body grounds. The feet come from the
// CPU every frame (`shadow.feet`, one `vec4` per slot: x and z in meters,
// radius in meters, strength; strength 0 is an off slot), written by
// `Presenter::set_contact_feet` from where the fighters really stand. There
// are no fixed positions: with no feet all the sand stays lit. The falloff is
// a smoothstep of the distance, so a blob has a soft edge, and it fades out up
// the body between 0.1 m and 0.3 m. Mirrors `contact_feet::contact_factor`
// (the CPU oracle; its parity test pins these constants).
// Only the window `fs` entry reads the feet block (group 0 binding 3): the
// offscreen `fs_display` and `fs_skinned` pipelines of `v1_gpu` and `v1_skin`
// bind no shadow uniform, so they keep the neutral 1.0 that `contact_live`
// starts at, and every consumer reads `sun_contact_factor` the same way.
const CONTACT_FEET_MAX: u32 = 8u;
var<private> contact_live: f32 = 1.0;
fn contact_from_feet(world: vec3<f32>) -> f32 {
    let ground = 1.0 - smoothstep(0.1, 0.3, world.y);
    if (ground <= 0.0) {
        return 1.0;
    }
    var keep = 1.0;
    for (var i = 0u; i < CONTACT_FEET_MAX; i = i + 1u) {
        let foot = shadow.feet[i];
        let fall = smoothstep(0.0, 1.0, 1.0 - length(world.xz - foot.xy) / max(foot.z, 0.001));
        keep = keep * (1.0 - foot.w * ground * fall);
    }
    return keep;
}
fn sun_contact_factor(world: vec3<f32>) -> f32 {
    return contact_live;
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
// Baked-ambient gain for the frame. `ibl_sh[0].w` is the cut word: the CPU
// packer leaves every pad word at 0.0 (lit as baked) and the live frame
// writes 1.0 there while the pass-truth gate renders its IBL-off frame.
fn ibl_gain() -> f32 {
    return 1.0 - clamp(ibl_sh[0].w, 0.0, 1.0);
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
    return max(e, vec3<f32>(0.0, 0.0, 0.0)) * ibl_gain();
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
// RL47 A1: the multiple-scatter gain `1 + F0 * (1 / E - 1)` (Filament 9.5)
// both specular lobes share. E is the lobe's directional albedo at F0 = 1; a
// valid table holds E >= 0.31, so the floor only bites a degenerate texel and
// caps the gain at 3 instead of 10000. CPU twin: `ibl::spec_lobe::ms_gain`
// (`ibl::energy_compensation` calls it); the device test
// `ms_gain_wgsl_matches_the_cpu_twin` runs this very function.
const MS_E_FLOOR: f32 = 0.25;
fn ms_gain(f0: vec3<f32>, e: f32) -> vec3<f32> {
    return vec3<f32>(1.0) + f0 * (1.0 / max(e, MS_E_FLOOR) - 1.0);
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
    // RL47: multiple-scatter gain (Filament 9.5, the same factor
    // `specular_term` puts on the direct lobe): E is this lobe's directional
    // albedo, so a roughness-1 white metal returns its light instead of 0.31.
    let comp = ms_gain(f0, fab.x + fab.y);
    return pre * (f0 * fab.x + vec3<f32>(fab.y, fab.y, fab.y)) * comp * ibl_gain();
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
// LF5 motion: the static forward pipeline also reads the previous world
// position from its own compact vec3 stream, so a refreshed body keeps its
// history without a second copy of the lighting vertex.
struct VPrev {
    @location(6) prev_pos: vec3<f32>,
};
struct SIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) joints: vec4<u32>,
    @location(4) weights: vec4<f32>,
    // EG12 slice c: the mesh tangent with handedness in w. It rides the
    // same joint matrices as the normal, so the tangent frame stays
    // orthogonal under skinning.
    @location(5) tangent: vec4<f32>,
};
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_n: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) draw_id: u32,
    @location(4) tangent: vec4<f32>,
    // LF5: this frame's and the previous frame's clip position. Clip
    // coordinates are linear in the world point, so perspective-correct
    // interpolation rebuilds them exactly; the fragment divides per pixel.
    // (An ndc carried through would interpolate a homography and drift on a
    // large triangle.)
    @location(5) cur_clip: vec4<f32>,
    @location(6) prev_clip: vec4<f32>,
};
struct FsOut {
    @location(0) color: vec4<f32>,
    @location(1) id: u32,
    // LF5: uv-space move from the previous frame to this one (y down).
    @location(2) motion: vec2<f32>,
};
struct SkyOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};
// Sky fragment output: color plus the draw-id target. Sky pixels claim no
// draw, so the id stays at the clear value; the 1x sky pipeline carries
// both attachments, the MSAA one ignores the extra location.
struct SkyFsOut {
    @location(0) color: vec4<f32>,
    @location(1) id: u32,
    // LF5: the sky holds still in uv space; a camera-moved sky stays zero.
    @location(2) motion: vec2<f32>,
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
    // No previous position reaches this entry: its passes write no motion.
    out.cur_clip = out.clip;
    out.prev_clip = out.clip;
    return out;
}

// LF5: the static forward entry. Same lighting vertex as `vs`, plus the
// previous world position from its own stream and the previous camera.
@vertex
fn vs_motion(in: VIn, prev: VPrev, @builtin(instance_index) ii: u32) -> VOut {
    var out: VOut;
    let wp = vec4<f32>(in.pos, 1.0);
    out.world_pos = wp.xyz;
    out.world_n = in.normal;
    out.uv = in.uv;
    out.tangent = in.tangent;
    let clip = cam.view_proj * wp;
    out.clip = clip;
    out.draw_id = ii;
    // LF5b: the jitter rides `clip` only; motion uses the steady matrix.
    out.cur_clip = steady_cam.view_proj * wp;
    // A fragment behind the previous camera keeps this frame's ndc in `fs`:
    // zero move, never a huge wrap from a negative w.
    out.prev_clip = prev_cam.view_proj * vec4<f32>(prev.prev_pos, 1.0);
    return out;
}

@vertex
fn vs_skinned(in: SIn, @builtin(instance_index) ii: u32) -> VOut {
    var out: VOut;
    var sp = vec3<f32>(0.0);
    var sn = vec3<f32>(0.0);
    var st = vec3<f32>(0.0);
    if (skin_params.has_skin == 0u) {
        sp = in.pos;
        sn = in.normal;
        st = in.tangent.xyz;
    } else {
        for (var k = 0u; k < 4u; k = k + 1u) {
            let idx = min(in.joints[k], 63u);
            let m = skin_joints[idx];
            let w = in.weights[k];
            sp += w * (m * vec4<f32>(in.pos, 1.0)).xyz;
            sn += w * (m * vec4<f32>(in.normal, 0.0)).xyz;
            st += w * (m * vec4<f32>(in.tangent.xyz, 0.0)).xyz;
        }
    }
    out.world_pos = sp;
    let l = length(sn);
    out.world_n = select(vec3<f32>(0.0, 1.0, 0.0), sn / max(l, 1e-12), l > 1e-6);
    out.uv = in.uv;
    // EG12 slice c: the tangent rides the same joint matrices as the
    // normal, so `eg12_world_normal` maps a normal map through the real
    // tangent frame instead of the old fixed +X dummy. A degenerate blend
    // falls back to that +X dummy, never to a NaN.
    let tl = length(st);
    out.tangent = vec4<f32>(
        select(vec3<f32>(1.0, 0.0, 0.0), st / max(tl, 1e-12), tl > 1e-6),
        in.tangent.w);
    out.clip = cam.view_proj * vec4<f32>(sp, 1.0);
    out.draw_id = ii;
    // This entry serves passes without a motion target: zero move.
    out.cur_clip = out.clip;
    out.prev_clip = out.clip;
    return out;
}

// LF5: the skinned forward entry. Same skinning as `vs_skinned`, plus the
// previous frame's palette so a posed body writes its own move.
@vertex
fn vs_skinned_motion(in: SIn, @builtin(instance_index) ii: u32) -> VOut {
    var out: VOut;
    var sp = vec3<f32>(0.0);
    var sn = vec3<f32>(0.0);
    var st = vec3<f32>(0.0);
    var prev_sp = in.pos;
    if (skin_params.has_skin == 0u) {
        sp = in.pos;
        sn = in.normal;
        st = in.tangent.xyz;
    } else {
        prev_sp = vec3<f32>(0.0);
        for (var k = 0u; k < 4u; k = k + 1u) {
            let idx = min(in.joints[k], 63u);
            let m = skin_joints[idx];
            let w = in.weights[k];
            sp += w * (m * vec4<f32>(in.pos, 1.0)).xyz;
            sn += w * (m * vec4<f32>(in.normal, 0.0)).xyz;
            st += w * (m * vec4<f32>(in.tangent.xyz, 0.0)).xyz;
            prev_sp += w * (prev_skin_joints[idx] * vec4<f32>(in.pos, 1.0)).xyz;
        }
    }
    out.world_pos = sp;
    let l = length(sn);
    out.world_n = select(vec3<f32>(0.0, 1.0, 0.0), sn / max(l, 1e-12), l > 1e-6);
    out.uv = in.uv;
    let tl = length(st);
    out.tangent = vec4<f32>(
        select(vec3<f32>(1.0, 0.0, 0.0), st / max(tl, 1e-12), tl > 1e-6),
        in.tangent.w);
    let clip = cam.view_proj * vec4<f32>(sp, 1.0);
    out.clip = clip;
    out.draw_id = ii;
    // LF5b: the jitter rides `clip` only; motion uses the steady matrix.
    out.cur_clip = steady_cam.view_proj * vec4<f32>(sp, 1.0);
    out.prev_clip = prev_cam.view_proj * vec4<f32>(prev_sp, 1.0);
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

// ---------------------------------------------------------------------------
// QL5 potato guard: the one fp16 and specular-aliasing guard of every look
// shader (this file today; a new lit file calls `look_guard` and joins the
// `LOOK_SHADERS` list in `crate::quality`, whose gate fails otherwise).
// ---------------------------------------------------------------------------
// The floors are QL1's (`crates/material/src/compile.rs`): perceptual
// roughness 0.089 and the smallest half normal 2^-14 (6.103515625e-5). Both
// are fp16 safe: they only keep a denominator off zero. QL1's anisotropy
// floor 0.001 guards the material BRDF's anisotropic lobe, which this file
// does not carry.
const LOOK_ROUGHNESS_FLOOR: f32 = 0.089;
const LOOK_MIN_HALF_NORMAL: f32 = 6.103515625e-5;
// The one clamp that is fp32 only: at a grazing view the Smith denominator
// divides by a near-zero NdotV and turns a bright sun into a one-pixel
// sparkle; 0.045 is the Frostbite floor that stops it. fp16 saturates the
// term long before that floor, so the potato preset switches this clamp off
// and keeps the two floors above.
const LOOK_ANALYTIC_GRAZING: f32 = 0.045;
// True on the potato preset (`crate::quality::look_guard_constants`).
override POTATO_GUARD: bool = false;

// One guard for every look term: raise `value` to its floor, then, when the
// term is the analytic-light grazing dot and the preset is not potato, to
// `LOOK_ANALYTIC_GRAZING`. No caller writes a bare clamp of its own.
fn look_guard(value: f32, floor: f32, analytic: bool) -> f32 {
    var out = max(value, floor);
    if (analytic && !POTATO_GUARD) {
        out = max(out, LOOK_ANALYTIC_GRAZING);
    }
    return out;
}

// RL47 A2: geometric specular anti-aliasing (Kaplanyan 2016; Tokuyoshi and
// Kaplanyan 2019; the constants and the form of O3DE `SpecularAA.azsli`). A
// normal that turns fast across one pixel spreads the half vectors the pixel
// integrates, which a point-sampled sharp lobe misses and shows as a sparkle
// that crawls on armour. The kernel (the squared alpha one pixel adds) comes
// from the screen derivatives of the shading normal in `fs`; both lobes (the
// sun's and the IBL's, prefilter mip and BRDF table included) then read the
// raised roughness. `fs_display` and `fs_skinned` take no derivative and keep
// a zero kernel, which leaves their roughness bit for bit. CPU twins:
// `ibl::spec_lobe::spec_aa_kernel` and `spec_aa_perceptual`; the device test
// `spec_aa_wgsl_matches_the_cpu_twin` runs these very functions.
const SPEC_AA_VARIANCE: f32 = 0.25;
const SPEC_AA_THRESHOLD: f32 = 0.18;
var<private> spec_aa_k: f32 = 0.0;
fn spec_aa_kernel(dn_dx: vec3<f32>, dn_dy: vec3<f32>) -> f32 {
    let variance = SPEC_AA_VARIANCE * (dot(dn_dx, dn_dx) + dot(dn_dy, dn_dy));
    return min(2.0 * variance, SPEC_AA_THRESHOLD);
}
// Perceptual roughness p gives alpha = p^2; the kernel adds to alpha^2.
fn spec_aa_perceptual(perceptual: f32, kernel: f32) -> f32 {
    let alpha = perceptual * perceptual;
    let raised = sqrt(sqrt(clamp(alpha * alpha + kernel, 0.0, 1.0)));
    return select(raised, perceptual, kernel <= 0.0);
}

// PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_lighting.wgsl): perceptualRoughnessToRoughness.
fn perceptual_roughness_to_roughness(p: f32) -> f32 {
    let c = clamp(look_guard(p, LOOK_ROUGHNESS_FLOOR, false), 0.0, 1.0);
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
    return f0 + (f90 - f0) * pow(max(1.0 - vdot_h, LOOK_MIN_HALF_NORMAL), 5.0);
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
    fr *= ms_gain(f0, fab.x + fab.y);
    return fr;
}

// LF8 kind words: the Rust owner is `crate::material_kind::MaterialKind`.
const KIND_SKIN: u32 = 1u;
const KIND_METAL: u32 = 2u;
const KIND_CLOTH: u32 = 3u;
// LF8 skin: a game subsurface lobe, not a physically accurate skin model.
// The power shapes the forward-scatter falloff; the tint reddens the
// transmitted term so skin reads apart from stone under the same light.
const SKIN_SUBSURFACE_POWER: f32 = 12.0;
const SKIN_SUBSURFACE_TINT: vec3<f32> = vec3<f32>(1.0, 0.55, 0.45);
// LF8 metal: anisotropic roughness floor, the alpha twin of
// LOOK_ROUGHNESS_FLOOR (Filament's MIN_ROUGHNESS role).
const LOOK_ANISO_MIN_ROUGHNESS: f32 = 0.089 * 0.089;

// LF8 metal: anisotropic GGX (Burley 2012 via Filament `D_GGX_Anisotropic`).
// `at`/`ab` are the alpha roughness along the tangent and bitangent.
fn d_ggx_anisotropic(at: f32, ab: f32, toh: f32, boh: f32, noh: f32) -> f32 {
    let a2 = at * ab;
    let d = vec3<f32>(ab * toh, at * boh, a2 * noh);
    let d2 = dot(d, d);
    let b2 = a2 / max(d2, 1e-12);
    return a2 * b2 * b2 * (1.0 / 3.141592653589793);
}

// LF8 metal: anisotropic Smith-GGX correlated visibility (Heitz 2014 via
// Filament `V_SmithGGXCorrelated_Anisotropic`).
fn v_smith_ggx_correlated_anisotropic(
    at: f32, ab: f32, tov: f32, bov: f32, tol: f32, bol: f32, nov: f32, nol: f32,
) -> f32 {
    let lambda_v = nol * length(vec3<f32>(at * tov, ab * bov, nov));
    let lambda_l = nov * length(vec3<f32>(at * tol, ab * bol, nol));
    return 0.5 / max(lambda_v + lambda_l, 0.0000077);
}

// LF8 cloth: Charlie sheen NDF (Estevez and Kulla 2017 via Filament
// `D_Charlie`).
fn d_charlie(roughness: f32, noh: f32) -> f32 {
    let inv_alpha = 1.0 / max(roughness, 0.0001);
    let sin2h = max(1.0 - noh * noh, 0.0078125);
    return (2.0 + inv_alpha) * pow(sin2h, inv_alpha * 0.5) / (2.0 * 3.141592653589793);
}

// LF8 cloth: Neubelt visibility (Neubelt and Pettineo 2013 via Filament
// `V_Neubelt`).
fn v_neubelt(nov: f32, nol: f32) -> f32 {
    return 1.0 / max(4.0 * (nol + nov - nol * nov), 0.00001532);
}

// The one directional light: linear radiance reflected toward the eye,
// before exposure and before any shading factor.
// PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_lighting.wgsl): directional_light combine:
// color = (diffuse + specular) * NdotL * light_color.
// LF8: `mat.kind` picks the lobe family. Kind 0 is the stone path every
// earlier frame took, byte for byte; the others keep the same diffuse and
// sun factor and swap the specular (skin also adds transmitted scatter).
fn direct_light(world_pos: vec3<f32>, world_n: vec3<f32>, tangent: vec4<f32>, albedo: vec3<f32>, sun_vis: f32) -> vec3<f32> {
    let n = normalize(world_n);
    let v = normalize(cam.camera_pos - world_pos);
    let l = normalize(light.dir_to_light);
    let h = normalize(l + v);
    let ndot_l = clamp(dot(n, l), 0.0, 1.0);
    let ndot_v = look_guard(max(dot(n, v), 0.0001), LOOK_MIN_HALF_NORMAL, true);
    let ndot_h = clamp(dot(n, h), 0.0, 1.0);
    let ldot_h = clamp(dot(l, h), 0.0, 1.0);

    let perceptual = spec_aa_perceptual(clamp(mat.roughness * orm_scale.y, 0.0, 1.0), spec_aa_k);
    let metallic = clamp(mat.metallic * orm_scale.z, 0.0, 1.0);
    let roughness = perceptual_roughness_to_roughness(perceptual);
    let diffuse = diffuse_color_term(albedo, metallic)
        * fd_burley(roughness, ndot_v, ndot_l, ldot_h);
    let f0 = f0_term(albedo, metallic);
    let sun_shadow = sun_contact_factor(world_pos) * sun_vis;

    if (mat.kind == KIND_CLOTH) {
        // Cloth: Charlie sheen specular with F fixed at the dielectric 0.04;
        // the diffuse lobe keeps Burley and is not Fresnel-multiplied.
        let d = d_charlie(roughness, ndot_h);
        let vv = v_neubelt(ndot_v, ndot_l);
        let spec = vec3<f32>(0.04) * d * vv;
        return (diffuse + spec) * ndot_l * light.light_color * sun_shadow;
    }
    let f = fresnel_term(f0, ldot_h);
    let fab = f_ab_term(perceptual, ndot_v);
    if (mat.kind == KIND_SKIN) {
        // Skin: wrap plus thickness transmittance on the diffuse lobe.
        // Transmitted scatter needs no NoL, so the dark side of a body keeps
        // a warm glow; the specular lobe stays today's isotropic GGX.
        let thickness = clamp(mat.param, 0.0, 1.0);
        let scatter_voh = clamp(dot(v, -l), 0.0, 1.0);
        let forward_scatter = exp2((scatter_voh - 1.0) * SKIN_SUBSURFACE_POWER);
        let back_scatter = (ndot_l * thickness + (1.0 - thickness)) * 0.5;
        let subsurface = mix(back_scatter, 1.0, forward_scatter) * (1.0 - thickness);
        let scatter = albedo * SKIN_SUBSURFACE_TINT
            * (subsurface * (1.0 / 3.141592653589793));
        let d = d_ggx(roughness, ndot_h);
        let vv = v_smith_ggx_correlated(roughness, ndot_v, ndot_l);
        let spec = specular_term(d, vv, f, f0, fab);
        return (scatter + (diffuse + spec) * ndot_l) * light.light_color * sun_shadow;
    }
    if (mat.kind == KIND_METAL) {
        // Metal: anisotropic GGX over the mesh tangent frame. The tangent is
        // orthogonalized against the shading normal, handedness is tangent.w
        // and bitangent = w * (n x t), the same frame `eg12_world_normal`
        // builds. A degenerate tangent picks a stable axis, so a mesh with no
        // tangent still shades.
        let aniso = clamp(mat.param, 0.0, 0.95);
        let at = max(roughness * (1.0 + aniso), LOOK_ANISO_MIN_ROUGHNESS);
        let ab = max(roughness * (1.0 - aniso), LOOK_ANISO_MIN_ROUGHNESS);
        let tx = tangent.xyz - n * dot(tangent.xyz, n);
        let tl2 = dot(tx, tx);
        var t = vec3<f32>(1.0, 0.0, 0.0);
        if (tl2 >= 1e-12) {
            t = tx / sqrt(tl2);
        } else {
            let axis = select(
                vec3<f32>(1.0, 0.0, 0.0),
                vec3<f32>(0.0, 1.0, 0.0),
                abs(n.x) > abs(n.y),
            );
            t = normalize(cross(axis, n));
        }
        let w = select(-1.0, 1.0, tangent.w >= 0.0);
        let b = w * cross(n, t);
        let d = d_ggx_anisotropic(at, ab, dot(t, h), dot(b, h), ndot_h);
        let vv = v_smith_ggx_correlated_anisotropic(
            at, ab, dot(t, v), dot(b, v), dot(t, l), dot(b, l), ndot_v, ndot_l,
        );
        let spec = specular_term(d, vv, f, f0, fab);
        return (diffuse + spec) * ndot_l * light.light_color * sun_shadow;
    }
    // Stone (kind 0): the isotropic Burley plus GGX path, unchanged.
    let d = d_ggx(roughness, ndot_h);
    let vv = v_smith_ggx_correlated(roughness, ndot_v, ndot_l);
    let spec = specular_term(d, vv, f, f0, fab);
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
    let roughness = perceptual_roughness_to_roughness(clamp(mat.roughness * orm_scale.y, 0.0, 1.0));
    let diffuse = diffuse_color_term(albedo, clamp(mat.metallic * orm_scale.z, 0.0, 1.0))
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

// RL47 A3: Lagarde specular occlusion (Lagarde and de Rousiers 2014, "Moving
// Frostbite to PBR"; Filament `SpecularAO_Lagarde`). The occlusion field
// darkens the reflection by what the lobe can see: a smooth lobe at a normal
// view reflects the open sky above a cavity, at a grazing view it reflects the
// cavity wall, and a rough lobe averages the whole hemisphere (the result
// tends to `ao`). `alpha` is the lobe's alpha (perceptual roughness squared),
// the argument Filament passes. At ao = 1 the result is 1 for every view and
// roughness. CPU twin: `ibl::spec_lobe::specular_ao`.
fn spec_occlusion(ndotv: f32, ao: f32, alpha: f32) -> f32 {
    return clamp(pow(ndotv + ao, exp2(-16.0 * alpha - 1.0)) - 1.0 + ao, 0.0, 1.0);
}
// RL47 A4: multi-bounce ambient occlusion (Jimenez et al. 2016, "Practical
// Realtime Strategies for Accurate Indirect Occlusion", the fit Filament ships
// as `gtaoMultiBounce`). Light bounces between the walls of a cavity, so a
// bright surface keeps more of its ambient than the plain field says, tinted
// by its own albedo. Never darker than `ao`, never brighter than open (the fit
// reads 1.0005 at ao = 1 for white, so the top is clamped and an open pixel
// stays exactly 1). CPU twin: `ibl::spec_lobe::multi_bounce_ao`.
fn multi_bounce_ao(ao: f32, albedo: vec3<f32>) -> vec3<f32> {
    let a = 2.0404 * albedo - vec3<f32>(0.3324);
    let b = -4.7951 * albedo + vec3<f32>(0.6417);
    let c = 2.7552 * albedo + vec3<f32>(0.6903);
    return min(max(vec3<f32>(ao), ((a * ao + b) * ao + c) * ao), vec3<f32>(1.0));
}

// The occlusion field at one fragment: an exact texel load (the field is
// full resolution and unfilterable). A fragment outside the field reads
// 1.0 (open) rather than a zero texel, so a frame that never runs the
// GTAO keeps its ambient.
fn ao_at(frag_xy: vec2<f32>) -> f32 {
    let dim = vec2<i32>(textureDimensions(ao_tex));
    let at = vec2<i32>(frag_xy);
    if (at.x < 0 || at.y < 0 || at.x >= dim.x || at.y >= dim.y) {
        return 1.0;
    }
    return textureLoad(ao_tex, at, 0).r;
}

// ---------------------------------------------------------------------------
// RL51 surface detail: sand and walls without a visible repeat. The stone
// kind's parameter word names the detail per mesh (`crate::hextile::
// SurfaceDetail`: 0 none, 1 floor, 2 wall); every other kind and every draw
// the presenter never named keeps the plain taps. Hex tiling after Mikkelsen's
// "Practical Real-Time Hex-Tiling" (JCGT 2022), translation only: each of the
// three triangle-grid taps is the tile shifted by a hash of its vertex, so the
// ripples and the mortar lines keep their direction and no normal is rotated.
// The CPU twin in `hextile.rs` pins the hash, the grid and the weights. The
// macro terms break the tile the texture cannot: value noise over the world
// position, plus dust on a wall's foot, so a wall is lit by its own history.
// ---------------------------------------------------------------------------
const DETAIL_FLOOR: u32 = 1u;
const DETAIL_WALL: u32 = 2u;
// Triangle-grid scale: 2 * sqrt(3) cells per tile edge.
const HEX_GRID_SCALE: f32 = 3.4641016;
// The weights are raised to this power (6) before they are normalized, so a
// fragment is mostly one tap and the blend band stays thin: the blend of two
// unrelated taps halves the local contrast, so the band is kept narrow.
fn hex_sharpen(w: vec3<f32>) -> vec3<f32> {
    let w2 = w * w;
    let k = w2 * w2 * w2;
    return k / (k.x + k.y + k.z);
}

fn surface_detail() -> u32 {
    if (mat.kind != 0u) {
        return 0u;
    }
    return u32(max(mat.param, 0.0) + 0.5);
}

// PCG2D (Jarzynski and Olano 2020, "Hash Functions for GPU Rendering"): an
// integer hash, so every adapter gives the same offsets (no sin() hash).
fn pcg2d(v_in: vec2<u32>) -> vec2<u32> {
    var v = v_in * 1664525u + 1013904223u;
    v.x += v.y * 1664525u;
    v.y += v.x * 1664525u;
    v = v ^ (v >> vec2<u32>(16u));
    v.x += v.y * 1664525u;
    v.y += v.x * 1664525u;
    v = v ^ (v >> vec2<u32>(16u));
    return v;
}

// The tile shift of one triangle-grid vertex, in tile units, in 0..1.
fn hex_offset(v: vec2<i32>) -> vec2<f32> {
    return vec2<f32>(pcg2d(bitcast<vec2<u32>>(v))) * (1.0 / 4294967296.0);
}

struct HexCell {
    st1: vec2<f32>,
    st2: vec2<f32>,
    st3: vec2<f32>,
    w: vec3<f32>,
};

// The three taps and their weights for one tile coordinate.
fn hex_cell(st: vec2<f32>) -> HexCell {
    let g = st * HEX_GRID_SCALE;
    let skew = vec2<f32>(g.x, -0.57735027 * g.x + 1.1547005 * g.y);
    let base = floor(skew);
    let fr = skew - base;
    let z = 1.0 - fr.x - fr.y;
    let s = select(0.0, 1.0, z <= 0.0);
    let s2 = 2.0 * s - 1.0;
    let w = vec3<f32>(-z * s2, s - fr.y * s2, s - fr.x * s2);
    let bi = vec2<i32>(base);
    let si = i32(s);
    var cell: HexCell;
    cell.st1 = st + hex_offset(bi + vec2<i32>(si, si));
    cell.st2 = st + hex_offset(bi + vec2<i32>(si, 1 - si));
    cell.st3 = st + hex_offset(bi + vec2<i32>(1 - si, si));
    cell.w = hex_sharpen(w);
    return cell;
}

// One hex-tiled map tap: three gradient samples (the gradient is the plain
// coordinate's, so the mip level matches the untiled draw) blended by weight.
fn hex_sample(tex: texture_2d<f32>, smp: sampler, cell: HexCell, dx: vec2<f32>, dy: vec2<f32>) -> vec3<f32> {
    return textureSampleGrad(tex, smp, cell.st1, dx, dy).rgb * cell.w.x
        + textureSampleGrad(tex, smp, cell.st2, dx, dy).rgb * cell.w.y
        + textureSampleGrad(tex, smp, cell.st3, dx, dy).rgb * cell.w.z;
}

// One value in 0..1 per integer lattice point, from the same PCG2D.
fn lattice_noise(c: vec2<i32>) -> f32 {
    return f32(pcg2d(bitcast<vec2<u32>>(c)).x >> 8u) * (1.0 / 16777216.0);
}

// Smooth value noise over a plane, 0..1.
fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = lattice_noise(c);
    let b = lattice_noise(c + vec2<i32>(1, 0));
    let d = lattice_noise(c + vec2<i32>(0, 1));
    let e = lattice_noise(c + vec2<i32>(1, 1));
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}

// Sand: patches of dry bright sand and trampled damp sand, 3 m and 1 m wide,
// a few percent of tone and a hair of warmth either way.
fn floor_macro(wp: vec3<f32>) -> vec3<f32> {
    let n = 0.6 * value_noise(wp.xz * 0.23) + 0.4 * value_noise(wp.xz * 0.9 + vec2<f32>(17.0, 3.0));
    let lum = mix(0.86, 1.1, n);
    let tint = mix(vec3<f32>(0.97, 0.95, 1.0), vec3<f32>(1.03, 1.01, 0.96), n);
    return vec3<f32>(lum) * tint;
}

// Stone: weathering streaks that run down the face, blotches over it, and
// sand kicked up on the foot of the wall (the first 1.8 m, darker and browner)
// so a wall stands in its sand instead of floating over it.
fn wall_macro(wp: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let up = smoothstep(0.5, 0.9, abs(n.y));
    let along = wp.x * abs(n.z) + wp.z * abs(n.x);
    let q = mix(vec2<f32>(along, wp.y), wp.xz, up);
    let streak = value_noise(vec2<f32>(q.x * 0.9, mix(q.y * 0.18, q.y * 0.9, up)));
    let blotch = value_noise(q * 0.31 + vec2<f32>(5.0, 9.0));
    let lum = mix(0.82, 1.08, 0.55 * streak + 0.45 * blotch);
    let dust = (1.0 - smoothstep(0.0, 1.8, wp.y)) * (0.45 + 0.55 * blotch);
    return vec3<f32>(lum) * mix(vec3<f32>(1.0), vec3<f32>(0.74, 0.62, 0.48), dust * 0.55);
}

fn surface_macro(detail: u32, wp: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    if (detail == DETAIL_FLOOR) {
        return floor_macro(wp);
    }
    if (detail == DETAIL_WALL) {
        return wall_macro(wp, n);
    }
    return vec3<f32>(1.0);
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
// modulates the material factor, the live sun map shadows the direct sun
// term, the baked lighting lifts the shadowed side, then fog mixes toward
// the horizon radiance. The blit tone-maps.
@fragment
fn fs(in: VOut) -> FsOut {
    contact_live = contact_from_feet(in.world_pos);
    // RL51: a mesh the presenter named for surface detail takes the hex-tiled
    // taps of all three maps (the one grid, the one gradient) and the macro
    // terms over its albedo; every other draw keeps its three plain taps.
    let detail = surface_detail();
    var base_texel: vec3<f32>;
    var map_texel: vec3<f32>;
    var orm: vec3<f32>;
    if (detail != 0u) {
        let dx = dpdx(in.uv);
        let dy = dpdy(in.uv);
        let cell = hex_cell(in.uv);
        base_texel = hex_sample(base_map, base_smp, cell, dx, dy);
        map_texel = hex_sample(normal_map, normal_smp, cell, dx, dy);
        orm = hex_sample(orm_map, normal_smp, cell, dx, dy);
    } else {
        base_texel = textureSample(base_map, base_smp, in.uv).rgb;
        map_texel = textureSample(normal_map, normal_smp, in.uv).rgb;
        orm = textureSample(orm_map, normal_smp, in.uv).rgb;
    }
    let albedo = mat.base_color * base_texel * surface_macro(detail, in.world_pos, normalize(in.world_n));
    // EG11b: the live sun depth map is the shadow. The old view-distance
    // cascade dim is gone: the map says where the sun is blocked, so no
    // band has to pretend.
    let sun_vis = sun_shadow_at(in.world_pos, in.world_n);
    // EG12 slice c: mapped normal from the bound normal map through the
    // tangent frame (handedness in tangent.w); degenerate tangents keep
    // the geometric normal inside `eg12_world_normal`.
    let map_n = eg12_decode_normal(map_texel, normalize(in.world_n));
    let mapped_n = eg12_world_normal(normalize(in.world_n), in.tangent, map_n);
    let n_ibl = normalize(mapped_n);
    let v_ibl = normalize(cam.camera_pos - in.world_pos);
    // VR2: the ORM texel (sampled above) scales the roughness and metallic
    // factors and carries the baked occlusion (R) that dims the ambient
    // diffuse below.
    orm_scale = orm;
    // RL47 A2: the pixel's footprint roughens both lobes; the derivatives
    // stay in uniform control flow, at the top of the entry.
    spec_aa_k = spec_aa_kernel(dpdx(n_ibl), dpdy(n_ibl));
    let rough_eff = spec_aa_perceptual(clamp(mat.roughness * orm.y, 0.0, 1.0), spec_aa_k);
    let metal_eff = clamp(mat.metallic * orm.z, 0.0, 1.0);
    let f0_ibl = f0_term(albedo, metal_eff);
    // EG12: AO scales the baked diffuse only; specular stays open
    // (pipeline rule 4, mirrors CPU `shade_ibl_occluded`).
    let ibl_diffuse_term = ibl_diffuse(n_ibl, albedo, metal_eff);
    let ibl_spec_term = ibl_specular(n_ibl, v_ibl, f0_ibl, rough_eff);
    // Contact darkens the baked ambient so feet ground in IBL wash; direct
    // sun keeps its own contact shade inside direct_light.
    let occlusion: f32 = sun_contact_factor(in.world_pos);
    // LF3 landing 2: the GTAO field scales the ambient diffuse only. The
    // direct sun and the torches keep their own light, so a sunlit open
    // texel does not move when the field comes on.
    let ao = ao_at(in.clip.xy);
    // RL47 A3 and A4: the field and the baked occlusion together darken the
    // reflection by what the lobe can see, and lift the ambient diffuse in a
    // cavity by the light that bounces between its walls. The direct sun and
    // the torches keep their own light, as before.
    let ao_all = ao * orm.x;
    let spec_ao = spec_occlusion(clamp(dot(n_ibl, v_ibl), 0.0, 1.0), ao_all, rough_eff * rough_eff);
    let lit = direct_light(in.world_pos, mapped_n, in.tangent, albedo, sun_vis) + ibl_diffuse_term * multi_bounce_ao(ao_all, albedo) * occlusion + ibl_spec_term * spec_ao + torch_frame_sum(in.world_pos, mapped_n, albedo);
    let radiance = max(lit * mat.exposure, vec3<f32>(0.0));
    let fog_f = fog_factor_finite(fog_fwd, cam.camera_pos, in.world_pos);
    var out: FsOut;
    out.color = vec4<f32>(mix(radiance, fog_fwd.color, fog_f), 1.0);
    out.id = in.draw_id;
    // LF5: uv-space move, y down, the pair TAA reprojects. The divide is
    // per fragment: a point behind the previous camera falls back to this
    // frame's ndc, hence zero move.
    let cur_ndc = in.cur_clip.xy / max(in.cur_clip.w, 1e-6);
    var prev_ndc = cur_ndc;
    if (in.prev_clip.w > 1e-6) {
        prev_ndc = in.prev_clip.xy / in.prev_clip.w;
    }
    out.motion = (cur_ndc - prev_ndc) * vec2<f32>(0.5, -0.5);
    return out;
}

// A3 debug G-buffer: the same geometry and material the window `fs` entry
// shades, written as the three channels `buffers::read_frame` reads. No
// default pass uses this entry; a debug capture builds its own pipeline.
//   location 0  RGBA8 world-space normal, octahedral pair in R,G as
//               `share * 0.5 + 0.5` (the Rust `octahedral_xy` /
//               `quantize_byte` pair; the target's round-to-nearest byte
//               lands within one step of the CPU's half-up), B is 1 on the
//               front hemisphere and 0 behind, A is 1.
//   location 1  RGBA8 LINEAR albedo (`base_color * base_map` before
//               lighting, never sRGB), A is 1.
//   location 2  R16Uint material family (`mat.kind`, the LF8 word the
//               shading branches read).
struct GbufferOut {
    @location(0) normal: vec4<f32>,
    @location(1) albedo: vec4<f32>,
    @location(2) id: u32,
};

@fragment
fn fs_gbuffer(in: VOut) -> GbufferOut {
    // Same mapped normal as `fs`, so the debug view shows what lit the pixel.
    let map_texel = textureSample(normal_map, normal_smp, in.uv).rgb;
    let map_n = eg12_decode_normal(map_texel, normalize(in.world_n));
    var n = eg12_world_normal(normalize(in.world_n), in.tangent, map_n);
    let spread = abs(n.x) + abs(n.y) + abs(n.z);
    n = select(vec3<f32>(0.0, 0.0, 1.0), n / max(spread, 1e-12), spread > 1e-12);
    // The back hemisphere folds inward; the reader unfolds it from the pair
    // alone, so the fold must keep the sign of z in the pair itself.
    let back = n.z < 0.0;
    var x = n.x;
    var y = n.y;
    if (back) {
        let ox = x;
        let oy = y;
        x = (1.0 - abs(oy)) * sign(ox);
        y = (1.0 - abs(ox)) * sign(oy);
    }
    var out: GbufferOut;
    out.normal = vec4<f32>(x * 0.5 + 0.5, y * 0.5 + 0.5, select(1.0, 0.0, back), 1.0);
    out.albedo = vec4<f32>(mat.base_color * textureSample(base_map, base_smp, in.uv).rgb, 1.0);
    out.id = mat.kind;
    return out;
}

// Sky radiance: the init-time sky-view bake sampled by ray direction,
// mixed toward the fog horizon by the infinite-distance factor. Linear,
// like `fs` (the blit tone-maps once). The horizon washes to the fog
// color so open sand keeps its tone; higher rays show the bake sun and
// haze instead of the old analytic ramp.
@fragment
fn fs_sky(in: SkyOut) -> SkyFsOut {
    let w = fog_fwd.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(w.xyz / w.w - fog_fwd.eye);
    let base = sky_view_sample(dir);
    let base_or_fog = select(fog_fwd.color, base, fog_fwd.sky_on > 0.5);
    let f = fog_factor_infinite(fog_fwd, fog_fwd.eye, dir);
    var out: SkyFsOut;
    out.color = vec4<f32>(mix(base_or_fog, fog_fwd.color, f), 1.0);
    // No draw claims a sky pixel: hold the id target's clear value.
    out.id = 0xFFFFFFFFu;
    out.motion = vec2<f32>(0.0);
    return out;
}

// Offscreen 8-bit capture (no blit pass): one light plus the baked SH
// ambient, exposure, the crate's one hue-keeping tone map, the shared
// fixed-pattern dither, sRGB. The bake lifts the shadowed side
// through the diffuse albedo so unlit faces never read pure black; the
// window `fs` entry samples the same bake. Contact scales the baked
// diffuse so captures ground like the window. EG12(a) only: direct sun
// stays untouched, AO scales diffuse only, specular stays open.
@fragment
fn fs_display(in: VOut) -> @location(0) vec4<f32> {
    let albedo = mat.base_color;
    let n_ibl = normalize(in.world_n);
    let v_ibl = normalize(cam.camera_pos - in.world_pos);
    let f0_ibl = f0_term(albedo, mat.metallic);
    let ibl_diffuse_term = ibl_diffuse(n_ibl, albedo, mat.metallic);
    let ibl_spec_term = ibl_specular(n_ibl, v_ibl, f0_ibl, mat.roughness);
    let occlusion: f32 = sun_contact_factor(in.world_pos);
    let lit = direct_light(in.world_pos, in.world_n, in.tangent, albedo, 1.0) + ibl_diffuse_term * occlusion + ibl_spec_term;
    let mapped = aces_tonemap(max(lit * mat.exposure, vec3<f32>(0.0)), 1.0, DISPLAY_WHITE);
    // The shared dither: one fixed 4x4 pattern, one 8-bit step, so the
    // 8-bit capture breaks the bands the tone map leaves behind.
    let px = vec2<u32>(u32(in.clip.x), u32(in.clip.y));
    let dithered = mapped + vec3<f32>(dither4(px.x, px.y));
    return vec4<f32>(linear_to_srgb_term(dithered), 1.0);
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
    // EG12: the ORM texel scales roughness/metallic and carries the baked
    // occlusion, exactly like the window `fs` entry.
    let orm = textureSample(orm_map, normal_smp, in.uv).rgb;
    let rough_eff = clamp(mat.roughness * orm.y, 0.0, 1.0);
    let metal_eff = clamp(mat.metallic * orm.z, 0.0, 1.0);
    let f0_ibl = f0_term(albedo, metal_eff);
    // EG12: baked diffuse scales by occlusion, specular stays open
    // (pipeline rule 4, mirrors CPU `shade_ibl_occluded` plus window `fs`).
    let ibl = ibl_sample(n_ibl, v_ibl, f0_ibl, rough_eff, albedo, metal_eff);
    let ibl_diffuse_term = ibl_diffuse(n_ibl, albedo, metal_eff);
    let ibl_spec_term = ibl - ibl_diffuse_term;
    let occlusion: f32 = sun_contact_factor(in.world_pos) * orm.x;
    // EG11b: the v1 skinned layouts carry no sun map bindings, so this entry
    // stays lit; the window parities compare two draws of one path.
    let lit = direct_light(in.world_pos, mapped_n, in.tangent, albedo, 1.0) + ibl_diffuse_term * occlusion + ibl_spec_term;
    let mapped = aces_tonemap(max(lit * mat.exposure, vec3<f32>(0.0)), 1.0, DISPLAY_WHITE);
    // The shared dither, the same fixed 4x4 pattern the display capture
    // and the live blit use.
    let px = vec2<u32>(u32(in.clip.x), u32(in.clip.y));
    let dithered = mapped + vec3<f32>(dither4(px.x, px.y));
    return vec4<f32>(linear_to_srgb_term(dithered), 1.0);
}
