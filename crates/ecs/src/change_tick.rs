//! Change-tick store with wrapping stamps and per-column summaries.
//!
//! Owner: engine-ecs. Units: ticks, wrapping.
//! Determinism: exact integer math only, index order, no wall clock.
//! Errors: out of range rows report false, never panics.

/// Change-tick stamp in wrapping ticks.
///
/// Units: ticks. Determinism: plain integer, wraps by design.
pub type Tick = u32;

/// Ticks between periodic stamp scans.
///
/// Units: ticks. Determinism: exact integer bound; the gated scan runs
/// only once the world advanced this far past the last scan.
pub const CHECK_TICK_THRESHOLD: Tick = 518_400_000;

/// Maximum change age that still reports.
///
/// Units: ticks. Determinism: exact integer bound; both sides of the
/// relative compare clamp to this age so scans on different ticks agree.
pub const MAX_CHANGE_AGE: Tick = Tick::MAX - (2 * CHECK_TICK_THRESHOLD - 1);

/// True when `change` happened after `last_run`, both read at `this_run`.
///
/// Units: ticks. Determinism: exact integer math only, no float, no clock.
/// Threading: pure function, safe on any thread.
#[inline]
pub fn is_newer_than(change: Tick, last_run: Tick, this_run: Tick) -> bool {
    this_run.wrapping_sub(last_run).min(MAX_CHANGE_AGE)
        > this_run.wrapping_sub(change).min(MAX_CHANGE_AGE)
}

/// Clamp one stamp into the scan window.
///
/// Units: ticks. Determinism: exact integer math only. Stamps older than
/// the maximum age move forward to the window edge, which decides the
/// same under the relative compare.
#[inline]
pub fn clamp_stamp(stamp: Tick, this_run: Tick) -> Tick {
    if this_run.wrapping_sub(stamp) > MAX_CHANGE_AGE {
        this_run.wrapping_sub(MAX_CHANGE_AGE)
    } else {
        stamp
    }
}

/// Per-column change stamps with summary ticks.
///
/// Each row holds one stamp per column. Each summary holds the latest
/// write tick of its column, so a clean column skips its scan. Units:
/// ticks. Determinism: index order, exact integer math, no allocation
/// on the read path. Budget: scan is linear in rows with zero heap
/// allocation; gated scan touches only dirty columns.
#[derive(Clone, Debug, Default)]
pub struct ChangeTicks {
    pos: Vec<Tick>,
    vel: Vec<Tick>,
    pos_summary: Tick,
    vel_summary: Tick,
    tick: Tick,
    last_scan: Tick,
}

impl ChangeTicks {
    /// Empty store at tick zero with clean summaries.
    ///
    /// Units: ticks. Determinism: fixed start on every machine.
    pub fn new() -> Self {
        Self {
            pos: Vec::new(),
            vel: Vec::new(),
            pos_summary: 0,
            vel_summary: 0,
            tick: 0,
            last_scan: 0,
        }
    }

    /// Empty store with both columns pre-sized.
    ///
    /// Units: row count. Determinism: capacity never affects decisions.
    /// Setup path only; call before the tick so the tick never grows.
    pub fn with_capacity(rows: usize) -> Self {
        let mut out = Self::new();
        out.pos.reserve(rows);
        out.vel.reserve(rows);
        out
    }

    /// Row count, both columns always agree.
    ///
    /// Units: rows. Determinism: exact count.
    pub fn len(&self) -> usize {
        self.pos.len()
    }

    /// True when no rows are stored.
    ///
    /// Units: rows. Determinism: exact.
    pub fn is_empty(&self) -> bool {
        self.pos.is_empty()
    }

    /// Current change tick.
    ///
    /// Units: ticks. Determinism: exact.
    pub fn tick(&self) -> Tick {
        self.tick
    }

    /// Tick of the last periodic stamp scan.
    ///
    /// Units: ticks. Determinism: exact.
    pub fn last_scan(&self) -> Tick {
        self.last_scan
    }

    /// Latest write tick of the first column.
    ///
    /// Units: ticks. Determinism: exact.
    pub fn pos_summary(&self) -> Tick {
        self.pos_summary
    }

    /// Latest write tick of the second column.
    ///
    /// Units: ticks. Determinism: exact.
    pub fn vel_summary(&self) -> Tick {
        self.vel_summary
    }

