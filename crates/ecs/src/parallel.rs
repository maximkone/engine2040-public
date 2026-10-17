//! parallel: deterministic fork-join over index ranges for bake/verify/import.
//!
//! Owner: this module; bake/verify/broadphase lanes call in, the sim tick
//! never does (Rapier parallelism stays out: dropped, never wired here).
//! Units: items (range indices) and chunks (elements per chunk).
//! Determinism: chunks reuse `jobs::split_ranges` boundaries, per-index work
//! draws keyed counter streams at absolute counters, and partials fold on the
//! caller in declared chunk order, so thread count, chunking and completion
//! order never enter the result. Errors: typed [`JobsError`]; empty ranges
//! refuse, and a reduce-order violation is impossible by construction.

use std::ops::Range;

use crate::jobs::{split_ranges, ChunkSize};
use crate::pool::{self, JobPool};

/// Typed fork-join failure. Every variant names the culprit; corrupt or
/// empty input is always an `Err`, never a guess or a partial fold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobsError {
    /// Range holds no items (`start >= end`); holds both bounds.
    EmptyRange {
        /// Range start that was offered.
        start: usize,
        /// Range end that was offered.
        end: usize,
    },
    /// A collect slot stayed empty after every job ran; holds its offset
    /// from the range start. Impossible by construction, kept as a loud
    /// `Err` so a coverage hole can never ship as a short or zeroed output.
    Uncovered {
        /// Offset of the first unwritten slot.
        index: usize,
    },
}

impl core::fmt::Display for JobsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EmptyRange { start, end } => {
                write!(f, "jobs fork-join refused empty range {start}..{end}")
            }
            Self::Uncovered { index } => {
                write!(f, "jobs collect left slot {index} unwritten")
            }
        }
    }
}

impl std::error::Error for JobsError {}

/// Fork `range` into index-ordered chunks of at most `chunk` items, run
/// `f` once per chunk sub-range as a pool job, then fold the partials
/// into `init` with `reduce` on the caller in declared chunk order. This is
/// [`fork_join_on`] on the crate's shared pool. A panicking `f` unwinds
/// on the caller exactly as the serial call would. Bulk loops only (bake,
/// verify, import, broadphase); never sim/net tick paths.
///
/// Determinism: chunk boundaries come from [`split_ranges`](crate::jobs::split_ranges)
/// and the fold always visits chunk `0, 1, ...` in order, so every
/// chunking of the same range — including a single chunk — folds the
/// same partial sequence shape. Callers keep order-sensitive work inside
/// `f` per index (or return per-chunk item vectors folded in order) and
/// keep `reduce` a plain fold over the ordered partials.
pub fn fork_join<T, F, Acc, G>(
    range: Range<usize>,
    chunk: ChunkSize,
    f: F,
    init: Acc,
    reduce: G,
) -> Result<Acc, JobsError>
where
    T: Send,
    F: Fn(Range<usize>) -> T + Sync,
    G: Fn(Acc, T) -> Acc,
{
    fork_join_on(pool::shared(), range, chunk, f, init, reduce)
}

/// Fork-join on an explicit pool: the same order and error contract as
/// [`fork_join`], but the caller picks the worker count, so thread-count
/// invariance (1, 2, 4, 8 workers) is provable pool by pool. One job per
/// chunk on the given pool, partials kept in chunk slots
/// ([`JobPool::par_chunks_map`](crate::pool::JobPool::par_chunks_map));
/// the single-chunk case runs inline with no job. A panicking `f` unwinds
/// on the caller exactly as the serial call would. Bulk loops only (bake,
/// verify, import, broadphase); never sim/net tick paths.
///
/// Caller (next): the arena bake lanes and the perf soak drive scaled fills
/// through this entry; per-index work samples a keyed counter stream
/// (`engine_core::CounterRng`) at the absolute item counter, so any
/// partition of the range draws identical words.
///
/// Determinism: the pool decides only where a job runs, never what it
/// returns; boundaries and the declared-order fold match [`fork_join`].
pub fn fork_join_on<T, F, Acc, G>(
    pool: &JobPool,
    range: Range<usize>,
    chunk: ChunkSize,
    f: F,
    init: Acc,
    reduce: G,
) -> Result<Acc, JobsError>
where
    T: Send,
    F: Fn(Range<usize>) -> T + Sync,
    G: Fn(Acc, T) -> Acc,
{
    let Range { start, end } = range;
    if start >= end {
        return Err(JobsError::EmptyRange { start, end });
    }
    // Offsets stay below `end`: each split offset is `< len`, so
    // `offset + start < end` cannot overflow.
    let bounds: Vec<Range<usize>> = split_ranges(end - start, chunk)
        .into_iter()
        .map(|(a, b)| (a + start)..(b + start))
        .collect();
    debug_assert!(!bounds.is_empty(), "nonempty range splits to a chunk");
    // One bound per job; a single bound runs inline. The pool re-raises
    // the original panic payload, matching what the serial call does.
    let partials = pool.par_chunks_map(&bounds, ChunkSize::new(1), |_, subs| {
        subs.first().cloned().map(&f)
    });
    // Declared-order fold on the caller: partials sit in chunk order, so
    // completion order never decides state.
    Ok(partials.into_iter().flatten().fold(init, reduce))
}

