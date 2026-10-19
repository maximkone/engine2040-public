//! Purpose: per-tick timeline rows for seeded desync hunts over the frozen
//! PF9 recording shape ([`RerunFrame`](super::rerun_sink::RerunFrame)).
//! Owner: [`Timeline`] owns a bounded observed series and a bounded baseline
//! copy; the caller owns the recorded hashes. Both rings are capped at
//! [`MAX_TIMELINE_TICKS`], so a long capture belongs to `mcap` instead.
//! Units: ticks (`u64`), state hashes (`u64`, the shared FNV-1a owner), and
//! row counts (`usize`). Rerun's `time_point` shape is reimplemented as
//! NDJSON text: `{"time_point":{"timeline":"tick","tick":N},...}`.
//! Determinism: no clock, no random, no float. Seeded ticks come from
//! [`engine_core::hash::mix64`], so the same seed always picks the same row
//! and field; iteration is oldest-first.
//! Errors: none. [`Timeline::first_divergence`] returns `None` when the
//! shared prefix matches, and [`seeded_desync`] returns `None` for an empty
//! baseline, where no tick exists to perturb.
//! Budget: a full baseline plus its observed copy holds 1,200 rows of 24
//! bytes (28.8 KB) on the heap; a push at the cap evicts the oldest row and
//! counts it, so steady state allocates nothing.
//!
//! Only ticks that both series hold are compared, position by position. A
//! trailing extra row is the PF9 sink's case
//! ([`RerunSink::first_divergent`](super::rerun_sink::RerunSink::first_divergent)),
//! not a field divergence here.

use std::collections::VecDeque;
use std::fmt;

use engine_core::hash::{hex16, mix64};

/// Rows retained per series: one golden 600-tick run.
pub const MAX_TIMELINE_TICKS: usize = 600;

/// One field of a timeline row that a desync can name. The first two are the
/// PF9 recording shape; `EngineHash` is the whole-engine hash for the tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineField {
    /// Simulation tick.
    Tick,
    /// Whole-engine state hash for the tick.
    EngineHash,
    /// Per-subsystem state hash kept by the PF9 recording shape.
    StateHash,
}

impl TimelineField {
    /// Every field in comparison order, earliest first.
    pub const ALL: [Self; 3] = [Self::Tick, Self::EngineHash, Self::StateHash];

    /// Stable field name used in reports and NDJSON.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Tick => "tick",
            Self::EngineHash => "engine_hash",
            Self::StateHash => "state_hash",
        }
    }

    /// Value this field holds in `row`.
    #[must_use]
    pub fn value(self, row: &TimelineRow) -> u64 {
        match self {
            Self::Tick => row.tick,
            Self::EngineHash => row.engine_hash,
            Self::StateHash => row.state_hash,
        }
    }

    /// Copy of `row` with this field moved by one (wrapping, so it always
    /// changes).
    #[must_use]
    fn bumped(self, row: TimelineRow) -> TimelineRow {
        match self {
            Self::Tick => TimelineRow {
                tick: row.tick.wrapping_add(1),
                ..row
            },
            Self::EngineHash => TimelineRow {
                engine_hash: row.engine_hash.wrapping_add(1),
                ..row
            },
            Self::StateHash => TimelineRow {
                state_hash: row.state_hash.wrapping_add(1),
                ..row
            },
        }
    }
}

/// One recorded tick: the PF9 shape (tick, state hash) plus the engine hash
/// the run carried at that tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimelineRow {
    /// Simulation tick.
    pub tick: u64,
    /// Whole-engine state hash for the tick.
    pub engine_hash: u64,
    /// Per-subsystem state hash, the field `rerun_sink` already records.
    pub state_hash: u64,
}

impl TimelineRow {
    /// Build one row.
    #[must_use]
    pub fn new(tick: u64, engine_hash: u64, state_hash: u64) -> Self {
        Self {
            tick,
            engine_hash,
            state_hash,
        }
    }

