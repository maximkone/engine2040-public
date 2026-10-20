//! SOMA-77 to fighter-65 retarget: the name map plus one synthetic cook.
//!
//! The source rig is the SOMA table (77 joints); the target rig is built from
//! the same pair list (65 joints). Binds are authored in meters. The clip
//! holds the fighter pose at t = 0, walks the root 0.30 m and plants or lifts
//! each foot four times in one second, so one cook covers bind scale, joint
//! routing, pose landing and the contact table.
#![cfg(feature = "import")]

use engine_anim::contract::{skinning_matrices, ContactTable, Joint, LocalPose, Skeleton};
use engine_anim::pose::QuatTrack;
use engine_anim::{Clip, Track};
use engine_assets::soma::{coverage, soma_pairs, RENAMES, SOMA77, UNUSED_SOMA};
use engine_assets::{
    retarget_cook, retarget_digest, BindPose, BoneMap, ClipTags, CookConfig, ImportedClip,
    RetargetError, RetargetPlan,
};

/// Identity rotation: the rest rotation on both rigs.
const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
/// Clip length in seconds: 60 ticks at the 60 Hz cook.
const DURATION: f32 = 1.0;
/// Foot local height at or below which the cook reads a plant (meters).
const GROUND_Y: f32 = -0.35;
/// Foot offset that plants a foot; the cook scales it by the bind ratio.
const PLANT: f32 = -0.02;
/// Foot offset that lifts a foot clear of [`GROUND_Y`].
const LIFT: f32 = 0.22;
/// Left arm swing at half the clip (a unit quaternion about x).
const SWING: [f32; 4] = [0.2, 0.0, 0.0, 0.979_795_9];

/// Left foot keys `(time, offset from bind)`: four plants and four lifts.
const LEFT_FOOT: [(f32, f32); 16] = [
    (0.00, PLANT),
    (0.10, PLANT),
    (0.12, LIFT),
    (0.23, LIFT),
    (0.25, PLANT),
    (0.35, PLANT),
    (0.37, LIFT),
    (0.48, LIFT),
    (0.50, PLANT),
    (0.60, PLANT),
    (0.62, LIFT),
    (0.73, LIFT),
    (0.75, PLANT),
    (0.85, PLANT),
    (0.87, LIFT),
    (0.98, LIFT),
];

/// Right foot keys, half a cycle behind the left: four plants, four lifts.
const RIGHT_FOOT: [(f32, f32); 17] = [
    (0.00, LIFT),
    (0.09, LIFT),
    (0.11, PLANT),
    (0.21, PLANT),
    (0.24, LIFT),
    (0.34, LIFT),
    (0.36, PLANT),
    (0.46, PLANT),
    (0.49, LIFT),
    (0.59, LIFT),
    (0.61, PLANT),
    (0.71, PLANT),
    (0.74, LIFT),
    (0.84, LIFT),
    (0.86, PLANT),
    (0.94, PLANT),
    (0.96, LIFT),
];

/// The source rig: SOMA joints in table order, parents resolved by name.
fn soma_skeleton() -> Skeleton {
    let mut joints: Vec<Joint> = Vec::with_capacity(SOMA77.len());
    for &(name, parent) in SOMA77.iter() {
        let parent = parent.map(|parent| {
            joints
                .iter()
                .position(|joint| joint.name == parent)
                .unwrap_or_else(|| panic!("{parent} must precede {name}"))
        });
        joints.push(Joint {
            name: name.to_string(),
            parent,
        });
    }
    Skeleton { joints }
}

fn soma_parent(name: &str) -> Option<&'static str> {
    SOMA77
        .iter()
        .find(|(soma, _)| *soma == name)
        .and_then(|(_, parent)| *parent)
}

/// The target rig: one joint per pair, parent-first because the pair list is.
/// A SOMA parent with no fighter joint (the second neck joint) is skipped, so
/// the fighter head hangs from the neck.
fn fighter_skeleton(pairs: &[(&str, &str)]) -> Skeleton {
    let mut joints: Vec<Joint> = Vec::with_capacity(pairs.len());
    for &(target, soma) in pairs {
        let mut ancestor = soma_parent(soma);
        let parent = loop {
            let Some(name) = ancestor else {
                break None;
            };
            match pairs.iter().position(|(_, source)| *source == name) {
                Some(index) => break Some(index),
                None => ancestor = soma_parent(name),
            }
        };
        joints.push(Joint {
            name: target.to_string(),
            parent,
        });
    }
    Skeleton { joints }
}

