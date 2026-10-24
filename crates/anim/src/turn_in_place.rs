//! turn_in_place: the view-driven body turn gate and the turn clip's play rate.
//!
//! Owner: `engine-anim`; consumer: the fighter look's presentation root (the
//! look reads the view yaw relative to the body and plays a turn clip when the
//! gate fires; the authority yaw is never touched).
//!
//! Source: ALS-Refactored `Source/ALS/Private/AlsAnimationInstance.cpp`
//! [`RefreshTurnInPlace`](https://github.com/Sixze/ALS-Refactored/blob/main/Source/ALS/Private/AlsAnimationInstance.cpp)
//! and [`RefreshRotateInPlace`](https://github.com/Sixze/ALS-Refactored/blob/main/Source/ALS/Private/AlsAnimationInstance.cpp),
//! with the defaults from `Source/ALS/Public/Settings/AlsTurnInPlaceSettings.h`
//! (`FAlsGeneralTurnInPlaceSettings`) and `AlsRotateInPlaceSettings.h`
//! (`FAlsRotateInPlaceSettings`).
//!
//! The gate accumulates an activation timer while the view yaw sits past
//! [`VIEW_YAW_ANGLE_THRESHOLD_DEG`] and the view is slow (below
//! [`VIEW_YAW_SPEED_THRESHOLD_DEG_S`]); the timer counts integer ticks at
//! [`TICKS_PER_SECOND`]. The delay is mapped from the absolute yaw over
//! `threshold..180` onto [`VIEW_YAW_ANGLE_TO_ACTIVATION_DELAY_S`]. ALS's default
//! curve is `{0.0f, 0.75f}`, so a *bigger* turn waits *longer*: a 90 degree
//! turn fires after 15 ticks, a 180 degree one after 45.
//!
//! Presentation only: no clock, no heap, the caller passes degrees and
//! degrees per second, and the sim hash never reads it. Every function is
//! total: a non-finite yaw or speed reads as "no turn" and re-arms the timer.

#[cfg(test)]
#[path = "turn_in_place_test.rs"]
mod tests;

/// Fixed tick the timer counts in, 60 Hz like the rest of the crate.
pub const TICKS_PER_SECOND: u32 = 60;

/// View yaw past which the body turns to catch up, in degrees. ALS
/// `FAlsGeneralTurnInPlaceSettings::ViewYawAngleThreshold{45.0f}`.
pub const VIEW_YAW_ANGLE_THRESHOLD_DEG: f32 = 45.0;

/// View yaw speed under which the view counts as settled, in degrees per
/// second. ALS `FAlsGeneralTurnInPlaceSettings::ViewYawSpeedThreshold{50.0f}`.
pub const VIEW_YAW_SPEED_THRESHOLD_DEG_S: f32 = 50.0;

/// Past this absolute yaw the 180 clip plays instead of the 90, in degrees.
/// ALS `FAlsGeneralTurnInPlaceSettings::Turn180AngleThreshold{130.0f}`.
pub const TURN_180_ANGLE_THRESHOLD_DEG: f32 = 130.0;

/// Activation delay at the yaw threshold and at 180 degrees, in seconds. ALS
/// `FAlsGeneralTurnInPlaceSettings::ViewYawAngleToActivationDelay{0.0f, 0.75f}`,
/// mapped over `ViewYawAngleThreshold..180`.
pub const VIEW_YAW_ANGLE_TO_ACTIVATION_DELAY_S: [f32; 2] = [0.0, 0.75];

/// The longest delay the curve can ask for: `0.75 s` at 60 Hz.
pub const MAX_DELAY_TICKS: u32 = 45;

/// Rotate-in-place reference yaw speeds, in degrees per second. ALS
/// `FAlsRotateInPlaceSettings::ReferenceViewYawSpeed{180.0f, 460.0f}`.
pub const ROTATE_REFERENCE_DEG_S: [f32; 2] = [180.0, 460.0];

/// Turn clip play rate at the low and high reference speed. ALS
/// `FAlsRotateInPlaceSettings::PlayRate{1.15f, 3.0f}`.
pub const ROTATE_PLAY_RATE: [f32; 2] = [1.15, 3.0];

/// Largest change of the stored rate in one tick.
///
/// chosen: ALS eases the mapped rate with `DamperExact`, a half-life damper
/// with no hard cap. A per-tick cap keeps the stored rate provable without a
/// half-life table, and `0.25` walks 1.15 to 3.0 in eight ticks (0.13 s).
pub const RATE_MAX_STEP_PER_TICK: f32 = 0.25;

/// Ticks the turn clip blends in over. ALS `BlendDuration{0.2f}` at 60 Hz.
pub const TURN_BLEND_TICKS: u32 = 12;

/// Which authored turn clip to play.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TurnClip {
    /// 90 degrees over the left shoulder.
    Left90,
    /// 90 degrees over the right shoulder.
    Right90,
    /// 180 degrees over the left shoulder.
    Left180,
    /// 180 degrees over the right shoulder.
    Right180,
}

impl TurnClip {
    /// True when this is a 180 degree clip.
    #[must_use]
    pub fn is_180(self) -> bool {
        matches!(self, TurnClip::Left180 | TurnClip::Right180)
    }

    /// True when the body turns over its left shoulder.
    #[must_use]
    pub fn turns_left(self) -> bool {
        matches!(self, TurnClip::Left90 | TurnClip::Left180)
    }
}

/// One turn the gate asked for: the clip and the view yaw that asked for it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Turn {
    /// Authored clip to play.
    pub clip: TurnClip,
    /// Signed view yaw relative to the body, in degrees: positive is the
    /// body's right.
    pub yaw_deg: f32,
}