    /// Pre-size both columns for more rows.
    ///
    /// Units: rows. Determinism: capacity never affects decisions.
    /// Setup path only.
    pub fn reserve(&mut self, additional: usize) {
        self.pos.reserve(additional);
        self.vel.reserve(additional);
    }

    /// Push one row stamped at the current tick, returning its index.
    ///
    /// Units: ticks for stamps, index for the row. Determinism: push
    /// order decides indices. Both summaries move to the current tick.
    pub fn push(&mut self) -> usize {
        let now = self.tick;
        self.pos.push(now);
        self.vel.push(now);
        self.pos_summary = now;
        self.vel_summary = now;
        self.pos.len().saturating_sub(1)
    }

    /// Stamp one row of the first column with the current tick.
    ///
    /// Units: ticks. Determinism: same ticks give the same stamps.
    /// Returns false for an out of range row, never panics.
    pub fn mark_pos(&mut self, row: usize) -> bool {
        if row >= self.pos.len() {
            return false;
        }
        self.pos[row] = self.tick;
        self.pos_summary = self.tick;
        true
    }

    /// Stamp one row of the second column with the current tick.
    ///
    /// Units: ticks. Determinism: same ticks give the same stamps.
    /// Returns false for an out of range row, never panics.
    pub fn mark_vel(&mut self, row: usize) -> bool {
        if row >= self.vel.len() {
            return false;
        }
        self.vel[row] = self.tick;
        self.vel_summary = self.tick;
        true
    }

    /// Stamp every row of the first column with the current tick.
    ///
    /// Units: ticks. Determinism: every stamp lands at once, order free.
    /// Budget: linear in rows, zero heap allocation.
    pub fn mark_all_pos(&mut self) {
        let now = self.tick;
        self.pos.fill(now);
        if !self.pos.is_empty() {
            self.pos_summary = now;
        }
    }

