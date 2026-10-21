//! pose: skeleton pose sampling, blending and root-motion extraction.
//!
//! Consumer: arena locomotion consumes [`RootTable`] as authoritative
//! per-tick root intent; render consumes sampled [`LocalPose`] through
//! [`skinning_matrices`](crate::contract::skinning_matrices). Data owner:
//! `engine-anim`; assets import into these types, anim never imports
//! assets or arena. Units: meters, seconds, radians; ticks at 60 Hz.
//!
//! Track mapping: [`Clip`] (see `lib.rs`) carries one position track per
//! joint (`[f32; 3]` local offsets in meters). That clip type has no rotation
//! lanes, so [`sample_pose`] emits identity rotations. Where authoring does
//! carry quaternions, pass them as [`QuatTrack`] slices to
//! [`sample_pose_with_rots`]: a joint keeps its sampled quat when a valid
//! track is present for its index, and falls back to identity otherwise.
//!
//! Sampling shape: a key track is read with a C1 curve, not with straight
//! lines. The key values are hit exactly, the rate at a key is the same on
//! both sides of it, and the sampled value never leaves the key pair's
//! span. A clip keyed at 24 Hz and read at 60 Hz otherwise changes its rate
//! at every key, and the arena's pop law reads that corner as a clip snap
//! (AM24).
//!
//! Determinism: sampling, blending and extraction use only `+ - * /`,
//! `%` (via [`f32::rem_euclid`]) and [`f32::sqrt`]; no transcendentals, no
//! wall clock, stable joint order. Same inputs twice give identical outputs.
//! Errors reuse the contract [`AnimError`] set; invalid external input
//! returns an error, never panics.

use super::clip::clamp01;
use super::contract::{
    validate_root, validate_skeleton, AnimError, LocalPose, RootDelta, RootTable, Skeleton,
};
use super::Clip;
use std::f32::consts::{PI, TAU};

/// One joint rotation track: strictly increasing times plus unit quaternions
/// `[x, y, z, w]`. Empty or structurally invalid tracks are not errors here;
/// the joint falls back to identity (safe bind pose).
#[derive(Clone, Debug, Default)]
pub struct QuatTrack {
    /// Sample times in seconds, strictly increasing.
    pub times: Vec<f32>,
    /// Unit quaternions, one per time.
    pub values: Vec<[f32; 4]>,
}

/// Clip duration: the latest track end time, or `0.0` when there is nothing
/// to loop over. Callers validate tracks first, so ends always exist here.
fn clip_duration(clip: &Clip) -> f32 {
    let mut dur = 0.0f32;
    for track in &clip.tracks {
        if let Some(end) = track.times.last() {
            if *end > dur {
                dur = *end;
            }
        }
    }
    dur
}

/// Loop phase wrap: times past the clip end wrap into `[0, duration)`.
/// Non-finite times sample the bind edge (`0.0`); non-positive or
/// non-finite durations disable wrapping and leave clamping to the sampler.
fn wrap_time(time: f32, duration: f32) -> f32 {
    if !time.is_finite() {
        return 0.0;
    }
    if !duration.is_finite() || duration <= 0.0 {
        return time;
    }
    let mut wrapped = time % duration;
    if wrapped < 0.0 {
        wrapped += duration;
    }
    if wrapped < 0.0 {
        0.0
    } else {
        wrapped
    }
}

/// Validate one position track: non-empty, times/values paired, finite data,
/// strictly increasing times. `NonFinite` names the bad lane; a time-order
/// violation reuses `BadRootOrder` carrying the joint index (the shared
/// time-sequence ordering error in the contract set).
fn check_track(joint: usize, times: &[f32], value_len: usize) -> Result<(), AnimError> {
    if times.is_empty() {
        return Err(AnimError::LengthMismatch {
            expected: times.len().max(1),
            got: value_len,
        });
    }
    if times.len() != value_len {
        return Err(AnimError::LengthMismatch {
            expected: times.len(),
            got: value_len,
        });
    }
    if times.iter().any(|t| !t.is_finite()) {
        return Err(AnimError::NonFinite("clip.times"));
    }
    let mut prev = times[0];
    for t in &times[1..] {
        if *t <= prev {
            return Err(AnimError::BadRootOrder(joint));
        }
        prev = *t;
    }
    Ok(())
}

/// Shortest-arc yaw delta wrapped into `[-PI, PI)`.
fn wrap_pi(delta: f32) -> f32 {
    (delta + PI).rem_euclid(TAU) - PI
}

