//! AM24: a keyframed clip read as a rate corner.
//!
//! The arena's pop law (`crates/game/src/view_truth.rs`) calls a joint that
//! misses its constant-velocity prediction by more than 10 cm a pop. The
//! fighter clips are keyed at 24 Hz and read at 60 Hz, so the straight line
//! between two keys changes its rate at every key: the fighter's feet and
//! hands leave their own speed by 11 to 15 cm every 1.5 frames.
//!
//! The pose sampler reads a key track with a C1 curve: the key values are
//! hit exactly, the rate at a key is the same on both sides of it, and the
//! sampled position never leaves the key pair's span. The corners stop
//! reading as pops.
//!
//! The fixture is the run clip's `leg_lower_R` world rotation, the seven
//! 24 Hz keys from `content/fighters/glb/clips/human_male_run.glb`, folded
//! into one track: the root of a two-joint rig carries the rotation and the
//! shin hangs at the joint's world offset. Read with straight lines the
//! shin misses its own velocity by 12.8 cm (a pop); read with the C1 curve
//! it misses by 8.5 cm (under the line).

use crate::contract::{skinning_matrices, Joint, LocalPose, Skeleton};
use crate::pose::{sample_pose_with_rots, QuatTrack};
use crate::{sample, Clip, Track};

/// The pop law's line: a joint that misses its constant-velocity prediction
/// by more than this in one tick popped (mirrors `POP_M` in the view truth).
const POP_M: f32 = 0.10;
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

