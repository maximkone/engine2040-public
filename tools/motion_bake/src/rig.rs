//! rig: the two skeletons a bake stands between.
//!
//! Source: SOMA-77, the teacher's skeleton, from `engine_assets::soma::SOMA77`
//! (names and parents) plus [`SOMA_REST`], the bake's own rest offsets in
//! meters. Target: the fighter-65 rig the anim contract names, derived from
//! `soma_pairs()` so there is exactly one name table. The fighter keeps the
//! SOMA proportions except the ankle bone, which is [`FOOT_STRETCH`] longer,
//! so the retarget bind-scale path (a non-1.0 bone-length ratio) is exercised
//! on the lanes the slip meter reads. A real fighter rig profile is the
//! follow-up; until one lands in `engine-assets` the bake ships its own.
//!
//! Determinism: fixed tables, `+ - * /` and `sqrt` only, no trig: the
//! half-angle quats are built from the small-angle vector form, which is
//! exact for the angles a stagger uses and identical on every CPU.

use engine_anim::contract::{Joint, Skeleton};
use engine_assets::retarget::BindPose;
use engine_assets::soma::{soma_pairs, SOMA77};

/// Fighter ankle-bone stretch over SOMA: the bind scale the bake proves.
pub const FOOT_STRETCH: f32 = 1.2;

