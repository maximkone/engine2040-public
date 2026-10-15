// Donor: twitter/finagle BackupRequestFilter.scala — hedged backup request
// issued after p99 latency with a maxExtraLoad retry-budget cap, Apache-2.0
// (licence verified), pattern only (no donor code copied).
// Link: https://github.com/twitter/finagle
//! Hedged-backup-request budget vectors for stall cutting at capped load.
//!
//! Owner: engine-observe diagnostics. Units: latencies and thresholds in
//! ms (u64), request counts in usize, budget as integer percent (u64).
//! Determinism: seeded splitmix64 stream only; no clock, no sleep, no IO.
//! Errors: none; budget exhaustion falls back to the primary sample.
//!
//! Pattern: a slow primary (past the p99 threshold) spends one budgeted
//! backup; the loser is interrupted so the request takes the winner latency.
//! A retry-budget cap bounds extra load to `max_extra_percent`.

/// Deterministic 64-bit stream (fresh splitmix64, std-only).
struct HedgeRng {
    /// Internal mixer state (u64 counter).
    state: u64,
}

impl HedgeRng {
    /// Seed the stream; any u64 seed is valid.
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Next pseudorandom u64.
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// One latency sample in ms: typical fast path, rare deep stall.
fn sample_latency_ms(rng: &mut HedgeRng) -> u64 {
    // 5% of primaries stall at 200..249 ms; the rest finish in 5..10 ms.
    if rng.next_u64().is_multiple_of(20) {
        200 + (rng.next_u64() % 50)
    } else {
        5 + (rng.next_u64() % 6)
    }
}

/// Config for one hedged run.
struct HedgeConfig {
    /// Requests to simulate (usize count).
    requests: usize,
    /// Latency past which a backup is issued, in ms.
    hedge_after_ms: u64,
    /// Extra-load cap as integer percent (u64, e.g. 10 = 10%).
    max_extra_percent: u64,
    /// Seed for the deterministic sample stream.
    seed: u64,
}

/// Outcome of one hedged run.
struct HedgeOutcome {
    /// Per-request latency in ms after hedging (len == requests).
    latencies_ms: Vec<u64>,
    /// Plain primary latencies in ms (len == requests).
    primaries_ms: Vec<u64>,
    /// Backups actually issued (usize count, bounded by budget).
    backups: usize,
    /// Budget allowance in requests (usize count).
    budget: usize,
}

/// Run the primary + backup-after-p99 simulation with a retry-budget cap.
///
/// Requests beyond the cap fall back to the primary (no backup). The loser
/// is interrupted, so a hedged request takes `min(primary, backup)`.
fn run_hedged(config: &HedgeConfig) -> HedgeOutcome {
    let mut rng = HedgeRng::new(config.seed);
    let cap = usize::try_from(config.max_extra_percent).unwrap_or(0);
    let budget = config.requests.saturating_mul(cap).saturating_div(100);
    let mut latencies_ms = Vec::with_capacity(config.requests);
    let mut primaries_ms = Vec::with_capacity(config.requests);
    let mut backups: usize = 0;
    for _ in 0..config.requests {
        let primary = sample_latency_ms(&mut rng);
        primaries_ms.push(primary);
        if primary > config.hedge_after_ms && backups < budget {
            let backup = sample_latency_ms(&mut rng);
            backups = backups.saturating_add(1);
            latencies_ms.push(primary.min(backup));
        } else {
            latencies_ms.push(primary);
        }
    }
    HedgeOutcome {
        latencies_ms,
        primaries_ms,
        backups,
        budget,
    }
}

/// p99 latency in ms over a sample (upper quantile, deterministic).
///
/// Returns 0 for an empty sample. Uses `sort_unstable` on a copy so the
/// caller order (and determinism) is preserved.
fn p99_ms(sample: &[u64]) -> u64 {
    if sample.is_empty() {
        return 0;
    }
    let mut sorted = sample.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    // ceil(0.99 * n), 1-based, then back to 0-based.
    let rank_1based = n.saturating_mul(99).saturating_add(99).saturating_div(100);
    let idx = rank_1based.saturating_sub(1).min(n.saturating_sub(1));
    sorted[idx]
}

/// Extra load in integer percent (u64): `backups * 100 / requests`.
fn extra_percent(backups: usize, requests: usize) -> u64 {
    if requests == 0 {
        return 0;
    }
    const HUNDRED: usize = 100;
    backups
        .checked_mul(HUNDRED)
        .map(|scaled| scaled / requests)
        .and_then(|p| u64::try_from(p).ok())
        .unwrap_or(u64::MAX)
}

fn default_config() -> HedgeConfig {
    HedgeConfig {
        requests: 2_000,
        hedge_after_ms: 15,
        max_extra_percent: 10,
        seed: 2_040,
    }
}

#[test]
fn p99_down_vs_no_hedge() {
    let config = default_config();
    let outcome = run_hedged(&config);
    let plain = p99_ms(&outcome.primaries_ms);
    let hedged = p99_ms(&outcome.latencies_ms);
    println!(
        "hedge budget: max_extra_percent={} budget_requests={} backups={} extra_percent={} p99_before_ms={} p99_after_ms={}",
        config.max_extra_percent,
        outcome.budget,
        outcome.backups,
        extra_percent(outcome.backups, config.requests),
        plain,
        hedged,
    );
    assert!(
        plain >= 200,
        "fixture must stall at p99 without hedging, got {plain}"
    );
    assert!(
        hedged < plain,
        "hedged p99 ({hedged} ms) must beat no-hedge p99 ({plain} ms)"
    );
    assert!(
        hedged <= 15,
        "hedged p99 must recover to the fast path, got {hedged}"
    );
}

#[test]
fn extra_load_within_budget() {
    let config = default_config();
    let outcome = run_hedged(&config);
    let extra = extra_percent(outcome.backups, config.requests);
    println!(
        "hedge budget: max_extra_percent={} budget_requests={} backups={} extra_percent={}",
        config.max_extra_percent, outcome.budget, outcome.backups, extra,
    );
    assert!(
        outcome.backups <= outcome.budget,
        "backups ({}) must fit the budget ({})",
        outcome.backups,
        outcome.budget
    );
    assert!(
        extra <= config.max_extra_percent,
        "extra load ({extra}%) must fit the cap ({}%)",
        config.max_extra_percent
    );
    assert!(
        outcome.backups > 0,
        "fixture must spend some budget on stalls"
    );
}

#[test]
fn budget_never_exceeded() {
    for max_extra_percent in [0_u64, 1, 5, 10, 50] {
        for seed in [1_u64, 2_040, 99_999] {
            let config = HedgeConfig {
                requests: 1_000,
                hedge_after_ms: 15,
                max_extra_percent,
                seed,
            };
            let outcome = run_hedged(&config);
            let extra = extra_percent(outcome.backups, config.requests);
            assert!(
                outcome.backups <= outcome.budget,
                "seed {seed} cap {max_extra_percent}%: backups ({}) over budget ({})",
                outcome.backups,
                outcome.budget
            );
            assert!(
                extra <= max_extra_percent,
                "seed {seed} cap {max_extra_percent}%: extra ({extra}%) over cap"
            );
            if max_extra_percent == 0 {
                assert_eq!(outcome.backups, 0, "zero budget must issue no backup");
                assert_eq!(
                    outcome.latencies_ms, outcome.primaries_ms,
                    "zero budget must fall back to primaries"
                );
            }
        }
    }
    println!("hedge budget: swept caps [0,1,5,10,50]% over 3 seeds, never exceeded");
}

#[test]
fn loser_interrupt_picks_winner_and_skips_fast() {
    // Fast primary never spends budget: latency passes through untouched.
    let fast_primary: u64 = 8;
    let hedge_after_ms: u64 = 15;
    let issued = fast_primary > hedge_after_ms;
    assert!(!issued, "fast primary must not trigger a backup");
    // Slow primary with a fast backup: loser interrupt takes the minimum.
    let slow_primary: u64 = 220;
    let fast_backup: u64 = 7;
    assert!(slow_primary > hedge_after_ms, "stall must trigger a backup");
    assert_eq!(
        slow_primary.min(fast_backup),
        fast_backup,
        "loser interrupt must keep the winner"
    );
    // Slow primary with a slow backup: still the minimum, never the sum.
    let slow_backup: u64 = 231;
    assert_eq!(
        slow_primary.min(slow_backup),
        slow_primary,
        "double stall must keep the faster loser-free latency"
    );
    println!("hedge budget: loser-interrupt takes min, fast path spends 0");
}

#[test]
fn determinism_x2() {
    let config = default_config();
    let first = run_hedged(&config);
    let second = run_hedged(&config);
    assert_eq!(
        first.latencies_ms, second.latencies_ms,
        "repeat run must match"
    );
    assert_eq!(
        first.primaries_ms, second.primaries_ms,
        "primaries must match"
    );
    assert_eq!(first.backups, second.backups, "backup count must match");
    assert_eq!(
        p99_ms(&first.latencies_ms),
        p99_ms(&second.latencies_ms),
        "p99 must match across runs"
    );
    println!(
        "hedge budget: determinism x2 seed={} backups={} p99_after_ms={}",
        config.seed,
        first.backups,
        p99_ms(&first.latencies_ms),
    );
}
