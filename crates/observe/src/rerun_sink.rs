//! Rerun recording of simulation state per tick for desync hunts.
//!
//! Consumer: the headless paired-rerun harness reads two sinks and names
//! the first divergent tick; the crash path in `crates/game` pushes the ring;
//! the supervisor keeps its own tape. Data owner: [`RerunSink`] owns a
//! 120-frame ring of (tick, hash) rows plus a 32-row ring of heal rows (the
//! proof of every heal that ran); the caller owns the hashed bytes. Units:
//! ticks (`u64`), state hashes (`u64` FNV-1a over little-endian words
//! supplied by the caller). Determinism: insertion order kept, oldest-first
//! iteration, saturating counters, no clock. Errors: none; push never fails
//! and over-capacity rows evict the oldest while counting overflow. Budget:
//! 120 rows (1,920 bytes) inline plus 32 heal rows inline; zero heap
//! allocation per push. Tracy span names are `&str` constants beside the
//! supervisor; there is no Tracy SDK dependency (licenses reviewed, nothing
//! vendored).
//!
//! Pixels, GPU passes and VRAM accounting stay out: deeper capture (2,000
//! frames, per-pass timers, manifest legs) belongs to the pilot harness. A
//! longer or richer capture goes straight into a [`ReplayRecording`]
//! (MCAP, no ring), and [`RerunSink::to_mcap`] exports the ring the same way.

use crate::mcap::{McapError, ReplayRecording};
use crate::supervisor::HealTapeRecord;

/// Maximum frames retained. The ring holds this many rows; deeper capture
/// stays with the pilot harness and never grows this type.
pub const MAX_RERUN_FRAMES: usize = 120;

/// Maximum heal rows retained beside the frames. A heal row is bigger than a
/// frame row, so the heal ring is smaller. Older rows evict and the drop
/// lands in the same overflow counter: nothing is dropped silently.
pub const MAX_RERUN_HEAL_ROWS: usize = 32;

/// One recorded tick: the tick number plus the simulation state hash for it.
/// The hash is opaque here; the caller hashes with the shared FNV-1a owner
/// over little-endian words so every platform records the same bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RerunFrame {
    /// Simulation tick (`u64`).
    pub tick: u64,
    /// State hash (`u64`) for `tick`.
    pub state_hash: u64,
}

/// Bounded oldest-first ring of [`RerunFrame`] rows. When full, pushing
/// evicts the oldest row and counts the drop via [`RerunSink::overflow_count`];
/// rows are never dropped silently. Backing storage is inline, so a push
/// performs no heap allocation.
#[derive(Debug, Clone)]
pub struct RerunSink {
    frames: [RerunFrame; MAX_RERUN_FRAMES],
    start: usize,
    len: usize,
    overflow: u64,
    /// Heal rows beside the frames: the proof of every heal that ran, so a
    /// hunt can name the first divergent tick and the heal that caused it.
    heals: [Option<HealTapeRecord>; MAX_RERUN_HEAL_ROWS],
    heal_start: usize,
    heal_rows: usize,
}

impl RerunSink {
    /// Empty sink holding zero of [`MAX_RERUN_FRAMES`] rows.
    #[must_use]
    pub fn new() -> Self {
        Self {
            frames: [RerunFrame {
                tick: 0,
                state_hash: 0,
            }; MAX_RERUN_FRAMES],
            start: 0,
            len: 0,
            overflow: 0,
            heals: [None; MAX_RERUN_HEAL_ROWS],
            heal_start: 0,
            heal_rows: 0,
        }
    }

    /// Maximum frames retained. Always [`MAX_RERUN_FRAMES`].
    #[must_use]
    pub fn capacity(&self) -> usize {
        MAX_RERUN_FRAMES
    }

    /// Frames currently retained, at most [`MAX_RERUN_FRAMES`].
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when no frames are retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Rows dropped because the ring was full. Monotonic.
    #[must_use]
    pub fn overflow_count(&self) -> u64 {
        self.overflow
    }

