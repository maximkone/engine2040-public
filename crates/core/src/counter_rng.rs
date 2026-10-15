//! counter_rng: keyed counter streams for forks, islands and chunks.
//! Owner: engine-core. Units: u64 keys, counters, outputs. Determinism: pure
//! integer F(key, counter); same seed and counter give the same word on every
//! platform; the v1 Rng stream stays frozen for the legacy golden. Errors:
//! none by construction; degenerate ranges return `lo` like v1. Budget:
//! 16 bytes per stream, two u64 multiplies per word, zero heap use.

// Pattern after Salmon et al., SC 2011 (Random123): a keyed counter function
// built by alternating round-key injection with a bijective nonlinear round.
// Everything below is written from scratch: the round shape (add key, xorshift,
// multiply by an odd constant, xorshift), the two-round schedule and all
// constants are our own; only the pattern is borrowed. Each step (xor, add,
// multiply-by-odd, xorshift) is invertible, so for a fixed key the map
// counter -> word is a permutation: 2^64 distinct words, no shared state,
// and any partition of a counter range draws identical words.

use crate::hash::{hash_bytes, mix64, GOLDEN_GAMMA as INC};

// Odd multipliers for the two multiply-permute rounds (fixed, nonzero, odd).
const M0: u64 = 0xBF58476D1CE4E5B9;
const M1: u64 = 0x94D049BB133111EB;
// Seed-expansion tags: distinct fixed values so the two key halves differ.
const KX0: u64 = 0xD1B54A32D192ED03;
const KX1: u64 = 0x8F1BBCDC42C5B6B5;
// Domain tags keep island / chunk / bare-fork ids apart for small ids.
const ISLAND_TAG: u64 = u64::from_le_bytes(*b"ISLAND\x00\x01");
const CHUNK_TAG: u64 = u64::from_le_bytes(*b"CHUNK\x00\x00\x01");

/// Keyed counter stream: `sample(counter)` is F(key, counter).
/// Copyable 16-byte value; drawing never mutates shared state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CounterRng {
    k0: u64,
    k1: u64,
}

impl CounterRng {
    /// New stream from a seed. Every seed bit reaches the key through the
    /// avalanche mix, so seeds `2k` and `2k + 1` draw different words: the
    /// v1 low-bit aliasing is gone while v1 itself stays frozen.
    pub fn new(seed: u64) -> Self {
        Self {
            k0: mix64(seed ^ KX0),
            k1: mix64(seed ^ KX1),
        }
    }

    /// Raw key words, for snapshot/restore and tests.
    pub fn key(&self) -> [u64; 2] {
        [self.k0, self.k1]
    }

    /// Pure draw: word `counter` of this stream. Two chunks reading disjoint
    /// counter ranges share nothing yet stay reproducible under any
    /// re-chunking; wraps with the counter at u64::MAX.
    pub fn sample(&self, counter: u64) -> u64 {
        let mut x = counter ^ self.k0;
        x = x.wrapping_add(self.k1);
        x ^= x >> 29;
        x = x.wrapping_mul(M0);
        x ^= x >> 32;
        x = x.wrapping_add(self.k0 ^ INC);
        x ^= x >> 29;
        x = x.wrapping_mul(M1);
        x ^= x >> 32;
        x ^ self.k1
    }

    /// Fill `out` with words `[base, base + len)`. Caller-owned slice, no
    /// allocation: filling whole or in 1/2/4/8 pieces gives identical words.
    pub fn fill(&self, out: &mut [u64], base: u64) {
        for (i, slot) in out.iter_mut().enumerate() {
            *slot =
                self.sample(base.wrapping_add(u64::try_from(i).expect("fill index fits in u64")));
        }
    }

    /// Child stream `stream`. The parent is unchanged (`&self`) and parent
    /// and child share no state afterwards.
    pub fn fork(&self, stream: u64) -> Self {
        let t = (stream.wrapping_mul(INC).wrapping_add(self.k0)) ^ self.k1;
        Self::new(t)
    }

