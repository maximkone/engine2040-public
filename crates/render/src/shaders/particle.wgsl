// particle.wgsl: soft round sprites over the seeded pool (RL52, soft pass 2026-10-24).
// Instanced quads fetch one particle by instance id, expand a camera
// billboard, stretch along view-plane velocity (sparks and blood only), and
// shape the quad into a round falloff per emitter kind: a wobbly soft puff for
// dust, a bright hot core for sparks, a tight dark drop for blood, a soft
// additive lick for flame. Dust and blood take the frame's light; sparks and
// flame emit. The quad is read as a sphere: where the stored scene depth cuts
// through it (floor, a body) its coverage fades out softly instead of ending on
// a hard line. Blend is premultiplied, so a kind picks how much of its light adds.
// Engine-owned WGSL, fixed math only, no wall clock. Method reimplements
// the Unity Particle Pack two-frame blend plus velocity stretch shape;
// zero donor bytes. PORT(kenney/particle-pack@zip-v1.0-2018) descriptors
// live in content/vfx, never in crates/. The flipbook atlas stays bound (one
// white texel until the sprite bytes land) and multiplies the shape, so a real
// atlas drops in without a shader edit. Depth bind: `present/vfx_soft.rs`.
struct DrawU {
    view_proj: mat4x4<f32>,
    cam_right: vec3<f32>,
    fade_range: f32,
    cam_up: vec3<f32>,
    stretch_gain: f32,
    atlas_cols: u32,
    atlas_rows: u32,
    stretch_max: f32,
    soft_on: u32,
    // `present/vfx_soft.rs` `VfxFrame`: view depth = zparams.y / (z + zparams.x).
    zparams: vec2<f32>,
    depth_on: u32,
    _pad: f32,
    // Linear light (sun catch over pi plus ambient) a lit sprite multiplies by.
    light: vec4<f32>,
};
struct DrawP {
    pos: vec3<f32>,
    size: f32,
    vel: vec3<f32>,
    alpha: f32,
    frame_a: u32,
    frame_b: u32,
    blend: f32,
    // Emitter kind: 0 dust, 1 flame, 2 sparks, 3 blood (`EmitterKind::index`).
    kind: u32,
    tint: vec4<f32>,
};
@group(0) @binding(0) var<uniform> draw: DrawU;
@group(0) @binding(1) var<storage, read> parts: array<DrawP>;
@group(0) @binding(2) var draw_tex: texture_2d<f32>;
@group(0) @binding(3) var draw_smp: sampler;
@group(1) @binding(0) var scene_depth: texture_depth_2d;

const KIND_DUST: u32 = 0u;
const KIND_FLAME: u32 = 1u;
const KIND_SPARKS: u32 = 2u;
const KIND_BLOOD: u32 = 3u;

fn draw_soft_fade(depth_gap: f32, fade_range: f32) -> f32 {
    if (fade_range <= 0.0) { return 1.0; }
    let t = clamp(depth_gap / fade_range, 0.0, 1.0);
    return 1.0 - t;
}
fn draw_flipbook_uv(cell: u32, corner_uv: vec2<f32>, cols: u32, rows: u32) -> vec2<f32> {
    let c = f32(cell % max(cols, 1u));
    let r = f32((cell / max(cols, 1u)) % max(rows, 1u));
    let span = vec2<f32>(1.0 / f32(max(cols, 1u)), 1.0 / f32(max(rows, 1u)));
    return vec2<f32>(c * span.x, r * span.y) + corner_uv * span;
}
struct DrawOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv_a: vec2<f32>,
    @location(1) uv_b: vec2<f32>,
    @location(2) blend: f32,
    @location(3) fade: f32,
    @location(4) tint_alpha: vec4<f32>,
    // Quad-local coordinate in -1..1 (the long axis is the stretch axis).
    @location(5) local: vec2<f32>,
    // Half the quad's size in metres, and the instance's kind and seed.
    @location(6) half_m: f32,
    @location(7) @interpolate(flat) kind: u32,
    @location(8) @interpolate(flat) seed: f32,
};
@vertex
fn draw_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> DrawOut {
    let p = parts[ii];
    var corner = vec2<f32>(0.0, 0.0);
    if (vi == 1u) { corner = vec2<f32>(1.0, 0.0); }
    if (vi == 2u) { corner = vec2<f32>(0.0, 1.0); }
    if (vi == 3u) { corner = vec2<f32>(1.0, 1.0); }
    let centered = corner - vec2<f32>(0.5, 0.5);
    let v_right = dot(p.vel, draw.cam_right);
    let v_up = dot(p.vel, draw.cam_up);
    let v2 = vec2<f32>(v_right, v_up);
    let speed = length(v2);
    // Only sparks and blood streak with their motion; a dust puff or a flame
    // lick stays round however fast the pool moved it.
    var stretch = 1.0;
    if (p.kind == KIND_SPARKS || p.kind == KIND_BLOOD) {
        stretch = min(1.0 + speed * draw.stretch_gain, draw.stretch_max);
    }
    var axis = vec2<f32>(1.0, 0.0);
    if (speed > 1e-4) { axis = v2 / speed; }
    let side = vec2<f32>(-axis.y, axis.x);
    // A drop of blood reads as a few pixels, not a card: the pool's size is the
    // quad, so the quad shrinks to the shape that fills it.
    var size = p.size;
    if (p.kind == KIND_BLOOD) { size = size * 0.7; }
    let offset = (axis * (centered.x * size * stretch) + side * (centered.y * size));
    let world = p.pos + draw.cam_right * offset.x + draw.cam_up * offset.y;
    var out: DrawOut;
    out.clip = draw.view_proj * vec4<f32>(world, 1.0);
    out.uv_a = draw_flipbook_uv(p.frame_a, corner, draw.atlas_cols, draw.atlas_rows);
    out.uv_b = draw_flipbook_uv(p.frame_b, corner, draw.atlas_cols, draw.atlas_rows);
    out.blend = p.blend;
    // The fade is the fragment's: it reads the frame depth and the shared range.
    out.fade = 1.0;
    out.tint_alpha = vec4<f32>(p.tint.rgb, p.tint.a * p.alpha);
    out.local = (corner - vec2<f32>(0.5, 0.5)) * 2.0;
    // The sphere the quad stands for: its radius across the thin axis.
    out.half_m = 0.5 * size;
    out.kind = p.kind;
    // A stable per-instance phase so neighbouring puffs do not repeat.
    out.seed = f32(ii) * 1.618 + p.pos.x * 7.0 + p.pos.z * 13.0;
    return out;
}