/// Fighter bind local offsets in meters, one row per pair in order. Identity
/// rest rotations, so the world pose is the sum along each chain.
static FIGHTER_BIND: [(&str, [f32; 3]); 65] = [
    ("hips", [0.00, 0.95, 0.00]),
    ("spine", [0.00, 0.07, 0.00]),
    ("spine1", [0.00, 0.07, 0.00]),
    ("spine2", [0.00, 0.07, 0.02]),
    ("neck", [0.00, 0.10, 0.00]),
    ("head", [0.00, 0.08, 0.00]),
    ("headtop_end", [0.00, 0.12, 0.00]),
    ("leftshoulder", [0.08, 0.04, 0.00]),
    ("leftarm", [0.10, 0.00, 0.00]),
    ("leftforearm", [0.28, 0.00, 0.00]),
    ("lefthand", [0.26, 0.00, 0.00]),
    ("lefthandthumb1", [0.02, -0.02, 0.06]),
    ("lefthandthumb2", [0.03, -0.02, 0.02]),
    ("lefthandthumb3", [0.02, -0.01, 0.02]),
    ("lefthandthumb4", [0.02, -0.01, 0.02]),
    ("lefthandindex1", [0.04, 0.00, -0.02]),
    ("lefthandindex2", [0.04, 0.00, 0.00]),
    ("lefthandindex3", [0.03, 0.00, 0.00]),
    ("lefthandindex4", [0.02, 0.00, 0.00]),
    ("lefthandmiddle1", [0.04, 0.00, 0.00]),
    ("lefthandmiddle2", [0.05, 0.00, 0.00]),
    ("lefthandmiddle3", [0.04, 0.00, 0.00]),
    ("lefthandmiddle4", [0.03, 0.00, 0.00]),
    ("lefthandring1", [0.04, 0.00, 0.02]),
    ("lefthandring2", [0.04, 0.00, 0.00]),
    ("lefthandring3", [0.03, 0.00, 0.00]),
    ("lefthandring4", [0.03, 0.00, 0.00]),
    ("lefthandpinky1", [0.03, 0.00, 0.04]),
    ("lefthandpinky2", [0.03, 0.00, 0.00]),
    ("lefthandpinky3", [0.02, 0.00, 0.00]),
    ("lefthandpinky4", [0.02, 0.00, 0.00]),
    ("rightshoulder", [-0.08, 0.04, 0.00]),
    ("rightarm", [-0.10, 0.00, 0.00]),
    ("rightforearm", [-0.28, 0.00, 0.00]),
    ("righthand", [-0.26, 0.00, 0.00]),
    ("righthandthumb1", [-0.02, -0.02, 0.06]),
    ("righthandthumb2", [-0.03, -0.02, 0.02]),
    ("righthandthumb3", [-0.02, -0.01, 0.02]),
    ("righthandthumb4", [-0.02, -0.01, 0.02]),
    ("righthandindex1", [-0.04, 0.00, -0.02]),
    ("righthandindex2", [-0.04, 0.00, 0.00]),
    ("righthandindex3", [-0.03, 0.00, 0.00]),
    ("righthandindex4", [-0.02, 0.00, 0.00]),
    ("righthandmiddle1", [-0.04, 0.00, 0.00]),
    ("righthandmiddle2", [-0.05, 0.00, 0.00]),
    ("righthandmiddle3", [-0.04, 0.00, 0.00]),
    ("righthandmiddle4", [-0.03, 0.00, 0.00]),
    ("righthandring1", [-0.04, 0.00, 0.02]),
    ("righthandring2", [-0.04, 0.00, 0.00]),
    ("righthandring3", [-0.03, 0.00, 0.00]),
    ("righthandring4", [-0.03, 0.00, 0.00]),
    ("righthandpinky1", [-0.03, 0.00, 0.04]),
    ("righthandpinky2", [-0.03, 0.00, 0.00]),
    ("righthandpinky3", [-0.02, 0.00, 0.00]),
    ("righthandpinky4", [-0.02, 0.00, 0.00]),
    ("leftupleg", [0.08, -0.05, 0.00]),
    ("leftleg", [0.01, -0.40, 0.00]),
    ("leftfoot", [0.00, -0.42, 0.00]),
    ("lefttoebase", [0.00, -0.02, 0.10]),
    ("lefttoe_end", [0.00, 0.00, 0.08]),
    ("rightupleg", [-0.08, -0.05, 0.00]),
    ("rightleg", [-0.01, -0.40, 0.00]),
    ("rightfoot", [0.00, -0.42, 0.00]),
    ("righttoebase", [0.00, -0.02, 0.10]),
    ("righttoe_end", [0.00, 0.00, 0.08]),
];