    /// One NDJSON object in rerun `time_point` shape, hashes as 16-char hex.
    /// The line carries no clock: time is the tick.
    #[must_use]
    pub fn to_ndjson(&self) -> String {
        format!(
            r#"{{"time_point":{{"timeline":"tick","tick":{tick}}},"engine_hash":"{engine}","state_hash":"{state}"}}"#,
            tick = self.tick,
            engine = hex16(self.engine_hash),
            state = hex16(self.state_hash),
        )
    }
}

/// First disagreement with the baseline series: the row's baseline tick, the
/// differing field name, and both values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimelineDivergence {
    /// Baseline tick of the first disagreeing row (positional pairing).
    pub tick: u64,
    /// Field in [`TimelineField::ALL`] order that differs first.
    pub field: TimelineField,
    /// Baseline value at that tick.
    pub baseline: u64,
    /// Observed value at that tick.
    pub observed: u64,
}

impl TimelineDivergence {
    /// Stable name of the differing field.
    #[must_use]
    pub fn field_name(&self) -> &'static str {
        self.field.name()
    }
}

impl fmt::Display for TimelineDivergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "tick {} field {} baseline {:016x} observed {:016x}",
            self.tick,
            self.field.name(),
            self.baseline,
            self.observed
        )
    }
}

/// Bounded oldest-first series of [`TimelineRow`], optionally bound to a
/// baseline series for [`Timeline::first_divergence`].
#[derive(Debug, Clone)]
pub struct Timeline {
    baseline: VecDeque<TimelineRow>,
    rows: VecDeque<TimelineRow>,
    overflow: u64,
}

impl Timeline {
    /// Empty series with no baseline.
    #[must_use]
    pub fn new() -> Self {
        Self {
            baseline: VecDeque::new(),
            rows: VecDeque::new(),
            overflow: 0,
        }
    }

    /// Empty observed series bound to a copy of `baseline`'s rows.
    #[must_use]
    pub fn against(baseline: &Self) -> Self {
        Self {
            baseline: baseline.rows.clone(),
            rows: VecDeque::new(),
            overflow: 0,
        }
    }

    /// Rows retained per series. Always [`MAX_TIMELINE_TICKS`].
    #[must_use]
    pub fn capacity(&self) -> usize {
        MAX_TIMELINE_TICKS
    }

    /// Observed rows currently retained.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when no observed row is retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Baseline rows this series compares against.
    #[must_use]
    pub fn baseline_len(&self) -> usize {
        self.baseline.len()
    }

    /// Rows evicted because the observed ring was full. Monotonic.
    #[must_use]
    pub fn overflow_count(&self) -> u64 {
        self.overflow
    }

    /// Record one tick. At the cap the oldest row is evicted and counted.
    pub fn push(&mut self, row: TimelineRow) {
        if self.rows.len() >= MAX_TIMELINE_TICKS && self.rows.pop_front().is_some() {
            self.overflow = self.overflow.saturating_add(1);
        }
        self.rows.push_back(row);
    }

    /// Oldest-first iterator over observed rows.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &TimelineRow> + '_ {
        self.rows.iter()
    }

    /// First tick where the observed row differs from the baseline row at
    /// the same position, checking fields in [`TimelineField::ALL`] order.
    /// Returns `None` when the shared prefix matches, including an empty
    /// series or a series with no baseline.
    #[must_use]
    pub fn first_divergence(&self) -> Option<TimelineDivergence> {
        for (base, seen) in self.baseline.iter().zip(self.rows.iter()) {
            for field in TimelineField::ALL {
                let baseline = field.value(base);
                let observed = field.value(seen);
                if baseline != observed {
                    return Some(TimelineDivergence {
                        tick: base.tick,
                        field,
                        baseline,
                        observed,
                    });
                }
            }
        }
        None
    }

    /// Export observed rows as NDJSON: one `time_point`-shaped object per
    /// line, oldest first. The empty series gives an empty string.
    #[must_use]
    pub fn to_ndjson(&self) -> String {
        let mut out = String::new();
        for row in &self.rows {
            out.push_str(&row.to_ndjson());
            out.push('\n');
        }
        out
    }
}

