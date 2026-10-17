//! tests: caps, recycling, determinism, refusals, sleeping, interpolation and the budget.

mod blast;
mod impact;
mod ladder;
mod materials;

use super::*;
use engine_anim::contract::{AnimError, Joint, LocalPose, Skeleton};
use engine_physics::rapier::FIXED_DT;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

/// One bone of the test humanoid: name, parent, local offset (meters), capsule (radius, half-height).
type BoneRow = (&'static str, Option<usize>, [f32; 3], (f32, f32));

/// Standing, pelvis at 1.1 m, arms hanging; no two unjoined capsules touch.
const BONES: [BoneRow; 14] = [
    ("pelvis", None, [0.0, 1.1, 0.0], (0.14, 0.05)),
    ("spine", Some(0), [0.0, 0.25, 0.0], (0.13, 0.05)),
    ("chest", Some(1), [0.0, 0.25, 0.0], (0.15, 0.08)),
    ("head", Some(2), [0.0, 0.38, 0.0], (0.11, 0.04)),
    ("upper_arm_l", Some(2), [-0.25, -0.1, 0.0], (0.05, 0.10)),
    ("lower_arm_l", Some(4), [0.0, -0.28, 0.0], (0.04, 0.10)),
    ("upper_arm_r", Some(2), [0.25, -0.1, 0.0], (0.05, 0.10)),
    ("lower_arm_r", Some(6), [0.0, -0.28, 0.0], (0.04, 0.10)),
    ("thigh_l", Some(0), [-0.1, -0.25, 0.0], (0.08, 0.15)),
    ("shin_l", Some(8), [0.0, -0.4, 0.0], (0.06, 0.15)),
    ("foot_l", Some(9), [0.0, -0.3, 0.0], (0.06, 0.03)),
    ("thigh_r", Some(0), [0.1, -0.25, 0.0], (0.08, 0.15)),
    ("shin_r", Some(11), [0.0, -0.4, 0.0], (0.06, 0.15)),
    ("foot_r", Some(12), [0.0, -0.3, 0.0], (0.06, 0.03)),
];

fn humanoid() -> (Rig, LocalPose) {
    let skeleton = Skeleton {
        joints: BONES
            .iter()
            .map(|(name, parent, _, _)| Joint {
                name: (*name).into(),
                parent: *parent,
            })
            .collect(),
    };
    let shapes = BONES
        .iter()
        .map(|(_, _, _, (radius, half_height))| BoneShape {
            radius: *radius,
            half_height: *half_height,
        })
        .collect();
    let pose = LocalPose {
        positions: BONES.iter().map(|b| b.2).collect(),
        rotations: vec![Transform::IDENTITY.rot; BONES.len()],
    };
    (Rig::new(skeleton, shapes).expect("humanoid rig"), pose)
}

/// The humanoid's pose moved by `by` meters.
fn moved(pose: &LocalPose, by: [f32; 3]) -> LocalPose {
    let mut out = pose.clone();
    for (coord, add) in out.positions[0].iter_mut().zip(by) {
        *coord += add;
    }
    out
}

fn limb_shape() -> BoneShape {
    BoneShape {
        radius: 0.06,
        half_height: 0.15,
    }
}

fn at(pos: [f32; 3]) -> Transform {
    Transform {
        pos,
        ..Transform::IDENTITY
    }
}

/// Caps nothing in a test reaches unless the test lowers one.
fn generous() -> Caps {
    Caps {
        bodies: 4096,
        ragdolls: 64,
        contacts: 100_000,
        statics: 16,
        solver_iterations: 4,
        ccd_substeps: 1,
        blast_bodies: 4096,
        impact_bodies: 4096,
    }
}

/// A world on a floor whose top is at y = 0.
fn on_floor(mut fx: PhysFx) -> PhysFx {
    fx.add_static_box([50.0, 0.5, 50.0], [0.0, -0.5, 0.0])
        .expect("floor");
    fx
}

fn arena(tier: Tier) -> PhysFx {
    on_floor(PhysFx::new(tier))
}

/// `n` boxes of 20 cm on a half-meter grid, `y` meters up, none touching.
fn debris_field(n: usize, y: f32) -> Vec<Shard> {
    (0..n)
        .map(|i| Shard {
            at: at([(i % 20) as f32 * 0.5 - 4.75, y, 4.0 + (i / 20) as f32 * 0.5]),
            half: [0.1; 3],
            velocity: [0.0; 3],
        })
        .collect()
}

fn step_n(fx: &mut PhysFx, n: usize) {
    for _ in 0..n {
        fx.step();
    }
}

fn limb_parts(fx: &PhysFx) -> Vec<u32> {
    fx.transforms(1.0)
        .expect("transforms")
        .iter()
        .filter_map(|p| match p.key {
            Key::Limb { part } => Some(part),
            _ => None,
        })
        .collect()
}

fn bones_of(fx: &PhysFx, slot: u8) -> Vec<Placed> {
    fx.transforms(1.0)
        .expect("transforms")
        .into_iter()
        .filter(|p| matches!(p.key, Key::Bone { slot: s, .. } if s == slot))
        .collect()
}

/// Two ragdolls, a limb and a debris batch over a floor, stepped a little: the busy scene the
/// determinism and refusal tests share.
fn scene() -> PhysFx {
    let (rig, pose) = humanoid();
    let mut fx = arena(Tier::Comfort);
    fx.spawn_ragdoll(
        0,
        &rig,
        &moved(&pose, [-2.0, 0.6, 0.0]),
        [1.5, 0.0, 0.0],
        [0.0, 0.0, 90.0],
    )
    .expect("ragdoll 0");
    fx.spawn_ragdoll(
        1,
        &rig,
        &moved(&pose, [2.0, 0.9, 0.0]),
        [0.0, 0.0, 0.0],
        [60.0, 0.0, 0.0],
    )
    .expect("ragdoll 1");
    fx.spawn_limb(7, limb_shape(), at([0.0, 2.0, -2.0]), [1.0, 2.0, 0.0])
        .expect("limb");
    fx.spawn_debris(&debris_field(30, 1.0), [0.0, 200.0, 40.0])
        .expect("debris");
    step_n(&mut fx, 10);
    fx
}

#[test]
fn every_tier_table_is_valid_and_climbs() {
    let tables: Vec<Caps> = Tier::ALL.iter().map(|t| t.caps()).collect();
    for caps in &tables {
        assert_eq!(caps.validate(), Ok(()));
    }
    for pair in tables.windows(2) {
        let (lo, hi) = (pair[0], pair[1]);
        assert!(lo.bodies < hi.bodies && lo.contacts < hi.contacts && lo.statics < hi.statics);
        assert!(lo.ragdolls <= hi.ragdolls && lo.solver_iterations <= hi.solver_iterations);
        assert!(lo.ccd_substeps <= hi.ccd_substeps);
    }
    // The plan's soak (8 ragdolls, 200 debris, 6 limbs) fits the comfortable tier.
    let comfort = Tier::Comfort.caps();
    assert!(comfort.ragdolls >= 8 && 8 * BONES.len() + 200 + 6 <= comfort.bodies);
}

#[test]
fn with_caps_refuses_a_zero_limit() {
    let ok = Tier::Web.caps();
    assert!(PhysFx::with_caps(ok).is_ok());
    assert!(PhysFx::with_caps(Caps {
        ccd_substeps: 0,
        ..ok
    })
    .is_ok());
    for bad in [
        Caps { bodies: 0, ..ok },
        Caps { ragdolls: 0, ..ok },
        Caps { contacts: 0, ..ok },
        Caps { statics: 0, ..ok },
        Caps {
            solver_iterations: 0,
            ..ok
        },
    ] {
        assert_eq!(PhysFx::with_caps(bad).err(), Some(Error::BadCaps));
    }
}

#[test]
fn body_cap_holds_under_ten_times_overspawn() {
    for tier in Tier::ALL {
        let cap = tier.caps().bodies;
        let mut fx = arena(tier);
        // Limbs on a 40 x 40 grid, 1.2 m apart: the newest `cap` never touch one another, so
        // only the body cap (not contact shedding) can be what holds the count.
        for part in 0..(10 * cap) as u32 {
            let cell = (part % 1600) as f32;
            let pos = [
                (cell % 40.0) * 1.2 - 24.0,
                1.0,
                (cell / 40.0).floor() * 1.2 - 24.0,
            ];
            fx.spawn_limb(part, limb_shape(), at(pos), [0.0; 3])
                .expect("limb");
            assert!(
                fx.stats().bodies <= cap,
                "{tier:?}: {} bodies",
                fx.stats().bodies
            );
            if part % 8 == 0 {
                fx.step();
            }
        }
        let stats = fx.stats();
        assert_eq!(stats.bodies, cap, "{tier:?}: the cap is used in full");
        assert_eq!(
            stats.recycled,
            9 * cap as u64,
            "{tier:?}: all the overspawn was recycled"
        );
        assert_eq!(fx.transforms(0.5).expect("transforms").len(), cap);

        // The same overspawn in one debris call: admitted up to the cap, not past it.
        let mut fx = arena(tier);
        let batch = fx
            .spawn_debris(&debris_field(10 * cap, 1.0), [0.0; 3])
            .expect("debris");
        assert_eq!(batch.admitted, cap, "{tier:?}");
        assert_eq!(fx.stats().bodies, cap, "{tier:?}");
    }
}

#[test]
fn the_oldest_body_is_recycled_first() {
    let mut fx = PhysFx::with_caps(Caps {
        bodies: 4,
        ..generous()
    })
    .expect("caps");
    let drop_at = |part: u32| at([part as f32, 5.0, 0.0]);
    for part in 1..=4 {
        fx.spawn_limb(part, limb_shape(), drop_at(part), [0.0; 3])
            .expect("limb");
    }
    assert_eq!(limb_parts(&fx), [1, 2, 3, 4]);
    fx.spawn_limb(5, limb_shape(), drop_at(5), [0.0; 3])
        .expect("limb");
    assert_eq!(limb_parts(&fx), [2, 3, 4, 5], "limb 1 was the oldest");
    step_n(&mut fx, 3);
    fx.spawn_limb(6, limb_shape(), drop_at(6), [0.0; 3])
        .expect("limb");
    assert_eq!(
        limb_parts(&fx),
        [3, 4, 5, 6],
        "stepping does not reorder the age"
    );
    assert_eq!(fx.stats().recycled, 2);

    // A respawn of a part replaces it: no recycling, and it becomes the newest.
    fx.spawn_limb(4, limb_shape(), drop_at(4), [0.0; 3])
        .expect("limb");
    assert_eq!(limb_parts(&fx), [3, 5, 6, 4]);
    assert_eq!(fx.stats().recycled, 2);
    assert_eq!(fx.stats().bodies, 4);
}

#[test]
fn a_ragdoll_is_recycled_whole_never_in_part() {
    let (rig, pose) = humanoid();
    let bones = rig.bones();
    let mut fx = PhysFx::with_caps(Caps {
        bodies: bones + 2,
        ..generous()
    })
    .expect("caps");
    fx.spawn_ragdoll(0, &rig, &pose, [0.0; 3], [0.0; 3])
        .expect("ragdoll");
    fx.spawn_limb(1, limb_shape(), at([5.0, 3.0, 0.0]), [0.0; 3])
        .expect("limb");
    fx.spawn_limb(2, limb_shape(), at([6.0, 3.0, 0.0]), [0.0; 3])
        .expect("limb");
    assert_eq!(fx.stats().bodies, bones + 2);

    fx.spawn_limb(3, limb_shape(), at([7.0, 3.0, 0.0]), [0.0; 3])
        .expect("limb");
    assert!(bones_of(&fx, 0).is_empty(), "the oldest unit went whole");
    assert_eq!(limb_parts(&fx), [1, 2, 3]);
    assert_eq!(fx.stats().recycled, bones as u64);
}

#[test]
fn the_ragdoll_cap_recycles_the_oldest_ragdoll_and_a_slot_respawn_replaces() {
    let (rig, pose) = humanoid();
    let bones = rig.bones();
    let mut fx = PhysFx::with_caps(Caps {
        ragdolls: 2,
        ..generous()
    })
    .expect("caps");
    for slot in 0..3u8 {
        let at_x = moved(&pose, [3.0 * f32::from(slot), 0.0, 0.0]);
        fx.spawn_ragdoll(slot, &rig, &at_x, [0.0; 3], [0.0; 3])
            .expect("ragdoll");
    }
    assert!(bones_of(&fx, 0).is_empty(), "slot 0 was the oldest ragdoll");
    assert_eq!(bones_of(&fx, 1).len(), bones);
    assert_eq!(bones_of(&fx, 2).len(), bones);
    assert_eq!(fx.stats().recycled, bones as u64);

    // Respawning slot 1 replaces it in place: same body count, nothing recycled by the cap.
    fx.spawn_ragdoll(1, &rig, &pose, [0.0; 3], [0.0; 3])
        .expect("respawn");
    assert_eq!(fx.stats().bodies, 2 * bones);
    assert_eq!(fx.stats().recycled, bones as u64);
}

#[test]
fn a_ragdoll_larger_than_the_cap_is_refused_and_nothing_changes() {
    let (rig, pose) = humanoid();
    let mut fx = PhysFx::with_caps(Caps {
        bodies: 5,
        ..generous()
    })
    .expect("caps");
    fx.spawn_limb(1, limb_shape(), at([0.0, 2.0, 0.0]), [0.0; 3])
        .expect("limb");
    let result = fx.spawn_ragdoll(0, &rig, &pose, [0.0; 3], [0.0; 3]);
    assert_eq!(
        result,
        Err(Error::OverCap {
            need: BONES.len(),
            cap: 5
        })
    );
    assert_eq!(
        limb_parts(&fx),
        [1],
        "the limb was not recycled for a spawn that cannot fit"
    );
    assert_eq!(fx.stats().recycled, 0);
}

#[test]
fn the_same_inputs_give_identical_transforms_twice() {
    let (mut a, mut b) = (scene(), scene());
    let start = a.transforms(1.0).expect("transforms");
    for tick in 0..180 {
        a.step();
        b.step();
        if tick % 30 == 0 {
            assert_eq!(a.transforms(0.37), b.transforms(0.37), "tick {tick}");
        }
    }
    assert_eq!(a.transforms(0.37), b.transforms(0.37));
    assert_eq!(a.transforms(1.0), b.transforms(1.0));
    assert_eq!(a.stats(), b.stats());
    assert_ne!(
        a.transforms(1.0).expect("transforms"),
        start,
        "the scene moved"
    );
}

#[test]
fn a_nan_impulse_is_refused_and_leaves_the_world_unchanged() {
    let (rig, pose) = humanoid();
    let mut touched = scene();
    let mut control = scene();
    let shard = debris_field(1, 3.0)[0];
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let impulse = Error::NonFinite(Field::Impulse);
        let velocity = Error::NonFinite(Field::Velocity);
        assert_eq!(
            touched.spawn_ragdoll(3, &rig, &pose, [0.0; 3], [0.0, bad, 0.0]),
            Err(impulse.clone())
        );
        assert_eq!(
            touched.spawn_debris(&[shard], [bad, 0.0, 0.0]).err(),
            Some(impulse)
        );
        assert_eq!(
            touched.spawn_ragdoll(3, &rig, &pose, [bad, 0.0, 0.0], [0.0; 3]),
            Err(velocity.clone())
        );
        assert_eq!(
            touched.spawn_limb(9, limb_shape(), at([0.0; 3]), [0.0, 0.0, bad]),
            Err(velocity.clone())
        );
        let nan_velocity = Shard {
            velocity: [bad, 0.0, 0.0],
            ..shard
        };
        assert_eq!(
            touched.spawn_debris(&[shard, nan_velocity], [0.0; 3]).err(),
            Some(velocity)
        );
        assert_eq!(
            touched.spawn_limb(9, limb_shape(), at([bad, 0.0, 0.0]), [0.0; 3]),
            Err(Error::NonFinite(Field::Position))
        );
        let nan_turn = Transform {
            rot: [0.0, bad, 0.0, 1.0],
            ..Transform::IDENTITY
        };
        assert_eq!(
            touched.spawn_limb(9, limb_shape(), nan_turn, [0.0; 3]),
            Err(Error::NonFinite(Field::Rotation))
        );
        let nan_shape = BoneShape {
            radius: bad,
            half_height: 0.1,
        };
        assert_eq!(
            touched.spawn_limb(9, nan_shape, at([0.0; 3]), [0.0; 3]),
            Err(Error::NonFinite(Field::Extent))
        );
        // A bad shard last in a list refuses the whole list: nothing of it is spawned.
        let nan_last = Shard {
            at: at([bad, 1.0, 5.0]),
            ..shard
        };
        assert_eq!(
            touched
                .spawn_debris(&[shard, shard, nan_last], [0.0; 3])
                .err(),
            Some(Error::NonFinite(Field::Position))
        );
        assert_eq!(touched.transforms(0.5), control.transforms(0.5));
    }
    let mut bad_pose = pose.clone();
    bad_pose.positions[4][1] = f32::NAN;
    assert_eq!(
        touched.spawn_ragdoll(3, &rig, &bad_pose, [0.0; 3], [0.0; 3]),
        Err(Error::Anim(AnimError::NonFinite("positions")))
    );
    assert_eq!(touched.stats(), control.stats());
    step_n(&mut touched, 90);
    step_n(&mut control, 90);
    assert_eq!(
        touched.transforms(0.5),
        control.transforms(0.5),
        "the refusals changed the future"
    );
    assert_eq!(touched.stats(), control.stats());
}

