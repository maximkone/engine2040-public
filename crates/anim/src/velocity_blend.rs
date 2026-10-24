//! velocity_blend: four damped body-frame movement axes for the loco blend.
//!
//! Owner: `engine-anim`; consumer: the locomotion blend, which asks
//! [`VelocityBlend::space_params`] and hands the pair to
//! [`weights_into`](super::space::weights_into). Presentation only; the sim
//! never reads it.
//!
//! Source: ALS-Refactored `Source/ALS/Private/AlsAnimationInstance.cpp`
//! `RefreshVelocityBlend`
//! (https://github.com/Sixze/ALS-Refactored/blob/main/Source/ALS/Private/AlsAnimationInstance.cpp),
//! donor fab.com/s/0eb8afffb774. The movement direction in the capsule frame is
//! split into four amplitudes (forward, backward, left, right) that each hold
//! the speed fraction on that side, eased with an exact half-life damper, and a
//! first-tick flag snaps them on spawn or teleport instead of easing up from
//! zero (`bInitializationRequired` in the donor).
//!
//! Units: meters per second in, unit-less amplitudes plus Q10 speed and
//! decidegree heading out. No clock, no heap: the caller passes seconds.

use super::space::{SpaceError, SpaceParams};

/// Eased-toward-speed half-life in seconds when the caller passes none.
///
/// chosen: the ALS asset field is `Grounded.VelocityBlendInterpolationHalfLife`,
/// whose shipped default was not read; 0.12 s settles a direction change in
/// about seven ticks at 60 Hz, which reads as a step rather than a slide.
pub const DEFAULT_HALF_LIFE_S: f32 = 0.12;

/// Largest angle error of the linear-ratio heading, in degrees (worst just off
/// 26.6 and 63.4 degrees). The blend space spaces its samples tens of degrees
/// apart, so this never picks the wrong one.
pub const DIAMOND_MAX_ERROR_DEG: f32 = 4.1;

/// Four body-frame amplitudes, each `0.0..=1.0`: the speed fraction moving
/// that way. Zero is at rest.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VelocityBlend {
    forward: f32,
    backward: f32,
    left: f32,
    right: f32,
}

impl VelocityBlend {
    /// At rest: every amplitude zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The target amplitudes for a body-frame velocity `[right, forward]` in
    /// m/s: the sign picks the side, the magnitude clamps to one. Non-finite
    /// input reads as rest.
    #[must_use]
    pub fn target(vel_body_mps: [f32; 2]) -> Self {
        let right = axis(vel_body_mps[0]);
        let left = axis(-vel_body_mps[0]);
        let forward = axis(vel_body_mps[1]);
        let backward = axis(-vel_body_mps[1]);
        Self {
            forward,
            backward,
            left,
            right,
        }
    }

    /// Ease every amplitude toward `target` over `dt_s` with `half_life_s`.
    /// A half-life of zero or less, or a non-finite argument, snaps: a broken
    /// frame never leaves the axes between two poses.
    pub fn step(&mut self, target: Self, half_life_s: f32, dt_s: f32) {
        if !half_life_s.is_finite() || half_life_s <= 0.0 || !dt_s.is_finite() {
            self.snap(target);
            return;
        }
        let alpha = half_life_alpha(dt_s, half_life_s);
        self.forward += (target.forward - self.forward) * alpha;
        self.backward += (target.backward - self.backward) * alpha;
        self.left += (target.left - self.left) * alpha;
        self.right += (target.right - self.right) * alpha;
    }

    /// Take `target` exactly: the spawn and teleport path.
    pub fn snap(&mut self, target: Self) {
        *self = target;
    }

    /// Speed fraction moving forward.
    #[must_use]
    pub fn forward(&self) -> f32 {
        self.forward
    }

    /// Speed fraction moving backward.
    #[must_use]
    pub fn backward(&self) -> f32 {
        self.backward
    }

    /// Speed fraction moving left.
    #[must_use]
    pub fn left(&self) -> f32 {
        self.left
    }

    /// Speed fraction moving right.
    #[must_use]
    pub fn right(&self) -> f32 {
        self.right
    }

    /// The pair the blend space queries: speed in m/s and heading from facing
    /// in degrees (+right), both from the amplitudes.
    ///
    /// The heading is the diamond approximation of `atan2`, never platform
    /// trig, so two runs on two machines pick the same sample. A refusal comes
    /// from [`SpaceParams::new`] when the speed leaves `0..=12`.
    pub fn space_params(&self, speed_scale_mps: f32) -> Result<SpaceParams, SpaceError> {
        let x = self.right - self.left;
        let z = self.forward - self.backward;
        let speed = (x * x + z * z).sqrt() * speed_scale_mps;
        SpaceParams::new(speed, diamond_deg(x, z))
    }
}

/// One side's amplitude: negative m/s is zero on that side, and the magnitude
/// clamps to one so a sprint never outruns the blend's full-scale speed.
fn axis(mps: f32) -> f32 {
    if mps.is_finite() {
        mps.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Fraction of the gap closed in one step of `dt_s` at `half_life_s`.
///
/// chosen: `dt / (dt + half_life)` closes exactly half the gap in one
/// half-life and none at zero, the two points the donor's damper fixes, and it
/// needs no `powf` (the quality ratchet keeps platform float math out of this
/// crate). A long step approaches one and never overshoots.
fn half_life_alpha(dt_s: f32, half_life_s: f32) -> f32 {
    let alpha = dt_s / (dt_s + half_life_s);
    if alpha.is_finite() {
        alpha.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// Heading from facing in degrees, `0` forward and `+90` right, from the
/// linear-ratio approximation of `atan`: within a quadrant the angle is
/// `45 * near / far`. Zero at rest, and every input stays in `-180..=180`.
fn diamond_deg(x: f32, z: f32) -> f32 {
    let ax = x.abs();
    let az = z.abs();
    if !(ax + az).is_finite() || ax + az <= f32::EPSILON {
        return 0.0;
    }
    let acute = if ax <= az {
        45.0 * ax / az
    } else {
        90.0 - 45.0 * az / ax
    };
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let deg = if z >= 0.0 { acute } else { 180.0 - acute };
    (sign * deg).clamp(-180.0, 180.0)
}

#[cfg(test)]
#[path = "velocity_blend_test.rs"]
mod tests;
