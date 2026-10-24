//! cue_sheet: authored cue rows for one scene, parsed and gated as data.
//!
//! A [`CueSheet`] is a caller-built list of [`CueRow`]s: tick, sound id, bus,
//! gain, duck, loop range and priority. [`CueSheet::new`] parses and validates
//! them into [`Cue`]s, [`CueSheet::cues_at`] picks what sounds on one tick in a
//! stable order, and [`CueSheet::gap_report`] lists ticks with no music or
//! ambience bed. Data owner: `engine-audio`. Units: ticks are fixed steps,
//! gains are whole dB (`i16`), priority is `u8` and higher wins.
//!
//! Policy for overlap: two looping ranges on the same bus that overlap are
//! **refused** at parse time with [`CueSheetError::LoopOverlap`]. A sheet is
//! therefore unambiguous: at most one loop runs on a bus at any tick, and a
//! caller that wants a crossfade authors a non-looping row beside the loop.
//! Two one-shot rows on one tick are legal (both play, order fixed by
//! `cues_at`), and a loop may start exactly when the previous loop ends.
//!
//! Determinism: parsing keeps authored order; `cues_at` sorts by tick, then
//! priority (high first), then sound id, then authored index; [`CueSheet::digest`]
//! hashes integers only. No clock, RNG or file IO; the one allocation is the
//! query vector a caller asks for.

use engine_core::Fnv64;

use crate::cue::CueId;
use crate::dsp::Bus;

/// Duck depth a ducked music row applies to the music lane, in dB.
pub const DUCK_DEPTH_DB: i16 = 12;
/// Quietest authored row gain, in dB.
pub const MIN_GAIN_DB: i16 = -80;
/// Loudest authored row gain, in dB.
pub const MAX_GAIN_DB: i16 = 6;
/// Most rows one sheet may hold.
pub const MAX_CUES: usize = 512;
/// Widest tick span [`CueSheet::gap_report`] scans.
pub const MAX_GAP_SPAN_TICKS: u64 = 100_000;
/// Ambience lane trim in dB, between the music and sfx buses.
pub const GAIN_AMBIENCE_DB: i16 = -9;

/// The four cue-sheet lanes. Three map onto the mixer's [`Bus`] set; the
/// ambience lane rides the sfx bus, and voice is the dialogue bus.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CueBus {
    /// Music bed.
    Music,
    /// Effects and impacts.
    Sfx,
    /// Spoken lines.
    Voice,
    /// Room and crowd bed.
    Ambience,
}

impl CueBus {
    /// Every lane in fixed order: music, sfx, voice, ambience.
    pub const ALL: [Self; 4] = [Self::Music, Self::Sfx, Self::Voice, Self::Ambience];

    /// Lane name as an authored row writes it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Music => "music",
            Self::Sfx => "sfx",
            Self::Voice => "voice",
            Self::Ambience => "ambience",
        }
    }

    /// Parse a lane name; `None` for anything outside [`CueBus::ALL`]. Pure.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "music" => Some(Self::Music),
            "sfx" => Some(Self::Sfx),
            "voice" => Some(Self::Voice),
            "ambience" => Some(Self::Ambience),
            _ => None,
        }
    }

    /// Mixer bus this lane feeds. Voice is the dialogue bus and ambience
    /// rides the sfx bus, so the sheet adds no fourth mixer bus. Pure.
    #[must_use]
    pub const fn mix_bus(self) -> Bus {
        match self {
            Self::Music => Bus::Music,
            Self::Sfx | Self::Ambience => Bus::Sfx,
            Self::Voice => Bus::Dialogue,
        }
    }

    /// Lane trim in dB, the same numbers the mixer's [`Bus`] uses.
    #[must_use]
    pub const fn base_gain_db(self) -> i16 {
        match self {
            Self::Music => -12,
            Self::Sfx => -3,
            Self::Voice => -6,
            Self::Ambience => GAIN_AMBIENCE_DB,
        }
    }

    /// One-byte digest code; never changes for a lane.
    const fn code(self) -> u8 {
        match self {
            Self::Music => 0,
            Self::Sfx => 1,
            Self::Voice => 2,
            Self::Ambience => 3,
        }
    }
}

