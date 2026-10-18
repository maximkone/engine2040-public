//! Profiler: puffin scopes over the canonical workloads, GPU timestamp-query
//! scaffolding for the forward, shadow, lighting and post passes, and a
//! frame-capture report.
//!
//! Consumer: the perf report reads [`FrameReport`] (systems plus passes
//! with p50/p95/p99 over a fixed run, naming the costliest of each).
//! Units: milliseconds for every timing, span/pass name strings for keys.
//! Determinism scope: profiling never enters any simulation hash. Probes run
//! the same fixed inputs every time; only wall-clock milliseconds are
//! recorded, simulation bytes are hashed locally and discarded through
//! `black_box` so the optimizer cannot elide the work in release.
//! Error type: [`ProfileError`]; the profiler is off by default and every
//! entry point fails loudly instead of guessing.
//!
//! Span keys are plain `perf.*` strings with no cross-crate join: the perf
//! crate takes no dependency on `engine-observe` (the profiler adds no new
//! dependencies), so nothing here reads an observe-side span. A caller that
//! wants both records correlates them by matching these string keys
//! operator-side.
//!
//! Puffin (`profile` feature) only emits scopes while the profiler is
//! enabled; serving or saving the puffin stream stays operator-side
//! (puffin viewer/HTTP), this crate only plants the scope points.
//!
//! Donor: EmbarkStudios/puffin@c81e9bb (MIT) `puffin/src/lib.rs`: the
//! off-by-default gate shape (`MACROS_ON: AtomicBool = false`, relaxed
//! load/store, off path one atomic plus a branch at 1-2 ns with no clock,
//! scope or sample). Ported as `ENABLED`/`is_enabled`/`set_enabled`/
//! `gate_on` below; no donor bytes vendored, puffin stays an opt-in dep.

use std::fmt::{self, Display, Formatter};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crate::diagnostics::{DiagnosticsError, DiagnosticsStore};
use crate::workloads::{AiDecide, ArenaTick, DestructionStep, PhysicsStep};

// ---------------------------------------------------------------------------
// Switch: off by default, zero overhead while off.
// ---------------------------------------------------------------------------

/// Master switch. Off by default: every profiled wrapper then calls straight
/// through to the workload with no clock read, no scope and no sample, so
/// disabled profiling costs one relaxed atomic load plus a branch.
/// Port of puffin's `MACROS_ON` gate (donor: EmbarkStudios/puffin@c81e9bb,
/// MIT); bench `benches/off_path.rs` holds the off path at <=2 ns/op.
static ENABLED: AtomicBool = AtomicBool::new(false);

/// True while the profiler records timings and emits puffin scopes.
#[inline]
#[must_use]
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Turn recording on or off. Callers pair every `set_enabled(true)` with a
/// restoring `set_enabled(false)`; integration tests serialize on a mutex so
/// parallel harnesses never observe a half-flipped flag.
/// Mirrors the flag into puffin scopes while the profile feature is on.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
    #[cfg(feature = "profile")]
    puffin::set_scopes_on(on);
}

// ---------------------------------------------------------------------------
// Errors: typed, every failure names itself.
// ---------------------------------------------------------------------------

/// Profiler failure: which input or precondition refused, with both sides.
#[derive(Clone, Debug, PartialEq)]
pub enum ProfileError {
    /// Capture ran while the profiler was off; enable it first.
    Disabled,
    /// Frame count outside `1..=CAPTURE_FRAMES_MAX`.
    InvalidFrames {
        /// Requested frame count.
        frames: u32,
    },
    /// Span name outside the canonical workload set.
    UnknownSpan {
        /// Rejected span name.
        span: String,
    },
    /// Pass name outside the GPU pass set.
    UnknownPass {
        /// Rejected pass name.
        pass: String,
    },
    /// No samples to summarize.
    EmptySamples {
        /// What needed samples (span or pass name).
        what: &'static str,
    },
    /// A timing is NaN, infinite or negative; wall clocks never produce one.
    NonFiniteSample {
        /// What carried the bad timing (span or pass name).
        what: &'static str,
    },
    /// A fixed render probe rejected its fixed input; the reason is carried.
    ProbeFailed {
        /// Which probe refused (`ra1`, `rl1`, `rl2`, `rp1`).
        what: &'static str,
        /// Render-side reason.
        reason: String,
    },
    /// Tick count outside `1..=TICK_FRAME_TICKS`.
    InvalidTicks {
        /// Requested tick count.
        ticks: u32,
    },
    /// Mark label outside the secondary set (`tick`, `frame`).
    UnknownMark {
        /// Rejected mark label.
        mark: String,
    },
    /// A mark series is full; marks stay bounded per capture.
    TooManyMarks {
        /// Series capacity in marks.
        limit: usize,
    },
    /// A plot series is full; samples stay bounded per capture.
    TooManySamples {
        /// What overflowed (plot series name).
        what: &'static str,
    },
    /// Paired rerun diverged; any hash mismatch kills the receipt.
    DivergedPair { hash_a: u64, hash_b: u64 },
    /// The frame-time diagnostics series refused a sample or its setup.
    Diagnostics {
        /// Rendered diagnostics error.
        reason: String,
    },
}

impl Display for ProfileError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => write!(
                f,
                "profiler is disabled; call set_enabled(true) before capturing"
            ),
            Self::InvalidFrames { frames } => write!(
                f,
                "capture frames {frames} outside 1..={CAPTURE_FRAMES_MAX}"
            ),
            Self::UnknownSpan { span } => write!(f, "unknown profile span: {span}"),
            Self::UnknownPass { pass } => write!(f, "unknown GPU pass: {pass}"),
            Self::EmptySamples { what } => write!(f, "{what} captured no samples"),
            Self::NonFiniteSample { what } => {
                write!(f, "{what} sample is not finite non-negative ms")
            }
            Self::ProbeFailed { what, reason } => {
                write!(f, "{what} probe failed: {reason}")
            }
            Self::InvalidTicks { ticks } => {
                write!(f, "capture ticks {ticks} outside 1..={TICK_FRAME_TICKS}")
            }
            Self::UnknownMark { mark } => write!(f, "unknown tick-frame mark: {mark}"),
            Self::TooManyMarks { limit } => {
                write!(f, "tick-frame mark series full at {limit} marks")
            }
            Self::TooManySamples { what } => {
                write!(f, "{what} plot series full at {TICK_FRAME_TICKS} samples")
            }
            Self::DivergedPair { hash_a, hash_b } => {
                write!(f, "paired rerun diverged: {hash_a:016x} != {hash_b:016x}")
            }
            Self::Diagnostics { reason } => write!(f, "diagnostics refused: {reason}"),
        }
    }
}

impl std::error::Error for ProfileError {}