#[test]
fn a_refusal_at_the_cap_recycles_nothing() {
    let (rig, pose) = humanoid();
    let mut fx = PhysFx::with_caps(Caps {
        bodies: rig.bones() + 1,
        ..generous()
    })
    .expect("caps");
    fx.spawn_limb(1, limb_shape(), at([5.0, 3.0, 0.0]), [0.0; 3])
        .expect("limb");
    fx.spawn_ragdoll(0, &rig, &pose, [0.0; 3], [0.0; 3])
        .expect("ragdoll");
    let full = fx.transforms(1.0);
    assert_eq!(fx.stats().bodies, rig.bones() + 1);

    // Each of these would need the oldest bodies' room; refused, none of it is taken.
    let nan = f32::NAN;
    assert!(fx
        .spawn_ragdoll(1, &rig, &pose, [0.0; 3], [nan, 0.0, 0.0])
        .is_err());
    assert!(fx
        .spawn_limb(2, limb_shape(), at([nan, 0.0, 0.0]), [0.0; 3])
        .is_err());
    assert!(fx
        .spawn_debris(&debris_field(3, 1.0), [0.0, nan, 0.0])
        .is_err());
    assert_eq!(fx.stats().recycled, 0);
    assert_eq!(fx.transforms(1.0), full);
}