impl core::fmt::Display for CueBus {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// One authored row as the caller builds it. `bus` is the lane name, so a
/// typo is a typed error instead of a silent lane change.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CueRow {
    /// Fixed tick the row starts on.
    pub tick: u64,
    /// Sound to play: an index into the caller's cue bank.
    pub sound: CueId,
    /// Lane name: `"music"`, `"sfx"`, `"voice"` or `"ambience"`.
    pub bus: &'static str,
    /// Row gain in dB, `MIN_GAIN_DB..=MAX_GAIN_DB`.
    pub gain_db: i16,
    /// Lower the music lane by [`DUCK_DEPTH_DB`] while this row sounds.
    /// Legal on the music lane only.
    pub duck: bool,
    /// Loop until this tick (exclusive); `None` plays once at `tick`.
    pub loop_until: Option<u64>,
    /// Playback priority, higher wins when voices are stolen or sorted.
    pub priority: u8,
}

/// One parsed, valid row.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Cue {
    /// Fixed tick the row starts on.
    pub tick: u64,
    /// Sound to play.
    pub sound: CueId,
    /// Parsed lane.
    pub bus: CueBus,
    /// Row gain in dB.
    pub gain_db: i16,
    /// True when the row ducks the music lane.
    pub duck: bool,
    /// Loop end tick (exclusive), if the row loops.
    pub loop_until: Option<u64>,
    /// Playback priority.
    pub priority: u8,
}

impl Cue {
    /// True when the row sounds at `tick`: exactly its start tick for a
    /// one-shot, or `tick..loop_until` for a loop. Pure.
    #[must_use]
    pub fn active_at(&self, tick: u64) -> bool {
        match self.loop_until {
            Some(end) => self.tick <= tick && tick < end,
            None => self.tick == tick,
        }
    }
}

/// Lane trims at one tick, in dB, duck already applied.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BusGains {
    /// Music lane trim.
    pub music_db: i16,
    /// Effects lane trim.
    pub sfx_db: i16,
    /// Voice lane trim.
    pub voice_db: i16,
    /// Ambience lane trim.
    pub ambience_db: i16,
}

impl BusGains {
    /// Trim for one lane. Pure.
    #[must_use]
    pub const fn get(self, bus: CueBus) -> i16 {
        match bus {
            CueBus::Music => self.music_db,
            CueBus::Sfx => self.sfx_db,
            CueBus::Voice => self.voice_db,
            CueBus::Ambience => self.ambience_db,
        }
    }
}

/// Ticks in the scanned span with neither a music nor an ambience row.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GapReport {
    /// First tick of the scanned span (the sheet's first row tick).
    pub first_tick: u64,
    /// Exclusive end of the scanned span (the sheet's last end tick).
    pub last_tick: u64,
    /// Ticks with no bed, ascending.
    pub ticks: Vec<u64>,
}

impl GapReport {
    /// Ticks with no bed, ascending.
    #[must_use]
    pub fn ticks(&self) -> &[u64] {
        &self.ticks
    }

    /// Number of bedless ticks.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ticks.len()
    }

    /// True when the whole span had a bed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ticks.is_empty()
    }
}

/// Cue-sheet failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CueSheetError {
    /// A row names a lane outside the four legal names; culprit carried.
    BadBus(&'static str),
    /// More than [`MAX_CUES`] rows, count carried.
    TooManyRows(usize),
    /// A row's gain is outside `MIN_GAIN_DB..=MAX_GAIN_DB`; row and gain
    /// carried.
    GainOutOfRange {
        /// Row index (0-based).
        row: usize,
        /// Offending gain in dB.
        gain_db: i16,
    },
    /// `duck` is set on a lane other than music; row and lane carried.
    DuckNotMusic {
        /// Row index (0-based).
        row: usize,
        /// Offending lane.
        bus: CueBus,
    },
    /// A loop ends at or before its start; row, start and end carried.
    BadLoopRange {
        /// Row index (0-based).
        row: usize,
        /// Start tick.
        tick: u64,
        /// Non-positive loop end tick.
        loop_until: u64,
    },
    /// Two looping ranges overlap on one lane; both row indexes carried.
    LoopOverlap {
        /// Lane both rows loop on.
        bus: CueBus,
        /// Earlier row index (0-based).
        first: usize,
        /// Later row index (0-based).
        second: usize,
    },
    /// The gap scan is wider than [`MAX_GAP_SPAN_TICKS`]; span carried.
    GapSpanTooLong {
        /// Requested span in ticks.
        span: u64,
    },
}

