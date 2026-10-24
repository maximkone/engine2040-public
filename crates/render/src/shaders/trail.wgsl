// trail.wgsl: sword ribbon from the game's blade-pair ring (LF14).
// One staged draw: read the base/tip strip the CPU ring built, expand the pair
// across the camera basis at the width scale, and carry the age alpha ramp.
// The fragment shapes it into a soft streak: it fades along its length (newest
// bright, oldest gone), across the blade (the tip carries the light, the hilt
// side and both edges melt), takes the sun's colour, and fades out against the
// frame's stored depth so a body in front hides it and a body it touches gets a
// soft contact instead of a hard slice. Blend is premultiplied: part additive
// light, part alpha, so the streak brightens a dark body and still reads on sand.
// Engine-owned WGSL, fixed math only, no wall clock. The bind that fills
// `TrailU` and the strip storage lives in `present/trail_draw_gpu.rs`; the depth
// bind (group 1) is `present/vfx_soft.rs`.
struct TrailU {
    view_proj: mat4x4<f32>,
    cam_right: vec3<f32>,
    width_scale: f32,
    cam_up: vec3<f32>,
    // Verts staged this frame: the storage buffer is fixed at the ring cap,
    // so the sweep look-ahead must stop at the live count, not `arrayLength`.
    vert_count: u32,
    // `present/vfx_soft.rs` `VfxFrame`: view depth = zparams.y / (z + zparams.x).
    zparams: vec2<f32>,
    depth_on: u32,
    _pad: f32,
    // Linear light (sun catch over pi plus ambient); the streak takes its cast.
    light: vec4<f32>,
};
struct StripVert {
    pos: vec3<f32>,
    alpha: f32,
};
@group(0) @binding(0) var<uniform> trail: TrailU;
@group(0) @binding(1) var<storage, read> strip: array<StripVert>;
@group(1) @binding(0) var scene_depth: texture_depth_2d;

// Peak opacity of the newest, brightest part of the streak.
const TRAIL_PEAK: f32 = 0.55;
// Share of the light that adds instead of blending: the premultiplied alpha
// word keeps (1 - ADD) of the coverage, so 0.5 is half additive.
const TRAIL_ADD: f32 = 0.5;
// Metres over which the streak fades in beyond a surface it touches.
const TRAIL_SOFT_M: f32 = 0.18;
// Steel with a cool bias; the sun's cast multiplies it.
const TRAIL_STEEL: vec3<f32> = vec3<f32>(0.86, 0.92, 1.0);

struct TrailOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) alpha: f32,
    @location(1) side: f32,
};
@vertex
fn trail_vs(@builtin(vertex_index) vi: u32) -> TrailOut {
    // Two verts per output point: even is the posed base, odd the width curve.
    let point = vi >> 1u;
    let side = f32(vi & 1u);
    let base = strip[point * 2u];
    let tip = strip[point * 2u + 1u];
    var along = tip.pos - base.pos;
    let ahead = point * 2u + 2u;
    if (ahead < trail.vert_count) {
        along = strip[ahead].pos - base.pos;
    }
    // Camera-facing perpendicular: the ribbon turns to the viewer, never edge-on.
    let to_cam = cross(trail.cam_right, trail.cam_up);
    var across = cross(along, to_cam);
    let span = length(across);
    if (span > 1e-5) { across = across / span; }
    let half_w = 0.5 * trail.width_scale * distance(base.pos, tip.pos);
    let edge = side * 2.0 - 1.0;
    let world = mix(base.pos, tip.pos, side) + across * (edge * half_w);
    var out: TrailOut;
    out.clip = trail.view_proj * vec4<f32>(world, 1.0);
    out.alpha = mix(base.alpha, tip.alpha, side);
    out.side = side;
    return out;
}

// View depth of the surface the frame stored under this pixel, in metres.
fn trail_scene_depth(frag_xy: vec2<f32>) -> f32 {
    let dims = vec2<i32>(textureDimensions(scene_depth));
    let px = clamp(vec2<i32>(frag_xy), vec2<i32>(0), dims - vec2<i32>(1));
    let z = textureLoad(scene_depth, px, 0);
    return trail.zparams.y / (z + trail.zparams.x);
}

@fragment
fn trail_fs(in: TrailOut) -> @location(0) vec4<f32> {
    let age = clamp(in.alpha, 0.0, 1.0);
    // Along the sweep: an eased ramp, newest bright, oldest gone; the very
    // newest pair is eased in so the head has no hard cut.
    let tail = age * age * (3.0 - 2.0 * age);
    let head = 1.0 - 0.75 * smoothstep(0.9, 1.0, age);
    // Across the blade: nothing at the hilt, full toward the tip, a soft edge
    // at the very tip so the silhouette is a falloff, not a border.
    let s = clamp(in.side, 0.0, 1.0);
    let profile = smoothstep(0.0, 0.55, s) * (1.0 - smoothstep(0.82, 1.0, s));
    var cover = TRAIL_PEAK * tail * head * profile;
    // Soft contact: fade to nothing where a surface is at or in front of the
    // ribbon, ease back in over TRAIL_SOFT_M of clear air.
    if (trail.depth_on != 0u) {
        let gap = trail_scene_depth(in.clip.xy) - 1.0 / in.clip.w;
        cover = cover * smoothstep(0.0, TRAIL_SOFT_M, gap + 0.03);
    }
    // The sun's cast, normalised so a dim light tints and does not darken it.
    let sun_cast = trail.light.rgb / max(max(trail.light.r, trail.light.g), max(trail.light.b, 1e-3));
    let tint = TRAIL_STEEL * mix(vec3<f32>(1.0), sun_cast, 0.5);
    // Premultiplied: rgb carries the full light, alpha keeps (1 - ADD) of it.
    return vec4<f32>(tint * cover * (1.0 + 0.6 * tail), cover * (1.0 - TRAIL_ADD));
}
