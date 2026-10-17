//! core: fixed tick, seeded rng, per-tick hash (FNV-1a 64).
//! Owner: engine-core (std-only; re-exports tick/hash/rng/config owners).
//! Units: ticks (u64), seconds and Hz (f32/u64), hashes and seeds (u64).
//! Determinism: fixed dt, seeded streams, little-endian hashes; no wall clock.
//! Errors: none at crate root; fallible callers use typed errors (e.g. TimeError).
//! Budget: no per-tick allocation; zero-alloc paths proven by alloc_probe tests.
pub mod adler;
#[cfg(any(test, feature = "test-util"))]
pub mod alloc_probe;
pub mod arena;
pub mod checked;
pub mod config;
pub mod counter_rng;
/// Bitwise IEEE CRC-32 shared by the PNG writers.
pub mod crc32;
pub mod det_math;
pub mod digest;
pub mod diverge;
pub mod fixed;
pub mod hash;
/// JSON string escaping for the hand-rolled writers.
pub mod json;
pub mod quat;
pub mod rng;
pub mod seal_key;
pub mod tick;
pub mod time;
pub mod vec3;
// One stream type crate-wide; a second same-named type let callers diverge silently.
pub use counter_rng::{CounterCursor, CounterRng};
// One hash owner: every crate imports these instead of re-deriving FNV or SplitMix.
pub use hash::{hash_bytes, hash_hex, hash_u64s, mix64, splitmix64, Fnv64};
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
pub use tick::{
    fold_hash, run_ticks, Tick, TickState, DT, LEGACY_DT, LEGACY_MAX_FRAME_DT, MAX_FRAME_DT,
    MAX_STEPS, TICK_HZ,
};
pub use time::{
    quantize_wall_secs, IntAcc, TimeError, MAX_FRAME_NS, NS_PER_SEC, STEP_NS, STEP_REM_THIRDS,
    STEP_THIRDS,
};
/// Fixed simulation step in seconds (60 Hz): the one owner for every crate's
/// `FIXED_DT` copy. Identical bits to [`tick::DT`]; `tick.rs` stays frozen.
pub const FIXED_DT: f32 = tick::DT;
/// Tick rate as `u32` (60 Hz): the narrow form every crate's ticks-per-second
/// constant derives from, so no call site repeats `as u32` on the rate.
pub const TICK_HZ_U32: u32 = checked::u32_from_u64(TICK_HZ);
/// Stillness window in ticks (3 s at 60 Hz): the one owner for every crate's
/// `STUCK_WINDOW_TICKS` copy. Value never changes.
pub const STUCK_WINDOW_TICKS: u32 = 3 * TICK_HZ_U32;