/// The shortest-arc normalized lerp between two keys of a rotation track:
/// the shape the rotation sampler had before the C1 curve, kept here as the
/// oracle.
fn straight_line_quat(track: &QuatTrack, t: f32) -> Option<[f32; 4]> {
    let n = track.values.len();
    if n == 0 || track.times.len() != n {
        return None;
    }
    if t <= track.times[0] {
        return Some(track.values[0]);
    }
    if t >= track.times[n - 1] {
        return Some(track.values[n - 1]);
    }
    let mut lo = 0usize;
    let mut hi = n - 1;
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if track.times[mid] <= t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let span = track.times[hi] - track.times[lo];
    let f = if span > 0.0 {
        ((t - track.times[lo]) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (a, b) = (track.values[lo], track.values[hi]);
    let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    let sign = if dot < 0.0 { -1.0 } else { 1.0 };
    let mixed = [
        a[0] + (sign * b[0] - a[0]) * f,
        a[1] + (sign * b[1] - a[1]) * f,
        a[2] + (sign * b[2] - a[2]) * f,
        a[3] + (sign * b[3] - a[3]) * f,
    ];
    let norm = (mixed[0] * mixed[0]
        + mixed[1] * mixed[1]
        + mixed[2] * mixed[2]
        + mixed[3] * mixed[3])
        .sqrt();
    if !norm.is_finite() || norm <= 0.0 {
        return None;
    }
    Some([
        mixed[0] / norm,
        mixed[1] / norm,
        mixed[2] / norm,
        mixed[3] / norm,
    ])
}

/// The straight-line pose: the straight line for the positions and the
/// shortest-arc lerp for the rotations. This is the shape the pose sampler
/// had before the C1 curve, kept here as the oracle.
fn straight_line_pose(skeleton: &Skeleton, clip: &Clip, rots: &[QuatTrack], t: f32) -> LocalPose {
    let positions = (0..skeleton.len())
        .map(|joint| sample(clip, joint, t))
        .collect();
    let rotations = (0..skeleton.len())
        .map(|joint| {
            rots.get(joint)
                .and_then(|track| straight_line_quat(track, t))
                .unwrap_or([0.0, 0.0, 0.0, 1.0])
        })
        .collect();
    LocalPose {
        positions,
        rotations,
    }
}

/// The world's position of one joint in a pose.
fn joint_world(skeleton: &Skeleton, pose: &LocalPose, joint: usize) -> [f32; 3] {
    let world = skinning_matrices(skeleton, pose).expect("the rig is valid");
    [world[joint][3][0], world[joint][3][1], world[joint][3][2]]
}

/// The pop law over the fixture's window: the largest miss of the joint's
/// constant-velocity prediction, in meters.
fn pop_miss(mut at: impl FnMut(f32) -> [f32; 3]) -> f32 {
    let dt = 1.0 / TICK_HZ;
    let mut prev: Option<[f32; 3]> = None;
    let mut vel: Option<[f32; 3]> = None;
    let mut worst = 0.0f32;
    for tick in 0..=TICKS {
        let now = at(FIXTURE_TIMES[0] + tick as f32 * dt);
        if let (Some(then), Some(v)) = (prev, vel) {
            let mut miss = 0.0f32;
            for k in 0..3 {
                miss = miss.max((now[k] - (then[k] + v[k] * dt)).abs());
            }
            worst = worst.max(miss);
        }
        if let Some(then) = prev {
            vel = Some([
                (now[0] - then[0]) / dt,
                (now[1] - then[1]) / dt,
                (now[2] - then[2]) / dt,
            ]);
        }
        prev = Some(now);
    }
    worst
}

/// The largest difference between the rate before a key and the rate after
/// it: the corner the straight line puts at every key.
fn rate_jump(at: &mut dyn FnMut(f32) -> [f32; 4], t: f32, eps: f32) -> f32 {
    let mid = at(t);
    let align = |q: [f32; 4]| {
        let dot = q[0] * mid[0] + q[1] * mid[1] + q[2] * mid[2] + q[3] * mid[3];
        if dot < 0.0 {
            [-q[0], -q[1], -q[2], -q[3]]
        } else {
            q
        }
    };
    let before = align(at(t - eps));
    let after = align(at(t + eps));
    let mut worst = 0.0f32;
    for k in 0..4 {
        let rate_before = (mid[k] - before[k]) / eps;
        let rate_after = (after[k] - mid[k]) / eps;
        worst = worst.max((rate_after - rate_before).abs());
    }
    worst
}

/// The same corner for a scalar channel: the largest difference between the
/// rate before a key and the rate after it. The quaternion rule above aligns
/// the sign of the quaternions it compares; a scalar channel has no sign to
/// align, so the corner is read straight off the two rates.
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

#[test]
fn a_24_hertz_rotation_track_stops_reading_as_a_joint_pop() {
    // The run clip's `leg_lower_R` world rotation, read at 60 Hz. The
    // straight line between two keys flips the shin's speed every key, so
    // the shin leaves its own speed by 12.8 cm in a tick: a pop. The C1
    // curve reads the same keys without the corner.
    let (skeleton, clip) = (rig(), fixture_clip());
    let rots = fixture_rots();
    let straight = pop_miss(|t| {
        let pose = straight_line_pose(&skeleton, &clip, &rots, t);
        joint_world(&skeleton, &pose, 1)
    });
    assert!(
        straight > POP_M,
        "the straight line must cross the pop line, got {straight} m"
    );
    let curved = pop_miss(|t| {
        let pose = sample_pose_with_rots(&skeleton, &clip, &rots, t).expect("the pose samples");
        joint_world(&skeleton, &pose, 1)
    });
    assert!(
        curved < POP_M,
        "the C1 curve must stay under the pop line, got {curved} m"
    );
    assert!(
        curved < straight * 0.75,
        "the C1 curve must beat the straight line by a quarter, got {curved} m vs {straight} m"
    );
    println!(
        "fixture joint miss: straight line {straight:.4} m, C1 curve {curved:.4} m (pop line {POP_M} m)"
    );
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

/// The key pair a time falls in: the two keys the sample sits between.
fn fixture_key_pair(t: f32) -> (usize, usize) {
    let mut lo = 0usize;
    for (k, time) in FIXTURE_TIMES.iter().enumerate() {
        if *time <= t {
            lo = k;
        }
    }
    (lo, (lo + 1).min(FIXTURE_TIMES.len() - 1))
}

#[test]
fn the_rotation_rate_does_not_turn_at_a_key() {
    // The corner the straight line puts at every key: the rate before the
    // key and the rate after it differ. The C1 curve carries one rate
    // through the key, so the two sides agree.
    let (skeleton, clip) = (rig(), fixture_clip());
    let rots = fixture_rots();
    let eps = 1e-4;
    for key in [1usize, 2] {
        let t = FIXTURE_TIMES[key];
        let mut straight_at = |t: f32| {
            straight_line_pose(&skeleton, &clip, &rots, t).rotations[0]
        };
        let straight = rate_jump(&mut straight_at, t, eps);
        assert!(
            straight > 0.5,
            "the straight line must turn at the key {key}, got {straight}"
        );
        let mut curved_at = |t: f32| {
            sample_pose_with_rots(&skeleton, &clip, &rots, t)
                .expect("the pose samples")
                .rotations[0]
        };
        let curved = rate_jump(&mut curved_at, t, eps);
        assert!(
            curved < straight * 0.05,
            "the C1 curve must not turn at the key {key}: {curved} vs {straight}"
        );
    }
}

#[test]
fn the_position_rate_does_not_turn_at_a_key() {
    // The same corner through the position channel: the swing's rate falls
    // from 2.4 m/s to 1.2 m/s at a key, and the straight line turns there.
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
    let seam = sample_pose_with_rots(&skeleton, &clip, &rots, FIXTURE_TIMES[6])
        .expect("the pose samples");
    let first = sample_pose_with_rots(&skeleton, &clip, &rots, FIXTURE_TIMES[0])
        .expect("the pose samples");
    assert_eq!(
        seam.rotations[0], first.rotations[0],
        "the clip's end must read as the clip's start"
    );
}

#[test]
fn the_curve_hits_its_last_key_exactly() {
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
fn a_flipped_quaternion_key_does_not_read_as_a_flip() {
    // The fixture's key 3 is written with the opposite sign: the same
    // rotation, the other way round. The sampler lines the neighbours up
    // with the key's own sign, so the track does not read as a flip: the
    // sampled rotation stays on the short arc of the keys it sits between
    // and stays a unit quaternion.
    //
    // The arc is the short way round, so the sample's angle from either end
    // of the pair is never larger than the pair's own angle. In dot terms:
    // |dot(sample, end)| stays at or above |dot(end, other end)|, the value
    // the pair's own angle gives. A sampler that took the long way round
    // would drop below it.
    let (skeleton, clip) = (rig(), fixture_clip());
    let rots = fixture_rots();
    for tick in 0..=TICKS {
        let t = FIXTURE_TIMES[0] + tick as f32 / TICK_HZ;
        let pose = sample_pose_with_rots(&skeleton, &clip, &rots, t).expect("the pose samples");
        let q = pose.rotations[0];
        let norm = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "the rotation left the unit sphere: {q:?}");
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
