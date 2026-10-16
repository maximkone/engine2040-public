//! rng: xorshift64 stream + splitmix fork substreams, LE digest.
//! Seeded contract: authoritative draws come only from `Rng`/`CounterRng`
//! substreams (`fork`/`fork_named`); unseeded entropy never reaches hashed
//! state. Hashed state folds through `hash_bytes`/`hash_u64s` (`Fnv64`)
//! in fixed order, never a default-seeded hasher or map iteration.
//! Legacy `new`/`next_u64` stays frozen for golden replays.
use crate::hash::{hash_bytes, hash_u64s, splitmix64, GOLDEN_GAMMA};

/// Legacy xorshift64 stream: 8 bytes, `Copy`, identical on every platform.
///
/// Frozen: the legacy golden replays this exact sequence. New systems that
/// draw in parallel or need seeds `2k` and `2k + 1` to differ use
/// [`crate::CounterRng`]; independent subsystems split with [`Rng::fork`] or
/// [`Rng::fork_named`] instead of sharing one stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rng(u64);
impl Rng {
    /// Frozen legacy seeding: forcing the low bit keeps the state nonzero, so
    /// seeds `2k` and `2k + 1` yield the same stream.
    ///
    /// TRAP — DO NOT "FIX": removing the `| 1` changes every legacy replay.
    /// `GOLDEN_600` depends on this aliasing. New code that needs adjacent
    /// seeds to differ uses [`crate::CounterRng`]; new fixtures expand seeds
    /// without aliasing.
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }
    /// Same low-bit aliasing as `new`.
    pub fn from_state(s: u64) -> Self {
        Self(s | 1)
    }
    /// Raw state, for snapshots; restore with [`Rng::from_state`].
    pub fn state(&self) -> u64 {
        self.0
    }
    /// Next word (xorshift 13/7/17); never zero.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    /// High 32 bits of the next word.
    pub fn next_u32(&mut self) -> u32 {
        u32::try_from(self.next_u64() >> 32).expect("high 32 bits fit in u32")
    }
    /// A value in `lo..hi` by modulo (slightly biased for ranges that do
    /// not divide 2^64); returns `lo` when `hi <= lo`.
    pub fn next_range(&mut self, lo: u64, hi: u64) -> u64 {
        if hi <= lo {
            return lo;
        }
        lo + (self.next_u64() % (hi - lo))
    }
    /// Child stream `stream`, mixed through SplitMix64; the parent does not
    /// advance, so forks taken in any order are identical.
    pub fn fork(&self, stream: u64) -> Self {
        let m = self.0.wrapping_add(stream.wrapping_mul(GOLDEN_GAMMA));
        Self(splitmix64(m) | 1)
    }
    /// Named substream: FNV-1a 64 the name, then fork.
    /// `fork_named("ai")` / `fork_named("world")` split deterministically.
    pub fn fork_named(&self, name: &str) -> Self {
        self.fork(hash_bytes(name.as_bytes()))
    }
    /// Child stream seeded from the next word; advances the parent once.
    pub fn fork_next(&mut self) -> Self {
        let s = self.next_u64();
        Self(splitmix64(s) | 1)
    }
    /// FNV-1a 64 over each word's little-endian bytes, in order.
    pub fn digest(vals: &[u64]) -> u64 {
        hash_u64s(vals)
    }
    /// Alias of [`Rng::digest`] for state-hash call sites.
    pub fn hash_state(vals: &[u64]) -> u64 {
        Self::digest(vals)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stream_same_seed() {
        let (mut a, mut b) = (Rng::new(2040), Rng::new(2040));
        let va: [u64; 8] = core::array::from_fn(|_| a.next_u64());
        let vb: [u64; 8] = core::array::from_fn(|_| b.next_u64());
        assert_eq!(va, vb);
        assert!(va.iter().any(|&x| x != 0));
    }
    #[test]
    fn first_value_golden() {
        assert_eq!(Rng::new(2040).next_u64(), 2189151099318);
        assert_eq!(Rng::new(0).next_u64(), 1082269761);
        assert_ne!(Rng::new(0).next_u64(), Rng::new(2).next_u64());
    }
    #[test]
    fn low_bit_alias_is_frozen() {
        use crate::CounterRng;
        // TRAP: `seed | 1` aliases 2k/2k+1 by design; GOLDEN_600 replays it.
        // Removing the `| 1` must break this test, never slip through.
        for k in [0u64, 1, 7, 1020, u64::MAX / 2] {
            let s0 = k.wrapping_mul(2);
            let s1 = s0.wrapping_add(1);
            assert_eq!(
                Rng::new(s0).state(),
                Rng::new(s1).state(),
                "alias moved for seeds {s0}/{s1}"
            );
            assert_eq!(
                Rng::from_state(s0).state(),
                Rng::from_state(s1).state(),
                "from_state alias moved for {s0}/{s1}"
            );
            let (mut a, mut b) = (Rng::new(s0), Rng::new(s1));
            let va: [u64; 4] = core::array::from_fn(|_| a.next_u64());
            let vb: [u64; 4] = core::array::from_fn(|_| b.next_u64());
            assert_eq!(va, vb, "aliased pair draws differ for {s0}/{s1}");
        }
        assert_ne!(
            Rng::new(0).state(),
            Rng::new(2).state(),
            "even seeds must stay distinct"
        );
        assert_ne!(
            CounterRng::new(0).key(),
            CounterRng::new(1).key(),
            "CounterRng must not alias adjacent seeds; use it for new code"
        );
    }
    #[test]
    fn fork_deterministic() {
        let p = Rng::new(7);
        assert_eq!(p.fork(1), p.fork(1));
        assert_ne!(p.fork(0), p.fork(1));
        assert_eq!(p.state(), Rng::new(7).state());
    }
    #[test]
    fn fork_named_golden() {
        let p = Rng::new(2040);
        assert_eq!(p.fork_named("ai"), p.fork_named("ai"));
        assert_ne!(p.fork_named("ai"), p.fork_named("world"));
        assert_eq!(p.state(), Rng::new(2040).state());
        let (mut a, mut b) = (p.fork_named("ai"), p.fork_named("ai"));
        assert_eq!(a.next_u64(), b.next_u64());
        assert_ne!(
            p.fork_named("ai").next_u64(),
            p.fork_named("world").next_u64()
        );
    }
    #[test]
    fn fork_golden() {
        assert_eq!(Rng::new(2040).fork(0).state(), 4794064739117914115);
        assert_eq!(Rng::new(2040).fork(1).state(), 16082742265117291817);
        let (mut a, mut b) = (Rng::new(7).fork(1), Rng::new(7).fork(2));
        assert_ne!(a.next_u64(), b.next_u64());
    }
    #[test]
    fn range_bounds() {
        let mut r = Rng::new(99);
        for _ in 0..16 {
            let v = r.next_range(5, 10);
            assert!((5..10).contains(&v));
        }
        assert_eq!(r.next_range(7, 7), 7);
        assert_eq!(r.next_range(10, 5), 10);
    }
    #[test]
    fn digest_golden() {
        let v = [4794064739117914115u64, 16082742265117291817u64];
        assert_eq!(Rng::digest(&v), 1231717912943920180);
        assert_eq!(Rng::hash_state(&v), Rng::digest(&v));
        assert_eq!(Rng::digest(&v), Rng::digest(&v));
        assert_ne!(Rng::digest(&v), Rng::digest(&v[1..]));
    }
    #[test]
    fn seeded_substreams_rerun_byte_identical() {
        // Authoritative draws rerun from the seed: the same seed gives zero
        // differing bytes across independent substreams.
        for seed in [0u64, 7, 2040] {
            let root = Rng::new(seed);
            let (mut a, mut b) = (root.fork_named("ai"), root.fork_named("ai"));
            let va: [u64; 8] = core::array::from_fn(|_| a.next_u64());
            let vb: [u64; 8] = core::array::from_fn(|_| b.next_u64());
            assert_eq!(va, vb);
            let mut ab = [0u8; 64];
            let mut bb = [0u8; 64];
            for (i, w) in va.iter().enumerate() {
                let bytes = w.to_le_bytes();
                let at = i * 8;
                ab[at..at + 8].copy_from_slice(&bytes);
            }
            for (i, w) in vb.iter().enumerate() {
                let bytes = w.to_le_bytes();
                let at = i * 8;
                bb[at..at + 8].copy_from_slice(&bytes);
            }
            let differing = ab.iter().zip(bb.iter()).filter(|(x, y)| x != y).count();
            assert_eq!(differing, 0, "seed {seed} rerun differed");
            assert_ne!(
                root.fork_named("ai").next_u64(),
                root.fork_named("world").next_u64()
            );
        }
    }
    #[test]
    fn hashed_state_uses_seeded_fnv_in_fixed_order() {
        // Seeded hasher contract: named substreams fold through hash_bytes,
        // and digests stay order-sensitive and stable across reruns.
        let root = Rng::new(2040);
        assert_eq!(root.fork_named("ai"), root.fork(hash_bytes(b"ai")));
        let v = [4794064739117914115u64, 16082742265117291817u64];
        assert_eq!(Rng::digest(&v), hash_u64s(&v));
        assert_eq!(Rng::digest(&v), Rng::digest(&v));
        assert_ne!(Rng::digest(&v), Rng::digest(&[v[1], v[0]]));
    }
}