/// The activation timer: `step` each tick, a `Turn` when the delay elapsed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TurnGate {
    timer_ticks: u32,
}

impl TurnGate {
    /// A gate with no history.
    #[must_use]
    pub fn new() -> Self {
        Self { timer_ticks: 0 }
    }

    /// Ticks the current attempt has accumulated.
    #[must_use]
    pub fn timer_ticks(&self) -> u32 {
        self.timer_ticks
    }

    /// Forget the current attempt.
    pub fn reset(&mut self) {
        self.timer_ticks = 0;
    }

    /// Advance one tick and report a turn the body owes.
    ///
    /// A moving body never turns. A view inside the angle threshold or faster
    /// than the speed threshold re-arms the timer, so the conditions must hold
    /// for the whole delay. A non-finite yaw or speed reads as no turn.
    pub fn step(&mut self, view_yaw_deg: f32, yaw_speed_deg_s: f32, moving: bool) -> Option<Turn> {
        let yaw = if view_yaw_deg.is_finite() {
            view_yaw_deg
        } else {
            self.reset();
            return None;
        };
        let settled =
            yaw_speed_deg_s.is_finite() && yaw_speed_deg_s.abs() < VIEW_YAW_SPEED_THRESHOLD_DEG_S;
        if moving || !settled || yaw.abs() <= VIEW_YAW_ANGLE_THRESHOLD_DEG {
            self.reset();
            return None;
        }
        self.timer_ticks = self.timer_ticks.saturating_add(1);
        if self.timer_ticks < delay_ticks(yaw) {
            return None;
        }
        self.reset();
        Some(Turn {
            clip: clip_for(yaw),
            yaw_deg: yaw,
        })
    }
}

/// Activation delay for `abs(yaw)` in integer ticks: `0` at the threshold,
/// [`MAX_DELAY_TICKS`] at 180 degrees and past it, rounded up so a delay is
/// never cut short.
#[must_use]
pub fn delay_ticks(view_yaw_deg: f32) -> u32 {
    if !view_yaw_deg.is_finite() {
        return MAX_DELAY_TICKS;
    }
    let yaw = whole_degrees(view_yaw_deg.abs());
    let threshold = whole_degrees(VIEW_YAW_ANGLE_THRESHOLD_DEG);
    if yaw <= threshold {
        return 0;
    }
    let span = 180u32 - u32::from(threshold);
    let over = u32::from(yaw) - u32::from(threshold);
    let ticks = (MAX_DELAY_TICKS * over).div_ceil(span);
    ticks.min(MAX_DELAY_TICKS)
}

/// The clip for a signed view yaw: 180 past the threshold, otherwise 90, and
/// the shoulder follows the sign.
fn clip_for(view_yaw_deg: f32) -> TurnClip {
    let big = view_yaw_deg.abs() >= TURN_180_ANGLE_THRESHOLD_DEG;
    match (big, view_yaw_deg < 0.0) {
        (true, true) => TurnClip::Left180,
        (true, false) => TurnClip::Right180,
        (false, true) => TurnClip::Left90,
        (false, false) => TurnClip::Right90,
    }
}

/// Whole degrees for the integer delay math.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the caller passes a finite absolute yaw clamped to 0..=180, so the rounded value is exact in u16"
)]
fn whole_degrees(deg: f32) -> u16 {
    deg.clamp(0.0, 180.0).round() as u16
}

/// Turn clip play rate for a view yaw speed: a clamped map from
/// `reference` (degrees per second) onto `rate`. Non-finite input, or a
/// reference range that is empty or reversed, reads `rate[0]`.
#[must_use]
pub fn turn_rate(yaw_speed_deg_s: f32, reference: [f32; 2], rate: [f32; 2]) -> f32 {
    let span = reference[1] - reference[0];
    if !yaw_speed_deg_s.is_finite() || !span.is_finite() || span <= 0.0 {
        return rate[0];
    }
    let t = ((yaw_speed_deg_s.abs() - reference[0]) / span).clamp(0.0, 1.0);
    rate[0] + (rate[1] - rate[0]) * t
}

/// The stored turn clip rate: the tick's mapped rate, walked toward at no more
/// than [`RATE_MAX_STEP_PER_TICK`] per tick, so a view flick cannot pop it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurnRate {
    rate: f32,
}

impl Default for TurnRate {
    fn default() -> Self {
        Self::new()
    }
}

impl TurnRate {
    /// A rate at rest: the slowest turn plays at `ROTATE_PLAY_RATE[0]`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            rate: ROTATE_PLAY_RATE[0],
        }
    }

    /// The rate the clip should play at this tick.
    #[must_use]
    pub fn rate(&self) -> f32 {
        self.rate
    }

    /// Step the rate toward `target`, moving at most [`RATE_MAX_STEP_PER_TICK`].
    /// A non-finite target keeps the stored rate; the result stays inside
    /// `ROTATE_PLAY_RATE`.
    pub fn step_towards(&mut self, target: f32) -> f32 {
        if target.is_finite() {
            let delta = (target - self.rate).clamp(-RATE_MAX_STEP_PER_TICK, RATE_MAX_STEP_PER_TICK);
            self.rate = (self.rate + delta).clamp(ROTATE_PLAY_RATE[0], ROTATE_PLAY_RATE[1]);
        }
        self.rate
    }

    /// Step the rate toward [`turn_rate`] of this tick's view yaw speed.
    pub fn step(&mut self, yaw_speed_deg_s: f32) -> f32 {
        self.step_towards(turn_rate(
            yaw_speed_deg_s,
            ROTATE_REFERENCE_DEG_S,
            ROTATE_PLAY_RATE,
        ))
    }
}
