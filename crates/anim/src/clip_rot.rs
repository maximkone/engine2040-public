//! clip_rot: a quantized rotation lane that sits beside [`QTrack`] on the same
//! clip tick grid. Positions stay in `clip.rs`; this module carries the
//! quaternion half a version-1 clip never had, so an old clip and its wire
//! format are untouched.
//!
//! Quantization: one unit quaternion is four `i16`, each component mapped from
//! `[-1, 1]` to `[-32767, 32767]` by `round(v * 32767)`, and the decoded four
//! are renormalized, because rounding a unit quaternion moves its length. The
//! slerp also renormalizes its result. `q` and `-q` are the same rotation, so
//! baking flips the sign to `w >= 0` (with the first nonzero of `x, y, z` as
//! the tie-break when `w == 0`); two sign-flipped keys therefore bake to the
//! same bytes and the same [`rot_hash`].
//!
//! Sampling mirrors `clip::sample_track`: NaN time reads the first key, times
//! before the first key and after the last clamp to that key, and `hint` is the
//! caller's cursor over the track (moved forward on a sequential read, else a
//! binary search re-seats it). The difference is the blend: quaternions take
//! the shortest arc, positions lerp.

use engine_core::Fnv64;

use crate::clip::clamp01;
use crate::contract::rotate;
use crate::inertialize::{acos_unit, sin_unit};

/// Largest stored component magnitude; also the decode divisor. The struct
/// field is `i16`, so the bound is one below `i16::MAX` to stay symmetric.
const Q_MAX_F: f32 = 32767.0;
/// Below this the decode or bake input is too small to hold a direction.
const Q_EPS: f32 = 1e-8;

/// One quantized rotation lane: strictly increasing times and one `[i16; 4]`
/// unit quaternion per time, mapped from `[-1, 1]` to `[-32767, 32767]`.
/// Empty = invalid (sample = identity).
#[derive(Clone, Debug, Default)]
pub struct RotTrack {
    /// Seconds, same grid as the clip's [`QTrack`](crate::clip::QTrack).
    pub times: Vec<f32>,
    /// Quantized `[x, y, z, w]`, sign-canonicalized to `w >= 0` at bake.
    pub keys: Vec<[i16; 4]>,
}

/// A weapon or prop socket: the joint it rides, its local offset, and the
/// rotation lane that turns the offset. Sampling is local; the caller applies
/// the joint's parent chain.
#[derive(Clone, Debug)]
pub struct SocketTrack {
    /// Joint index in the skeleton the clip was baked for.
    pub joint: usize,
    /// Local offset in meters, unrotated.
    pub offset: [f32; 3],
    /// The socket's local rotation lane.
    pub rot: RotTrack,
}

