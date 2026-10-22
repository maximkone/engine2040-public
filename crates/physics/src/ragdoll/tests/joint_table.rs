//! Joint-table tests for the ragdoll: the named rows and their kinds, the knee
//! range, the spawn read and the two refusals. The spawn read measures the private
//! joint set and body order, so this module stays a child of `ragdoll`.

use super::super::*;
use super::{humanoid_pose, humanoid_shapes, humanoid_skeleton, unit01};
use engine_anim::contract::Joint;

/// The Rapier joint whose second body is `bone`, with the bone pair it joins.
fn joint_data(r: &Ragdoll, bone: usize) -> &rapier3d::dynamics::GenericJoint {
    let handle = r.order[bone];
    r.impulse_joints
        .iter()
        .find(|(_, joint)| joint.body2() == handle)
        .map(|(_, joint)| &joint.data)
        .expect("one joint per bone but the root")
}

/// Bone `(parent, child)` pairs of every Rapier joint.
fn joint_pairs(r: &Ragdoll) -> Vec<(usize, usize)> {
    r.impulse_joints
        .iter()
        .map(|(_, joint)| {
            let parent = r
                .order
                .iter()
                .position(|h| *h == joint.body1())
                .expect("parent body");
            let child = r
                .order
                .iter()
                .position(|h| *h == joint.body2())
                .expect("child body");
            (parent, child)
        })
        .collect()
}

#[test]
fn joint_table_humanoid_defaults() {
    let skeleton = humanoid_skeleton();
    let table = JointTable::humanoid(&skeleton);
    assert_eq!(table.len(), 17);
    assert!(!table.is_empty());
    assert!(table.is_valid(), "every row inside 0..180 degrees");
    // Hinge knees and elbows (4), ball cone plus twist elsewhere (13); the named
    // rows carry their own numbers, so depth is only the fallback.
    let mut hinges = 0usize;
    let mut named_balls = 0usize;
    let depths = joint_depths(&skeleton);
    for (i, joint) in skeleton.joints.iter().enumerate() {
        let spec = table.spec(i).expect("row per bone");
        if joint.name.contains("knee") || joint.name.contains("elbow") {
            assert_eq!(spec.kind, JointKind::Hinge, "bone {i} must hinge");
            assert_eq!((spec.twist_min, spec.twist_max), (0.0, 0.0));
            assert!((spec.swing_min - 1.0 * 0.017_453_292).abs() < 1e-5);
            assert!((spec.swing_limit - 145.0 * 0.017_453_292).abs() < 1e-5);
            assert_eq!(spec.friction_torque, JOINT_FRICTION_HINGE_NM);
            assert_eq!(spec.spring_hz, JOINT_SPRING_HINGE_HZ);
            hinges += 1;
        } else {
            assert_eq!(spec.kind, JointKind::Ball, "bone {i} ball");
            assert!(
                spec.twist_min < 0.0 && spec.twist_max > 0.0,
                "bone {i} keeps a twist range"
            );
            assert_eq!(spec.friction_torque, JOINT_FRICTION_BALL_NM);
            assert_eq!(spec.spring_hz, JOINT_SPRING_BALL_HZ);
            assert_eq!(spec.swing_min, 0.0, "bone {i} ball keeps no low end");
        }
        // A known name departs from the depth fallback; the root and an
        // unknown name (ankle, wrist) keep it.
        let depth_limit = swing_limit(depths[i]);
        if joint.parent.is_none() || joint.name.contains("ankle") || joint.name.contains("wrist") {
            assert!(
                (spec.swing_limit - depth_limit).abs() < 1e-6,
                "bone {i} ({}) keeps the depth fallback",
                joint.name
            );
            assert_eq!(spec.swing_lateral, spec.swing_limit, "bone {i} cone");
        } else {
            assert!(
                (spec.swing_limit - depth_limit).abs() > 1e-6,
                "bone {i} ({}) must read the table, not depth",
                joint.name
            );
            if spec.kind == JointKind::Ball {
                named_balls += 1;
            }
        }
        assert!(spec.swing_lateral <= spec.swing_limit, "bone {i} cone");
        assert!(
            (5.0..=10.0).contains(&spec.friction_torque),
            "friction 5..10 N.m: {spec:?}"
        );
        assert!(
            (0.5..=2.0).contains(&spec.spring_hz),
            "spring 0.5..2 Hz: {spec:?}"
        );
    }
    assert_eq!(hinges, 4, "knees plus elbows hinge");
    assert_eq!(named_balls, 8, "eight named balls read the table");
    // The hips carry the asymmetric thigh row: 70 forward, 40 to the side.
    let hip = table.spec(11).expect("hip_l");
    assert!(hip.swing_limit > hip.swing_lateral && hip.swing_lateral > 0.0);
    assert_eq!(table.spec(99), None, "past-end bone is none");
    // Failure cases: non-finite hinge maps to the low end, twist clamps both ends.
    let hinge = table.spec(6).expect("elbow_l");
    assert_eq!(hinge.clamp_hinge(f32::NAN), hinge.swing_min);
    assert_eq!(hinge.clamp_hinge(-0.5), hinge.swing_min);
    assert_eq!(hinge.clamp_hinge(0.5), 0.5);
    assert_eq!(hinge.clamp_hinge(99.0), hinge.swing_limit);
    let ball = table.spec(2).expect("chest");
    assert_eq!(ball.clamp_twist(-99.0), ball.twist_min);
    assert_eq!(ball.clamp_twist(99.0), ball.twist_max);
    assert_eq!(ball.clamp_twist(f32::INFINITY), ball.twist_min);
    // Same skeleton twice gives the same table.
    assert_eq!(table, JointTable::humanoid(&humanoid_skeleton()));
    println!("JOINT_TABLE hinges={hinges} rows=17 friction=5..10Nm spring=0.5..2Hz");
}