#[test]
fn finite_but_absurd_input_is_refused_too() {
    let (rig, pose) = humanoid();
    let mut fx = arena(Tier::Web);
    let tiny_shard = Shard {
        half: [0.01; 3],
        ..debris_field(1, 1.0)[0]
    };
    assert_eq!(
        fx.spawn_limb(1, limb_shape(), at([1.0e6, 0.0, 0.0]), [0.0; 3]),
        Err(Error::OutOfRange(Field::Position))
    );
    assert_eq!(
        fx.spawn_limb(1, limb_shape(), at([0.0; 3]), [500.0, 0.0, 0.0]),
        Err(Error::OutOfRange(Field::Velocity))
    );
    let no_turn = Transform {
        rot: [0.0; 4],
        ..Transform::IDENTITY
    };
    assert_eq!(
        fx.spawn_limb(1, limb_shape(), no_turn, [0.0; 3]),
        Err(Error::OutOfRange(Field::Rotation))
    );
    for radius in [0.0, -0.1, 50.0] {
        let shape = BoneShape {
            radius,
            half_height: 0.1,
        };
        assert_eq!(
            fx.spawn_limb(1, shape, at([0.0; 3]), [0.0; 3]),
            Err(Error::OutOfRange(Field::Extent))
        );
    }
    assert_eq!(
        fx.spawn_ragdoll(0, &rig, &pose, [0.0; 3], [1.0e9, 0.0, 0.0]),
        Err(Error::OutOfRange(Field::Impulse))
    );
    // 5000 N s into a 17 kg pelvis is 290 m/s: a bug upstream, not a blast.
    assert_eq!(
        fx.spawn_ragdoll(0, &rig, &pose, [0.0; 3], [5_000.0, 0.0, 0.0]),
        Err(Error::OutOfRange(Field::Impulse))
    );
    // 5000 N s into one 0.016 kg shard is the same bug.
    assert_eq!(
        fx.spawn_debris(&[tiny_shard], [5_000.0, 0.0, 0.0]).err(),
        Some(Error::OutOfRange(Field::Impulse))
    );
    assert_eq!(fx.spawn_debris(&[], [0.0; 3]).err(), Some(Error::NoShards));
    let mut skewed = pose.clone();
    skewed.rotations[2] = [0.0, 0.0, 0.0, 2.0];
    assert_eq!(
        fx.spawn_ragdoll(0, &rig, &skewed, [0.0; 3], [0.0; 3]),
        Err(Error::Anim(AnimError::UnnormalizedQuat(2)))
    );
    assert_eq!(fx.stats().bodies, 0, "every refusal left the world empty");
    assert!(Error::NonFinite(Field::Impulse)
        .to_string()
        .contains("impulse"));
    assert!(Error::OverCap { need: 9, cap: 4 }.to_string().contains('9'));
}

