//! test_env: the one place this crate's tests write process environment
//! variables. `set_var` and `remove_var` are unsafe from Rust 2024 on because
//! a thread reading the environment outside std (libc `getenv`) could race
//! the write. These tests read their variables only through std, which
//! synchronizes its own environment access, every key belongs to one test,
//! and all writes go through one lock.
#![cfg(test)]

use std::sync::{Mutex, PoisonError};

/// Serializes this crate's test-time environment writes.
static ENV_WRITES: Mutex<()> = Mutex::new(());

/// Set `key` to `value` for the calling test.
#[expect(
    unsafe_code,
    reason = "test-only environment write, serialized by ENV_WRITES; std is the only reader"
)]
pub(crate) fn set(key: &str, value: &str) {
    let _guard = ENV_WRITES.lock().unwrap_or_else(PoisonError::into_inner);
    // SAFETY: std synchronizes its own environment reads and writes, nothing
    // in this test binary reads the environment through libc, and `key`
    // belongs to the calling test alone.
    unsafe { std::env::set_var(key, value) };
}

/// Remove `key` for the calling test.
#[expect(
    unsafe_code,
    reason = "test-only environment write, serialized by ENV_WRITES; std is the only reader"
)]
pub(crate) fn remove(key: &str) {
    let _guard = ENV_WRITES.lock().unwrap_or_else(PoisonError::into_inner);
    // SAFETY: as for `set`: std-only readers and a key owned by one test.
    unsafe { std::env::remove_var(key) };
}

/// The unsafe writes above round-trip through std's reader. This is the
/// test Miri runs for them (CQ15): the socket-backed tests that use these
/// helpers cannot run under Miri.
#[test]
fn set_then_remove_round_trips_through_std() {
    const KEY: &str = "E2040_TEST_ENV_ROUND_TRIP";
    set(KEY, "one");
    assert_eq!(std::env::var(KEY).as_deref(), Ok("one"));
    set(KEY, "two");
    assert_eq!(std::env::var(KEY).as_deref(), Ok("two"));
    remove(KEY);
    assert_eq!(std::env::var(KEY), Err(std::env::VarError::NotPresent));
}