    /// Named child stream: FNV-1a64 the name, then fork.
    pub fn fork_named(&self, name: &str) -> Self {
        self.fork(hash_bytes(name.as_bytes()))
    }

    /// Island stream: fork under the island domain tag, so island `i` never
    /// equals chunk `i` or a bare `fork(i)` for ordinary ids.
    pub fn for_island(&self, island: u64) -> Self {
        self.fork(island ^ ISLAND_TAG)
    }

    /// Chunk stream: fork under the chunk domain tag.
    pub fn for_chunk(&self, chunk: u64) -> Self {
        self.fork(chunk ^ CHUNK_TAG)
    }
}

/// Sequential cursor over one stream range. The cursor is the only stateful
/// consumer: it owns its key plus position (24 bytes). Parallel chunks use
/// `sample`/`fill` at absolute counters and never share a cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CounterCursor {
    rng: CounterRng,
    next: u64,
}

impl CounterCursor {
    /// Cursor over `CounterRng::new(seed)` starting at counter 0.
    pub fn new(seed: u64) -> Self {
        Self::from_rng(CounterRng::new(seed), 0)
    }

    /// Cursor over `rng` starting at absolute counter `base`.
    pub fn from_rng(rng: CounterRng, base: u64) -> Self {
        Self { rng, next: base }
    }

    /// The underlying stream.
    pub fn rng(&self) -> CounterRng {
        self.rng
    }

    /// Next unread absolute counter.
    pub fn position(&self) -> u64 {
        self.next
    }

    /// Next word; advances the position with wraparound.
    pub fn next_u64(&mut self) -> u64 {
        let v = self.rng.sample(self.next);
        self.next = self.next.wrapping_add(1);
        v
    }

    /// Next word folded to 32 bits.
    pub fn next_u32(&mut self) -> u32 {
        u32::try_from(self.next_u64() >> 32).expect("high 32 bits fit in u32")
    }