#[test]
fn joint_table_knee_flex_reaches_145_not_more() {
    let skeleton = humanoid_skeleton();
    let table = JointTable::humanoid(&skeleton);
    let r = Ragdoll::spawn(
        &skeleton,
        &humanoid_pose([0.0, 1.5, 0.0]),
        &humanoid_shapes(),
    )
    .expect("humanoid spawns");
    let deg = 0.017_453_292;
    // A knee lives in Rapier as a one-axis hinge: 1..145 degrees on AngX, no
    // limit on the other two axes.
    for bone in [12usize, 15] {
        let data = joint_data(&r, bone);
        let limits = data.limits(JointAxis::AngX).expect("knee hinge limits");
        assert!((limits.min - deg).abs() < 1e-5, "knee {bone} low end");
        assert!(
            (limits.max - 145.0 * deg).abs() < 1e-5,
            "knee {bone} high end"
        );
        assert_eq!(data.limits(JointAxis::AngY), None);
        assert_eq!(data.limits(JointAxis::AngZ), None);
        // A deep squat asks for 3 rad; the row stops it at 145 degrees and not
        // a hair more.
        let spec = table.spec(bone).expect("knee row");
        assert_eq!(spec.clamp_hinge(3.0), spec.swing_limit);
        assert!(spec.swing_limit <= 145.0 * deg + 1e-6);
    }
    // The elbow folds the other way: -145..-1 degrees.
    let limits = joint_data(&r, 6)
        .limits(JointAxis::AngX)
        .expect("elbow hinge limits");
    assert!((limits.min + 145.0 * deg).abs() < 1e-5);
    assert!((limits.max + deg).abs() < 1e-5);
    let hash = r.state_hash();
    println!("JOINT_KNEE flex_deg=145 axis=AngX hash={hash:016x}");
}

