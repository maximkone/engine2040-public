//! duck: the finisher beat ducks the music and crowd buses, then restores.
//!
//! Consumer: [`CombatMix`](crate::mix::CombatMix), which carries one of these
//! and answers its bus gains per tick; the window's audio update feeds it the
//! game's finisher beat (`FinisherBeat::active`). The beat halves the bed and
//! the stands so the killing blow is heard, then lifts them back.
//!
//! Units: a linear bus gain 0.5..=1.0, ticks at 60 Hz. Determinism: one float
//! ramp per tick from the previous gain, no clock, no RNG; the same beat tape
//! gives the same gain curve twice.

/// Music and crowd bus gain at the bottom of the duck.
pub const DUCK_GAIN: f32 = 0.5;
/// Ticks the half gain is reached in.
pub const DUCK_TICKS: u32 = 4;
/// Ticks the restore to full gain takes.
pub const RESTORE_TICKS: u32 = 12;
/// Largest gain step one tick may take: the duck's own 0.125 plus headroom.
pub const MAX_STEP: f32 = 0.15;

/// Bus duck under one event: a gain that falls to [`DUCK_GAIN`] over
/// [`DUCK_TICKS`] and climbs back to 1.0 over [`RESTORE_TICKS`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeatDuck {
    gain: f32,
}

impl BeatDuck {
    /// A full-gain bus: nothing ducked yet.
    pub fn new() -> Self {
        Self { gain: 1.0 }
    }

    /// The bus gain now, 0.5..=1.0.
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Advance one tick with `beat` true while the event holds, and answer the
    /// gain to apply to this tick's music and crowd samples. The ramp never
    /// steps more than [`MAX_STEP`], so a duck is heard as a fall, not a cut.
    pub fn tick(&mut self, beat: bool) -> f32 {
        let step = if beat {
            (1.0 - DUCK_GAIN) / tick_share(DUCK_TICKS)
        } else {
            (1.0 - DUCK_GAIN) / tick_share(RESTORE_TICKS)
        };
        self.gain = if beat {
            (self.gain - step).max(DUCK_GAIN)
        } else {
            (self.gain + step).min(1.0)
        };
        self.gain
    }
}

impl Default for BeatDuck {
    fn default() -> Self {
        Self::new()
    }
}

/// Ramp ticks as a float divisor; zero ticks still means one step.
fn tick_share(ticks: u32) -> f32 {
    f32::from(u16::try_from(ticks.max(1)).unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duck_halves_the_bus_inside_the_beat_and_restores_after() {
        let mut duck = BeatDuck::new();
        assert_eq!(duck.gain(), 1.0, "a fresh bus is at full gain");
        // Four ticks of beat reach the half gain, and never step too far.
        let mut prev = duck.gain();
        for tick in 0..DUCK_TICKS {
            let gain = duck.tick(true);
            assert!(
                (prev - gain).abs() <= MAX_STEP,
                "tick {tick} stepped {} (max {MAX_STEP})",
                (prev - gain).abs()
            );
            prev = gain;
        }
        assert!(
            (duck.gain() - DUCK_GAIN).abs() <= 0.02,
            "inside the beat the music gain is 0.5, got {}",
            duck.gain()
        );
        // A held beat stays at the bottom.
        assert_eq!(duck.tick(true), DUCK_GAIN);
        // Twelve quiet ticks restore full gain.
        for tick in 0..RESTORE_TICKS {
            let gain = duck.tick(false);
            assert!(
                (prev - gain).abs() <= MAX_STEP,
                "restore tick {tick} stepped {} (max {MAX_STEP})",
                (prev - gain).abs()
            );
            prev = gain;
        }
        assert!(
            (duck.gain() - 1.0).abs() <= 1e-6,
            "after the restore the music gain is 1.0, got {}",
            duck.gain()
        );
        // Past the restore it holds full.
        assert_eq!(duck.tick(false), 1.0);
    }

    #[test]
    fn duck_a_second_beat_mid_restore_falls_again_without_a_cut() {
        let mut duck = BeatDuck::new();
        for _ in 0..DUCK_TICKS {
            duck.tick(true);
        }
        // Three restore ticks lift it, then the next beat arrives.
        for _ in 0..3 {
            duck.tick(false);
        }
        assert!(duck.gain() > DUCK_GAIN && duck.gain() < 0.7);
        let mut prev = duck.gain();
        for _ in 0..DUCK_TICKS {
            let gain = duck.tick(true);
            assert!((prev - gain).abs() <= MAX_STEP, "the re-duck steps a cut");
            prev = gain;
        }
        assert!((duck.gain() - DUCK_GAIN).abs() <= 0.02);
        // The same tape twice: the same curve twice.
        let run = || {
            let mut duck = BeatDuck::new();
            let mut out = Vec::new();
            for tick in 0..24 {
                out.push(duck.tick(tick < 6 || (12..18).contains(&tick)));
            }
            out
        };
        let first = run();
        assert_eq!(first, run(), "the same beat tape ducks the same way");
    }
}
