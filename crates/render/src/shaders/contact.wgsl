// contact.wgsl: screen-space contact shadows, ambient only.
//
// One thread per full-res pixel. Reads the frame's depth field and the GTAO
// occlusion field, marches 16 even steps over 0.3 m from the pixel's world
// point toward the sun, and multiplies the occlusion by a shade when a
// surface sits within 0.1 m in front of the ray. The sun's own shadow map is
// untouched: this only recovers the last centimetres where a body meets the
// ground.
//
// Mirror: `crate::contact_shadow` (the CPU oracle) pins the same constants and
// the same front-of-ray test; the parity test drives both over one depth
// field.
//
// Donor shape: bevyengine/bevy@main crates/bevy_pbr/src/contact_shadows.rs
// (linear steps 16, thickness 0.1, length 0.3; zero bytes vendored).

struct ContactU {
    inv_vp: mat4x4<f32>,
    vp: mat4x4<f32>,
    eye: vec3<f32>,
    strength: f32,
    sun: vec3<f32>,
    _pad0: f32,
    full_wh: vec2<u32>,
    _pad1: vec2<u32>,
};

@group(0) @binding(0) var<uniform> u: ContactU;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
// The GTAO field, read and written in place: one thread reads only its own
// pixel, so no race and no extra full-res texture.
@group(0) @binding(2) var ao_tex: texture_storage_2d<r32float, read_write>;

const STEPS: u32 = 16u;
const LENGTH_M: f32 = 0.3;
const THICKNESS_M: f32 = 0.1;
const BIAS_M: f32 = 0.02;
const MAX_SHADE: f32 = 0.55;

fn clamp_px(p: vec2<i32>) -> vec2<i32> {
    return clamp(p, vec2<i32>(0), vec2<i32>(u.full_wh) - vec2<i32>(1));
}

fn world_at(c: vec2<i32>) -> vec3<f32> {
    let p = clamp_px(c);
    let d = textureLoad(depth_tex, p, 0);
    let nx = 2.0 * (f32(p.x) + 0.5) / f32(u.full_wh.x) - 1.0;
    let ny = 1.0 - 2.0 * (f32(p.y) + 0.5) / f32(u.full_wh.y);
    let q = u.inv_vp * vec4<f32>(nx, ny, d, 1.0);
    return q.xyz / q.w;
}

// World point to the pixel it lands on; false when it leaves the picture.
fn project(p: vec3<f32>, out_px: ptr<function, vec2<i32>>) -> bool {
    let clip = u.vp * vec4<f32>(p, 1.0);
    if (clip.w <= 1e-6) {
        return false;
    }
    let ndc = clip.xy / clip.w;
    if (ndc.x < -1.0 || ndc.x > 1.0 || ndc.y < -1.0 || ndc.y > 1.0) {
        return false;
    }
    let x = (ndc.x * 0.5 + 0.5) * f32(u.full_wh.x);
    let y = (0.5 - ndc.y * 0.5) * f32(u.full_wh.y);
    *out_px = clamp_px(vec2<i32>(i32(x), i32(y)));
    return true;
}

@compute @workgroup_size(8, 8)
fn contact_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= u.full_wh.x || gid.y >= u.full_wh.y) {
        return;
    }
    let c = vec2<i32>(i32(gid.x), i32(gid.y));
    let ao = textureLoad(ao_tex, c).r;
    let depth = textureLoad(depth_tex, c, 0);
    if (depth >= 1.0 || u.strength <= 0.0) {
        textureStore(ao_tex, c, vec4<f32>(ao, 0.0, 0.0, 0.0));
        return;
    }
    let world = world_at(c);
    let step = LENGTH_M / f32(STEPS);
    var shade = 0.0;
    var i = 1u;
    loop {
        if (i > STEPS) {
            break;
        }
        let ray = world + u.sun * (step * f32(i));
        var px: vec2<i32>;
        if (!project(ray, &px)) {
            break;
        }
        let surface = world_at(px);
        let to_eye = u.eye - ray;
        let eye_len = length(to_eye);
        if (eye_len > 1e-6) {
            let front = dot(surface - ray, to_eye / eye_len);
            if (front > BIAS_M && front < THICKNESS_M) {
                // Nearer occluders read darker; the first hit ends the march.
                shade = 1.0 - f32(i - 1u) / f32(STEPS);
                break;
            }
        }
        i = i + 1u;
    }
    let factor = clamp(1.0 - shade * (1.0 - MAX_SHADE) * u.strength, 0.0, 1.0);
    textureStore(ao_tex, c, vec4<f32>(ao * factor, 0.0, 0.0, 0.0));
}
