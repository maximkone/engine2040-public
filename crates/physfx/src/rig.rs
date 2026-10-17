//! rig: a ragdoll's skeleton and bone shapes, validated once.

use crate::check;
use crate::error::Error;
use engine_anim::contract::{validate_skeleton, AnimError, Skeleton};
use engine_physics::ragdoll::{joint_depths, BoneShape};

/// A skeleton plus one capsule per bone. Build it once per body type and spawn many ragdolls
/// from it; the joint depths that fix the swing limits are computed here, not per spawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Rig {
    skeleton: Skeleton,
    shapes: Vec<BoneShape>,
    depths: Vec<u32>,
}

impl Rig {
    /// Validates the skeleton (names, parent-first order) and the shapes (one per bone, sizes in
    /// range). Nothing is kept on failure.
    pub fn new(skeleton: Skeleton, shapes: Vec<BoneShape>) -> Result<Self, Error> {
        validate_skeleton(&skeleton).map_err(Error::Anim)?;
        if shapes.len() != skeleton.len() {
            return Err(Error::Anim(AnimError::LengthMismatch {
                expected: skeleton.len(),
                got: shapes.len(),
            }));
        }
        for shape in &shapes {
            check::shape(*shape)?;
        }
        let depths = joint_depths(&skeleton);
        Ok(Self {
            skeleton,
            shapes,
            depths,
        })
    }

    /// Bodies one ragdoll of this rig takes from the cap.
    pub fn bones(&self) -> usize {
        self.shapes.len()
    }

    pub(crate) fn skeleton(&self) -> &Skeleton {
        &self.skeleton
    }

    pub(crate) fn shapes(&self) -> &[BoneShape] {
        &self.shapes
    }

    pub(crate) fn depths(&self) -> &[u32] {
        &self.depths
    }
}
