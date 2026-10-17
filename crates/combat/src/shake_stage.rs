//! shake_stage: the trauma-curve shake stage as pure integer functions (GP21).
//!
//! The camera director keeps one trauma value in permille (`0..=1000`) that
//! hits raise and ticks decay. This stage turns that value and a tick into a
//! unit shake sample: magnitude follows the squared trauma curve so a jab
//! stays small beside a big hit, and direction follows the smooth gamefeel
//! wave pair instead of white noise. Presentation only: nothing here reads
//! or writes hashed state, and the sample is a function of (trauma, tick), so
//! restoring the pose is subtracting the same sample again.
//!
//! Owner: engine-combat. Consumer (caller must hook in): `shake_offset` in
//! `crates/game/src/camera.rs`, which scales [`ShakeSample`] by the director's
//! `max_shake_m / 1000` per axis. Units: permille of the shake cap. Errors:
//! none; trauma past full clamps. Budget: a few integer ops, no heap.

use crate::gamefeel::shake_wave_q16;

/// Full trauma in permille: the curve input scale and the output scale.
pub const TRAUMA_FULL_PERMILLE: u32 = 1000;

/// One shake sample in signed permille of the shake cap, per axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ShakeSample {
    /// Sideways share of the cap, `-1000..=1000`.
    pub x_permille: i32,
    /// Vertical share of the cap, `-1000..=1000`.
    pub y_permille: i32,
}

/// Shake magnitude for a trauma, as permille of the cap: `trauma^2 / full`.
/// Zero trauma is exactly zero; full trauma is exactly the cap.
#[must_use]
pub const fn curve_permille(trauma_permille: u32) -> u32 {
    let t = if trauma_permille > TRAUMA_FULL_PERMILLE {
        TRAUMA_FULL_PERMILLE
    } else {
        trauma_permille
    };
    t * t / TRAUMA_FULL_PERMILLE
}

/// The shake sample for a trauma at a presented tick. Both axes stay inside
/// `+-curve_permille(trauma)`; zero trauma returns exactly zero.
#[must_use]
pub fn sample(trauma_permille: u32, tick: u32) -> ShakeSample {
    let mag = i64::from(curve_permille(trauma_permille));
    let (wx, wy) = shake_wave_q16(tick);
    let axis = |wave: i32| i32::try_from(i64::from(wave) * mag / 65_536).unwrap_or(0);
    ShakeSample {
        x_permille: axis(wx),
        y_permille: axis(wy),
    }
}

/// How big the small shake is beside the big one, in permille of the big
/// shake's cap share (`curve(small) / curve(big)`); zero when big is zero.
#[must_use]
pub const fn separation_permille(small_trauma: u32, big_trauma: u32) -> u32 {
    let big = curve_permille(big_trauma);
    if big == 0 {
        0
    } else {
        curve_permille(small_trauma) * 1000 / big
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A landed hit's in-sim trauma (`TRAUMA_HIT` in game `camera_feed.rs`).
    const JAB: u32 = 180;
    /// A big hit: a hit plus a knockdown cue in one tick (180 + 350, plus a
    /// follow-up hit before decay).
    const SUPER: u32 = 710;

    fn peak(trauma: u32) -> i32 {
        (0..600)
            .map(|t| {
                let s = sample(trauma, t);
                s.x_permille.abs().max(s.y_permille.abs())
            })
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn jab_displaces_at_most_a_quarter_of_super() {
        assert!(separation_permille(JAB, SUPER) <= 250);
        // Measured over 600 ticks, not just the formula.
        assert!(
            peak(JAB) * 4 <= peak(SUPER),
            "{} vs {}",
            peak(JAB),
            peak(SUPER)
        );
        assert_eq!(separation_permille(JAB, 0), 0);
    }

    #[test]
    fn zero_trauma_restores_exactly_every_tick() {
        for tick in 0..600 {
            assert_eq!(sample(0, tick), ShakeSample::default(), "tick {tick}");
        }
    }

    #[test]
    fn shake_never_exceeds_its_curve_and_full_reaches_the_cap() {
        for trauma in [1, 90, 180, 500, 1000, 4000] {
            let bound = i32::try_from(curve_permille(trauma)).unwrap();
            assert!(peak(trauma) <= bound, "trauma {trauma}");
        }
        assert_eq!(curve_permille(4000), 1000);
        assert!(peak(1000) >= 990, "full trauma reaches the cap");
    }

    #[test]
    fn samples_replay_identically() {
        let run = || (0..300).map(|t| sample(600, t)).collect::<Vec<_>>();
        assert_eq!(run(), run());
    }

    #[test]
    fn the_wave_is_smooth_not_white_noise() {
        // The reference waves turn 1.1 and 1.7 rad per tick, so one tick moves
        // x by at most 2*sin(0.55) = 1.05 and y by 2*sin(0.85) = 1.5 of the cap.
        // White noise would jump the full 2.0 range.
        let (mut worst_x, mut worst_y) = (0, 0);
        for t in 1..600 {
            let (a, b) = (sample(1000, t - 1), sample(1000, t));
            worst_x = worst_x.max((a.x_permille - b.x_permille).abs());
            worst_y = worst_y.max((a.y_permille - b.y_permille).abs());
        }
        assert!(worst_x <= 1060 && worst_y <= 1510, "{worst_x} {worst_y}");
    }
}