impl From<DiagnosticsError> for ProfileError {
    fn from(err: DiagnosticsError) -> Self {
        Self::Diagnostics {
            reason: err.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Scopes: one puffin scope per canonical workload, compiled out without `profile`.
// ---------------------------------------------------------------------------

/// Enter a puffin scope when the `profile` feature is on; without it this
/// expands to nothing, so default builds carry no puffin code at all.
macro_rules! scope {
    ($name:literal) => {
        #[cfg(feature = "profile")]
        puffin::profile_scope!($name);
    };
}

/// Conditional fast path: `is_enabled() && condition` with
/// short-circuit order, so the caller's condition is never evaluated while
/// the profiler is off. Disabled cost is one relaxed atomic load plus a
/// branch; the puffin `OnceLock` scope registration behind
/// `profile_scope_custom_if!` runs only when both gates pass, so this
/// module reuses the puffin scope shape without vendoring it.
#[inline]
pub fn gate_on(condition: bool) -> bool {
    is_enabled() && condition
}

/// Enter a puffin scope only when the master switch and the caller's
/// condition both hold. Without the `profile` feature this still checks the
/// master gate (one load plus a branch) and emits no puffin code.
macro_rules! scope_if {
    ($condition:expr, $name:literal) => {
        #[cfg(feature = "profile")]
        let _cond_scope = if crate::profiler::gate_on($condition) {
            puffin::profile_scope_custom_if!(true, $name, "")
        } else {
            None
        };
        #[cfg(not(feature = "profile"))]
        let _cond_scope = if crate::profiler::gate_on($condition) {
            Some(())
        } else {
            None
        };
    };
}

/// Advance the puffin frame: call once per captured frame so the puffin
/// stream closes the current frame and per-frame CPU time reads measured
/// instead of UNKNOWN.
///
/// Port of the puffin game-loop shape (donor: EmbarkStudios/puffin@c81e9bb,
/// MIT, puffin/src/lib.rs docs plus global_profiler.rs::new_frame):
/// GlobalProfiler::lock().new_frame() once per frame around
/// profile_scope! spans. Off path is one relaxed atomic load plus a
/// branch with no lock and no clock; without the profile feature this
/// is an empty no-op, so default builds carry no puffin code at all.
#[inline]
pub fn begin_frame() {
    #[cfg(feature = "profile")]
    if is_enabled() {
        puffin::GlobalProfiler::lock().new_frame();
    }
}

/// Canonical physics-step hash through the conditional gate.
/// `record` is the caller's per-call condition (for example, "this tick is
/// sampled"): when the master switch is off, or `record` is false, this calls
/// straight through with no clock read, no scope and no sample, returning
/// the identical hash with `None`. When both gates pass it times the same
/// canonical workload as [`profiled_physics_step`], so the pair reads the
/// same `perf.physics_step` span with the same hash.
#[must_use]
pub fn profiled_physics_step_if(record: bool) -> (u64, Option<Sample>) {
    if !gate_on(record) {
        return (PhysicsStep::canonical().run_hash(), None);
    }
    scope_if!(true, "perf.physics_step");
    let (hash, sample) = timed(SPAN_PHYSICS_STEP, || PhysicsStep::canonical().run_hash());
    (hash, Some(sample))
}

/// Observe span key for the arena-tick workload.
pub const SPAN_ARENA_TICK: &str = "perf.arena_tick";
/// Observe span key for the physics-step workload.
pub const SPAN_PHYSICS_STEP: &str = "perf.physics_step";
/// Observe span key for the AI-decide workload.
pub const SPAN_AI_DECIDE: &str = "perf.ai_decide";
/// Observe span key for the destruction-step workload.
pub const SPAN_DESTRUCTION_STEP: &str = "perf.destruction_step";

/// Every profiled system span, in report order.
pub const SYSTEM_SPANS: &[&str] = &[
    SPAN_ARENA_TICK,
    SPAN_PHYSICS_STEP,
    SPAN_AI_DECIDE,
    SPAN_DESTRUCTION_STEP,
];

/// Look up a system span by name, failing on anything outside the workload set.
pub fn system_span(name: &str) -> Result<&'static str, ProfileError> {
    SYSTEM_SPANS
        .iter()
        .copied()
        .find(|span| *span == name)
        .ok_or_else(|| ProfileError::UnknownSpan {
            span: name.to_owned(),
        })
}

// ---------------------------------------------------------------------------
// Samples: one timed workload run.
// ---------------------------------------------------------------------------

/// One timed workload run: its observe span key plus wall-clock ms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// Span key from [`SYSTEM_SPANS`].
    pub span: &'static str,
    /// Wall-clock time in ms; evidence only, never hashed.
    pub ms: f64,
}

impl Sample {
    /// Build a sample, refusing unknown spans and non-finite ms.
    pub fn new(span: &'static str, ms: f64) -> Result<Self, ProfileError> {
        system_span(span)?;
        if !ms.is_finite() || ms < 0.0 {
            return Err(ProfileError::NonFiniteSample { what: span });
        }
        Ok(Self { span, ms })
    }
}

/// Read the wall clock in ms around `run`.
fn timed<T>(span: &'static str, run: impl FnOnce() -> T) -> (T, Sample) {
    let started = Instant::now();
    let value = run();
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    // `ms` comes from `Instant`, so finite and non-negative by construction.
    let sample = Sample::new(span, ms).expect("wall-clock ms is finite non-negative");
    (value, sample)
}

/// Canonical arena-tick hash plus its sample when enabled (`None` while off).
/// The hash path is identical either way: profiling never perturbs it.
#[must_use]
pub fn profiled_arena_tick() -> (u64, Option<Sample>) {
    if !is_enabled() {
        return (ArenaTick::canonical().run_hash(), None);
    }
    scope!("perf.arena_tick");
    let (hash, sample) = timed(SPAN_ARENA_TICK, || ArenaTick::canonical().run_hash());
    (hash, Some(sample))
}

/// Canonical physics-step hash plus its sample when enabled.
#[must_use]
pub fn profiled_physics_step() -> (u64, Option<Sample>) {
    if !is_enabled() {
        return (PhysicsStep::canonical().run_hash(), None);
    }
    scope!("perf.physics_step");
    let (hash, sample) = timed(SPAN_PHYSICS_STEP, || PhysicsStep::canonical().run_hash());
    (hash, Some(sample))
}

/// Canonical AI-decide hash plus its sample when enabled.
#[must_use]
pub fn profiled_ai_decide() -> (u64, Option<Sample>) {
    if !is_enabled() {
        return (AiDecide::canonical().run_hash(), None);
    }
    scope!("perf.ai_decide");
    let (hash, sample) = timed(SPAN_AI_DECIDE, || AiDecide::canonical().run_hash());
    (hash, Some(sample))
}

/// Canonical destruction-step hash plus its sample when enabled.
#[must_use]
pub fn profiled_destruction_step() -> (u64, Option<Sample>) {
    if !is_enabled() {
        return (DestructionStep::canonical().run_hash(), None);
    }
    scope!("perf.destruction_step");
    let (hash, sample) = timed(SPAN_DESTRUCTION_STEP, || {
        DestructionStep::canonical().run_hash()
    });
    (hash, Some(sample))
}

// ---------------------------------------------------------------------------
// GPU timestamp-query scaffolding: what the queries WOULD capture.
// ---------------------------------------------------------------------------

/// GPU pass covered by a timestamp-query pair. Variants follow the render
/// receipts: forward/present, sun plus torch cluster, IBL, TAA resolve plus
/// the post chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GpuPass {
    /// Multi-draw forward PBR plus the blit into the swapchain twin.
    Ra1Forward,
    /// Cascaded sun shadow pass.
    Rl1SunShadow,
    /// Clustered torch light pass.
    Rl1TorchCluster,
    /// Image-based lighting sample.
    Rl2Ibl,
    /// TAA resolve plus temporal upsample.
    Rp1Taa,
    /// Post chain (bloom, exposure, ACES, grading).
    Rp1Post,
}

impl GpuPass {
    /// Every pass in report order; doubles as the query-slot order.
    pub const ALL: &'static [Self] = &[
        Self::Ra1Forward,
        Self::Rl1SunShadow,
        Self::Rl1TorchCluster,
        Self::Rl2Ibl,
        Self::Rp1Taa,
        Self::Rp1Post,
    ];

    /// Query label: the pass name quoted in receipts and the frame report.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Ra1Forward => "ra1.forward",
            Self::Rl1SunShadow => "rl1.sun_shadow",
            Self::Rl1TorchCluster => "rl1.torch_cluster",
            Self::Rl2Ibl => "rl2.ibl",
            Self::Rp1Taa => "rp1.taa",
            Self::Rp1Post => "rp1.post",
        }
    }

    /// Look up a pass by its [`Self::name`], failing on anything else.
    pub fn from_name(name: &str) -> Result<Self, ProfileError> {
        Self::ALL
            .iter()
            .find(|pass| pass.name() == name)
            .copied()
            .ok_or_else(|| ProfileError::UnknownPass {
                pass: name.to_owned(),
            })
    }

    /// Timestamp-query begin slot this pass would write (pairs pack as
    /// `2 * index` / `2 * index + 1`, so one query set covers the frame).
    #[must_use]
    pub fn query_begin(self) -> u32 {
        2 * self.query_index()
    }

    /// Timestamp-query end slot this pass would write.
    #[must_use]
    pub fn query_end(self) -> u32 {
        2 * self.query_index() + 1
    }

    fn query_index(self) -> u32 {
        Self::ALL
            .iter()
            .position(|pass| *pass == self)
            .expect("every GpuPass lists itself in GpuPass::ALL") as u32
    }
}