fn lerp3(a: [f32; 3], b: [f32; 3], w: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * w,
        a[1] + (b[1] - a[1]) * w,
        a[2] + (b[2] - a[2]) * w,
    ]
}

/// Normalized lerp between unit quats (slerp-lite): shortest path, NaN-safe.
/// Degenerate input (non-finite dot or zero-length lerp) falls back to the
/// safe bind pose (identity) or `a` for exact antipodes. This differs from
/// the graph eval `nlerp` on that antipodal case (which returns identity),
/// so the two paths are not bit-for-bit identical.
fn nlerp(a: [f32; 4], b: [f32; 4], w: f32) -> [f32; 4] {
    const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    if !dot.is_finite() {
        return IDENTITY;
    }
    let s = if dot < 0.0 { -1.0 } else { 1.0 };
    let q = [
        a[0] + (s * b[0] - a[0]) * w,
        a[1] + (s * b[1] - a[1]) * w,
        a[2] + (s * b[2] - a[2]) * w,
        a[3] + (s * b[3] - a[3]) * w,
    ];
    let norm2 = q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3];
    if !norm2.is_finite() {
        return IDENTITY;
    }
    if norm2 <= 0.0 {
        return a;
    }
    let inv = 1.0 / norm2.sqrt();
    [q[0] * inv, q[1] * inv, q[2] * inv, q[3] * inv]
}

/// Tangent rate cap: a key's tangent never runs past this many times the
/// smallest chord rate around it, so the sampled curve stays monotone inside
/// a key pair and never leaves the two keys it sits between.
const TANGENT_RATE_CAP: f32 = 3.0;

/// The rate between two key values per second. A non-positive span reads as
/// no rate.
fn rate_between<const N: usize>(a: [f32; N], b: [f32; N], span: f32) -> [f32; N] {
    if !span.is_finite() || span <= 0.0 {
        return [0.0; N];
    }
    let mut out = [0.0; N];
    for k in 0..N {
        out[k] = (b[k] - a[k]) / span;
    }
    out
}

/// The tangent from a key's central difference and the two chords around
/// it: zeroed where the central difference disagrees with the chord into or
/// out of the key (a local extremum reads as a flat spot) and capped at
/// [`TANGENT_RATE_CAP`] times the smaller of the two chord rates, so the
/// curve never leaves the key pair it sits in.
fn tangent_of<const N: usize>(
    central: [f32; N],
    chord_in: Option<[f32; N]>,
    chord_out: Option<[f32; N]>,
) -> [f32; N] {
    let mut out = [0.0; N];
    for k in 0..N {
        let c = central[k];
        let agrees_in = chord_in.is_none_or(|ch| c * ch[k] > 0.0);
        let agrees_out = chord_out.is_none_or(|ch| c * ch[k] > 0.0);
        if !agrees_in || !agrees_out {
            continue;
        }
        let mut limit = f32::INFINITY;
        if let Some(ch) = chord_in {
            limit = limit.min(TANGENT_RATE_CAP * ch[k].abs());
        }
        if let Some(ch) = chord_out {
            limit = limit.min(TANGENT_RATE_CAP * ch[k].abs());
        }
        out[k] = c.signum() * c.abs().min(limit);
    }
    out
}

/// The tangent at one key of a scalar track, in units per second.
///
/// The tangent is the central difference of the neighbouring keys, so the
/// rate on both sides of the key is the same (the curve is C1 there). It is
/// zeroed where it disagrees with the chord into or out of the key (a local
/// extremum reads as a flat spot) and capped at [`TANGENT_RATE_CAP`] times
/// the smaller of the two chord rates, so the curve never leaves the key
/// pair it sits in.
fn tangent_at<const N: usize>(times: &[f32], values: &[[f32; N]], i: usize) -> [f32; N] {
    let n = values.len();
    let prev = i.saturating_sub(1);
    let next = (i + 1).min(n - 1);
    let chord_in = if i > 0 {
        Some(rate_between(values[prev], values[i], times[i] - times[prev]))
    } else {
        None
    };
    let chord_out = if i + 1 < n {
        Some(rate_between(values[i], values[next], times[next] - times[i]))
    } else {
        None
    };
    let central = rate_between(values[prev], values[next], times[next] - times[prev]);
    tangent_of(central, chord_in, chord_out)
}

