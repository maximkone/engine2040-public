//! PGO harness: before/after wall ms for the frame-slice path.
//! Status: library-only, test-only in production (no binary flag implements
//! `--pgo`; `crates/perf/src/main.rs` handles `--alloc-compare`,
//! `--vram-plan`, `--heap-peak`, `--capture` and `--mixed-skip` only).
//! Units: frames, words, hash, bytes, count, ms. Determinism: hash and
//! alloc totals pin; wall ms is presentation-only beside them, never hashed.
//! Errors: typed [`PgoError`]. Budget: the arena path allocates nothing
//! steady-state; the heap path allocates per frame (negative control).
//! Caller: tests call `csv_row`/`compare_ms`/`run_report` directly; no
//! production binary caller prints the CSV row below.

use crate::alloc;
use engine_core::arena::{ArenaError, SliceArena};
use engine_core::{Fnv64, Rng};
use std::fmt::{self, Display, Formatter};

/// Cull sort words staged per frame (render stand-in: packed depth+index).
pub const WORDS_CULL: usize = 1024;
/// Pose words staged per frame (anim stand-in: packed fixed-point pairs).
pub const WORDS_POSE: usize = 512;
/// Mix words staged per frame (audio stand-in: packed sample pairs).
pub const WORDS_MIX: usize = 256;
/// Presized words for one frame: every shape at full size, no headroom
/// needed because typed words carry no alignment padding.
pub const WORDS_PER_FRAME: usize = WORDS_CULL + WORDS_POSE + WORDS_MIX;

/// Acceptance bar for a profile-guided swap: the pgo sample must sit at
/// least this far below the baseline at an identical hash/alloc row.
pub const SPEEDUP_PCT: f64 = 5.0;

/// CSV columns shared by baseline and pgo rows, so the two compare field
/// for field. `delta_pct` is `(baseline - pgo) / baseline * 100`.
pub const CSV_HEADER: &str =
    "workload,frames,hash,alloc_bytes,alloc_count,baseline_ms,pgo_ms,delta_pct";

/// Frame-slice workload over the core slice arena: fixed shapes, seeded
/// contents, byte-identical replays folded with [`Fnv64`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PgoConfig {
    /// Stream seed for the frame contents.
    pub seed: u64,
    /// Frames per run.
    pub frames: u32,
}

impl PgoConfig {
    /// Workload name for reports and budget errors.
    pub const NAME: &'static str = "frame_slice_pgo";

    /// Canonical workload: seed 7, 120 frames of three-owner slice scratch.
    #[must_use]
    pub const fn canonical() -> Self {
        Self {
            seed: 7,
            frames: 120,
        }
    }

    /// Same shape with a different seed (regression proofs).
    #[must_use]
    pub const fn with_seed(self, seed: u64) -> Self {
        Self { seed, ..self }
    }
}

/// One measured canonical run: hash plus the thread-local allocations made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PgoReport {
    /// Workload name, matching [`PgoConfig::NAME`].
    pub workload: &'static str,
    /// Frames simulated.
    pub frames: u32,
    /// Canonical hash of the run.
    pub hash: u64,
    /// Bytes allocated on the run thread.
    pub alloc_bytes: u64,
    /// Allocation count on the run thread.
    pub alloc_count: u64,
}

/// PGO comparison failure: which side refused, with the value carried.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PgoError {
    /// A wall sample is NaN or infinite (`side` is `"baseline"`, `"pgo"`
    /// or `"row"`).
    NonFiniteMs {
        /// Which side carries the sample.
        side: &'static str,
        /// Offending milliseconds.
        value_ms: f64,
    },
    /// The baseline sample is zero or negative, so no speedup ratio exists.
    NonPositiveBaseline {
        /// Offending baseline milliseconds.
        value_ms: f64,
    },
    /// A pgo sample is negative, so no honest comparison exists.
    NegativePgo {
        /// Offending pgo milliseconds.
        value_ms: f64,
    },
}

