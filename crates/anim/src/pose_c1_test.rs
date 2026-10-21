//! AM24: a keyframed clip read as a rate corner.
//!
//! The arena's pop law (`crates/game/src/view_truth.rs`) calls a joint that
//! misses its constant-velocity prediction by more than 10 cm a pop. The
//! fighter clips are keyed at 24 Hz and read at 60 Hz, so the straight line
//! between two keys changes its rate at every key: the fighter's feet and
//! hands leave their own speed by 11 to 15 cm every 1.5 frames.
//!
//! The pose sampler reads a position key track with a C1 curve: the key
//! values are hit exactly, the rate at a key is the same on both sides of
//! it, and the sampled position never leaves the key pair's span. The
//! corners stop reading as pops.
//!
//! A rotation key track is read with the straight shortest-arc lerp. The
//! pop law reads a joint's world angular rate, and a per-segment Hermite
//! cannot keep its peak rate under the chord's rate (the mean value
//! theorem), so no C1 rotation read can beat the straight read on that law.
//! The arena's own line is the measure: on the seeded 2-fighter line the C1
//! rotation read pops five times and the straight read pops none.
//!
//! The fixture is the run clip's `leg_lower_R` world rotation, the seven
//! 24 Hz keys from `content/fighters/glb/clips/human_male_run.glb`, folded
//! into one track: the root of a two-joint rig carries the rotation and the
//! shin hangs at the joint's world offset.

use crate::clip::clamp01;
use crate::contract::{Joint, Skeleton};
use crate::pose::{sample_pose_with_rots, QuatTrack};
use crate::{sample, Clip, Track};

/// The key rate of the fighter clips: 24 Hz.
const KEY_HZ: f32 = 24.0;
/// The sim tick: 60 Hz.
const TICK_HZ: f32 = 60.0;
/// The fixture's key times: the run clip's keys 5 to 11, at 24 Hz.
const FIXTURE_TIMES: [f32; 7] = [
    5.0 / 24.0,
    6.0 / 24.0,
    7.0 / 24.0,
    8.0 / 24.0,
    9.0 / 24.0,
    10.0 / 24.0,
    11.0 / 24.0,
];
/// The fixture's rotations: the run clip's `leg_lower_R` world rotation at
/// every key. Key 3 is written with the opposite sign (the same rotation,
/// the other way round), exactly as the clip stores it.
const FIXTURE_ROTS: [[f32; 4]; 7] = [
    [-0.955753, -0.006372, 0.032897, 0.292401],
    [-0.916248, -0.023671, 0.046343, 0.398280],
    [-0.840536, -0.046262, 0.046546, 0.538944],
    [0.819035, 0.053147, -0.043424, -0.570533],
    [0.812448, 0.056730, -0.039370, -0.579568],
    [0.783757, 0.063538, -0.033904, -0.617355],
    [0.771149, 0.065532, -0.024569, -0.633047],
];
/// The joint's world offset from the rig's root, in meters.
const FIXTURE_OFFSET: [f32; 3] = [-0.25361, 1.17926, 0.59426];
/// The ticks the pop law reads: the fixture's window at 60 Hz.
const TICKS: usize = 14;
/// The bound the rotation lane's rate has to stay under, as a multiple of
/// the key pair's own chord rate. The straight read runs at 1.00 of it (the
/// measured 1.0025); a C1 read of the same keys runs at 1.64.
const CHORD_RATE_BOUND: f32 = 1.1;

/// A two-joint rig: a root and a shin below it.
fn rig() -> Skeleton {
    Skeleton {
        joints: vec![
            Joint {
                name: "root".into(),
                parent: None,
            },
            Joint {
                name: "shin".into(),
                parent: Some(0),
            },
        ],
    }
}

/// The fixture's clip: a still root and the shin at its world offset.
fn fixture_clip() -> Clip {
    Clip {
        tracks: vec![
            Track {
                times: FIXTURE_TIMES.to_vec(),
                values: vec![[0.0, 0.0, 0.0]; FIXTURE_TIMES.len()],
            },
            Track {
                times: FIXTURE_TIMES.to_vec(),
                values: vec![FIXTURE_OFFSET; FIXTURE_TIMES.len()],
            },
        ],
    }
}