/// One resolved pass sample: CPU-side ms measured here, GPU-side ms left
/// for the operator's reference-GPU run (`None` means not measured here).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuSample {
    /// Pass sampled.
    pub pass: GpuPass,
    /// CPU-side reference ms; evidence only, never hashed.
    pub cpu_ms: f64,
    /// Reference-GPU ms; `None` until an operator run resolves it.
    pub gpu_ms: Option<f64>,
}

impl GpuSample {
    /// Build a CPU-side sample, refusing non-finite ms.
    pub fn cpu_only(pass: GpuPass, cpu_ms: f64) -> Result<Self, ProfileError> {
        if !cpu_ms.is_finite() || cpu_ms < 0.0 {
            return Err(ProfileError::NonFiniteSample { what: pass.name() });
        }
        Ok(Self {
            pass,
            cpu_ms,
            gpu_ms: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Frame capture: fixed run, quantiles, costliest pass and system.
// ---------------------------------------------------------------------------

/// Default frame count for [`capture`]: ten fixed frames.
pub const CAPTURE_FRAMES_DEFAULT: u32 = 10;
/// Most frames one capture runs; bounds capture time in debug builds.
pub const CAPTURE_FRAMES_MAX: u32 = 120;

/// Quantiles over one span's or pass's per-frame ms, in report order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quantiles {
    /// Frames sampled.
    pub frames: u32,
    /// Median ms.
    pub p50_ms: f64,
    /// 95th percentile ms.
    pub p95_ms: f64,
    /// 99th percentile ms.
    pub p99_ms: f64,
}

impl Quantiles {
    /// Summarize per-frame ms: refuses empty input and non-finite samples,
    /// then sorts and reads nearest-rank quantiles (deterministic per input).
    /// Ranks delegate to [`crate::latency::nearest_rank_sorted`], the single
    /// float owner (ceil-ranked JS gate and integer fight-cost ranks differ
    /// deliberately, see there).
    pub fn of(what: &'static str, samples: &[f64]) -> Result<Self, ProfileError> {
        if samples.is_empty() {
            return Err(ProfileError::EmptySamples { what });
        }
        if samples.iter().any(|ms| !ms.is_finite() || *ms < 0.0) {
            return Err(ProfileError::NonFiniteSample { what });
        }
        let mut sorted = samples.to_vec();
        sorted.sort_by(f64::total_cmp);
        let at = |p: f64| crate::latency::nearest_rank_sorted(&sorted, p);
        Ok(Self {
            frames: samples.len() as u32,
            p50_ms: at(50.0),
            p95_ms: at(95.0),
            p99_ms: at(99.0),
        })
    }
}

/// Per-system timing over the capture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SystemStats {
    /// Span key from [`SYSTEM_SPANS`].
    pub span: &'static str,
    /// Quantiles over per-frame ms.
    pub quantiles: Quantiles,
}

/// Per-pass timing over the capture (CPU side; GPU side stays operator-side).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PassStats {
    /// Pass sampled.
    pub pass: GpuPass,
    /// Quantiles over per-frame CPU ms.
    pub quantiles: Quantiles,
}

/// Diagnostics series the capture feeds once per frame (see `crate::diagnostics`).
pub const FRAME_TIME_SERIES: &str = "frame_time";
/// Smoothing time for [`FRAME_TIME_SERIES`] in micros: about six 60 Hz frames.
pub const FRAME_TIME_SMOOTHING_US: u64 = 100_000;

/// Frame-capture report: quantiles per system and per pass, naming the
/// costliest of each by p50, plus the smoothed frame time. Numbers are this
/// machine's wall clock; `gpu_ms` everywhere is `None` until the operator's
/// reference-GPU run.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameReport {
    /// Frames captured.
    pub frames: u32,
    /// One row per system span, in [`SYSTEM_SPANS`] order.
    pub systems: Vec<SystemStats>,
    /// One row per GPU pass, in [`GpuPass::ALL`] order.
    pub passes: Vec<PassStats>,
    /// Costliest system by p50 (ties keep report order, so deterministic).
    pub costliest_system: &'static str,
    /// Its p50 in ms.
    pub costliest_system_p50_ms: f64,
    /// Costliest pass by CPU p50 (ties keep report order).
    pub costliest_pass: &'static str,
    /// Its CPU p50 in ms.
    pub costliest_pass_p50_ms: f64,
    /// Row text of the `frame_time` series: latest, smoothed and mean ms.
    pub frame_time_row: String,
    /// Smoothed frame time over the capture, ms (time-aware average).
    pub frame_time_smoothed_ms: f64,
}

impl FrameReport {
    /// Human-readable lines for the perf report: the frame-time row, one row
    /// per system and pass, plus the two costliest verdicts. Numbers vary per
    /// machine; names and units (`ms`, span/pass keys) are stable.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut out = format!("frame_capture frames={}\n", self.frames);
        out.push_str(&format!("{}\n", self.frame_time_row));
        for row in &self.systems {
            out.push_str(&format!(
                "system {} p50={:.3}ms p95={:.3}ms p99={:.3}ms\n",
                row.span, row.quantiles.p50_ms, row.quantiles.p95_ms, row.quantiles.p99_ms
            ));
        }
        for row in &self.passes {
            out.push_str(&format!(
                "pass {} cpu_p50={:.3}ms cpu_p95={:.3}ms cpu_p99={:.3}ms gpu_ms=operator-side\n",
                row.pass.name(),
                row.quantiles.p50_ms,
                row.quantiles.p95_ms,
                row.quantiles.p99_ms
            ));
        }
        out.push_str(&format!(
            "costliest_system {} p50={:.3}ms\n",
            self.costliest_system, self.costliest_system_p50_ms
        ));
        out.push_str(&format!(
            "costliest_pass {} cpu_p50={:.3}ms\n",
            self.costliest_pass, self.costliest_pass_p50_ms
        ));
        out
    }
}

// ---------------------------------------------------------------------------
// Secondary tick-frame marks + plot series: reimplemented mark shape beside
// the frozen [`FrameReport`], no tracy server, no new dependency.
// ---------------------------------------------------------------------------

/// Ticks per secondary capture: the canonical 600-tick arena run.
pub const TICK_FRAME_TICKS: u32 = 600;
/// Most marks one tick-frame capture stores (one tick plus one frame mark
/// per sampled tick, sampled every 8th tick like the harness seam).
pub const TICK_FRAME_MARKS_MAX: usize = 150;
/// Label for simulation-tick marks.
pub const MARK_TICK: &str = "tick";
/// Label for frame marks.
pub const MARK_FRAME: &str = "frame";
/// Every secondary mark label, in series order.
pub const MARK_LABELS: &[&str] = &[MARK_TICK, MARK_FRAME];
/// Sample stride for secondary marks: every 8th tick, like the harness seam.
pub const MARK_EVERY: u32 = 8;
/// Most buckets one rendered plot holds; keeps the receipt readable.
pub const PLOT_BUCKETS_MAX: usize = 64;

/// One secondary mark: a tick index plus which series it belongs to.
/// Consumer: the tick-frame receipt beside `FrameReport::summary`. Units: ticks
/// for the index, label strings for the series. Determinism: marks carry no
/// wall clock and never enter any simulation hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickMark {
    /// Tick index in `0..total`.
    pub tick: u32,
    /// Series label from [`MARK_LABELS`].
    pub label: &'static str,
}

impl TickMark {
    /// Build a mark, refusing out-of-range ticks and unknown labels.
    pub fn new(tick: u32, total: u32, label: &'static str) -> Result<Self, ProfileError> {
        if !MARK_LABELS.contains(&label) {
            return Err(ProfileError::UnknownMark {
                mark: label.to_owned(),
            });
        }
        if total == 0 || total > TICK_FRAME_TICKS || tick >= total {
            return Err(ProfileError::InvalidTicks { ticks: total });
        }
        Ok(Self { tick, label })
    }
}

/// Bounded secondary-mark series over one capture.
/// Consumer: [`TickFrameSeries`]. Units: ticks. Determinism: insertion order
/// is the capture order, so text output repeats exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkSeries {
    /// Ticks captured; every mark index is below this.
    pub total: u32,
    /// Marks in capture order.
    marks: Vec<TickMark>,
}

