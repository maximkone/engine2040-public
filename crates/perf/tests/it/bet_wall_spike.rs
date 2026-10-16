//! BET spike: seeded budget-regression wall-clock test (the timing
//! quarantine pattern).
//! Consumer: the perf quality gate (`docs/plan/quality.md`) — proves a wall-clock
//! overrun fails loudly when run, while the default suite stays green.
//!
//! Quarantine: `#[ignore = "timing: ..."]` (the timing rule). Default
//! `cargo test -p engine-perf` skips this test (PASSES quarantined); the
//! overrun verdict appears only under
//! `cargo test --release -p engine-perf --test it bet_wall_spike:: -- --ignored --test-threads=1`.
//!
//! Shape: the test asserts the seeded run is WITHIN budget, so a wall-clock
//! overrun is a test FAILURE (red), not a green proof. The seeded regression
//! below is a fixed 25 ms sleep against a 5 ms bar, so the trip never depends
//! on machine speed — even the fastest machine observes 25 ms > 5 ms.
//! Timing never enters a hash; golden `d57b5c8ae3096614` stands.
//! Production budgets untouched: `BET_SEEDED_SLEEP_BUDGET_MS` is local to
//! this spike, not posted in `baselines::TIME_BUDGETS_MS`.
//!
//! Kill rule: DELETE this file, do not promote, if EITHER verdict refuses:
//!   KILL-1: default suite must report `ignored` (quarantined), never
//!     `FAILED` and never `ok`-by-running — a wall-clock assert running in
//!     the default suite is the failure mode the quarantine prevents.
//!   KILL-2: `--ignored` run must report `FAILED` with `time budget` naming
//!     `bet_seeded_sleep` — a pass under `--ignored` despite the seeded
//!     25 ms overrun means the gate cannot catch a regression and the spike
//!     is worthless.

use engine_perf::budget::check_time_ms;
use std::time::Instant;

/// Seeded fixture: workload name plus its wall-clock budget in ms.
const WORKLOAD: &str = "bet_seeded_sleep";
/// Local kill bar for the fixture (spike-only, never posted as a gate).
const BET_SEEDED_SLEEP_BUDGET_MS: f64 = 5.0;
/// Seeded regression: fixed sleep (ms) that must exceed the bar on any
/// machine, with margin for timer slop (25 ms >> 5 ms).
const REGRESSION_SLEEP_MS: u64 = 25;

#[test]
#[ignore = "timing: release-mode BET wall-clock spike, run with --release -- --ignored --test-threads=1"]
fn bet_seeded_overrun_fails_the_time_gate() {
    let start = Instant::now();
    std::thread::sleep(std::time::Duration::from_millis(REGRESSION_SLEEP_MS));
    let actual_ms = start.elapsed().as_secs_f64() * 1000.0;
    assert!(
        actual_ms > BET_SEEDED_SLEEP_BUDGET_MS,
        "{WORKLOAD} regression really exceeds the bar: ran {actual_ms:.3} ms vs budget {BET_SEEDED_SLEEP_BUDGET_MS:.1} ms"
    );
    // Budget assert: the seeded overrun FAILS here (red), proving the gate
    // catches wall-clock regressions when run.
    check_time_ms(WORKLOAD, BET_SEEDED_SLEEP_BUDGET_MS, actual_ms).expect("within budget");
}