impl Display for PgoError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NonFiniteMs { side, value_ms } => {
                write!(f, "pgo: {side} sample is not finite ({value_ms} ms)")
            }
            Self::NonPositiveBaseline { value_ms } => write!(
                f,
                "pgo: baseline sample is not positive ({value_ms} ms): no speedup ratio exists"
            ),
            Self::NegativePgo { value_ms } => {
                write!(f, "pgo: pgo sample is negative ({value_ms} ms)")
            }
        }
    }
}

impl std::error::Error for PgoError {}

/// Wall-clock comparison of one workload: baseline ms against pgo ms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PgoDelta {
    /// Baseline wall ms for the workload (heap path).
    pub baseline_ms: f64,
    /// Pgo wall ms for the same workload (arena-slice path).
    pub pgo_ms: f64,
    /// Speedup in percent: `(baseline - pgo) / baseline * 100`.
    pub delta_pct: f64,
}

impl PgoDelta {
    /// Acceptance verdict: the pgo row clears the bar only when it is at
    /// least [`SPEEDUP_PCT`] faster. A slower row reads negative and fails;
    /// hash/alloc identity is checked separately by the report gate.
    #[must_use]
    pub fn passes(self) -> bool {
        self.delta_pct >= SPEEDUP_PCT
    }
}

/// Compare one workload's wall samples: baseline ms against pgo ms. Pure
/// over its inputs; run-to-run movement lives in the sampling, never in
/// hidden state.
///
/// # Errors
///
/// [`PgoError::NonFiniteMs`] on a NaN or infinite sample,
/// [`PgoError::NonPositiveBaseline`] on a zero or negative baseline,
/// [`PgoError::NegativePgo`] on a negative pgo sample.
pub fn compare_ms(baseline_ms: f64, pgo_ms: f64) -> Result<PgoDelta, PgoError> {
    if !baseline_ms.is_finite() {
        return Err(PgoError::NonFiniteMs {
            side: "baseline",
            value_ms: baseline_ms,
        });
    }
    if !pgo_ms.is_finite() {
        return Err(PgoError::NonFiniteMs {
            side: "pgo",
            value_ms: pgo_ms,
        });
    }
    if baseline_ms <= 0.0 {
        return Err(PgoError::NonPositiveBaseline {
            value_ms: baseline_ms,
        });
    }
    if pgo_ms < 0.0 {
        return Err(PgoError::NegativePgo { value_ms: pgo_ms });
    }
    Ok(PgoDelta {
        baseline_ms,
        pgo_ms,
        delta_pct: (baseline_ms - pgo_ms) / baseline_ms * 100.0,
    })
}

/// Run one frame on `arena`: reset, stream the three slice shapes, sort the
/// cull keys in stack scratch (the render-style transient compute), stage
/// every shape into the arena and fold the staged slices back into a frame
/// hash. The arena must already fit [`WORDS_PER_FRAME`]; setup owns the
/// presize, frames never grow.
///
/// # Errors
///
/// [`ArenaError::Overflow`] when the arena was not presized.
pub fn run_frame(arena: &mut SliceArena, seed: u64, frame: u32) -> Result<u64, ArenaError> {
    arena.reset();
    let mut stream = Rng::new(seed).fork(u64::from(frame));
    let mut keys = [0u32; WORDS_CULL];
    for key in keys.iter_mut() {
        *key = stream.next_u32();
    }
    let mut pose = [0u32; WORDS_POSE];
    for word in pose.iter_mut() {
        *word = stream.next_u32();
    }
    let mut mix = [0u32; WORDS_MIX];
    for word in mix.iter_mut() {
        *word = stream.next_u32();
    }
    keys.sort_unstable();
    let mut fold = Fnv64::new();
    fold.write_u32(frame);
    let staged = arena.push_slice(&keys)?;
    for word in staged.iter() {
        fold.write_u32(*word);
    }
    let staged = arena.push_slice(&pose)?;
    for word in staged.iter() {
        fold.write_u32(*word);
    }
    let staged = arena.push_slice(&mix)?;
    for word in staged.iter() {
        fold.write_u32(*word);
    }
    Ok(fold.finish())
}

