//! camera_feel: flax-style follow rig plus rate-independent damping.
//!
//! Status: retired, no production caller (only the `lib.rs` export plus
//! unit tests; the presentation feed does not call it).
//! Owner: this module owns feel tick state, nothing else.
//! Units: meters, seconds, fixed ticks; trauma `0..=1000`.
//! Determinism: tick is integer `+ - *` only, replays twice;
//! present floats never feed sim state, golden unchanged.
//! Errors: [`CameraError`] at every boundary, never panics.
//! Budget: zero heap bytes per tick or frame.
//! Canonical owners elsewhere: follow is `camera::frame_pair`, shake is
//! `camera::shake_offset`, damping is `camera_rig::damper_blend`.
//! PORT(maximkone/flax-mcp@a26f960:plugins/gameplay/GameSide/
//! Game.Shared.Camera/Feel{CameraRig,Damping,Impulse,Transition}*.cs),
//! MIT; clean-room integer plus presentation rewrite, no vendor bytes.

use crate::camera::{frame_pair, shake_offset, CameraError, CameraPose, DirectorParams, Vec3};
use engine_core::splitmix64;

/// Hard cap for queued impulse permille per burst.
pub const FEEL_MAX_IMPULSE: u32 = 1000;
/// Hard cap for transition length in ticks (10 s at 60 Hz).
pub const FEEL_MAX_TRANSITION_TICKS: u32 = 600;

/// Integer-only tick state, no floats. Never hashed, beside the sim.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct FeelTick {
    /// Trauma permille `0..=1000` after decay.
    pub trauma_permille: u32,
    /// Last tick served, `None` before tick zero.
    pub last_tick: Option<u32>,
    /// Queued impulse waiting for the next advance.
    pub pending_impulse: u32,
}

impl FeelTick {
    /// Empty state before tick zero. Pure.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// Queue a burst: saturating add, clamped. Integer only.
    pub fn push_impulse(&mut self, amount_permille: u32) {
        self.pending_impulse = self
            .pending_impulse
            .saturating_add(amount_permille.min(FEEL_MAX_IMPULSE))
            .min(FEEL_MAX_IMPULSE);
    }
    /// Advance to `tick`: fold impulse, decay by elapsed ticks.
    /// Integer `+ - *` only. Ticks run forward only.
    pub fn advance(&mut self, tick: u32, decay_per_tick: u32) -> Result<u32, CameraError> {
        if let Some(last) = self.last_tick {
            if tick < last {
                return Err(CameraError::TickWentBackwards { last, next: tick });
            }
            let decay = tick.saturating_sub(last).saturating_mul(decay_per_tick);
            self.trauma_permille = self.trauma_permille.saturating_sub(decay);
        }
        self.trauma_permille = self
            .trauma_permille
            .saturating_add(self.pending_impulse)
            .min(FEEL_MAX_IMPULSE);
        self.pending_impulse = 0;
        self.last_tick = Some(tick);
        Ok(self.trauma_permille)
    }
    /// Digest over tick state for replay checks. Integer only.
    #[must_use]
    pub fn digest(self) -> u64 {
        let base = u64::from(self.trauma_permille).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        splitmix64(base.wrapping_add(u64::from(self.last_tick.unwrap_or(0))))
    }
}

/// Follow pose through the frozen pair fit. Same inputs, same pose twice.
#[must_use]
pub fn follow_pose(
    focus_m: Vec3,
    anchor_m: Option<Vec3>,
    params: &DirectorParams,
) -> Option<CameraPose> {
    frame_pair(focus_m, anchor_m, params).ok()
}

/// Exponential blend factor for `dt_s`. Presentation only, never sim state.
pub fn damp_factor(lambda: f32, dt_s: f32) -> Result<f32, CameraError> {
    if !lambda.is_finite() || lambda < 0.0 {
        return Err(CameraError::BadParam("feel_lambda"));
    }
    if !dt_s.is_finite() || dt_s <= 0.0 {
        return Err(CameraError::BadParam("feel_dt"));
    }
    Ok(1.0 - (-lambda * dt_s).exp())
}