impl core::fmt::Display for CueSheetError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadBus(name) => write!(
                f,
                "bad bus {name:?}: legal values are music, sfx, voice, ambience; recovery: rename the row's bus"
            ),
            Self::TooManyRows(n) => write!(
                f,
                "{n} rows over the {MAX_CUES} cap; recovery: split the sheet"
            ),
            Self::GainOutOfRange { row, gain_db } => write!(
                f,
                "row {row}: gain {gain_db} dB outside {MIN_GAIN_DB}..={MAX_GAIN_DB}; recovery: clamp the row's gain"
            ),
            Self::DuckNotMusic { row, bus } => write!(
                f,
                "row {row}: duck on the {bus} lane; duck lowers music only; recovery: set the row's bus to music or clear duck"
            ),
            Self::BadLoopRange {
                row,
                tick,
                loop_until,
            } => write!(
                f,
                "row {row}: loop_until {loop_until} must be past tick {tick}; recovery: raise loop_until or drop the loop"
            ),
            Self::LoopOverlap {
                bus,
                first,
                second,
            } => write!(
                f,
                "rows {first} and {second} loop over each other on the {bus} lane; recovery: end the earlier loop at or before the later one starts"
            ),
            Self::GapSpanTooLong { span } => write!(
                f,
                "gap scan span {span} over the {MAX_GAP_SPAN_TICKS} cap; recovery: scan a shorter sheet"
            ),
        }
    }
}

impl std::error::Error for CueSheetError {}

/// Validated cue sheet: authored rows in order, with the lane trims and the
/// queries the scene loader asks for.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CueSheet {
    rows: Vec<Cue>,
}

impl CueSheet {
    /// Parse and validate caller-built rows. Two looping ranges on one lane
    /// that overlap are refused, never resolved by a pick. Pure.
    ///
    /// # Errors
    ///
    /// Returns [`CueSheetError`] for a bad lane name, an out-of-range gain, a
    /// duck outside music, a reversed loop range, overlapping loops on one
    /// lane, or too many rows.
    pub fn new(rows: &[CueRow]) -> Result<Self, CueSheetError> {
        if rows.len() > MAX_CUES {
            return Err(CueSheetError::TooManyRows(rows.len()));
        }
        let mut cues: Vec<Cue> = Vec::with_capacity(rows.len());
        for (index, row) in rows.iter().enumerate() {
            let bus = CueBus::from_name(row.bus).ok_or(CueSheetError::BadBus(row.bus))?;
            if row.gain_db < MIN_GAIN_DB || row.gain_db > MAX_GAIN_DB {
                return Err(CueSheetError::GainOutOfRange {
                    row: index,
                    gain_db: row.gain_db,
                });
            }
            if row.duck && bus != CueBus::Music {
                return Err(CueSheetError::DuckNotMusic { row: index, bus });
            }
            if let Some(end) = row.loop_until {
                if end <= row.tick {
                    return Err(CueSheetError::BadLoopRange {
                        row: index,
                        tick: row.tick,
                        loop_until: end,
                    });
                }
            }
            if row.loop_until.is_some() {
                for (other_index, other) in cues.iter().enumerate() {
                    if other.bus == bus && other.loop_until.is_some() && loops_overlap(other, row) {
                        return Err(CueSheetError::LoopOverlap {
                            bus,
                            first: other_index,
                            second: index,
                        });
                    }
                }
            }
            cues.push(Cue {
                tick: row.tick,
                sound: row.sound,
                bus,
                gain_db: row.gain_db,
                duck: row.duck,
                loop_until: row.loop_until,
                priority: row.priority,
            });
        }
        Ok(Self { rows: cues })
    }

    /// Parsed rows in authored order.
    #[must_use]
    pub fn rows(&self) -> &[Cue] {
        &self.rows
    }

