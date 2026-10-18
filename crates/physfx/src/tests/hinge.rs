//! hinge: knees and elbows are hinges. A knee folds far past the old swing cone and never bends
//! the wrong way (QR3).

use super::*;

/// Knee flexion range, radians, counted from the straight leg: 0 to about 137 degrees.
const KNEE: [f32; 2] = [0.0, 2.4];
/// Elbow flexion range: the forearm folds forward, which is negative about the side axis.
const ELBOW: [f32; 2] = [-2.5, 0.0];

/// The humanoid with a hinge at both knees (the shins) and both elbows (the lower arms).
pub(super) fn hinged() -> (Rig, LocalPose) {
    let (rig, pose) = humanoid();
    let hinge = |name: &str, range: [f32; 2]| Hinge {
        bone: bone(name),
        min: range[0],
        max: range[1],
    };
    let hinges = [
        hinge("shin_l", KNEE),
        hinge("shin_r", KNEE),
        hinge("lower_arm_l", ELBOW),
        hinge("lower_arm_r", ELBOW),
    ];
    (rig.with_hinges(&hinges).expect("hinges"), pose)
}

/// The long axis of a placed capsule.
pub(super) fn long_axis(placed: &Placed) -> [f32; 3] {
    let [x, y, z, w] = placed.at.rot;
    [
        2.0 * (x * y - w * z),
        1.0 - 2.0 * (x * x + z * z),
        2.0 * (y * z + w * x),
    ]
}

/// How far `child` is folded against `parent` about the side axis (X), radians. Positive folds
/// the child backward, the way a knee bends.
pub(super) fn flexion(bones: &[Placed], parent: usize, child: usize) -> f32 {
    let (p, c) = (long_axis(&bones[parent]), long_axis(&bones[child]));
    let side = p[1] * c[2] - p[2] * c[1];
    side.atan2(p[0] * c[0] + p[1] * c[1] + p[2] * c[2])
}

/// The left shin folded back by `angle` radians. The shin turns about its own center and the
/// center moves with it, as a bent leg's does.
fn folded_knee(angle: f32) -> LocalPose {
    let (_, mut pose) = humanoid();
    let shin = bone("shin_l");
    let (sin, cos) = angle.sin_cos();
    pose.positions[shin] = [0.0, -0.2 - 0.2 * cos, -0.2 * sin];
    pose.rotations[shin] = [(angle / 2.0).sin(), 0.0, 0.0, (angle / 2.0).cos()];
    pose
}

/// Spawns the hinged humanoid 3 m up, so only its own joints act, lands `hit` and runs `ticks`
/// steps. Returns the flexion of `child` against `parent` after every step.
fn run(
    pose: &LocalPose,
    hit: impl FnOnce(&mut PhysFx),
    parent: &str,
    child: &str,
    ticks: usize,
) -> Vec<f32> {
    let (rig, _) = hinged();
    let mut fx = arena(Tier::Comfort);
    fx.spawn_ragdoll(0, &rig, &moved(pose, [0.0, 3.0, 0.0]), [0.0; 3], [0.0; 3])
        .expect("ragdoll");
    hit(&mut fx);
    (0..ticks)
        .map(|_| {
            fx.step();
            flexion(&bones_of(&fx, 0), bone(parent), bone(child))
        })
        .collect()
}

/// A shove of `ns` newton-seconds along Z at the center of the shin or the lower arm of the ragdoll
/// `run` spawns, 3 m up (the standing humanoid's offsets added up). Only that bone is within 10 cm
/// of the point.
fn shove(bone_name: &str, forward: bool, ns: u32) -> Impact {
    let at_mm = match bone_name {
        "shin_l" => [-100, 3450, 0],
        "lower_arm_l" => [-250, 4220, 0],
        other => panic!("no shove point for {other}"),
    };
    Impact {
        at_mm,
        direction: [0, 0, if forward { 1 } else { -1 }],
        impulse_ns: ns,
        radius_mm: 100,
    }
}

#[test]
fn a_folded_knee_stays_folded() {
    let angle = 1.5;
    let flex = run(&folded_knee(angle), |_| {}, "thigh_l", "shin_l", 30);
    let (lo, hi) = flex
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), f| (lo.min(*f), hi.max(*f)));
    println!("knee folded at {angle} rad spawn: flexion {lo:.2}..{hi:.2} over 30 ticks");
    assert!(lo > 1.3 && hi < 1.7, "the fold holds: {lo}..{hi}");
}

/// The flexion curve of the left knee or elbow after a shove, 40 ticks.
fn shoved(joint: (&str, &str), forward: bool, ns: u32) -> Vec<f32> {
    let (_, pose) = hinged();
    let hit = shove(joint.1, forward, ns);
    run(
        &pose,
        |fx| {
            fx.apply_impact(hit).expect("impact");
        },
        joint.0,
        joint.1,
        40,
    )
}

fn peak(curve: &[f32]) -> f32 {
    curve.iter().copied().fold(f32::MIN, f32::max)
}

fn trough(curve: &[f32]) -> f32 {
    curve.iter().copied().fold(f32::MAX, f32::min)
}

#[test]
fn a_knee_folds_past_the_old_cone_one_way_only() {
    let knee = ("thigh_l", "shin_l");
    let top = peak(&shoved(knee, false, 40));
    let bottom = trough(&shoved(knee, true, 20));
    println!("knee: folds to {top:.2} rad, hyperextends to {bottom:.2} rad");
    assert!(top > 0.9, "a swept-back shin folds past 0.6 rad: {top}");
    assert!(top < KNEE[1] + 0.3, "and stops at the limit: {top}");
    assert!(bottom > -0.12, "a knee does not bend forward: {bottom}");
}

#[test]
fn an_elbow_folds_forward_only() {
    let elbow = ("upper_arm_l", "lower_arm_l");
    let bottom = trough(&shoved(elbow, true, 8));
    let top = peak(&shoved(elbow, false, 4));
    println!("elbow: folds to {bottom:.2} rad, hyperextends to {top:.2} rad");
    assert!(bottom < -0.9, "a swept-forward forearm folds: {bottom}");
    assert!(bottom > ELBOW[0] - 0.3, "and stops at the limit: {bottom}");
    assert!(top < 0.2, "an elbow does not bend backward: {top}");
}

#[test]
fn a_bad_hinge_is_refused() {
    let (rig, _) = humanoid();
    let knee = |bone: usize, min: f32, max: f32| [Hinge { bone, min, max }];
    let shin = bone("shin_l");
    assert!(rig.clone().with_hinges(&knee(shin, 0.0, 2.4)).is_ok());
    assert_eq!(
        rig.clone().with_hinges(&knee(0, 0.0, 2.4)),
        Err(Error::OutOfRange(Field::Bone)),
        "the root has no joint"
    );
    assert_eq!(
        rig.clone().with_hinges(&knee(BONES.len(), 0.0, 2.4)),
        Err(Error::OutOfRange(Field::Bone))
    );
    assert_eq!(
        rig.clone().with_hinges(&knee(shin, 2.4, 0.0)),
        Err(Error::OutOfRange(Field::Angle)),
        "reversed"
    );
    assert_eq!(
        rig.clone().with_hinges(&knee(shin, 0.0, f32::NAN)),
        Err(Error::NonFinite(Field::Angle))
    );
}
