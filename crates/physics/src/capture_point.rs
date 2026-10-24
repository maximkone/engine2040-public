//! capture_point: one landed blow picks hold, step or fall by the capture point.
//!
//! The capture point is the planar point a body's momentum would carry its
//! center of mass to in one pendulum time, `sqrt(height / g)`: standing still
//! it sits under the center of mass, and a shove leads it toward the blow.
//! Measured against the support region (the feet) it says what the victim must
//! do: well inside holds (a stagger), near the edge steps the blow out, and at
//! or past the edge falls. Classic balance control (2603.08619 uses the capture
//! point as a training signal).
//!
//! Consumer: the arena hit pipeline may hand one landed blow to
//! [`strike_choice`] beside `BalanceState::apply_impact`; nothing in the fixed
//! tick calls this module today, so the golden hash is untouched.
//! Owner: this module owns [`SupportRegion`], [`CaptureVerdict`],
//! [`CaptureChoice`], [`pendulum_ticks`], [`capture_point_mm`],
//! [`capture_choice`] and [`strike_choice`]; it reuses `balance`'s integer
//! millimetres, Q8 impulse ceiling and knockback velocity rather than mirroring
//! them. Units: integer millimetres on the XZ plane, millimetres per tick at
//! 60 Hz, Q8 impulse units, Q10 direction (1024 reads unity).
//! Determinism: integer math only, `engine_core::fixed::isqrt` for the pendulum
//! root (no float, no libm), no allocation, and a blow mirrored left/right
//! mirrors the choice exactly. Errors: an over-ceiling impulse comes back as
//! `BalanceError::BadImpulse`, never a panic on caller data.
#![forbid(unsafe_code)]

use super::balance::{
    knockback_velocity_q10, BalanceError, GaitVerdict, MAX_IMPULSE_Q8, SUPPORT_HALF_MM,
};
use engine_core::fixed::isqrt;

/// Gravity, millimetres per second squared: the pendulum time reads this one
/// value, so every machine measures the same time for the same height.
pub const GRAVITY_MM_S2: i64 = 9807;
/// Fixed-step tick rate: the pendulum time is measured in ticks because the
/// velocity is millimetres per tick.
pub const TICK_HZ: i64 = 60;
/// Default support half-extent, millimetres: the stance box of
/// [`SupportRegion::stance`], shared with the balance capture bias so both read
/// one support width.
pub const STANCE_HALF_MM: i32 = SUPPORT_HALF_MM;
/// Hold margin inside the support edge, millimetres: a capture point this far
/// inside the edge still holds. Between the edge and this margin the victim is
/// on its toes and steps, and at or past the edge it falls.
pub const STEP_MARGIN_MM: i32 = 40;

/// What the victim must do about one blow, ordered by severity
/// (`Hold < Step < Fall`): a harder blow never picks a milder reaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CaptureVerdict {
    /// The capture point stays well inside the support: the stance holds and
    /// the fighter answers late (a stagger).
    Hold,
    /// The capture point reached the support edge: the victim steps the blow
    /// out to put its feet back under the momentum.
    Step,
    /// The capture point left the support: the victim falls.
    Fall,
}

impl CaptureVerdict {
    /// The gait level this choice asks for, so a caller in the balance ladder
    /// reads the same three rungs as the shove-response matrix.
    pub const fn gait(self) -> GaitVerdict {
        match self {
            Self::Hold => GaitVerdict::Staggered,
            Self::Step => GaitVerdict::Stumbling,
            Self::Fall => GaitVerdict::Fallen,
        }
    }

    /// Stable hash code: hold reads 0 through fall reads 2.
    pub const fn code(self) -> u8 {
        match self {
            Self::Hold => 0,
            Self::Step => 1,
            Self::Fall => 2,
        }
    }
}

/// The support region a body stands on: the feet box on the XZ plane, centre
/// plus per-axis half-extent, integer millimetres.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupportRegion {
    /// Support centre, millimetres, `[x, z]`: the point between the feet.
    pub centre_mm: [i32; 2],
    /// Half-extent per axis, millimetres; negatives clamp at zero, so a point
    /// support still answers.
    pub half_mm: [i32; 2],
}

impl SupportRegion {
    /// Build from a centre and per-axis half-extents.
    pub const fn new(centre_mm: [i32; 2], half_mm: [i32; 2]) -> Self {
        Self { centre_mm, half_mm }
    }

    /// Square support of one half-extent about a centre.
    pub const fn square(centre_mm: [i32; 2], half_mm: i32) -> Self {
        Self::new(centre_mm, [half_mm, half_mm])
    }

    /// The default stance: feet centred on the origin under a
    /// [`STANCE_HALF_MM`] box.
    pub const fn stance() -> Self {
        Self::square([0, 0], STANCE_HALF_MM)
    }
}

/// One capture-point decision: where the momentum points, how long the
/// pendulum takes to get there, the slack left to the support edge and the
/// reaction the victim owes. Integer millimetres throughout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureChoice {
    /// Capture point on the XZ plane, millimetres: center of mass plus the
    /// velocity carried one pendulum time.
    pub cp_mm: [i32; 2],
    /// Pendulum time, ticks at 60 Hz: `sqrt(height / g)`, floored.
    pub pendulum_ticks: u32,
    /// Slack to the support edge, millimetres: the tightest axis of the
    /// support, positive inside it and negative past it.
    pub slack_mm: i32,
    /// What the victim must do.
    pub verdict: CaptureVerdict,
}