    /// Word in `[lo, hi)`; `hi <= lo` returns `lo`, matching v1.
    pub fn next_range(&mut self, lo: u64, hi: u64) -> u64 {
        if hi <= lo {
            return lo;
        }
        lo + (self.next_u64() % (hi - lo))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rng;

    const SEED: u64 = 2040;

    #[test]
    fn same_seed_twice_identical() {
        let (a, b) = (CounterRng::new(SEED), CounterRng::new(SEED));
        assert_eq!(a, b);
        let va: [u64; 64] =
            core::array::from_fn(|i| a.sample(u64::try_from(i).expect("array index fits in u64")));
        let vb: [u64; 64] =
            core::array::from_fn(|i| b.sample(u64::try_from(i).expect("array index fits in u64")));
        assert_eq!(va, vb);
        assert!(va.iter().any(|&x| x != 0));
        let mut c = CounterCursor::new(SEED);
        for w in va {
            assert_eq!(c.next_u64(), w);
        }
    }

    #[test]
    fn chunkings_1_2_4_8_identical() {
        const N: usize = 256;
        let rng = CounterRng::new(SEED);
        let mut whole = [0u64; N];
        rng.fill(&mut whole, 17);
        for parts in [1usize, 2, 4, 8] {
            let mut part = [0u64; N];
            let step = N / parts;
            for p in 0..parts {
                let lo = p * step;
                rng.fill(
                    &mut part[lo..lo + step],
                    17 + u64::try_from(lo).expect("chunk offset fits in u64"),
                );
            }
            assert_eq!(part, whole, "chunking into {parts} differs");
        }
    }

    #[test]
    fn chunking_survives_counter_wrap() {
        let rng = CounterRng::new(99);
        let base = u64::MAX - 3;
        let mut whole = [0u64; 8];
        rng.fill(&mut whole, base);
        let mut split = [0u64; 8];
        rng.fill(&mut split[..5], base);
        rng.fill(&mut split[5..], base.wrapping_add(5));
        assert_eq!(whole, split);
    }

    #[test]
    fn adjacent_seeds_differ() {
        // v1 froze the low bit so 2k/2k+1 aliased; v2 keys must not.
        for k in [0u64, 1, 2, 7, 100, 1020, u64::MAX >> 33, u64::MAX >> 1] {
            let s0 = k.wrapping_mul(2);
            let s1 = s0.wrapping_add(1);
            let (a, b) = (CounterRng::new(s0), CounterRng::new(s1));
            assert_ne!(a.key(), b.key(), "aliased keys for seeds {s0}/{s1}");
            let va: [u64; 8] = core::array::from_fn(|i| {
                a.sample(u64::try_from(i).expect("array index fits in u64"))
            });
            let vb: [u64; 8] = core::array::from_fn(|i| {
                b.sample(u64::try_from(i).expect("array index fits in u64"))
            });
            assert_ne!(va, vb, "aliased stream for seeds {s0}/{s1}");
            assert_ne!(a.key()[0], a.key()[1], "collapsed key halves at {s0}");
        }
    }

    #[test]
    fn forks_islands_chunks_split() {
        let p = CounterRng::new(SEED);
        assert_eq!(p.fork(3), p.fork(3));
        assert_ne!(p.fork(3).key(), p.fork(4).key());
        assert_ne!(p.fork(3).sample(0), p.fork(4).sample(0));
        assert_eq!(p, CounterRng::new(SEED));
        assert_eq!(p.fork_named("ai"), p.fork_named("ai"));
        assert_ne!(
            p.fork_named("ai").sample(0),
            p.fork_named("world").sample(0)
        );
        assert_ne!(p.fork(0).key(), p.key());
        for i in 0..16u64 {
            assert_ne!(
                p.for_island(i).key(),
                p.for_chunk(i).key(),
                "island/chunk alias at {i}"
            );
            assert_ne!(p.for_island(i).key(), p.fork(i).key());
            assert_ne!(p.for_chunk(i).key(), p.fork(i).key());
            assert_ne!(p.for_island(i).sample(0), p.for_island(i + 1000).sample(0));
        }
    }

    #[test]
    fn outputs_cover_space() {
        // 1024 consecutive words are pairwise distinct; a collapsed round
        // function would repeat here.
        let rng = CounterRng::new(SEED);
        let mut v = [0u64; 1024];
        rng.fill(&mut v, 0);
        let mut sorted = v;
        sorted.sort_unstable();
        assert!(sorted.windows(2).all(|w| w[0] != w[1]));
    }

    #[test]
    fn seed_bit_flips_move_half_the_bits() {
        // One flipped seed bit must flip ~half of all output bits; a missing
        // key injection would leave this near 0%.
        let a: [u64; 64] = core::array::from_fn(|i| {
            CounterRng::new(SEED).sample(u64::try_from(i).expect("array index fits in u64"))
        });
        let b: [u64; 64] = core::array::from_fn(|i| {
            CounterRng::new(SEED ^ 1).sample(u64::try_from(i).expect("array index fits in u64"))
        });
        let flips: u32 = a.iter().zip(&b).map(|(x, y)| (x ^ y).count_ones()).sum();
        let frac = f64::from(flips) / 4096.0;
        assert!((0.40..0.60).contains(&frac), "seed avalanche {frac}");
    }

    #[test]
    fn counter_bit_flips_move_half_the_bits() {
        // Adjacent counters must be uncorrelated; a counter shifted straight
        // through would leave this near 1/64.
        let rng = CounterRng::new(SEED);
        let mut flips = 0u32;
        for i in 0..64u64 {
            flips += (rng.sample(i) ^ rng.sample(i ^ 1)).count_ones();
        }
        let frac = f64::from(flips) / 4096.0;
        assert!((0.40..0.60).contains(&frac), "counter avalanche {frac}");
    }

    #[test]
    fn range_and_cursor_bounds() {
        let mut c = CounterCursor::new(7);
        assert_eq!(c.position(), 0);
        for _ in 0..32 {
            let v = c.next_range(5, 10);
            assert!((5..10).contains(&v));
        }
        assert_eq!(c.next_range(7, 7), 7);
        assert_eq!(c.next_range(10, 5), 10);
        let stream = CounterRng::new(7).for_chunk(2);
        let mut d = CounterCursor::from_rng(stream, 100);
        assert_eq!(d.position(), 100);
        assert_eq!(d.next_u64(), stream.sample(100));
        assert_eq!(d.position(), 101);
        assert_eq!(d.rng(), stream);
    }

    #[test]
    fn stream_state_sizes() {
        assert_eq!(core::mem::size_of::<CounterRng>(), 16);
        assert_eq!(core::mem::size_of::<CounterCursor>(), 24);
    }

    #[test]
    fn v1_golden_unchanged() {
        // Counter streams must never move the frozen v1 stream the legacy golden replays.
        assert_eq!(Rng::new(2040).next_u64(), 2189151099318);
        assert_eq!(Rng::new(0).next_u64(), 1082269761);
        assert_eq!(Rng::new(2040).fork(0).state(), 4794064739117914115);
        assert_eq!(Rng::new(2040).fork(1).state(), 16082742265117291817);
        let v = [4794064739117914115u64, 16082742265117291817u64];
        assert_eq!(Rng::digest(&v), 1231717912943920180);
    }

    /// The crate's counting allocator proves zero heap use.
    fn stays_put(f: impl FnOnce()) {
        let ((), heap) = crate::alloc_probe::measure(f);
        assert_eq!(heap.bytes, 0, "counter draws must not allocate bytes");
        assert_eq!(heap.calls, 0, "counter draws must not allocate");
    }

    #[test]
    fn draws_and_fills_allocate_nothing() {
        let mut buf = [0u64; 512];
        let mut scratch = [0u64; 8];
        stays_put(|| {
            let rng = CounterRng::new(2040);
            let mut acc = 0u64;
            for i in 0..1024u64 {
                acc = acc.wrapping_add(rng.sample(i));
            }
            rng.fill(&mut buf, 0);
            rng.fill(&mut scratch, u64::MAX - 4);
            let child = rng.for_island(3).for_chunk(5).fork_named("ai").fork(9);
            let mut cur = CounterCursor::from_rng(child, 0);
            for _ in 0..64 {
                acc = acc.wrapping_add(cur.next_u64());
            }
            acc = acc.wrapping_add(u64::from(cur.next_u32()) + cur.next_range(1, 6));
            std::hint::black_box(acc);
        });
    }

    #[test]
    #[ignore = "timing: counter-vs-xorshift fill throughput; run under --perf in release"]
    fn counter_fill_within_2x_xorshift() {
        const N: usize = 1 << 18;
        let mut buf = vec![0u64; N];
        let cr = CounterRng::new(SEED);
        cr.fill(&mut buf, 0);
        let mut legacy = Rng::new(SEED);
        for s in buf.iter_mut() {
            *s = legacy.next_u64();
        }
        #[expect(
            clippy::disallowed_types,
            clippy::disallowed_methods,
            reason = "timing test reads the wall clock; it never enters the tick"
        )]
        fn best(mut f: impl FnMut()) -> u128 {
            let mut top = u128::MAX;
            for _ in 0..5 {
                let t = std::time::Instant::now();
                f();
                top = top.min(t.elapsed().as_nanos());
            }
            top
        }
        let x = best(|| {
            for s in buf.iter_mut() {
                *s = legacy.next_u64();
            }
            std::hint::black_box(buf.iter().fold(0u64, |a, &w| a.wrapping_add(w)));
        });
        let c = best(|| {
            cr.fill(&mut buf, 0);
            std::hint::black_box(buf.iter().fold(0u64, |a, &w| a.wrapping_add(w)));
        });
        let ratio = c as f64 / x as f64;
        println!("counter {c}ns vs xorshift {x}ns per {N} words: ratio {ratio:.2}");
        assert!(
            ratio < 2.0,
            "counter fill {c}ns must stay within 2x of xorshift {x}ns"
        );
    }
}