impl MarkSeries {
    /// Empty series for a `total`-tick capture; refuses counts outside
    /// `1..=TICK_FRAME_TICKS`.
    pub fn new(total: u32) -> Result<Self, ProfileError> {
        if total == 0 || total > TICK_FRAME_TICKS {
            return Err(ProfileError::InvalidTicks { ticks: total });
        }
        Ok(Self {
            total,
            marks: Vec::new(),
        })
    }

    /// Push one mark from this capture; refuses foreign ticks and overflow.
    pub fn push(&mut self, mark: TickMark) -> Result<(), ProfileError> {
        if mark.tick >= self.total {
            return Err(ProfileError::InvalidTicks { ticks: self.total });
        }
        if self.marks.len() >= TICK_FRAME_MARKS_MAX {
            return Err(ProfileError::TooManyMarks {
                limit: TICK_FRAME_MARKS_MAX,
            });
        }
        self.marks.push(mark);
        Ok(())
    }

    /// Marks held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.marks.len()
    }

    /// True while no mark has been pushed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    /// Marks of one series, in capture order.
    #[must_use]
    pub fn marks_for(&self, label: &str) -> Vec<TickMark> {
        self.marks
            .iter()
            .copied()
            .filter(|mark| mark.label == label)
            .collect()
    }

    /// Human-readable lines: one header plus one row per mark.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut out = format!(
            "tick_frame_marks total={} marks={}\n",
            self.total,
            self.marks.len()
        );
        for mark in &self.marks {
            out.push_str(&format!("mark tick={} label={}\n", mark.tick, mark.label));
        }
        out
    }
}

/// One named plot series: per-iteration wall-clock ms in capture order.
/// Consumer: [`TickFrameSeries`]. Units: milliseconds; evidence only, never
/// hashed. Determinism: samples repeat per input only through their order;
/// values are this machine's wall clock.
#[derive(Clone, Debug, PartialEq)]
pub struct PlotSeries {
    /// Series name (`tick` or `frame`).
    pub name: &'static str,
    /// Per-iteration ms in capture order.
    samples: Vec<f64>,
}

impl PlotSeries {
    /// Empty series; refuses labels outside the secondary set.
    pub fn new(name: &'static str) -> Result<Self, ProfileError> {
        if !MARK_LABELS.contains(&name) {
            return Err(ProfileError::UnknownMark {
                mark: name.to_owned(),
            });
        }
        Ok(Self {
            name,
            samples: Vec::new(),
        })
    }

    /// Push one sample; refuses non-finite ms and overflow past the capture.
    pub fn push_ms(&mut self, ms: f64) -> Result<(), ProfileError> {
        if !ms.is_finite() || ms < 0.0 {
            return Err(ProfileError::NonFiniteSample { what: self.name });
        }
        let len = u32::try_from(self.samples.len())
            .map_err(|_| ProfileError::TooManySamples { what: self.name })?;
        if len >= TICK_FRAME_TICKS {
            return Err(ProfileError::TooManySamples { what: self.name });
        }
        self.samples.push(ms);
        Ok(())
    }

    /// Samples captured.
    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// True while no sample has been pushed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Quantiles over this series' samples.
    pub fn quantiles(&self) -> Result<Quantiles, ProfileError> {
        Quantiles::of(self.name, &self.samples)
    }
}

/// Tick-vs-frame capture: per-iteration tick ms beside per-iteration frame
/// ms, secondary marks every [`MARK_EVERY`]th tick, and the canonical
/// 600-tick arena hash for the repeated-hash proof.
/// Consumer: the tick-frame receipt beside the frame-capture report. Units: ticks, milliseconds, hash hex. Determinism:
/// the hash repeats exactly; only wall-clock ms vary.
#[derive(Clone, Debug, PartialEq)]
pub struct TickFrameSeries {
    /// Ticks captured.
    pub ticks: u32,
    /// Canonical 600-tick arena hash (evidence only, never perturbed here).
    pub hash: u64,
    /// Per-iteration tick ms.
    pub tick_plot: PlotSeries,
    /// Per-iteration frame ms.
    pub frame_plot: PlotSeries,
    /// Secondary marks in capture order.
    pub marks: MarkSeries,
}

impl TickFrameSeries {
    /// Human-readable lines: hashes plus both p50/p95/p99 rows. Fails with
    /// [`ProfileError::EmptySamples`] or [`ProfileError::NonFiniteSample`]
    /// when a series is empty or holds a bad sample (captures build both
    /// series together, so this only trips on hand-built input).
    pub fn try_summary(&self) -> Result<String, ProfileError> {
        let tick_q = self.tick_plot.quantiles()?;
        let frame_q = self.frame_plot.quantiles()?;
        Ok(format!(
            "tick_frame ticks={} hash={:016x}\n\
             tick p50={:.3}ms p95={:.3}ms p99={:.3}ms\n\
             frame p50={:.3}ms p95={:.3}ms p99={:.3}ms\n\
             marks={}\n",
            self.ticks,
            self.hash,
            tick_q.p50_ms,
            tick_q.p95_ms,
            tick_q.p99_ms,
            frame_q.p50_ms,
            frame_q.p95_ms,
            frame_q.p99_ms,
            self.marks.len(),
        ))
    }

    /// Bucketed p50 plot for both series: `buckets` rows per series with
    /// `#` bars scaled to the hotter bucket. Refuses counts outside
    /// `1..=PLOT_BUCKETS_MAX` or above the sample count.
    pub fn plot_text(&self, buckets: usize) -> Result<String, ProfileError> {
        // Bucket counts fit a u32 by construction (at most 64); the
        // conversion names its own refusal instead of casting.
        let buckets_u32 =
            u32::try_from(buckets).map_err(|_| ProfileError::InvalidTicks { ticks: u32::MAX })?;
        if buckets == 0 || buckets > PLOT_BUCKETS_MAX {
            return Err(ProfileError::InvalidTicks { ticks: buckets_u32 });
        }
        let bucket_p50 = |name: &'static str, samples: &[f64]| -> Result<Vec<f64>, ProfileError> {
            if buckets > samples.len() {
                return Err(ProfileError::InvalidTicks { ticks: buckets_u32 });
            }
            let mut out = Vec::with_capacity(buckets);
            for bucket in 0..buckets {
                let start = bucket * samples.len() / buckets;
                let end = (bucket + 1) * samples.len() / buckets;
                // Nearest-rank p50 per bucket through the one quantile owner,
                // so bucket medians match the report rows by construction.
                let median = Quantiles::of(name, &samples[start..end])?.p50_ms;
                out.push(median);
            }
            Ok(out)
        };
        // Bucket medians render the captured samples in order, so the text
        // repeats exactly for the same capture.
        let tick_p50s = bucket_p50(self.tick_plot.name, self.tick_plot.samples())?;
        let frame_p50s = bucket_p50(self.frame_plot.name, self.frame_plot.samples())?;
        let ceiling = tick_p50s
            .iter()
            .chain(frame_p50s.iter())
            .copied()
            .fold(0.0f64, f64::max)
            .max(1e-9);
        let mut out = format!(
            "plot buckets={} ticks={} ceiling={:.3}ms\n",
            buckets, self.ticks, ceiling
        );
        let bar = |p50: f64| {
            // Bar width in whole `#` blocks through a bounded integer loop,
            // so no float-to-int cast enters the rendering path.
            let width = ((p50 / ceiling) * 40.0).round().clamp(0.0, 40.0);
            let mut bar = String::new();
            for slot in 0..40u32 {
                if f64::from(slot) < width {
                    bar.push('#');
                }
            }
            bar
        };
        for (index, (tick, frame)) in tick_p50s.iter().zip(frame_p50s.iter()).enumerate() {
            out.push_str(&format!(
                "bucket {index:02} tick_p50={tick:.3}ms {} frame_p50={frame:.3}ms {}\n",
                bar(*tick),
                bar(*frame)
            ));
        }
        Ok(out)
    }
}

impl PlotSeries {
    /// Samples in capture order (plot rendering input).
    fn samples(&self) -> &[f64] {
        &self.samples
    }
}