    /// Record one tick. Evicts the oldest row when full and counts the drop;
    /// never fails and never allocates. Callers should push ticks in order so
    /// [`RerunSink::first_divergent`] compares matching positions.
    pub fn push(&mut self, tick: u64, state_hash: u64) {
        let frame = RerunFrame { tick, state_hash };
        if self.len < MAX_RERUN_FRAMES {
            let pos = (self.start + self.len) % MAX_RERUN_FRAMES;
            self.frames[pos] = frame;
            self.len += 1;
        } else {
            self.frames[self.start] = frame;
            self.start = (self.start + 1) % MAX_RERUN_FRAMES;
            self.overflow = self.overflow.saturating_add(1);
        }
    }

    /// Oldest-first iterator over retained frames.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &RerunFrame> + '_ {
        (0..self.len).map(move |index| {
            let pos = (self.start + index) % MAX_RERUN_FRAMES;
            &self.frames[pos]
        })
    }

    /// Record one heal row beside the frames. Evicts the oldest row when the
    /// heal ring is full and counts the drop in the same overflow counter;
    /// never fails and never allocates.
    pub fn push_heal(&mut self, row: &HealTapeRecord) {
        let pos = (self.heal_start + self.heal_rows) % MAX_RERUN_HEAL_ROWS;
        if self.heal_rows < MAX_RERUN_HEAL_ROWS {
            self.heals[pos] = Some(*row);
            self.heal_rows += 1;
        } else {
            self.heals[self.heal_start] = Some(*row);
            self.heal_start = (self.heal_start + 1) % MAX_RERUN_HEAL_ROWS;
            self.overflow = self.overflow.saturating_add(1);
        }
    }

    /// Oldest-first iterator over the retained heal rows.
    pub fn heal_rows(&self) -> impl DoubleEndedIterator<Item = &HealTapeRecord> + '_ {
        (0..self.heal_rows).filter_map(move |index| {
            let pos = (self.heal_start + index) % MAX_RERUN_HEAL_ROWS;
            self.heals[pos].as_ref()
        })
    }

    /// Number of heal rows currently retained.
    #[must_use]
    pub fn heal_len(&self) -> usize {
        self.heal_rows
    }

    /// The first heal row that was not proved (not recovered and not rolled
    /// back), oldest first. `None` when every heal on the sink proved itself.
    #[must_use]
    pub fn first_unproved_heal(&self) -> Option<&HealTapeRecord> {
        self.heal_rows().find(|row| !row.validation.is_green())
    }

    /// First tick where the two recordings disagree, oldest-first. Compares
    /// position by position: a tick or hash mismatch returns the tick from
    /// `self` at that position; when one recording is a strict prefix of the
    /// other, returns the first extra tick. Returns `None` when both hold the
    /// same rows. Determinism: pure comparison over stored rows, no clock.
    #[must_use]
    pub fn first_divergent(&self, other: &Self) -> Option<u64> {
        let mut left = self.iter();
        let mut right = other.iter();
        loop {
            match (left.next(), right.next()) {
                (Some(a), Some(b)) => {
                    if a.tick != b.tick || a.state_hash != b.state_hash {
                        return Some(a.tick);
                    }
                }
                (Some(a), None) => return Some(a.tick),
                (None, Some(b)) => return Some(b.tick),
                (None, None) => return None,
            }
        }
    }
}

impl RerunSink {
    /// Export the retained rows as an MCAP recording with one `/tick/hash`
    /// message per row, oldest first; [`crate::mcap::first_divergent_tick`]
    /// on two parsed exports names the same tick as
    /// [`RerunSink::first_divergent`]. Rows beyond the 120-frame ring are
    /// gone; for a longer capture feed a [`ReplayRecording`] directly.
    ///
    /// # Errors
    ///
    /// Returns [`McapError`] only for an impossible size overflow.
    pub fn to_mcap(&self) -> Result<Vec<u8>, McapError> {
        let mut recording = ReplayRecording::new()?;
        for frame in self.iter() {
            recording.tick_hash(frame.tick, frame.state_hash)?;
        }
        Ok(recording.finish())
    }
}

impl Default for RerunSink {
    /// Empty sink; same as [`RerunSink::new`].
    fn default() -> Self {
        Self::new()
    }
}