/// SOMA-77 rest offsets in meters, parent-relative, in `SOMA77` order. The 65
/// mapped rows mirror the AM26 cook fixture's fighter bind (`retarget_soma.rs`
/// `FIGHTER_BIND`); the 12 SOMA-only rows (jaw, eyes, the second neck joint
/// and the eight finger tips) are authored here so the teacher FK has a full
/// body. A rig profile in `engine-assets` replaces this table later.
pub static SOMA_REST: [(&str, [f32; 3]); 77] = [
    ("hips", [0.00, 0.95, 0.00]),
    ("spine1", [0.00, 0.07, 0.00]),
    ("spine2", [0.00, 0.07, 0.00]),
    ("chest", [0.00, 0.07, 0.02]),
    ("neck1", [0.00, 0.10, 0.00]),
    ("neck2", [0.00, 0.03, 0.00]),
    ("head", [0.00, 0.05, 0.00]),
    ("headend", [0.00, 0.12, 0.00]),
    ("jaw", [0.00, -0.02, 0.06]),
    ("lefteye", [0.03, 0.02, 0.07]),
    ("righteye", [-0.03, 0.02, 0.07]),
    ("leftshoulder", [0.08, 0.04, 0.00]),
    ("leftarm", [0.10, 0.00, 0.00]),
    ("leftforearm", [0.28, 0.00, 0.00]),
    ("lefthand", [0.26, 0.00, 0.00]),
    ("lefthandthumb1", [0.02, -0.02, 0.06]),
    ("lefthandthumb2", [0.03, -0.02, 0.02]),
    ("lefthandthumb3", [0.02, -0.01, 0.02]),
    ("lefthandthumbend", [0.02, -0.01, 0.02]),
    ("lefthandindex1", [0.04, 0.00, -0.02]),
    ("lefthandindex2", [0.04, 0.00, 0.00]),
    ("lefthandindex3", [0.03, 0.00, 0.00]),
    ("lefthandindex4", [0.02, 0.00, 0.00]),
    ("lefthandindexend", [0.015, 0.00, 0.00]),
    ("lefthandmiddle1", [0.04, 0.00, 0.00]),
    ("lefthandmiddle2", [0.05, 0.00, 0.00]),
    ("lefthandmiddle3", [0.04, 0.00, 0.00]),
    ("lefthandmiddle4", [0.03, 0.00, 0.00]),
    ("lefthandmiddleend", [0.015, 0.00, 0.00]),
    ("lefthandring1", [0.04, 0.00, 0.02]),
    ("lefthandring2", [0.04, 0.00, 0.00]),
    ("lefthandring3", [0.03, 0.00, 0.00]),
    ("lefthandring4", [0.03, 0.00, 0.00]),
    ("lefthandringend", [0.015, 0.00, 0.00]),
    ("lefthandpinky1", [0.03, 0.00, 0.04]),
    ("lefthandpinky2", [0.03, 0.00, 0.00]),
    ("lefthandpinky3", [0.02, 0.00, 0.00]),
    ("lefthandpinky4", [0.02, 0.00, 0.00]),
    ("lefthandpinkyend", [0.015, 0.00, 0.00]),
    ("rightshoulder", [-0.08, 0.04, 0.00]),
    ("rightarm", [-0.10, 0.00, 0.00]),
    ("rightforearm", [-0.28, 0.00, 0.00]),
    ("righthand", [-0.26, 0.00, 0.00]),
    ("righthandthumb1", [-0.02, -0.02, 0.06]),
    ("righthandthumb2", [-0.03, -0.02, 0.02]),
    ("righthandthumb3", [-0.02, -0.01, 0.02]),
    ("righthandthumbend", [-0.02, -0.01, 0.02]),
    ("righthandindex1", [-0.04, 0.00, -0.02]),
    ("righthandindex2", [-0.04, 0.00, 0.00]),
    ("righthandindex3", [-0.03, 0.00, 0.00]),
    ("righthandindex4", [-0.02, 0.00, 0.00]),
    ("righthandindexend", [-0.015, 0.00, 0.00]),
    ("righthandmiddle1", [-0.04, 0.00, 0.00]),
    ("righthandmiddle2", [-0.05, 0.00, 0.00]),
    ("righthandmiddle3", [-0.04, 0.00, 0.00]),
    ("righthandmiddle4", [-0.03, 0.00, 0.00]),
    ("righthandmiddleend", [-0.015, 0.00, 0.00]),
    ("righthandring1", [-0.04, 0.00, 0.02]),
    ("righthandring2", [-0.04, 0.00, 0.00]),
    ("righthandring3", [-0.03, 0.00, 0.00]),
    ("righthandring4", [-0.03, 0.00, 0.00]),
    ("righthandringend", [-0.015, 0.00, 0.00]),
    ("righthandpinky1", [-0.03, 0.00, 0.04]),
    ("righthandpinky2", [-0.03, 0.00, 0.00]),
    ("righthandpinky3", [-0.02, 0.00, 0.00]),
    ("righthandpinky4", [-0.02, 0.00, 0.00]),
    ("righthandpinkyend", [-0.015, 0.00, 0.00]),
    ("leftleg", [0.08, -0.05, 0.00]),
    ("leftshin", [0.01, -0.40, 0.00]),
    ("leftfoot", [0.00, -0.42, 0.00]),
    ("lefttoebase", [0.00, -0.02, 0.10]),
    ("lefttoeend", [0.00, 0.00, 0.08]),
    ("rightleg", [-0.08, -0.05, 0.00]),
    ("rightshin", [-0.01, -0.40, 0.00]),
    ("rightfoot", [0.00, -0.42, 0.00]),
    ("righttoebase", [0.00, -0.02, 0.10]),
    ("righttoeend", [0.00, 0.00, 0.08]),
];

/// Joint index by name, `None` when the skeleton has no such joint.
pub fn joint_index(skeleton: &Skeleton, name: &str) -> Option<usize> {
    skeleton.joints.iter().position(|joint| joint.name == name)
}

