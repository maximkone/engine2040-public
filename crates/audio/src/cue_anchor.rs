//! cue_anchor: place a cue by musical anchor plus offset, and a forward-only
//! clock.
//!
//! Caller: the scene loader reads an authored anchor and offset, calls
//! [`resolve`] with the caller's [`CueTimebase`], and writes the tick it gets
//! into [`CueRow::tick`](crate::cue_sheet::CueRow::tick); the sheet stays
//! tick-based, so this module adds no second cue store. The audio thread reads
//! [`CueClock`] instead of a wall clock: [`CueClock::advance`] never reports a
//! tick below the one it reported before, so a late or replayed update cannot
//! walk the mix backwards.
//!
//! Units: anchors are beat, bar or tick indexes; offsets are whole beats or
//! whole milliseconds; gains stay the sheet's whole dB. One tick is one beat
//! only when the caller sets `ticks_per_minute == bpm`; nothing here assumes
//! it. Determinism: integer division only (documented rounding), no float, no
//! clock read, no RNG. Failure mode: an offset that lands before tick 0 or a
//! zero rate returns [`AnchorError`] naming the anchor, the offset and one
//! recovery; nothing panics.

use engine_core::Fnv64;

/// Where a cue hangs from. Indexes count from 0, so `Beat(0)` and
/// `Downbeat(0)` are the first beat and first bar.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Anchor {
    /// Beat index from the start.
    Beat(u32),
    /// Bar index from the start; one bar is [`CueTimebase::beats_per_bar`]
    /// beats.
    Downbeat(u32),
    /// Raw tick number, tempo-independent.
    Marker(u32),
    /// Tick 0: the segment's first tick.
    SegmentStart,
}

impl core::fmt::Display for Anchor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Beat(index) => write!(f, "Beat({index})"),
            Self::Downbeat(index) => write!(f, "Downbeat({index})"),
            Self::Marker(tick) => write!(f, "Marker({tick})"),
            Self::SegmentStart => write!(f, "SegmentStart"),
        }
    }
}

/// Shift applied to an anchor.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CueOffset {
    /// Whole beats, negative to pull the cue earlier.
    Beats(i32),
    /// Whole milliseconds, converted with [`CueTimebase::ms_per_tick`];
    /// integer division truncates toward zero.
    Ms(i32),
}

impl core::fmt::Display for CueOffset {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Beats(beats) => write!(f, "Beats({beats})"),
            Self::Ms(ms) => write!(f, "Ms({ms})"),
        }
    }
}

/// The caller's grid: tick rate, tempo, meter and tick length. All four
/// values are at least 1 (see [`CueTimebase::new`]).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CueTimebase {
    /// Ticks in one minute: the tick rate the anchor math divides by.
    pub ticks_per_minute: u32,
    /// Beats per minute.
    pub bpm: u32,
    /// Beats in one bar, used only by [`Anchor::Downbeat`].
    pub beats_per_bar: u32,
    /// Milliseconds in one tick, used only by [`CueOffset::Ms`].
    pub ms_per_tick: u32,
}

impl CueTimebase {
    /// Timebase from its four rates; every rate must be at least 1. Pure.
    ///
    /// # Errors
    ///
    /// Returns [`AnchorError::BadTimebase`] naming the first zero field.
    pub fn new(
        ticks_per_minute: u32,
        bpm: u32,
        beats_per_bar: u32,
        ms_per_tick: u32,
    ) -> Result<Self, AnchorError> {
        for (field, value) in [
            ("ticks_per_minute", ticks_per_minute),
            ("bpm", bpm),
            ("beats_per_bar", beats_per_bar),
            ("ms_per_tick", ms_per_tick),
        ] {
            if value == 0 {
                return Err(AnchorError::BadTimebase { field });
            }
        }
        Ok(Self {
            ticks_per_minute,
            bpm,
            beats_per_bar,
            ms_per_tick,
        })
    }

    /// Ticks in one beat: `ticks_per_minute / bpm`, floored, at least 1
    /// (a tick rate below the tempo rounds to one tick per beat). Pure.
    #[must_use]
    pub fn ticks_per_beat(self) -> u64 {
        (u64::from(self.ticks_per_minute) / u64::from(self.bpm)).max(1)
    }

    /// Ticks in one bar: one beat times [`Self::beats_per_bar`], saturating.
    /// Pure.
    #[must_use]
    pub fn ticks_per_bar(self) -> u64 {
        self.ticks_per_beat()
            .saturating_mul(u64::from(self.beats_per_bar))
    }
}