/// The world pose the clip asks for at t = 0, summed by hand from
/// [`FIGHTER_BIND`]: the root walks +0.10 m on x, the six leg and spine trap
/// joints carry their authored offsets, the left foot plants 0.01 m down and
/// the right foot lifts 0.11 m up after the 0.5 bind scale.
static FIGHTER_POSE: [(&str, [f32; 3]); 65] = [
    ("hips", [0.10, 0.95, 0.00]),
    ("spine", [0.10, 1.03, 0.00]),
    ("spine1", [0.10, 1.12, 0.00]),
    ("spine2", [0.10, 1.22, 0.02]),
    ("neck", [0.10, 1.32, 0.02]),
    ("head", [0.10, 1.40, 0.02]),
    ("headtop_end", [0.10, 1.52, 0.02]),
    ("leftshoulder", [0.18, 1.26, 0.02]),
    ("leftarm", [0.28, 1.26, 0.02]),
    ("leftforearm", [0.56, 1.26, 0.02]),
    ("lefthand", [0.82, 1.26, 0.02]),
    ("lefthandthumb1", [0.84, 1.24, 0.08]),
    ("lefthandthumb2", [0.87, 1.22, 0.10]),
    ("lefthandthumb3", [0.89, 1.21, 0.12]),
    ("lefthandthumb4", [0.91, 1.20, 0.14]),
    ("lefthandindex1", [0.86, 1.26, 0.00]),
    ("lefthandindex2", [0.90, 1.26, 0.00]),
    ("lefthandindex3", [0.93, 1.26, 0.00]),
    ("lefthandindex4", [0.95, 1.26, 0.00]),
    ("lefthandmiddle1", [0.86, 1.26, 0.02]),
    ("lefthandmiddle2", [0.91, 1.26, 0.02]),
    ("lefthandmiddle3", [0.95, 1.26, 0.02]),
    ("lefthandmiddle4", [0.98, 1.26, 0.02]),
    ("lefthandring1", [0.86, 1.26, 0.04]),
    ("lefthandring2", [0.90, 1.26, 0.04]),
    ("lefthandring3", [0.93, 1.26, 0.04]),
    ("lefthandring4", [0.96, 1.26, 0.04]),
    ("lefthandpinky1", [0.85, 1.26, 0.06]),
    ("lefthandpinky2", [0.88, 1.26, 0.06]),
    ("lefthandpinky3", [0.90, 1.26, 0.06]),
    ("lefthandpinky4", [0.92, 1.26, 0.06]),
    ("rightshoulder", [0.02, 1.26, 0.02]),
    ("rightarm", [-0.08, 1.26, 0.02]),
    ("rightforearm", [-0.36, 1.26, 0.02]),
    ("righthand", [-0.62, 1.26, 0.02]),
    ("righthandthumb1", [-0.64, 1.24, 0.08]),
    ("righthandthumb2", [-0.67, 1.22, 0.10]),
    ("righthandthumb3", [-0.69, 1.21, 0.12]),
    ("righthandthumb4", [-0.71, 1.20, 0.14]),
    ("righthandindex1", [-0.66, 1.26, 0.00]),
    ("righthandindex2", [-0.70, 1.26, 0.00]),
    ("righthandindex3", [-0.73, 1.26, 0.00]),
    ("righthandindex4", [-0.75, 1.26, 0.00]),
    ("righthandmiddle1", [-0.66, 1.26, 0.02]),
    ("righthandmiddle2", [-0.71, 1.26, 0.02]),
    ("righthandmiddle3", [-0.75, 1.26, 0.02]),
    ("righthandmiddle4", [-0.78, 1.26, 0.02]),
    ("righthandring1", [-0.66, 1.26, 0.04]),
    ("righthandring2", [-0.70, 1.26, 0.04]),
    ("righthandring3", [-0.73, 1.26, 0.04]),
    ("righthandring4", [-0.76, 1.26, 0.04]),
    ("righthandpinky1", [-0.65, 1.26, 0.06]),
    ("righthandpinky2", [-0.68, 1.26, 0.06]),
    ("righthandpinky3", [-0.70, 1.26, 0.06]),
    ("righthandpinky4", [-0.72, 1.26, 0.06]),
    ("leftupleg", [0.19, 0.90, 0.00]),
    ("leftleg", [0.20, 0.51, 0.00]),
    ("leftfoot", [0.20, 0.08, 0.00]),
    ("lefttoebase", [0.20, 0.06, 0.10]),
    ("lefttoe_end", [0.20, 0.06, 0.18]),
    ("rightupleg", [0.01, 0.90, 0.00]),
    ("rightleg", [0.00, 0.51, 0.00]),
    ("rightfoot", [0.00, 0.20, 0.00]),
    ("righttoebase", [0.00, 0.18, 0.10]),
    ("righttoe_end", [0.00, 0.18, 0.18]),
];

