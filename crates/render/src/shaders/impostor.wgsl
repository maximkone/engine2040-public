// impostor.wgsl: instanced camera-facing quads sampling an octahedral atlas
// of a mesh baked from an N by N grid of view directions (albedo + normal),
// blending the three nearest views.
//
// Mirror of `impostor.rs`: `octa_encode`, `octa_decode`, `impostor_basis`
// and `impostor_pick` are the same math as `octa_encode`, `octa_decode`,
// `bake_basis` and `pick_frames`, so the baker (`present/impostor_gpu.rs`,
// `impostor_bake.wgsl`) and this draw agree on every frame direction, sprite
// axis and blend weight.
//
// Window draw (RL64): one quad per instance (6 vertices, no vertex buffer),
// recorded inside the forward pass after the mesh draws. It reads the frame's
// own camera, sun, fog, baked-ambient and previous-camera buffers, so the
// crowd is lit, fogged and reprojected like every mesh beside it.
//   fs_main       HDR colour only (4x MSAA pass and test targets)
//   fs_main_gbuf  HDR + id + motion (the 1x forward pass)
// The atlas stores colour premultiplied by coverage (the 2x bake is box
// filtered down), so the draw divides by the blended coverage and the cut-out
// edge never carries the black clear colour.
// PORT(wojtekpil/Godot-Octahedral-Impostors: grid of octahedral frames,
// three-frame blend; reimplemented, zero bytes vendored).

struct CamU {
    view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    _pad0: f32,
};

struct LightU {
    // Unit direction toward the sun, then its linear radiance.
    dir_to_light: vec3<f32>,
    _pad0: f32,
    light_color: vec3<f32>,
    intensity: f32,
};

struct FogFwdU {
    inv_view_proj: mat4x4<f32>,
    eye: vec3<f32>,
    density: f32,
    color: vec3<f32>,
    sky_on: f32,
    zenith: vec3<f32>,
    _pad0: f32,
    plane: vec4<f32>,
};

struct PrevCamU {
    view_proj: mat4x4<f32>,
};

struct ImpostorItem {
    // Bounding-sphere radius in meters of the baked figure.
    radius: f32,
    // Frames per atlas side, frame edge in pixels, alpha cutoff in 0..1.
    frames_per_side: u32,
    frame_px: u32,
    cutoff: f32,
    // 1 shades with the frame's sun and baked ambient, 0 returns the albedo.
    lit: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

struct Instance {
    // Sphere centre in meters and a uniform scale of the sprite.
    center: vec3<f32>,
    scale: f32,
    // Tunic tint (linear multiplier colour) and the yaw about +y in radians.
    tint: vec3<f32>,
    yaw: f32,
};

@group(0) @binding(0) var<uniform> cam: CamU;
@group(0) @binding(1) var<uniform> light: LightU;
@group(0) @binding(2) var<uniform> fog_fwd: FogFwdU;
@group(0) @binding(3) var<uniform> ibl_sh: array<vec4<f32>, 9>;
@group(0) @binding(4) var<uniform> prev_cam: PrevCamU;
@group(0) @binding(5) var<uniform> steady_cam: PrevCamU;
@group(0) @binding(6) var<uniform> item: ImpostorItem;
@group(0) @binding(7) var albedo_atlas: texture_2d<f32>;
@group(0) @binding(8) var normal_atlas: texture_2d<f32>;
@group(0) @binding(9) var atlas_smp: sampler;
@group(0) @binding(10) var<storage, read> insts: array<Instance>;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) frame_a: u32,
    @location(2) @interpolate(flat) frame_b: u32,
    @location(3) @interpolate(flat) frame_c: u32,
    @location(4) @interpolate(flat) weights: vec3<f32>,
    @location(5) @interpolate(flat) tint: vec3<f32>,
    @location(6) @interpolate(flat) yaw: f32,
    @location(7) @interpolate(flat) fog_f: f32,
    @location(8) cur_clip: vec4<f32>,
    @location(9) prev_clip: vec4<f32>,
};

fn sgn(v: f32) -> f32 {
    if (v < 0.0) {
        return -1.0;
    }
    return 1.0;
}

// Direction (any non-zero length) to atlas UV, +y at the center.
fn octa_encode(dir: vec3<f32>) -> vec2<f32> {
    let d = normalize(dir);
    let s = abs(d.x) + abs(d.y) + abs(d.z);
    var o = vec2<f32>(d.x / s, d.z / s);
    if (d.y < 0.0) {
        let p = o;
        o = vec2<f32>((1.0 - abs(p.y)) * sgn(p.x), (1.0 - abs(p.x)) * sgn(p.y));
    }
    return clamp(o * 0.5 + vec2<f32>(0.5), vec2<f32>(0.0), vec2<f32>(1.0));
}

