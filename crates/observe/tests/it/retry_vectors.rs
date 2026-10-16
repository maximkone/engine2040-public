//! Retry-vs-fatal predicate vectors for tick-boundary heals.
//!
//! Owner: engine-observe diagnostics. Units: tries (u64),
//! delays and totals in ms (u64). Determinism: no clock, no sleep;
//! total_delay_ms is the saturating sum of consumed delay entries.
//! Errors: ProbeError carries the retryable/fatal predicate.
//!
//! PORT(jimmycuadra/retry:src/lib.rs) MIT: idea of retry_with_index,
//! current_try (1-based) and total_delay accumulation reimplemented
//! fresh here without any clock in the tick path.

/// Probe error with an explicit retry-vs-fatal class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeError {
    /// Transient fault: a further try is allowed while budget remains.
    Retryable,
    /// Permanent fault: stop at once, no further try, no further delay.
    Fatal,
}

impl ProbeError {
    /// Retry predicate: only Retryable permits another try.
    #[must_use]
    fn is_retryable(self) -> bool {
        matches!(self, Self::Retryable)
    }
}

/// Outcome of one bounded retry run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RetryOutcome<T> {
    /// Final operation value (Ok on success, last error otherwise).
    result: Result<T, ProbeError>,
    /// Attempts actually executed (u64 try count).
    tries: u64,
    /// Saturating sum of consumed inter-try delays in ms.
    total_delay_ms: u64,
}

/// Bounded retry runner: calls op(current_try) with 1-based current_try,
/// consuming delays_ms[current_try - 1] between attempts.
///
/// Stops on success, on a fatal error, when max_tries attempts are spent,
/// or when max_tries is zero (no attempt runs). Never sleeps.
fn retry_with_index<T>(
    max_tries: u64,
    delays_ms: &[u64],
    mut op: impl FnMut(u64) -> Result<T, ProbeError>,
) -> RetryOutcome<T> {
    let mut total_delay_ms: u64 = 0;
    let mut tries: u64 = 0;
    let mut attempt: u64 = 0;
    while attempt < max_tries {
        attempt = attempt.saturating_add(1);
        let current_try = attempt;
        match op(current_try) {
            Ok(value) => {
                tries = current_try;
                return RetryOutcome {
                    result: Ok(value),
                    tries,
                    total_delay_ms,
                };
            }
            Err(kind) => {
                tries = current_try;
                if !kind.is_retryable() {
                    return RetryOutcome {
                        result: Err(kind),
                        tries,
                        total_delay_ms,
                    };
                }
                if current_try >= max_tries {
                    return RetryOutcome {
                        result: Err(kind),
                        tries,
                        total_delay_ms,
                    };
                }
                // current_try is 1-based, so the gap after try N is index N-1.
                let gap: usize = (current_try - 1).try_into().unwrap_or(usize::MAX);
                let delay = delays_ms.get(gap).copied().unwrap_or(0);
                total_delay_ms = total_delay_ms.saturating_add(delay);
            }
        }
    }
    RetryOutcome {
        result: Err(ProbeError::Retryable),
        tries,
        total_delay_ms,
    }
}
#[test]
fn fail_twice_then_ok_uses_three_tries() {
    let delays_ms = [10_u64, 20, 30, 40];
    let mut calls: u64 = 0;
    let outcome = retry_with_index(5, &delays_ms, |current_try| {
        calls = calls.saturating_add(1);
        assert_eq!(calls, current_try);
        if current_try < 3 {
            Err(ProbeError::Retryable)
        } else {
            Ok(7_u32)
        }
    });
    assert_eq!(outcome.result, Ok(7_u32));
    assert_eq!(outcome.tries, 3_u64);
    assert_eq!(calls, 3_u64);
    // Two gaps consumed: 10 + 20.
    assert_eq!(outcome.total_delay_ms, 30_u64);
}