fn fighter_bind(skeleton: &Skeleton) -> BindPose {
    assert_eq!(skeleton.len(), FIGHTER_BIND.len(), "bind rows match joints");
    let positions = FIGHTER_BIND
        .iter()
        .enumerate()
        .map(|(index, (name, position))| {
            assert_eq!(skeleton.joints[index].name, *name, "bind row {index}");
            *position
        })
        .collect();
    BindPose {
        positions,
        rotations: vec![IDENTITY; skeleton.len()],
    }
}

/// The SOMA bind: every mapped SOMA joint takes its fighter pair's local
/// offset, so both rigs agree on the bone lengths; the two feet double theirs,
/// which makes the cook scale their travel by 0.5. Unmapped SOMA joints sit at
/// their parent's origin.
fn soma_bind(source: &Skeleton, pairs: &[(&str, &str)], fighter_bind: &BindPose) -> BindPose {
    let mut positions = Vec::with_capacity(source.len());
    for joint in &source.joints {
        let mapped = pairs
            .iter()
            .position(|(_, soma)| *soma == joint.name)
            .map(|slot| fighter_bind.positions[slot]);
        let position = match joint.name.as_str() {
            "leftfoot" => [-0.01, -0.44, 0.00],
            "rightfoot" => [0.01, -0.44, 0.00],
            _ => mapped.unwrap_or([0.0, 0.0, 0.0]),
        };
        positions.push(position);
    }
    BindPose {
        positions,
        rotations: vec![IDENTITY; source.len()],
    }
}

/// The clip's bind-relative offset per SOMA joint: the trap joints that a
/// plain name match would bind to the wrong fighter joint, the feet, and the
/// root are the only moving joints.
fn clip_offset(name: &str) -> [f32; 3] {
    match name {
        "spine1" => [0.0, 0.01, 0.0],
        "spine2" => [0.0, 0.02, 0.0],
        "chest" => [0.0, 0.03, 0.0],
        "leftleg" => [0.01, 0.0, 0.0],
        "leftshin" => [0.0, 0.01, 0.0],
        "rightleg" => [-0.01, 0.0, 0.0],
        "rightshin" => [0.0, 0.01, 0.0],
        _ => [0.0, 0.0, 0.0],
    }
}

fn foot_track(base: [f32; 3], keys: &[(f32, f32)]) -> Track {
    Track {
        times: keys.iter().map(|(time, _)| *time).collect(),
        values: keys
            .iter()
            .map(|(_, offset)| [base[0], base[1] + offset, base[2]])
            .collect(),
    }
}