#[test]
fn a_rig_is_checked_once_at_build_time() {
    let (rig, _) = humanoid();
    assert_eq!(rig.bones(), BONES.len());
    let skeleton = Skeleton {
        joints: BONES
            .iter()
            .map(|(name, parent, _, _)| Joint {
                name: (*name).into(),
                parent: *parent,
            })
            .collect(),
    };
    let shapes: Vec<BoneShape> = (0..BONES.len()).map(|_| limb_shape()).collect();
    assert!(Rig::new(skeleton.clone(), shapes.clone()).is_ok());

    let mut short = shapes.clone();
    short.pop();
    assert_eq!(
        Rig::new(skeleton.clone(), short).err(),
        Some(Error::Anim(AnimError::LengthMismatch {
            expected: 14,
            got: 13
        }))
    );
    let mut flat = shapes.clone();
    flat[3].radius = 0.0;
    assert_eq!(
        Rig::new(skeleton.clone(), flat).err(),
        Some(Error::OutOfRange(Field::Extent))
    );
    let mut nan = shapes;
    nan[3].half_height = f32::NAN;
    assert_eq!(
        Rig::new(skeleton, nan).err(),
        Some(Error::NonFinite(Field::Extent))
    );
    assert_eq!(
        Rig::new(Skeleton::default(), Vec::new()).err(),
        Some(Error::Anim(AnimError::EmptySkeleton))
    );

    // A pose from another rig does not fit this one.
    let mut fx = arena(Tier::Web);
    let stub = LocalPose {
        positions: vec![[0.0; 3]],
        rotations: vec![Transform::IDENTITY.rot],
    };
    assert_eq!(
        fx.spawn_ragdoll(0, &rig, &stub, [0.0; 3], [0.0; 3]),
        Err(Error::Anim(AnimError::LengthMismatch {
            expected: 14,
            got: 1
        }))
    );
}