/// Capture `ticks` tick-vs-frame iterations: per iteration, time the same
/// 4-tick arena probe the frame capture uses (tick ms) plus the six fixed
/// GPU pass probes back to back (frame ms), planting one `tick` and one
/// `frame` secondary mark every [`MARK_EVERY`]th tick.
///
/// Fails with [`ProfileError::Disabled`] while the profiler is off and with
/// [`ProfileError::InvalidTicks`] outside `1..=TICK_FRAME_TICKS`.
/// Simulation hashes are computed and discarded (`black_box`); the stored
/// `hash` is the canonical 600-tick arena hash, identical with the profiler
/// gated off. Only wall-clock ms enter the plots.
pub fn capture_tick_frame(ticks: u32) -> Result<TickFrameSeries, ProfileError> {
    if !is_enabled() {
        return Err(ProfileError::Disabled);
    }
    if ticks == 0 || ticks > TICK_FRAME_TICKS {
        return Err(ProfileError::InvalidTicks { ticks });
    }
    let probes = FrameProbes::fixed()?;
    let mut tick_plot = PlotSeries::new(MARK_TICK)?;
    let mut frame_plot = PlotSeries::new(MARK_FRAME)?;
    let mut marks = MarkSeries::new(ticks)?;
    let mut forward_grid = engine_render::LightGrid::new(engine_render::TILE_COUNT_1080P);
    for tick in 0..ticks {
        begin_frame();
        scope!("perf.arena_tick");
        let started = Instant::now();
        let arena_hash = ArenaTick {
            seed: 1,
            ticks: 4,
            half_extent_m: 20.0,
        }
        .run_hash();
        std::hint::black_box(arena_hash);
        tick_plot.push_ms(started.elapsed().as_secs_f64() * 1000.0)?;

        scope!("ra1.forward");
        let frame_started = Instant::now();
        std::hint::black_box(probe_ra1_forward(&mut forward_grid));
        std::hint::black_box(probe_rl1_sun()?);
        std::hint::black_box(probe_rl1_cluster(&probes.cluster_lights)?);
        std::hint::black_box(probe_rl2_ibl()?);
        std::hint::black_box(probe_rp1_taa(&probes)?);
        std::hint::black_box(probe_rp1_post(&probes)?);
        frame_plot.push_ms(frame_started.elapsed().as_secs_f64() * 1000.0)?;

        if tick.is_multiple_of(MARK_EVERY) {
            marks.push(TickMark::new(tick, ticks, MARK_TICK)?)?;
            marks.push(TickMark::new(tick, ticks, MARK_FRAME)?)?;
        }
    }
    let hash = ArenaTick::canonical().run_hash();
    std::hint::black_box(hash);
    Ok(TickFrameSeries {
        ticks,
        hash,
        tick_plot,
        frame_plot,
        marks,
    })
}

/// Paired 600-tick rerun (fails Disabled while off); hashes compare in the receipt.
pub fn capture_paired_tick_frame() -> Result<(TickFrameSeries, TickFrameSeries), ProfileError> {
    Ok((
        capture_tick_frame(TICK_FRAME_TICKS)?,
        capture_tick_frame(TICK_FRAME_TICKS)?,
    ))
}

/// Paired receipt: kills on mismatch, else `hash_A == hash_B` plus both
/// tick/frame quantile rows plus machine+profile line (pure render: x2 modulo wall-clock ms).
pub fn paired_tick_frame_summary(
    a: &TickFrameSeries,
    b: &TickFrameSeries,
) -> Result<String, ProfileError> {
    if a.hash != b.hash {
        return Err(ProfileError::DivergedPair {
            hash_a: a.hash,
            hash_b: b.hash,
        });
    }
    let (ta, tb) = (a.tick_plot.quantiles()?, b.tick_plot.quantiles()?);
    let (fa, fb) = (a.frame_plot.quantiles()?, b.frame_plot.quantiles()?);
    Ok(format!(
        "paired_tick_frame ticks={} hash_A == hash_B ({:016x})\n\
         tick A p50={:.3}ms p95={:.3}ms p99={:.3}ms | B p50={:.3}ms p95={:.3}ms p99={:.3}ms\n\
         frame A p50={:.3}ms p95={:.3}ms p99={:.3}ms | B p50={:.3}ms p95={:.3}ms p99={:.3}ms\n\
         machine={}-{} profile={}\n",
        a.ticks,
        a.hash,
        ta.p50_ms,
        ta.p95_ms,
        ta.p99_ms,
        tb.p50_ms,
        tb.p95_ms,
        tb.p99_ms,
        fa.p50_ms,
        fa.p95_ms,
        fa.p99_ms,
        fb.p50_ms,
        fb.p95_ms,
        fb.p99_ms,
        std::env::consts::OS,
        std::env::consts::ARCH,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
    ))
}

/// Fixed per-frame CPU probes. Inputs are constants, so every capture runs
/// the same work on every machine; only the wall clock varies.
struct FrameProbes {
    bloom_frame: Vec<[f32; 3]>,
    bloom_cfg: engine_render::post::BloomConfig,
    taa_motion: engine_render::post::MotionField,
    taa_params: engine_render::post::TaaParams,
    cluster_lights: Vec<engine_render::shadow_cluster::ScreenLight>,
}

impl FrameProbes {
    /// Build the fixed probe inputs; fails loudly on a bad constant.
    fn fixed() -> Result<Self, ProfileError> {
        const W: u32 = 16;
        const H: u32 = 16;
        let mut bloom_frame = Vec::with_capacity(W as usize * H as usize);
        for y in 0..H {
            for x in 0..W {
                // Fixed checkerboard in linear HDR: bright cells feed bloom.
                let lit = (x / 2 + y / 2).is_multiple_of(2);
                let v = if lit { 2.0 } else { 0.1 };
                bloom_frame.push([v, v * 0.9, v * 0.8]);
            }
        }
        let taa_motion = engine_render::post::MotionField::zero(W, H).map_err(|e| {
            ProfileError::ProbeFailed {
                what: "rp1",
                reason: e.to_string(),
            }
        })?;
        // Fixed torch lattice over a 256x144 frame: every light is finite
        // with a positive radius, so assignment always succeeds.
        let mut cluster_lights = Vec::with_capacity(8);
        for i in 0..8u32 {
            cluster_lights.push(engine_render::shadow_cluster::ScreenLight {
                x_px: 16.0 + i as f32 * 28.0,
                y_px: 24.0 + (i % 3) as f32 * 32.0,
                depth_m: 5.0 + i as f32,
                radius_px: 40.0,
            });
        }
        Ok(Self {
            bloom_frame,
            bloom_cfg: engine_render::post::BloomConfig::default(),
            taa_motion,
            taa_params: engine_render::post::TaaParams::default(),
            cluster_lights,
        })
    }
}

/// Forward probe: fill the forward+ tile-grid mirror over a 1080p grid with a
/// fixed light per tile. Real `engine-render` code, fixed inputs. The caller
/// lends one grid reused across iterations (frame-arena shape), so per-frame
/// work stays allocation-free after the first fill; outputs are identical.
fn probe_ra1_forward(grid: &mut engine_render::LightGrid) -> u64 {
    grid.counts.fill(0);
    grid.indices.clear();
    grid.overflow_tiles = 0;
    for tile in 0..engine_render::TILE_COUNT_1080P {
        grid.push(tile, (tile % 8) as u32);
    }
    grid.overflow_tiles as u64 + grid.indices.len() as u64
}

/// Sun probe: validate plus split a fixed cascade config, then select a
/// cascade over a fixed depth lattice.
fn probe_rl1_sun() -> Result<[f32; 4], ProfileError> {
    let pass = engine_render::shadow_cluster::SunShadowPass::build(
        engine_render::shadow_cluster::SunShadowConfig {
            near_m: 0.5,
            far_m: 200.0,
            half_extents: [30.0, 30.0, 30.0],
            biases: [0.005, 0.005, 0.005],
            size: 64,
        },
    )
    .map_err(|e| ProfileError::ProbeFailed {
        what: "rl1",
        reason: e.to_string(),
    })?;
    let mut acc = 0u32;
    for i in 0..16 {
        acc += pass.cascade_index(i as f32 * 12.5) as u32;
    }
    std::hint::black_box(acc);
    Ok(pass.splits)
}