/// The cubic Hermite blend of two key values with their tangents, `u` in
/// 0..=1 across a segment `h` seconds wide.
fn hermite<const N: usize>(
    start: [f32; N],
    end: [f32; N],
    m0: [f32; N],
    m1: [f32; N],
    h: f32,
    u: f32,
) -> [f32; N] {
    let u2 = u * u;
    let u3 = u2 * u;
    let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
    let h10 = u3 - 2.0 * u2 + u;
    let h01 = -2.0 * u3 + 3.0 * u2;
    let h11 = u3 - u2;
    let mut out = [0.0; N];
    for k in 0..N {
        out[k] = h00 * start[k] + h10 * h * m0[k] + h01 * end[k] + h11 * h * m1[k];
    }
    out
}

/// The key pair a time falls in: `lo` and `lo + 1`, with the time inside the
/// pair as `u` in 0..=1. `None` when the track cannot hold a pair.
fn key_pair(times: &[f32], t: f32) -> Option<(usize, f32)> {
    let n = times.len();
    if n < 2 || !t.is_finite() {
        return None;
    }
    if t <= times[0] {
        return Some((0, 0.0));
    }
    if t >= times[n - 1] {
        return Some((n - 2, 1.0));
    }
    let mut lo = 0usize;
    let mut hi = n - 1;
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if times[mid] <= t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let span = times[hi] - times[lo];
    let u = if span > 0.0 {
        ((t - times[lo]) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    Some((lo, u))
}

/// The dot of two quaternions.
fn quat_dot(a: [f32; 4], b: [f32; 4]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}

/// A unit quaternion from any four finite components; a degenerate one reads
/// as the identity.
fn unit_quat(q: [f32; 4]) -> [f32; 4] {
    let n2 = q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3];
    if !n2.is_finite() || n2 <= 0.0 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let inv = 1.0 / n2.sqrt();
    [q[0] * inv, q[1] * inv, q[2] * inv, q[3] * inv]
}

/// Sample one position track with the C1 curve: the key values are hit
/// exactly, the rate at a key is the same on both sides of it, and the
/// sampled value never leaves the key pair's span. A keyframed clip read
/// with straight lines changes its rate at every key, and the arena's pop
/// law reads that corner as a clip snap (AM24). A track with fewer than
/// three keys reads as the straight line.
fn sample_track_c1(times: &[f32], values: &[[f32; 3]], t: f32) -> [f32; 3] {
    let n = values.len();
    if times.len() != n || n == 0 {
        return [0.0; 3];
    }
    if n < 3 {
        // Fewer than three keys: the straight line between the two keys.
        if n == 1 {
            return values[0];
        }
        let span = times[1] - times[0];
        let f = if span > 0.0 {
            ((t - times[0]) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (a, b) = (values[0], values[1]);
        return [
            a[0] + (b[0] - a[0]) * f,
            a[1] + (b[1] - a[1]) * f,
            a[2] + (b[2] - a[2]) * f,
        ];
    }
    let Some((lo, u)) = key_pair(times, t) else {
        return [0.0; 3];
    };
    let hi = lo + 1;
    let h = times[hi] - times[lo];
    let m0 = tangent_at(times, values, lo);
    let m1 = tangent_at(times, values, hi);
    hermite(values[lo], values[hi], m0, m1, h, u)
}

/// Sample one rotation track with the C1 curve, the same shape as the
/// position track: the key quaternions are hit exactly and the rate at a key
/// is the same on both sides of it. The neighbours are lined up with the
/// key's own sign first, so a key written as `-q` (the same rotation) does
/// not read as a flip.
fn sample_quat_track_c1(track: &QuatTrack, t: f32) -> Option<[f32; 4]> {
    let n = track.values.len();
    if track.times.len() != n || n == 0 {
        return None;
    }
    if track.times.iter().any(|v| !v.is_finite()) {
        return None;
    }
    if track.values.iter().flatten().any(|v| !v.is_finite()) {
        return None;
    }
    if n < 3 {
        return sample_quat_track(track, t);
    }
    let (lo, u) = key_pair(&track.times, t)?;
    let hi = lo + 1;
    let p1 = track.values[lo];
    // The segment's second key on the short arc: a key written as `-q` is
    // the same rotation, so the pair never reads as a flip.
    let flip = quat_dot(p1, track.values[hi]) < 0.0;
    let p2 = if flip {
        [
            -track.values[hi][0],
            -track.values[hi][1],
            -track.values[hi][2],
            -track.values[hi][3],
        ]
    } else {
        track.values[hi]
    };
    let h = track.times[hi] - track.times[lo];
    let m0 = quat_tangent_at(track, lo);
    let mut m1 = quat_tangent_at(track, hi);
    if flip {
        // The tangent belongs to the key's own sign; the segment's end is
        // the other sign, so the tangent turns with it.
        for m in m1.iter_mut() {
            *m = -*m;
        }
    }
    Some(unit_quat(hermite(p1, p2, m0, m1, h, u)))
}

/// The tangent at one key of a rotation track: the component-wise central
/// difference of the neighbouring keys, with the neighbours lined up with
/// the key's own sign so a key written as `-q` does not read as a flip.
fn quat_tangent_at(track: &QuatTrack, i: usize) -> [f32; 4] {
    let n = track.values.len();
    let prev = i.saturating_sub(1);
    let next = (i + 1).min(n - 1);
    let p = track.values[i];
    let align = |q: [f32; 4]| {
        if quat_dot(q, p) < 0.0 {
            [-q[0], -q[1], -q[2], -q[3]]
        } else {
            q
        }
    };
    let prev_v = align(track.values[prev]);
    let next_v = align(track.values[next]);
    let central = rate_between(prev_v, next_v, track.times[next] - track.times[prev]);
    let chord_in = if i > 0 {
        Some(rate_between(prev_v, p, track.times[i] - track.times[prev]))
    } else {
        None
    };
    let chord_out = if i + 1 < n {
        Some(rate_between(p, next_v, track.times[next] - track.times[i]))
    } else {
        None
    };
    tangent_of(central, chord_in, chord_out)
}

/// Sample one quat track at a wrapped time; `None` when the track is absent
/// or invalid (caller falls back to identity).
fn sample_quat_track(track: &QuatTrack, time: f32) -> Option<[f32; 4]> {
    if track.times.len() != track.values.len() || track.times.is_empty() {
        return None;
    }
    if track.times.iter().any(|t| !t.is_finite()) {
        return None;
    }
    let mut prev = track.times[0];
    for t in &track.times[1..] {
        if *t <= prev {
            return None;
        }
        prev = *t;
    }
    if track.values.iter().flatten().any(|v| !v.is_finite()) {
        return None;
    }
    let last = track.times.len() - 1;
    if time <= track.times[0] {
        return Some(track.values[0]);
    }
    if time >= track.times[last] {
        return Some(track.values[last]);
    }
    let mut lo = 0usize;
    let mut hi = last;
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if track.times[mid] <= time {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let span = track.times[hi] - track.times[lo];
    let f = if span <= 0.0 {
        0.0
    } else {
        clamp01((time - track.times[lo]) / span)
    };
    Some(nlerp(track.values[lo], track.values[hi], f))
}

/// Sample a skeleton pose from a position clip at `time` seconds.
///
/// Positions come from `clip.tracks[joint]` sampled with loop-phase wrap
/// (times past the clip end wrap into `[0, duration)`); rotations are
/// identity because [`Clip`] carries no quaternion lanes. Rejects an invalid
/// skeleton, a joint-count mismatch, and malformed/non-finite tracks.
pub fn sample_pose(skeleton: &Skeleton, clip: &Clip, time: f32) -> Result<LocalPose, AnimError> {
    sample_pose_with_rots(skeleton, clip, &[], time)
}

/// Sample a skeleton pose with optional rotation tracks into caller buffers.
///
/// Same validation and bit-identical output as [`sample_pose_with_rots`],
/// but reuses caller-owned `out` buffers: positions/rotations are cleared
/// then filled within capacity, so one warm call at the fighter joint count
/// makes steady-state frames allocation-free. On error `out` is untouched.
/// Units: meters, seconds, radians; ticks at 60 Hz. Determinism: same inputs
/// twice give identical outputs; no wall clock. Errors: same set as
/// [`sample_pose_with_rots`].
pub fn sample_pose_with_rots_into(
    skeleton: &Skeleton,
    clip: &Clip,
    rots: &[QuatTrack],
    time: f32,
    out: &mut LocalPose,
) -> Result<(), AnimError> {
    validate_skeleton(skeleton)?;
    if clip.tracks.len() != skeleton.len() {
        return Err(AnimError::LengthMismatch {
            expected: skeleton.len(),
            got: clip.tracks.len(),
        });
    }
    for (joint, track) in clip.tracks.iter().enumerate() {
        check_track(joint, &track.times, track.values.len())?;
        if track.values.iter().flatten().any(|v| !v.is_finite()) {
            return Err(AnimError::NonFinite("clip.values"));
        }
    }
    let wrapped = wrap_time(time, clip_duration(clip));
    out.positions.clear();
    out.rotations.clear();
    out.positions.reserve(skeleton.len());
    out.rotations.reserve(skeleton.len());
    for joint in 0..skeleton.len() {
        out.positions.push(sample_track_c1(
            &clip.tracks[joint].times,
            &clip.tracks[joint].values,
            wrapped,
        ));
        out.rotations.push(
            rots.get(joint)
                .and_then(|track| sample_quat_track_c1(track, wrapped))
                .unwrap_or([0.0, 0.0, 0.0, 1.0]),
        );
    }
    Ok(())
}

/// Sample a skeleton pose with optional rotation tracks.
///
/// Same as [`sample_pose`], plus per-joint quaternions: joint `j` keeps the
/// sampled quat when `rots[j]` exists and is valid, else identity. Extra
/// rotation tracks past the joint count are ignored.
pub fn sample_pose_with_rots(
    skeleton: &Skeleton,
    clip: &Clip,
    rots: &[QuatTrack],
    time: f32,
) -> Result<LocalPose, AnimError> {
    let mut out = LocalPose {
        positions: Vec::new(),
        rotations: Vec::new(),
    };
    sample_pose_with_rots_into(skeleton, clip, rots, time, &mut out)?;
    Ok(out)
}

/// Blend two poses joint by joint: positions lerp, rotations take the
/// shortest-arc normalized lerp. `t` is `clamp01` with NaN-safe `0.0`, and
/// the endpoints are exact (`t <= 0` returns `a`, `t >= 1` returns `b`).
/// The output covers the joints both poses share; non-finite components fall
/// back per lane to `a`, then `b`, then `0.0` (identity for rotations).
pub fn blend_poses(a: &LocalPose, b: &LocalPose, t: f32) -> LocalPose {
    const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    let w = clamp01(t);
    let joints = a
        .positions
        .len()
        .min(a.rotations.len())
        .min(b.positions.len())
        .min(b.rotations.len());
    if w <= 0.0 {
        return LocalPose {
            positions: a.positions[..joints].to_vec(),
            rotations: a.rotations[..joints].to_vec(),
        };
    }
    if w >= 1.0 {
        return LocalPose {
            positions: b.positions[..joints].to_vec(),
            rotations: b.rotations[..joints].to_vec(),
        };
    }
    let mut positions = Vec::with_capacity(joints);
    let mut rotations = Vec::with_capacity(joints);
    for joint in 0..joints {
        let blended = lerp3(a.positions[joint], b.positions[joint], w);
        let mut out = [0.0f32; 3];
        for lane in 0..3 {
            out[lane] = if blended[lane].is_finite() {
                blended[lane]
            } else if a.positions[joint][lane].is_finite() {
                a.positions[joint][lane]
            } else if b.positions[joint][lane].is_finite() {
                b.positions[joint][lane]
            } else {
                0.0
            };
        }
        positions.push(out);
        let q = nlerp(a.rotations[joint], b.rotations[joint], w);
        rotations.push(if q.iter().all(|v| v.is_finite()) {
            q
        } else {
            IDENTITY
        });
    }
    LocalPose {
        positions,
        rotations,
    }
}

/// Extract per-tick root motion from one loop of samples: planar `pos_xz`
/// positions in meters plus `yaw` angles in radians, one entry per tick.
///
/// Delta `k` spans sample `k` to sample `(k + 1) % n`, so the last delta
/// wraps last-to-first and the table tiles seamlessly when the clip loops;
/// yaw deltas take the shortest arc (`[-PI, PI)`). Ticks run
/// `start_tick..start_tick + n`. Summing the first `n - 1` deltas reproduces
/// the loop travel (`last - first`); including the seam closes the loop.
/// Rejects length mismatches and non-finite samples; the result always
/// passes [`validate_root`].
pub fn extract_root(
    pos_xz: &[[f32; 2]],
    yaw: &[f32],
    start_tick: u32,
) -> Result<RootTable, AnimError> {
    if pos_xz.len() != yaw.len() {
        return Err(AnimError::LengthMismatch {
            expected: pos_xz.len(),
            got: yaw.len(),
        });
    }
    if pos_xz.iter().flatten().any(|v| !v.is_finite()) || yaw.iter().any(|v| !v.is_finite()) {
        return Err(AnimError::NonFinite("root"));
    }
    let n = pos_xz.len();
    let mut deltas = Vec::with_capacity(n);
    for k in 0..n {
        let next = (k + 1) % n;
        deltas.push(RootDelta {
            tick: start_tick.wrapping_add(k as u32),
            dp: [
                pos_xz[next][0] - pos_xz[k][0],
                pos_xz[next][1] - pos_xz[k][1],
            ],
            dyaw: wrap_pi(yaw[next] - yaw[k]),
        });
    }
    let table = RootTable { deltas };
    validate_root(&table)?;
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::super::contract::{skinning_matrices, validate_pose};
    use super::super::{Clip, Track};
    use super::*;

    fn skeleton() -> Skeleton {
        use super::super::contract::Joint;
        Skeleton {
            joints: vec![
                Joint {
                    name: "root".into(),
                    parent: None,
                },
                Joint {
                    name: "spine".into(),
                    parent: Some(0),
                },
            ],
        }
    }

    fn walk_clip() -> Clip {
        Clip {
            tracks: vec![
                Track {
                    times: vec![0.0, 1.0],
                    values: vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
                },
                Track {
                    times: vec![0.0, 1.0],
                    values: vec![[0.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
                },
            ],
        }
    }

    #[test]
    fn phase_wrap_past_clip_end() {
        let (skeleton, clip) = (skeleton(), walk_clip());
        let a = sample_pose(&skeleton, &clip, 0.25).unwrap();
        let b = sample_pose(&skeleton, &clip, 1.25).unwrap();
        assert_eq!(a, b);
        let c = sample_pose(&skeleton, &clip, 2.5).unwrap();
        let d = sample_pose(&skeleton, &clip, 0.5).unwrap();
        assert_eq!(c, d);
        // Wrapped sample matches the analytic midpoint.
        assert_eq!(a.positions[0], [0.5, 0.0, 0.0]);
        assert_eq!(a.rotations[0], [0.0, 0.0, 0.0, 1.0]);
        validate_pose(&skeleton, &a).unwrap();
    }

    #[test]
    fn invalid_skeleton_and_track_rejection() {
        let clip = walk_clip();
        assert_eq!(
            sample_pose(&Skeleton::default(), &clip, 0.0),
            Err(AnimError::EmptySkeleton)
        );
        let mut bad = skeleton();
        bad.joints[1].parent = Some(7);
        assert_eq!(sample_pose(&bad, &clip, 0.0), Err(AnimError::BadParent(1)));
        // Joint-count mismatch names expected vs got.
        let short = Clip {
            tracks: vec![clip.tracks[0].clone()],
        };
        assert_eq!(
            sample_pose(&skeleton(), &short, 0.0),
            Err(AnimError::LengthMismatch {
                expected: 2,
                got: 1
            })
        );
        // Times/values desync inside a track is rejected.
        let mut desync = walk_clip();
        desync.tracks[0].values.pop();
        assert_eq!(
            sample_pose(&skeleton(), &desync, 0.0),
            Err(AnimError::LengthMismatch {
                expected: 2,
                got: 1
            })
        );
        // Non-finite samples are rejected by lane.
        let mut nan = walk_clip();
        nan.tracks[0].values[1][0] = f32::NAN;
        assert_eq!(
            sample_pose(&skeleton(), &nan, 0.0),
            Err(AnimError::NonFinite("clip.values"))
        );
        let mut nan_t = walk_clip();
        nan_t.tracks[1].times[1] = f32::INFINITY;
        assert_eq!(
            sample_pose(&skeleton(), &nan_t, 0.0),
            Err(AnimError::NonFinite("clip.times"))
        );
        // Non-increasing times are rejected, never silently sampled.
        let mut unsorted = walk_clip();
        unsorted.tracks[0].times = vec![0.0, 0.0];
        assert_eq!(
            sample_pose(&skeleton(), &unsorted, 0.0),
            Err(AnimError::BadRootOrder(0))
        );
    }

    #[test]
    fn root_deltas_sum_to_full_loop_travel() {
        // Three ticks of steady +x travel, yaw holding still.
        let pos = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]];
        let yaw = [0.0, 0.0, 0.0];
        let table = extract_root(&pos, &yaw, 7).unwrap();
        assert_eq!(table.deltas.len(), 3);
        assert_eq!(table.deltas[0].tick, 7);
        assert_eq!(table.deltas[2].tick, 9);
        assert_eq!(table.deltas[0].dp, [1.0, 0.0]);
        assert_eq!(table.deltas[1].dp, [1.0, 0.0]);
        // Loop-aware seam: last sample wraps back to the first.
        assert_eq!(table.deltas[2].dp, [-2.0, 0.0]);
        // Interior deltas sum to the full-loop travel (last - first).
        let travel = [
            table.deltas[0].dp[0] + table.deltas[1].dp[0],
            table.deltas[0].dp[1] + table.deltas[1].dp[1],
        ];
        assert_eq!(travel, [2.0, 0.0]);
        // Including the seam closes the loop exactly.
        let closed = [
            travel[0] + table.deltas[2].dp[0],
            travel[1] + table.deltas[2].dp[1],
        ];
        assert_eq!(closed, [0.0, 0.0]);
    }

    #[test]
    fn root_yaw_takes_shortest_arc_across_branch_cut() {
        // Yaw spins +90 deg per tick through a full turn; the seam from
        // -90 deg back to 0 deg takes the short +90 deg arc, completing it.
        let pos = [[0.0, 0.0]; 4];
        let quarter = PI / 2.0;
        let yaw = [0.0, quarter, PI, -quarter];
        let table = extract_root(&pos, &yaw, 0).unwrap();
        assert!((table.deltas[0].dyaw - quarter).abs() < 1e-6);
        assert!((table.deltas[1].dyaw - quarter).abs() < 1e-6);
        assert!((table.deltas[2].dyaw - quarter).abs() < 1e-6);
        assert!((table.deltas[3].dyaw - quarter).abs() < 1e-6);
        // Full-turn travel is preserved across the cut.
        let total: f32 = table.deltas.iter().map(|d| d.dyaw).sum();
        assert!((total - TAU).abs() < 1e-5, "{total}");
    }

    #[test]
    fn extract_root_rejects_bad_samples() {
        assert_eq!(
            extract_root(&[[0.0, 0.0]], &[0.0, 0.0], 0),
            Err(AnimError::LengthMismatch {
                expected: 1,
                got: 2
            })
        );
        assert_eq!(
            extract_root(&[[f32::NAN, 0.0]], &[0.0], 0),
            Err(AnimError::NonFinite("root"))
        );
        assert_eq!(
            extract_root(&[[0.0, 0.0]], &[f32::INFINITY], 0),
            Err(AnimError::NonFinite("root"))
        );
        // Empty input extracts an empty (still valid) table.
        assert_eq!(
            extract_root(&[], &[], 3).unwrap(),
            RootTable { deltas: vec![] }
        );
    }

    #[test]
    fn blend_endpoints_exact_and_nan_safe() {
        // Literal poses (sampling wraps at the clip end, so endpoints come
        // from data, not from clamped sample times).
        let a = LocalPose {
            positions: vec![[0.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; 2],
        };
        let b = LocalPose {
            positions: vec![[2.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; 2],
        };
        assert_eq!(blend_poses(&a, &b, 0.0), a);
        assert_eq!(blend_poses(&a, &b, 1.0), b);
        // Out-of-range weights clamp to the endpoints bit-exactly.
        assert_eq!(blend_poses(&a, &b, -3.0), a);
        assert_eq!(blend_poses(&a, &b, 3.0), b);
        // NaN weight is safe and selects `a`.
        assert_eq!(blend_poses(&a, &b, f32::NAN), a);
        // Midpoint positions lerp; rotations stay unit.
        let mid = blend_poses(&a, &b, 0.5);
        assert_eq!(mid.positions[0], [1.0, 0.0, 0.0]);
        validate_pose(&skeleton(), &mid).unwrap();
        // NaN data never escapes as NaN: falls back per lane.
        let mut poisoned = b.clone();
        poisoned.positions[0][0] = f32::NAN;
        let safe = blend_poses(&a, &poisoned, 0.5);
        assert!(safe.positions[0][0].is_finite());
        assert_eq!(safe.positions[0][0], a.positions[0][0]);
        validate_pose(&skeleton(), &safe).unwrap();
    }

    #[test]
    fn rotation_tracks_blend_shortest_arc() {
        let (skeleton, clip) = (skeleton(), walk_clip());
        // 90-degree yaw on the root joint, from named constants (no trig in
        // this crate): half-angle PI/4, sin == cos == FRAC_1_SQRT_2.
        let rots = vec![
            QuatTrack {
                times: vec![0.0, 1.0],
                values: vec![
                    [0.0, 0.0, 0.0, 1.0],
                    [
                        0.0,
                        std::f32::consts::FRAC_1_SQRT_2,
                        0.0,
                        std::f32::consts::FRAC_1_SQRT_2,
                    ],
                ],
            },
            QuatTrack::default(),
        ];
        let start = sample_pose_with_rots(&skeleton, &clip, &rots, 0.0).unwrap();
        assert_eq!(start.rotations[0], [0.0, 0.0, 0.0, 1.0]);
        // Missing/invalid rot tracks fall back to identity.
        assert_eq!(start.rotations[1], [0.0, 0.0, 0.0, 1.0]);
        // Midpoint of identity to 90-deg yaw is the 45-deg yaw quat.
        let mid = sample_pose_with_rots(&skeleton, &clip, &rots, 0.5).unwrap();
        for (got, want) in mid.rotations[0]
            .iter()
            .zip([0.0, 0.38268343, 0.0, 0.923_879_5])
        {
            assert!((got - want).abs() < 1e-5, "{got} vs {want}");
        }
        validate_pose(&skeleton, &mid).unwrap();
        // Rotation sampling wraps with the same phase as positions.
        assert_eq!(
            sample_pose_with_rots(&skeleton, &clip, &rots, 1.5).unwrap(),
            mid
        );
        // A corrupt rot track degrades to identity, never an error.
        let bad = vec![QuatTrack {
            times: vec![0.0, 0.0],
            values: vec![[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 1.0]],
        }];
        let fallback = sample_pose_with_rots(&skeleton, &clip, &bad, 0.5).unwrap();
        assert_eq!(fallback.rotations[0], [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn replay_is_identical_across_runs() {
        let (skeleton, clip) = (skeleton(), walk_clip());
        let first = sample_pose(&skeleton, &clip, 1.25).unwrap();
        let second = sample_pose(&skeleton, &clip, 1.25).unwrap();
        assert_eq!(first, second);
        let blend_first = blend_poses(&first, &second, 0.37);
        let blend_second = blend_poses(&first, &second, 0.37);
        assert_eq!(blend_first, blend_second);
        let pos = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]];
        let yaw = [0.0, 0.0, 0.0];
        assert_eq!(
            extract_root(&pos, &yaw, 7).unwrap(),
            extract_root(&pos, &yaw, 7).unwrap()
        );
        // Sampled poses feed skinning directly (the render consumer path).
        let matrices = skinning_matrices(&skeleton, &first).unwrap();
        assert_eq!(matrices.len(), 2);
        assert_eq!(matrices[0][3], [0.5, 0.0, 0.0, 1.0]);
        assert_eq!(matrices, skinning_matrices(&skeleton, &first).unwrap());
    }

    #[test]
    fn repeated_sampling_reuses_buffer_without_alloc() {
        use engine_core::alloc_probe::measure;
        let (skeleton, clip) = (skeleton(), walk_clip());
        let rots = vec![
            QuatTrack {
                times: vec![0.0, 1.0],
                values: vec![
                    [0.0, 0.0, 0.0, 1.0],
                    [
                        0.0,
                        std::f32::consts::FRAC_1_SQRT_2,
                        0.0,
                        std::f32::consts::FRAC_1_SQRT_2,
                    ],
                ],
            },
            QuatTrack::default(),
        ];
        let mut pose = LocalPose {
            positions: Vec::new(),
            rotations: Vec::new(),
        };
        sample_pose_with_rots_into(&skeleton, &clip, &rots, 0.25, &mut pose).unwrap();
        let first = pose.clone();
        sample_pose_with_rots_into(&skeleton, &clip, &rots, 0.25, &mut pose).unwrap();
        assert_eq!(pose, first, "same time twice must be bit-identical");
        let fresh = sample_pose_with_rots(&skeleton, &clip, &rots, 0.25).unwrap();
        assert_eq!(
            pose, fresh,
            "reuse path must match allocating path bit for bit"
        );
        let ((), heap) = measure(|| {
            for t in [0.25f32, 0.5, 0.75, 1.25] {
                sample_pose_with_rots_into(&skeleton, &clip, &rots, t, &mut pose).unwrap();
                std::hint::black_box(&pose);
            }
        });
        assert_eq!(
            heap.calls, 0,
            "steady-state reuse must not allocate, got {heap:?}"
        );
    }
}