// View depth of the surface the frame stored under this pixel, in metres.
fn draw_scene_depth(frag_xy: vec2<f32>) -> f32 {
    let dims = vec2<i32>(textureDimensions(scene_depth));
    let px = clamp(vec2<i32>(frag_xy), vec2<i32>(0), dims - vec2<i32>(1));
    let z = textureLoad(scene_depth, px, 0);
    return draw.zparams.y / (z + draw.zparams.x);
}

@fragment
fn draw_fs(in: DrawOut) -> @location(0) vec4<f32> {
    let tex_a = textureSample(draw_tex, draw_smp, in.uv_a);
    let tex_b = textureSample(draw_tex, draw_smp, in.uv_b);
    let tex = mix(tex_a, tex_b, clamp(in.blend, 0.0, 1.0));
    let c = in.local;
    let r2 = dot(c, c);
    if (r2 >= 1.0) { discard; }
    let base_a = tex.a * in.tint_alpha.a * in.fade;
    var shape = 0.0;
    var rgb = tex.rgb * in.tint_alpha.rgb;
    // Share of the light that adds instead of blending (premultiplied alpha
    // word keeps `1 - add` of the coverage).
    var add = 0.0;
    if (in.kind == KIND_DUST) {
        // A soft puff with a wobbly outline, so a cloud is not a stack of discs.
        let wob = 1.0 + 0.22 * sin(c.x * 4.1 + in.seed) * sin(c.y * 3.7 + in.seed * 1.7);
        let t = clamp(1.0 - r2 * wob, 0.0, 1.0);
        shape = 0.7 * t * t;
        rgb = rgb * draw.light.rgb;
    } else if (in.kind == KIND_BLOOD) {
        // A tight drop with an anti-aliased rim, dark and a touch glossy.
        let r = sqrt(r2);
        shape = 1.0 - smoothstep(0.55, 1.0, r);
        let gloss = pow(clamp(1.0 - length(c - vec2<f32>(-0.3, 0.3)), 0.0, 1.0), 4.0);
        rgb = rgb * draw.light.rgb + vec3<f32>(0.5) * gloss * draw.light.g;
    } else if (in.kind == KIND_SPARKS) {
        // A hot core with a short falloff, over-bright so the bloom takes it.
        shape = exp(-r2 * 5.0);
        rgb = rgb * 4.0;
        add = 1.0;
    } else {
        // Flame: a soft lick, mostly additive.
        let t = clamp(1.0 - r2, 0.0, 1.0);
        shape = t * t;
        rgb = rgb * 1.6;
        add = 0.7;
    }
    var a = base_a * shape;
    // Soft contact: the quad is a sphere of radius half_m about its centre; its
    // front surface is nearer than the centre plane by half_m * sqrt(1 - r2).
    // Fade out as the stored surface comes up through that front.
    if (draw.depth_on != 0u && in.half_m > 1e-5) {
        let front = 1.0 / in.clip.w - in.half_m * sqrt(1.0 - r2);
        let gap = draw_scene_depth(in.clip.xy) - front;
        a = a * draw_soft_fade(gap, draw.fade_range);
    }
    return vec4<f32>(rgb * a, a * (1.0 - add));
}