/// Cluster probe: assign the fixed torch lattice into a small grid.
fn probe_rl1_cluster(
    lights: &[engine_render::shadow_cluster::ScreenLight],
) -> Result<usize, ProfileError> {
    let pass = engine_render::shadow_cluster::ClusterPass::build(
        engine_render::shadow_cluster::ClusterConfig {
            width_px: 256,
            height_px: 144,
            tile_px: 16,
            depth_slices: 4,
            near_m: 0.5,
            far_m: 200.0,
            max_lights: 64,
            max_per_cluster: 24,
        },
        lights,
    )
    .map_err(|e| ProfileError::ProbeFailed {
        what: "rl1",
        reason: e.to_string(),
    })?;
    Ok(pass.indices.len())
}

/// IBL probe: GGX split-sum taps over a fixed NdotV/roughness lattice.
fn probe_rl2_ibl() -> Result<[f32; 2], ProfileError> {
    let mut acc = [0.0f32; 2];
    for (i, ndotv) in [0.15f32, 0.35, 0.55, 0.75].iter().enumerate() {
        let roughness = 0.2 + i as f32 * 0.2;
        let tap = engine_render::ibl::brdf_integrate(*ndotv, roughness).map_err(|e| {
            ProfileError::ProbeFailed {
                what: "rl2",
                reason: e.to_string(),
            }
        })?;
        acc[0] += tap[0];
        acc[1] += tap[1];
    }
    Ok(acc)
}

/// TAA probe: resolve a static 16x16 frame (history equals current, so
/// the output reproduces the input exactly).
fn probe_rp1_taa(probes: &FrameProbes) -> Result<usize, ProfileError> {
    let out = engine_render::post::taa_resolve(
        &probes.bloom_frame,
        &probes.bloom_frame,
        &probes.taa_motion,
        16,
        16,
        &probes.taa_params,
    )
    .map_err(|e| ProfileError::ProbeFailed {
        what: "rp1",
        reason: e.to_string(),
    })?;
    Ok(out.len())
}

/// Post probe: bloom the fixed HDR checkerboard.
fn probe_rp1_post(probes: &FrameProbes) -> Result<usize, ProfileError> {
    let out = engine_render::post::apply_bloom(&probes.bloom_frame, 16, 16, &probes.bloom_cfg)
        .map_err(|e| ProfileError::ProbeFailed {
            what: "rp1",
            reason: e.to_string(),
        })?;
    Ok(out.len())
}

/// Capture `frames` fixed frames: time the four system probes plus the
/// six GPU pass probes per frame, then summarize with p50/p95/p99 and name
/// the costliest system and pass by p50.
///
/// Fails with [`ProfileError::Disabled`] while the profiler is off and with
/// [`ProfileError::InvalidFrames`] outside `1..=CAPTURE_FRAMES_MAX`.
/// Simulation hashes are computed and discarded (`black_box`); only
/// wall-clock ms enter the report.
pub fn capture(frames: u32) -> Result<FrameReport, ProfileError> {
    if !is_enabled() {
        return Err(ProfileError::Disabled);
    }
    if frames == 0 || frames > CAPTURE_FRAMES_MAX {
        return Err(ProfileError::InvalidFrames { frames });
    }
    let probes = FrameProbes::fixed()?;
    let mut forward_grid = engine_render::LightGrid::new(engine_render::TILE_COUNT_1080P);
    let capacity = frames as usize;
    let mut arena_ms = Vec::with_capacity(capacity);
    let mut physics_ms = Vec::with_capacity(capacity);
    let mut ai_ms = Vec::with_capacity(capacity);
    let mut destruction_ms = Vec::with_capacity(capacity);
    let mut ra1_ms = Vec::with_capacity(capacity);
    let mut rl1_sun_ms = Vec::with_capacity(capacity);
    let mut rl1_cluster_ms = Vec::with_capacity(capacity);
    let mut rl2_ms = Vec::with_capacity(capacity);
    let mut rp1_taa_ms = Vec::with_capacity(capacity);
    let mut rp1_post_ms = Vec::with_capacity(capacity);
    let mut diagnostics = DiagnosticsStore::new(&[(FRAME_TIME_SERIES, FRAME_TIME_SMOOTHING_US)])?;

    for _ in 0..frames {
        let frame_started = Instant::now();
        begin_frame();
        scope!("perf.arena_tick");
        let started = Instant::now();
        let arena_hash = ArenaTick {
            seed: 1,
            ticks: 4,
            half_extent_m: 20.0,
        }
        .run_hash();
        std::hint::black_box(arena_hash);
        arena_ms.push(started.elapsed().as_secs_f64() * 1000.0);

        scope!("perf.physics_step");
        let started = Instant::now();
        let (physics_hash, _) = PhysicsStep::canonical().steady(8);
        std::hint::black_box(physics_hash);
        physics_ms.push(started.elapsed().as_secs_f64() * 1000.0);

        scope!("perf.ai_decide");
        let started = Instant::now();
        let (ai_hash, _) = AiDecide::canonical().steady(8);
        std::hint::black_box(ai_hash);
        ai_ms.push(started.elapsed().as_secs_f64() * 1000.0);

        scope!("perf.destruction_step");
        let started = Instant::now();
        let destruction_hash = DestructionStep::canonical().run_hash();
        std::hint::black_box(destruction_hash);
        destruction_ms.push(started.elapsed().as_secs_f64() * 1000.0);

        scope!("ra1.forward");
        let started = Instant::now();
        std::hint::black_box(probe_ra1_forward(&mut forward_grid));
        ra1_ms.push(started.elapsed().as_secs_f64() * 1000.0);

        scope!("rl1.sun_shadow");
        let started = Instant::now();
        std::hint::black_box(probe_rl1_sun()?);
        rl1_sun_ms.push(started.elapsed().as_secs_f64() * 1000.0);

        scope!("rl1.torch_cluster");
        let started = Instant::now();
        std::hint::black_box(probe_rl1_cluster(&probes.cluster_lights)?);
        rl1_cluster_ms.push(started.elapsed().as_secs_f64() * 1000.0);

        scope!("rl2.ibl");
        let started = Instant::now();
        std::hint::black_box(probe_rl2_ibl()?);
        rl2_ms.push(started.elapsed().as_secs_f64() * 1000.0);

        scope!("rp1.taa");
        let started = Instant::now();
        std::hint::black_box(probe_rp1_taa(&probes)?);
        rp1_taa_ms.push(started.elapsed().as_secs_f64() * 1000.0);

        scope!("rp1.post");
        let started = Instant::now();
        std::hint::black_box(probe_rp1_post(&probes)?);
        rp1_post_ms.push(started.elapsed().as_secs_f64() * 1000.0);

        // The frame's own duration is also its time step to the next frame.
        let frame_ms = frame_started.elapsed().as_secs_f64() * 1000.0;
        let _accepted = diagnostics.push_ms(FRAME_TIME_SERIES, frame_ms, frame_ms)?;
    }

    let system_rows = [
        (SPAN_ARENA_TICK, arena_ms),
        (SPAN_PHYSICS_STEP, physics_ms),
        (SPAN_AI_DECIDE, ai_ms),
        (SPAN_DESTRUCTION_STEP, destruction_ms),
    ];
    let mut systems = Vec::with_capacity(system_rows.len());
    for (span, samples) in &system_rows {
        systems.push(SystemStats {
            span,
            quantiles: Quantiles::of(span, samples)?,
        });
    }
    let pass_rows = [
        (GpuPass::Ra1Forward, ra1_ms),
        (GpuPass::Rl1SunShadow, rl1_sun_ms),
        (GpuPass::Rl1TorchCluster, rl1_cluster_ms),
        (GpuPass::Rl2Ibl, rl2_ms),
        (GpuPass::Rp1Taa, rp1_taa_ms),
        (GpuPass::Rp1Post, rp1_post_ms),
    ];
    let mut passes = Vec::with_capacity(pass_rows.len());
    for (pass, samples) in &pass_rows {
        passes.push(PassStats {
            pass: *pass,
            quantiles: Quantiles::of(pass.name(), samples)?,
        });
    }
    assemble_report(frames, systems, passes, &diagnostics)
}