// Atlas UV back to a unit direction.
fn octa_decode(uv: vec2<f32>) -> vec3<f32> {
    var o = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)) * 2.0 - vec2<f32>(1.0);
    let y = 1.0 - abs(o.x) - abs(o.y);
    if (y < 0.0) {
        let p = o;
        o = vec2<f32>((1.0 - abs(p.y)) * sgn(p.x), (1.0 - abs(p.x)) * sgn(p.y));
    }
    return normalize(vec3<f32>(o.x, y, o.y));
}

// Sprite right and up for a camera along `to_camera`; the baker uses the
// same axes, so the sprite is never rotated against its source frame.
fn impostor_basis(to_camera: vec3<f32>) -> array<vec3<f32>, 2> {
    let z = normalize(to_camera);
    var up_ref = vec3<f32>(0.0, 1.0, 0.0);
    if (abs(z.x) < 0.0001 && abs(z.z) < 0.0001) {
        up_ref = vec3<f32>(0.0, 0.0, -1.0);
    }
    let x = normalize(cross(up_ref, z));
    let y = cross(z, x);
    return array<vec3<f32>, 2>(x, y);
}

struct Pick {
    frames: vec3<u32>,
    weights: vec3<f32>,
};

// The three nearest frames and their barycentric weights (sum 1).
fn impostor_pick(dir: vec3<f32>, n: u32) -> Pick {
    let uv = octa_encode(dir);
    let span = f32(n - 1u);
    let g = uv * span;
    let last = n - 2u;
    let ix = min(u32(floor(g.x)), last);
    let iy = min(u32(floor(g.y)), last);
    let fx = clamp(g.x - f32(ix), 0.0, 1.0);
    let fy = clamp(g.y - f32(iy), 0.0, 1.0);
    var p: Pick;
    if (fx + fy <= 1.0) {
        p.frames = vec3<u32>(iy * n + ix, iy * n + ix + 1u, (iy + 1u) * n + ix);
        p.weights = vec3<f32>(1.0 - fx - fy, fx, fy);
    } else {
        let w0 = fx + fy - 1.0;
        let w1 = 1.0 - fx;
        p.frames = vec3<u32>((iy + 1u) * n + ix + 1u, (iy + 1u) * n + ix, iy * n + ix + 1u);
        p.weights = vec3<f32>(w0, w1, 1.0 - w0 - w1);
    }
    p.weights = max(p.weights, vec3<f32>(0.0));
    p.weights = p.weights / (p.weights.x + p.weights.y + p.weights.z);
    return p;
}

// Rotation about +y by `a` radians (x toward -z for a positive angle).
fn rot_y(v: vec3<f32>, a: f32) -> vec3<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec3<f32>(v.x * c + v.z * s, v.y, -v.x * s + v.z * c);
}

// Texel position inside one frame, inset half a texel so bilinear taps
// never read the neighbouring frame.
fn frame_sample_uv(frame: u32, uv: vec2<f32>) -> vec2<f32> {
    let n = item.frames_per_side;
    let inset = 0.5 / f32(item.frame_px);
    let local = clamp(uv, vec2<f32>(inset), vec2<f32>(1.0 - inset));
    let cell = vec2<f32>(f32(frame % n), f32(frame / n));
    return (cell + local) / f32(n);
}

// Height fog, the same closed form as `forward.wgsl` `fog_factor_finite`.
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

// Baked-ambient irradiance from the frame's SH, as `forward.wgsl`.
fn ibl_irradiance(n: vec3<f32>) -> vec3<f32> {
    var e = ibl_sh[0].xyz * 0.282095;
    e += ibl_sh[1].xyz * (-0.488603 * n.y);
    e += ibl_sh[2].xyz * (0.488603 * n.z);
    e += ibl_sh[3].xyz * (-0.488603 * n.x);
    e += ibl_sh[4].xyz * (1.092548 * n.x * n.y);
    e += ibl_sh[5].xyz * (-1.092548 * n.y * n.z);
    e += ibl_sh[6].xyz * (0.315392 * (3.0 * n.z * n.z - 1.0));
    e += ibl_sh[7].xyz * (-1.092548 * n.x * n.z);
    e += ibl_sh[8].xyz * (0.546274 * (n.x * n.x - n.y * n.y));
    let gain = 1.0 - clamp(ibl_sh[0].w, 0.0, 1.0);
    return max(e, vec3<f32>(0.0)) * gain;
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let inst = insts[ii];
    let c = corners[vi % 6u];
    let to_cam = normalize(cam.camera_pos - inst.center);
    let basis = impostor_basis(to_cam);
    let world = inst.center + (basis[0] * c.x + basis[1] * c.y) * (item.radius * inst.scale);
    // The frames were baked with the figure at yaw 0: look it up from the
    // camera direction turned back by the instance's yaw.
    let pick = impostor_pick(rot_y(to_cam, -inst.yaw), item.frames_per_side);
    var out: VsOut;
    out.clip = cam.view_proj * vec4<f32>(world, 1.0);
    // Frame UV: x right, y down, so flip the quad's up axis.
    out.uv = vec2<f32>(c.x * 0.5 + 0.5, 0.5 - c.y * 0.5);
    out.frame_a = pick.frames.x;
    out.frame_b = pick.frames.y;
    out.frame_c = pick.frames.z;
    out.weights = pick.weights;
    out.tint = inst.tint;
    out.yaw = inst.yaw;
    out.fog_f = fog_factor_finite(fog_fwd, cam.camera_pos, inst.center);
    // Motion comes from the un-jittered matrices, like the mesh draws.
    out.cur_clip = steady_cam.view_proj * vec4<f32>(world, 1.0);
    out.prev_clip = prev_cam.view_proj * vec4<f32>(world, 1.0);
    return out;
}