/// The teacher skeleton: SOMA-77 joints in table order, parents resolved by
/// name so a table edit cannot reorder a chain silently.
pub fn soma_skeleton() -> Skeleton {
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

/// The teacher bind: [`SOMA_REST`] offsets in meters, identity rotations.
pub fn soma_offsets() -> BindPose {
    assert_eq!(SOMA_REST.len(), SOMA77.len(), "rest rows match SOMA");
    BindPose {
        positions: SOMA_REST.iter().map(|(_, offset)| *offset).collect(),
        rotations: vec![[0.0, 0.0, 0.0, 1.0]; SOMA_REST.len()],
    }
}

/// The teacher bind in world space: the summed offsets of [`SOMA_REST`] from
/// the rest root, identity rotations. This is what a retarget of world-space
/// teacher joints reads: the bind a world delta is taken from, and the source
/// bone lengths the bind scale divides by.
pub fn soma_bind() -> BindPose {
    let skeleton = soma_skeleton();
    BindPose {
        positions: rest_world(&skeleton, &soma_offsets()),
        rotations: vec![[0.0, 0.0, 0.0, 1.0]; skeleton.len()],
    }
}

/// SOMA joint index by name.
pub fn soma_index(name: &str) -> usize {
    SOMA77
        .iter()
        .position(|(soma, _)| *soma == name)
        .unwrap_or_else(|| panic!("soma has no {name}"))
}

/// The fighter skeleton: one joint per `soma_pairs()` row, parent-first. A
/// SOMA parent with no fighter joint (the second neck joint) is skipped, so
/// the fighter head hangs from the neck.
pub fn fighter_skeleton() -> Skeleton {
    let pairs = soma_pairs();
    let mut joints: Vec<Joint> = Vec::with_capacity(pairs.len());
    for &(target, soma) in &pairs {
        let mut ancestor = SOMA77[soma_index(soma)].1;
        let parent = loop {
            let Some(name) = ancestor else {
                break None;
            };
            match pairs.iter().position(|(_, source)| *source == name) {
                Some(index) => break Some(index),
                None => ancestor = SOMA77[soma_index(name)].1,
            }
        };
        joints.push(Joint {
            name: target.to_string(),
            parent,
        });
    }
    Skeleton { joints }
}

/// The fighter bind: the SOMA rest offset of each mapped joint, with the two
/// ankles stretched by [`FOOT_STRETCH`] so their bind scale is not 1.0.
pub fn fighter_bind() -> BindPose {
    let positions = soma_pairs()
        .iter()
        .map(|(target, soma)| {
            let mut offset = SOMA_REST[soma_index(soma)].1;
            if *target == "leftfoot" || *target == "rightfoot" {
                offset[1] *= FOOT_STRETCH;
            }
            offset
        })
        .collect();
    BindPose {
        positions,
        rotations: vec![[0.0, 0.0, 0.0, 1.0]; soma_pairs().len()],
    }
}

/// World rest positions for a skeleton whose bind rotations are identity: the
/// sum of offsets along each chain.
pub fn rest_world(skeleton: &Skeleton, bind: &BindPose) -> Vec<[f32; 3]> {
    let mut world = vec![[0.0f32; 3]; skeleton.len()];
    for (index, joint) in skeleton.joints.iter().enumerate() {
        let local = bind.positions[index];
        world[index] = match joint.parent {
            None => local,
            Some(parent) => [
                world[parent][0] + local[0],
                world[parent][1] + local[1],
                world[parent][2] + local[2],
            ],
        };
    }
    world
}

/// A unit quaternion for a turn of `angle` radians about `axis`, built from
/// the small-angle vector form (`w = sqrt(1 - |v|^2)`), so no trig runs and
/// the value is identical on every CPU. Good for the sub-radian turns a
/// teacher clip uses; a larger turn is normalized by the retarget cook.
pub fn turn(axis: [f32; 3], angle: f32) -> [f32; 4] {
    let norm = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
    if norm <= 0.0 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let half = angle * 0.5;
    let x = axis[0] / norm * half;
    let y = axis[1] / norm * half;
    let z = axis[2] / norm * half;
    let w = (1.0 - (x * x + y * y + z * z)).max(0.0).sqrt();
    [x, y, z, w]
}

/// Rotation matrix of a quaternion `[x, y, z, w]` (unit up to f32 error).
pub fn quat_to_mat3(q: [f32; 4]) -> [[f32; 3]; 3] {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - w * z),
            2.0 * (x * z + w * y),
        ],
        [
            2.0 * (x * y + w * z),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - w * x),
        ],
        [
            2.0 * (x * z - w * y),
            2.0 * (y * z + w * x),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ]
}

