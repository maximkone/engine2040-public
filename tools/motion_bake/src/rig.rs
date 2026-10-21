//! rig: the math a bake stands between.
//!
//! The two skeletons now live in the engine, in `engine_assets::fighter`:
//! SOMA-77 (the teacher's, from `engine_assets::soma::SOMA77` plus the rest
//! offsets in meters) and the fighter-65 rig the anim contract names, derived
//! from `soma_pairs()` so there is exactly one name table. The fighter keeps
//! the SOMA proportions except the ankle bone, which is 1.2x longer,
//! longer, so the retarget bind-scale path (a non-1.0 bone-length ratio) is
//! exercised on the lanes the slip meter reads. One rig, one joint order: the
//! bake and the tick read it from the same place.
//!
//! Determinism: fixed tables, `+ - * /` and `sqrt` only, no trig: the
//! half-angle quats are built from the small-angle vector form, which is
//! exact for the angles a stagger uses and identical on every CPU.

pub use engine_assets::fighter::{
    fighter_bind, fighter_skeleton, joint_index, rest_world, soma_bind, soma_offsets, soma_skeleton,
};

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
    use engine_assets::fighter::{soma_index, SOMA_REST};
    use engine_assets::soma::SOMA77;

    #[test]
    fn rest_rows_follow_the_soma_table() {
        assert_eq!(SOMA_REST.len(), 77);
        for (row, (name, parent)) in SOMA_REST.iter().zip(SOMA77.iter()) {
            assert_eq!(row.0, *name, "rest row order");
            let _ = parent;
        }
        let source = soma_skeleton().expect("the teacher table is parent-first");
        assert_eq!(source.len(), 77);
        let fighter = fighter_skeleton().expect("the pair table is parent-first");
        assert_eq!(fighter.len(), 65);
        let bind = fighter_bind().expect("every mapped joint has a rest row");
        assert_eq!(bind.positions.len(), 65);
        // The stance: the ankle rests 8 cm over the ground at SOMA's lengths.
        let world = soma_bind().expect("the teacher table is parent-first");
        let ankle = world.positions[soma_index("leftfoot").expect("the foot is in the table")];
        assert!((ankle[1] - 0.08).abs() < 1e-6, "ankle {ankle:?}");
        let hips = world.positions[soma_index("hips").expect("the hips are in the table")];
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