/// Resolve `anchor` plus `offset` to an absolute tick on `timebase`. Pure.
///
/// # Errors
///
/// Returns [`AnchorError::BeforeZero`] when the offset lands the cue before
/// tick 0, carrying the anchor, the offset and the recovery.
pub fn resolve(
    anchor: Anchor,
    offset: CueOffset,
    timebase: &CueTimebase,
) -> Result<u64, AnchorError> {
    let per_beat = i128::from(timebase.ticks_per_beat());
    let per_bar = i128::from(timebase.ticks_per_bar());
    let base = match anchor {
        Anchor::Beat(index) => i128::from(index).saturating_mul(per_beat),
        Anchor::Downbeat(index) => i128::from(index).saturating_mul(per_bar),
        Anchor::Marker(tick) => i128::from(tick),
        Anchor::SegmentStart => 0,
    };
    let shift = match offset {
        CueOffset::Beats(beats) => i128::from(beats).saturating_mul(per_beat),
        CueOffset::Ms(ms) => i128::from(ms) / i128::from(timebase.ms_per_tick),
    };
    let tick = base.saturating_add(shift);
    if tick < 0 {
        return Err(AnchorError::BeforeZero { anchor, offset });
    }
    // Values past `u64::MAX` saturate: an authored anchor that far out is a
    // content bug, not a panic site.
    Ok(u64::try_from(tick).unwrap_or(u64::MAX))
}

/// Anchor failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AnchorError {
    /// The anchor plus offset lands before tick 0; both are carried.
    BeforeZero {
        /// Anchor the cue hangs from.
        anchor: Anchor,
        /// Offset applied to it.
        offset: CueOffset,
    },
    /// One timebase rate is zero; the field name is carried.
    BadTimebase {
        /// Zero-valued field name.
        field: &'static str,
    },
}

impl core::fmt::Display for AnchorError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BeforeZero { anchor, offset } => write!(
                f,
                "anchor {anchor} with offset {offset} lands before tick 0; recovery: raise the offset or move the anchor later"
            ),
            Self::BadTimebase { field } => write!(
                f,
                "timebase field {field} is 0; recovery: give ticks per minute, bpm, beats per bar and ms per tick values of at least 1"
            ),
        }
    }
}

impl std::error::Error for AnchorError {}

/// Forward-only tick clock. [`Self::advance`] keeps a raw position that may
/// lag after a backward request and a smoothed position that never falls; the
/// smoothed value is the one the mix reads.
///
/// Rounding: both positions are whole ticks. The clock snaps to the tick it
/// was advanced to and does not interpolate between ticks, so a backward
/// request holds the previous whole tick instead of easing down.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CueClock {
    raw_tick: u64,
    smoothed_tick: u64,
    held_requests: u64,
}

impl CueClock {
    /// Clock whose raw and smoothed positions both start at `start`. Pure.
    #[must_use]
    pub const fn new(start: u64) -> Self {
        Self {
            raw_tick: start,
            smoothed_tick: start,
            held_requests: 0,
        }
    }

    /// Last requested position, which a backward request may pull down.
    #[must_use]
    pub const fn raw_tick(&self) -> u64 {
        self.raw_tick
    }

    /// Position the mix reads: never below the last one it read.
    #[must_use]
    pub const fn smoothed_tick(&self) -> u64 {
        self.smoothed_tick
    }

    /// How many backward requests the clock held instead of reporting.
    #[must_use]
    pub const fn held_requests(&self) -> u64 {
        self.held_requests
    }

    /// Move by `ticks` (negative to step back) and return the smoothed
    /// position. The raw position clamps at 0; a request that would end below
    /// the smoothed position is held and counted. Pure.
    pub fn advance(&mut self, ticks: i64) -> u64 {
        let next = i128::from(self.raw_tick).saturating_add(i128::from(ticks));
        let next = u64::try_from(next.max(0)).unwrap_or(u64::MAX);
        self.raw_tick = next;
        if next < self.smoothed_tick {
            self.held_requests = self.held_requests.saturating_add(1);
        } else {
            self.smoothed_tick = next;
        }
        self.smoothed_tick
    }

    /// FNV-1a64 over the raw tick, the smoothed tick and the held count,
    /// integers only. Same sequence, same digest. Pure.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut hash = Fnv64::new();
        hash.write_u64(self.raw_tick);
        hash.write_u64(self.smoothed_tick);
        hash.write_u64(self.held_requests);
        hash.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 120 BPM on a 960 tick-per-minute grid: 8 ticks per beat, 4/4 bars of
    /// 32 ticks, 10 ms per tick.
    fn timebase() -> CueTimebase {
        CueTimebase::new(960, 120, 4, 10).expect("the fixture rates are legal")
    }

