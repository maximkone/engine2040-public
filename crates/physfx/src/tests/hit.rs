//! hit: a blow spreads over the bones near it and adds exactly its impulse (QR11); the joints
//! then brace and fade back to the resting tone in 0.3 s (QR10).

use super::hinge::{flexion, hinged, long_axis};
use super::*;

/// The humanoid ragdoll in slot 0, 3 m up, at rest.
fn hung() -> PhysFx {
    let (rig, pose) = humanoid();
    let mut fx = arena(Tier::Comfort);
    fx.spawn_ragdoll(0, &rig, &moved(&pose, [0.0, 3.0, 0.0]), [0.0; 3], [0.0; 3])
        .expect("ragdoll");
    fx
}

/// The middle of the chest of `hung`, meters.
const CHEST: [f32; 3] = [0.0, 4.6, 0.0];

#[test]
fn a_blow_spreads_over_the_bones_in_reach_and_adds_exactly_its_impulse() {
    let mut fx = hung();
    let blow = [30.0, 12.0, -20.0];
    let pushed = fx.hit_ragdoll(0, CHEST, 0.5, blow).expect("hit");
    let sum = fx.ragdoll_momentum(0);
    println!("blow {blow:?} Ns spread over {pushed} bones, momentum {sum:?}");
    assert!(pushed >= 5, "the chest, spine, head and arms are in reach");
    for k in 0..3 {
        assert!(
            (sum[k] - blow[k]).abs() < 1.0e-4,
            "axis {k}: {} against {}",
            sum[k],
            blow[k]
        );
    }
}

#[test]
fn a_blow_outside_every_bone_reach_goes_to_the_nearest_bone() {
    let mut fx = hung();
    let blow = [0.0, 0.0, 15.0];
    let pushed = fx.hit_ragdoll(0, [1.0, 4.6, 0.0], 0.1, blow).expect("hit");
    assert_eq!(pushed, 1, "one bone takes it all");
    let sum = fx.ragdoll_momentum(0);
    assert!((sum[2] - blow[2]).abs() < 1.0e-4, "{sum:?}");
}

#[test]
fn a_bad_blow_is_refused_and_changes_nothing() {
    let mut fx = hung();
    let hand_at = [0.25, 3.0 + 1.22, 0.0];
    assert_eq!(
        fx.hit_ragdoll(0, hand_at, 0.02, [5000.0, 0.0, 0.0]),
        Err(Error::OutOfRange(Field::Impulse)),
        "5000 Ns on a forearm is a bug upstream, not a blow"
    );
    assert_eq!(
        fx.hit_ragdoll(0, [f32::NAN, 0.0, 0.0], 0.5, [1.0; 3]),
        Err(Error::NonFinite(Field::Position))
    );
    assert_eq!(
        fx.hit_ragdoll(0, CHEST, 0.0, [1.0; 3]),
        Err(Error::OutOfRange(Field::Extent))
    );
    assert_eq!(fx.ragdoll_momentum(0), [0.0; 3]);
    assert_eq!(
        fx.ragdoll_strength(0),
        Some(RAGDOLL_TONE),
        "no hit, no brace: the resting tone"
    );
    assert_eq!(
        fx.hit_ragdoll(3, CHEST, 0.5, [1.0; 3]),
        Ok(0),
        "no ragdoll in slot 3"
    );
    assert_eq!(fx.ragdoll_strength(3), None);
}

#[test]
fn strength_goes_from_one_to_the_tone_over_three_tenths_of_a_second() {
    let mut fx = hung();
    assert_eq!(
        fx.ragdoll_strength(0),
        Some(RAGDOLL_TONE),
        "nobody hit it: it holds itself at the tone"
    );
    fx.hit_ragdoll(0, CHEST, 0.5, [10.0, 0.0, 0.0])
        .expect("hit");
    assert_eq!(fx.ragdoll_strength(0), Some(1.0));
    let ticks = 18;
    assert!(
        (HIT_FADE_S / FIXED_DT - 18.0).abs() < 1.0e-3,
        "0.3 s is 18 ticks at 60 Hz"
    );
    let mut curve = Vec::new();
    for _ in 0..ticks {
        fx.step();
        curve.push(fx.ragdoll_strength(0).expect("ragdoll"));
    }
    println!("strength every tick: {curve:.2?}");
    assert!(
        (curve[ticks / 2 - 1] - 0.5).abs() < 0.01,
        "half way at 0.15 s"
    );
    assert!(
        (curve[ticks - 1] - RAGDOLL_TONE).abs() < 1.0e-5,
        "back at the tone by 0.3 s"
    );
    assert!(curve.windows(2).all(|w| w[1] <= w[0]), "it only falls");
    assert!(curve.iter().all(|s| *s >= RAGDOLL_TONE), "never under the tone");
    step_n(&mut fx, 10);
    assert_eq!(
        fx.ragdoll_strength(0),
        Some(RAGDOLL_TONE),
        "and stays at the tone, never a sack"
    );
}

