//! follow: the pose drive tracks the clip instead of lagging a tick (Jolt's two-pose motor
//! drive), writes nothing at weight 0, and never chases a teleport.

use super::*;
use rapier3d::math::{Rotation, Vector};

/// The humanoid ragdoll in slot 0, standing 3.2 m up, at rest.
fn standing() -> PhysFx {
    let (rig, pose) = humanoid();
    let mut fx = arena(Tier::Comfort);
    fx.spawn_ragdoll(0, &rig, &moved(&pose, [0.0, 3.2, 0.0]), [0.0; 3], [0.0; 3])
        .expect("ragdoll");
    fx
}

/// The bones of slot 0 as one pose slice, in skeleton order: what a caller hands the drive.
fn clip(fx: &PhysFx) -> Vec<Transform> {
    let mut out = vec![Transform::IDENTITY; BONES.len()];
    for placed in bones_of(fx, 0) {
        if let Key::Bone { bone, .. } = placed.key {
            out[usize::from(bone)] = placed.at;
        }
    }
    out
}

/// The point to shove: the chest bone of the clip, meters.
fn chest_of(pose: &[Transform]) -> [f32; 3] {
    pose[bone("chest")].pos
}

/// `rest` with the left arm chain swung by `angle` radians about the shoulder: the clip pose one
/// tick ahead. Positions and rotations both move, as a real clip moves them.
fn arm_clip(rest: &[Transform], angle: f32) -> Vec<Transform> {
    let mut out = rest.to_vec();
    let shoulder = bone("upper_arm_l");
    let pivot = rest[shoulder].pos;
    let turn = Rotation::from_rotation_x(angle);
    for name in ["upper_arm_l", "lower_arm_l"] {
        let at = bone(name);
        let p = rest[at].pos;
        let arm = Vector::new(p[0] - pivot[0], p[1] - pivot[1], p[2] - pivot[2]);
        let swung = turn * arm + Vector::new(pivot[0], pivot[1], pivot[2]);
        out[at].pos = [swung.x, swung.y, swung.z];
        let was = rest[at].rot;
        let r = turn * Rotation::from_xyzw(was[0], was[1], was[2], was[3]);
        out[at].rot = [r.x, r.y, r.z, r.w];
    }
    out
}

