//! seed: fixture-v2 SplitMix64 expansion. Same seed gives the same stream on
//! every machine; neighbouring seeds give different streams (no aliasing).

/// One SplitMix64 step over the held state.
pub fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(engine_core::hash::GOLDEN_GAMMA);
    engine_core::mix64(*state)
}

/// Expand `seed` into `count` stream words for tapes and placements.
pub fn expand(seed: u64, count: usize) -> Vec<u64> {
    let mut state = seed;
    (0..count).map(|_| next(&mut state)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_twice_gives_same_stream() {
        assert_eq!(expand(2040, 64), expand(2040, 64));
    }

    #[test]
    fn neighbouring_seeds_differ() {
        // Fixture-v2 rule: seeds 2k and 2k+1 (and 0 vs 1) never alias.
        assert_ne!(expand(2_000, 8), expand(2_001, 8));
        assert_ne!(expand(0, 8), expand(1, 8));
        assert_ne!(expand(2040, 8), expand(999_983, 8));
    }

    #[test]
    fn stream_prefix_is_stable() {
        let full = expand(7, 16);
        assert_eq!(&full[..4], &*expand(7, 4));
    }
}
