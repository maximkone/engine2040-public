//! an1: a glTF clip pose remapped onto the AN1 body rig and composed into one
//! row-vector skin matrix per body joint (`inverse_bind * joint_world`). The
//! 43 skinned bones map by name; the armature and mesh nodes keep the bind
//! local and their identity inverse binds.

use crate::contract::{
    skinning_matrices, validate_pose, validate_skeleton, AnimError, LocalPose, Skeleton,
};
use engine_core::mat4;

/// One source-pose row per AN1 joint; `None` is a body node the clip rig does
/// not have and that takes the bind local.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JointRemap {
    an1_to_source: Vec<Option<usize>>,
}

impl JointRemap {
    /// Map every AN1 joint to the same-named source joint. The 43 skinned
    /// bones land one to one; a body-only node reads `None`.
    pub fn by_name(source: &Skeleton, an1: &Skeleton) -> Result<Self, AnimError> {
        validate_skeleton(source)?;
        validate_skeleton(an1)?;
        let rows = an1
            .joints
            .iter()
            .map(|joint| {
                source
                    .joints
                    .iter()
                    .position(|origin| origin.name == joint.name)
            })
            .collect();
        Ok(Self {
            an1_to_source: rows,
        })
    }

    /// Row count; always the AN1 joint count for built maps.
    pub fn len(&self) -> usize {
        self.an1_to_source.len()
    }

    /// True when the map carries no rows.
    pub fn is_empty(&self) -> bool {
        self.an1_to_source.is_empty()
    }

    /// Joints with a source row.
    pub fn mapped(&self) -> usize {
        self.an1_to_source.iter().flatten().count()
    }

    /// Source pose joint for an AN1 joint, `None` when it takes the bind.
    pub fn source_of(&self, an1_joint: usize) -> Option<usize> {
        self.an1_to_source.get(an1_joint).copied().flatten()
    }
}

/// AN1 skin failures: a map built for another rig, a pose too short for a
/// mapped row, or a bad inverse-bind table.
#[derive(Clone, PartialEq, Debug)]
pub enum An1Error {
    /// The map rows differ from the AN1 skeleton.
    RemapLength {
        /// AN1 joint count.
        expected: usize,
        /// Map row count.
        got: usize,
    },
    /// A mapped source row lies past the pose.
    SourceJoint {
        /// AN1 joint carrying the mapped row.
        joint: usize,
        /// Source pose row.
        source: usize,
        /// Source pose row count.
        len: usize,
    },
    /// A pose or bind failed the anim contract.
    Pose(AnimError),
    /// One inverse bind per AN1 joint is required.
    InverseBindCount {
        /// AN1 joint count.
        expected: usize,
        /// Inverse-bind row count.
        got: usize,
    },
    /// A non-finite inverse-bind cell; joint carried.
    NonFiniteInverseBind(usize),
}

impl From<AnimError> for An1Error {
    fn from(error: AnimError) -> Self {
        Self::Pose(error)
    }
}

/// Build the AN1-order pose: mapped rows copy the source pose, unmapped rows
/// take the AN1 bind local.
pub fn remap_pose(
    remap: &JointRemap,
    an1: &Skeleton,
    bind: &LocalPose,
    pose: &LocalPose,
) -> Result<LocalPose, An1Error> {
    if remap.len() != an1.len() {
        return Err(An1Error::RemapLength {
            expected: an1.len(),
            got: remap.len(),
        });
    }
    validate_pose(an1, bind)?;
    let mut positions = Vec::with_capacity(an1.len());
    let mut rotations = Vec::with_capacity(an1.len());
    for (joint, source) in remap.an1_to_source.iter().enumerate() {
        match source {
            Some(source) => {
                let (Some(position), Some(rotation)) =
                    (pose.positions.get(*source), pose.rotations.get(*source))
                else {
                    return Err(An1Error::SourceJoint {
                        joint,
                        source: *source,
                        len: pose.positions.len().min(pose.rotations.len()),
                    });
                };
                positions.push(*position);
                rotations.push(*rotation);
            }
            None => {
                positions.push(bind.positions[joint]);
                rotations.push(bind.rotations[joint]);
            }
        }
    }
    Ok(LocalPose {
        positions,
        rotations,
    })
}