#[test]
fn joint_table_spawn_read_is_a_measurement() {
    let skeleton = humanoid_skeleton();
    let table = JointTable::try_humanoid(&skeleton).expect("the humanoid table");
    assert_eq!(
        table,
        JointTable::humanoid(&skeleton),
        "spawn reads the table"
    );
    let r = Ragdoll::spawn(
        &skeleton,
        &humanoid_pose([0.0, 1.5, 0.0]),
        &humanoid_shapes(),
    )
    .expect("humanoid spawns");
    // One joint per bone but the root.
    assert_eq!(r.impulse_joints.len(), skeleton.len() - 1);
    let pairs = joint_pairs(&r);
    assert_eq!(pairs.len(), 16);
    let hash = r.state_hash();
    let poses = r.bone_poses();
    let mut hinges = 0usize;
    let mut balls = 0usize;
    for &(parent, child) in &pairs {
        assert_eq!(skeleton.joints[child].parent, Some(parent), "bone {child}");
        let spec = table.spec(child).expect("row");
        let data = joint_data(&r, child);
        match spec.kind {
            JointKind::Hinge => {
                let limits = data.limits(JointAxis::AngX).expect("hinge axis");
                let knee = skeleton.joints[child].name.contains("knee");
                let (want_min, want_max) = if knee {
                    (spec.swing_min, spec.swing_limit)
                } else {
                    (-spec.swing_limit, -spec.swing_min)
                };
                assert!((limits.min - want_min).abs() < 1e-6, "bone {child}");
                assert!((limits.max - want_max).abs() < 1e-6, "bone {child}");
                assert_eq!(data.limits(JointAxis::AngY), None);
                assert_eq!(data.limits(JointAxis::AngZ), None);
                hinges += 1;
            }
            JointKind::Ball => {
                let x = data.limits(JointAxis::AngX).expect("cone x");
                let y = data.limits(JointAxis::AngY).expect("twist");
                let z = data.limits(JointAxis::AngZ).expect("cone z");
                assert!((x.max - spec.swing_limit).abs() < 1e-6, "bone {child}");
                assert!((z.max - spec.swing_lateral).abs() < 1e-6, "bone {child}");
                assert!((y.min - spec.twist_min).abs() < 1e-6, "bone {child}");
                assert!((y.max - spec.twist_max).abs() < 1e-6, "bone {child}");
                balls += 1;
            }
        }
    }
    assert_eq!((hinges, balls), (4, 12), "four hinges, twelve balls");
    // The read changed nothing: the animated pose and the state hash stay put.
    assert_eq!(r.state_hash(), hash);
    assert_eq!(r.bone_poses(), poses);
    println!(
        "JOINT_SPAWN_READ joints={} hinges={hinges} balls={balls} hash={hash:016x}",
        r.impulse_joints.len()
    );
}

#[test]
fn joint_table_refuses_a_missing_joint() {
    let mut skeleton = humanoid_skeleton();
    // Detach one leg: two roots, so 17 bones carry 15 links.
    skeleton.joints[12].parent = None;
    assert_eq!(
        JointTable::try_humanoid(&skeleton),
        Err(JointError::JointCount {
            bodies: 17,
            links: 15
        })
    );
    // Spawn refuses the same skeleton before a body is built.
    let err = Ragdoll::spawn(
        &skeleton,
        &humanoid_pose([0.0, 1.5, 0.0]),
        &humanoid_shapes(),
    )
    .err()
    .expect("a detached knee must be refused");
    assert!(matches!(
        err,
        Error::Joint(JointError::JointCount {
            bodies: 17,
            links: 15
        })
    ));
    // Control: two roots are a free scene, not a torn rig. A blade and a limb
    // share one pipeline and no joint joins them, so the count rule lets them
    // through; the CCD bullet scene is built this way.
    let free = Skeleton {
        joints: vec![
            Joint {
                name: "blade".into(),
                parent: None,
            },
            Joint {
                name: "limb".into(),
                parent: None,
            },
        ],
    };
    crate::joint_limits::check_joint_count(&free).expect("two free roots are not a torn rig");
    let table = JointTable::try_humanoid(&free).expect("free scene table");
    assert_eq!(table.len(), 2);
    let pose = LocalPose {
        positions: vec![[-1.0, 0.5, 0.0], [0.0, 0.5, 0.0]],
        rotations: vec![[0.0, 0.0, 0.0, 1.0]; 2],
    };
    let shapes = vec![
        BoneShape {
            radius: 0.02,
            half_height: 0.30,
        },
        BoneShape {
            radius: 0.05,
            half_height: 0.40,
        },
    ];
    let r = Ragdoll::spawn(&free, &pose, &shapes).expect("free pair spawns");
    assert!(r.impulse_joints.is_empty(), "free bodies carry no joint");
    // Control: the intact humanoid spawns.
    assert!(Ragdoll::spawn(
        &humanoid_skeleton(),
        &humanoid_pose([0.0, 1.5, 0.0]),
        &humanoid_shapes()
    )
    .is_ok());
    println!("JOINT_REFUSAL bones=17 links=15 detached_knee=refused free_roots=2 spawn=ok");
}

