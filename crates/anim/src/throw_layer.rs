//! throw_layer: the spear throw as a masked override on the final pose (SG3).
//!
//! Consumer: the fighter look runs [`apply_throw`] as its last pose stage, after
//! the inertial offsets and the planted feet, so the arm layer never feeds an
//! inertializer (an additive layer would be inertialized twice). Owner:
//! `engine-anim`; this module owns the throw timeline and the arm targets, and
//! reuses [`crate::ik::solve_limb`] for the arm and the shortest-arc blend of
//! [`crate::layers`] for the weight. The arm loads back, sweeps forward to point where the
//! spear goes, holds, and lets go. Units: meters, unit quats `[x, y, z, w]`,
//! ticks at 60 Hz; the body faces +z with y up. Determinism: `+ - * /` and
//! `sqrt` only, no clock; equal inputs give equal bits. Errors: [`IkError`]; a
//! refused call leaves the pose as it was. Heap: none per call.

use super::blend::lerp3;
use super::contract::{LocalPose, Skeleton};
use super::graph::nlerp;
use super::ik::{limb_positions, solve_limb, IkError, Limb};

/// Ticks the arm takes to load back from the clip's arm.
pub const WIND_TICKS: f32 = 4.0;
/// Ticks the arm takes to sweep from loaded to pointing.
pub const RELEASE_TICKS: f32 = 3.0;
/// Ticks the arm stays pointing after the sweep.
pub const HOLD_TICKS: f32 = 8.0;
/// Ticks the arm takes to give the pose back to the clip.
pub const RECOVER_TICKS: f32 = 16.0;
/// The age at which the arm first points.
const POINT_AT_TICKS: f32 = WIND_TICKS + RELEASE_TICKS;
/// The age at which the hold ends and the arm starts to give the pose back. A
/// caller that wants the pose to stay while a tether holds stops the clock here.
pub const HOLD_END_TICKS: f32 = POINT_AT_TICKS + HOLD_TICKS;
/// Age at which the whole layer is gone.
pub const TOTAL_TICKS: f32 = HOLD_END_TICKS + RECOVER_TICKS;

/// Hand target when loaded, in arm lengths from the shoulder: behind and above
/// the shoulder, a little out. `x` is scaled by the arm's side.
const LOADED: [f32; 3] = [0.30, 0.55, -0.45];
/// Hand target when pointing, in arm lengths from the shoulder: ahead and a
/// little high, almost straight.
const POINTING: [f32; 3] = [0.05, 0.25, 0.95];
/// The elbow bends toward this, `x` scaled by the arm's side: down, out, back.
const ELBOW: [f32; 3] = [0.4, -1.0, -0.3];

