//! error: typed refusals. A refused call changes nothing.

use engine_anim::contract::AnimError;
use std::fmt::{Display, Formatter};

/// The input a refusal names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// A position or a static box centre.
    Position,
    /// A rotation quaternion.
    Rotation,
    /// A start velocity.
    Velocity,
    /// A blast or hit impulse.
    Impulse,
    /// A shape size: capsule radius, half-height or box half-extent.
    Extent,
    /// The interpolation alpha.
    Alpha,
    /// A flight time in ticks.
    Ticks,
    /// A heading with no direction in it (all zeros).
    Direction,
    /// A joint range or an angle, radians.
    Angle,
    /// A bone index the rig does not have, or the root where a joint is needed.
    Bone,
}

impl Field {
    fn name(self) -> &'static str {
        match self {
            Field::Position => "position",
            Field::Rotation => "rotation",
            Field::Velocity => "velocity",
            Field::Impulse => "impulse",
            Field::Extent => "extent",
            Field::Alpha => "alpha",
            Field::Ticks => "ticks",
            Field::Direction => "direction",
            Field::Angle => "angle",
            Field::Bone => "bone",
        }
    }
}

/// Why a call was refused. Every variant leaves the world exactly as it was.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// NaN or infinity in the named input.
    NonFinite(Field),
    /// Finite, but outside the bounds this layer accepts.
    OutOfRange(Field),
    /// A cap table with a zero limit where one is needed.
    BadCaps,
    /// One spawn alone is larger than the whole cap, so recycling cannot make room for it.
    OverCap {
        /// Bodies (or static boxes) the call needs.
        need: usize,
        /// The cap that cannot hold them.
        cap: usize,
    },
    /// `spawn_debris` was given no shards.
    NoShards,
    /// The skeleton, pose or bone list failed the animation contract.
    Anim(AnimError),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NonFinite(field) => write!(f, "{} is not finite", field.name()),
            Error::OutOfRange(field) => write!(f, "{} is out of range", field.name()),
            Error::BadCaps => f.write_str("cap table needs every limit above zero"),
            Error::OverCap { need, cap } => write!(f, "{need} needed, the cap holds {cap}"),
            Error::NoShards => f.write_str("no shards to spawn"),
            Error::Anim(e) => write!(f, "invalid skeleton, pose or bones: {e:?}"),
        }
    }
}

impl std::error::Error for Error {}