/// Deterministic arena-slice hash for `config`: a warmed, presized arena
/// runs every frame with no heap use. The pgo (profile-guided, presized)
/// side of the before/after pair.
///
/// # Errors
///
/// [`ArenaError::Overflow`] when the shapes stop fitting the presize (a
/// shape change without a presize bump: fix the constant).
pub fn arena_hash(config: &PgoConfig) -> Result<u64, ArenaError> {
    let mut arena = SliceArena::with_capacity_words(WORDS_PER_FRAME);
    run_frames(&mut arena, config.seed, config.frames)
}

/// Fold every frame of the workload on an already-presized `arena`.
///
/// Split from [`arena_hash`] so the alloc report can presize at setup and
/// measure the steady-state frames only. Same streams, same fold, same
/// hash as [`arena_hash`].
///
/// # Errors
///
/// [`ArenaError::Overflow`] when the arena was not presized.
pub fn run_frames(arena: &mut SliceArena, seed: u64, frames: u32) -> Result<u64, ArenaError> {
    let mut fold = Fnv64::new();
    for frame in 0..frames {
        fold.write_u64(run_frame(arena, seed, frame)?);
    }
    Ok(fold.finish())
}

/// Deterministic heap hash for `config`: the same streams and the same fold
/// as [`arena_hash`], but each frame stages heap `Vec`s instead of arena
/// slices. Must equal [`arena_hash`]: the slice path is faithful, only the
/// allocation behavior differs (the baseline side of the pair).
#[must_use]
pub fn heap_hash(config: &PgoConfig) -> u64 {
    let mut fold = Fnv64::new();
    for frame in 0..config.frames {
        let mut stream = Rng::new(config.seed).fork(u64::from(frame));
        let mut keys = Vec::with_capacity(WORDS_CULL);
        for _ in 0..WORDS_CULL {
            keys.push(stream.next_u32());
        }
        // Baseline path stages one heap Vec per shape on purpose, so the
        // alloc gate observes the cost the arena removes. No waiver token
        // is needed: heap_hash is not a hot function, so nothing is excused.
        let mut pose = Vec::with_capacity(WORDS_POSE);
        for _ in 0..WORDS_POSE {
            pose.push(stream.next_u32());
        }
        // Third baseline shape, same reason as above.
        let mut mix = Vec::with_capacity(WORDS_MIX);
        for _ in 0..WORDS_MIX {
            mix.push(stream.next_u32());
        }
        keys.sort_unstable();
        let mut frame_fold = Fnv64::new();
        frame_fold.write_u32(frame);
        for word in keys.iter().chain(pose.iter()).chain(mix.iter()) {
            frame_fold.write_u32(*word);
        }
        fold.write_u64(frame_fold.finish());
    }
    fold.finish()
}

/// Measured arena-slice report for `config`: the canonical hash plus the
/// thread-local allocations made (zero on pass). The pgo row of the CSV.
///
/// # Errors
///
/// [`ArenaError::Overflow`] when the shapes stop fitting the presize.
pub fn run_report(config: &PgoConfig) -> Result<PgoReport, ArenaError> {
    // Setup presizes outside the window: the report pins the steady-state
    // frames, which must allocate nothing.
    let mut arena = SliceArena::with_capacity_words(WORDS_PER_FRAME);
    let (hash, observed) = alloc::measure(|| run_frames(&mut arena, config.seed, config.frames));
    let hash = hash?;
    Ok(PgoReport {
        workload: PgoConfig::NAME,
        frames: config.frames,
        hash,
        alloc_bytes: observed.bytes,
        alloc_count: observed.count,
    })
}

/// A/B wall timing of the two frame paths in milliseconds (wall-clock
/// evidence only; pass or fail never depends on it). The heap path runs
/// first as the baseline, the arena-slice path second as pgo; both run
/// `iters` full canonical runs with the optimizer sink engaged.
///
/// # Errors
///
/// [`ArenaError::Overflow`] when the arena presize stops fitting (refuses
/// the timed pgo side instead of growing mid-frame).
pub fn time_both(config: &PgoConfig, iters: u32) -> Result<(f64, f64), ArenaError> {
    let start = std::time::Instant::now();
    for _ in 0..iters {
        std::hint::black_box(heap_hash(config));
    }
    let baseline_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = std::time::Instant::now();
    for _ in 0..iters {
        std::hint::black_box(arena_hash(config)?);
    }
    let pgo_ms = start.elapsed().as_secs_f64() * 1000.0;
    Ok((baseline_ms, pgo_ms))
}