impl Default for Timeline {
    /// Empty series; same as [`Timeline::new`].
    fn default() -> Self {
        Self::new()
    }
}

/// Row a seed picks in a series of `len` rows, or `None` when `len` is zero.
/// Pure: `mix64(seed)` modulo the row count.
#[must_use]
pub fn seeded_index(seed: u64, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let len64 = u64::try_from(len).unwrap_or(u64::MAX);
    let slot = mix64(seed) % len64;
    usize::try_from(slot).ok()
}

/// Field a seed perturbs: `mix64(seed) % 3` into [`TimelineField::ALL`].
#[must_use]
pub fn seeded_field(seed: u64) -> TimelineField {
    match mix64(seed) % 3 {
        0 => TimelineField::Tick,
        1 => TimelineField::EngineHash,
        _ => TimelineField::StateHash,
    }
}

/// Seeded-desync harness: copy `baseline`, then bump one seeded row's seeded
/// field by one. Returns `None` for an empty baseline, where no tick can be
/// perturbed.
#[must_use]
pub fn seeded_desync(baseline: &Timeline, seed: u64) -> Option<Timeline> {
    let index = seeded_index(seed, baseline.len())?;
    let field = seeded_field(seed);
    let mut observed = Timeline::against(baseline);
    for (position, row) in baseline.iter().enumerate() {
        let next = if position == index {
            field.bumped(*row)
        } else {
            *row
        };
        observed.push(next);
    }
    Some(observed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASELINE_TICKS: u64 = 120;

    fn baseline() -> Timeline {
        let mut series = Timeline::new();
        for tick in 0..BASELINE_TICKS {
            series.push(TimelineRow::new(
                tick,
                0x51ed_0000_0000_0000 ^ tick,
                0xae20_0000_0000_0000 ^ tick.wrapping_mul(31),
            ));
        }
        series
    }

    fn observed_from(base: &Timeline, rows: &[TimelineRow]) -> Timeline {
        let mut observed = Timeline::against(base);
        for row in rows {
            observed.push(*row);
        }
        observed
    }

    fn rows(series: &Timeline) -> Vec<TimelineRow> {
        series.iter().copied().collect()
    }

    #[test]
    fn empty_series_has_no_divergence() {
        let empty = Timeline::new();
        assert!(empty.first_divergence().is_none());
        let mut observed = Timeline::against(&empty);
        assert!(observed.first_divergence().is_none());
        observed.push(TimelineRow::new(1, 2, 3));
        assert!(observed.first_divergence().is_none());
        let bound = Timeline::against(&baseline());
        assert!(bound.first_divergence().is_none());
    }

    #[test]
    fn identical_series_have_no_divergence() {
        let base = baseline();
        let observed = observed_from(&base, &rows(&base));
        assert_eq!(observed.baseline_len(), base.len());
        assert!(observed.first_divergence().is_none());
    }

    #[test]
    fn divergence_on_first_tick_is_named() {
        let base = baseline();
        let mut seen = rows(&base);
        seen[0].state_hash = seen[0].state_hash.wrapping_add(9);
        let divergence = observed_from(&base, &seen).first_divergence().unwrap();
        assert_eq!(divergence.tick, 0);
        assert_eq!(divergence.field, TimelineField::StateHash);
        assert_eq!(divergence.field_name(), "state_hash");
        assert_eq!(divergence.baseline, rows(&base)[0].state_hash);
        assert_eq!(divergence.observed, seen[0].state_hash);
    }

    #[test]
    fn divergence_on_last_tick_is_named() {
        let base = baseline();
        let mut seen = rows(&base);
        let last = seen.len() - 1;
        seen[last].engine_hash = seen[last].engine_hash.wrapping_add(1);
        let divergence = observed_from(&base, &seen).first_divergence().unwrap();
        assert_eq!(divergence.tick, BASELINE_TICKS - 1);
        assert_eq!(divergence.field, TimelineField::EngineHash);
    }

    #[test]
    fn earliest_of_two_divergences_wins() {
        let base = baseline();
        let mut seen = rows(&base);
        seen[70].state_hash = seen[70].state_hash.wrapping_add(4);
        seen[21].tick = seen[21].tick.wrapping_add(2);
        let divergence = observed_from(&base, &seen).first_divergence().unwrap();
        assert_eq!(divergence.tick, 21);
        assert_eq!(divergence.field, TimelineField::Tick);
    }

    #[test]
    fn a_non_hash_field_is_named_as_tick() {
        let base = baseline();
        let mut seen = rows(&base);
        seen[33].tick = seen[33].tick.wrapping_add(1);
        let divergence = observed_from(&base, &seen).first_divergence().unwrap();
        assert_eq!(divergence.tick, 33);
        assert_eq!(divergence.field_name(), "tick");
        assert_eq!(divergence.baseline, 33);
        assert_eq!(divergence.observed, 34);
        assert!(divergence.to_string().contains("field tick"));
    }

    #[test]
    fn seeded_desync_names_the_perturbed_tick_ten_of_ten() {
        let base = baseline();
        let source = rows(&base);
        let seeds: Vec<u64> = (1..=10).collect();
        let mut named = Vec::new();
        for seed in &seeds {
            let seen = seeded_desync(&base, *seed).expect("non-empty baseline");
            let divergence = seen.first_divergence().expect("seed perturbs one row");
            // Find the perturbed row by diffing, independent of the seed map.
            let changed: Vec<usize> = rows(&seen)
                .iter()
                .zip(source.iter())
                .enumerate()
                .filter(|(_, (left, right))| left != right)
                .map(|(index, _)| index)
                .collect();
            assert_eq!(changed.len(), 1, "seed {seed} perturbs exactly one row");
            let index = changed[0];
            assert_eq!(divergence.tick, source[index].tick, "seed {seed} tick");
            assert_eq!(divergence.field, seeded_field(*seed), "seed {seed} field");
            assert_eq!(divergence.baseline, divergence.field.value(&source[index]));
            assert_eq!(
                divergence.observed,
                divergence.field.value(&rows(&seen)[index])
            );
            assert_ne!(divergence.baseline, divergence.observed);
            named.push(divergence.tick);
        }
        assert_eq!(named.len(), 10);
        named.sort_unstable();
        named.dedup();
        assert_eq!(named.len(), 10, "ten seeds name ten rows");
    }

    #[test]
    fn seeded_pick_is_pure_and_empty_baseline_has_no_desync() {
        assert_eq!(seeded_index(7, 120), seeded_index(7, 120));
        assert_eq!(seeded_index(7, 0), None);
        assert!(seeded_desync(&Timeline::new(), 3).is_none());
        assert_eq!(seeded_field(5), TimelineField::Tick);
        assert_eq!(seeded_field(3), TimelineField::StateHash);
    }

    #[test]
    fn ndjson_rows_keep_time_point_shape_and_hashes() {
        let mut series = Timeline::new();
        series.push(TimelineRow::new(12, 0x0f0f_0f0f_0f0f_0f0f, 0));
        let text = series.to_ndjson();
        assert!(text.starts_with(r#"{"time_point":{"timeline":"tick","tick":12}"#));
        assert!(text.contains(r#""engine_hash":"0f0f0f0f0f0f0f0f""#));
        assert!(text.contains(r#""state_hash":"0000000000000000""#));
        assert_eq!(text.lines().count(), 1);
        assert!(Timeline::new().to_ndjson().is_empty());
    }
}