/// Where the arm is in a throw: how much of the layer shows and how far the
/// sweep has gone.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ThrowCue {
    /// Share of the layer over the clip's arm, `0.0..=1.0`.
    pub weight: f32,
    /// Sweep from loaded (`0.0`) to pointing (`1.0`).
    pub reach: f32,
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The cue `age` ticks after the spear left the hand (fractions are sub-tick
/// time), or `None` once the layer is gone or `age` is not a finite time at
/// or after the throw.
#[must_use]
pub fn throw_cue(age: f32) -> Option<ThrowCue> {
    if !(0.0..TOTAL_TICKS).contains(&age) {
        return None;
    }
    let sweep = age - WIND_TICKS;
    let recover = age - HOLD_END_TICKS;
    Some(ThrowCue {
        weight: if age < WIND_TICKS {
            smooth(age / WIND_TICKS)
        } else {
            1.0 - smooth(recover / RECOVER_TICKS)
        },
        reach: smooth(sweep / RELEASE_TICKS),
    })
}

/// Lays the throw over `pose`: the arm of `arm` (shoulder, elbow, wrist) is
/// solved to the cue's hand target and blended over its current rotations by
/// the cue's weight, the shortest-arc blend of [`crate::layers::apply_override`].
/// Only those three rotations change; positions, bone lengths and every other
/// joint keep their bits, and weight zero changes nothing at all. The side of the body comes from the shoulder's
/// own offset from the midline (`x`).
///
/// Errors: everything [`solve_limb`] refuses; the pose is untouched then.
pub fn apply_throw(
    skeleton: &Skeleton,
    pose: &mut LocalPose,
    arm: Limb,
    cue: ThrowCue,
) -> Result<(), IkError> {
    let weight = if cue.weight.is_finite() {
        cue.weight.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if weight == 0.0 {
        return Ok(());
    }
    let [shoulder, elbow, wrist] = limb_positions(skeleton, pose, arm)?;
    let dist = |a: [f32; 3], b: [f32; 3]| {
        let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    };
    let length = dist(shoulder, elbow) + dist(elbow, wrist);
    let out = if shoulder[0] < 0.0 { -1.0 } else { 1.0 };
    let side = |v: [f32; 3]| [v[0] * out, v[1], v[2]];
    let reach = lerp3(side(LOADED), side(POINTING), cue.reach.clamp(0.0, 1.0));
    let target = [
        shoulder[0] + reach[0] * length,
        shoulder[1] + reach[1] * length,
        shoulder[2] + reach[2] * length,
    ];
    let joints = [arm.root, arm.mid, arm.end];
    let before = joints.map(|joint| pose.rotations[joint]);
    solve_limb(skeleton, pose, arm, target, side(ELBOW))?;
    if weight < 1.0 {
        for (joint, base) in joints.into_iter().zip(before) {
            pose.rotations[joint] = nlerp(base, pose.rotations[joint], weight);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::Joint;
    use engine_core::alloc_probe::measure;

    const ARM_LEN: f32 = 0.45;

    /// Pelvis, spine, a head, then the right arm (shoulder, elbow, wrist) hanging
    /// 0.25 m and 0.2 m down from a shoulder 0.2 m to the side, then a left
    /// shoulder that the layer must not touch. Joint 5 is the right shoulder.
    fn rig() -> (Skeleton, LocalPose, Limb) {
        let names = [
            ("pelvis", None),
            ("spine", Some(0)),
            ("head", Some(1)),
            ("arm_upper_l", Some(1)),
            ("arm_upper_r", Some(1)),
            ("arm_lower_r", Some(4)),
            ("palm_r", Some(5)),
        ];
        let skeleton = Skeleton {
            joints: names
                .iter()
                .map(|(name, parent)| Joint {
                    name: (*name).to_string(),
                    parent: *parent,
                })
                .collect(),
        };
        let positions = vec![
            [0.0, 1.0, 0.0],
            [0.0, 0.3, 0.0],
            [0.0, 0.3, 0.0],
            [0.2, 0.25, 0.0],
            [-0.2, 0.25, 0.0],
            [0.0, -0.25, 0.0],
            [0.0, -0.2, 0.0],
        ];
        let rotations = vec![[0.0, 0.0, 0.0, 1.0]; positions.len()];
        let arm = Limb::new(&skeleton, 4, 5, 6).expect("arm chain");
        (
            skeleton,
            LocalPose {
                positions,
                rotations,
            },
            arm,
        )
    }

    fn hand(skeleton: &Skeleton, pose: &LocalPose, arm: Limb) -> [f32; 3] {
        limb_positions(skeleton, pose, arm).expect("limb")[2]
    }

    fn shoulder(skeleton: &Skeleton, pose: &LocalPose, arm: Limb) -> [f32; 3] {
        limb_positions(skeleton, pose, arm).expect("limb")[0]
    }

    #[test]
    fn the_cue_loads_sweeps_holds_and_goes() {
        let at = |age: f32| throw_cue(age).expect("inside the throw");
        assert_eq!(
            at(0.0),
            ThrowCue {
                weight: 0.0,
                reach: 0.0
            }
        );
        assert_eq!(
            at(WIND_TICKS),
            ThrowCue {
                weight: 1.0,
                reach: 0.0
            }
        );
        assert_eq!(
            at(POINT_AT_TICKS),
            ThrowCue {
                weight: 1.0,
                reach: 1.0
            }
        );
        assert_eq!(
            at(HOLD_END_TICKS),
            ThrowCue {
                weight: 1.0,
                reach: 1.0
            }
        );
        let mid = at(WIND_TICKS + RELEASE_TICKS * 0.5);
        assert!(
            mid.reach > 0.4 && mid.reach < 0.6 && mid.weight == 1.0,
            "{mid:?}"
        );
        // The weight never rises after the hold and the reach never falls.
        let mut last = at(0.0);
        for step in 1..(TOTAL_TICKS * 8.0) as u32 - 1 {
            let now = at(step as f32 / 8.0);
            assert!(now.reach >= last.reach, "reach fell at {step}");
            assert!(now.weight >= 0.0 && now.weight <= 1.0);
            last = now;
        }
        assert_eq!(throw_cue(TOTAL_TICKS), None);
        assert_eq!(throw_cue(-0.5), None);
        assert_eq!(throw_cue(f32::NAN), None);
    }

    #[test]
    fn weight_zero_changes_nothing() {
        let (skeleton, base, arm) = rig();
        let mut pose = base.clone();
        apply_throw(
            &skeleton,
            &mut pose,
            arm,
            ThrowCue {
                weight: 0.0,
                reach: 1.0,
            },
        )
        .expect("ok");
        assert_eq!(pose, base);
    }

    #[test]
    fn only_the_three_arm_rotations_move() {
        let (skeleton, base, arm) = rig();
        let mut pose = base.clone();
        apply_throw(
            &skeleton,
            &mut pose,
            arm,
            ThrowCue {
                weight: 1.0,
                reach: 1.0,
            },
        )
        .expect("ok");
        assert_eq!(pose.positions, base.positions, "a layer of rotations only");
        for joint in 0..base.rotations.len() {
            let moved = pose.rotations[joint] != base.rotations[joint];
            assert_eq!(
                moved,
                [arm.root, arm.mid, arm.end].contains(&joint),
                "joint {joint}"
            );
        }
    }

    #[test]
    fn pointing_reaches_ahead_and_loaded_sits_behind_the_shoulder() {
        let (skeleton, base, arm) = rig();
        let shoulder_at = shoulder(&skeleton, &base, arm);
        let mut pointing = base.clone();
        apply_throw(
            &skeleton,
            &mut pointing,
            arm,
            ThrowCue {
                weight: 1.0,
                reach: 1.0,
            },
        )
        .expect("ok");
        let mut loaded = base.clone();
        apply_throw(
            &skeleton,
            &mut loaded,
            arm,
            ThrowCue {
                weight: 1.0,
                reach: 0.0,
            },
        )
        .expect("ok");
        let ahead = hand(&skeleton, &pointing, arm)[2] - shoulder_at[2];
        let behind = hand(&skeleton, &loaded, arm)[2] - shoulder_at[2];
        assert!(ahead > 0.85 * ARM_LEN, "pointing hand is {ahead} m ahead");
        assert!(behind < -0.15 * ARM_LEN, "loaded hand is {behind} m behind");
        // The loaded hand is above the shoulder, the pointing one near its height.
        assert!(hand(&skeleton, &loaded, arm)[1] > shoulder_at[1]);
        // The arm keeps its bone lengths: the shoulder stays where it was.
        let after = shoulder(&skeleton, &pointing, arm);
        assert_eq!(after, shoulder_at);
    }

    #[test]
    fn the_arm_loads_out_away_from_the_midline_on_either_side() {
        let (skeleton, base, arm) = rig();
        for sign in [-1.0f32, 1.0] {
            let mut seated = base.clone();
            seated.positions[4][0] = 0.2 * sign;
            let mut pose = seated.clone();
            apply_throw(
                &skeleton,
                &mut pose,
                arm,
                ThrowCue {
                    weight: 1.0,
                    reach: 0.0,
                },
            )
            .expect("ok");
            let side = hand(&skeleton, &pose, arm)[0] - shoulder(&skeleton, &pose, arm)[0];
            assert!(
                side * sign > 0.0,
                "shoulder side {sign}: hand is {side} m across"
            );
        }
    }

    #[test]
    fn half_weight_lands_between_the_clip_and_the_layer() {
        let (skeleton, base, arm) = rig();
        let run = |weight: f32| {
            let mut pose = base.clone();
            apply_throw(&skeleton, &mut pose, arm, ThrowCue { weight, reach: 1.0 }).expect("ok");
            hand(&skeleton, &pose, arm)[2]
        };
        let (clip, half, full) = (run(0.0), run(0.5), run(1.0));
        assert!(clip < half && half < full, "{clip} {half} {full}");
        // Rotations stay unit through the blend.
        let mut pose = base;
        apply_throw(
            &skeleton,
            &mut pose,
            arm,
            ThrowCue {
                weight: 0.5,
                reach: 1.0,
            },
        )
        .expect("ok");
        for q in &pose.rotations {
            let n = q.iter().map(|c| c * c).sum::<f32>();
            assert!((n - 1.0).abs() < 1.0e-5, "{q:?}");
        }
    }

    #[test]
    fn a_refused_solve_leaves_the_pose_alone() {
        let (skeleton, base, arm) = rig();
        let mut pose = base.clone();
        pose.positions[5] = [0.0; 3];
        let before = pose.clone();
        let result = apply_throw(
            &skeleton,
            &mut pose,
            arm,
            ThrowCue {
                weight: 1.0,
                reach: 1.0,
            },
        );
        assert!(result.is_err());
        assert_eq!(pose, before);
    }

    #[test]
    fn the_same_cue_gives_the_same_bits_and_allocates_nothing() {
        let (skeleton, base, arm) = rig();
        let cue = throw_cue(5.5).expect("inside");
        let (mut a, mut b) = (base.clone(), base);
        apply_throw(&skeleton, &mut a, arm, cue).expect("ok");
        let (result, allocs) = measure(|| apply_throw(&skeleton, &mut b, arm, cue));
        result.expect("ok");
        assert_eq!(a, b);
        assert_eq!(
            allocs.calls, 0,
            "the throw layer allocates nothing per call"
        );
    }
}