#[test]
fn fatal_on_first_makes_zero_retries_past_fatal() {
    let delays_ms = [10_u64, 20, 30];
    let mut calls: u64 = 0;
    let outcome = retry_with_index(5, &delays_ms, |current_try| -> Result<u32, ProbeError> {
        calls = calls.saturating_add(1);
        assert_eq!(current_try, 1_u64);
        Err(ProbeError::Fatal)
    });
    assert_eq!(outcome.result, Err(ProbeError::Fatal));
    assert_eq!(outcome.tries, 1_u64);
    assert_eq!(outcome.total_delay_ms, 0_u64);
    // The fatal predicate stops the run: exactly one attempt, zero retries.
    assert_eq!(calls, 1_u64);
}

#[test]
fn fatal_on_second_stops_after_one_delay() {
    let delays_ms = [15_u64, 25, 35];
    let mut calls: u64 = 0;
    let outcome = retry_with_index(5, &delays_ms, |current_try| -> Result<u32, ProbeError> {
        calls = calls.saturating_add(1);
        match current_try {
            1 => Err(ProbeError::Retryable),
            2 => Err(ProbeError::Fatal),
            _ => panic!("must not run past a fatal error"),
        }
    });
    assert_eq!(outcome.result, Err(ProbeError::Fatal));
    assert_eq!(outcome.tries, 2_u64);
    assert_eq!(calls, 2_u64);
    // Only the gap after try 1 was consumed.
    assert_eq!(outcome.total_delay_ms, 15_u64);
}

#[test]
fn retryable_exhaustion_keeps_last_error_and_full_delay() {
    let delays_ms = [10_u64, 20, 30];
    let mut calls: u64 = 0;
    let outcome = retry_with_index(3, &delays_ms, |current_try| -> Result<u32, ProbeError> {
        calls = calls.saturating_add(1);
        assert!((1..=3).contains(&current_try));
        Err(ProbeError::Retryable)
    });
    assert_eq!(outcome.result, Err(ProbeError::Retryable));
    assert_eq!(outcome.tries, 3_u64);
    assert_eq!(calls, 3_u64);
    // Two gaps for three tries: 10 + 20; the trailing entry is never used.
    assert_eq!(outcome.total_delay_ms, 30_u64);
}
#[test]
fn single_try_never_consumes_delay() {
    let delays_ms = [99_u64];
    let outcome = retry_with_index(1, &delays_ms, |current_try| -> Result<u32, ProbeError> {
        assert_eq!(current_try, 1_u64);
        Err(ProbeError::Retryable)
    });
    assert_eq!(outcome.result, Err(ProbeError::Retryable));
    assert_eq!(outcome.tries, 1_u64);
    assert_eq!(outcome.total_delay_ms, 0_u64);
}

#[test]
fn zero_budget_runs_no_attempts() {
    let delays_ms = [10_u64, 20];
    let mut calls: u64 = 0;
    let outcome = retry_with_index(0, &delays_ms, |_| -> Result<u32, ProbeError> {
        calls = calls.saturating_add(1);
        Ok(1_u32)
    });
    assert_eq!(calls, 0_u64);
    assert_eq!(outcome.tries, 0_u64);
    assert_eq!(outcome.total_delay_ms, 0_u64);
    assert_eq!(outcome.result, Err(ProbeError::Retryable));
}

#[test]
fn current_try_is_one_based_and_monotonic() {
    let delays_ms = [1_u64, 2, 3, 4, 5];
    let mut seen: [u64; 4] = [0; 4];
    let mut count: usize = 0;
    let outcome = retry_with_index(4, &delays_ms, |current_try| -> Result<u32, ProbeError> {
        seen[count] = current_try;
        count += 1;
        Err(ProbeError::Retryable)
    });
    assert_eq!(seen, [1_u64, 2, 3, 4]);
    assert_eq!(count, 4);
    assert_eq!(outcome.tries, 4_u64);
    assert_eq!(outcome.total_delay_ms, 1_u64 + 2 + 3);
}

#[test]
fn total_delay_saturates_instead_of_wrapping() {
    let delays_ms = [u64::MAX, u64::MAX, u64::MAX];
    let outcome = retry_with_index(4, &delays_ms, |_| -> Result<u32, ProbeError> {
        Err(ProbeError::Retryable)
    });
    assert_eq!(outcome.tries, 4_u64);
    assert_eq!(outcome.total_delay_ms, u64::MAX);
}