/// How far the lower left arm ends up from where the clip puts it, meters.
fn arm_error(fx: &PhysFx, want: &[Transform]) -> f32 {
    let now = clip(fx);
    let at = bone("lower_arm_l");
    let (a, b) = (now[at].pos, want[at].pos);
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

#[test]
fn follow_drive_tracks_the_clip_without_lagging_a_tick() {
    let mut lagging = standing();
    let mut tracking = standing();
    let rest = clip(&tracking);
    let want = arm_clip(&rest, 25.0_f32.to_radians());
    let chest = chest_of(&rest);
    for fx in [&mut lagging, &mut tracking] {
        fx.hit_ragdoll(0, chest, 0.5, [25.0, 0.0, 0.0])
            .expect("shove");
    }
    lagging
        .drive_pose_position_only(0, &rest, &want, FIXED_DT, 1.0)
        .expect("position drive");
    let driven = tracking
        .drive_pose(0, &rest, &want, FIXED_DT, 1.0)
        .expect("pose drive");
    assert!(driven > 0, "the arm chain is driven");
    lagging.step();
    tracking.step();
    let without = arm_error(&lagging, &want);
    let with = arm_error(&tracking, &want);
    println!(
        "lower_arm_l to the clip one tick on: position only {without:.4} m, with the pose speed \
         {with:.4} m ({driven} joints driven)"
    );
    assert!(
        with < without,
        "the pose speed must cut the lag: {with} m against {without} m"
    );
}

#[test]
fn follow_weight_zero_keeps_todays_bytes() {
    let mut plain = standing();
    let mut zero = standing();
    let rest = clip(&plain);
    let want = arm_clip(&rest, 20.0_f32.to_radians());
    let chest = chest_of(&rest);
    let pushed = plain
        .hit_ragdoll(0, chest, 0.5, [25.0, 0.0, 0.0])
        .expect("shove");
    assert_eq!(
        zero.hit_ragdoll(0, chest, 0.5, [25.0, 0.0, 0.0]),
        Ok(pushed)
    );
    for _ in 0..20 {
        assert_eq!(
            zero.drive_pose(0, &rest, &want, FIXED_DT, 0.0),
            Ok(0),
            "weight 0 drives nothing"
        );
        plain.step();
        zero.step();
    }
    assert_eq!(clip(&plain), clip(&zero), "weight 0 writes no byte");
    assert_eq!(plain.ragdoll_strength(0), zero.ragdoll_strength(0));
    // A body with no muscle cannot be driven at any weight.
    let (rig, pose) = humanoid();
    let limp = rig.with_muscle_permille(0).expect("muscle");
    let mut sack = arena(Tier::Comfort);
    sack.spawn_ragdoll(0, &limp, &moved(&pose, [0.0, 3.2, 0.0]), [0.0; 3], [0.0; 3])
        .expect("ragdoll");
    let still = clip(&sack);
    assert_eq!(
        sack.drive_pose(0, &still, &still, FIXED_DT, 1.0),
        Ok(0),
        "a limp body gets no drive"
    );
    assert_eq!(clip(&sack), still);
}

#[test]
fn follow_refuses_to_chase_a_teleport() {
    let mut fx = standing();
    let rest = clip(&fx);
    let (_, wide_before) = fx.follow_drive_state();
    // The clip flips the arm half a turn between two ticks: 188 rad/s of joint speed at 60 Hz.
    let jumped = arm_clip(&rest, std::f32::consts::PI);
    assert!(
        fx.drive_pose(0, &rest, &jumped, FIXED_DT, 1.0)
            .expect("drive")
            > 0
    );
    let (fastest, wide_after) = fx.follow_drive_state();
    println!(
        "a half-turn pose jump asks {fastest:.2} rad/s, the cap {MAX_DRIVE_OMEGA}; the target \
         angle walked {:.3} rad, the cap {MAX_DRIVE_ANGLE}",
        wide_after - wide_before
    );
    assert!(
        fastest <= MAX_DRIVE_OMEGA + 1.0e-4,
        "the target speed must not pass the cap: {fastest}"
    );
    assert!(
        fastest >= MAX_DRIVE_OMEGA - 1.0e-4,
        "the teleport must reach the cap, or it was not a teleport: {fastest}"
    );
    assert!(
        wide_after <= wide_before + MAX_DRIVE_ANGLE + 1.0e-4,
        "the target angle jumped {wide_after} from {wide_before}"
    );
    fx.step();
    // A capped drive cannot follow half a turn in one tick.
    let turned = {
        let now = clip(&fx);
        let at = bone("lower_arm_l");
        let (a, b) = (now[at].rot, rest[at].rot);
        let dot: f32 = (0..4).map(|i| a[i] * b[i]).sum();
        2.0 * dot.abs().clamp(-1.0, 1.0).acos()
    };
    println!("the lower arm turned {turned:.3} rad in one tick");
    assert!(
        turned < std::f32::consts::PI,
        "the body followed the teleport: {turned} rad in one tick"
    );
}

#[test]
fn follow_drive_repeats_and_refuses_bad_input() {
    let run = || {
        let mut fx = standing();
        let rest = clip(&fx);
        let want = arm_clip(&rest, 15.0_f32.to_radians());
        let chest = chest_of(&rest);
        fx.hit_ragdoll(0, chest, 0.5, [20.0, 0.0, 0.0])
            .expect("shove");
        for _ in 0..20 {
            fx.drive_pose(0, &rest, &want, FIXED_DT, 0.8)
                .expect("drive");
            fx.step();
        }
        clip(&fx)
    };
    assert_eq!(run(), run(), "same pose, same bytes");
    let mut fx = standing();
    let rest = clip(&fx);
    assert_eq!(
        fx.drive_pose(0, &rest, &rest, 0.0, 1.0),
        Err(Error::OutOfRange(Field::Ticks))
    );
    assert_eq!(
        fx.drive_pose(0, &rest, &rest, f32::NAN, 1.0),
        Err(Error::NonFinite(Field::Ticks))
    );
    assert_eq!(
        fx.drive_pose(0, &rest, &rest, FIXED_DT, f32::NAN),
        Err(Error::NonFinite(Field::Muscle))
    );
    let mut bad = rest.clone();
    bad[bone("chest")].rot = [f32::NAN; 4];
    assert_eq!(
        fx.drive_pose(0, &rest, &bad, FIXED_DT, 1.0),
        Err(Error::NonFinite(Field::Rotation))
    );
    assert_eq!(
        fx.drive_pose(0, &rest[1..], &rest, FIXED_DT, 1.0),
        Err(Error::PoseLength {
            expected: BONES.len(),
            got: BONES.len() - 1
        })
    );
    assert_eq!(
        fx.drive_pose(3, &rest, &rest, FIXED_DT, 1.0),
        Ok(0),
        "no ragdoll in slot 3"
    );
    assert_eq!(clip(&fx), rest, "a refused drive writes nothing");
}