/// How far the head has moved against the chest, meters, at its most over `ticks` steps after
/// `hit` lands on the ragdoll `hung` makes. The chest is heavy, so a shove on the head moves it
/// against the chest until the swing cone stops it (limp) or the motors hold it (braced).
fn head_drift(hit: impl FnOnce(&mut PhysFx), ticks: usize) -> f32 {
    let gap = |fx: &PhysFx| {
        let bones = bones_of(fx, 0);
        let (head, chest) = (bones[bone("head")].at.pos, bones[bone("chest")].at.pos);
        [head[0] - chest[0], head[1] - chest[1], head[2] - chest[2]]
    };
    let mut fx = hung();
    let rest = gap(&fx);
    hit(&mut fx);
    (0..ticks)
        .map(|_| {
            fx.step();
            let now = gap(&fx);
            (0..3)
                .map(|k| (now[k] - rest[k]).powi(2))
                .sum::<f32>()
                .sqrt()
        })
        .fold(0.0, f32::max)
}

#[test]
fn a_braced_neck_gives_way_less_than_a_limp_one() {
    let head = [0.0, 3.0 + 1.98, 0.0];
    let shove = Impact {
        at_mm: [0, 4980, 0],
        direction: [1, 0, 0],
        impulse_ns: 15,
        radius_mm: 100,
    };
    for ticks in [5, 10] {
        let limp = head_drift(
            |fx| {
                fx.apply_impact(shove).expect("impact");
            },
            ticks,
        );
        let braced = head_drift(
            |fx| {
                fx.hit_ragdoll(0, head, 0.1, [15.0, 0.0, 0.0]).expect("hit");
            },
            ticks,
        );
        println!("head drift after {ticks} ticks: limp {limp:.3} m, braced {braced:.3} m");
        assert!(limp > 0.04, "the shove moves a limp head: {limp}");
        assert!(
            braced < 0.6 * limp,
            "a braced neck holds: {braced} against {limp}"
        );
    }
}

/// How far a ragdoll with `muscle` permille folds away from its spawn shape by falling 1 m and
/// lying for a second with nobody hitting it: the summed change of every bone's angle to its
/// parent, radians.
fn fold_after_a_fall(muscle: u16) -> f32 {
    let (rig, pose) = humanoid();
    let rig = rig.with_muscle_permille(muscle).expect("muscle");
    let mut fx = arena(Tier::Comfort);
    fx.spawn_ragdoll(0, &rig, &moved(&pose, [0.0, 1.0, 0.0]), [0.0; 3], [0.0; 3])
        .expect("ragdoll");
    let bends = |fx: &PhysFx| {
        let bones = bones_of(fx, 0);
        BONES
            .iter()
            .enumerate()
            .filter_map(|(at, b)| b.1.map(|parent| (at, parent)))
            .map(|(at, parent)| {
                let (a, p) = (long_axis(&bones[at]), long_axis(&bones[parent]));
                (a[0] * p[0] + a[1] * p[1] + a[2] * p[2]).clamp(-1.0, 1.0).acos()
            })
            .collect::<Vec<f32>>()
    };
    let spawn = bends(&fx);
    step_n(&mut fx, 90);
    bends(&fx)
        .iter()
        .zip(&spawn)
        .map(|(now, was)| (now - was).abs())
        .sum()
}