/// The fixture's rotation track: the root carries the joint's world
/// rotation.
fn fixture_rots() -> Vec<QuatTrack> {
    vec![QuatTrack {
        times: FIXTURE_TIMES.to_vec(),
        values: FIXTURE_ROTS.to_vec(),
    }]
}

/// Key times of a 24 Hz track, `keys` of them, starting one key in.
fn key_times(keys: usize) -> Vec<f32> {
    (0..keys).map(|k| (k + 1) as f32 / KEY_HZ).collect()
}

/// The key pair a time falls in: the two keys the sample sits between, with
/// the time inside the pair as `u` in 0..=1.
fn pair_of(times: &[f32], t: f32) -> Option<(usize, f32)> {
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

/// The fixture's key pair at a time: the two keys the sample sits between.
fn fixture_key_pair(t: f32) -> (usize, usize) {
    let (lo, _) = pair_of(&FIXTURE_TIMES, t).expect("the fixture has keys");
    (lo, (lo + 1).min(FIXTURE_TIMES.len() - 1))
}

/// A unit quaternion from any four components: the rotation a quaternion
/// names does not depend on its length, so a key that is a hair off unit
/// still names one rotation.
fn unit(q: [f32; 4]) -> [f32; 4] {
    let n2 = q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3];
    let inv = 1.0 / n2.sqrt();
    [q[0] * inv, q[1] * inv, q[2] * inv, q[3] * inv]
}

/// The dot of two quaternions.
fn qdot(a: [f32; 4], b: [f32; 4]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}

