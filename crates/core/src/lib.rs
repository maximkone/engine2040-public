//! core: fixed tick, seeded rng, per-tick hash (FNV-1a 64).
//! Owner: engine-core (std-only; re-exports tick/hash/rng/config owners).
//! Units: ticks (u64), seconds and Hz (f32/u64), hashes and seeds (u64).
//! Determinism: fixed dt, seeded streams, little-endian hashes; no wall clock.
//! Errors: none at crate root; fallible callers use typed errors (e.g. TimeError).
//! Budget: no per-tick allocation; zero-alloc paths proven by alloc_probe tests.
#[cfg(any(test, feature = "test-util"))]
pub mod alloc_probe;
pub mod adler;
pub mod arena;
pub mod checked;
pub mod config;
pub mod counter_rng;
pub mod det_math;
pub mod digest;
pub mod fixed;
pub mod hash;
pub mod quat;
pub mod rng;
pub mod seal_key;
pub mod tick;
pub mod time;
pub mod vec3;
/// Off by default; Fixed stays the default sim math.
pub const DET_MATH_ENABLED: bool = false;
// One stream type crate-wide; a second same-named type let callers diverge silently.
pub use counter_rng::{CounterCursor, CounterRng};
// One hash owner: every crate imports these instead of re-deriving FNV or SplitMix.
pub use hash::{hash_bytes, hash_u64s, mix64, splitmix64, Fnv64};
// One quaternion product: every crate composes rotations to the same bits.
pub use quat::{quat_mul, quat_mul_fixed};
pub use rng::Rng;
pub use seal_key::{
    decode_seal_hex, parse_hex4, parse_seal_key, seal_key_from_env, SealKeyError,
    MIN_SEAL_KEY_BYTES,
};
// This crate's unit tests prove zero-allocation paths with the shared probe.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOC: alloc_probe::CountingAlloc = alloc_probe::CountingAlloc;
#[expect(deprecated, reason = "legacy alias stays exported for frozen receipts")]
pub use tick::{
    fold_hash, run_ticks, verify_same_seed, DefaultAcc, FixedAcc, LegacyFixedAcc, Tick, TickState,
    VerifyError, DT, LEGACY_DT, LEGACY_MAX_FRAME_DT, MAX_FRAME_DT, MAX_STEPS, TICK_HZ,
};
pub use time::{
    push_and_step, quantize_wall_secs, IntAcc, TimeError, MAX_FRAME_NS, NS_PER_SEC, STEP_NS,
    STEP_REM_THIRDS, STEP_THIRDS,
};
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rng_same_seed_same_stream() {
        let (mut a, mut b) = (Rng::new(2040), Rng::new(2040));
        let va: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        let vb: Vec<u64> = (0..8).map(|_| b.next_u64()).collect();
        assert_eq!(va, vb);
        assert!(va.iter().any(|&x| x != 0));
    }
    #[test]
    fn rng_zero_seed_live_and_distinct() {
        let (mut a, mut b) = (Rng::new(0), Rng::new(2));
        assert_ne!(a.next_u64(), b.next_u64());
    }
    #[test]
    fn fnv_empty_is_offset_basis() {
        assert_eq!(hash_bytes(&[]), 0xcbf29ce484222325);
        assert_ne!(hash_bytes(b"a"), hash_bytes(b"b"));
    }
    #[test]
    fn run_ticks_deterministic_and_sized() {
        let h1 = run_ticks(8, 2040, |t, _| t.0);
        let h2 = run_ticks(8, 2040, |t, _| t.0);
        assert_eq!(h1.len(), 8);
        assert_eq!(h1, h2);
        assert_ne!(h1.first(), h1.last());
    }
}
