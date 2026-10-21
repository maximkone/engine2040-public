//! fighter: the rig the teacher lane lands on, and the teacher's rest pose.
//!
//! Consumer: the offline motion bake retargets a SOMA-77 teacher clip onto
//! the fighter-65 rig through this module, and the tick reads the same joint
//! order when it plays a baked pack. Data owner: `engine-assets`.
//! Source: nv-tlabs/kimodo `kimodo/skeleton/definitions.py`,
//! `SOMASkeleton77.bone_order_names_with_parents` (Apache-2.0); the rest
//! offsets below are the bake's own table, in meters, parent-relative.
//! Determinism: fixed tables, parent-first order, string compares only.

use engine_anim::contract::{Joint, Skeleton};

use crate::retarget::BindPose;
use crate::soma::{soma_pairs, SOMA77};

/// Fighter ankle-bone stretch over SOMA: the bind scale the bake proves.
pub const FOOT_STRETCH: f32 = 1.2;

/// SOMA-77 rest offsets in meters, parent-relative, in `SOMA77` order. The 65
/// mapped rows mirror the AM26 cook fixture's fighter bind (`retarget_soma.rs`
/// `FIGHTER_BIND`); the 12 SOMA-only rows (jaw, eyes, the second neck joint
/// and the eight finger tips) are authored here so the teacher FK has a full
/// body. The table lives in `engine-assets` so the bake and the tick read one
/// rest pose; a measured rig profile replaces it later.
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

/// Why a rig could not be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RigError {
    /// A joint's parent is not in the table before it.
    MissingParent {
        /// The parent that was not there yet.
        parent: String,
        /// The joint that wanted it.
        joint: String,
    },
    /// A joint name is not in the table.
    MissingJoint {
        /// The name that was not in the table.
        name: String,
    },
}

impl std::fmt::Display for RigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RigError::MissingParent { parent, joint } => {
                write!(f, "the parent {parent} of {joint} does not come first")
            }
            RigError::MissingJoint { name } => write!(f, "the rig has no joint {name}"),
        }
    }
}

impl std::error::Error for RigError {}

/// The teacher skeleton: SOMA-77 joints in table order, parents resolved by
/// name so a table edit cannot reorder a chain silently.
pub fn soma_skeleton() -> Result<Skeleton, RigError> {
    let mut joints: Vec<Joint> = Vec::with_capacity(SOMA77.len());
    for &(name, parent) in SOMA77.iter() {
        let parent = match parent {
            None => None,
            Some(parent) => Some(
                joints
                    .iter()
                    .position(|joint| joint.name == parent)
                    .ok_or_else(|| RigError::MissingParent {
                        parent: parent.to_string(),
                        joint: name.to_string(),
                    })?,
            ),
        };
        joints.push(Joint {
            name: name.to_string(),
            parent,
        });
    }
    Ok(Skeleton { joints })
}

/// The teacher bind: [`SOMA_REST`] offsets in meters, identity rotations.
pub fn soma_offsets() -> BindPose {
    BindPose {
        positions: SOMA_REST.iter().map(|(_, offset)| *offset).collect(),
        rotations: vec![[0.0, 0.0, 0.0, 1.0]; SOMA_REST.len()],
    }
}

/// The teacher bind in world space: the summed offsets of [`SOMA_REST`] from
/// the rest root, identity rotations. This is what a retarget of world-space
/// teacher joints reads: the bind a world delta is taken from, and the source
/// bone lengths the bind scale divides by.
pub fn soma_bind() -> Result<BindPose, RigError> {
    let skeleton = soma_skeleton()?;
    Ok(BindPose {
        positions: rest_world(&skeleton, &soma_offsets()),
        rotations: vec![[0.0, 0.0, 0.0, 1.0]; skeleton.len()],
    })
}

/// SOMA joint index by name, `None` when the table has no such joint.
pub fn soma_index(name: &str) -> Option<usize> {
    SOMA77.iter().position(|(soma, _)| *soma == name)
}

/// The fighter skeleton: one joint per `soma_pairs()` row, parent-first. A
/// SOMA parent with no fighter joint (the second neck joint) is skipped, so
/// the fighter head hangs from the neck.
pub fn fighter_skeleton() -> Result<Skeleton, RigError> {
    let pairs = soma_pairs();
    let mut joints: Vec<Joint> = Vec::with_capacity(pairs.len());
    for &(target, soma) in &pairs {
        let mut ancestor = SOMA77[soma_index(soma).ok_or_else(|| RigError::MissingJoint {
            name: soma.to_string(),
        })?]
        .1;
        let parent = loop {
            let Some(name) = ancestor else {
                break None;
            };
            match pairs.iter().position(|(_, source)| *source == name) {
                Some(index) => break Some(index),
                None => {
                    ancestor = SOMA77[soma_index(name).ok_or_else(|| RigError::MissingJoint {
                        name: name.to_string(),
                    })?]
                    .1;
                }
            }
        };
        joints.push(Joint {
            name: target.to_string(),
            parent,
        });
    }
    Ok(Skeleton { joints })
}

/// The fighter bind: the SOMA rest offset of each mapped joint, with the two
/// ankles stretched by [`FOOT_STRETCH`] so their bind scale is not 1.0.
pub fn fighter_bind() -> Result<BindPose, RigError> {
    let positions = soma_pairs()
        .iter()
        .map(|(target, soma)| {
            let row = SOMA_REST
                .iter()
                .find(|(name, _)| *name == *soma)
                .ok_or_else(|| RigError::MissingJoint {
                    name: soma.to_string(),
                })?;
            let mut offset = row.1;
            if *target == "leftfoot" || *target == "rightfoot" {
                offset[1] *= FOOT_STRETCH;
            }
            Ok(offset)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BindPose {
        positions,
        rotations: vec![[0.0, 0.0, 0.0, 1.0]; soma_pairs().len()],
    })
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