/// Quaternion `[x, y, z, w]` of a rotation matrix, Shepperd's branch order,
/// normalized; a non-rotation matrix would still read as a unit quat because
/// the caller only feeds `quat_to_mat3` output back in.
pub fn mat3_to_quat(m: [[f32; 3]; 3]) -> [f32; 4] {
    let trace = m[0][0] + m[1][1] + m[2][2];
    let q = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [
            (m[2][1] - m[1][2]) / s,
            (m[0][2] - m[2][0]) / s,
            (m[1][0] - m[0][1]) / s,
            0.25 * s,
        ]
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
        [
            0.25 * s,
            (m[0][1] + m[1][0]) / s,
            (m[0][2] + m[2][0]) / s,
            (m[2][1] - m[1][2]) / s,
        ]
    } else if m[1][1] > m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
        [
            (m[0][1] + m[1][0]) / s,
            0.25 * s,
            (m[1][2] + m[2][1]) / s,
            (m[0][2] - m[2][0]) / s,
        ]
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
        [
            (m[0][2] + m[2][0]) / s,
            (m[1][2] + m[2][1]) / s,
            0.25 * s,
            (m[1][0] - m[0][1]) / s,
        ]
    };
    let norm = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if norm > 0.0 {
        [q[0] / norm, q[1] / norm, q[2] / norm, q[3] / norm]
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rest_rows_follow_the_soma_table() {
        assert_eq!(SOMA_REST.len(), 77);
        for (row, (name, parent)) in SOMA_REST.iter().zip(SOMA77.iter()) {
            assert_eq!(row.0, *name, "rest row order");
            let _ = parent;
        }
        let source = soma_skeleton();
        assert_eq!(source.len(), 77);
        let fighter = fighter_skeleton();
        assert_eq!(fighter.len(), 65);
        let bind = fighter_bind();
        assert_eq!(bind.positions.len(), 65);
        // The stance: the ankle rests 8 cm over the ground at SOMA's lengths.
        let world = soma_bind();
        let ankle = world.positions[soma_index("leftfoot")];
        assert!((ankle[1] - 0.08).abs() < 1e-6, "ankle {ankle:?}");
        let hips = world.positions[soma_index("hips")];
        assert!((hips[1] - 0.95).abs() < 1e-6, "hips {hips:?}");
        // The fighter ankle bone is 1.2x SOMA's, so the bind scale is not 1.
        let foot = bind.positions[joint_index(&fighter, "leftfoot").expect("foot")];
        assert!((foot[1] + 0.504).abs() < 1e-6, "foot bind {foot:?}");
    }

    #[test]
    fn turn_and_matrix_round_trip() {
        for (axis, angle) in [
            ([0.0, 1.0, 0.0], 0.14),
            ([1.0, 0.0, 0.0], -0.2),
            ([0.0, 0.0, 1.0], 0.05),
        ] {
            let q = turn(axis, angle);
            let norm = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
            assert!((norm - 1.0).abs() < 1e-6, "unit turn {q:?}");
            let back = mat3_to_quat(quat_to_mat3(q));
            for lane in 0..4 {
                assert!(
                    (back[lane] - q[lane]).abs() < 1e-5,
                    "lane {lane}: {} vs {}",
                    back[lane],
                    q[lane]
                );
            }
            // The matrix turns +Z toward +X for a +Y turn (right-handed).
            if axis == [0.0, 1.0, 0.0] {
                let m = quat_to_mat3(q);
                assert!(m[0][2] > 0.0, "yaw sends +Z toward +X");
            }
        }
        assert_eq!(turn([0.0, 0.0, 0.0], 1.0), [0.0, 0.0, 0.0, 1.0]);
    }
}
