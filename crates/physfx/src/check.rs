//! check: every input is validated here, before a spawn touches the world.

use crate::error::{Error, Field};
use crate::types::{Blast, Transform};
use engine_physics::ragdoll::BoneShape;

/// Largest position component accepted, meters.
pub(crate) const MAX_COORD: f32 = 1_000.0;
/// [`MAX_COORD`] in millimeters, for the integer positions of authority events.
const MAX_COORD_MM: u32 = 1_000_000;
/// Largest blast radius accepted, millimeters (100 m).
const MAX_BLAST_RADIUS_MM: u32 = 100_000;
/// The body a blast impulse is quoted for, kilograms.
pub(crate) const BLAST_REFERENCE_KG: f32 = 70.0;
/// Longest flight accepted, ticks (20 s at 60 Hz): staying up longer needs a launch speed past
/// [`MAX_SPEED`], so a longer one would be refused as too fast anyway.
const MAX_FLIGHT_TICKS: u32 = 1_200;
/// Largest start velocity component accepted, m/s.
pub(crate) const MAX_SPEED: f32 = 100.0;
/// Largest impulse component accepted, newton-seconds.
pub(crate) const MAX_IMPULSE: f32 = 100_000.0;
/// Smallest shape size accepted, meters: Rapier loses contacts on slivers.
const MIN_EXTENT: f32 = 0.01;
/// Largest dynamic shape size accepted, meters.
const MAX_EXTENT: f32 = 5.0;

/// A vector whose every component is finite and within `limit`.
pub(crate) fn vec3(v: [f32; 3], limit: f32, field: Field) -> Result<(), Error> {
    if v.iter().any(|c| !c.is_finite()) {
        Err(Error::NonFinite(field))
    } else if v.iter().any(|c| c.abs() > limit) {
        Err(Error::OutOfRange(field))
    } else {
        Ok(())
    }
}

/// A transform with an in-range position and a rotation that can be made unit length; returns
/// it with the rotation normalized.
pub(crate) fn transform(at: Transform) -> Result<Transform, Error> {
    vec3(at.pos, MAX_COORD, Field::Position)?;
    if at.rot.iter().any(|c| !c.is_finite()) {
        return Err(Error::NonFinite(Field::Rotation));
    }
    let len = at.rot.iter().map(|c| c * c).sum::<f32>().sqrt();
    if len < 1e-6 {
        return Err(Error::OutOfRange(Field::Rotation));
    }
    Ok(Transform {
        pos: at.pos,
        rot: at.rot.map(|c| c / len),
    })
}

/// One dynamic size: finite and within `[MIN_EXTENT, MAX_EXTENT]`.
pub(crate) fn extent(value: f32) -> Result<(), Error> {
    if !value.is_finite() {
        Err(Error::NonFinite(Field::Extent))
    } else if !(MIN_EXTENT..=MAX_EXTENT).contains(&value) {
        Err(Error::OutOfRange(Field::Extent))
    } else {
        Ok(())
    }
}

/// A capsule: the radius is a dynamic size, the half-height may be zero (a sphere).
pub(crate) fn shape(shape: BoneShape) -> Result<(), Error> {
    extent(shape.radius)?;
    if shape.half_height == 0.0 {
        return Ok(());
    }
    extent(shape.half_height)
}

/// A static box: every half-extent finite, above zero and within the world.
pub(crate) fn static_half(half: [f32; 3]) -> Result<(), Error> {
    vec3(half, MAX_COORD, Field::Extent)?;
    if half.iter().any(|c| *c <= 0.0) {
        return Err(Error::OutOfRange(Field::Extent));
    }
    Ok(())
}

/// The velocity an impulse adds to a body of `mass` kilograms. An impulse that would throw the
/// body past [`MAX_SPEED`] is refused: it is a bug upstream, not a blast.
pub(crate) fn velocity_gain(impulse: [f32; 3], mass: f32) -> Result<[f32; 3], Error> {
    let gain = impulse.map(|c| c / mass);
    vec3(gain, MAX_SPEED, Field::Impulse)?;
    Ok(gain)
}

/// A blast with its center inside the world, a reach above zero and within [`MAX_BLAST_RADIUS_MM`],
/// and an impulse that does not throw the reference body past [`MAX_SPEED`].
pub(crate) fn blast(blast: Blast) -> Result<(), Error> {
    if blast
        .center_mm
        .iter()
        .any(|c| c.unsigned_abs() > MAX_COORD_MM)
    {
        return Err(Error::OutOfRange(Field::Position));
    }
    if blast.radius_mm == 0 || blast.radius_mm > MAX_BLAST_RADIUS_MM {
        return Err(Error::OutOfRange(Field::Extent));
    }
    velocity_gain([blast.impulse_ns as f32, 0.0, 0.0], BLAST_REFERENCE_KG)?;
    Ok(())
}

/// A flight time of at least one tick and at most [`MAX_FLIGHT_TICKS`].
pub(crate) fn flight_ticks(ticks: u32) -> Result<(), Error> {
    if (1..=MAX_FLIGHT_TICKS).contains(&ticks) {
        Ok(())
    } else {
        Err(Error::OutOfRange(Field::Ticks))
    }
}
