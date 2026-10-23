// trail.wgsl: sword ribbon from the game's blade-pair ring (LF14).
// One staged draw: read the base/tip strip the CPU ring built, expand the pair
// across the camera basis at the width scale, and carry the age alpha ramp.
// Engine-owned WGSL, fixed math only, no wall clock. The bind that fills
// `TrailU` and the strip storage lives in `present/trail_draw_gpu.rs`.
struct TrailU {
    view_proj: mat4x4<f32>,
    cam_right: vec3<f32>,
    width_scale: f32,
    cam_up: vec3<f32>,
    // Verts staged this frame: the storage buffer is fixed at the ring cap,
    // so the sweep look-ahead must stop at the live count, not `arrayLength`.
    vert_count: u32,
};
struct StripVert {
    pos: vec3<f32>,
    alpha: f32,
};
@group(0) @binding(0) var<uniform> trail: TrailU;
@group(0) @binding(1) var<storage, read> strip: array<StripVert>;
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
@fragment
fn trail_fs(in: TrailOut) -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 1.0, 1.0, in.alpha);
}
