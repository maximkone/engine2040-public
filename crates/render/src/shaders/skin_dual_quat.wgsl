// AM30: dual-quaternion skinning, the 32-byte alternative to the mat4 joint palette.
//
// One joint is a unit dual quaternion: `real` is the rotation, `dual` is `0.5 * t * real`, so the
// pair carries the same rigid transform as a 4x4 matrix in half the bytes (8 floats against 16).
// A vertex blended the dual way keeps its radius through a twist where a linear blend of two
// matrices collapses it toward the joint axis (the candy wrapper), which is why the shoulder and
// wrist chains ask for it.
//
// Layout matches `engine_anim::contract`: `[real.x, real.y, real.z, real.w, dual.x, ...]`, one
// entry per joint in skeleton order, and `SkinPalette::DualQuats` is off unless the frame asks
// for it. Units: meters, unit quaternions, `[x, y, z, w]`.
//
// This file is a library of functions and constants, not a pipeline: the forward entries call
// `dq_skin_*` when `skin_params.dq != 0u`. It compiles alone, so the shader gate covers it.

/// One joint as a unit dual quaternion: rotation in `real`, `0.5 * t * real` in `dual`.
struct DualQuat {
    real: vec4<f32>,
    dual: vec4<f32>,
}

/// Bytes one joint takes in the palette buffer.
const DQ_JOINT_BYTES: u32 = 32u;

/// Joints one palette holds, the same cap as the matrix palette.
const DQ_MAX_JOINTS: u32 = 64u;

/// Conjugate of a unit quaternion: the inverse rotation.
fn dq_conj(q: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(-q.x, -q.y, -q.z, q.w);
}

/// The translation a dual quaternion carries: `2 * dual * conj(real)`.
fn dq_translation(q: DualQuat) -> vec3<f32> {
    let t = q.dual * dq_conj(q.real);
    return 2.0 * t.xyz;
}

/// Rotate a direction by the dual quaternion's real part only.
fn dq_rotate(q: DualQuat, v: vec3<f32>) -> vec3<f32> {
    let rotated = q.real * vec4<f32>(v, 0.0) * dq_conj(q.real);
    return rotated.xyz;
}

/// A vertex or a normal through one joint: rotate, then add the joint's translation for a point.
fn dq_transform_point(q: DualQuat, p: vec3<f32>) -> vec3<f32> {
    return dq_rotate(q, p) + dq_translation(q);
}

/// Blend two joints for one vertex weight pair, shortest arc first, then renormalize.
///
/// `dot(real_a, real_b) < 0` means the two quaternions face away from each other and the blend
/// would swing it the long way round; flipping both parts keeps the blend on the short arc.
fn dq_blend(a: DualQuat, wa: f32, b: DualQuat, wb: f32) -> DualQuat {
    let facing = dot(a.real, b.real) >= 0.0;
    let sign = select(-1.0, 1.0, facing);
    var out: DualQuat;
    out.real = a.real * wa + b.real * (wb * sign);
    out.dual = a.dual * wa + b.dual * (wb * sign);
    let len = length(out.real);
    if (len > 1e-6) {
        out.real = out.real / len;
        out.dual = out.dual / len;
    } else {
        out = DualQuat(vec4<f32>(0.0, 0.0, 0.0, 1.0), vec4<f32>(0.0));
    }
    // Drop the dual part's share along the real part: a unit dual quaternion is orthogonal.
    out.dual = out.dual - out.real * dot(out.real, out.dual);
    return out;
}

/// Skin one point over four joint slots and weights, the dual-quaternion way.
fn dq_skin_point(palette: array<DualQuat, 64>, joints: vec4<u32>, weights: vec4<f32>, p: vec3<f32>) -> vec3<f32> {
    var sum: DualQuat = DualQuat(vec4<f32>(0.0), vec4<f32>(0.0));
    var first = true;
    for (var k = 0u; k < 4u; k = k + 1u) {
        let idx = min(joints[k], DQ_MAX_JOINTS - 1u);
        let w = weights[k];
        if (w <= 0.0) {
            continue;
        }
        let q = palette[idx];
        if (first) {
            sum = DualQuat(q.real * w, q.dual * w);
            first = false;
        } else {
            sum = dq_blend(sum, 1.0, q, w);
        }
    }
    return dq_transform_point(sum, p);
}

/// Skin one direction (normal or tangent) the same way, translation excluded.
fn dq_skin_vector(palette: array<DualQuat, 64>, joints: vec4<u32>, weights: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    var sum: DualQuat = DualQuat(vec4<f32>(0.0), vec4<f32>(0.0));
    var first = true;
    for (var k = 0u; k < 4u; k = k + 1u) {
        let idx = min(joints[k], DQ_MAX_JOINTS - 1u);
        let w = weights[k];
        if (w <= 0.0) {
            continue;
        }
        let q = palette[idx];
        if (first) {
            sum = DualQuat(q.real * w, q.dual * w);
            first = false;
        } else {
            sum = dq_blend(sum, 1.0, q, w);
        }
    }
    return dq_rotate(sum, v);
}
