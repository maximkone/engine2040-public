//! impact_punch: the 3-tick punch a heavy hit earns (row SW140, the owner's
//! realistic lane: a zoom punch, an exposure lift and a radial blur, no manga
//! frame and no extra render pass).
//!
//! The envelope is integer permille and total: a strike reads full on its own
//! tick, two thirds on the next, one third on the last, and zero from then on,
//! so the picture always returns to the shipped look without a snap. Every
//! strength is a multiplier over a shipped value: 1000 leaves it alone.
//!
//! Determinism: presentation only. No tick, sim value or golden hash reads a
//! punch; the same strike on the same tick gives the same three strengths.
//! Errors: none; every function is total.
//!
//! Tier policy: [`enabled_for_tier`] is off on `Potato` and `Mobile` (the
//! blur's taps are the only per-pixel cost), so a weak tier paints the shipped
//! frame and pays nothing.

use engine_render::quality::Quality;

/// Ticks the punch lasts (row: 3).
pub const PUNCH_TICKS: u32 = 3;
/// The fov narrows by this share of itself at the strike (row: about 6%).
pub const FOV_NARROW_PERMILLE: u32 = 60;
/// The grade's exposure lift at the strike, in thousandths of a stop.
pub const EXPOSURE_LIFT_MILLI: u32 = 120;
/// Radial blur strength at the strike, permille of the shader's tap radius.
pub const BLUR_PERMILLE: u32 = 350;
/// A full envelope, and the unit every strength is a fraction of.
pub const FULL_PERMILLE: u32 = 1000;

/// Whether a tier paints the punch: off on the two weak tiers.
#[must_use]
pub const fn enabled_for_tier(quality: Quality) -> bool {
    !matches!(quality, Quality::Potato | Quality::Mobile)
}

/// The punch and the tick it is on, as view state the caller owns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImpactPunch {
    age: u32,
    live: bool,
}