fn clip(source: &Skeleton, bind: &BindPose) -> ImportedClip {
    let mut positions = Vec::with_capacity(source.len());
    let mut rotations = Vec::with_capacity(source.len());
    for (index, joint) in source.joints.iter().enumerate() {
        let base = bind.positions[index];
        let track = match joint.name.as_str() {
            "hips" => Track {
                times: vec![0.0, 0.5, 1.0],
                values: vec![
                    [base[0] + 0.10, base[1], base[2]],
                    [base[0] + 0.25, base[1] + 0.01, base[2] + 0.02],
                    [base[0] + 0.40, base[1], base[2]],
                ],
            },
            "leftfoot" => foot_track(base, &LEFT_FOOT),
            "rightfoot" => foot_track(base, &RIGHT_FOOT),
            name => {
                let offset = clip_offset(name);
                Track {
                    times: vec![0.0],
                    values: vec![[
                        base[0] + offset[0],
                        base[1] + offset[1],
                        base[2] + offset[2],
                    ]],
                }
            }
        };
        positions.push(track);
        let rotation = if joint.name == "leftarm" {
            QuatTrack {
                times: vec![0.0, 0.5, 1.0],
                values: vec![IDENTITY, SWING, IDENTITY],
            }
        } else {
            QuatTrack {
                times: vec![0.0],
                values: vec![IDENTITY],
            }
        };
        rotations.push(rotation);
    }
    ImportedClip {
        name: "soma_walk".to_string(),
        duration: DURATION,
        positions: Clip { tracks: positions },
        rotations,
    }
}

fn pose_at_zero(clip: &ImportedClip) -> LocalPose {
    LocalPose {
        positions: clip
            .positions
            .tracks
            .iter()
            .map(|track| track.values[0])
            .collect(),
        rotations: clip.rotations.iter().map(|track| track.values[0]).collect(),
    }
}

/// Plant and lift edges per foot: a plant opens a window, a lift closes it.
fn contact_edges(table: &ContactTable) -> ([usize; 2], [usize; 2]) {
    let mut plants = [0usize; 2];
    let mut lifts = [0usize; 2];
    let mut down = [false; 2];
    for contact in &table.contacts {
        let foot = usize::from(contact.foot);
        if contact.down && !down[foot] {
            plants[foot] += 1;
        }
        if !contact.down && down[foot] {
            lifts[foot] += 1;
        }
        down[foot] = contact.down;
    }
    (plants, lifts)
}

fn joint_named(skeleton: &Skeleton, name: &str) -> usize {
    skeleton
        .joints
        .iter()
        .position(|joint| joint.name == name)
        .unwrap_or_else(|| panic!("{name} is not a joint"))
}