/// Compose one AN1 row-vector skin matrix per joint:
/// `inverse_bind * joint_world`, so the bind pose reads identity.
pub fn an1_skinning_matrices(
    an1: &Skeleton,
    pose: &LocalPose,
    inverse_binds: &[[[f32; 4]; 4]],
) -> Result<Vec<[[f32; 4]; 4]>, An1Error> {
    if inverse_binds.len() != an1.len() {
        return Err(An1Error::InverseBindCount {
            expected: an1.len(),
            got: inverse_binds.len(),
        });
    }
    for (joint, bind) in inverse_binds.iter().enumerate() {
        if bind.iter().flatten().any(|value| !value.is_finite()) {
            return Err(An1Error::NonFiniteInverseBind(joint));
        }
    }
    let world = skinning_matrices(an1, pose)?;
    Ok(inverse_binds
        .iter()
        .zip(world.iter())
        .map(|(inverse, joint)| mat4::mul(inverse, joint))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::Joint;

    /// Skinned bones in the shipped CT7 rig; the body adds the armature and
    /// mesh nodes beside them.
    const BONES: usize = 43;

    /// A unit yaw quaternion from a rational half-angle pair (`x` over `w`):
    /// the fixture needs no libm rounding, only a normalised `[x, 0, 0, w]`.
    fn tilt(x: f32, w: f32) -> [f32; 4] {
        let len = (x * x + w * w).sqrt();
        [0.0, x / len, 0.0, w / len]
    }

    fn identity() -> [[f32; 4]; 4] {
        let mut out = [[0.0f32; 4]; 4];
        for (i, row) in out.iter_mut().enumerate() {
            row[i] = 1.0;
        }
        out
    }

    /// Inverse of an AN1 row-vector affine matrix: transpose the basis rows,
    /// then undo the translation in the new basis.
    fn inverse_affine(m: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
        let mut out = identity();
        for (i, row) in out.iter_mut().enumerate().take(3) {
            *row = [m[0][i], m[1][i], m[2][i], 0.0];
        }
        let t = m[3];
        out[3] = [
            -(t[0] * out[0][0] + t[1] * out[1][0] + t[2] * out[2][0]),
            -(t[0] * out[0][1] + t[1] * out[1][1] + t[2] * out[2][1]),
            -(t[0] * out[0][2] + t[1] * out[1][2] + t[2] * out[2][2]),
            1.0,
        ];
        out
    }

    /// The shipped rig's shape at a small scale: one armature root, the mesh
    /// node beside it, and 43 bones in a shallow tree, so a bind world row
    /// stays near one meter as a fighter's does.
    struct Rig {
        an1: Skeleton,
        clip: Skeleton,
        bind: LocalPose,
        clip_bind: LocalPose,
        inverse_binds: Vec<[[f32; 4]; 4]>,
    }

    /// The local offset of a branch bone, by its slot in the four-way fork.
    const FORK: [[f32; 3]; 4] = [
        [0.05, 0.08, 0.0],
        [-0.05, 0.08, 0.0],
        [0.0, 0.08, 0.05],
        [0.0, 0.08, -0.05],
    ];

    /// The bone a branch hangs on: a three-joint spine, then four-way forks.
    fn origin_of(bone: usize) -> usize {
        match bone {
            0..=2 => bone.saturating_sub(1),
            _ => 2 + (bone - 3) / 4,
        }
    }

    fn rig() -> Rig {
        let mut an1 = Skeleton::default();
        let mut clip = Skeleton::default();
        an1.joints.push(Joint {
            name: "human_male".to_string(),
            parent: None,
        });
        clip.joints.push(Joint {
            name: "human_male".to_string(),
            parent: None,
        });
        an1.joints.push(Joint {
            name: "body_fighter".to_string(),
            parent: Some(0),
        });
        for bone in 0..BONES {
            let name = format!("bone_{bone:02}");
            // Bone 0 hangs on the armature root, which both rigs carry; the
            // rest on an earlier bone, so every row lists its parent first.
            let origin = origin_of(bone);
            an1.joints.push(Joint {
                name: name.clone(),
                parent: Some(if bone == 0 { 0 } else { 2 + origin }),
            });
            clip.joints.push(Joint {
                name,
                parent: Some(if bone == 0 { 0 } else { 1 + origin }),
            });
        }
        let mut positions = vec![[0.0f32; 3]; an1.len()];
        let mut rotations = vec![[0.0f32, 0.0, 0.0, 1.0]; an1.len()];
        for bone in 0..BONES {
            let joint = 2 + bone;
            positions[joint] = if bone == 0 {
                [0.0, 0.9, 0.0]
            } else {
                FORK[bone % FORK.len()]
            };
            let lean = (1 + (bone % 3)) as f32;
            let sign = if bone % 2 == 0 { 1.0 } else { -1.0 };
            rotations[joint] = tilt(sign * lean, 40.0);
        }
        let bind = LocalPose {
            positions,
            rotations,
        };
        // The clip rig is the AN1 rig without the mesh node at row 1.
        let keep: Vec<usize> = (0..an1.len()).filter(|joint| *joint != 1).collect();
        let clip_bind = LocalPose {
            positions: keep.iter().map(|joint| bind.positions[*joint]).collect(),
            rotations: keep.iter().map(|joint| bind.rotations[*joint]).collect(),
        };
        let worlds = skinning_matrices(&an1, &bind).expect("bind composes");
        let inverse_binds = worlds
            .iter()
            .enumerate()
            .map(|(joint, world)| {
                if joint < 2 {
                    identity()
                } else {
                    inverse_affine(world)
                }
            })
            .collect();
        Rig {
            an1,
            clip,
            bind,
            clip_bind,
            inverse_binds,
        }
    }

    fn bits(matrices: &[[[f32; 4]; 4]]) -> Vec<u32> {
        matrices
            .iter()
            .flatten()
            .flatten()
            .map(|value| value.to_bits())
            .collect()
    }

    #[test]
    fn the_remap_is_a_bijection_over_43_bones_and_two_nodes_stay_identity() {
        let rig = rig();
        let remap = JointRemap::by_name(&rig.clip, &rig.an1).expect("same-name map");
        assert_eq!(remap.len(), BONES + 2);
        assert_eq!(remap.mapped(), BONES + 1, "43 bones plus the shared root");
        assert_eq!(remap.source_of(1), None, "the mesh node has no clip row");
        for joint in 2..rig.an1.len() {
            let source = remap.source_of(joint).expect("a bone row");
            assert_eq!(rig.clip.joints[source].name, rig.an1.joints[joint].name);
        }
        let mut sources: Vec<usize> = (0..remap.len())
            .filter_map(|j| remap.source_of(j))
            .collect();
        sources.sort_unstable();
        sources.dedup();
        assert_eq!(sources.len(), BONES + 1, "no source row drives two joints");
        assert_eq!(rig.inverse_binds[0], identity(), "armature bind");
        assert_eq!(rig.inverse_binds[1], identity(), "mesh node bind");
    }

    #[test]
    fn the_bind_pose_reads_identity_times_inverse_bind() {
        let rig = rig();
        let remap = JointRemap::by_name(&rig.clip, &rig.an1).expect("same-name map");
        let pose = remap_pose(&remap, &rig.an1, &rig.bind, &rig.clip_bind).expect("bind remaps");
        assert_eq!(pose, rig.bind, "the clip bind is the AN1 bind row for row");
        let matrices =
            an1_skinning_matrices(&rig.an1, &pose, &rig.inverse_binds).expect("bind matrices");
        assert_eq!(matrices.len(), rig.an1.len());
        for (joint, matrix) in matrices.iter().enumerate() {
            for (row, values) in matrix.iter().enumerate() {
                for (cell, value) in values.iter().enumerate() {
                    let want = if row == cell { 1.0 } else { 0.0 };
                    assert!(
                        (value - want).abs() < 1e-5,
                        "bind[{joint}][{row}][{cell}] = {value}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_tick_gives_byte_identical_matrices_twice() {
        let rig = rig();
        let remap = JointRemap::by_name(&rig.clip, &rig.an1).expect("same-name map");
        let mut posed = rig.clip_bind.clone();
        posed.positions[1][0] += 0.25;
        for row in [3usize, 8, 20] {
            posed.rotations[row] = tilt(1.0, 4.0);
        }
        let pose = remap_pose(&remap, &rig.an1, &rig.bind, &posed).expect("tick remaps");
        let first = an1_skinning_matrices(&rig.an1, &pose, &rig.inverse_binds).expect("tick");
        let second = an1_skinning_matrices(&rig.an1, &pose, &rig.inverse_binds).expect("tick");
        assert_eq!(
            bits(&first),
            bits(&second),
            "the same tick gives the same bits"
        );
        let bind = an1_skinning_matrices(&rig.an1, &rig.bind, &rig.inverse_binds).expect("bind");
        assert_ne!(bits(&first), bits(&bind), "the posed tick moved the chain");
        assert_eq!(
            first[1],
            identity(),
            "the mesh node keeps the identity bind"
        );
    }

    #[test]
    fn a_short_pose_row_is_refused_with_its_row() {
        let rig = rig();
        let remap = JointRemap::by_name(&rig.clip, &rig.an1).expect("same-name map");
        let short = LocalPose {
            positions: rig.clip_bind.positions[..10].to_vec(),
            rotations: rig.clip_bind.rotations[..10].to_vec(),
        };
        let err = remap_pose(&remap, &rig.an1, &rig.bind, &short).expect_err("short pose");
        assert!(matches!(
            err,
            An1Error::SourceJoint {
                joint: 11,
                source: 10,
                len: 10
            }
        ));
    }

    #[test]
    fn a_bad_inverse_bind_table_is_refused() {
        let rig = rig();
        let short = &rig.inverse_binds[..rig.an1.len() - 1];
        assert!(matches!(
            an1_skinning_matrices(&rig.an1, &rig.bind, short),
            Err(An1Error::InverseBindCount { expected, got })
                if expected == rig.an1.len() && got == rig.an1.len() - 1
        ));
        let mut broken = rig.inverse_binds.clone();
        broken[7][2][3] = f32::NAN;
        assert!(matches!(
            an1_skinning_matrices(&rig.an1, &rig.bind, &broken),
            Err(An1Error::NonFiniteInverseBind(7))
        ));
    }

    #[test]
    fn an_unnormalized_source_quat_is_refused_by_its_an1_row() {
        let rig = rig();
        let remap = JointRemap::by_name(&rig.clip, &rig.an1).expect("same-name map");
        let mut posed = rig.clip_bind.clone();
        posed.rotations[3] = [0.0, 2.0, 0.0, 0.0];
        let pose = remap_pose(&remap, &rig.an1, &rig.bind, &posed).expect("rows copy");
        assert_eq!(
            an1_skinning_matrices(&rig.an1, &pose, &rig.inverse_binds),
            Err(An1Error::Pose(AnimError::UnnormalizedQuat(4))),
            "clip row 3 is AN1 row 4"
        );
    }
}
