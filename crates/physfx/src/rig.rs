//! rig: a ragdoll's skeleton and bone shapes, validated once.

use crate::check;
use crate::error::{Error, Field};
use engine_anim::contract::{validate_skeleton, AnimError, Skeleton};
use engine_physics::ragdoll::{joint_depths, BoneShape};
use std::f32::consts::TAU;

/// A joint that bends about one axis, a knee or an elbow, where the other joints are balls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hinge {
    /// The child bone of the joint; the root has no joint to bend.
    pub bone: usize,
    /// Smallest flexion, radians, counted from the straight limb about the body's side axis
    /// (the pelvis X axis): a knee folds the shin backward, which is positive.
    pub min: f32,
    /// Largest flexion, radians, `min` or more.
    pub max: f32,
}

/// A skeleton plus one capsule per bone. Build it once per body type and spawn many ragdolls
/// from it; the joint depths that fix the swing limits are computed here, not per spawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Rig {
    skeleton: Skeleton,
    shapes: Vec<BoneShape>,
    depths: Vec<u32>,
    hinges: Vec<Hinge>,
    /// How hard the joints brace when a blow lands, permille of the whole.
    muscle_permille: u16,
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
            hinges: Vec::new(),
            muscle_permille: crate::body::MUSCLE_FULL,
        })
    }

    /// Sets how hard the joints brace when a blow lands, permille of the whole:
    /// 1000 braces as hard as the motors can, 0 gives way at once. A value past
    /// the whole is refused and nothing is kept.
    pub fn with_muscle_permille(mut self, permille: u16) -> Result<Self, Error> {
        if permille > crate::body::MUSCLE_FULL {
            return Err(Error::OutOfRange(Field::Muscle));
        }
        self.muscle_permille = permille;
        Ok(self)
    }

    /// How hard the joints brace when a blow lands, permille of the whole.
    pub(crate) fn muscle_permille(&self) -> u16 {
        self.muscle_permille
    }

    /// Makes the joints of `hinges` hinges; every other joint stays a ball. A bone that is the
    /// root or not in the rig, or a range that is not finite, reversed or past a turn, is refused
    /// and nothing is kept.
    pub fn with_hinges(mut self, hinges: &[Hinge]) -> Result<Self, Error> {
        for hinge in hinges {
            if hinge.bone >= self.shapes.len() || self.parent(hinge.bone).is_none() {
                return Err(Error::OutOfRange(Field::Bone));
            }
            if !hinge.min.is_finite() || !hinge.max.is_finite() {
                return Err(Error::NonFinite(Field::Angle));
            }
            if hinge.min > hinge.max || hinge.min < -TAU || hinge.max > TAU {
                return Err(Error::OutOfRange(Field::Angle));
            }
        }
        self.hinges = hinges.to_vec();
        Ok(self)
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

    pub(crate) fn hinges(&self) -> &[Hinge] {
        &self.hinges
    }

    /// The parent bone of `bone`, `None` for the root.
    pub(crate) fn parent(&self, bone: usize) -> Option<usize> {
        self.skeleton.joints[bone].parent
    }
}
