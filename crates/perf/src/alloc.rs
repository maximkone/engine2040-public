//! Allocation counting for the budget gate: the shared
//! `engine_core::alloc_probe` installed as this crate's global allocator.
//! Counting is per thread and armed only inside [`measure`], so parallel
//! tests never pollute each other's budgets and unmeasured code behaves
//! exactly like `System`.

use engine_core::alloc_probe::{self, CountingAlloc};

#[global_allocator]
static GLOBAL_ALLOC: CountingAlloc = CountingAlloc;

/// Allocation totals observed on the calling thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AllocSnapshot {
    /// Sum of requested `Layout::size` bytes.
    pub bytes: u64,
    /// Number of successful allocations.
    pub count: u64,
}

/// Run `f` with counting enabled on the calling thread and return its value
/// plus the allocations it made. Nested calls restart the inner count and
/// restore the outer window, so helper nesting stays balanced.
pub fn measure<T>(f: impl FnOnce() -> T) -> (T, AllocSnapshot) {
    let (value, seen) = alloc_probe::measure(f);
    let observed = AllocSnapshot {
        bytes: seen.bytes,
        count: seen.calls,
    };
    (value, observed)
}

/// Peak heap observed over one run, in bytes.
///
/// Owner: this module owns only the pure math over recorded samples; it
/// never reads a clock or the OS. Determinism: same inputs give the same
/// peak twice. Errors: none, construction cannot fail.
/// Caller: `crates/perf/src/main.rs --heap-peak` plus the `heap_peak`
/// integration test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeakSnapshot {
    /// Largest live-heap sample in bytes.
    pub peak_bytes: u64,
}

impl PeakSnapshot {
    /// Snapshot the largest sample via [`peak_bytes`]; empty reads 0.
    ///
    /// Units: bytes in, snapshot out. Determinism: pure `u64` max, same
    /// slice gives the same snapshot twice. Errors: none.
    /// Caller: `crates/perf/src/main.rs --heap-peak`.
    #[must_use]
    pub fn of(samples: &[u64]) -> Self {
        Self {
            peak_bytes: peak_bytes(samples),
        }
    }
}

/// Return the largest sample as `peak_bytes`, or 0 when empty.
///
/// Units: bytes in, bytes out. Determinism: pure `u64` max, same slice
/// gives the same answer twice. Errors: none.
/// Caller: [`PeakSnapshot::of`] plus the `--heap-peak` harness via
/// [`peak_of`]. GS14 free `peak_bytes` beside `peak_of`; the repeat
/// assert keeps the two maxima tied without duplicating the loop.
pub fn peak_bytes(samples: &[u64]) -> u64 {
    let peak = peak_of(samples);
    debug_assert_eq!(peak, peak_of(samples), "peak repeats deterministically");
    peak
}

/// Return the largest sample, or 0 when empty.
///
/// Units: bytes in, bytes out. Determinism: pure `u64` math, same slice
/// gives the same answer twice. Errors: none.
pub fn peak_of(samples: &[u64]) -> u64 {
    let mut peak: u64 = 0;
    for &sample in samples {
        if sample > peak {
            peak = sample;
        }
    }
    peak
}

/// Return true when `peak_bytes` fits within 10 percent above `baseline_bytes`.
///
/// Units: bytes in, boolean out. Determinism: pure integer math, same inputs
/// give the same answer twice. Errors: none; a zero baseline allows only a
/// zero peak.
pub fn allows(peak_bytes: u64, baseline_bytes: u64) -> bool {
    if baseline_bytes == 0 {
        return peak_bytes == 0;
    }
    u128::from(peak_bytes) * 100 <= u128::from(baseline_bytes) * 110
}

/// CSV header for the heap-peak report (`target/reports/heap-peak.csv`).
///
/// Owner: this module owns the pure format; the integration test owns the
/// file write. Units: header text, no numbers. Determinism: constant.
/// Errors: none.
pub const HEAP_PEAK_CSV_HEADER: &str = "workload,peak_bytes";

/// Format one heap-peak CSV row for `workload` with `peak_bytes`.
///
/// Units: bytes in, one `workload,bytes` line out. Determinism: same inputs
/// give the same line. Errors: none; the workload name is trusted ASCII.
pub fn heap_peak_csv_row(workload: &str, peak_bytes: u64) -> String {
    format!("{workload},{peak_bytes}")
}

/// Format the full heap-peak CSV (header plus one row plus trailing newline).
///
/// Units: bytes in, CSV text out. Determinism: pure string join, same inputs
/// give the same text. Errors: none.
pub fn heap_peak_csv(workload: &str, peak_bytes: u64) -> String {
    format!(
        "{HEAP_PEAK_CSV_HEADER}\n{}\n",
        heap_peak_csv_row(workload, peak_bytes)
    )
}

