//! Seeded full-jitter backoff vectors killing same-tick retry collisions.
//! Owner: engine-observe diagnostics. Units: ms (`u64`), attempt (`u32`).
//! Determinism: pure word-to-delay map, no clock; same word gives same delay.
//! Errors: none (total function; saturating and clamped math).
//! Donors (idea-only, fresh std-only reimplementation, licences verified):
//! nicholas-ruest/jitter-map (MIT) https://github.com/nicholas-ruest/jitter-map
//! Remade-With-Rust/rusty_rtos_backoff (Apache-2.0) https://github.com/Remade-With-Rust/rusty_rtos_backoff

/// Deterministic 64-bit mixer turning `(seed, client)` into a random word.
///
/// Plain splitmix64, std-only, no clock. Caller supplies the word to the
/// backoff below so replay with the same seed ticks identically.
fn splitmix64(mut state: u64) -> u64 {
    state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

/// Seeded full-jitter backoff: uniform delay in `[0, min(cap, base * 2^attempt)]`.
///
/// `word` is the caller-supplied random `u64` (for example one `splitmix64`
/// output per client per tick). The inclusive upper edge maps `u64::MAX` to
/// the window top and `0` to `0`; the window itself is clamped to `cap`, and
/// `cap == 0` always yields `0`. No clock, no allocation, no new dependency.
#[must_use]
fn full_jitter_backoff(base_ms: u64, cap_ms: u64, attempt: u32, word: u64) -> u64 {
    if cap_ms == 0 {
        return 0;
    }
    let shift = u64::from(attempt);
    let grown = if shift >= 64 {
        u64::MAX
    } else {
        base_ms.saturating_mul(1_u64 << shift)
    };
    let window = grown.min(cap_ms);
    // Inclusive range `[0, window]`: length is window + 1 as u128 so the
    // u64::MAX edge never overflows; multiply-high keeps it unbiased and
    // makes both edges reachable (0 -> 0, MAX -> window).
    let range_len = u128::from(window).wrapping_add(1);
    let scaled = (u128::from(word).wrapping_mul(range_len)) >> 64;
    u64::try_from(scaled).unwrap_or(window)
}

#[test]
fn same_seed_and_attempt_gives_same_delay_twice() {
    let (base, cap, attempt, word) = (100_u64, 5_000_u64, 3_u32, 0x1234_5678_9ABC_DEF0_u64);
    let first = full_jitter_backoff(base, cap, attempt, word);
    let second = full_jitter_backoff(base, cap, attempt, word);
    assert_eq!(first, second);
    // Same word on a different attempt opens a different window.
    let other = full_jitter_backoff(base, cap, attempt + 1, word);
    assert!(first <= 800_u64 && other <= 1_600_u64);
    assert_eq!(full_jitter_backoff(base, cap, attempt, word), first);
}

#[test]
fn six_attempt_max_sequence_is_pinned() {
    // Base 100 ms, cap 1_000 ms, max word: windows are
    // 100, 200, 400, 800, then clamped to 1_000, 1_000.
    let expected = [100_u64, 200, 400, 800, 1_000, 1_000];
    for (attempt, want) in expected.iter().enumerate() {
        let attempt_u32 = u32::try_from(attempt).unwrap_or(u32::MAX);
        let got = full_jitter_backoff(100, 1_000, attempt_u32, u64::MAX);
        assert_eq!(got, *want, "attempt {attempt}");
    }
}

#[test]
fn top_and_bottom_values_are_reachable() {
    assert_eq!(full_jitter_backoff(100, 1_000, 2, u64::MAX), 400_u64);
    assert_eq!(full_jitter_backoff(100, 1_000, 2, 0), 0_u64);
    assert_eq!(full_jitter_backoff(100, 1_000, 9, u64::MAX), 1_000_u64);
    // Full u64 window must not overflow the inclusive edge.
    assert_eq!(
        full_jitter_backoff(u64::MAX, u64::MAX, 0, u64::MAX),
        u64::MAX
    );
    assert_eq!(full_jitter_backoff(u64::MAX, u64::MAX, 0, 0), 0_u64);
}

#[test]
fn clamp_holds_for_huge_attempts_and_empty_cap() {
    assert_eq!(
        full_jitter_backoff(100, 1_000, u32::MAX, u64::MAX),
        1_000_u64
    );
    assert_eq!(
        full_jitter_backoff(100, 1_000, 100, 0xFFFF_FFFF_FFFF_FFFF),
        1_000_u64
    );
    assert_eq!(full_jitter_backoff(100, 0, 3, 0xABCD_EF01_2345_6789), 0_u64);
    assert_eq!(
        full_jitter_backoff(0, 1_000, 3, 0xABCD_EF01_2345_6789),
        0_u64
    );
    // Base above cap clamps down to cap.
    assert_eq!(full_jitter_backoff(9_999, 50, 0, u64::MAX), 50_u64);
    assert!(full_jitter_backoff(100, 500, 200, 0x1357_2468_ACE0_1234) <= 500_u64);
}

#[test]
fn same_tick_32_clients_collide_rarely() {
    const CLIENTS: usize = 32;
    const SEED: u64 = 2_040;
    let mut delays = [0_u64; CLIENTS];
    for (index, slot) in delays.iter_mut().enumerate() {
        let index_u64 = u64::try_from(index).unwrap_or(u64::MAX);
        let word = splitmix64(SEED.wrapping_add(index_u64.wrapping_mul(0x9E37_79B9_7F4A_7C15)));
        *slot = full_jitter_backoff(100, 10_000, 5, word);
    }
    let mut sorted = delays;
    sorted.sort_unstable();
    let mut distinct = 1_usize;
    for pair in sorted.windows(2) {
        if pair[0] != pair[1] {
            distinct += 1;
        }
    }
    let collisions = CLIENTS - distinct;
    let rate = f64::from(u32::try_from(collisions).unwrap_or(u32::MAX))
        / f64::from(u32::try_from(CLIENTS).unwrap_or(1));
    println!(
        "passed: 32-client same-tick collisions={collisions}/{CLIENTS} rate={rate:.3} distinct={distinct} delays={delays:?}"
    );
    // Window for attempt 5 is [0, 3_200] (3_201 values), so 32 seeded
    // clients must spread out; more than 6 collisions means jitter is broken.
    assert!(collisions <= 6, "collisions={collisions} delays={delays:?}");
    assert!(rate < 0.25, "rate={rate}");
}