/// Tracy zone names for rerun work beside the supervisor catalog. String
/// constants only; there is no Tracy SDK dependency, so zones stay
/// tick-based and deterministic in tests while adapters attach wall time.
pub mod tracy {
    /// Span around recording one frame into the sink.
    pub const RERUN_RECORD: &str = "rerun.record";
    /// Span around comparing two sinks for the first divergent tick.
    pub const RERUN_COMPARE: &str = "rerun.compare";
    /// Zone around the record path per tick; record leg of the hunt.
    pub const RECORD_TICK: &str = "rerun.record.tick";
    /// Zone around the compare scan; scan leg of the hunt.
    pub const COMPARE_SCAN: &str = "rerun.compare.scan";
    /// Frame-mark label; one mark per presented frame for BENCH joins.
    pub const FRAME: &str = "rerun.frame";
    /// Plot series for per-tick ms beside the perf `tick` marks.
    pub const PLOT_TICK: &str = "rerun.plot.tick";
    /// Plot series for per-frame ms beside the perf `frame` marks.
    pub const PLOT_FRAME: &str = "rerun.plot.frame";
    /// Every zone name in report order; BENCH joins on these string keys.
    pub const ZONES: &[&str] = &[
        RERUN_RECORD,
        RERUN_COMPARE,
        RECORD_TICK,
        COMPARE_SCAN,
        FRAME,
        PLOT_TICK,
        PLOT_FRAME,
    ];
    /// Typed refusal for names outside [`ZONES`].
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum ZoneError {
        /// Name outside the zone set.
        UnknownZone {
            /// Rejected zone name.
            zone: String,
        },
    }
    impl core::fmt::Display for ZoneError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            match self {
                Self::UnknownZone { zone } => write!(f, "unknown rerun zone: {zone}"),
            }
        }
    }
    impl std::error::Error for ZoneError {}
    /// Look up a zone by name, failing on anything outside [`ZONES`].
    pub fn zone(name: &str) -> Result<&'static str, ZoneError> {
        ZONES
            .iter()
            .copied()
            .find(|known| *known == name)
            .ok_or_else(|| ZoneError::UnknownZone {
                zone: name.to_owned(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heal::HealKind;
    use crate::recovery::{HealScope, HealValidation};
    use crate::EventCode;

    #[test]
    fn holds_120_frames_oldest_first() {
        let mut sink = RerunSink::new();
        assert!(sink.is_empty());
        assert_eq!(sink.capacity(), MAX_RERUN_FRAMES);
        assert_eq!(sink.capacity(), 120);
        for tick in 0..130 {
            sink.push(tick, tick.wrapping_mul(31));
        }
        assert_eq!(sink.len(), MAX_RERUN_FRAMES);
        assert_eq!(sink.overflow_count(), 10);
        let ticks: Vec<u64> = sink.iter().map(|frame| frame.tick).collect();
        assert_eq!(ticks.len(), MAX_RERUN_FRAMES);
        assert_eq!(ticks[0], 10);
        assert_eq!(ticks[MAX_RERUN_FRAMES - 1], 129);
    }

    #[test]
    fn identical_sinks_have_no_divergence() {
        let mut left = RerunSink::new();
        let mut right = RerunSink::new();
        assert_eq!(left.first_divergent(&right), None);
        for tick in 0..MAX_RERUN_FRAMES as u64 {
            left.push(tick, tick.wrapping_mul(7));
            right.push(tick, tick.wrapping_mul(7));
        }
        assert_eq!(left.first_divergent(&right), None);
        assert_eq!(right.first_divergent(&left), None);
    }

    #[test]
    fn names_first_mismatched_hash() {
        let mut left = RerunSink::new();
        let mut right = RerunSink::new();
        for tick in 0..MAX_RERUN_FRAMES as u64 {
            left.push(tick, tick);
            right.push(tick, if tick == 41 { tick + 1 } else { tick });
        }
        assert_eq!(left.first_divergent(&right), Some(41));
    }

    #[test]
    fn extra_tail_counts_as_divergent() {
        let mut short = RerunSink::new();
        let mut long = RerunSink::new();
        for tick in 0..10 {
            short.push(tick, tick);
            long.push(tick, tick);
        }
        long.push(10, 10);
        assert_eq!(short.first_divergent(&long), Some(10));
        assert_eq!(long.first_divergent(&short), Some(10));
    }

    #[test]
    fn span_names_use_rerun_namespace() {
        assert!(tracy::RERUN_RECORD.starts_with("rerun."));
        assert!(tracy::RERUN_COMPARE.starts_with("rerun."));
        assert!(!tracy::RERUN_RECORD.is_empty());
    }

    #[test]
    fn zone_lookup_accepts_set_and_refuses_strangers() {
        for zone in tracy::ZONES {
            assert_eq!(tracy::zone(zone), Ok(*zone));
        }
        assert_eq!(tracy::zone(tracy::RECORD_TICK), Ok(tracy::RECORD_TICK));
        assert_eq!(tracy::zone(tracy::COMPARE_SCAN), Ok(tracy::COMPARE_SCAN));
        for stranger in ["gen_ai.request", "rerun.unknown", ""] {
            assert!(tracy::zone(stranger).is_err(), "{stranger} is refused");
        }
        let err = tracy::zone("gen_ai.request").unwrap_err();
        assert!(err.to_string().contains("gen_ai.request"));
        let _: &dyn std::error::Error = &err;
    }

    #[test]
    fn every_zone_mark_and_plot_name_lives_under_rerun() {
        for zone in tracy::ZONES {
            assert!(zone.starts_with("rerun."), "{zone} keeps the namespace");
        }
        assert!(tracy::RECORD_TICK.starts_with("rerun."));
        assert!(tracy::COMPARE_SCAN.starts_with("rerun."));
        assert!(tracy::FRAME.starts_with("rerun."));
        assert!(tracy::PLOT_TICK.starts_with("rerun."));
        assert!(tracy::PLOT_FRAME.starts_with("rerun."));
    }

    #[test]
    fn plot_labels_follow_tick_then_frame_order() {
        assert_eq!(tracy::FRAME, "rerun.frame");
        assert!(tracy::PLOT_TICK.ends_with(".tick"));
        assert!(tracy::PLOT_FRAME.ends_with(".frame"));
        let suffixes: Vec<&str> = [tracy::PLOT_TICK, tracy::PLOT_FRAME]
            .iter()
            .map(|name| name.rsplit('.').next().unwrap_or(""))
            .collect();
        assert_eq!(suffixes, vec!["tick", "frame"]);
    }

    fn heal_row(tick: u64, validation: HealValidation) -> HealTapeRecord {
        HealTapeRecord {
            code: EventCode::NET_CLIENT_DESYNC,
            fault_tick: tick,
            applied_tick: Some(tick),
            kind: HealKind::AiTakeover,
            attempt: 1,
            backoff_ms: 0,
            quarantined: false,
            scope: HealScope::State,
            before_digest: Some(0x11),
            after_digest: Some(if validation == HealValidation::Recovered {
                0x11
            } else {
                0x22
            }),
            validation,
        }
    }

    /// The heal rows ride inside the same bounded sink and the same overflow
    /// counter: nothing is dropped silently.
    #[test]
    fn rerun_sink_carries_the_heal_rows_inside_the_same_budget() {
        let mut sink = RerunSink::new();
        assert_eq!(sink.heal_len(), 0);
        for index in 0..(MAX_RERUN_HEAL_ROWS + 5) as u64 {
            sink.push_heal(&heal_row(index, HealValidation::Recovered));
        }
        assert_eq!(sink.heal_len(), MAX_RERUN_HEAL_ROWS);
        assert_eq!(
            sink.overflow_count(),
            5,
            "the dropped heal rows are counted, never silent"
        );
        let ticks: Vec<u64> = sink.heal_rows().map(|row| row.fault_tick).collect();
        assert_eq!(ticks.len(), MAX_RERUN_HEAL_ROWS);
        assert_eq!(ticks[0], 5, "the oldest rows evicted first");
    }

    /// A hunt names the first divergent tick and the heal that caused it.
    #[test]
    fn rerun_sink_names_the_first_divergent_tick_after_a_bad_heal() {
        let mut left = RerunSink::new();
        let mut right = RerunSink::new();
        for tick in 0..MAX_RERUN_FRAMES as u64 {
            left.push(tick, tick);
            right.push(tick, if tick == 41 { tick + 1 } else { tick });
        }
        right.push_heal(&heal_row(41, HealValidation::NotRecovered));
        right.push_heal(&heal_row(42, HealValidation::Recovered));
        assert_eq!(left.first_divergent(&right), Some(41));
        let bad = right
            .first_unproved_heal()
            .expect("the bad heal is on the sink");
        assert_eq!(bad.fault_tick, 41);
        assert_eq!(bad.validation, HealValidation::NotRecovered);
        assert_eq!(right.heal_len(), 2);
    }
}