/// Pendulum time in ticks at 60 Hz: `sqrt(height / g)`, floored, so the
/// capture lead uses no float and no libm. A height at or under zero (a body
/// on the ground) reads zero ticks: momentum carries a body with no standing
/// height nowhere, so a downed fighter never re-falls from its own velocity.
pub fn pendulum_ticks(height_mm: i32) -> u32 {
    if height_mm <= 0 {
        return 0;
    }
    // t = sqrt(h / g) seconds is t^2 = (HZ^2 * h) / g ticks squared; the
    // 32-bit height keeps the product inside i64 (about 7.7e12 at the top).
    let squared = TICK_HZ * TICK_HZ * i64::from(height_mm) / GRAVITY_MM_S2;
    if squared <= 0 {
        return 0;
    }
    let root = isqrt(u128::try_from(squared).unwrap_or(0));
    // Past 2^32 ticks is 2 years of standing: hold the ceiling, never panic.
    u32::try_from(root).unwrap_or(u32::MAX)
}

/// Capture point on the XZ plane, millimetres: the center of mass plus the
/// carried velocity times one pendulum time. Integer math only; the lead
/// saturates at the `i32` millimetre ends so a wild caller cannot overflow.
pub fn capture_point_mm(com_mm: [i32; 3], velocity_mm_tick: [i32; 2]) -> [i32; 2] {
    let ticks = i64::from(pendulum_ticks(com_mm[1]));
    [
        lead_mm(com_mm[0], velocity_mm_tick[0], ticks),
        lead_mm(com_mm[2], velocity_mm_tick[1], ticks),
    ]
}

/// One planar lane's capture point: base plus velocity over `ticks`, held
/// inside the millimetre range. Mirror-exact: negating both inputs negates the
/// output, because the multiplication and the saturating add are symmetric.
fn lead_mm(base_mm: i32, velocity_mm_tick: i32, ticks: i64) -> i32 {
    let travelled = i64::from(velocity_mm_tick).saturating_mul(ticks);
    saturate_mm(i64::from(base_mm).saturating_add(travelled))
}

/// Narrow a widened millimetre value into `i32`, holding the nearer end.
fn saturate_mm(v: i64) -> i32 {
    // The clamp proves the narrowing; the fallback never runs.
    i32::try_from(v.clamp(i64::from(i32::MIN), i64::from(i32::MAX))).unwrap_or(0)
}

/// Slack of one planar lane, millimetres: the support half-extent minus the
/// distance from the support centre, so it reads positive inside the edge and
/// negative past it. Integer math only, symmetric in the sign of the gap.
fn axis_slack_mm(cp_mm: i32, centre_mm: i32, half_mm: i32) -> i32 {
    let gap = i64::from(cp_mm)
        .saturating_sub(i64::from(centre_mm))
        .saturating_abs();
    let inside = i64::from(half_mm.max(0)).saturating_sub(gap);
    saturate_mm(inside)
}

/// Read one blow: the capture point against the support decides hold, step or
/// fall. The tightest axis decides, so a capture point outside the box on
/// either plane falls; well inside holds; at or past the edge falls; between
/// the edge and [`STEP_MARGIN_MM`] inside it steps. Integer math only, no
/// allocation, and a mirrored input mirrors the choice.
pub fn capture_choice(
    com_mm: [i32; 3],
    support: SupportRegion,
    velocity_mm_tick: [i32; 2],
) -> CaptureChoice {
    let cp_mm = capture_point_mm(com_mm, velocity_mm_tick);
    let slack_mm = axis_slack_mm(cp_mm[0], support.centre_mm[0], support.half_mm[0]).min(
        axis_slack_mm(cp_mm[1], support.centre_mm[1], support.half_mm[1]),
    );
    let verdict = if slack_mm >= STEP_MARGIN_MM {
        CaptureVerdict::Hold
    } else if slack_mm > 0 {
        CaptureVerdict::Step
    } else {
        CaptureVerdict::Fall
    };
    CaptureChoice {
        cp_mm,
        pendulum_ticks: pendulum_ticks(com_mm[1]),
        slack_mm,
        verdict,
    }
}

/// Read one landed hit: the Q8 impulse along a Q10 planar direction becomes
/// the velocity the body carries through [`knockback_velocity_q10`], which
/// then picks the reaction. Over-ceiling impulses are refused with nothing
/// read, matching the balance ladder's own ceiling.
pub fn strike_choice(
    com_mm: [i32; 3],
    support: SupportRegion,
    impulse_q8: u16,
    dir_q10: [i32; 2],
) -> Result<CaptureChoice, BalanceError> {
    if impulse_q8 > MAX_IMPULSE_Q8 {
        return Err(BalanceError::BadImpulse(impulse_q8));
    }
    let velocity_mm_tick = knockback_velocity_q10(impulse_q8, dir_q10);
    Ok(capture_choice(com_mm, support, velocity_mm_tick))
}

#[cfg(test)]
#[path = "capture_point_tests.rs"]
mod tests;
