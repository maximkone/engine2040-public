//! rng_wyhash: the wyrand-shape counter stream (wyhash family).
//! Owner: engine-core (std-only). Units: u64 seeds, counters and words.
//! Determinism: a pure integer word function takes (seed, counter) to one
//! `u64`; no shared state, no platform math, so the same seed and counter give
//! the same word everywhere, and any partition of a counter range draws the
//! same words. Errors: none by construction. Budget: 16 bytes per stream, one
//! 128-bit multiply and two xors per word, no heap use.
//!
//! Shape: the wyrand generator of the wyhash family (the one that passed
//! BigCrush and PractRand), written here as a counter: `sample(n)` is the n-th
//! draw of a sequential wyrand seeded with the same seed, which is what makes
//! `[base, base + len)` chunking free of shared state. Nothing hashed reads
//! this stream: the frozen legacy `Rng` words stay the golden's source.
//!
//! PORT: eldruin/wyhash-rs (`wyhash` 0.6.0, MIT OR Apache-2.0) `wyrng` shape,
//! constants and vector stream from wangyi-fudan/wyhash `wyhash.h` (The
//! Unlicense), the reference that Rust port matches. Reimplemented in plain
//! integer arithmetic: no vendor bytes, no dependency, no `unsafe`.
//! EVIDENCE: docs/EVIDENCE.md, wyhash-rs row.

/// wyrand's increment: seed advances by this odd constant per draw.
const MUL: u64 = 0xa076_1d64_78bd_642f;
/// wyrand's mix constant: drawn into the state before the multiply fold.
const XOR: u64 = 0xe703_7ed1_a0b4_28db;

/// The reference mix: a 64x64 multiply folded to 64 bits by xoring its high
/// and low halves. Every input bit reaches the output; the map is not one to
/// one, which is why the stream is a counter over the whole word.
#[inline]
fn wymix(a: u64, b: u64) -> u64 {
    let product = u128::from(a) * u128::from(b);
    let folded = product ^ (product >> 64);
    let [b0, b1, b2, b3, b4, b5, b6, b7, ..] = folded.to_le_bytes();
    u64::from_le_bytes([b0, b1, b2, b3, b4, b5, b6, b7])
}

/// The wyrand-shape counter stream: `sample(n)` is draw `n` of a sequential
/// wyrand seeded with `seed`. A 16-byte `Copy` value; drawing never mutates
/// shared state and `next_u64` advances only the caller's own copy. The
/// upstream vectors hold for `next_u64` from a fresh stream (see the tests).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WyRng {
    /// The stream's seed: upstream's mutable seed word before its first draw.
    seed: u64,
    /// Absolute counter of the next draw (`next_u64` reads it, then advances).
    next: u64,
}

impl WyRng {
    /// New stream from a seed; `sample(0)` is its first draw.
    pub const fn new(seed: u64) -> Self {
        Self { seed, next: 0 }
    }

    /// The stream's seed, for snapshots and tests.
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// The next counter to read, for snapshots and tests.
    pub const fn position(&self) -> u64 {
        self.next
    }

    /// Pure draw: word `counter` of this stream, counting from 0. Two chunks
    /// reading disjoint counter ranges share nothing yet stay reproducible
    /// under any re-chunking; the counter wraps at `u64::MAX`.
    pub fn sample(&self, counter: u64) -> u64 {
        let state = self
            .seed
            .wrapping_add(MUL.wrapping_mul(counter.wrapping_add(1)));
        wymix(state ^ XOR, state)
    }

    /// Next word, advancing the position (wraps with the counter).
    pub fn next_u64(&mut self) -> u64 {
        let word = self.sample(self.next);
        self.next = self.next.wrapping_add(1);
        word
    }