    /// Row count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when the sheet holds no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Rows sounding at `tick`, sorted by start tick, then priority (high
    /// first), then sound id, then authored index. Pure; allocates the
    /// returned vector only.
    #[must_use]
    pub fn cues_at(&self, tick: u64) -> Vec<&Cue> {
        let mut picks: Vec<(usize, &Cue)> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, cue)| cue.active_at(tick))
            .collect();
        picks.sort_by(|(left_index, left), (right_index, right)| {
            left.tick
                .cmp(&right.tick)
                .then_with(|| right.priority.cmp(&left.priority))
                .then_with(|| left.sound.0.cmp(&right.sound.0))
                .then_with(|| left_index.cmp(right_index))
        });
        picks.into_iter().map(|(_, cue)| cue).collect()
    }

    /// Lane trims at `tick`: each lane's base trim, with the music lane
    /// lowered by [`DUCK_DEPTH_DB`] while any ducked music row sounds. No
    /// other lane moves. Pure.
    #[must_use]
    pub fn bus_gains_at(&self, tick: u64) -> BusGains {
        let ducked = self
            .rows
            .iter()
            .any(|cue| cue.duck && cue.bus == CueBus::Music && cue.active_at(tick));
        let music_db = if ducked {
            CueBus::Music.base_gain_db().saturating_sub(DUCK_DEPTH_DB)
        } else {
            CueBus::Music.base_gain_db()
        };
        BusGains {
            music_db,
            sfx_db: CueBus::Sfx.base_gain_db(),
            voice_db: CueBus::Voice.base_gain_db(),
            ambience_db: CueBus::Ambience.base_gain_db(),
        }
    }

    /// Ticks from the sheet's first row tick up to its last end tick with
    /// neither a music nor an ambience row sounding. Pure.
    ///
    /// # Errors
    ///
    /// Returns [`CueSheetError::GapSpanTooLong`] when the span exceeds
    /// [`MAX_GAP_SPAN_TICKS`].
    pub fn gap_report(&self) -> Result<GapReport, CueSheetError> {
        let first_tick = self.rows.iter().map(|cue| cue.tick).min().unwrap_or(0);
        let last_tick = self
            .rows
            .iter()
            .map(|cue| cue.loop_until.unwrap_or_else(|| cue.tick.saturating_add(1)))
            .max()
            .unwrap_or(0);
        let span = last_tick.saturating_sub(first_tick);
        if span > MAX_GAP_SPAN_TICKS {
            return Err(CueSheetError::GapSpanTooLong { span });
        }
        let mut ticks = Vec::new();
        let mut tick = first_tick;
        while tick < last_tick {
            let bedded = self.rows.iter().any(|cue| {
                (cue.bus == CueBus::Music || cue.bus == CueBus::Ambience) && cue.active_at(tick)
            });
            if !bedded {
                ticks.push(tick);
            }
            tick = tick.saturating_add(1);
        }
        Ok(GapReport {
            first_tick,
            last_tick,
            ticks,
        })
    }

    /// FNV-1a64 over every parsed row, integers only, field widths fixed.
    /// Same rows give the same digest twice. Pure.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut hash = Fnv64::new();
        for cue in &self.rows {
            hash.write_u64(cue.tick);
            hash.write_u16(cue.sound.0);
            hash.write_u8(cue.bus.code());
            hash.write_i32(i32::from(cue.gain_db));
            hash.write_bool(cue.duck);
            hash.write_u64(cue.loop_until.unwrap_or(u64::MAX));
            hash.write_u8(cue.priority);
        }
        hash.finish()
    }
}