/// Ordered slot-write collect: run `f(i)` for every `i` in `range` on `pool`
/// and return the results in index order. The output is sized to exactly
/// `range.len()` slots up front, each job owns a disjoint index slice of it
/// and writes slot `i` once, and the sealed result must cover every slot, so
/// no job output is moved, appended or reordered afterwards. Equal to the
/// serial `range.map(f).collect()` for every worker count and chunking. A
/// panicking `f` unwinds on the caller. Bulk loops only (bake, verify,
/// anim-eval lanes); never sim/net tick paths.
///
/// Caller: [`crate::dispatch_plan::DispatchPlan`] runs every stage through it.
///
/// Determinism: slot `i` is written only with `f(start + i)`; the pool decides
/// where a job runs, never which slot it fills.
pub fn collect_indexed_on<T, F>(
    pool: &JobPool,
    range: Range<usize>,
    chunk: ChunkSize,
    f: F,
) -> Result<Vec<T>, JobsError>
where
    T: Send,
    F: Fn(usize) -> T + Sync,
{
    let Range { start, end } = range;
    if start >= end {
        return Err(JobsError::EmptyRange { start, end });
    }
    let len = end - start;
    let mut slots: Vec<Option<T>> = Vec::with_capacity(len);
    slots.resize_with(len, || None);
    let size = chunk.value();
    if len <= size {
        for (i, slot) in slots.iter_mut().enumerate() {
            *slot = Some(f(start + i));
        }
    } else {
        let f = &f;
        pool.scope(|s| {
            for (k, part) in slots.chunks_mut(size).enumerate() {
                let base = start + k * size;
                s.spawn(move || {
                    for (i, slot) in part.iter_mut().enumerate() {
                        *slot = Some(f(base + i));
                    }
                });
            }
        });
    }
    seal_slots(slots)
}