    /// Low 32 bits of the next word, the narrowing the reference port uses.
    pub fn next_u32(&mut self) -> u32 {
        let [b0, b1, b2, b3, ..] = self.next_u64().to_le_bytes();
        u32::from_le_bytes([b0, b1, b2, b3])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checked::f64_from_u64;
    use crate::hash::{hash_u64s, GOLDEN_GAMMA};

    // Upstream vectors. The first words are printed by the reference port's own
    // doc comments (`wyrng` from seed 3, `WyRng` from the byte seed [0..7] and
    // the default seed 0); the three later words follow the same reference
    // formula, so the whole pinned stream is a hard number.
    const SEED_ZERO: [u64; 4] = [
        0x111c_b3a7_8f59_a58e,
        0xceab_d938_ff4e_856d,
        0x61fb_5131_8f47_d2a4,
        0x78bd_03c4_9190_9760,
    ];
    const SEED_THREE: [u64; 4] = [
        0x03e9_9a77_2750_dcbe,
        0x6665_c0de_f316_abb1,
        0xcdcd_e43f_5c0d_b7c0,
        0xcfb5_4431_bf74_1e5f,
    ];
    /// The reference's byte seed `[0, 1, 2, 3, 4, 5, 6, 7]`, read LE.
    const SEED_BYTES: u64 = 0x0706_0504_0302_0100;
    const SEED_BYTES_FIRST: u64 = 0xd730_1357_74c6_ae31;
    /// The seed word the reference prints after one draw from seed 3.
    const SEED_THREE_AFTER_ONE: u64 = 0xa076_1d64_78bd_6432;
    /// FNV-1a 64 over the first 1024 words drawn from [`SEED_BYTES`], LE bytes.
    const DIGEST_1024: u64 = 0x0406_eac9_cd39_36c7;
    /// The p95 bar per draw: 0.01 ms in nanoseconds.
    const P95_BAR_NS: u64 = 10_000;

    fn draws4(rng: &mut WyRng) -> [u64; 4] {
        core::array::from_fn(|_| rng.next_u64())
    }

    #[test]
    fn upstream_vectors_pin_the_stream() {
        // Twice over: the sequential walk and the pure `sample` of a fresh
        // stream must both be the reference words, so a drift in either the
        // state advance or the mix shows up here.
        assert_eq!(draws4(&mut WyRng::new(0)), SEED_ZERO);
        assert_eq!(
            core::array::from_fn(|i| WyRng::new(0).sample(i as u64)),
            SEED_ZERO
        );
        assert_eq!(draws4(&mut WyRng::new(3)), SEED_THREE);
        assert_eq!(
            core::array::from_fn(|i| WyRng::new(3).sample(i as u64)),
            SEED_THREE
        );
        assert_eq!(WyRng::new(SEED_BYTES).next_u64(), SEED_BYTES_FIRST);
        assert_eq!(WyRng::new(SEED_BYTES).sample(0), SEED_BYTES_FIRST);
        // The reference advances its seed by exactly one increment per draw.
        assert_eq!(WyRng::new(3).seed().wrapping_add(MUL), SEED_THREE_AFTER_ONE);
    }

    #[test]
    fn sequential_walk_matches_the_pure_sample_and_advances() {
        let mut rng = WyRng::new(SEED_BYTES);
        assert_eq!(rng.position(), 0);
        for i in 0..64u64 {
            let fresh = WyRng::new(SEED_BYTES);
            assert_eq!(rng.next_u64(), fresh.sample(i), "draw {i}");
            assert_eq!(rng.position(), i + 1, "position after draw {i}");
            assert_eq!(rng.seed(), SEED_BYTES, "the seed never moves");
        }
    }

    #[test]
    fn chunkings_1_2_4_8_identical() {
        const N: usize = 256;
        let rng = WyRng::new(SEED_BYTES);
        let whole: [u64; N] = core::array::from_fn(|i| rng.sample(i as u64));
        for parts in [1usize, 2, 4, 8] {
            let mut part = [0u64; N];
            let step = N / parts;
            for (p, chunk) in part.chunks_mut(step).enumerate() {
                let lo = p * step;
                for (i, slot) in chunk.iter_mut().enumerate() {
                    *slot = rng.sample((lo + i) as u64);
                }
            }
            assert_eq!(part, whole, "chunking into {parts} differs");
        }
        // A chunk read at an offset is that slice of the range: where a reader
        // starts, and who read before it, does not move a word. Near the counter
        // top a second stream draws the same words, so the wrap has no seam.
        let offset: [u64; 8] = core::array::from_fn(|i| rng.sample(17 + i as u64));
        assert_eq!(&offset[..], &whole[17..25], "an offset chunk is that slice");
        let seam = u64::MAX - 3;
        let top: [u64; 8] = core::array::from_fn(|i| rng.sample(seam.wrapping_add(i as u64)));
        let again: [u64; 8] =
            core::array::from_fn(|i| WyRng::new(SEED_BYTES).sample(seam.wrapping_add(i as u64)));
        assert_eq!(
            top, again,
            "the same counters draw the same words at the top"
        );
    }

    #[test]
    fn stream_digest_is_pinned_and_reruns_identically() {
        // Cross-platform equality: the digest is integer-only, so the runner and
        // this PC must print the same 16 hex digits from the same seed.
        let mut rng = WyRng::new(SEED_BYTES);
        let words: [u64; 1024] = core::array::from_fn(|_| rng.next_u64());
        assert_eq!(hash_u64s(&words), DIGEST_1024);
        let mut again = WyRng::new(SEED_BYTES);
        let second: [u64; 1024] = core::array::from_fn(|_| again.next_u64());
        assert_eq!(second, words, "the rerun is byte identical");
        let mut other = WyRng::new(SEED_BYTES ^ 1);
        let alt: [u64; 1024] = core::array::from_fn(|_| other.next_u64());
        assert_ne!(
            hash_u64s(&alt),
            DIGEST_1024,
            "a different seed moves the digest"
        );
    }

    #[test]
    fn next_u32_is_the_low_half_of_the_next_word() {
        let mut rng = WyRng::new(11);
        for i in 0..16u64 {
            let word = WyRng::new(11).sample(i);
            let low = u32::try_from(word & u64::from(u32::MAX)).expect("masked to 32 bits");
            assert_eq!(rng.next_u32(), low, "narrowing of draw {i}");
        }
    }

    #[test]
    fn seeded_sweep_is_balanced_and_uncorrelated() {
        // 64 seeds x 256 words: bit balance, a chi-square over the 16 low-nibble
        // buckets, and the lag-1 serial correlation of the raw words. The bars
        // are hard numbers: the balance window is about +-4 sigma of 2^20 bits,
        // the chi-square bar is the 0.1% critical value at 15 degrees of
        // freedom, and the correlation bar is well under one sigma for 16k pairs.
        const SEEDS: u64 = 64;
        const WORDS: usize = 256;
        const CHI2_BAR: f64 = 37.7;
        const CORR_BAR: f64 = 0.01;
        let mut bits = 0u64;
        let mut ones = 0u64;
        let mut buckets = [0u64; 16];
        let (mut sx, mut sy, mut sxx, mut syy, mut sxy) = (0.0f64, 0.0, 0.0, 0.0, 0.0);
        let mut pairs = 0u64;
        for s in 0..SEEDS {
            let mut rng = WyRng::new(s.wrapping_mul(GOLDEN_GAMMA) ^ 0xa5a5_5a5a);
            let mut prev = 0.0f64;
            for word_index in 0..WORDS {
                let word = rng.next_u64();
                bits += 64;
                ones += u64::from(word.count_ones());
                buckets[usize::try_from(word & 0xf).expect("low nibble")] += 1;
                let y = f64_from_u64(word);
                if word_index > 0 {
                    sx += prev;
                    sy += y;
                    sxx += prev * prev;
                    syy += y * y;
                    sxy += prev * y;
                    pairs += 1;
                }
                prev = y;
            }
        }
        let balance = f64_from_u64(ones) / f64_from_u64(bits);
        let expected = f64_from_u64(SEEDS * WORDS as u64) / 16.0;
        let chi2: f64 = buckets
            .iter()
            .map(|n| {
                let d = f64_from_u64(*n) - expected;
                d * d / expected
            })
            .sum();
        let n = f64_from_u64(pairs);
        let corr = (n * sxy - sx * sy) / ((n * sxx - sx * sx) * (n * syy - sy * sy)).sqrt();
        println!(
            "wyrand sweep: bits={bits} balance={balance:.6} chi2={chi2:.3} lag1={corr:.6} pairs={pairs}"
        );
        assert!(
            (0.499..0.501).contains(&balance),
            "bit balance {balance:.6} outside the window"
        );
        assert!(chi2 < CHI2_BAR, "chi-square {chi2:.3} over the bar");
        assert!(
            corr.abs() < CORR_BAR,
            "lag-1 correlation {corr:.6} over the bar"
        );
    }

    #[test]
    fn draws_and_samples_allocate_nothing() {
        let mut buf = [0u64; 512];
        let mut scratch = [0u64; 8];
        let ((), heap) = crate::alloc_probe::measure(|| {
            let rng = WyRng::new(2040);
            let mut acc = 0u64;
            for i in 0..1024u64 {
                acc = acc.wrapping_add(rng.sample(i));
            }
            for (i, slot) in buf.iter_mut().enumerate() {
                *slot = rng.sample(i as u64);
            }
            for (i, slot) in scratch.iter_mut().enumerate() {
                *slot = rng.sample(0xfedc_ba98_7654_3210u64.wrapping_add(i as u64));
            }
            let mut walk = WyRng::new(7);
            for _ in 0..64 {
                acc = acc.wrapping_add(walk.next_u64());
            }
            acc = acc.wrapping_add(u64::from(walk.next_u32()));
            std::hint::black_box(acc);
        });
        assert_eq!(heap.calls, 0, "wyrand draws must not allocate");
        assert_eq!(heap.bytes, 0, "wyrand draws must not allocate bytes");
    }

    #[test]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    #[ignore = "timing: p95 per draw under the 0.01 ms bar; run under --perf in release"]
    fn p95_draw_time_stays_under_the_bar() {
        const BATCHES: usize = 1000;
        const DRAWS: u64 = 256;
        let rng = WyRng::new(SEED_BYTES);
        let mut acc = 0u64;
        // Warm the draw path and the counter, outside the measured window.
        for i in 0..DRAWS {
            acc = acc.wrapping_add(rng.sample(i));
        }
        let mut ns = [0u64; BATCHES];
        for (batch, slot) in ns.iter_mut().enumerate() {
            let base = (batch as u64).wrapping_mul(DRAWS);
            let start = std::time::Instant::now();
            for i in 0..DRAWS {
                acc = acc.wrapping_add(rng.sample(base.wrapping_add(i)));
            }
            let elapsed = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
            *slot = elapsed / DRAWS;
        }
        std::hint::black_box(acc);
        ns.sort_unstable();
        let p50 = ns[BATCHES / 2];
        let p95 = ns[BATCHES * 95 / 100];
        println!("wyrand per draw: p50 {p50} ns, p95 {p95} ns over {BATCHES} x {DRAWS} draws");
        assert!(
            p95 <= P95_BAR_NS,
            "p95 {p95} ns per draw over the {P95_BAR_NS} ns (0.01 ms) bar"
        );
    }
}
