//! finisher_beat: the screen-wide white flash on a killing blow (row SW22).
//!
//! A killing blow earns a 4-tick beat that freezes the frame, then a
//! full-screen white overlay at 0.9 alpha fades over 5 ticks: the screen is
//! white on the beat tick and exactly clear on beat + 9. Donor: Kelpekk/Juicee
//! `addons/juicee/effects/freeze_frame_effect.gd` `_apply` (0.06 s freeze,
//! flash 0.9, fade 0.08 s), read 2026-10-24; 0.06 s is four ticks at 60 Hz and
//! 0.08 s is five, which is where the row's numbers land.
//!
//! Pure and integer: no tick, no renderer, no heap. The caller strikes the
//! beat when a fatal `FightEvent` lands, steps it once per tick, and reads the
//! alpha to composite the overlay. Presentation only: the sim never reads
//! this, the tick count and the golden `d57b5c8ae3096614` never move. Alpha is
//! permille here and one float at the edge ([`FinisherBeat::alpha`]).
//! Determinism: integer counters; the same strike tick gives the same fade.
//! Errors: none; every function is total.

/// Ticks the beat freezes the frame (row: "a 4-tick beat").
pub const BEAT_TICKS: u32 = 4;
/// Overlay alpha at full flash, in permille (row: 0.9 alpha).
pub const FLASH_MILLI: u32 = 900;
/// Ticks the flash fades over once the beat ends (row: fades over 5 ticks).
pub const FADE_TICKS: u32 = 5;
/// Ticks from the strike to a clear screen: the row's "exactly 0 by beat+9".
pub const BEAT_END_TICKS: u32 = BEAT_TICKS + FADE_TICKS;
/// The overlay's own color: full-screen white (row).
pub const OVERLAY_RGB: [u8; 3] = [255, 255, 255];

/// The finisher beat and its white overlay, as view state the caller owns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FinisherBeat {
    age: u32,
    live: bool,
}

impl FinisherBeat {
    /// No beat: the screen is clear.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            age: 0,
            live: false,
        }
    }

    /// A killing blow landed on this tick: start the beat, or restart it when
    /// a second finisher lands inside the fade.
    pub fn strike(&mut self) {
        self.age = 0;
        self.live = true;
    }

    /// One tick of the beat. The beat goes clear on the tick it reaches
    /// [`BEAT_END_TICKS`], so the same strike always clears at the same tick.
    pub fn step(&mut self) {
        if !self.live {
            return;
        }
        self.age = self.age.saturating_add(1);
        if self.age >= BEAT_END_TICKS {
            self.age = 0;
            self.live = false;
        }
    }

    /// Overlay alpha now, in permille: full 0.9 through the beat, a linear
    /// ramp down over the fade, zero once clear.
    #[must_use]
    pub const fn alpha_milli(&self) -> u32 {
        if !self.live {
            return 0;
        }
        if self.age < BEAT_TICKS {
            return FLASH_MILLI;
        }
        let faded = self.age - BEAT_TICKS;
        if faded >= FADE_TICKS {
            return 0;
        }
        FLASH_MILLI * (FADE_TICKS - faded) / FADE_TICKS
    }

    /// The overlay alpha as the renderer wants it, `0.0..=0.9`: the one float
    /// at the presentation edge, never hashed.
    #[must_use]
    pub fn alpha(&self) -> f32 {
        let milli = u16::try_from(self.alpha_milli()).unwrap_or(0);
        f32::from(milli) / 1000.0
    }

    /// True while the beat freezes the frame: the first [`BEAT_TICKS`] ticks.
    #[must_use]
    pub const fn holds(&self) -> bool {
        self.live && self.age < BEAT_TICKS
    }

    /// True while the screen is not clear (beat or fade still running).
    #[must_use]
    pub const fn live(&self) -> bool {
        self.live
    }

    /// Ticks since the strike ([`BEAT_END_TICKS`] or more reads as clear).
    #[must_use]
    pub const fn age(&self) -> u32 {
        self.age
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finisher_overlay_flashes_white_and_fades() {
        // SW22: white on the beat tick, clear on beat + 9, on a killing blow.
        let mut beat = FinisherBeat::new();
        assert_eq!(
            (beat.live(), beat.alpha_milli()),
            (false, 0),
            "a clear screen before any finisher"
        );
        beat.strike();
        // Beat ticks 0..4: full flash, frame held.
        for tick in 0..BEAT_TICKS {
            assert_eq!(beat.alpha_milli(), FLASH_MILLI, "white on tick {tick}");
            assert!(beat.alpha_milli() > 0, "the screen is white on the beat");
            assert!(beat.holds(), "the beat holds the frame at {tick}");
            assert_eq!(beat.age(), tick);
            if tick + 1 < BEAT_TICKS {
                beat.step();
            }
        }
        assert_eq!(beat.alpha(), 0.9, "the overlay is 0.9 alpha at full flash");
        // The beat ends and the fade runs five ticks, never rising.
        beat.step();
        assert_eq!((beat.live(), beat.holds()), (true, false), "fade, no hold");
        assert_eq!(beat.alpha_milli(), 900, "the fade starts at full flash");
        let mut last = beat.alpha_milli();
        for offset in 1..FADE_TICKS {
            beat.step();
            let now = beat.alpha_milli();
            assert!(now < last, "fade {offset} steps down: {last} -> {now}");
            last = now;
        }
        assert_eq!(
            beat.alpha_milli(),
            180,
            "one fifth left on the last fade tick"
        );
        // Beat + 9: exactly clear, and it stays clear.
        beat.step();
        assert_eq!(beat.age(), 0, "the beat is over");
        assert_eq!(beat.alpha_milli(), 0, "exactly 0 by beat + 9");
        assert!(!beat.live());
        assert!(!beat.holds());
        beat.step();
        assert_eq!(beat.alpha_milli(), 0, "a clear screen stays clear");
    }

    #[test]
    fn no_killing_blow_means_no_overlay() {
        // Nothing strikes, nothing flashes: stepping alone never lights it up.
        let mut beat = FinisherBeat::new();
        for _ in 0..20 {
            beat.step();
            assert_eq!(beat.alpha_milli(), 0);
            assert!(!beat.live());
        }
        // The overlay is the row's white, full screen, at the row's numbers.
        assert_eq!(OVERLAY_RGB, [255, 255, 255], "the overlay is white");
        assert_eq!(
            (BEAT_TICKS, FADE_TICKS, FLASH_MILLI),
            (4, 5, 900),
            "the row's 4-tick beat, 5-tick fade and 0.9 alpha"
        );
        assert_eq!(BEAT_END_TICKS, 9, "clear on beat + 9");
        assert_eq!(beat.alpha(), 0.0);
    }

    #[test]
    fn a_second_finisher_restarts_the_beat_from_white() {
        // Two killing blows inside one fade: the second starts a fresh beat,
        // never a stretched one, so the clear tick stays a fixed distance out.
        let mut beat = FinisherBeat::new();
        beat.strike();
        for _ in 0..BEAT_TICKS {
            beat.step();
        }
        assert_eq!(beat.alpha_milli(), 900, "mid-fade");
        beat.strike();
        assert_eq!(beat.alpha_milli(), FLASH_MILLI, "back to full white");
        assert_eq!(beat.age(), 0);
        assert!(beat.holds(), "the new beat holds the frame again");
        for _ in 0..BEAT_END_TICKS {
            beat.step();
        }
        assert_eq!(beat.alpha_milli(), 0, "clear again nine ticks later");
    }
}