/// Full-coverage check: every slot written, else the first hole is named.
fn seal_slots<T>(slots: Vec<Option<T>>) -> Result<Vec<T>, JobsError> {
    let mut out = Vec::with_capacity(slots.len());
    for (index, slot) in slots.into_iter().enumerate() {
        out.push(slot.ok_or(JobsError::Uncovered { index })?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::JobPool;
    use engine_core::{hash_u64s, CounterRng};

    /// Per-index work shared by the order proofs below.
    fn work(i: usize) -> u64 {
        (i as u64).wrapping_mul(0x9e3779b97f4a7c15).rotate_left(17)
    }

    /// Serial sum of `work` over a range: the oracle every chunking must match.
    fn serial_sum(range: Range<usize>) -> u64 {
        range.map(work).fold(0, |a, x| a.wrapping_add(x))
    }

    /// Chunk-partial sum helper: per-chunk fold in index order.
    fn chunk_sum(sub: Range<usize>) -> u64 {
        sub.map(work).fold(0, |a, x| a.wrapping_add(x))
    }

    #[test]
    fn empty_ranges_refuse_loudly() {
        assert_eq!(
            fork_join(5..5, ChunkSize::new(4), chunk_sum, 0, |a, p| a + p).err(),
            Some(JobsError::EmptyRange { start: 5, end: 5 })
        );
        // Inverted bounds refuse too; built as a struct literal so the
        // range-expression lint does not fire on the test itself.
        assert_eq!(
            fork_join(
                Range { start: 9, end: 3 },
                ChunkSize::new(4),
                chunk_sum,
                0,
                |a, p| a + p
            )
            .err(),
            Some(JobsError::EmptyRange { start: 9, end: 3 })
        );
        assert_eq!(
            fork_join(0..0, ChunkSize::new(0), chunk_sum, 0, |a, p| a + p).err(),
            Some(JobsError::EmptyRange { start: 0, end: 0 })
        );
        // Error text names the bounds for operator diagnostics.
        assert!(JobsError::EmptyRange { start: 9, end: 3 }
            .to_string()
            .contains("9..3"));
    }

    #[test]
    fn single_element_matches_serial_under_every_chunking() {
        for chunk in [0, 1, 2, 7, 64, 1024] {
            let got = fork_join(41..42, ChunkSize::new(chunk), chunk_sum, 0, |a: u64, p| {
                a.wrapping_add(p)
            })
            .expect("single item runs");
            assert_eq!(got, serial_sum(41..42), "chunk {chunk}");
        }
    }

    #[test]
    fn one_thread_and_n_threads_fold_the_same_sum() {
        // Single-thread shape (one chunk) against many-thread shapes.
        let single = fork_join(0..1000, ChunkSize::new(4096), chunk_sum, 0, |a: u64, p| {
            a.wrapping_add(p)
        })
        .expect("single chunk runs");
        assert_eq!(single, serial_sum(0..1000));
        for chunk in [1, 2, 3, 7, 64, 333] {
            let par = fork_join(0..1000, ChunkSize::new(chunk), chunk_sum, 0, |a: u64, p| {
                a.wrapping_add(p)
            })
            .expect("chunked run");
            assert_eq!(par, single, "chunk {chunk}");
        }
    }

    #[test]
    fn odd_splits_match_serial() {
        // Lengths no chunk divides: remainders, primes, offset starts.
        for (start, end, chunk) in [
            (0, 10, 3),
            (0, 101, 7),
            (5, 1009, 64),
            (3, 97, 1),
            (7, 8, 99),
        ] {
            let par = fork_join(
                start..end,
                ChunkSize::new(chunk),
                chunk_sum,
                0,
                |a: u64, p| a.wrapping_add(p),
            )
            .expect("odd split runs");
            assert_eq!(par, serial_sum(start..end), "{start}..{end} c{chunk}");
        }
        // Boundaries reused from jobs.rs exactly: 10 items at 3 per chunk.
        assert_eq!(
            split_ranges(10, ChunkSize::new(3)),
            vec![(0, 3), (3, 6), (6, 9), (9, 10)]
        );
    }

    #[test]
    fn declared_order_holds_for_noncommutative_folds() {
        // Per-chunk item vectors folded in chunk order equal the serial
        // item order, so even an order-sensitive combine agrees.
        let serial: Vec<u64> = (0..500).map(work).collect();
        for chunk in [1, 3, 7, 64, 500, 4096] {
            let ordered = fork_join(
                0..500,
                ChunkSize::new(chunk),
                |sub| sub.map(work).collect::<Vec<u64>>(),
                Vec::new(),
                |mut acc: Vec<u64>, mut part| {
                    acc.append(&mut part);
                    acc
                },
            )
            .expect("ordered fold runs");
            assert_eq!(ordered, serial, "chunk {chunk}");
        }
        // Polynomial hash over the ordered items: any reorder changes it.
        let hash = |xs: &[u64]| {
            xs.iter().fold(0u64, |a, x| {
                a.wrapping_mul(1_000_003).wrapping_add(*x ^ 0x9e3779b9)
            })
        };
        let serial_hash = hash(&serial);
        for chunk in [2, 5, 37] {
            let par = fork_join(
                0..500,
                ChunkSize::new(chunk),
                |sub| sub.map(work).collect::<Vec<u64>>(),
                Vec::new(),
                |mut acc: Vec<u64>, mut part| {
                    acc.append(&mut part);
                    acc
                },
            )
            .expect("hash fold runs");
            assert_eq!(hash(&par), serial_hash, "chunk {chunk}");
            // A reversed chunk visit would hash differently: the test is real.
            let mut reversed = par.clone();
            reversed.reverse();
            assert_ne!(hash(&reversed), serial_hash);
        }
    }

    #[test]
    fn partials_arrive_in_declared_chunk_order() {
        // Reduce records each chunk's start; the visit order must be sorted.
        let expected: Vec<usize> = split_ranges(100, ChunkSize::new(7))
            .into_iter()
            .map(|(a, _)| a)
            .collect();
        for _ in 0..20 {
            let order = fork_join(
                0..100,
                ChunkSize::new(7),
                |sub| {
                    // Jitter completion order without wall clocks: uneven
                    // busy work plus a yield on odd chunks.
                    let mut x = sub.start as u64;
                    for _ in 0..(sub.start % 11) * 500 {
                        x = x.wrapping_mul(0x9e3779b97f4a7c15).rotate_left(5);
                    }
                    if sub.start % 2 == 1 {
                        std::thread::yield_now();
                    }
                    (sub.start, x ^ sub.end as u64)
                },
                Vec::new(),
                |mut acc: Vec<usize>, (start, _)| {
                    acc.push(start);
                    acc
                },
            )
            .expect("ordered visit runs");
            assert_eq!(order, expected);
        }
    }

    #[test]
    fn repeated_runs_hash_identically_despite_jitter() {
        // Completion-order independence: 50 jittered runs, one digest.
        let mut first = None;
        for _ in 0..50 {
            let got = fork_join(
                0..512,
                ChunkSize::new(9),
                |sub| {
                    let mut x = 0u64;
                    for i in sub.clone() {
                        if i % 2 == 0 {
                            std::thread::yield_now();
                        }
                        x = x.wrapping_add(work(i));
                    }
                    x
                },
                0u64,
                |a, p| a.wrapping_add(p),
            )
            .expect("jittered run");
            assert_eq!(got, serial_sum(0..512));
            if let Some(prev) = first {
                assert_eq!(got, prev);
            } else {
                first = Some(got);
            }
        }
    }

    #[test]
    fn a_panicking_chunk_unwinds_on_the_caller() {
        let run = std::panic::catch_unwind(|| {
            fork_join(
                0..100,
                ChunkSize::new(10),
                |sub| {
                    let at = sub.start;
                    assert!(at != 40, "chunk {at} blew up");
                    sub.start
                },
                0,
                |a, p| a + p,
            )
        });
        let payload = run.expect_err("never a silent zero");
        let text = payload
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_default();
        assert!(text.contains("chunk 40 blew up"), "{text:?}");
    }

    #[test]
    fn nonzero_base_ranges_match_serial() {
        // Offset ranges (bake windows rarely start at zero) fold the same.
        let got = fork_join(1000..2000, ChunkSize::new(63), chunk_sum, 0, |a: u64, p| {
            a.wrapping_add(p)
        })
        .expect("offset range runs");
        assert_eq!(got, serial_sum(1000..2000));
    }

    /// Bake seed shared by the lane proofs below.
    const BAKE_SEED: u64 = 2040;

    /// Overlapping box strip for the broadphase proof: box `i` covers
    /// `[cell, cell + 2]` on integer cells, so neighbours overlap and chunk
    /// folds in declared order reproduce the serial pair order exactly.
    fn chunk_pairs(cells: &[u64], sub: Range<usize>) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for i in sub {
            for j in (i + 1)..cells.len() {
                let (a, b) = (cells[i], cells[j]);
                let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                if hi - lo <= 2 {
                    out.push((i, j));
                }
            }
        }
        out
    }

    fn serial_pairs(cells: &[u64]) -> Vec<(usize, usize)> {
        chunk_pairs(cells, 0..cells.len())
    }

    #[test]
    fn bake_words_match_serial_across_threads_and_chunks() {
        // Bake lane: per-index words drawn from a keyed counter stream at
        // absolute counters, so every partition draws identical words and the
        // declared-order fold reproduces the serial byte order.
        const LEN: usize = 4096;
        let mut serial = vec![0u64; LEN];
        CounterRng::new(BAKE_SEED).fill(&mut serial, 0);
        assert!(serial.windows(2).any(|w| w[0] != w[1]));
        let serial_hash = hash_u64s(&serial);
        for workers in [0, 1, 2, 4, 8] {
            let pool = JobPool::new(workers).expect("pool starts");
            for chunk in [1, 7, 64, 512, LEN] {
                let got = fork_join_on(
                    &pool,
                    0..LEN,
                    ChunkSize::new(chunk),
                    |sub| {
                        let base = u64::try_from(sub.start).expect("bake index fits in u64");
                        let mut part = vec![0u64; sub.len()];
                        CounterRng::new(BAKE_SEED).fill(&mut part, base);
                        part
                    },
                    Vec::new(),
                    |mut acc: Vec<u64>, mut part| {
                        acc.append(&mut part);
                        acc
                    },
                )
                .expect("bake runs");
                assert_eq!(got, serial, "workers {workers} chunk {chunk}");
                assert_eq!(
                    hash_u64s(&got),
                    serial_hash,
                    "workers {workers} chunk {chunk}"
                );
            }
        }
        // The shared-pool entry agrees with the explicit pools.
        let single = fork_join(
            0..LEN,
            ChunkSize::new(LEN + 1),
            |sub| {
                let base = u64::try_from(sub.start).expect("bake index fits in u64");
                let mut part = vec![0u64; sub.len()];
                CounterRng::new(BAKE_SEED).fill(&mut part, base);
                part
            },
            Vec::new(),
            |mut acc: Vec<u64>, mut part| {
                acc.append(&mut part);
                acc
            },
        )
        .expect("single chunk runs");
        assert_eq!(single, serial);
    }

    #[test]
    fn verify_sums_match_serial_across_threads_and_chunks() {
        // Verify lane: per-chunk partial sums folded in declared order give
        // one total whatever the worker count or the chunking.
        const LEN: usize = 2048;
        let mut words = vec![0u64; LEN];
        CounterRng::new(99).fill(&mut words, 0);
        let serial: u64 = words.iter().fold(0, |a, x| a.wrapping_add(*x));
        for workers in [0, 1, 2, 4, 8] {
            let pool = JobPool::new(workers).expect("pool starts");
            for chunk in [1, 9, 128, LEN] {
                let got = fork_join_on(
                    &pool,
                    0..LEN,
                    ChunkSize::new(chunk),
                    |sub| sub.map(|i| words[i]).fold(0u64, |a, x| a.wrapping_add(x)),
                    0u64,
                    |a, p| a.wrapping_add(p),
                )
                .expect("verify runs");
                assert_eq!(got, serial, "workers {workers} chunk {chunk}");
            }
        }
        // Not vacuous: one flipped word moves the total.
        words[7] = words[7].wrapping_add(1);
        assert_ne!(words.iter().fold(0u64, |a, x| a.wrapping_add(*x)), serial);
    }

    #[test]
    fn broadphase_pairs_match_serial_across_threads_and_chunks() {
        // Broadphase lane: overlapping index pairs emitted per chunk in index
        // order, so every worker count and chunking lists the same pairs.
        const COUNT: usize = 256;
        let mut cells = vec![0u64; COUNT];
        CounterRng::new(7).fill(&mut cells, 0);
        let origin: Vec<u64> = cells.iter().map(|w| w % 64).collect();
        let serial = serial_pairs(&origin);
        assert!(!serial.is_empty(), "the fixture must overlap somewhere");
        for workers in [0, 1, 2, 4, 8] {
            let pool = JobPool::new(workers).expect("pool starts");
            for chunk in [1, 5, 64, COUNT] {
                let got = fork_join_on(
                    &pool,
                    0..COUNT,
                    ChunkSize::new(chunk),
                    |sub| chunk_pairs(&origin, sub),
                    Vec::new(),
                    |mut acc: Vec<(usize, usize)>, mut part| {
                        acc.append(&mut part);
                        acc
                    },
                )
                .expect("broadphase runs");
                assert_eq!(got, serial, "workers {workers} chunk {chunk}");
            }
        }
    }

    #[test]
    fn slot_collect_matches_serial_across_threads_and_chunks() {
        // Slot `i` holds `work(start + i)` whatever the worker count, the
        // chunking or the range offset; the output is exactly range-long.
        for (start, end) in [(0, 1), (0, 1000), (37, 1100)] {
            let serial: Vec<u64> = (start..end).map(work).collect();
            for workers in [0, 1, 2, 4, 8] {
                let pool = JobPool::new(workers).expect("pool starts");
                for chunk in [0, 1, 7, 64, 333, 4096] {
                    let got = collect_indexed_on(&pool, start..end, ChunkSize::new(chunk), work)
                        .expect("collect runs");
                    assert_eq!(got, serial, "{start}..{end} w{workers} c{chunk}");
                    assert_eq!(got.len(), end - start);
                }
            }
        }
    }

    #[test]
    fn slot_collect_refuses_empty_and_names_a_hole() {
        let pool = JobPool::new(2).expect("pool starts");
        assert_eq!(
            collect_indexed_on(&pool, 4..4, ChunkSize::new(2), work).err(),
            Some(JobsError::EmptyRange { start: 4, end: 4 })
        );
        // A slot no job wrote is a typed error naming the slot, never a
        // short or defaulted output.
        let holey = vec![Some(1u64), Some(2), None, Some(4)];
        assert_eq!(seal_slots(holey), Err(JobsError::Uncovered { index: 2 }));
        assert!(JobsError::Uncovered { index: 2 }
            .to_string()
            .contains("slot 2"));
        assert_eq!(seal_slots(vec![Some(7u64)]), Ok(vec![7]));
    }

    #[test]
    fn a_panicking_slot_unwinds_on_the_caller() {
        let pool = JobPool::new(2).expect("pool starts");
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            collect_indexed_on(&pool, 0..100, ChunkSize::new(10), |i| {
                assert!(i != 55, "slot {i} blew up");
                i
            })
        }));
        let payload = run.expect_err("never a silent hole");
        let text = payload
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_default();
        assert!(text.contains("slot 55 blew up"), "{text:?}");
    }

    /// Scaled bake throughput: parallel must clear 2x over serial, else the
    /// bake lane is killed (kept single-threaded). Timing only; the byte
    /// proof above stays the determinism gate. Run in release with `-- --ignored`.
    #[test]
    #[ignore = "timing: needs a warm multi-core box; run under --perf in release"]
    fn scaled_bake_parallel_clears_2x_over_serial() {
        #[expect(
            clippy::disallowed_types,
            clippy::disallowed_methods,
            reason = "timing test reads the wall clock; it never enters the tick"
        )]
        fn best_ms(mut run: impl FnMut()) -> f64 {
            let mut best = f64::MAX;
            for _ in 0..5 {
                let start = std::time::Instant::now();
                run();
                let ms = start.elapsed().as_secs_f64() * 1000.0;
                if ms < best {
                    best = ms;
                }
            }
            best
        }
        // Per-item bake word: one keyed draw plus mixing rounds, heavy enough
        // that thread startup never decides the ratio.
        fn word(rng: &CounterRng, i: u64) -> u64 {
            let mut v = rng.sample(i);
            for _ in 0..30 {
                v = engine_core::splitmix64(v ^ i);
            }
            v
        }
        const LEN: usize = 200_000;
        let rng = CounterRng::new(BAKE_SEED);
        let to_word = |i: usize| word(&rng, u64::try_from(i).expect("index fits in u64"));
        let serial_ms = best_ms(|| {
            let out: Vec<u64> = (0..LEN).map(to_word).collect();
            std::hint::black_box(out.iter().fold(0u64, |a, x| a.wrapping_add(*x)));
        });
        let pool = JobPool::new(8).expect("pool starts");
        let parallel_ms = best_ms(|| {
            let out = fork_join_on(
                &pool,
                0..LEN,
                ChunkSize::new(4096),
                |sub| sub.map(to_word).collect::<Vec<u64>>(),
                Vec::new(),
                |mut acc: Vec<u64>, mut part| {
                    acc.append(&mut part);
                    acc
                },
            )
            .expect("scaled bake runs");
            std::hint::black_box(out.iter().fold(0u64, |a, x| a.wrapping_add(*x)));
        });
        // Bytes stay bit-identical at scale.
        let serial: Vec<u64> = (0..LEN).map(to_word).collect();
        let parallel = fork_join_on(
            &pool,
            0..LEN,
            ChunkSize::new(4096),
            |sub| sub.map(to_word).collect::<Vec<u64>>(),
            Vec::new(),
            |mut acc: Vec<u64>, mut part| {
                acc.append(&mut part);
                acc
            },
        )
        .expect("scaled bake runs");
        assert_eq!(parallel, serial, "scale must not move bytes");
        // The slot-write collect over the same workload: same bytes, and it
        // must clear 2x too or the collect is killed in favour of fork_join_on.
        let slots = collect_indexed_on(&pool, 0..LEN, ChunkSize::new(4096), to_word)
            .expect("scaled collect runs");
        assert_eq!(slots, serial, "slot collect must not move bytes");
        let collect_ms = best_ms(|| {
            let out = collect_indexed_on(&pool, 0..LEN, ChunkSize::new(4096), to_word)
                .expect("scaled collect runs");
            std::hint::black_box(out.iter().fold(0u64, |a, x| a.wrapping_add(*x)));
        });
        let ratio = serial_ms / parallel_ms;
        let collect_ratio = serial_ms / collect_ms;
        println!(
            "scaled bake {LEN} items: serial {serial_ms:.1}ms fork_join {parallel_ms:.1}ms ratio {ratio:.2} slot_collect {collect_ms:.1}ms ratio {collect_ratio:.2}"
        );
        assert!(
            ratio >= 2.0,
            "parallel bake {parallel_ms:.1}ms must clear 2x over serial {serial_ms:.1}ms or the lane is killed"
        );
        assert!(
            collect_ratio >= 2.0,
            "slot collect {collect_ms:.1}ms must clear 2x over serial {serial_ms:.1}ms or it is killed"
        );
    }
}