/// Parse the heap-peak baseline file (`crates/perf/baselines/heap_peak.txt`).
///
/// The file holds one base-10 `u64` on its first line (written by the first
/// quiet run). Leading/trailing ASCII whitespace is ignored; anything else
/// reads `None` so the gate fails closed instead of passing by default.
/// Units: text in, bytes out. Determinism: pure parse. Errors: `None` on
/// empty or non-numeric input, never a panic.
pub fn parse_heap_peak_baseline(text: &str) -> Option<u64> {
    text.lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .and_then(|line| line.parse::<u64>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_only_inside_the_window() {
        let ((), before) = measure(|| {});
        assert_eq!(before, AllocSnapshot { bytes: 0, count: 0 });
        let ((), one) = measure(|| {
            std::hint::black_box(vec![0u8; 64]);
        });
        assert_eq!(one.count, 1);
        assert_eq!(one.bytes, 64);
        // Allocating after the window closed is not charged to the next one.
        std::hint::black_box(vec![0u8; 256]);
        let ((), after) = measure(|| {});
        assert_eq!(after, AllocSnapshot { bytes: 0, count: 0 });
    }

    #[test]
    fn other_threads_do_not_pollute_this_thread() {
        // The worker's megabyte must land on the worker's counters only; this
        // thread is charged for spawn/join bookkeeping, far below it.
        const WORKER_BYTES: u64 = 1 << 20;
        let ((), here) = measure(|| {
            std::thread::spawn(|| {
                let ((), there) = measure(|| {
                    std::hint::black_box(vec![0u8; 1 << 20]);
                });
                assert_eq!(there.bytes, WORKER_BYTES);
                assert_eq!(there.count, 1);
            })
            .join()
            .expect("worker joins");
        });
        assert!(
            here.bytes < WORKER_BYTES,
            "worker allocation leaked into the spawning thread: {here:?}"
        );
    }

    #[test]
    fn heap_peak_csv_names_workload_and_peak() {
        let text = heap_peak_csv("arena_tick_8", 57_238);
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some(HEAP_PEAK_CSV_HEADER));
        assert_eq!(lines.next(), Some("arena_tick_8,57238"));
        assert_eq!(
            heap_peak_csv_row("arena_tick_8", 57_238),
            "arena_tick_8,57238"
        );
    }

    #[test]
    fn heap_peak_baseline_parses_and_rejects_garbage() {
        assert_eq!(parse_heap_peak_baseline("57238\n"), Some(57_238));
        assert_eq!(parse_heap_peak_baseline("  57238  \n"), Some(57_238));
        assert_eq!(parse_heap_peak_baseline(""), None);
        assert_eq!(parse_heap_peak_baseline("nope\n"), None);
    }

    #[test]
    fn heap_peak_64mib_over_baseline_trips_gate() {
        const BASELINE: u64 = 57_238;
        assert!(
            allows(BASELINE, BASELINE),
            "baseline peak passes its own gate"
        );
        assert!(
            !allows(BASELINE + (64 << 20), BASELINE),
            "baseline plus 64 MiB fails the 10 percent rule"
        );
    }

    #[test]
    fn peak_snapshot_of_matches_peak_of_x2() {
        // `PeakSnapshot::of` wraps `peak_of` so the `--heap-peak` caller
        // reads one typed snapshot; same samples give the same peak twice.
        let samples = [57_238, 57_238];
        let first = PeakSnapshot::of(&samples);
        let second = PeakSnapshot::of(&samples);
        assert_eq!(first, second, "same samples give the same snapshot twice");
        assert_eq!(first.peak_bytes, peak_of(&samples));
        assert_eq!(PeakSnapshot::of(&[]).peak_bytes, 0);
        assert_eq!(PeakSnapshot::of(&[1, 9, 4]).peak_bytes, 9);
    }

    #[test]
    fn peak_bytes_matches_peak_of_x2() {
        // GS14 free `peak_bytes` beside `peak_of`; same slice gives the
        // same bytes twice, and the typed snapshot reads the same number.
        assert_eq!(peak_bytes(&[]), 0, "empty reads 0");
        assert_eq!(peak_bytes(&[1, 9, 4]), 9, "max sample wins");
        assert_eq!(peak_bytes(&[57_238, 57_238]), 57_238);
        let samples = [4_u64, 9, 3, 17, 11];
        assert_eq!(
            peak_bytes(&samples),
            peak_of(&samples),
            "free peak_bytes matches peak_of"
        );
        assert_eq!(
            peak_bytes(&samples),
            peak_bytes(&samples),
            "same slice gives same peak_bytes twice"
        );
        assert_eq!(
            PeakSnapshot::of(&samples).peak_bytes,
            peak_bytes(&samples),
            "typed snapshot matches free peak_bytes"
        );
    }
}
