// Donor: afex/hystrix-go, MIT, https://github.com/afex/hystrix-go (hystrix/hystrix.go).
//! Rolling error-percent circuit breaker vectors stopping MCP retry storms.
//!
//! Owner: engine-observe diagnostics. Units: calls (`u64`), error percent
//! (`u32`). Determinism: fixed 10-call ring, no clock, same vector twice
//! gives the same `opened_at`. Errors: [`BreakerError`] typed rejections.
//!
//! The breaker under test is `engine_observe::breaker::RollingBreaker`, the
//! only copy in the crate (`crates/observe/src/breaker.rs`).

use engine_observe::breaker::{BreakerError, RollingBreaker};

/// Feed one outcome vector through a fresh breaker.
fn run_vector(outcomes: &[bool]) -> RollingBreaker {
    let mut breaker = RollingBreaker::new();
    for ok in outcomes {
        breaker.admit().expect("vector admits while closed");
        breaker.observe(*ok);
    }
    breaker
}

#[test]
fn opens_at_threshold_open_at_10() {
    // 5 errors over 10 calls hits exactly 50%: the storm guard must open.
    let outcomes = [
        false, false, false, false, false, true, true, true, true, true,
    ];
    let breaker = run_vector(&outcomes);
    assert!(breaker.is_open(), "50% errors over 10 must open");
    assert_eq!(breaker.opened_at(), Some(10));
    assert_eq!(breaker.attempt(), 10);
    assert_eq!(breaker.admit(), Err(BreakerError::Open));
    assert_eq!(BreakerError::Open.to_string(), "circuit is open");
    println!("open-at 10 passed");
}

#[test]
fn half_open_single_probe_readmits() {
    let outcomes = [
        false, false, false, false, false, true, true, true, true, true,
    ];
    let mut breaker = run_vector(&outcomes);
    assert!(breaker.is_open());
    // Open rejects the retry storm without touching the window.
    assert_eq!(breaker.admit(), Err(BreakerError::Open));
    assert_eq!(breaker.attempt(), 10);
    // Single probe admitted; success closes and clears the window.
    breaker.half_open_probe().expect("one probe admitted");
    assert!(breaker.is_half_open());
    assert_eq!(breaker.admit(), Ok(()));
    breaker.observe(true);
    assert!(!breaker.is_open());
    assert!(!breaker.is_half_open());
    assert_eq!(breaker.admit(), Ok(()));
    // Fresh window: one error alone must not reopen.
    breaker.observe(false);
    assert!(!breaker.is_open());
    println!("probe re-admit passed");
}

#[test]
fn half_open_probe_failure_reopens_at_once() {
    let outcomes = [
        false, false, false, false, false, true, true, true, true, true,
    ];
    let mut breaker = run_vector(&outcomes);
    breaker.half_open_probe().expect("one probe admitted");
    assert!(breaker.is_half_open());
    breaker.observe(false);
    assert!(breaker.is_open());
    assert!(!breaker.is_half_open());
    assert_eq!(breaker.admit(), Err(BreakerError::Open));
    println!("probe failure reopen passed");
}

#[test]
fn stays_closed_under_low_errors() {
    // 4 errors over 10 calls is 40%: below the guard, calls keep flowing.
    let outcomes = [
        false, false, false, false, true, true, true, true, true, true,
    ];
    let breaker = run_vector(&outcomes);
    assert!(!breaker.is_open());
    assert_eq!(breaker.opened_at(), None);
    assert_eq!(breaker.admit(), Ok(()));
    assert_eq!(breaker.error_percent(), 40);
    println!("closed under low errors passed");
}

#[test]
fn same_vector_twice_gives_same_open_at() {
    let outcomes = [
        false, true, false, true, false, true, false, true, false, true,
    ];
    let first = run_vector(&outcomes);
    let second = run_vector(&outcomes);
    assert_eq!(first.opened_at(), second.opened_at());
    assert_eq!(first.attempt(), second.attempt());
    assert_eq!(first.is_open(), second.is_open());
    assert_eq!(first.opened_at(), Some(10));
    println!("determinism x2 open-at 10 passed");
}