#[test]
fn sleeping_bodies_stop_costing_steps() {
    let mut fx = arena(Tier::Comfort);
    fx.spawn_debris(&debris_field(24, 0.4), [0.0; 3])
        .expect("debris");
    step_n(&mut fx, 2);
    assert_eq!(
        fx.stats().awake,
        24,
        "freshly dropped bodies are all in the solver"
    );

    let mut ticks = 0;
    while fx.stats().awake > 0 {
        fx.step();
        ticks += 1;
        assert!(
            ticks < 600,
            "{} bodies still awake after 10 s",
            fx.stats().awake
        );
    }
    let rest = fx.transforms(1.0).expect("transforms");
    for _ in 0..120 {
        fx.step();
        let stats = fx.stats();
        assert_eq!(
            (stats.awake, stats.contacts),
            (0, 0),
            "a sleeping body is not solved"
        );
    }
    assert_eq!(
        fx.transforms(1.0).expect("transforms"),
        rest,
        "asleep means not moving"
    );
    // The blend between the last two ticks must not drift a settled body.
    assert_eq!(fx.transforms(0.0).expect("transforms"), rest);
    assert_eq!(fx.transforms(0.5).expect("transforms"), rest);

    // A new body wakes the solver again.
    fx.spawn_limb(1, limb_shape(), at([0.0, 2.0, 4.0]), [0.0; 3])
        .expect("limb");
    fx.step();
    assert!(fx.stats().awake >= 1);
}