/// The length of a quaternion.
fn qlen(q: [f32; 4]) -> f32 {
    (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt()
}

/// The straight shortest-arc read of a rotation track: the shape the
/// rotation lane has always had, kept here as the oracle. It mirrors the
/// sampler's own arithmetic - the same sign flip, the same normalization,
/// the same key search - so the lane and the oracle agree bit for bit.
fn straight_line_quat(track: &QuatTrack, time: f32) -> Option<[f32; 4]> {
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
    // The shortest arc: a key written as `-q` is the same rotation, so the
    // pair is read the short way round.
    let (a, b) = (track.values[lo], track.values[hi]);
    let dot = qdot(a, b);
    let s = if dot < 0.0 { -1.0 } else { 1.0 };
    let mixed = [
        a[0] + (s * b[0] - a[0]) * f,
        a[1] + (s * b[1] - a[1]) * f,
        a[2] + (s * b[2] - a[2]) * f,
        a[3] + (s * b[3] - a[3]) * f,
    ];
    let norm2 =
        mixed[0] * mixed[0] + mixed[1] * mixed[1] + mixed[2] * mixed[2] + mixed[3] * mixed[3];
    if !norm2.is_finite() {
        return None;
    }
    if norm2 <= 0.0 {
        return Some(a);
    }
    let inv = 1.0 / norm2.sqrt();
    Some([
        mixed[0] * inv,
        mixed[1] * inv,
        mixed[2] * inv,
        mixed[3] * inv,
    ])
}

/// The angle a pair of rotations names, as `sqrt(1 - |dot|)`: the term
/// grows with the angle, so a ratio of two of them is the ratio of the
/// angles themselves, and no transcendental runs.
fn angle_term(a: [f32; 4], b: [f32; 4]) -> f32 {
    let dot = qdot(unit(a), unit(b)).abs().min(1.0);
    (1.0 - dot).max(0.0).sqrt()
}

/// The same corner for a scalar channel: the largest difference between the
/// rate before a key and the rate after it. A quaternion channel has a sign
/// to align, so the corner there is read through the rotations; a scalar
/// channel has none, so the corner is read straight off the two rates.
fn rate_jump_scalar(at: &mut dyn FnMut(f32) -> f32, t: f32, eps: f32) -> f32 {
    let mid = at(t);
    let before = at(t - eps);
    let after = at(t + eps);
    let rate_before = (mid - before) / eps;
    let rate_after = (after - mid) / eps;
    (rate_after - rate_before).abs()
}

/// A position track with the fighter clips' shape: a swing of 15 cm a key
/// that slows down and comes back.
fn swing_track() -> Track {
    let values = vec![
        [0.0, 0.0, 0.0],
        [0.10, 0.0, 0.0],
        [0.15, 0.0, 0.0],
        [0.10, 0.0, 0.0],
        [0.0, 0.0, 0.0],
    ];
    Track {
        times: key_times(values.len()),
        values,
    }
}

/// The tangent rate cap the C1 shape used: a key's tangent never runs past
/// this many times the smallest chord rate around it.
const TANGENT_RATE_CAP: f32 = 3.0;

/// The rate between two quaternion keys per second.
fn rate4(a: [f32; 4], b: [f32; 4], span: f32) -> [f32; 4] {
    if !span.is_finite() || span <= 0.0 {
        return [0.0; 4];
    }
    let mut out = [0.0; 4];
    for k in 0..4 {
        out[k] = (b[k] - a[k]) / span;
    }
    out
}

/// The tangent from a key's central difference and the two chords around it,
/// zeroed at a local extremum and capped at the chord rate.
fn tangent4(
    central: [f32; 4],
    chord_in: Option<[f32; 4]>,
    chord_out: Option<[f32; 4]>,
) -> [f32; 4] {
    let mut out = [0.0; 4];
    for k in 0..4 {
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

/// The cubic Hermite blend of two key values with their tangents.
fn hermite4(
    start: [f32; 4],
    end: [f32; 4],
    m0: [f32; 4],
    m1: [f32; 4],
    h: f32,
    u: f32,
) -> [f32; 4] {
    let u2 = u * u;
    let u3 = u2 * u;
    let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
    let h10 = u3 - 2.0 * u2 + u;
    let h01 = -2.0 * u3 + 3.0 * u2;
    let h11 = u3 - u2;
    let mut out = [0.0; 4];
    for k in 0..4 {
        out[k] = h00 * start[k] + h10 * h * m0[k] + h01 * end[k] + h11 * h * m1[k];
    }
    out
}

/// The tangent at one key of a rotation track: the component-wise central
/// difference of the neighbouring keys, with the neighbours lined up with
/// the key's own sign.
fn quat_tangent_at(track: &QuatTrack, i: usize) -> [f32; 4] {
    let n = track.values.len();
    let prev = i.saturating_sub(1);
    let next = (i + 1).min(n - 1);
    let p = track.values[i];
    let align = |q: [f32; 4]| {
        if qdot(q, p) < 0.0 {
            [-q[0], -q[1], -q[2], -q[3]]
        } else {
            q
        }
    };
    let prev_v = align(track.values[prev]);
    let next_v = align(track.values[next]);
    let central = rate4(prev_v, next_v, track.times[next] - track.times[prev]);
    let chord_in = if i > 0 {
        Some(rate4(prev_v, p, track.times[i] - track.times[prev]))
    } else {
        None
    };
    let chord_out = if i + 1 < n {
        Some(rate4(p, next_v, track.times[next] - track.times[i]))
    } else {
        None
    };
    tangent4(central, chord_in, chord_out)
}

/// The C1 rotation read: a per-segment Hermite over the quaternion
/// components, with the tangents the neighbouring keys' central differences
/// and the neighbours lined up with the key's own sign. This is the shape
/// AM24's first landing used, kept here as the oracle the rotation lane
/// must not read as.
fn curved_read(track: &QuatTrack, t: f32) -> Option<[f32; 4]> {
    let n = track.values.len();
    if track.times.len() != n || n == 0 {
        return None;
    }
    if n < 3 {
        return straight_line_quat(track, t);
    }
    let (lo, u) = pair_of(&track.times, t)?;
    let hi = lo + 1;
    let p1 = track.values[lo];
    // The segment's second key on the short arc: a key written as `-q` is
    // the same rotation, so the pair never reads as a flip.
    let flip = qdot(p1, track.values[hi]) < 0.0;
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
    Some(unit(hermite4(p1, p2, m0, m1, h, u)))
}

/// The worst ratio of a read's per-tick rate to the key pair's own chord
/// rate, over the fixture's window. Only the steps inside one key pair
/// count: a step over a key is the corner the straight read puts there and
/// it has no chord of its own.
fn worst_rate_ratio(at: &mut dyn FnMut(f32) -> [f32; 4]) -> f32 {
    let dt = 1.0 / TICK_HZ;
    let mut worst = 0.0f32;
    let mut prev: Option<([f32; 4], f32)> = None;
    for tick in 0..=TICKS {
        let t = FIXTURE_TIMES[0] + tick as f32 * dt;
        let q = at(t);
        if let Some((before, before_t)) = prev {
            let (lo, hi) = fixture_key_pair(t);
            if fixture_key_pair(before_t).0 == lo {
                let step = angle_term(before, q);
                let chord = angle_term(FIXTURE_ROTS[lo], FIXTURE_ROTS[hi]);
                let span = FIXTURE_TIMES[hi] - FIXTURE_TIMES[lo];
                let ratio = (step / chord) * (span / dt);
                worst = worst.max(ratio);
            }
        }
        prev = Some((q, t));
    }
    worst
}

#[test]
fn the_position_rate_does_not_turn_at_a_key() {
    // The corner the straight line puts at every key: the rate before the
    // key and the rate after it differ. The C1 curve carries one rate
    // through the key, so the two sides agree.
    let skeleton = rig();
    let clip = Clip {
        tracks: vec![swing_track(), swing_track()],
    };
    let eps = 1e-4;
    for key in [1usize, 2, 3] {
        let t = key_times(5)[key];
        let mut straight_at = |t: f32| sample(&clip, 1, t)[0];
        let straight = rate_jump_scalar(&mut straight_at, t, eps);
        assert!(
            straight > 1.0,
            "the straight line must turn at the key {key}, got {straight}"
        );
        let mut curved_at = |t: f32| {
            sample_pose_with_rots(&skeleton, &clip, &[], t)
                .expect("the pose samples")
                .positions[1][0]
        };
        let curved = rate_jump_scalar(&mut curved_at, t, eps);
        assert!(
            curved < straight * 0.05,
            "the C1 curve must not turn at the key {key}: {curved} vs {straight}"
        );
    }
}

#[test]
fn the_sampled_position_never_leaves_the_key_span() {
    // The C1 curve is monotone inside a key pair: the sampled value never
    // runs past the two keys it sits between, so the curve adds no
    // overshoot of its own.
    let skeleton = rig();
    let clip = Clip {
        tracks: vec![swing_track(), swing_track()],
    };
    for key in 0..4 {
        let t0 = key_times(5)[key];
        let t1 = key_times(5)[key + 1];
        let a = clip.tracks[1].values[key][0];
        let b = clip.tracks[1].values[key + 1][0];
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        let mut t = t0;
        while t < t1 {
            let pose = sample_pose_with_rots(&skeleton, &clip, &[], t).expect("the pose samples");
            let x = pose.positions[1][0];
            assert!(
                x >= lo - 1e-5 && x <= hi + 1e-5,
                "the sample at {t} left the key span: {x} not in {lo}..{hi}"
            );
            t += 1.0 / (TICK_HZ * 4.0);
        }
    }
}

#[test]
fn the_sampled_pose_hits_every_key_exactly() {
    let (skeleton, clip) = (rig(), fixture_clip());
    let rots = fixture_rots();
    // The clip's last key sits at the clip's end, where the phase wraps back
    // to the first key: that is the loop seam (a separate open item), so the
    // keys inside the clip are the ones the curve has to hit.
    for key in 0..FIXTURE_TIMES.len() - 1 {
        let t = FIXTURE_TIMES[key];
        let pose = sample_pose_with_rots(&skeleton, &clip, &rots, t).expect("the pose samples");
        assert_eq!(
            pose.positions[1], FIXTURE_OFFSET,
            "the position key {key} moved"
        );
        let want = unit(FIXTURE_ROTS[key]);
        let got = unit(pose.rotations[0]);
        let dot: f32 = want.iter().zip(got.iter()).map(|(a, b)| a * b).sum();
        assert!(
            (dot.abs() - 1.0).abs() < 1e-5,
            "the rotation key {key} moved: {got:?} vs {want:?}"
        );
    }
    // The seam: the clip's end reads as the clip's start, the same way the
    // straight line read it.
    let seam =
        sample_pose_with_rots(&skeleton, &clip, &rots, FIXTURE_TIMES[6]).expect("the pose samples");
    let first =
        sample_pose_with_rots(&skeleton, &clip, &rots, FIXTURE_TIMES[0]).expect("the pose samples");
    assert_eq!(
        seam.rotations[0], first.rotations[0],
        "the clip's end must read as the clip's start"
    );
}

#[test]
fn a_rotation_track_ends_on_its_last_key_exactly() {
    // The clip's end is the wrap point, so a rotation track that ends with
    // the clip is never sampled at its last key. A track that ends inside
    // the clip has that key hit exactly like every other key.
    let skeleton = rig();
    let clip = Clip {
        tracks: vec![
            Track {
                times: vec![0.0, 0.5, 1.0],
                values: vec![[0.0, 0.0, 0.0]; 3],
            },
            Track {
                times: vec![0.0, 0.5, 1.0],
                values: vec![[0.0, 0.0, 0.0]; 3],
            },
        ],
    };
    let rots = vec![QuatTrack {
        times: vec![0.0, 0.25, 0.5],
        values: vec![
            [0.0, 0.0, 0.0, 1.0],
            [
                0.0,
                0.0,
                std::f32::consts::FRAC_1_SQRT_2,
                std::f32::consts::FRAC_1_SQRT_2,
            ],
            [1.0, 0.0, 0.0, 0.0],
        ],
    }];
    let pose = sample_pose_with_rots(&skeleton, &clip, &rots, 0.5).expect("the pose samples");
    let want = unit(rots[0].values[2]);
    let got = unit(pose.rotations[0]);
    let dot: f32 = want.iter().zip(got.iter()).map(|(a, b)| a * b).sum();
    assert!(
        (dot.abs() - 1.0).abs() < 1e-5,
        "the last rotation key moved: {got:?} vs {want:?}"
    );
}

#[test]
fn the_rotation_lane_is_the_straight_shortest_arc() {
    // The rotation lane reads the straight shortest arc, bit for bit: the
    // straight line between two keys, taken the short way round, is the
    // shape the lane has always had. A lane that read a C1 curve here would
    // run its rate past the chord's rate (the next test) and would pop on
    // the arena's own line.
    let (skeleton, clip) = (rig(), fixture_clip());
    let rots = fixture_rots();
    for tick in 0..=TICKS {
        let t = FIXTURE_TIMES[0] + tick as f32 / TICK_HZ;
        let pose = sample_pose_with_rots(&skeleton, &clip, &rots, t).expect("the pose samples");
        let want = straight_line_quat(&rots[0], t).expect("the oracle reads the track");
        assert_eq!(
            pose.rotations[0], want,
            "the rotation lane left the straight arc at {t}"
        );
    }
}

#[test]
fn a_flipped_quaternion_key_does_not_read_as_a_flip() {
    // The fixture's key 3 is written with the opposite sign: the same
    // rotation, the other way round. The lane lines the neighbours up with
    // the key's own sign, so the track does not read as a flip: the sampled
    // rotation stays on the short arc of the keys it sits between and stays
    // a unit quaternion.
    //
    // The arc is the short way round, so the sample's angle from either end
    // of the pair is never larger than the pair's own angle. In dot terms:
    // |dot(sample, end)| stays at or above |dot(end, other end)|, the value
    // the pair's own angle gives. A sampler that took the long way round
    // would drop below it. The comparison is on the rotation, not on the
    // raw length: the fixture's keys are a hair off unit, so the lengths
    // are not the angles.
    let (skeleton, clip) = (rig(), fixture_clip());
    let rots = fixture_rots();
    for tick in 0..=TICKS {
        let t = FIXTURE_TIMES[0] + tick as f32 / TICK_HZ;
        let pose = sample_pose_with_rots(&skeleton, &clip, &rots, t).expect("the pose samples");
        let q = pose.rotations[0];
        let norm = qlen(q);
        // The lane hands a key back unchanged at the clip's edge, and the
        // fixture's keys are a hair off unit (4e-5), so the length is
        // compared with the key's own length. Between keys the lane mixes
        // and normalizes, so the sample is unit.
        let want_len = if tick == 0 {
            qlen(FIXTURE_ROTS[0])
        } else {
            1.0
        };
        assert!(
            (norm - want_len).abs() < 1e-5,
            "the rotation left the unit sphere: {q:?}"
        );
        let (lo, hi) = fixture_key_pair(t);
        let p1 = unit(FIXTURE_ROTS[lo]);
        let p2 = unit(FIXTURE_ROTS[hi]);
        // The pair's own angle, in the dot the half angle gives.
        let pair = qdot(p1, p2).abs();
        for (key, end) in [(lo, p1), (hi, p2)] {
            let got = qdot(unit(q), end).abs();
            assert!(
                got >= pair - 1e-4,
                "the rotation at {t} left the short arc of the keys {lo}..{hi}, \
                 turning away from the key {key}: {q:?}"
            );
        }
    }
}

#[test]
fn the_rotation_rate_never_runs_past_the_chord() {
    // The pop law reads a joint's world angular rate, and the rate a
    // rotation track reads at is the rate the pair of keys it sits between
    // names. The straight read stays on that chord (1.00 of it on this
    // fixture), and that is what keeps the arena's line at pops=0.
    let (skeleton, clip) = (rig(), fixture_clip());
    let rots = fixture_rots();
    let mut lane_at = |t: f32| {
        sample_pose_with_rots(&skeleton, &clip, &rots, t)
            .expect("the pose samples")
            .rotations[0]
    };
    let worst = worst_rate_ratio(&mut lane_at);
    assert!(
        worst < CHORD_RATE_BOUND,
        "the rotation lane must not run past the chord's rate, got {worst}"
    );
    println!("rotation lane: worst rate {worst:.4} of the chord's rate");
}

#[test]
fn a_curved_rotation_read_runs_past_the_chord() {
    // The other shape, the one AM24's first landing used: a C1 curve over
    // the same keys. Its peak rate runs past the chord's rate (1.64 of it
    // on this fixture), which is why the rotation lane does not read it:
    // the pop law reads the rate, and the rate is what the curve bulges.
    let rots = fixture_rots();
    let mut curved_at = |t: f32| curved_read(&rots[0], t).expect("the curve reads the track");
    let worst = worst_rate_ratio(&mut curved_at);
    assert!(
        worst > CHORD_RATE_BOUND,
        "the C1 read must run past the chord's rate, got {worst}"
    );
    println!("curved rotation read: worst rate {worst:.4} of the chord's rate");
}

#[test]
fn sampling_repeats_bit_for_bit() {
    let (skeleton, clip) = (rig(), fixture_clip());
    let rots = fixture_rots();
    let first = sample_pose_with_rots(&skeleton, &clip, &rots, 0.3).expect("the pose samples");
    let second = sample_pose_with_rots(&skeleton, &clip, &rots, 0.3).expect("the pose samples");
    assert_eq!(first, second, "the same sample twice gives the same pose");
}

#[test]
fn a_short_track_reads_as_the_straight_line() {
    // Two keys cannot carry a tangent: the track reads as the straight line
    // between them.
    let skeleton = rig();
    let clip = Clip {
        tracks: vec![
            Track {
                times: vec![0.0, 0.5],
                values: vec![[0.0, 0.0, 0.0], [0.0, 0.0, 0.0]],
            },
            Track {
                times: vec![0.0, 0.5],
                values: vec![[0.0, -0.5, 0.0], [0.5, -0.5, 0.0]],
            },
        ],
    };
    let pose = sample_pose_with_rots(&skeleton, &clip, &[], 0.25).expect("the pose samples");
    assert_eq!(pose.positions[1], [0.25, -0.5, 0.0]);
}