/// True when two parsed looping rows share at least one tick. Pure.
fn loops_overlap(first: &Cue, second: &CueRow) -> bool {
    let Some(first_end) = first.loop_until else {
        return false;
    };
    let Some(second_end) = second.loop_until else {
        return false;
    };
    first.tick < second_end && second.tick < first_end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        tick: u64,
        sound: u16,
        bus: &'static str,
        gain_db: i16,
        duck: bool,
        loop_until: Option<u64>,
        priority: u8,
    ) -> CueRow {
        CueRow {
            tick,
            sound: CueId(sound),
            bus,
            gain_db,
            duck,
            loop_until,
            priority,
        }
    }

    /// Four rows over three ticks: an ambience loop, a music duck and two
    /// one-shots.
    fn four_row_sheet() -> CueSheet {
        CueSheet::new(&[
            row(0, 10, "ambience", -20, false, Some(30), 5),
            row(10, 20, "sfx", -3, false, None, 2),
            row(10, 5, "music", -6, true, None, 7),
            row(20, 30, "voice", 0, false, None, 1),
        ])
        .expect("four fixture rows are legal")
    }

    fn sounds(picks: &[&Cue]) -> Vec<u16> {
        picks.iter().map(|cue| cue.sound.0).collect()
    }

    #[test]
    fn a_four_row_sheet_picks_the_expected_rows_at_three_ticks() {
        let sheet = four_row_sheet();
        assert_eq!(sounds(&sheet.cues_at(0)), vec![10]);
        // Same tick 10: the priority-7 music row comes before the sfx row,
        // and the ambience loop that started at tick 0 comes before both.
        assert_eq!(sounds(&sheet.cues_at(10)), vec![10, 5, 20]);
        assert_eq!(sounds(&sheet.cues_at(20)), vec![10, 30]);
        // The ambience loop ends at tick 30, exclusive.
        assert!(sheet.cues_at(30).is_empty());
        assert_eq!(sheet.len(), 4);
        assert!(!sheet.is_empty());
    }

    #[test]
    fn a_ducked_music_row_lowers_only_the_music_lane() {
        let sheet = four_row_sheet();
        let ducked = sheet.bus_gains_at(10);
        let quiet = sheet.bus_gains_at(0);
        assert_eq!(
            ducked.music_db,
            CueBus::Music.base_gain_db() - DUCK_DEPTH_DB
        );
        assert_eq!(quiet.music_db, CueBus::Music.base_gain_db());
        assert_eq!(
            (ducked.sfx_db, ducked.voice_db, ducked.ambience_db),
            (quiet.sfx_db, quiet.voice_db, quiet.ambience_db),
            "only the music lane moves"
        );
        assert_eq!(
            ducked.get(CueBus::Music),
            -12 - DUCK_DEPTH_DB,
            "the mixer's music trim plus the duck"
        );
        assert_eq!(
            f32::from(CueBus::Music.base_gain_db()),
            Bus::Music.gain_db(),
            "sheet trims reuse the mixer's numbers"
        );
        assert_eq!(
            f32::from(CueBus::Voice.base_gain_db()),
            Bus::Dialogue.gain_db()
        );
        assert_eq!(f32::from(CueBus::Sfx.base_gain_db()), Bus::Sfx.gain_db());
        assert_eq!(CueBus::Voice.mix_bus(), Bus::Dialogue);
        assert_eq!(CueBus::Ambience.mix_bus(), Bus::Sfx);
    }

    #[test]
    fn a_bad_bus_names_the_legal_lanes_and_a_recovery() {
        let error = CueSheet::new(&[row(0, 1, "dialog", 0, false, None, 0)])
            .expect_err("dialog is not a lane");
        assert_eq!(error, CueSheetError::BadBus("dialog"));
        let text = error.to_string();
        for lane in ["music", "sfx", "voice", "ambience"] {
            assert!(text.contains(lane), "legal lane {lane} named: {text}");
        }
        assert!(text.contains("recovery:"), "a recovery is named: {text}");
        for bus in CueBus::ALL {
            assert_eq!(CueBus::from_name(bus.name()), Some(bus));
        }
    }

    #[test]
    fn two_runs_of_the_same_rows_are_equal() {
        let rows = [
            row(0, 10, "ambience", -20, false, Some(30), 5),
            row(10, 20, "sfx", -3, false, None, 2),
            row(10, 5, "music", -6, true, None, 7),
            row(20, 30, "voice", 0, false, None, 1),
        ];
        let first = CueSheet::new(&rows).expect("fixture rows are legal");
        let second = CueSheet::new(&rows).expect("fixture rows are legal");
        assert_eq!(first.digest(), second.digest());
        assert_eq!(first.rows(), second.rows());
        for tick in [0u64, 10, 20, 30] {
            assert_eq!(sounds(&first.cues_at(tick)), sounds(&second.cues_at(tick)));
            assert_eq!(first.bus_gains_at(tick), second.bus_gains_at(tick));
        }
        assert_eq!(first, second);
    }

    #[test]
    fn overlapping_loops_on_one_lane_are_refused() {
        let overlap = CueSheet::new(&[
            row(0, 1, "music", 0, false, Some(100), 0),
            row(50, 2, "music", 0, false, Some(150), 0),
        ])
        .expect_err("loops overlap on music");
        assert_eq!(
            overlap,
            CueSheetError::LoopOverlap {
                bus: CueBus::Music,
                first: 0,
                second: 1,
            }
        );
        assert!(overlap.to_string().contains("recovery:"));

        // Touching ranges and a loop on another lane are legal.
        CueSheet::new(&[
            row(0, 1, "music", 0, false, Some(100), 0),
            row(100, 2, "music", 0, false, Some(150), 0),
            row(0, 3, "ambience", 0, false, Some(150), 0),
        ])
        .expect("touching loops and a second lane are legal");

        let same_tick = CueSheet::new(&[
            row(0, 1, "music", 0, false, Some(10), 0),
            row(0, 2, "music", 0, false, Some(10), 0),
        ])
        .expect_err("two loops on one tick and one lane overlap");
        assert_eq!(
            same_tick,
            CueSheetError::LoopOverlap {
                bus: CueBus::Music,
                first: 0,
                second: 1,
            }
        );
    }

    #[test]
    fn gap_report_lists_ticks_with_no_bed() {
        let sheet = CueSheet::new(&[
            row(0, 1, "ambience", 0, false, Some(4), 0),
            row(0, 2, "voice", 0, false, None, 0),
            row(10, 3, "music", 0, false, Some(14), 0),
        ])
        .expect("fixture rows are legal");
        let gaps = sheet.gap_report().expect("a short sheet scans");
        assert_eq!((gaps.first_tick, gaps.last_tick), (0, 14));
        assert_eq!(gaps.ticks(), &[4, 5, 6, 7, 8, 9]);
        assert_eq!(gaps.len(), 6);
        assert!(!gaps.is_empty());

        let bedded = CueSheet::new(&[row(0, 1, "music", 0, false, Some(100), 0)])
            .expect("one loop is legal");
        assert!(bedded.gap_report().expect("a short sheet scans").is_empty());
        let empty = CueSheet::new(&[]).expect("an empty sheet is legal");
        assert!(empty.gap_report().expect("empty scans").is_empty());
    }

    #[test]
    fn bad_rows_are_typed_errors() {
        let duck = CueSheet::new(&[row(0, 1, "sfx", 0, true, None, 0)])
            .expect_err("duck is a music-only flag");
        assert_eq!(
            duck,
            CueSheetError::DuckNotMusic {
                row: 0,
                bus: CueBus::Sfx,
            }
        );
        assert!(duck.to_string().contains("music"));

        let gain = CueSheet::new(&[row(0, 1, "sfx", MAX_GAIN_DB + 1, false, None, 0)])
            .expect_err("gain over the cap");
        assert_eq!(
            gain,
            CueSheetError::GainOutOfRange {
                row: 0,
                gain_db: MAX_GAIN_DB + 1,
            }
        );
        assert!(gain.to_string().contains("-80..=6"));

        let range = CueSheet::new(&[row(20, 1, "sfx", 0, false, Some(20), 0)])
            .expect_err("an empty loop range");
        assert_eq!(
            range,
            CueSheetError::BadLoopRange {
                row: 0,
                tick: 20,
                loop_until: 20,
            }
        );

        let many = vec![row(0, 1, "sfx", 0, false, None, 0); MAX_CUES + 1];
        assert_eq!(
            CueSheet::new(&many),
            Err(CueSheetError::TooManyRows(MAX_CUES + 1))
        );
    }

    #[test]
    fn a_long_span_stops_the_gap_scan() {
        let sheet = CueSheet::new(&[
            row(0, 1, "voice", 0, false, None, 0),
            row(MAX_GAP_SPAN_TICKS + 10, 2, "voice", 0, false, None, 0),
        ])
        .expect("two one-shots are legal");
        let error = sheet.gap_report().expect_err("span is over the cap");
        assert_eq!(
            error,
            CueSheetError::GapSpanTooLong {
                span: MAX_GAP_SPAN_TICKS + 11,
            }
        );
        assert!(error.to_string().contains("recovery:"));
    }
}