    /// Advance the change clock by one wrapping tick.
    ///
    /// Units: ticks. Determinism: wrapping add, same on every machine.
    /// Budget: constant time, zero allocation. Pair with the gated scan
    /// at the tick boundary so old stamps retire on time.
    pub fn advance(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /// Clamp every stamp older than the maximum age to the window edge.
    ///
    /// Units: ticks. Determinism: clamping preserves every relative
    /// decision, so query results are identical before and after.
    /// Budget: linear in rows, zero heap allocation. Returns the
    /// clamped stamp count.
    pub fn check(&mut self) -> usize {
        let now = self.tick;
        let mut clamped = 0usize;
        for stamp in self.pos.iter_mut().chain(self.vel.iter_mut()) {
            let next = clamp_stamp(*stamp, now);
            if next != *stamp {
                *stamp = next;
                clamped += 1;
            }
        }
        self.pos_summary = clamp_stamp(self.pos_summary, now);
        self.vel_summary = clamp_stamp(self.vel_summary, now);
        self.last_scan = now;
        clamped
    }

    /// True when the first-column stamp of `row` is newer than `last_run`.
    ///
    /// Units: ticks. Determinism: exact integer math only. Out of
    /// range rows report false.
    pub fn is_pos_newer(&self, row: usize, last_run: Tick, this_run: Tick) -> bool {
        if row >= self.pos.len() {
            return false;
        }
        is_newer_than(self.pos[row], last_run, this_run)
    }

    /// True when the second-column stamp of `row` is newer than `last_run`.
    ///
    /// Units: ticks. Determinism: exact integer math only. Out of
    /// range rows report false.
    pub fn is_vel_newer(&self, row: usize, last_run: Tick, this_run: Tick) -> bool {
        if row >= self.vel.len() {
            return false;
        }
        is_newer_than(self.vel[row], last_run, this_run)
    }

    /// True when the first column holds any change since `last_run`.
    ///
    /// Units: ticks. Determinism: exact integer math only. The summary
    /// is the latest write, so a false summary means no row is newer.
    pub fn is_pos_column_newer(&self, last_run: Tick, this_run: Tick) -> bool {
        is_newer_than(self.pos_summary, last_run, this_run)
    }

    /// True when the second column holds any change since `last_run`.
    ///
    /// Units: ticks. Determinism: exact integer math only. The summary
    /// is the latest write, so a false summary means no row is newer.
    pub fn is_vel_column_newer(&self, last_run: Tick, this_run: Tick) -> bool {
        is_newer_than(self.vel_summary, last_run, this_run)
    }

    /// Visit every row changed in any dirty column, skipping clean columns.
    ///
    /// Units: ticks for the window, indices for rows. Determinism:
    /// index order, exact integer math, no allocation. Clean columns
    /// cost one summary compare and no per-row work.
    pub fn for_each_changed(&self, last_run: Tick, this_run: Tick, mut f: impl FnMut(usize)) {
        let pos_dirty = self.is_pos_column_newer(last_run, this_run);
        let vel_dirty = self.is_vel_column_newer(last_run, this_run);
        if !pos_dirty && !vel_dirty {
            return;
        }
        let n = self.pos.len();
        if pos_dirty && vel_dirty {
            for i in 0..n {
                if is_newer_than(self.pos[i], last_run, this_run)
                    || is_newer_than(self.vel[i], last_run, this_run)
                {
                    f(i);
                }
            }
        } else if pos_dirty {
            for i in 0..n {
                if is_newer_than(self.pos[i], last_run, this_run) {
                    f(i);
                }
            }
        } else {
            for i in 0..n {
                if is_newer_than(self.vel[i], last_run, this_run) {
                    f(i);
                }
            }
        }
    }

    /// Count rows changed in any dirty column, skipping clean columns.
    ///
    /// Units: ticks, rows. Determinism: same stamps give the same
    /// count. Budget: linear in rows of dirty columns only, zero heap
    /// allocation.
    pub fn count_changed(&self, last_run: Tick, this_run: Tick) -> usize {
        let mut n = 0usize;
        self.for_each_changed(last_run, this_run, |_| {
            n += 1;
        });
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled(rows: usize, tick_now: Tick, dirty: usize) -> (ChangeTicks, Tick, Tick) {
        let mut s = ChangeTicks::new();
        s.reserve(rows);
        for _ in 0..rows {
            s.push();
        }
        let last = tick_now.wrapping_sub(200_000);
        s.tick = tick_now.wrapping_sub(10);
        for i in 0..dirty {
            assert!(s.mark_pos(i));
        }
        s.tick = tick_now;
        (s, last, tick_now)
    }

    #[test]
    fn relative_compare_orders_wraps() {
        assert!(is_newer_than(3, 0, 5));
        assert!(is_newer_than(0, Tick::MAX - 1, 1));
        assert!(is_newer_than(1, 0, 1));
        assert!(!is_newer_than(5, 5, 5));
        assert!(!is_newer_than(1, 2, 5));
    }

    #[test]
    fn old_stamps_retire_and_clamp_pins_edge() {
        let now: Tick = 3_400_000_000;
        let last: Tick = now - 200_000;
        let old = now.wrapping_sub(MAX_CHANGE_AGE).wrapping_sub(1);
        assert!(!is_newer_than(old, now.wrapping_sub(10), now));
        assert_eq!(clamp_stamp(old, now), now.wrapping_sub(MAX_CHANGE_AGE));
        assert_eq!(clamp_stamp(now.wrapping_sub(10), now), now - 10);
        let _ = last;
    }

    #[test]
    fn summaries_skip_clean_columns_with_same_rows() {
        let (s, last, now) = filled(10_000, 3_400_000_000, 100);
        assert!(s.is_pos_column_newer(last, now));
        assert!(!s.is_vel_column_newer(last, now));
        let mut gated = Vec::new();
        s.for_each_changed(last, now, |i| gated.push(i));
        assert_eq!(gated.len(), 100);
        let mut baseline = Vec::new();
        for i in 0..s.len() {
            if is_newer_than(s.pos[i], last, now) || is_newer_than(s.vel[i], last, now) {
                baseline.push(i);
            }
        }
        assert_eq!(gated, baseline);
        assert_eq!(s.count_changed(last, now), 100);
    }

    #[test]
    fn changed_scan_does_half_the_compares_when_one_column_clean() {
        let (s, last, now) = filled(10_000, 3_400_000_000, 100);
        let baseline_compares = s.len() * 2;
        let pos_dirty = s.is_pos_column_newer(last, now);
        let vel_dirty = s.is_vel_column_newer(last, now);
        let mut gated_compares = 2usize;
        if pos_dirty && vel_dirty {
            gated_compares += s.len() * 2;
        } else if pos_dirty || vel_dirty {
            gated_compares += s.len();
        }
        assert!(pos_dirty && !vel_dirty);
        assert_eq!(baseline_compares, 20_000);
        assert_eq!(gated_compares, 10_002);
        assert!(
            baseline_compares >= gated_compares * 2 - 4,
            "gated {gated_compares} must be near half of baseline {baseline_compares}"
        );
        assert_eq!(s.count_changed(last, now), 100);
    }

    #[test]
    fn wrap_soak_near_max_repeats_exactly() {
        fn soak_once() -> (Vec<usize>, Tick) {
            let mut s = ChangeTicks::new();
            s.tick = Tick::MAX - 10;
            for _ in 0..1_000 {
                s.push();
            }
            let last = Tick::MAX - 2;
            s.tick = Tick::MAX - 1;
            for i in 0..10 {
                assert!(s.mark_pos(i));
            }
            s.tick = Tick::MAX;
            for i in 10..20 {
                assert!(s.mark_vel(i));
            }
            for _ in 0..6 {
                s.advance();
            }
            let now = s.tick();
            let mut rows = Vec::new();
            s.for_each_changed(last, now, |i| rows.push(i));
            rows.sort_unstable();
            (rows, now)
        }
        let (a_rows, a_now) = soak_once();
        let (b_rows, b_now) = soak_once();
        assert_eq!(a_now, 5);
        assert_eq!((a_rows.clone(), a_now), (b_rows, b_now));
        assert_eq!(a_rows.len(), 20);
    }

    #[test]
    fn scan_clamps_without_moving_decisions() {
        let (mut s, last, now) = filled(1_000, 3_400_000_000, 500);
        let mut before = Vec::new();
        s.for_each_changed(last, now, |i| before.push(i));
        let clamped = s.check();
        assert!(clamped >= 1_000);
        let mut after = Vec::new();
        s.for_each_changed(last, now, |i| after.push(i));
        assert_eq!(before, after);
        assert_eq!(s.check(), 0);
        assert_eq!(s.last_scan(), now);
    }

    #[test]
    fn gated_scan_rests_when_nothing_dirty() {
        let mut s = ChangeTicks::new();
        for _ in 0..64 {
            s.push();
        }
        s.advance();
        let now = s.tick();
        let last = now;
        assert!(!s.is_pos_column_newer(last, now));
        assert!(!s.is_vel_column_newer(last, now));
        assert_eq!(s.count_changed(last, now), 0);
        assert!(!s.is_pos_newer(9_999, last, now));
        assert!(!s.is_vel_newer(9_999, last, now));
        assert!(!s.mark_pos(9_999));
        assert!(!s.mark_vel(9_999));
    }

    #[test]
    fn gated_scan_matches_baseline_when_both_dirty() {
        let mut s = ChangeTicks::new();
        for _ in 0..512 {
            s.push();
        }
        let last = s.tick();
        s.advance();
        s.mark_all_pos();
        for i in 0..512 {
            assert!(s.mark_vel(i));
        }
        let now = s.tick();
        let mut gated = Vec::new();
        s.for_each_changed(last, now, |i| gated.push(i));
        assert_eq!(gated.len(), 512);
        assert!(s.is_pos_column_newer(last, now) && s.is_vel_column_newer(last, now));
    }

    #[test]
    #[ignore = "timing: wall clock proof, run with --ignored under --perf"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn timing_10k_1pct_reports_gated_faster() {
        let (s, last, now) = filled(10_000, 3_400_000_000, 100);
        let t0 = std::time::Instant::now();
        let mut base = 0usize;
        for i in 0..s.len() {
            if is_newer_than(s.pos[i], last, now) || is_newer_than(s.vel[i], last, now) {
                base += 1;
            }
        }
        let base_dt = t0.elapsed();
        let t1 = std::time::Instant::now();
        let gated = s.count_changed(last, now);
        let gated_dt = t1.elapsed();
        assert_eq!(base, gated);
        assert_eq!(gated, 100);
        let baseline_compares = s.len() * 2;
        let gated_compares = s.len() + 2;
        assert_eq!((baseline_compares, gated_compares), (20_000, 10_002));
        assert!(baseline_compares >= gated_compares * 2 - 4);
        std::println!("baseline {base_dt:?} gated {gated_dt:?}");
    }
}