#[test]
fn joint_table_no_joint_over_limit_1000_seeds() {
    let names = [
        "pelvis",
        "spine",
        "chest",
        "neck",
        "head",
        "shoulder_l",
        "upper_arm_l",
        "elbow_l",
        "forearm_l",
        "wrist_l",
        "hip_l",
        "thigh_l",
        "knee_l",
        "shin_l",
        "ankle_l",
        "shoulder_r",
        "upper_arm_r",
        "elbow_r",
        "knee_r",
        "ankle_r",
    ];
    let mut worst_deg = 0.0f32;
    for seed in 0..1_000u64 {
        let mut rng = engine_core::Rng::new(7_000).fork(seed);
        let bones = 2 + usize::try_from(rng.next_u64() % 19).expect("small");
        let skeleton = Skeleton {
            joints: (0..bones)
                .map(|i| {
                    let parent = if i == 0 {
                        None
                    } else {
                        let below = u64::try_from(i).expect("small");
                        Some(usize::try_from(rng.next_u64() % below).expect("small"))
                    };
                    let pick = usize::try_from(rng.next_u64() % 20).expect("small");
                    Joint {
                        name: names[pick].into(),
                        parent,
                    }
                })
                .collect(),
        };
        let table = JointTable::try_humanoid(&skeleton).expect("one tree");
        for (i, spec) in table.rows.iter().enumerate() {
            assert!(spec.is_valid(), "seed {seed} bone {i} {spec:?}");
            assert!(spec.swing_limit <= JOINT_LIMIT_MAX_RAD, "seed {seed}");
            assert!(spec.swing_lateral <= JOINT_LIMIT_MAX_RAD, "seed {seed}");
            assert!(spec.swing_min <= spec.swing_limit, "seed {seed}");
            assert!(spec.twist_min >= -JOINT_LIMIT_MAX_RAD, "seed {seed}");
            assert!(spec.twist_max <= JOINT_LIMIT_MAX_RAD, "seed {seed}");
            worst_deg = worst_deg.max(spec.swing_limit / 0.017_453_292);
            // A seeded raw angle lands inside the row, never outside it.
            let raw = unit01(rng.next_u64()) * 12.0 - 6.0;
            if spec.kind == JointKind::Hinge {
                let clamped = spec.clamp_hinge(raw);
                assert!(
                    (spec.swing_min..=spec.swing_limit).contains(&clamped),
                    "seed {seed} bone {i} hinge {raw} -> {clamped}"
                );
            } else {
                let clamped = spec.clamp_twist(raw);
                assert!(
                    (spec.twist_min..=spec.twist_max).contains(&clamped),
                    "seed {seed} bone {i} twist {raw} -> {clamped}"
                );
            }
        }
    }
    assert!(worst_deg <= JOINT_LIMIT_MAX_DEG, "worst {worst_deg}");
    println!(
        "JOINT_LIMITS seeds=1000 worst_swing_deg={worst_deg:.1} max_deg={JOINT_LIMIT_MAX_DEG}"
    );
}