struct Shaded {
    rgb: vec3<f32>,
    cover: f32,
};

fn shade(in: VsOut) -> Shaded {
    let ua = frame_sample_uv(in.frame_a, in.uv);
    let ub = frame_sample_uv(in.frame_b, in.uv);
    let uc = frame_sample_uv(in.frame_c, in.uv);
    let a0 = textureSampleLevel(albedo_atlas, atlas_smp, ua, 0.0);
    let a1 = textureSampleLevel(albedo_atlas, atlas_smp, ub, 0.0);
    let a2 = textureSampleLevel(albedo_atlas, atlas_smp, uc, 0.0);
    let w = in.weights;
    let cover = a0.a * w.x + a1.a * w.y + a2.a * w.z;
    var out: Shaded;
    out.cover = cover;
    if (cover < item.cutoff) {
        out.rgb = vec3<f32>(0.0);
        return out;
    }
    // Colour, normal and tunic mask are stored premultiplied by coverage.
    let inv = 1.0 / max(cover, 0.0001);
    var albedo = (a0.rgb * w.x + a1.rgb * w.y + a2.rgb * w.z) * inv;
    let n0 = textureSampleLevel(normal_atlas, atlas_smp, ua, 0.0);
    let n1 = textureSampleLevel(normal_atlas, atlas_smp, ub, 0.0);
    let n2 = textureSampleLevel(normal_atlas, atlas_smp, uc, 0.0);
    let packed = (n0 * w.x + n1 * w.y + n2 * w.z) * inv;
    let mask = clamp(packed.w, 0.0, 1.0);
    // Tunic pixels take the instance's colour but keep the baked shading.
    let lum = dot(albedo, vec3<f32>(0.3, 0.59, 0.11));
    albedo = mix(albedo, in.tint * (0.5 + lum), mask);
    if (item.lit < 0.5) {
        out.rgb = albedo;
        return out;
    }
    // Baked world normals sit in the figure's yaw-0 frame: turn them with it.
    let n_local = normalize(packed.xyz * 2.0 - vec3<f32>(1.0));
    let n = rot_y(n_local, in.yaw);
    let ndl = clamp(dot(n, normalize(light.dir_to_light)), 0.0, 1.0);
    let inv_pi = 0.3183098861837907;
    let direct = albedo * inv_pi * ndl * light.light_color;
    let ambient = albedo * ibl_irradiance(n) * inv_pi;
    let radiance = max(direct + ambient, vec3<f32>(0.0));
    out.rgb = mix(radiance, fog_fwd.color, in.fog_f);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let s = shade(in);
    if (s.cover < item.cutoff) {
        discard;
    }
    return vec4<f32>(s.rgb, 1.0);
}

struct GbufOut {
    @location(0) color: vec4<f32>,
    @location(1) id: u32,
    @location(2) motion: vec2<f32>,
};

@fragment
fn fs_main_gbuf(in: VsOut) -> GbufOut {
    let s = shade(in);
    if (s.cover < item.cutoff) {
        discard;
    }
    var out: GbufOut;
    out.color = vec4<f32>(s.rgb, 1.0);
    // No mesh draw claims an impostor pixel: hold the id target's clear value.
    out.id = 0xFFFFFFFFu;
    let cur_ndc = in.cur_clip.xy / max(in.cur_clip.w, 1e-6);
    var prev_ndc = cur_ndc;
    if (in.prev_clip.w > 1e-6) {
        prev_ndc = in.prev_clip.xy / in.prev_clip.w;
    }
    out.motion = (cur_ndc - prev_ndc) * vec2<f32>(0.5, -0.5);
    return out;
}
