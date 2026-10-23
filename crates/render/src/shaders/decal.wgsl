// decal.wgsl: the fight's blood on the floor (FEEL5).
// One ground quad per resident splat: expand the center across the XZ plane,
// lift it a hair off the sand, and shape a ragged soft-rimmed blob whose alpha
// carries the tick fade. The pass tests the frame's depth read-only: a body in
// front hides the blood and the sand keeps its own pixels. Engine-owned WGSL,
// fixed math only, no wall clock. The bind that fills `DecalU` and the splat
// storage lives in `present/decal_draw_gpu.rs`.
struct DecalU {
    view_proj: mat4x4<f32>,
};
struct Splat {
    center: vec3<f32>,
    radius: f32,
    tint: vec4<f32>,
};
@group(0) @binding(0) var<uniform> decal: DecalU;
@group(0) @binding(1) var<storage, read> splats: array<Splat>;

// Lift of the splat above the ground it landed on, meters: enough to win the
// depth test against the sand, small enough to stay flat beside a foot.
const SPLAT_LIFT_M: f32 = 0.012;

struct DecalOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
    @location(2) seed: f32,
};

fn decal_hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

// Value noise: two smooth steps over a hashed lattice, so the rim varies
// without a texture or a second buffer.
fn decal_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = decal_hash(i);
    let b = decal_hash(i + vec2<f32>(1.0, 0.0));
    let c = decal_hash(i + vec2<f32>(0.0, 1.0));
    let d = decal_hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

@vertex
fn decal_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> DecalOut {
    let s = splats[ii];
    // Triangle-strip corners: (-1,-1) (1,-1) (-1,1) (1,1).
    var corner = vec2<f32>(-1.0, -1.0);
    if ((vi & 1u) == 1u) { corner.x = 1.0; }
    if ((vi & 2u) == 2u) { corner.y = 1.0; }
    let world = vec3<f32>(
        s.center.x + corner.x * s.radius,
        s.center.y + SPLAT_LIFT_M,
        s.center.z + corner.y * s.radius,
    );
    var out: DecalOut;
    out.clip = decal.view_proj * vec4<f32>(world, 1.0);
    out.uv = corner;
    out.tint = s.tint;
    // Two splats at the same spot share a rim; a nearby hit gets its own.
    out.seed = fract(s.center.x * 0.731 + s.center.z * 0.317) * 32.0;
    return out;
}

@fragment
fn decal_fs(in: DecalOut) -> @location(0) vec4<f32> {
    let r = length(in.uv);
    let n = decal_noise(in.uv * 2.6 + vec2<f32>(in.seed, in.seed * 0.37));
    // The rim reaches further where the noise is high: a splash, not a disc.
    let edge = 0.78 + 0.22 * n;
    if (r >= edge) { discard; }
    let falloff = clamp((edge - r) / 0.3, 0.0, 1.0);
    let alpha = in.tint.a * falloff * (0.6 + 0.4 * n);
    if (alpha <= 0.002) { discard; }
    return vec4<f32>(in.tint.rgb * (0.7 + 0.6 * n), alpha);
}