#[test]
fn transforms_blend_between_the_last_two_ticks() {
    let mut fx = PhysFx::new(Tier::Web);
    fx.spawn_limb(7, limb_shape(), at([0.0, 10.0, 0.0]), [0.0; 3])
        .expect("limb");
    let y = |fx: &PhysFx, alpha: f32| fx.transforms(alpha).expect("transforms")[0].at.pos[1];
    assert_eq!(y(&fx, 0.5), 10.0, "before the first step there is one pose");
    step_n(&mut fx, 2);
    let (prev, curr) = (y(&fx, 0.0), y(&fx, 1.0));
    assert!(
        curr < prev && prev < 10.0,
        "the limb falls: {prev} then {curr}"
    );
    assert!((y(&fx, 0.5) - (prev + curr) / 2.0).abs() < 1e-5);
    assert_eq!(y(&fx, -3.0), prev, "alpha clamps at the previous tick");
    assert_eq!(y(&fx, 9.0), curr, "alpha clamps at the latest tick");
    assert_eq!(
        fx.transforms(f32::NAN).err(),
        Some(Error::NonFinite(Field::Alpha))
    );
    assert_eq!(
        fx.transforms(f32::INFINITY).err(),
        Some(Error::NonFinite(Field::Alpha))
    );
}

#[test]
fn rotations_blend_along_the_short_arc() {
    let quarter_turn = [0.0, 0.5f32.sqrt(), 0.0, 0.5f32.sqrt()];
    let (a, b) = (
        Transform::IDENTITY,
        Transform {
            rot: quarter_turn,
            ..Transform::IDENTITY
        },
    );
    assert_eq!(Transform::blend(a, b, 0.0), a);
    assert_eq!(Transform::blend(a, b, 1.0), b);
    // Halfway through a quarter turn about Y is an eighth turn: sin(pi/8), cos(pi/8).
    let mid = Transform::blend(a, b, 0.5).rot;
    assert!(
        (mid[1] - 0.382_683_43).abs() < 2e-3 && (mid[3] - 0.923_879_5).abs() < 2e-3,
        "{mid:?}"
    );
    // q and -q are the same rotation: the blend does not swing the long way round.
    let flipped = Transform {
        rot: [0.0, 0.0, 0.0, -1.0],
        ..Transform::IDENTITY
    };
    let same = Transform::blend(a, flipped, 0.5).rot;
    assert!(
        (same[3].abs() - 1.0).abs() < 1e-6 && same[..3].iter().all(|c| c.abs() < 1e-6),
        "{same:?}"
    );
}