#[test]
fn retarget_soma_cooks_the_fighter_pose_twice_with_eight_contact_windows() {
    let source = soma_skeleton();
    let pairs = soma_pairs();
    let fighter = fighter_skeleton(&pairs);
    assert_eq!(source.len(), 77, "SOMA rig");
    assert_eq!(fighter.len(), 65, "fighter rig");

    let fighter_bind = fighter_bind(&fighter);
    let source_bind = soma_bind(&source, &pairs, &fighter_bind);
    let map = BoneMap::build(&source, &fighter, &pairs).expect("every pair resolves");
    assert_eq!(map.len(), 65, "one map row per fighter joint");
    assert!(
        (0..65).all(|slot| map.source_of(slot).is_some()),
        "every fighter joint is mapped"
    );

    // The offset-chain traps route through the renames, not plain names.
    for (target, soma_name) in [
        ("leftupleg", "leftleg"),
        ("leftleg", "leftshin"),
        ("spine1", "spine2"),
        ("spine2", "chest"),
        ("rightupleg", "rightleg"),
        ("rightleg", "rightshin"),
    ] {
        let slot = joint_named(&fighter, target);
        let origin = map.source_of(slot).expect("trap is mapped");
        assert_eq!(
            source.joints[origin].name, soma_name,
            "{target} reads the wrong SOMA joint"
        );
    }

    let clip = clip(&source, &source_bind);
    let config = CookConfig {
        root_joint: joint_named(&fighter, "hips"),
        left_foot: joint_named(&fighter, "leftfoot"),
        right_foot: joint_named(&fighter, "rightfoot"),
        ground_y: GROUND_Y,
        start_tick: 0,
    };
    let plan = RetargetPlan {
        source_clip: &clip,
        source: &source,
        source_bind: &source_bind,
        target: &fighter,
        target_bind: &fighter_bind,
        map: &map,
        config: &config,
    };
    let first = retarget_cook(&plan, ClipTags::default()).expect("first cook");
    let second = retarget_cook(&plan, ClipTags::default()).expect("second cook");
    assert_eq!(first, second, "the cook repeats bit for bit");
    assert_eq!(
        retarget_digest(&first),
        retarget_digest(&second),
        "the retarget digest repeats"
    );
    assert_eq!(first.root.deltas.len(), 60, "one second at 60 Hz");

    let world = skinning_matrices(&fighter, &pose_at_zero(&first.clip)).expect("pose validates");
    assert_eq!(world.len(), FIGHTER_POSE.len());
    for (slot, (name, expected)) in FIGHTER_POSE.iter().enumerate() {
        assert_eq!(fighter.joints[slot].name, *name, "pose row {slot}");
        let landed = world[slot][3];
        let delta = [
            landed[0] - expected[0],
            landed[1] - expected[1],
            landed[2] - expected[2],
        ];
        let distance = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
        assert!(
            distance <= 0.02,
            "{name} lands {distance:.4} m off: [{}, {}, {}] vs {expected:?}",
            landed[0],
            landed[1],
            landed[2]
        );
    }

    // A bind-relative rotation rides the target quat lane unchanged.
    let arm = joint_named(&fighter, "leftarm");
    let swing = first.clip.rotations[arm].values[1];
    for axis in 0..4 {
        assert!(
            (swing[axis] - SWING[axis]).abs() <= 1e-4,
            "left arm key 1 axis {axis}: {} vs {}",
            swing[axis],
            SWING[axis]
        );
    }

    let (plants, lifts) = contact_edges(&first.contacts);
    assert_eq!(first.contacts.contacts.len(), 120, "two feet over 60 ticks");
    assert_eq!(plants, [4, 4], "four plants per foot");
    assert_eq!(lifts, [4, 4], "four lifts per foot");
    assert_eq!(plants[0] + plants[1], 8, "eight contact windows");
}

#[test]
fn retarget_soma_map_covers_52_exact_13_renamed_12_unused() {
    let source = soma_skeleton();
    let pairs = soma_pairs();
    let fighter = fighter_skeleton(&pairs);
    assert_eq!(coverage(), (52, 13, 12));
    assert_eq!(pairs.len(), fighter.len());
    assert_eq!(pairs.len(), 65);
    let mapped: Vec<&str> = pairs.iter().map(|(_, source)| *source).collect();
    for unused in UNUSED_SOMA {
        assert!(
            !mapped.contains(&unused),
            "{unused} is listed unused but mapped"
        );
        assert_eq!(
            source
                .joints
                .iter()
                .filter(|joint| joint.name == unused)
                .count(),
            1,
            "{unused} exists once in SOMA"
        );
    }
    for (target, soma_name) in RENAMES {
        assert!(
            pairs
                .iter()
                .any(|(name, origin)| *name == target && *origin == soma_name),
            "{target} <- {soma_name} is missing from the pair list"
        );
    }
}

#[test]
fn retarget_soma_unknown_pair_name_is_a_missing_bone() {
    let source = soma_skeleton();
    let fighter = fighter_skeleton(&soma_pairs());
    let mut pairs = soma_pairs();
    pairs.push(("leftthigh", "leftleg"));
    assert_eq!(
        BoneMap::build(&source, &fighter, &pairs),
        Err(RetargetError::MissingBone {
            name: "leftthigh".to_string()
        })
    );
    pairs.pop();
    pairs.push(("leftupleg", "leftfemur"));
    assert_eq!(
        BoneMap::build(&source, &fighter, &pairs),
        Err(RetargetError::MissingBone {
            name: "leftfemur".to_string()
        })
    );
    pairs.pop();
    assert!(
        BoneMap::build(&source, &fighter, &pairs).is_ok(),
        "the untouched pair list still builds"
    );
}
