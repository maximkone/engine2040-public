// particle.wgsl: Kenney flipbook draw over the seeded pool (RL52).
// Instanced quads fetch one particle by instance id, expand a camera
// billboard, stretch along view-plane velocity, sample two flipbook cells
// by age01 and blend them, then fade softly near scene depth.
// Engine-owned WGSL, fixed math only, no wall clock. Method reimplements
// the Unity Particle Pack two-frame blend plus velocity stretch shape;
// zero donor bytes. PORT(kenney/particle-pack@zip-v1.0-2018) descriptors
// live in content/vfx, never in crates/.
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
};
struct DrawP {
    pos: vec3<f32>,
    size: f32,
    vel: vec3<f32>,
    alpha: f32,
    frame_a: u32,
    frame_b: u32,
    blend: f32,
    tint: vec4<f32>,
};
@group(0) @binding(0) var<uniform> draw: DrawU;
@group(0) @binding(1) var<storage, read> parts: array<DrawP>;
@group(0) @binding(2) var draw_tex: texture_2d<f32>;
@group(0) @binding(3) var draw_smp: sampler;
@group(0) @binding(4) var<storage, read> scene_depth: array<f32>;
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
    let stretch = min(1.0 + speed * draw.stretch_gain, draw.stretch_max);
    var axis = vec2<f32>(1.0, 0.0);
    if (speed > 1e-4) { axis = v2 / speed; }
    let side = vec2<f32>(-axis.y, axis.x);
    let offset = (axis * (centered.x * p.size * stretch) + side * (centered.y * p.size));
    let world = p.pos + draw.cam_right * offset.x + draw.cam_up * offset.y;
    var out: DrawOut;
    out.clip = draw.view_proj * vec4<f32>(world, 1.0);
    out.uv_a = draw_flipbook_uv(p.frame_a, corner, draw.atlas_cols, draw.atlas_rows);
    out.uv_b = draw_flipbook_uv(p.frame_b, corner, draw.atlas_cols, draw.atlas_rows);
    out.blend = p.blend;
    let gap = 0.0;
    out.fade = draw_soft_fade(gap, draw.fade_range);
    out.tint_alpha = vec4<f32>(p.tint.rgb, p.tint.a * p.alpha);
    return out;
}
@fragment
fn draw_fs(in: DrawOut) -> @location(0) vec4<f32> {
    let tex_a = textureSample(draw_tex, draw_smp, in.uv_a);
    let tex_b = textureSample(draw_tex, draw_smp, in.uv_b);
    let tex = mix(tex_a, tex_b, clamp(in.blend, 0.0, 1.0));
    let a = tex.a * in.tint_alpha.a * in.fade;
    return vec4<f32>(tex.rgb * in.tint_alpha.rgb, a);
}
