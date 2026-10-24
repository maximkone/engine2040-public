//! turn_look: each fighter's view yaw from presentation state, and the root
//! turn it drives.
//!
//! Owner: `engine-game`; consumer: the fighter look's presentation root. The
//! view yaw is the bearing to the nearest rival, clamped to the aim limit;
//! [`TurnGate`] decides when the body owes a turn and which authored clip
//! plays. The authority yaw (`slot.yaw`) is never read for writing here: the
//! turn lives in the presentation root offset only, so the sim and its hash
//! cannot see it.
//!
//! Source: ALS-Refactored turn in place (`crates/anim/src/turn_in_place.rs`
//! carries the donor constants); this module is the missing view-yaw feed that
//! module expects.

use engine_anim::turn_in_place::{
    TurnClip, TurnGate, TurnRate, TICKS_PER_SECOND, TURN_BLEND_TICKS,
};

/// Furthest the view may sit off the body, in degrees: the rival aim limit
/// (`engine_anim::ik::AimLimits::rival`, 70 degrees each side).
pub const AIM_CLAMP_DEG: f32 = 70.0;

/// [TICKS_PER_SECOND] as a float, for the view yaw speed. The pair is kept
/// equal by 	urn_in_place_the_tick_rate_float_matches_the_gate.
const TICKS_PER_SECOND_F: f32 = 60.0;

/// A fighter's view yaw: the bearing from `from` to `to`, relative to the
/// body yaw, in degrees, positive toward the body's right, clamped to
/// [`AIM_CLAMP_DEG`]. A target on the body reads 0, and a non-finite input
/// reads 0, so a broken frame owns no turn.
#[must_use]
pub fn aim_yaw_deg(body_yaw_deg: f32, from: [f32; 2], to: [f32; 2]) -> f32 {
    if !body_yaw_deg.is_finite() || !from.iter().all(|v| v.is_finite()) {
        return 0.0;
    }
    if !to.iter().all(|v| v.is_finite()) {
        return 0.0;
    }
    let (dx, dz) = (to[0] - from[0], to[1] - from[1]);
    if dx.abs() < 1.0e-6 && dz.abs() < 1.0e-6 {
        return 0.0;
    }
    // Zero faces +z and a quarter turn faces +x, so the bearing is
    // atan2(x, z) and the view yaw is that minus the body yaw.
    let bearing = dx.atan2(dz).to_degrees() - body_yaw_deg;
    wrap_deg(bearing).clamp(-AIM_CLAMP_DEG, AIM_CLAMP_DEG)
}

/// Wrap an angle into `-180..=180`, degrees.
fn wrap_deg(deg: f32) -> f32 {
    let mut d = deg % 360.0;
    if d > 180.0 {
        d -= 360.0;
    }
    if d < -180.0 {
        d += 360.0;
    }
    d
}

/// What the look reads after one presented tick.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ViewTurn {
    /// Root yaw offset from the authority yaw, in degrees, positive right.
    /// The look adds this to the presentation root only.
    pub root_offset_deg: f32,
    /// The clip this tick started, if the gate fired.
    pub fired: Option<TurnClip>,
    /// Play rate of the clip being eased, from the view yaw speed.
    pub play_rate: f32,
}

/// One fighter's view-driven turn: the gate, the running clip and the eased
/// root offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AimTurn {
    gate: TurnGate,
    rate: TurnRate,
    last_view_deg: f32,
    view_deg: f32,
    offset_deg: f32,
    from_deg: f32,
    to_deg: f32,
    ease_left: u32,
    ease_total: u32,
}

impl Default for AimTurn {
    fn default() -> Self {
        Self::new()
    }
}

impl AimTurn {
    /// A fighter looking straight ahead: no view, no offset.
    #[must_use]
    pub fn new() -> Self {
        Self {
            gate: TurnGate::new(),
            rate: TurnRate::new(),
            last_view_deg: 0.0,
            view_deg: 0.0,
            offset_deg: 0.0,
            from_deg: 0.0,
            to_deg: 0.0,
            ease_left: 0,
            ease_total: 0,
        }
    }

    /// The current root offset, degrees.
    #[must_use]
    pub fn root_offset_deg(&self) -> f32 {
        self.offset_deg
    }

    /// The view yaw the last tick read, degrees.
    #[must_use]
    pub fn view_deg(&self) -> f32 {
        self.view_deg
    }

    /// One presented tick: `view_deg` is [`aim_yaw_deg`] for this tick,
    /// `moving` whether the body is translating.
    ///
    /// The gate decides the clip; the root offset eases from where it is to
    /// the new view over [`TURN_BLEND_TICKS`], so a flick of the aim never
    /// snaps the body. Nothing here writes the authority yaw.
    pub fn step(&mut self, view_deg: f32, moving: bool) -> ViewTurn {
        let view = if view_deg.is_finite() {
            view_deg.clamp(-AIM_CLAMP_DEG, AIM_CLAMP_DEG)
        } else {
            self.last_view_deg
        };
        let speed = (view - self.last_view_deg) * TICKS_PER_SECOND_F;
        self.last_view_deg = view;
        self.view_deg = view;
        let rate = self.rate.step(speed);
        let fired = self.gate.step(view, speed, moving);
        if fired.is_some() {
            // The body owes a turn: ease the root onto the new view over the
            // clip's blend, from wherever the root sits now.
            self.from_deg = self.offset_deg;
            self.to_deg = view;
            self.ease_total = TURN_BLEND_TICKS;
            self.ease_left = TURN_BLEND_TICKS;
        } else if self.ease_left == 0 && self.offset_deg != view {
            // No clip running: keep the root behind a slow view by the same
            // blend, so a walking aim still reads.
            self.from_deg = self.offset_deg;
            self.to_deg = view;
            self.ease_total = TURN_BLEND_TICKS;
            self.ease_left = TURN_BLEND_TICKS;
        }
        if self.ease_left > 0 {
            let done = f32::from(u16::try_from(self.ease_total - self.ease_left + 1).unwrap_or(1));
            let total = f32::from(u16::try_from(self.ease_total).unwrap_or(1));
            let t = done / total;
            let s = t * t * (3.0 - 2.0 * t);
            self.offset_deg = self.from_deg + (self.to_deg - self.from_deg) * s;
            self.ease_left -= 1;
            if self.ease_left == 0 {
                self.offset_deg = self.to_deg;
            }
        }
        ViewTurn {
            root_offset_deg: self.offset_deg,
            fired: fired.map(|turn| turn.clip),
            play_rate: rate,
        }
    }
}

#[cfg(test)]
#[path = "turn_look_test.rs"]
mod tests;