#[test]
fn a_launched_ragdoll_falls_stays_above_the_floor_and_does_not_explode() {
    let (rig, pose) = humanoid();
    let mut fx = arena(Tier::Comfort);
    let start = moved(&pose, [0.0, 1.0, 0.0]);
    fx.spawn_ragdoll(0, &rig, &start, [2.0, 0.0, 0.0], [0.0, 0.0, 120.0])
        .expect("ragdoll");
    let mut fastest: f32 = 0.0;
    for _ in 0..360 {
        let before = bones_of(&fx, 0);
        fx.step();
        for (old, new) in before.iter().zip(bones_of(&fx, 0)) {
            let d: f32 = (0..3)
                .map(|k| (new.at.pos[k] - old.at.pos[k]).powi(2))
                .sum();
            fastest = fastest.max(d.sqrt() / FIXED_DT);
        }
    }
    assert!(fastest < 30.0, "a bone moved at {fastest} m/s");
    let end = bones_of(&fx, 0);
    assert_eq!(end.len(), rig.bones());
    for (placed, (_, _, _, (radius, _))) in end.iter().zip(&BONES) {
        assert!(placed
            .at
            .pos
            .iter()
            .all(|c| c.is_finite() && c.abs() < 60.0));
        assert!(
            placed.at.pos[1] > radius - 0.03,
            "{:?} sank into the floor",
            placed.key
        );
    }
    let root = end[0].at.pos;
    assert!(
        root[2] > 1.0,
        "the hit pushed the pelvis along +z: {root:?}"
    );
    assert!(
        root[0] > 0.5,
        "the fighter's velocity carried it along +x: {root:?}"
    );
    assert!(root[1] < 1.1, "the body came down from 2.1 m: {root:?}");
}

#[test]
fn contacts_over_the_cap_shed_the_oldest_debris_and_never_a_ragdoll() {
    let (rig, pose) = humanoid();
    let cap = 40;
    let mut fx = on_floor(
        PhysFx::with_caps(Caps {
            contacts: cap,
            ..generous()
        })
        .expect("caps"),
    );
    fx.spawn_ragdoll(
        0,
        &rig,
        &moved(&pose, [-20.0, 0.0, 0.0]),
        [0.0; 3],
        [0.0; 3],
    )
    .expect("ragdoll");
    // A tight heap of 150 boxes overlapping a little: far more touching pairs than the cap.
    let heap: Vec<Shard> = (0..150)
        .map(|i| Shard {
            at: at([
                (i % 5) as f32 * 0.18,
                0.15 + (i / 25) as f32 * 0.18,
                ((i / 5) % 5) as f32 * 0.18,
            ]),
            half: [0.1; 3],
            velocity: [0.0; 3],
        })
        .collect();
    fx.spawn_debris(&heap, [0.0; 3]).expect("debris");
    step_n(&mut fx, 240);
    let stats = fx.stats();
    assert!(stats.recycled > 0, "the heap was thinned");
    assert!(
        stats.contacts <= cap,
        "{} contacts after shedding caught up",
        stats.contacts
    );
    assert_eq!(
        bones_of(&fx, 0).len(),
        rig.bones(),
        "the ragdoll was never shed"
    );
    assert!(stats.bodies < rig.bones() + heap.len());
}

#[test]
fn the_static_cap_and_bad_boxes_are_refused() {
    let mut fx = PhysFx::with_caps(Caps {
        statics: 2,
        ..generous()
    })
    .expect("caps");
    fx.add_static_box([1.0; 3], [0.0; 3]).expect("first");
    fx.add_static_box([1.0; 3], [5.0, 0.0, 0.0])
        .expect("second");
    assert_eq!(
        fx.add_static_box([1.0; 3], [9.0, 0.0, 0.0]),
        Err(Error::OverCap { need: 3, cap: 2 })
    );
    let mut fx = arena(Tier::Web);
    assert_eq!(
        fx.add_static_box([0.0, 1.0, 1.0], [0.0; 3]),
        Err(Error::OutOfRange(Field::Extent))
    );
    assert_eq!(
        fx.add_static_box([f32::NAN, 1.0, 1.0], [0.0; 3]),
        Err(Error::NonFinite(Field::Extent))
    );
    assert_eq!(
        fx.add_static_box([1.0; 3], [f32::NAN, 0.0, 0.0]),
        Err(Error::NonFinite(Field::Position))
    );
    assert_eq!(
        fx.add_static_box([1.0; 3], [0.0, 5.0e3, 0.0]),
        Err(Error::OutOfRange(Field::Position))
    );
}

#[test]
fn stats_count_what_the_step_did() {
    let mut fx = arena(Tier::Web);
    assert_eq!(fx.stats(), Stats::default());
    // A limb lying on the floor touches it.
    fx.spawn_limb(
        1,
        BoneShape {
            radius: 0.1,
            half_height: 0.2,
        },
        at([0.0, 0.09, 0.0]),
        [0.0; 3],
    )
    .expect("limb");
    fx.step();
    let stats = fx.stats();
    assert_eq!((stats.bodies, stats.steps, stats.recycled), (1, 1, 0));
    assert!(stats.contacts >= 1 && stats.awake == 1, "{stats:?}");
}