impl ImpactPunch {
    /// No punch: every strength zero and the shipped frame.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            age: 0,
            live: false,
        }
    }

    /// A heavy blow landed on this tick: start the punch, or restart it when a
    /// second one lands inside the three ticks.
    pub fn strike(&mut self) {
        self.age = 0;
        self.live = true;
    }

    /// One tick of the punch. It goes quiet on the tick it reaches
    /// [`PUNCH_TICKS`], so the same strike always clears on the same tick.
    pub fn step(&mut self) {
        if !self.live {
            return;
        }
        self.age = self.age.saturating_add(1);
        if self.age >= PUNCH_TICKS {
            self.live = false;
        }
    }

    /// Whether a punch is in flight.
    #[must_use]
    pub fn live(&self) -> bool {
        self.live
    }

    /// The tick of the punch, zero on the strike tick.
    #[must_use]
    pub fn age(&self) -> u32 {
        self.age
    }

    /// The envelope: 1000, 666, 333, then 0. Integer division, no drift.
    #[must_use]
    pub fn env_permille(&self) -> u32 {
        if !self.live {
            return 0;
        }
        (PUNCH_TICKS - self.age.min(PUNCH_TICKS)) * FULL_PERMILLE / PUNCH_TICKS
    }

    /// The fov for this tick: narrowed toward the shipped value, so the camera
    /// pushes in and eases back. `on` is the tier's [`enabled_for_tier`].
    #[must_use]
    pub fn fov_y(&self, shipped_fov_y: f32, on: bool) -> f32 {
        if !on {
            return shipped_fov_y;
        }
        let narrow = FOV_NARROW_PERMILLE * self.env_permille() / FULL_PERMILLE;
        let keep = f32::from(u16::try_from(FULL_PERMILLE - narrow).unwrap_or(1000)) / 1000.0;
        shipped_fov_y * keep
    }

    /// The grade's exposure lift for this tick, in thousandths of a stop.
    #[must_use]
    pub fn exposure_lift_milli(&self, on: bool) -> u32 {
        if on {
            EXPOSURE_LIFT_MILLI * self.env_permille() / FULL_PERMILLE
        } else {
            0
        }
    }

    /// The radial blur strength for this tick, permille of the tap radius.
    #[must_use]
    pub fn blur_permille(&self, on: bool) -> u32 {
        if on {
            BLUR_PERMILLE * self.env_permille() / FULL_PERMILLE
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOV: f32 = core::f32::consts::FRAC_PI_3;

    fn ticks(on: bool) -> Vec<(u32, u32, u32)> {
        let mut punch = ImpactPunch::new();
        punch.strike();
        let mut out = Vec::new();
        for _ in 0..5 {
            out.push((
                punch.exposure_lift_milli(on),
                punch.blur_permille(on),
                punch.fov_y(FOV, on).to_bits(),
            ));
            punch.step();
        }
        out
    }

    #[test]
    fn the_envelope_is_three_ticks_and_quiet_after() {
        let mut punch = ImpactPunch::new();
        punch.strike();
        assert_eq!(punch.env_permille(), 1000, "full on the strike tick");
        punch.step();
        assert_eq!(punch.env_permille(), 666);
        punch.step();
        assert_eq!(punch.env_permille(), 333);
        punch.step();
        assert!(!punch.live());
        assert_eq!(punch.env_permille(), 0, "clear on the tick the row ends");
    }

    #[test]
    fn a_strike_narrows_the_fov_and_lifts_the_grade() {
        let mut punch = ImpactPunch::new();
        punch.strike();
        let narrowed = punch.fov_y(FOV, true);
        assert!(narrowed < FOV, "the camera pushes in");
        assert!(
            (narrowed - FOV * 0.94).abs() < 1.0e-5,
            "about 6% narrower, got {narrowed}"
        );
        assert_eq!(punch.exposure_lift_milli(true), EXPOSURE_LIFT_MILLI);
        assert_eq!(punch.blur_permille(true), BLUR_PERMILLE);
        // Easing back on the next two ticks, never past the shipped fov.
        let mut last = narrowed;
        for _ in 0..2 {
            punch.step();
            let now = punch.fov_y(FOV, true);
            assert!(now > last && now <= FOV, "eases back: {last} -> {now}");
            last = now;
        }
        punch.step();
        assert_eq!(punch.fov_y(FOV, true), FOV, "exactly shipped after 3 ticks");
        assert_eq!(punch.exposure_lift_milli(true), 0);
        assert_eq!(punch.blur_permille(true), 0);
    }

    #[test]
    fn the_weak_tiers_paint_the_shipped_frame() {
        assert!(!enabled_for_tier(Quality::Potato));
        assert!(!enabled_for_tier(Quality::Mobile));
        assert!(enabled_for_tier(Quality::Comfort));
        assert!(enabled_for_tier(Quality::High));
        for (exposure, blur, fov) in ticks(false) {
            assert_eq!((exposure, blur, fov), (0, 0, FOV.to_bits()));
        }
        for (exposure, blur, fov) in ticks(true) {
            assert!(exposure > 0 || blur > 0 || fov == FOV.to_bits());
        }
    }

    #[test]
    fn two_punches_on_the_same_ticks_are_equal() {
        assert_eq!(ticks(true), ticks(true));
        let mut a = ImpactPunch::new();
        let mut b = ImpactPunch::new();
        a.strike();
        b.strike();
        for _ in 0..4 {
            assert_eq!(a, b);
            assert_eq!(a.blur_permille(true), b.blur_permille(true));
            a.step();
            b.step();
        }
    }

    #[test]
    fn a_second_strike_restarts_the_punch() {
        let mut punch = ImpactPunch::new();
        punch.strike();
        punch.step();
        punch.step();
        assert_eq!(punch.env_permille(), 333);
        punch.strike();
        assert_eq!(punch.env_permille(), 1000, "a new blow is full again");
    }
}