/// Normalize, or identity when the input is degenerate or non-finite.
fn normalize(q: [f32; 4]) -> [f32; 4] {
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if !n.is_finite() || n < Q_EPS {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let inv = 1.0 / n;
    [q[0] * inv, q[1] * inv, q[2] * inv, q[3] * inv]
}

/// Flip to `w >= 0`; when `w == 0` the first nonzero of `x, y, z` picks the
/// sign, so `q` and `-q` land on one representative even at a 180-degree turn.
fn canonical(mut q: [f32; 4]) -> [f32; 4] {
    let flip = if q[3] > 0.0 {
        false
    } else if q[3] < 0.0 {
        true
    } else {
        q[0] < 0.0 || (q[0] == 0.0 && (q[1] < 0.0 || (q[1] == 0.0 && q[2] < 0.0)))
    };
    if flip {
        for c in &mut q {
            *c = -*c;
        }
    }
    q
}

/// Round to the nearest integer and narrow to `i16` without a numeric cast.
/// The input is already clamped to the quant range. Adding `1.5 * 2^23` makes
/// the field hold an exact integer, so the low 23 bits minus `2^22` are it.
fn round_i16(x: f32) -> i16 {
    const MAGIC: f32 = 12_582_912.0; // 1.5 * 2^23
    const LOW_MASK: u32 = 0x007F_FFFF;
    const BIAS: i32 = 4_194_304; // 2^22
    let low = i32::try_from((x + MAGIC).to_bits() & LOW_MASK).unwrap_or(BIAS);
    i16::try_from(low - BIAS).unwrap_or(0)
}

/// Quantize one unit quaternion after normalizing and canonicalizing it.
fn quantize_q(q: [f32; 4]) -> [i16; 4] {
    let q = canonical(normalize(q));
    let mut out = [0i16; 4];
    for (slot, c) in out.iter_mut().zip(q) {
        *slot = round_i16(c.clamp(-1.0, 1.0) * Q_MAX_F);
    }
    out
}

/// Decode one key back to a unit quaternion.
fn decode_q(k: [i16; 4]) -> [f32; 4] {
    normalize([
        f32::from(k[0]) / Q_MAX_F,
        f32::from(k[1]) / Q_MAX_F,
        f32::from(k[2]) / Q_MAX_F,
        f32::from(k[3]) / Q_MAX_F,
    ])
}

/// Normalized lerp: the near-parallel fallback, where slerp is its limit.
fn lerp_norm(a: [f32; 4], b: [f32; 4], f: f32) -> [f32; 4] {
    normalize([
        a[0] + (b[0] - a[0]) * f,
        a[1] + (b[1] - a[1]) * f,
        a[2] + (b[2] - a[2]) * f,
        a[3] + (b[3] - a[3]) * f,
    ])
}

/// Shortest-arc spherical blend; near-parallel pairs take the normalized-lerp
/// branch so the division by `sin(theta)` never underflows. The angle and its
/// sine come from the crate's integer-free polynomial pair, never the platform
/// libm, so a replay on another PC reads the same bits.
fn slerp(a: [f32; 4], mut b: [f32; 4], f: f32) -> [f32; 4] {
    let mut dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    if dot < 0.0 {
        dot = -dot;
        for c in &mut b {
            *c = -*c;
        }
    }
    let dot = dot.clamp(-1.0, 1.0);
    if dot > 0.9995 {
        return lerp_norm(a, b, f);
    }
    let theta = acos_unit(dot);
    let sin_theta = sin_unit(theta);
    if !sin_theta.is_finite() || sin_theta.abs() < 1e-6 {
        return lerp_norm(a, b, f);
    }
    let wa = sin_unit((1.0 - f) * theta) / sin_theta;
    let wb = sin_unit(f * theta) / sin_theta;
    normalize([
        a[0] * wa + b[0] * wb,
        a[1] * wa + b[1] * wb,
        a[2] * wa + b[2] * wb,
        a[3] * wa + b[3] * wb,
    ])
}

/// Interval start and fraction, the same answer `clip::sample_track` gets: a
/// cursor that still sits behind `t` walks forward, one that sits ahead is
/// re-seated by a partition point. Clamping happens before this is called, so
/// `times[0] <= t < times[last]` holds here.
fn interval(times: &[f32], hint: usize, t: f32) -> (usize, f32) {
    let last = times.len() - 1;
    let start = hint.min(last);
    let lo = if times[start] <= t {
        let ahead = times[start..]
            .iter()
            .position(|&x| x > t)
            .unwrap_or(last - start);
        start + ahead - 1
    } else {
        times[..last].partition_point(|&x| x <= t)
    };
    let (t0, t1) = (times[lo], times[lo + 1]);
    let s = t1 - t0;
    (lo, if s <= 0.0 { 0.0 } else { clamp01((t - t0) / s) })
}

/// Bake unit quaternions into a rotation lane. Mismatched lengths or an empty
/// input give an empty track, the same convention as [`bake_track`].
///
/// [`bake_track`]: crate::clip::bake_track
pub fn bake_rot_track(times: Vec<f32>, quats: Vec<[f32; 4]>) -> RotTrack {
    if times.len() != quats.len() || times.is_empty() {
        return RotTrack {
            times: Vec::new(),
            keys: Vec::new(),
        };
    }
    let mut keys = Vec::with_capacity(quats.len());
    for q in quats {
        keys.push(quantize_q(q));
    }
    RotTrack { times, keys }
}

/// Sample the lane: NaN reads the first key, both ends clamp, neighbours slerp
/// on the shortest arc and the result is renormalized. Empty or mismatched
/// track = identity.
pub fn sample_rot_track(tr: &RotTrack, t: f32, hint: &mut usize) -> [f32; 4] {
    if tr.times.len() != tr.keys.len() || tr.times.is_empty() {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let t = if t.is_nan() { tr.times[0] } else { t };
    let last = tr.times.len() - 1;
    if t <= tr.times[0] {
        *hint = 0;
        return decode_q(tr.keys[0]);
    }
    if t >= tr.times[last] {
        *hint = last;
        return decode_q(tr.keys[last]);
    }
    let (lo, f) = interval(&tr.times, *hint, t);
    *hint = lo;
    slerp(decode_q(tr.keys[lo]), decode_q(tr.keys[lo + 1]), f)
}

/// Build a socket from a joint index, its local offset and a rotation lane.
pub fn socket_track(joint: usize, offset: [f32; 3], rot: RotTrack) -> SocketTrack {
    SocketTrack { joint, offset, rot }
}

/// Sample a socket: the local offset turned by the sampled rotation, plus that
/// rotation. The joint index is carried for the caller's parent chain.
pub fn sample_socket(tr: &SocketTrack, t: f32) -> ([f32; 3], [f32; 4]) {
    let mut hint = 0usize;
    let q = sample_rot_track(&tr.rot, t, &mut hint);
    (rotate(&tr.offset, &q), q)
}

/// FNV-1a64 over the lane, same shape as [`qclip_hash`]: times as `f32` bits,
/// keys as their `i16` bits.
///
/// [`qclip_hash`]: crate::clip::qclip_hash
pub fn rot_hash(tr: &RotTrack) -> u64 {
    let mut h = Fnv64::new();
    h.write_len(tr.times.len());
    h.write_len(tr.keys.len());
    for t in &tr.times {
        h.write_f32(*t);
    }
    for k in &tr.keys {
        for x in k {
            h.write_u16(u16::from_ne_bytes(x.to_ne_bytes()));
        }
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::quat_mul;

    fn turn_180() -> RotTrack {
        bake_rot_track(
            vec![0.0, 1.0],
            vec![[0.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 0.0]],
        )
    }

    #[test]
    fn midpoint_of_180_is_90() {
        let tr = turn_180();
        let mut hint = 0usize;
        let q = sample_rot_track(&tr, 0.5, &mut hint);
        let s = 0.5f32.sqrt();
        let want = [0.0, s, 0.0, s];
        for i in 0..4 {
            assert!((q[i] - want[i]).abs() < 2e-3, "comp {i}: {q:?}");
        }
    }

    #[test]
    fn sign_flipped_bakes_share_a_hash() {
        let a = bake_rot_track(vec![0.0], vec![[0.1, 0.2, 0.3, 0.9]]);
        let b = bake_rot_track(vec![0.0], vec![[-0.1, -0.2, -0.3, -0.9]]);
        assert_eq!(rot_hash(&a), rot_hash(&b));
        // w == 0 has no sign to prefer, so the first nonzero component decides.
        let c = bake_rot_track(vec![0.0], vec![[0.0, 1.0, 0.0, 0.0]]);
        let d = bake_rot_track(vec![0.0], vec![[0.0, -1.0, 0.0, 0.0]]);
        assert_eq!(rot_hash(&c), rot_hash(&d));
    }

    #[test]
    fn clamps_at_both_ends() {
        let tr = turn_180();
        let mut hint = 1usize;
        let first = sample_rot_track(&tr, -10.0, &mut hint);
        assert_eq!(hint, 0);
        assert_eq!(first, decode_q(tr.keys[0]));
        let last = sample_rot_track(&tr, 10.0, &mut hint);
        assert_eq!(hint, 1);
        assert_eq!(last, decode_q(tr.keys[1]));
    }

    #[test]
    fn nan_time_reads_the_first_key() {
        let tr = turn_180();
        let (mut ha, mut hb) = (0usize, 1usize);
        assert_eq!(
            sample_rot_track(&tr, f32::NAN, &mut ha),
            sample_rot_track(&tr, 0.0, &mut hb)
        );
        assert_eq!(ha, 0);
    }

    #[test]
    fn empty_and_mismatched_bake_empty() {
        assert!(bake_rot_track(Vec::new(), Vec::new()).times.is_empty());
        assert!(bake_rot_track(vec![0.0], Vec::new()).keys.is_empty());
        let mut hint = 0usize;
        assert_eq!(
            sample_rot_track(&RotTrack::default(), 0.5, &mut hint),
            [0.0, 0.0, 0.0, 1.0]
        );
    }

    #[test]
    fn three_joint_chain_socket_lands_on_hand_math() {
        // Joint 2 of a 3-joint chain, offset one metre along local +X.
        let sock = socket_track(2, [1.0, 0.0, 0.0], turn_180());
        let (local, q) = sample_socket(&sock, 0.5);
        // A 90-degree Y turn maps +X to -Z.
        for (i, want) in [0.0, 0.0, -1.0].iter().enumerate() {
            assert!((local[i] - want).abs() < 2e-3, "comp {i}: {local:?}");
        }
        // The chain's parent is one more 90-degree Y turn, so the world pose is
        // a half turn about Y and the hand sits at world -X.
        let parent = [0.0, 0.5f32.sqrt(), 0.0, 0.5f32.sqrt()];
        let world = rotate(&local, &parent);
        for (i, want) in [-1.0, 0.0, 0.0].iter().enumerate() {
            assert!((world[i] - want).abs() < 2e-3, "comp {i}: {world:?}");
        }
        let composed = quat_mul(parent, q);
        assert!(
            composed[3].abs() < 2e-3,
            "half turn has w = 0: {composed:?}"
        );
        assert!((composed[1].abs() - 1.0).abs() < 2e-3);
    }

    #[test]
    fn two_bakes_hash_equal() {
        let times = vec![0.0, 0.5, 1.0];
        let quats = vec![
            [0.0, 0.0, 0.0, 1.0],
            [0.0, 0.382_683_43, 0.0, 0.923_879_5],
            [0.0, 1.0, 0.0, 0.0],
        ];
        let a = bake_rot_track(times.clone(), quats.clone());
        let b = bake_rot_track(times, quats);
        assert_eq!(rot_hash(&a), rot_hash(&b));
        let mut c = a.clone();
        c.times[1] = 0.25;
        assert_ne!(rot_hash(&a), rot_hash(&c));
    }
}