/// One CSV row in the shared harness shape: the pinned hash/alloc columns
/// plus the wall pair and their delta. A future instrumented-PGO build
/// plugs its samples into this same function, so columns cannot drift.
///
/// Wall ms joins a baseline file only from a quiesced box; every other
/// timed row stays observed-only.
///
/// # Errors
///
/// [`PgoError::NonFiniteMs`] (`side: "row"`) on a NaN or infinite sample,
/// [`PgoError::NegativePgo`] on a negative one,
/// [`PgoError::NonPositiveBaseline`] on a zero or negative baseline.
pub fn csv_row(report: &PgoReport, baseline_ms: f64, pgo_ms: f64) -> Result<String, PgoError> {
    let delta = compare_ms(baseline_ms, pgo_ms)?;
    for (side, value) in [("row-baseline", baseline_ms), ("row-pgo", pgo_ms)] {
        if !value.is_finite() {
            return Err(PgoError::NonFiniteMs {
                side: "row",
                value_ms: value,
            });
        }
        if value < 0.0 {
            return Err(PgoError::NegativePgo { value_ms: value });
        }
        let _ = side;
    }
    Ok(format!(
        "{},{},{:016x},{},{},{:.3},{:.3},{:.2}",
        report.workload,
        report.frames,
        report.hash,
        report.alloc_bytes,
        report.alloc_count,
        baseline_ms,
        pgo_ms,
        delta.delta_pct,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arena_and_heap_agree_and_repeat() {
        let config = PgoConfig::canonical();
        let arena = arena_hash(&config).expect("presized scratch fits");
        let heap = heap_hash(&config);
        assert_eq!(arena, heap, "slice path folds what the heap path folds");
        assert_eq!(
            arena_hash(&config).expect("repeat fits"),
            arena,
            "arena path repeats"
        );
        assert_eq!(heap_hash(&config), heap, "heap path repeats");
        assert_ne!(arena, 0, "canonical run folds real frame words");
    }

    #[test]
    fn seed_moves_the_hash() {
        let config = PgoConfig::canonical();
        let moved = PgoConfig::canonical().with_seed(1_000_007);
        let base = arena_hash(&config).expect("canonical fits");
        let other = arena_hash(&moved).expect("seeded fits");
        assert_ne!(base, other, "seed moves the folded frame hashes");
        assert_ne!(
            heap_hash(&config),
            heap_hash(&moved),
            "seed moves the heap fold too"
        );
    }

    #[test]
    fn steady_arena_allocates_nothing_heap_allocates() {
        let ((), probe) = alloc::measure(|| {
            std::hint::black_box(vec![0u8; 64]);
        });
        assert!(probe.count >= 1, "alloc probe is blind");
        let config = PgoConfig::canonical();
        let first = run_report(&config).expect("presized fits");
        let second = run_report(&config).expect("repeat fits");
        assert_eq!(first, second, "steady reports repeat exactly");
        assert_eq!(first.alloc_bytes, 0, "arena path allocates no bytes");
        assert_eq!(first.alloc_count, 0, "arena path makes no allocations");
        let (heap, observed) = alloc::measure(|| heap_hash(&config));
        assert_eq!(heap, first.hash, "heap path hashes identically");
        assert!(
            observed.count > 0,
            "heap baseline really allocates: {observed:?}"
        );
    }

    #[test]
    fn empty_arena_refuses_the_frame_loudly() {
        let mut arena = SliceArena::new();
        let error = run_frame(&mut arena, PgoConfig::canonical().seed, 0)
            .expect_err("unpresized arena refuses the frame");
        assert!(
            matches!(error, ArenaError::Overflow { .. }),
            "refusal names overflow: {error}"
        );
        assert_eq!(arena.used_words(), 0, "refused frame moves no cursor");
        assert!(!error.to_string().is_empty(), "{error:?} explains itself");
        let _: &dyn std::error::Error = &error;
    }

    #[test]
    fn five_percent_faster_passes_the_bar() {
        let delta = compare_ms(100.0, 95.0).expect("clean samples compare");
        assert_eq!(delta.delta_pct, 5.0);
        assert!(delta.passes(), "exactly 5% clears the pgo bar");
        let faster = compare_ms(100.0, 80.0).expect("clean samples compare");
        assert!(faster.passes(), "20% clears the bar");
    }

    #[test]
    fn slower_or_narrow_gains_fail_the_bar() {
        let narrow = compare_ms(100.0, 95.001).expect("clean samples compare");
        assert!(
            narrow.delta_pct < SPEEDUP_PCT,
            "4.999% really sits under the bar"
        );
        assert!(!narrow.passes(), "a narrow gain keeps the baseline row");
        let slower = compare_ms(100.0, 110.0).expect("clean samples compare");
        assert_eq!(slower.delta_pct, -10.0);
        assert!(!slower.passes(), "a slower pgo fails");
    }

    #[test]
    fn bad_samples_refuse_loudly() {
        assert!(
            matches!(
                compare_ms(f64::NAN, 1.0),
                Err(PgoError::NonFiniteMs {
                    side: "baseline",
                    ..
                })
            ),
            "NaN baseline names its side"
        );
        assert_eq!(
            compare_ms(1.0, f64::INFINITY),
            Err(PgoError::NonFiniteMs {
                side: "pgo",
                value_ms: f64::INFINITY,
            })
        );
        assert_eq!(
            compare_ms(0.0, 0.0),
            Err(PgoError::NonPositiveBaseline { value_ms: 0.0 })
        );
        assert_eq!(
            compare_ms(5.0, -1.0),
            Err(PgoError::NegativePgo { value_ms: -1.0 })
        );
        for error in [
            PgoError::NonFiniteMs {
                side: "row",
                value_ms: f64::NAN,
            },
            PgoError::NonPositiveBaseline { value_ms: 0.0 },
            PgoError::NegativePgo { value_ms: -1.0 },
        ] {
            assert!(!error.to_string().is_empty(), "{error:?} explains itself");
            let _: &dyn std::error::Error = &error;
        }
    }

    #[test]
    fn csv_row_keeps_eight_columns() {
        let report = PgoReport {
            workload: PgoConfig::NAME,
            frames: 120,
            hash: 0x1234,
            alloc_bytes: 0,
            alloc_count: 0,
        };
        assert!(CSV_HEADER.split(',').count() == 8, "{}", CSV_HEADER);
        let row = csv_row(&report, 10.0, 9.0).expect("row prints");
        assert_eq!(row.split(',').count(), 8, "columns never drift: {row}");
        assert!(row.starts_with("frame_slice_pgo,120,"), "{row}");
        assert!(row.ends_with(",10.000,9.000,10.00"), "{row}");
        assert!(
            matches!(
                csv_row(&report, f64::NAN, 9.0),
                Err(PgoError::NonFiniteMs { .. })
            ),
            "NaN baseline refuses"
        );
        assert_eq!(
            csv_row(&report, 10.0, -2.0),
            Err(PgoError::NegativePgo { value_ms: -2.0 })
        );
    }

    #[test]
    fn timed_pair_reports_two_finite_ms() {
        let config = PgoConfig::canonical();
        let (baseline_ms, pgo_ms) = time_both(&config, 2).expect("presized fits");
        assert!(
            baseline_ms.is_finite() && baseline_ms >= 0.0,
            "baseline sample is honest: {baseline_ms}"
        );
        assert!(
            pgo_ms.is_finite() && pgo_ms >= 0.0,
            "pgo sample is honest: {pgo_ms}"
        );
        eprintln!("pgo_timing baseline_ms={baseline_ms:.3} pgo_ms={pgo_ms:.3}");
        let row = PgoReport {
            workload: PgoConfig::NAME,
            frames: config.frames,
            hash: arena_hash(&config).expect("timed hash repeats"),
            alloc_bytes: 0,
            alloc_count: 0,
        };
        let text = csv_row(&row, baseline_ms, pgo_ms).expect("timed row prints");
        assert_eq!(text.split(',').count(), 8, "{text}");
    }
}
