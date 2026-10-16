//! crossfade: per-slot clip crossfade over integer ticks.
//!
//! Consumer: the game animation bridge drives one [`SlotFade`] per fighter
//! and blends the leaving pose into the current one through [`blend_pair`].
//! Data owner: this module owns the fade count and the weight math; the clip
//! bank length and the pose sampler stay with their owners. Units: integer
//! ticks in, unitless weight out; pose seconds stay at the sampler.
//! Determinism: integer state only; same ticks give the same weight twice.
//! Errors: none at runtime; out-of-range weights clamp, a zero total cuts.

use engine_core::Fnv64;

use super::contract::LocalPose;
use super::pose::blend_poses;
use super::state_machine::FIGHTER_CROSSFADE_TICKS;

/// New-clip weight for a fade with `fade_left` ticks to go out of `total`.
/// A zero total cuts at once (1.0); `fade_left` past the total holds the old
/// clip (0.0). The leaving pose stays frozen at the switch frame: over nine
/// ticks the hold reads as a blend, and the full leaving-age path stays with
/// the state-machine playback.
pub fn fade_weight(fade_left: u32, total: u32) -> f32 {
    if total == 0 {
        return 1.0;
    }
    if fade_left >= total {
        return 0.0;
    }
    1.0 - ticks_f32(fade_left) / ticks_f32(total)
}

/// Exact `u32` to `f32` for fade counts: every fade here runs hundreds of
/// ticks at most, far below the exactly representable 2^24 range.
fn ticks_f32(ticks: u32) -> f32 {
    f32::from(u16::try_from(ticks).unwrap_or(u16::MAX))
}

/// One slot's leaving clip while a crossfade runs. All integers: the sampler
/// converts the phase to seconds, so no float ever enters hashed state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SlotFade {
    /// Clip index being left (the caller's bank order).
    pub prev_clip: u8,
    /// Leaving phase in ticks since that clip's start.
    pub prev_phase_ticks: u32,
    /// Ticks to go; 0 means at rest on the current clip.
    pub fade_left: u32,
}

impl SlotFade {
    /// At rest: no leaving clip, full weight on the current one.
    pub const fn idle() -> Self {
        Self {
            prev_clip: 0,
            prev_phase_ticks: 0,
            fade_left: 0,
        }
    }

    /// Stash the leaving cue and restart the full fade. Call it after the
    /// tick clock below, so the switch frame still shows the old clip whole.
    pub fn retarget(&mut self, prev_clip: u8, prev_phase_ticks: u32) {
        self.prev_clip = prev_clip;
        self.prev_phase_ticks = prev_phase_ticks;
        self.fade_left = FIGHTER_CROSSFADE_TICKS;
    }

    /// Advance the fade clock one tick; saturates at rest. Runs before the
    /// tick's event fold, so a retarget later the same tick restarts whole.
    pub fn tick(&mut self) {
        self.fade_left = self.fade_left.saturating_sub(1);
    }

    /// True while the leaving clip still shows.
    pub fn blending(&self) -> bool {
        self.fade_left > 0
    }

    /// New-clip weight now: 0.0 on the switch frame, 1.0 at rest.
    pub fn weight(&self) -> f32 {
        fade_weight(self.fade_left, FIGHTER_CROSSFADE_TICKS)
    }

    /// FNV over every field: the replay proof for the fade state.
    pub fn digest(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_u8(self.prev_clip);
        h.write_u32(self.prev_phase_ticks);
        h.write_u32(self.fade_left);
        h.finish()
    }
}

/// Blend the leaving pose into the current one at this fade position: the
/// shared pose blend with the new-clip weight, so endpoints cut exactly and
/// the midpoint mixes both clips. Caller: the game animation bridge, which
/// samples both poses and shows this result while [`SlotFade::blending`].
pub fn blend_pair(prev: &LocalPose, current: &LocalPose, fade_left: u32) -> LocalPose {
    blend_poses(
        prev,
        current,
        fade_weight(fade_left, FIGHTER_CROSSFADE_TICKS),
    )
}

#[cfg(test)]
mod tests {
    use super::super::contract::LocalPose;
    use super::{blend_pair, fade_weight, SlotFade};
    use engine_core::alloc_probe::measure;

    const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    fn flat(value: f32) -> LocalPose {
        LocalPose {
            positions: vec![[value; 3]; 2],
            rotations: vec![IDENTITY; 2],
        }
    }

    #[test]
    fn fade_weight_covers_the_count() {
        assert_eq!(fade_weight(9, 9), 0.0);
        assert_eq!(fade_weight(0, 9), 1.0);
        assert_eq!(fade_weight(0, 0), 1.0);
        assert_eq!(fade_weight(40, 9), 0.0);
        // Nine ticks are odd, so no frame lands exactly halfway.
        let mid = fade_weight(5, 9);
        assert!((mid - 4.0 / 9.0).abs() < 1e-6, "{mid}");
        assert_ne!(mid, 0.5);
    }

    #[test]
    fn midpoint_blends_both_clips() {
        let (prev, current) = (flat(0.0), flat(10.0));
        let got = blend_pair(&prev, &current, 5);
        // Analytic mix: 10 * 4/9, not a restated constant.
        for joint in 0..2 {
            assert!((got.positions[joint][0] - 40.0 / 9.0).abs() < 1e-4);
        }
        assert_ne!(got, prev);
        assert_ne!(got, current);
        assert_eq!(got, blend_pair(&prev, &current, 5));
    }

    #[test]
    fn endpoints_are_exact_cuts() {
        let (prev, current) = (flat(0.0), flat(10.0));
        assert_eq!(blend_pair(&prev, &current, 9), prev);
        assert_eq!(blend_pair(&prev, &current, 0), current);
    }

    #[test]
    fn retarget_restarts_the_full_fade() {
        let mut fade = SlotFade::idle();
        assert!(!fade.blending());
        fade.retarget(2, 41);
        assert!(fade.blending());
        assert_eq!(fade.prev_clip, 2);
        assert_eq!(fade.prev_phase_ticks, 41);
        for _ in 0..6 {
            fade.tick();
        }
        fade.retarget(4, 7);
        assert_eq!(
            fade.fade_left,
            super::super::state_machine::FIGHTER_CROSSFADE_TICKS
        );
        assert_eq!(fade.prev_clip, 4);
    }

    #[test]
    fn same_ticks_replay_identically() {
        let mut first = SlotFade::idle();
        let mut second = SlotFade::idle();
        first.retarget(1, 3);
        second.retarget(1, 3);
        for _ in 0..12 {
            assert_eq!(first.digest(), second.digest());
            assert_eq!(first.weight(), second.weight());
            first.tick();
            second.tick();
        }
    }

    #[test]
    fn steady_state_allocates_nothing() {
        let mut fade = SlotFade::idle();
        fade.retarget(0, 0);
        let ((), heap) = measure(|| {
            for _ in 0..60 {
                fade.tick();
                let _ = fade.weight();
                let _ = fade.blending();
                let _ = fade.digest();
            }
        });
        assert_eq!(heap.calls, 0, "fade ticks must not allocate");
    }

    #[test]
    fn tick_saturates_at_rest() {
        let mut fade = SlotFade::idle();
        fade.tick();
        assert_eq!(fade.fade_left, 0);
        assert_eq!(fade.weight(), 1.0);
    }
}
