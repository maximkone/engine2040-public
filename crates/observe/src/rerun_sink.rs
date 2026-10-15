//! Rerun recording of simulation state per tick for desync hunts.
//!
//! Consumer: the headless paired-rerun harness reads two sinks and names
//! the first divergent tick; the supervisor keeps its own tape. Data owner:
//! [`RerunSink`] owns a 120-frame ring of (tick, hash) rows; the caller owns
//! the hashed bytes. Units: ticks (`u64`), state hashes (`u64` FNV-1a over
//! little-endian words supplied by the caller). Determinism: insertion order
//! kept, oldest-first iteration, saturating counters, no clock. Errors: none;
//! push never fails and over-capacity rows evict the oldest while counting
//! overflow. Budget: 120 rows (1,920 bytes) inline; zero heap allocation per
//! push. Tracy span names are `&str` constants beside the supervisor; there
//! is no Tracy SDK dependency (licenses reviewed, nothing vendored).
//!
//! Pixels, GPU passes and VRAM accounting stay out: deeper capture (2,000
//! frames, per-pass timers, manifest legs) belongs to the pilot harness.

/// Maximum frames retained. The ring holds this many rows; deeper capture
/// stays with the pilot harness and never grows this type.
pub const MAX_RERUN_FRAMES: usize = 120;

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
        debug_assert!(tracy::zone(tracy::RECORD_TICK).is_ok());
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

    /// First tick where the two recordings disagree, oldest-first. Compares
    /// position by position: a tick or hash mismatch returns the tick from
    /// `self` at that position; when one recording is a strict prefix of the
    /// other, returns the first extra tick. Returns `None` when both hold the
    /// same rows. Determinism: pure comparison over stored rows, no clock.
    #[must_use]
    pub fn first_divergent(&self, other: &Self) -> Option<u64> {
        debug_assert!(tracy::zone(tracy::COMPARE_SCAN).is_ok());
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
}