/// Transition alpha from integer progress, smoothed. Count is integer;
/// the curve is presentation only.
pub fn transition_alpha(elapsed: u32, total: u32) -> Result<f32, CameraError> {
    if total == 0 || total > FEEL_MAX_TRANSITION_TICKS {
        return Err(CameraError::BadParam("feel_transition"));
    }
    let num = u16::try_from(elapsed.min(total).min(u32::from(u16::MAX))).unwrap_or(u16::MAX);
    let den = u16::try_from(total.min(u32::from(u16::MAX))).unwrap_or(u16::MAX);
    let linear = f32::from(num) / f32::from(den.max(1));
    Ok(linear * linear * (3.0 - 2.0 * linear))
}

/// Shaken eye: pose eye plus integer-hash shake. Never stored.
#[must_use]
pub fn shaken_eye(pose: &CameraPose, trauma: u32, tick: u32, max_m: f32) -> Vec3 {
    pose.eye_m + shake_offset(trauma, tick, max_m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe;

    fn params() -> DirectorParams {
        DirectorParams::default()
    }

    #[test]
    fn follow_pose_headless_valid() {
        let focus = Vec3::new(0.0, 1.0, 0.0);
        let anchor = Some(Vec3::new(2.0, 1.0, 0.0));
        let a = follow_pose(focus, anchor, &params()).expect("pose builds");
        let b = follow_pose(focus, anchor, &params()).expect("pose replays");
        assert_eq!(a.eye_m.x.to_bits(), b.eye_m.x.to_bits());
        assert!(a.eye_m.is_finite() && a.look_m.is_finite());
        assert!(a.eye_m.y > a.look_m.y, "boom sits above look");
    }

    #[test]
    fn damping_rate_independent() {
        let (mut fast, mut slow) = (0.0, 0.0);
        for _ in 0..60 {
            let f = damp_factor(6.0, 1.0 / 60.0).expect("factor");
            fast += (10.0 - fast) * f;
        }
        for _ in 0..30 {
            let f = damp_factor(6.0, 1.0 / 30.0).expect("factor");
            slow += (10.0 - slow) * f;
        }
        assert!(
            (fast - slow).abs() < 0.05,
            "strides converge: {fast} vs {slow}"
        );
        assert!(slow > 9.0, "damping moves: {slow}");
    }

    #[test]
    fn impulse_dispatch_replays() {
        let (mut a, mut b) = (FeelTick::new(), FeelTick::new());
        for state in [&mut a, &mut b] {
            state.push_impulse(400);
            state.advance(0, 60).expect("tick zero");
            state.push_impulse(900);
            state.advance(1, 60).expect("tick one");
        }
        assert_eq!(a.trauma_permille, 1000);
        assert_eq!(a.digest(), b.digest(), "same seed replays twice");
        a.advance(11, 60).expect("decay");
        assert_eq!(a.trauma_permille, 400, "ten ticks decay 600");
    }

    #[test]
    fn failures_name_the_row() {
        let mut tick = FeelTick::new();
        tick.advance(5, 60).expect("first");
        assert_eq!(
            tick.advance(4, 60),
            Err(CameraError::TickWentBackwards { last: 5, next: 4 })
        );
        assert_eq!(
            damp_factor(-1.0, 0.01),
            Err(CameraError::BadParam("feel_lambda"))
        );
        assert_eq!(
            transition_alpha(0, 0),
            Err(CameraError::BadParam("feel_transition"))
        );
        assert!(follow_pose(Vec3::new(f32::NAN, 0.0, 0.0), None, &params()).is_none());
    }

    #[test]
    fn tick_allocates_nothing() {
        let (_, probe) = alloc_probe::measure(|| std::hint::black_box(vec![0u8; 32]));
        assert!(probe.calls >= 1, "probe observes allocation");
        let (folded, steady) = alloc_probe::measure(|| {
            let (mut folded, mut state) = (0u64, FeelTick::new());
            for tick in 0..64 {
                state.push_impulse(10);
                folded = folded.wrapping_add(u64::from(state.advance(tick, 1).unwrap_or(0)));
            }
            folded
        });
        assert_eq!(steady.calls, 0, "integer tick must not allocate");
        assert!(folded != 0, "sanity: ticks advanced");
    }
}
