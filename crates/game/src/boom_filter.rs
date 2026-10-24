//! boom_filter: the fight camera's boom keeps its length through a pillar graze.
//!
//! Owner: game lane. Consumer: [`crate::camera_rig::FightCamSpring::eye_m`], which
//! sweeps the boom against the arena boxes every frame; this filter sits between that
//! sweep and the eye. A sweep that clips a pillar for two frames used to pop the eye in
//! and out, and a fighter walking past a wall made it breathe.
//!
//! Rules (donor Unity Cinemachine `CinemachineDeoccluder`, Body stage,
//! `VcamExtraState::UpdateDistanceSmoothing`): an occlusion must last
//! [`OCCLUSION_HOLD_TICKS`] before the eye moves at all, so a graze is not seen; the
//! pulled distance then eases in on a [`PULL_IN_S`] timescale (90 percent inside 12
//! ticks) and releases on a slower [`RELEASE_S`] one (90 percent inside 24 ticks); and
//! while occluded the length never moves back out past the nearest point seen, so a
//! body walking behind a pillar cannot move the eye through it.
//!
//! Units: meters, seconds, 60 Hz ticks. Determinism: presentation only, pure over its
//! inputs (the same frames give the same length twice); it never reaches a tick hash.
//! Budget: a handful of float ops per frame, no heap.

/// Ticks an occlusion must last before the eye moves: 0.25 s at 60 Hz.
pub const OCCLUSION_HOLD_TICKS: u32 = 15;
/// Seconds the hold lasts: [`OCCLUSION_HOLD_TICKS`] at the arena tick.
pub const OCCLUSION_HOLD_S: f32 = 0.25;
/// Seconds the pull-in takes to reach 90 percent of a steady occlusion.
pub const PULL_IN_S: f32 = 0.2;
/// Seconds the release takes to reach 90 percent of the clear boom.
pub const RELEASE_S: f32 = 0.4;
/// A raw hit this close to the asked length is not an occlusion, meters.
pub const GRAZE_EPS_M: f32 = 0.002;
/// One tick of the arena's 60 Hz rate: the dt a bad frame time falls back to.
const TICK_S: f32 = 1.0 / 60.0;

/// The boom length the eye shows, held and eased against the sweep.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoomFilter {
    /// Length the eye shows now, meters; `0.0` until the first sweep.
    len_m: f32,
    /// Nearest length the sweep asked for in this occlusion, meters.
    nearest_m: f32,
    /// Seconds the current occlusion has lasted.
    occluded_s: f32,
}

impl BoomFilter {
    /// One frame: `ask_m` is the length the shot wants (the unoccluded boom) and
    /// `raw_m` the sweep's hit length (`ask_m` itself when nothing blocks). Answers
    /// the length the eye takes this frame.
    pub fn step(&mut self, dt_s: f32, ask_m: f32, raw_m: f32) -> f32 {
        let dt = if dt_s.is_finite() && dt_s > 0.0 {
            dt_s
        } else {
            TICK_S
        };
        if !ask_m.is_finite() || ask_m <= 0.0 || !raw_m.is_finite() {
            return self.len_m;
        }
        // The sweep never asks for more than the boom: floor a stray hit and keep the
        // eye out of the target.
        let raw = raw_m.clamp(0.0, ask_m);
        if self.len_m <= 0.0 {
            self.len_m = raw;
            self.nearest_m = raw;
            return self.len_m;
        }
        if raw < ask_m - GRAZE_EPS_M {
            self.occluded_s += dt;
            self.nearest_m = self.nearest_m.min(raw);
            if self.occluded_s < OCCLUSION_HOLD_S {
                // Inside the hold: a graze moves nothing at all.
                return self.len_m;
            }
            let share = eased_share(dt, PULL_IN_S);
            let next = self.len_m + (self.nearest_m - self.len_m) * share;
            // Pull in only: never past the nearest point, never back out.
            self.len_m = next.clamp(self.nearest_m, self.len_m);
            return self.len_m;
        }
        self.occluded_s = 0.0;
        self.nearest_m = self.len_m;
        let share = eased_share(dt, RELEASE_S);
        let next = self.len_m + (ask_m - self.len_m) * share;
        // Release only: never past the boom, never further in.
        self.len_m = next.clamp(self.len_m, ask_m);
        self.len_m
    }
}

/// Share of the remaining distance one `dt` covers on a `time_s` (to 90 percent)
/// timescale: `1 - 10^(-dt/time)`, so the pull-in and release keep the donor's shape.
fn eased_share(dt_s: f32, time_s: f32) -> f32 {
    let time = if time_s.is_finite() && time_s > 0.0 {
        time_s
    } else {
        TICK_S
    };
    (1.0 - f32.powf(10.0, -dt_s / time)).clamp(0.0, 1.0)
}

#[cfg(test)]
#[path = "boom_filter_tests.rs"]
mod tests;