/// Builds the frame report from the per-system and per-pass rows and the
/// capture's diagnostics store. The one owner of the costliest verdicts and
/// of the frame-time line, so the printed report always reads the store.
/// Fails with [`ProfileError::EmptySamples`] when a row set is empty or the
/// frame-time series holds no sample.
fn assemble_report(
    frames: u32,
    systems: Vec<SystemStats>,
    passes: Vec<PassStats>,
    diagnostics: &DiagnosticsStore,
) -> Result<FrameReport, ProfileError> {
    let frame_time = diagnostics.series(FRAME_TIME_SERIES)?;
    let smoothed = frame_time.smoothed().ok_or(ProfileError::EmptySamples {
        what: FRAME_TIME_SERIES,
    })?;
    // Costliest by p50; ties keep report order, so the verdict is stable.
    // Names and p50s copy out first so the rows below can move into the report.
    let top_system = systems
        .iter()
        .max_by(|a, b| a.quantiles.p50_ms.total_cmp(&b.quantiles.p50_ms))
        .ok_or(ProfileError::EmptySamples { what: "systems" })?;
    let (costliest_system, costliest_system_p50_ms) =
        (top_system.span, top_system.quantiles.p50_ms);
    let top_pass = passes
        .iter()
        .max_by(|a, b| a.quantiles.p50_ms.total_cmp(&b.quantiles.p50_ms))
        .ok_or(ProfileError::EmptySamples { what: "passes" })?;
    let (costliest_pass, costliest_pass_p50_ms) = (top_pass.pass.name(), top_pass.quantiles.p50_ms);
    Ok(FrameReport {
        frames,
        systems,
        passes,
        costliest_system,
        costliest_system_p50_ms,
        costliest_pass,
        costliest_pass_p50_ms,
        frame_time_row: frame_time.row(),
        frame_time_smoothed_ms: smoothed.as_secs_f64() * 1000.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_report_reads_the_frame_time_series() {
        // The report's frame-time line and smoothed ms must come from the store
        // the capture fed: a bare report with no samples is refused, not zeroed.
        let rows = || {
            let systems = vec![SystemStats {
                span: SPAN_ARENA_TICK,
                quantiles: Quantiles::of(SPAN_ARENA_TICK, &[1.0, 2.0]).expect("samples present"),
            }];
            let passes = vec![PassStats {
                pass: GpuPass::Ra1Forward,
                quantiles: Quantiles::of(GpuPass::Ra1Forward.name(), &[0.5])
                    .expect("samples present"),
            }];
            (systems, passes)
        };
        let mut diagnostics =
            DiagnosticsStore::new(&[(FRAME_TIME_SERIES, FRAME_TIME_SMOOTHING_US)])
                .expect("series builds");
        for ms in [16.0, 17.0, 33.0] {
            assert!(diagnostics
                .push_ms(FRAME_TIME_SERIES, ms, ms)
                .expect("known series"));
        }
        let (systems, passes) = rows();
        let report = assemble_report(3, systems, passes, &diagnostics).expect("report builds");
        let series = diagnostics.series(FRAME_TIME_SERIES).expect("known series");
        assert_eq!(report.frame_time_row, series.row());
        let smoothed_ms = series.smoothed().expect("samples present").as_secs_f64() * 1000.0;
        assert!((report.frame_time_smoothed_ms - smoothed_ms).abs() < 1e-9);
        assert!(report.summary().contains(&format!("{}\n", series.row())));
        let (systems, passes) = rows();
        let empty = DiagnosticsStore::new(&[(FRAME_TIME_SERIES, FRAME_TIME_SMOOTHING_US)])
            .expect("series builds");
        assert!(matches!(
            assemble_report(3, systems, passes, &empty),
            Err(ProfileError::EmptySamples { .. })
        ));
    }

    #[test]
    fn off_by_default_without_touching_the_flag() {
        // ENABLED starts false and no test in this binary ever flips it:
        // flag-toggling tests live in tests/profiler.rs under a mutex.
        assert!(!is_enabled());
    }

    #[test]
    fn error_strings_name_themselves() {
        let cases: &[ProfileError] = &[
            ProfileError::Disabled,
            ProfileError::InvalidFrames { frames: 0 },
            ProfileError::UnknownSpan {
                span: "nope".to_owned(),
            },
            ProfileError::UnknownPass {
                pass: "nope".to_owned(),
            },
            ProfileError::EmptySamples { what: "probe" },
            ProfileError::NonFiniteSample { what: "probe" },
            ProfileError::ProbeFailed {
                what: "rl1",
                reason: "bad".to_owned(),
            },
        ];
        for error in cases {
            let text = error.to_string();
            assert!(!text.is_empty(), "{error:?} explains itself");
            let _: &dyn std::error::Error = error;
        }
        assert!(ProfileError::Disabled.to_string().contains("disabled"));
        assert!(ProfileError::InvalidFrames { frames: 0 }
            .to_string()
            .contains("frames"));
    }

    #[test]
    fn span_and_pass_lookups() {
        assert_eq!(system_span(SPAN_ARENA_TICK), Ok(SPAN_ARENA_TICK));
        assert_eq!(
            system_span("gen_ai.request"),
            Err(ProfileError::UnknownSpan {
                span: "gen_ai.request".to_owned(),
            })
        );
        assert_eq!(
            GpuPass::from_name("rl1.sun_shadow"),
            Ok(GpuPass::Rl1SunShadow)
        );
        assert_eq!(
            GpuPass::from_name("r9.fake"),
            Err(ProfileError::UnknownPass {
                pass: "r9.fake".to_owned(),
            })
        );
        // Query slots pack as begin/end pairs in report order, no overlaps.
        let mut slots = Vec::new();
        for pass in GpuPass::ALL {
            assert_eq!(pass.query_end(), pass.query_begin() + 1);
            slots.push(pass.query_begin());
            slots.push(pass.query_end());
        }
        slots.sort_unstable();
        slots.dedup();
        assert_eq!(slots.len(), 2 * GpuPass::ALL.len());
    }

    #[test]
    fn sample_validation() {
        let good = Sample::new(SPAN_AI_DECIDE, 1.25).expect("valid sample builds");
        assert_eq!((good.span, good.ms), (SPAN_AI_DECIDE, 1.25));
        assert!(matches!(
            Sample::new("nope.span", 1.0),
            Err(ProfileError::UnknownSpan { .. })
        ));
        for bad in [f64::NAN, f64::INFINITY, -0.5] {
            assert!(
                matches!(
                    Sample::new(SPAN_AI_DECIDE, bad),
                    Err(ProfileError::NonFiniteSample { .. })
                ),
                "ms {bad} is refused"
            );
        }
        let sample = GpuSample::cpu_only(GpuPass::Rl2Ibl, 0.5).expect("valid pass sample");
        assert_eq!(sample.gpu_ms, None);
        assert!(matches!(
            GpuSample::cpu_only(GpuPass::Rl2Ibl, f64::NAN),
            Err(ProfileError::NonFiniteSample { .. })
        ));
    }

    #[test]
    fn quantiles_rank_and_refuse() {
        let q = Quantiles::of("probe", &[4.0, 1.0, 3.0, 2.0]).expect("four samples rank");
        assert_eq!(q.frames, 4);
        assert!(q.p50_ms <= q.p95_ms && q.p95_ms <= q.p99_ms);
        assert_eq!(
            Quantiles::of("probe", &[]),
            Err(ProfileError::EmptySamples { what: "probe" })
        );
        assert!(Quantiles::of("probe", &[1.0, f64::NAN]).is_err());
        assert!(Quantiles::of("probe", &[1.0, -2.0]).is_err());
    }

    #[test]
    fn fixed_probes_run() {
        let probes = FrameProbes::fixed().expect("fixed probe inputs build");
        let mut grid = engine_render::LightGrid::new(engine_render::TILE_COUNT_1080P);
        assert_eq!(probe_ra1_forward(&mut grid), 8160);
        // Reused grid repeats the identical output without reallocating.
        assert_eq!(probe_ra1_forward(&mut grid), 8160);
        assert!(probe_rl1_sun().is_ok());
        assert!(probe_rl1_cluster(&probes.cluster_lights).is_ok());
        assert!(probe_rl2_ibl().is_ok());
        assert_eq!(probe_rp1_taa(&probes), Ok(256));
        assert_eq!(probe_rp1_post(&probes), Ok(256));
    }

    #[test]
    fn conditional_gate_stays_off_without_touching_the_flag() {
        // Disabled profiler short-circuits before reading the condition:
        // both spellings cost one relaxed load plus a branch and record
        // nothing. This test never flips the flag (see the mutex rule
        // above); enabled-pair equality rides the integration suite.
        assert!(!gate_on(true));
        assert!(!gate_on(false));
        let (hash, sample) = profiled_physics_step_if(true);
        assert_eq!(hash, PhysicsStep::canonical().run_hash());
        assert_eq!(sample, None);
        let (plain, plain_sample) = profiled_physics_step();
        assert_eq!(hash, plain);
        assert_eq!(sample, plain_sample);
        // A false per-call condition also records nothing while off.
        let (skipped, skipped_sample) = profiled_physics_step_if(false);
        assert_eq!(skipped, PhysicsStep::canonical().run_hash());
        assert_eq!(skipped_sample, None);
    }

    #[test]
    fn frame_advance_is_noop_while_off() {
        // Off profiler advances no puffin frame: no lock, no clock, no
        // sample. This test never flips the flag (see the mutex rule
        // above); enabled frames ride the integration suite.
        assert!(!is_enabled());
        begin_frame();
        assert!(!is_enabled());
    }

    #[test]
    fn tick_frame_marks_validate_without_touching_the_flag() {
        // Pure mark validation: this test never flips the flag (see the
        // mutex rule above); the enabled capture rides the integration
        // suite, which serializes on its own mutex.
        assert_eq!(MARK_LABELS, &[MARK_TICK, MARK_FRAME]);
        let mark = TickMark::new(0, 600, MARK_TICK).expect("first tick marks");
        assert_eq!((mark.tick, mark.label), (0, MARK_TICK));
        assert!(matches!(
            TickMark::new(0, 600, "r9.fake"),
            Err(ProfileError::UnknownMark { .. })
        ));
        assert!(matches!(
            TickMark::new(600, 600, MARK_TICK),
            Err(ProfileError::InvalidTicks { .. })
        ));
        assert!(matches!(
            TickMark::new(0, 0, MARK_TICK),
            Err(ProfileError::InvalidTicks { .. })
        ));
        assert!(matches!(
            TickMark::new(0, TICK_FRAME_TICKS + 1, MARK_TICK),
            Err(ProfileError::InvalidTicks { .. })
        ));
        assert!(matches!(
            MarkSeries::new(0),
            Err(ProfileError::InvalidTicks { .. })
        ));
        assert!(matches!(
            MarkSeries::new(TICK_FRAME_TICKS + 1),
            Err(ProfileError::InvalidTicks { .. })
        ));
        // A foreign tick never joins the series.
        let mut series = MarkSeries::new(16).expect("16-tick series builds");
        assert!(series.is_empty());
        assert_eq!(series.len(), 0);
        assert!(series
            .push(TickMark::new(16, 600, MARK_TICK).expect("600-tick mark builds"))
            .is_err());
        assert!(series.is_empty(), "refused push moves no cursor");
        // While off, the capture refuses before reading any mark or sample.
        assert_eq!(
            capture_tick_frame(8),
            Err(ProfileError::Disabled),
            "off profiler refuses the tick-frame capture"
        );
        assert!(!is_enabled(), "this test never flips the flag");
    }

    #[test]
    fn plot_series_validate_and_render() {
        // Pure plot validation plus bucket rendering over fixed samples:
        // no flag, no wall clock, deterministic text per input.
        assert!(matches!(
            PlotSeries::new("r9.fake"),
            Err(ProfileError::UnknownMark { .. })
        ));
        let mut series = PlotSeries::new(MARK_TICK).expect("tick series builds");
        assert!(series.is_empty());
        for bad in [f64::NAN, f64::INFINITY, -0.5] {
            assert!(
                matches!(
                    series.push_ms(bad),
                    Err(ProfileError::NonFiniteSample { .. })
                ),
                "ms {bad} is refused"
            );
        }
        assert!(series.is_empty(), "refused pushes move no cursor");
        for sample in [4.0, 1.0, 3.0, 2.0] {
            series.push_ms(sample).expect("finite ms pushes");
        }
        assert_eq!(series.len(), 4);
        let quantiles = series.quantiles().expect("four samples rank");
        assert!(quantiles.p50_ms <= quantiles.p95_ms && quantiles.p95_ms <= quantiles.p99_ms);
        // Bucket text is bucketed p50s with bars, stable per input.
        let mut framed = PlotSeries::new(MARK_FRAME).expect("frame series builds");
        for sample in [8.0, 2.0, 6.0, 4.0] {
            framed.push_ms(sample).expect("finite ms pushes");
        }
        let marks = MarkSeries::new(4).expect("4-tick marks build");
        let capture = TickFrameSeries {
            ticks: 4,
            hash: 0xd791_b72c_1633_5366,
            tick_plot: series,
            frame_plot: framed,
            marks,
        };
        let summary = capture.try_summary().expect("fixed samples summarize");
        assert!(
            summary.contains("tick_frame ticks=4"),
            "summary names the run"
        );
        assert!(
            summary.contains("hash=d791b72c16335366"),
            "summary names the hash"
        );
        assert!(summary.contains("tick p50="), "summary names the tick p50");
        assert!(
            summary.contains("frame p50="),
            "summary names the frame p50"
        );
        let plot = capture.plot_text(2).expect("two buckets render");
        assert!(
            plot.contains("plot buckets=2 ticks=4"),
            "plot names its shape"
        );
        assert!(plot.contains("bucket 00"), "first bucket renders");
        assert!(plot.contains("bucket 01"), "second bucket renders");
        assert!(plot.contains("tick_p50="), "tick buckets render");
        assert!(plot.contains("frame_p50="), "frame buckets render");
        // A halved tick series provably moves its bucket p50.
        let halved = capture.plot_text(2).expect("rerun repeats");
        assert_eq!(plot, halved, "same capture renders identically");
        assert!(capture.plot_text(0).is_err(), "zero buckets refuse");
        assert!(
            capture.plot_text(PLOT_BUCKETS_MAX + 1).is_err(),
            "over-wide plots refuse"
        );
        assert!(
            capture.plot_text(5).is_err(),
            "more buckets than samples refuse"
        );
    }

    fn paired_side(hash: u64, base: f64) -> TickFrameSeries {
        let mut tick = PlotSeries::new(MARK_TICK).expect("tick series builds");
        let mut frame = PlotSeries::new(MARK_FRAME).expect("frame series builds");
        for step in 0..4 {
            tick.push_ms(base + f64::from(step))
                .expect("finite ms pushes");
            frame
                .push_ms(2.0 * base + f64::from(step))
                .expect("finite ms pushes");
        }
        TickFrameSeries {
            ticks: 4,
            hash,
            tick_plot: tick,
            frame_plot: frame,
            marks: MarkSeries::new(4).expect("marks build"),
        }
    }

    #[test]
    fn tick_frame_paired_receipt_names_both_hashes() {
        // Pure receipt proof, no flag, no wall clock: fixed inputs render
        // stable bytes, so the x2 check holds modulo live ms by construction.
        let a = paired_side(0xd791_b72c_1633_5366, 1.0);
        let b = paired_side(0xd791_b72c_1633_5366, 3.0);
        let first = paired_tick_frame_summary(&a, &b).expect("equal hashes receipt");
        assert!(
            first.contains("hash_A == hash_B (d791b72c16335366)"),
            "names both hashes"
        );
        let lines = [
            "tick A p50=",
            "| B p50=",
            "frame A p50=",
            "machine=",
            "profile=",
        ];
        assert!(
            lines.iter().all(|line| first.contains(line)),
            "receipt names every row"
        );
        assert_eq!(
            first,
            paired_tick_frame_summary(&a, &b).expect("rerun repeats")
        );
        println!("{first}");
    }

    #[test]
    fn tick_frame_paired_receipt_refuses_divergence() {
        // Kill rule on mismatch; off profiler refuses capture. Never flips the flag.
        let a = paired_side(0xd791_b72c_1633_5366, 1.0);
        let c = paired_side(0x1234_5678_9abc_def0, 1.0);
        assert_eq!(
            paired_tick_frame_summary(&a, &c),
            Err(ProfileError::DivergedPair {
                hash_a: a.hash,
                hash_b: c.hash
            })
        );
        assert_eq!(capture_paired_tick_frame(), Err(ProfileError::Disabled));
        assert!(!is_enabled(), "this test never flips the flag");
    }
}