static CLOCK_US: AtomicU64 = AtomicU64::new(0);

/// Every read advances the clock by 250 microseconds.
fn fake_clock() -> u64 {
    CLOCK_US.fetch_add(250, Ordering::SeqCst)
}

#[test]
fn stats_micros_come_from_the_clock_the_caller_gives() {
    let mut plain = PhysFx::new(Tier::Web);
    plain.step();
    assert_eq!(plain.stats().micros, 0, "no clock, no reading");
    let mut timed = PhysFx::new(Tier::Web).with_clock(fake_clock);
    timed.step();
    assert_eq!(timed.stats().micros, 250);
}

/// Every `.rs` and `Cargo.toml` file under `dir` that names the physical layer.
fn files_naming_physfx(dir: &Path) -> Vec<String> {
    let mut hits = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n != "target") {
                    pending.push(path);
                }
                continue;
            }
            let wanted = path.extension().is_some_and(|e| e == "rs")
                || path.file_name().is_some_and(|n| n == "Cargo.toml");
            let named = std::fs::read_to_string(&path)
                .is_ok_and(|text| text.contains("engine_physfx") || text.contains("engine-physfx"));
            if wanted && named {
                hits.push(path.display().to_string());
            }
        }
    }
    hits
}

#[test]
fn sim_crates_never_name_physfx() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for name in ["arena", "combat", "proof"] {
        let hits = files_naming_physfx(&crates.join(name));
        assert!(
            hits.is_empty(),
            "crates/{name} must not use the physical layer: {hits:?}"
        );
    }
    // The scan can see a leak: it finds this very crate, and a planted file.
    assert!(!files_naming_physfx(&crates.join("physfx").join("src")).is_empty());
    let planted = std::env::temp_dir().join(format!("physfx-gate-{}", std::process::id()));
    std::fs::create_dir_all(&planted).expect("temp dir");
    std::fs::write(planted.join("leak.rs"), "use engine_physfx::PhysFx;\n").expect("plant");
    assert_eq!(files_naming_physfx(&planted).len(), 1);
    std::fs::remove_dir_all(&planted).expect("clean up");
}

/// Microseconds since the first read, from the real clock (tests may use `Instant`).
fn wall_micros() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    u64::try_from(START.get_or_init(Instant::now).elapsed().as_micros()).unwrap_or(u64::MAX)
}

/// The soak of the plan: 8 ragdolls, 200 debris bodies and 6 severed limbs on the comfortable
/// tier. In release a tick must cost under 4 ms; a debug build only runs the scene.
#[test]
fn budget_eight_ragdolls_two_hundred_debris_six_limbs() {
    let (rig, pose) = humanoid();
    let mut fx = arena(Tier::Comfort).with_clock(wall_micros);
    for slot in 0..8u8 {
        let start = moved(&pose, [2.5 * f32::from(slot) - 8.75, 0.5, 0.0]);
        let knock = [0.0, 0.0, 40.0 + 10.0 * f32::from(slot)];
        fx.spawn_ragdoll(slot, &rig, &start, [0.0, 0.0, 1.0], knock)
            .expect("ragdoll");
    }
    fx.spawn_debris(&debris_field(200, 2.0), [0.0, 150.0, 0.0])
        .expect("debris");
    for part in 0..6u32 {
        let pos = [part as f32 - 2.5, 2.5, -3.0];
        fx.spawn_limb(part, limb_shape(), at(pos), [0.0, 1.0, -1.0])
            .expect("limb");
    }
    assert_eq!(fx.stats().bodies, 8 * rig.bones() + 200 + 6);

    let mut micros = Vec::new();
    for _ in 0..180 {
        fx.step();
        micros.push(fx.stats().micros);
    }
    assert_eq!(
        fx.stats().recycled,
        0,
        "the soak fits the tier without shedding"
    );
    micros.sort_unstable();
    let mean = micros.iter().sum::<u64>() / micros.len() as u64;
    let (p95, max) = (micros[micros.len() * 95 / 100], micros[micros.len() - 1]);
    println!(
        "physfx budget: {} bodies, 180 ticks: mean {mean} us, p95 {p95} us, max {max} us (bar: mean < 4000 us in release)",
        fx.stats().bodies
    );
    if !cfg!(debug_assertions) {
        assert!(mean < 4_000, "mean {mean} us per tick is over the 4 ms bar");
        assert!(p95 < 8_000, "p95 {p95} us per tick");
    }
}