#[test]
fn a_toned_body_keeps_its_shape_where_a_limp_one_folds() {
    // The owner's "floppy af": a ragdoll nobody hit used to have no motor at all.
    let limp = fold_after_a_fall(0);
    let toned = fold_after_a_fall(900);
    println!("fold after a 1 m fall: limp {limp:.3} rad, toned {toned:.3} rad");
    assert!(limp > 0.3, "a sack folds: {limp}");
    assert!(
        toned < 0.6 * limp,
        "the tone holds the body's shape: {toned} against {limp}"
    );
}

#[test]
fn a_held_brace_stays_up_while_asked_and_fades_to_the_tone_after() {
    let mut fx = hung();
    let mut held = f32::MAX;
    for _ in 0..30 {
        assert!(fx.brace_ragdoll(0, 0.6).is_some());
        fx.step();
        held = held.min(fx.ragdoll_strength(0).expect("ragdoll"));
    }
    assert!(held > 0.5, "the pin weight holds while asked: {held}");
    step_n(&mut fx, 20);
    assert_eq!(fx.ragdoll_strength(0), Some(RAGDOLL_TONE), "then the tone");
    assert_eq!(fx.brace_ragdoll(3, 0.6), None, "no ragdoll in slot 3");
    assert_eq!(
        fx.brace_ragdoll(0, f32::NAN),
        Some(RAGDOLL_TONE),
        "a bad weight braces nothing"
    );
    // A body with no muscle cannot be braced at all.
    let (rig, pose) = humanoid();
    let limp = rig.with_muscle_permille(0).expect("muscle");
    let mut sack = arena(Tier::Comfort);
    sack.spawn_ragdoll(0, &limp, &moved(&pose, [0.0, 3.0, 0.0]), [0.0; 3], [0.0; 3])
        .expect("ragdoll");
    assert_eq!(sack.brace_ragdoll(0, 0.6), Some(0.0));
}

#[test]
fn a_braced_ball_joint_holds_the_pose_it_spawned_in() {
    // The left upper arm is raised 20 degrees. A motor aimed at zero angle would pull it back to
    // the body's side; aimed at the angles it spawned with, it stays.
    let (rig, mut pose) = humanoid();
    let arm = bone("upper_arm_l");
    let half = 10.0_f32.to_radians();
    pose.rotations[arm] = [0.0, 0.0, half.sin(), half.cos()];
    let mut fx = arena(Tier::Comfort);
    fx.spawn_ragdoll(0, &rig, &moved(&pose, [0.0, 3.0, 0.0]), [0.0; 3], [0.0; 3])
        .expect("ragdoll");
    let lift = |fx: &PhysFx| {
        let bones = bones_of(fx, 0);
        let (c, a) = (long_axis(&bones[bone("chest")]), long_axis(&bones[arm]));
        (c[0] * a[0] + c[1] * a[1] + c[2] * a[2])
            .clamp(-1.0, 1.0)
            .acos()
    };
    let start = lift(&fx);
    assert!(
        (start - 0.35).abs() < 0.02,
        "20 degrees at the spawn: {start}"
    );
    fx.hit_ragdoll(0, [0.0, 4.1, 0.0], 0.1, [1.0, 0.0, 0.0])
        .expect("hit");
    step_n(&mut fx, 12);
    let now = lift(&fx);
    println!("upper arm lift: {start:.3} rad at the spawn, {now:.3} rad braced");
    assert!(
        (now - start).abs() < 0.08,
        "the brace holds the pose: {now}"
    );
}

#[test]
fn a_hit_hinged_ragdoll_is_not_thrown_wild() {
    let (rig, pose) = hinged();
    let mut fx = arena(Tier::Comfort);
    fx.spawn_ragdoll(0, &rig, &moved(&pose, [0.0, 3.0, 0.0]), [0.0; 3], [0.0; 3])
        .expect("ragdoll");
    fx.hit_ragdoll(0, CHEST, 0.5, [60.0, 20.0, -40.0])
        .expect("hit");
    step_n(&mut fx, 60);
    assert!(
        fx.ragdoll_speed(0).is_some_and(|v| v < 30.0),
        "the body is not thrown wild"
    );
    let bones = bones_of(&fx, 0);
    assert!(bones.iter().all(|b| b.at.pos.iter().all(|c| c.is_finite())));
    assert!(flexion(&bones, bone("thigh_l"), bone("shin_l")).abs() <= 2.6);
}