    #[test]
    fn a_downbeat_minus_one_beat_lands_one_beat_early() {
        let timebase = timebase();
        assert_eq!(timebase.ticks_per_beat(), 8);
        assert_eq!(timebase.ticks_per_bar(), 32);
        let downbeat = resolve(Anchor::Downbeat(2), CueOffset::Beats(0), &timebase)
            .expect("bar 2 is after tick 0");
        assert_eq!(downbeat, 64);
        let early = resolve(Anchor::Downbeat(2), CueOffset::Beats(-1), &timebase)
            .expect("one beat before tick 64 is still positive");
        assert_eq!(early, 56);
        assert_eq!(early, downbeat - timebase.ticks_per_beat());
        assert_eq!(
            early,
            resolve(Anchor::Beat(7), CueOffset::Beats(0), &timebase)
                .expect("the fixture resolves cleanly")
        );
        // One tick is one beat only when the caller says so.
        let one_to_one = CueTimebase::new(120, 120, 4, 10).expect("rates are legal");
        assert_eq!(one_to_one.ticks_per_beat(), 1);
        assert_eq!(
            resolve(Anchor::Beat(7), CueOffset::Beats(0), &one_to_one)
                .expect("the fixture resolves cleanly"),
            7
        );
    }

    #[test]
    fn a_marker_anchor_resolves_to_its_tick_and_ms_offsets_use_ms_per_tick() {
        let timebase = timebase();
        assert_eq!(
            resolve(Anchor::Marker(480), CueOffset::Beats(0), &timebase)
                .expect("the fixture resolves cleanly"),
            480
        );
        assert_eq!(
            resolve(Anchor::Marker(480), CueOffset::Beats(1), &timebase)
                .expect("the fixture resolves cleanly"),
            488
        );
        assert_eq!(
            resolve(Anchor::SegmentStart, CueOffset::Ms(250), &timebase)
                .expect("the fixture resolves cleanly"),
            25
        );
        assert_eq!(
            resolve(Anchor::Marker(10), CueOffset::Ms(-25), &timebase)
                .expect("the fixture resolves cleanly"),
            8,
            "integer division truncates toward zero"
        );
    }

    #[test]
    fn an_offset_before_zero_is_refused_with_the_anchor_and_offset() {
        let error = resolve(Anchor::Beat(0), CueOffset::Beats(-1), &timebase())
            .expect_err("beat 0 minus one beat is negative");
        assert_eq!(
            error,
            AnchorError::BeforeZero {
                anchor: Anchor::Beat(0),
                offset: CueOffset::Beats(-1),
            }
        );
        let text = error.to_string();
        assert!(text.contains("Beat(0)"), "anchor named: {text}");
        assert!(text.contains("Beats(-1)"), "offset named: {text}");
        assert!(text.contains("recovery:"), "recovery named: {text}");

        let zero = CueTimebase::new(960, 0, 4, 10).expect_err("bpm 0 is refused");
        assert_eq!(zero, AnchorError::BadTimebase { field: "bpm" });
        assert!(zero.to_string().contains("recovery:"));
    }

    #[test]
    fn a_backwards_advance_holds_the_last_value_and_the_digest_is_stable() {
        let mut first = CueClock::new(0);
        assert_eq!(first.advance(120), 120);
        assert_eq!(first.advance(-5), 120, "a backward request is held");
        assert_eq!(first.smoothed_tick(), 120);
        assert_eq!(first.raw_tick(), 115);
        assert_eq!(first.held_requests(), 1);

        let mut second = CueClock::new(0);
        second.advance(120);
        second.advance(-5);
        assert_eq!(first.digest(), second.digest(), "two runs agree");
        assert_eq!(first.smoothed_tick(), second.smoothed_tick());

        let held = first.digest();
        assert_eq!(first.advance(-5), 120);
        assert_eq!(first.held_requests(), 2);
        assert_ne!(first.digest(), held, "the hold count is in the receipt");

        // Raw clamps at 0, and the smoothed value still never falls.
        assert_eq!(first.advance(-10_000), 120);
        assert_eq!(first.raw_tick(), 0);
        let mut forward = CueClock::new(0);
        assert_eq!(forward.advance(7), 7);
        assert_eq!(forward.advance(-3), 7);
        assert_eq!(forward.advance(10), 14);
    }
}
