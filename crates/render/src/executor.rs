//! executor: CPU-side frame-graph executor over recorded pass fns.
//!
//! Purpose: run already-ordered passes, time each on the CPU and report
//! one receipt; plan transient aliasing; refuse GPU submit loudly on the
//! headless twin and drive the `graph.rs` submitter on window builds.
//! Owner: render lane (RA2); twin of the `graph.rs` planner, wired by RA8.
//! Units: bytes, whole ms, integer percent. Determinism: input order runs
//! as given; allocator keys are integers (size, first, name), never float.
//! Errors: `ExecError::Unsupported` names the missing GPU path on the
//! headless twin; the window submit below returns the graph submitter error
//! (which has no `Unsupported` variant: a device is in hand).
//!
//! Std only on the headless twin: zero crate imports outside the
//! window-gated submit, so this file also compiles alone via
//! `rustc --edition 2021 --test crates/render/src/executor.rs`.

use std::error::Error;
use std::fmt;
use std::time::Instant;

/// Pass index in run order.
pub type PassId = usize;

/// One transient row for the alias planner: closed lifetime
/// `[first, last]` in run-order positions plus size in bytes.
#[derive(Clone, Debug)]
pub struct TransientRow {
    /// Transient resource name.
    pub name: String,
    /// Backing size in bytes.
    pub size_bytes: u64,
    /// First run-order position using it.
    pub first: usize,
    /// Last run-order position using it.
    pub last: usize,
}

impl TransientRow {
    /// One row with a closed lifetime.
    pub fn new(name: &str, size_bytes: u64, first: usize, last: usize) -> Self {
        Self {
            name: name.to_string(),
            size_bytes,
            first,
            last,
        }
    }
}

/// One backing slot: strictly disjoint-lifetime rows sharing `size_bytes`.
#[derive(Clone, Debug)]
pub struct AliasSlot {
    /// Slot index in placement order.
    pub id: usize,
    /// Peak member size in bytes.
    pub size_bytes: u64,
    /// Member names in placement order.
    pub members: Vec<String>,
}

/// Alias plan: slots plus byte totals and the integer reuse percent
/// (bytes in shared slots over total transient bytes).
#[derive(Clone, Debug)]
pub struct AliasPlan {
    /// Backing slots in placement order.
    pub slots: Vec<AliasSlot>,
    /// Sum of row sizes in bytes (before aliasing).
    pub total_bytes: u64,
    /// Sum of slot peaks in bytes (after aliasing).
    pub backing_bytes: u64,
    /// Bytes in shared slots over total bytes, 0..=100.
    pub reuse_pct: u32,
}

/// Largest-first first-fit over strictly disjoint lifetimes.
/// Deterministic: ties break by (first, name). Strict means touching
/// (`last == first`) never shares: same-position reuse would need the
/// explicit alias barriers the compiled graph records.
pub fn plan_aliases(rows: &[TransientRow]) -> AliasPlan {
    let mut seq: Vec<usize> = (0..rows.len()).collect();
    seq.sort_by(|&a, &b| {
        rows[b]
            .size_bytes
            .cmp(&rows[a].size_bytes)
            .then_with(|| rows[a].first.cmp(&rows[b].first))
            .then_with(|| rows[a].name.cmp(&rows[b].name))
    });
    let mut slots: Vec<Vec<usize>> = Vec::new();
    for &ri in &seq {
        let mut placed = usize::MAX;
        for (si, slot) in slots.iter().enumerate() {
            let fits = slot
                .iter()
                .all(|&m| rows[m].last < rows[ri].first || rows[ri].last < rows[m].first);
            if fits {
                placed = si;
                break;
            }
        }
        if placed == usize::MAX {
            placed = slots.len();
            slots.push(Vec::new());
        }
        slots[placed].push(ri);
    }
    let mut total_bytes = 0u64;
    let mut backing_bytes = 0u64;
    let mut shared_bytes = 0u64;
    let mut out: Vec<AliasSlot> = Vec::with_capacity(slots.len());
    for (id, slot) in slots.iter().enumerate() {
        let mut peak = 0u64;
        let mut members = Vec::with_capacity(slot.len());
        for &m in slot {
            total_bytes = total_bytes.saturating_add(rows[m].size_bytes);
            peak = peak.max(rows[m].size_bytes);
            members.push(rows[m].name.clone());
        }
        backing_bytes = backing_bytes.saturating_add(peak);
        if slot.len() > 1 {
            for &m in slot {
                shared_bytes = shared_bytes.saturating_add(rows[m].size_bytes);
            }
        }
        out.push(AliasSlot {
            id,
            size_bytes: peak,
            members,
        });
    }
    let reuse_pct = if total_bytes == 0 {
        100
    } else {
        u32::try_from(u128::from(shared_bytes) * 100 / u128::from(total_bytes)).unwrap_or(100)
    };
    AliasPlan {
        slots: out,
        total_bytes,
        backing_bytes,
        reuse_pct,
    }
}

/// One pass declaration for the CPU executor.
#[derive(Clone, Debug)]
pub struct CpuPassDesc {
    /// Pass name for receipts.
    pub name: String,
    /// Draw calls recorded by this pass.
    pub draws: u64,
    /// Triangles recorded by this pass.
    pub triangles: u64,
}

impl CpuPassDesc {
    /// One pass with recorded work.
    pub fn new(name: &str, draws: u64, triangles: u64) -> Self {
        Self {
            name: name.to_string(),
            draws,
            triangles,
        }
    }
}

/// Per-pass scratch pad; extras add to the receipt.
#[derive(Debug)]
pub struct ExecCtx {
    pass: PassId,
    frame_index: u64,
    extra_draws: u64,
    extra_triangles: u64,
}

impl ExecCtx {
    /// Pass index running now.
    pub fn pass(&self) -> PassId {
        self.pass
    }
    /// Frame index running now.
    pub fn frame_index(&self) -> u64 {
        self.frame_index
    }
    /// Add draws/triangles found at record time (saturates, never wraps).
    pub fn drew(&mut self, draws: u64, triangles: u64) {
        self.extra_draws = self.extra_draws.saturating_add(draws);
        self.extra_triangles = self.extra_triangles.saturating_add(triangles);
    }
}

/// Recorded pass fn.
type PassFn = Box<dyn FnMut(&mut ExecCtx) + Send>;

/// CPU executor: runs passes in add order, times each, sums one receipt.
#[derive(Default)]
pub struct CpuExecutor {
    passes: Vec<CpuPassDesc>,
    fns: Vec<Option<PassFn>>,
}

impl fmt::Debug for CpuExecutor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CpuExecutor")
            .field("passes", &self.passes)
            .field("fn_count", &self.fns.len())
            .finish()
    }
}

impl CpuExecutor {
    /// Empty executor.
    pub fn new() -> Self {
        Self::default()
    }
    /// Add a pass with its optional record fn; returns its id.
    pub fn add_pass(&mut self, desc: CpuPassDesc, f: Option<PassFn>) -> PassId {
        let id = self.passes.len();
        self.passes.push(desc);
        self.fns.push(f);
        id
    }
    /// Pass count.
    pub fn pass_count(&self) -> usize {
        self.passes.len()
    }
    /// Run every pass in add order; time each in whole ms.
    pub fn run(&mut self, frame_index: u64) -> ExecReceipt {
        let mut receipt = ExecReceipt {
            frame_index,
            draws: 0,
            triangles: 0,
            per_pass_cpu_ms: Vec::with_capacity(self.passes.len()),
            total_cpu_ms: 0,
        };
        for id in 0..self.passes.len() {
            let started = Instant::now();
            let mut ctx = ExecCtx {
                pass: id,
                frame_index,
                extra_draws: 0,
                extra_triangles: 0,
            };
            if let Some(f) = self.fns[id].as_mut() {
                f(&mut ctx);
            }
            let desc = &self.passes[id];
            receipt.draws = receipt
                .draws
                .saturating_add(desc.draws)
                .saturating_add(ctx.extra_draws);
            receipt.triangles = receipt
                .triangles
                .saturating_add(desc.triangles)
                .saturating_add(ctx.extra_triangles);
            let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            receipt.total_cpu_ms = receipt.total_cpu_ms.saturating_add(ms);
            receipt.per_pass_cpu_ms.push(ExecPassTiming {
                pass: desc.name.clone(),
                cpu_ms: ms,
                // No device on the CPU leg: always None here, resolved later
                // from the window QuerySet receipt via resolve_gpu_ms only.
                gpu_ms: None,
            });
        }
        receipt
    }
}

/// Per-pass CPU timing in whole ms, plus GPU timestamp scaffolding.
///
/// PF2 pattern: CPU ms is always filled; `gpu_ms` stays `None` until the
/// window `QuerySet` submit resolves it via [`ExecPassTiming::resolve_gpu`]
/// (wired by [`ExecReceipt::resolve_gpu_ms`]).
/// Units: `cpu_ms` whole ms, `gpu_ms` fractional ms once resolved.
#[derive(Clone, Debug)]
pub struct ExecPassTiming {
    /// Pass name.
    pub pass: String,
    /// Wall time spent running this pass, in whole ms.
    pub cpu_ms: u64,
    /// GPU time for this pass in ms; `None` until Q1 device timestamps
    /// exist. Never guessed: only [`ExecPassTiming::resolve_gpu`] sets it.
    pub gpu_ms: Option<f64>,
}

impl ExecPassTiming {
    /// One timing row with CPU ms filled and GPU ms pending (`None`).
    /// Timings are evidence only, never hashed.
    pub fn new(pass: &str, cpu_ms: u64) -> Self {
        Self {
            pass: pass.to_string(),
            cpu_ms,
            gpu_ms: None,
        }
    }

    /// Attach a device-measured GPU timestamp (Q1 path), refusing
    /// non-finite or negative ms loudly instead of storing a NaN.
    pub fn resolve_gpu(&mut self, ms: f64) -> Result<(), ExecError> {
        if !ms.is_finite() || ms < 0.0 {
            return Err(ExecError::BadTimestamp(format!(
                "gpu timestamp for pass '{}' must be finite and >= 0, got {ms}",
                self.pass
            )));
        }
        self.gpu_ms = Some(ms);
        Ok(())
    }
}

/// One executed frame from the CPU executor.
///
/// Receipt-shape reconciliation (RA8 documents, never renames):
/// - `graph::FrameReceipt` is the compiled-graph receipt: draws,
///   triangles, per-pass CPU ms plus transient aliasing bytes/pct.
/// - `present::FrameReceipt` is the presenter receipt: resolution,
///   draws, validation errors, adapter/driver plus cache-budget rows.
/// - This `executor::ExecReceipt` is the CPU-executor receipt: draws,
///   triangles, per-pass CPU ms plus per-pass GPU timestamps (`None` until
///   the window `QuerySet` submit resolves them via [`ExecReceipt::resolve_gpu_ms`])
///   feeding capture reports; [`ExecReceipt::costliest_pass`]
///   names the costliest pass for the capture report.
#[derive(Clone, Debug)]
pub struct ExecReceipt {
    /// Frame index executed.
    pub frame_index: u64,
    /// Draw calls: declared work plus fn extras.
    pub draws: u64,
    /// Triangles: declared work plus fn extras.
    pub triangles: u64,
    /// CPU ms per pass, in run order.
    pub per_pass_cpu_ms: Vec<ExecPassTiming>,
    /// Sum of per-pass CPU ms.
    pub total_cpu_ms: u64,
}

impl ExecReceipt {
    /// Attach window per-pass GPU ms (compiled order) via
    /// [`ExecPassTiming::resolve_gpu`]; `None` entries stay unresolved so a
    /// fallback device degrades to `None` instead of failing. Extra entries
    /// are ignored, missing entries leave the tail unresolved. Timings are
    /// evidence only, never hashed.
    pub fn resolve_gpu_ms(&mut self, gpu_ms: &[Option<f64>]) -> Result<(), ExecError> {
        for (timing, sample) in self.per_pass_cpu_ms.iter_mut().zip(gpu_ms.iter()) {
            if let Some(ms) = sample {
                timing.resolve_gpu(*ms)?;
            }
        }
        Ok(())
    }
    /// Name of the costliest pass by CPU ms (first wins ties, so the
    /// verdict is deterministic); `None` when no pass ran. GPU ms stays
    /// out of the verdict until Q1 resolves it on every pass.
    #[must_use]
    pub fn costliest_pass(&self) -> Option<&str> {
        let mut best: Option<&ExecPassTiming> = None;
        for timing in &self.per_pass_cpu_ms {
            match &best {
                None => best = Some(timing),
                Some(current) if timing.cpu_ms > current.cpu_ms => best = Some(timing),
                Some(_) => {}
            }
        }
        best.map(|timing| timing.pass.as_str())
    }
}

// Scoped GPU timers, idea from Wumpf/wgpu-profiler (MIT OR Apache-2.0,
// https://github.com/Wumpf/wgpu-profiler): scope.rs/profiler.rs/
// profiler_query.rs size one query pair per scope, resolve + read back,
// roll up per-scope ms. Fresh impl below; pixels unchanged.
/// Open scope guard: begin tick held until `end` posts ms. Pure math.
pub struct GpuScope {
    /// Pass index in run order.
    pub pass: usize,
    begin: u64,
}
impl GpuScope {
    /// Open scope `pass` at `begin` tick.
    pub fn begin(pass: usize, begin: u64) -> Self {
        Self { pass, begin }
    }
    /// Close at `end` tick via queue period (ns/tick); inverted or bad
    /// period degrades to `None`, never fails pixels.
    pub fn end(self, end: u64, period_ns: f32) -> (usize, Option<f64>) {
        (self.pass, scope_ms(self.begin, end, period_ns))
    }
}
/// Scope delta to ms; `None` on inverted ticks or bad period. Pure.
fn scope_ms(begin: u64, end: u64, period_ns: f32) -> Option<f64> {
    if end < begin || !period_ns.is_finite() || period_ns <= 0.0 {
        return None;
    }
    let d = end.saturating_sub(begin);
    let hi = u32::try_from(d >> 32).unwrap_or(u32::MAX);
    let lo = u32::try_from(d & 0xffff_ffff).unwrap_or(u32::MAX);
    let ms = (f64::from(hi) * 4_294_967_296.0 + f64::from(lo)) * f64::from(period_ns) / 1_000_000.0;
    if ms.is_finite() && ms >= 0.0 {
        Some(ms)
    } else {
        None
    }
}
/// Roll scope pairs into `receipt` gpu ms; `None` legs degrade to `None`
/// (map/readback failure path), never fail pixels.
pub fn rollup_gpu_scopes(
    receipt: &mut ExecReceipt,
    scopes: &[(Option<u64>, Option<u64>)],
    period_ns: f32,
) {
    for (timing, scope) in receipt.per_pass_cpu_ms.iter_mut().zip(scopes.iter()) {
        if let (Some(begin), Some(end)) = *scope {
            if let Some(value) = scope_ms(begin, end, period_ns) {
                let _ = timing.resolve_gpu(value);
            }
        }
    }
}

/// Typed executor failures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecError {
    /// GPU submit without a device or bind groups.
    Unsupported(String),
    /// GPU timestamp that is non-finite or negative.
    BadTimestamp(String),
}

impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(detail) => write!(f, "exec unsupported: {detail}"),
            Self::BadTimestamp(detail) => write!(f, "exec bad timestamp: {detail}"),
        }
    }
}

impl Error for ExecError {}

/// GPU submit (headless twin): typed refusal, never a silent pretend success.
pub fn gpu_submit() -> Result<(), ExecError> {
    Err(ExecError::Unsupported(
        "gpu submit needs a wgpu device plus RL1/RL2/RP1 bind groups; the CPU executor records only"
            .to_string(), // alloc-ok: cold headless-twin refusal, never a steady frame
    ))
}

// ----- Window timestamp pairs (one owner: this module) -----
// One timestamp pair per compiled pass (`2 * pos`, `2 * pos + 1`) in a
// `2 * passes` u64 QuerySet buffer, scaled by the queue timestamp period
// to ms. The `graph` window submit writes the `timestamp_writes` arg and
// posts `per_pass_gpu_ms`; [`ExecReceipt::resolve_gpu_ms`] wires those
// into [`ExecPassTiming`]. Missing support never fails startup: the submit
// records without queries and posts `None` per pass. Units: ticks, ns per
// tick, ms. Evidence only, never hashed.

// The pass executor's own tests still call these; production code in this file
// does not, so the glob is test-only (graph calls `crate::timing` directly).
#[cfg(test)]
pub(crate) use crate::present::timing::*;

// ----- GX1: window submit through the graph submitter (one encode owner) -----
// The encode (pass pipeline, look bind groups, RA2-aliased slot targets)
// lives in `graph.rs`; this method builds the executor pass list into that
// same compiled shape and drives it, so WIN1 and ED1 run one graph on both
// legs. Budget and steady-state story are the graph submitter's (one owner:
// `graph::GPU_SUBMIT_BUDGET_MS` plus its reuse test); this path adds no new
// budget and no new error variant.

/// Cold refusal: every transient span must lie inside the compiled pass
/// range and be ordered (its `first`/`last` index this list directly).
#[cfg(feature = "gpu")]
fn check_transient_span(
    row: &TransientRow,
    count: usize,
) -> Result<(), crate::graph::GpuSubmitError> {
    if row.first >= count || row.last >= count || row.first > row.last {
        let (f, l) = (row.first, row.last);
        let name = &row.name;
        let reason = format!("transient `{name}` span [{f}, {l}] misses the {count}-pass range"); // alloc-ok: cold refusal
        return Err(crate::graph::GpuSubmitError::Invalid(reason));
    }
    Ok(())
}

/// Window submit over the executor pass list (GX1).
#[cfg(feature = "gpu")]
impl CpuExecutor {
    /// Record one wgpu pass per executor pass in add order with the shared
    /// look bind group, aliasing `transients` by the RA2 plan through the
    /// graph submitter; submit once and read back the last target. Add order
    /// is the run order (the CPU twin runs the same list), so transient
    /// `first`/`last` positions index this list directly. Fails loudly via
    /// the graph submitter error, which never reports `Unsupported`.
    pub fn submit_gpu_window(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &crate::gtao::LookFrame,
        submitter: &mut crate::graph::GpuSubmitter,
        transients: &[TransientRow],
    ) -> Result<crate::graph::GpuFrameReceipt, crate::graph::GpuSubmitError> {
        use crate::graph::{FrameGraph, GpuSubmitError, PassDesc};
        let count = self.passes.len();
        for row in transients {
            check_transient_span(row, count)?;
        }
        let mut graph = FrameGraph::new();
        for (index, pass) in self.passes.iter().enumerate() {
            let mut desc = PassDesc::new(&pass.name).work(pass.draws, pass.triangles);
            for row in transients {
                if row.first == index {
                    let first = match u32::try_from(row.first) {
                        Ok(id) => id,
                        Err(_) => {
                            let reason = format!("pass id {} exceeds u32", row.first); // alloc-ok: cold refusal, never a steady frame
                            return Err(GpuSubmitError::Invalid(reason));
                        }
                    };
                    let last = match u32::try_from(row.last) {
                        Ok(id) => id,
                        Err(_) => {
                            let reason = format!("pass id {} exceeds u32", row.last); // alloc-ok: cold refusal, never a steady frame
                            return Err(GpuSubmitError::Invalid(reason));
                        }
                    };
                    desc = desc.transient_span(&row.name, row.size_bytes, first, last);
                }
            }
            graph.add_pass(desc);
        }
        for index in 1..count {
            graph.add_edge(index - 1, index).map_err(|e| {
                let reason = format!("linear chain: {e}"); // alloc-ok: cold refusal
                GpuSubmitError::Invalid(reason)
            })?;
        }
        let compiled = graph
            .compile()
            .map_err(|e| GpuSubmitError::Invalid(format!("compile: {e}")))?; // alloc-ok: cold refusal, never a steady frame
        compiled.submit_gpu(device, queue, frame, submitter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    const HDR: u64 = 16_588_800;
    const DEPTH: u64 = 8_294_400;
    const RESOLVE: u64 = 8_294_400;
    const CASCADE: u64 = 4_194_304;
    const CLUSTER: u64 = 783_360;
    const DECAL: u64 = 2_097_152;
    const BLOOM: [u64; 4] = [4_147_200, 1_036_800, 259_200, 65_280];

    fn arena_rows() -> Vec<TransientRow> {
        vec![
            TransientRow::new("hdr", HDR, 1, 3),
            TransientRow::new("depth", DEPTH, 1, 2),
            TransientRow::new("resolve_scratch", RESOLVE, 3, 3),
            TransientRow::new("cascade_scratch", CASCADE, 0, 0),
            TransientRow::new("bloom_mip0", BLOOM[0], 4, 4),
            TransientRow::new("decal_tile", DECAL, 2, 2),
            TransientRow::new("bloom_mip1", BLOOM[1], 4, 4),
            TransientRow::new("cluster_grid", CLUSTER, 1, 1),
            TransientRow::new("bloom_mip2", BLOOM[2], 4, 4),
            TransientRow::new("bloom_mip3", BLOOM[3], 4, 4),
        ]
    }

    #[test]
    fn runs_in_add_order_and_sums_work() {
        let mut exec = CpuExecutor::new();
        let seen: Arc<Mutex<Vec<usize>>> = Default::default();
        for (i, name) in ["shadow", "forward", "post"].iter().enumerate() {
            let seen_each = Arc::clone(&seen);
            let f: PassFn = Box::new(move |ctx: &mut ExecCtx| {
                seen_each.lock().expect("test lock").push(ctx.pass());
                ctx.drew(1, 100);
            });
            assert_eq!(exec.add_pass(CpuPassDesc::new(name, 10, 1_000), Some(f)), i);
        }
        let receipt = exec.run(3);
        assert_eq!(*seen.lock().expect("test lock"), vec![0, 1, 2]);
        assert_eq!(
            (receipt.frame_index, receipt.draws, receipt.triangles),
            (3, 33, 3_300)
        );
        assert_eq!(receipt.per_pass_cpu_ms.len(), 3);
        assert_eq!(
            receipt.total_cpu_ms,
            receipt
                .per_pass_cpu_ms
                .iter()
                .map(|t| t.cpu_ms)
                .sum::<u64>()
        );
    }

    #[test]
    fn pass_without_fn_still_reports_declared_work() {
        let mut exec = CpuExecutor::new();
        exec.add_pass(CpuPassDesc::new("present", 1, 1), None);
        let receipt = exec.run(0);
        assert_eq!((receipt.draws, receipt.triangles), (1, 1));
    }

    #[test]
    fn gpu_submit_refuses_loudly() {
        match gpu_submit() {
            Err(ExecError::Unsupported(detail)) => {
                assert!(!detail.is_empty(), "never silent");
                assert!(detail.contains("wgpu"), "names the missing path: {detail}");
            }
            Ok(()) => panic!("a stub submit must never report success"),
            Err(ExecError::BadTimestamp(detail)) => {
                panic!("submit refuses via Unsupported, not timestamps: {detail}")
            }
        }
    }

    #[test]
    fn arena_plan_hits_the_reuse_gate() {
        let plan = plan_aliases(&arena_rows());
        assert!(
            plan.reuse_pct >= 80,
            "reuse {}% below the gate",
            plan.reuse_pct
        );
        let expect_total =
            HDR + DEPTH + RESOLVE + CASCADE + CLUSTER + DECAL + BLOOM.iter().sum::<u64>();
        assert_eq!(plan.total_bytes, expect_total);
        // Peaks mirror the graph twin: HDR, depth/resolve, decal/cluster,
        // plus the smallest bloom mip alone (same-pass mips overlap).
        assert_eq!(plan.backing_bytes, HDR + DEPTH + DECAL + BLOOM[3]);
        // HDR aliases with bloom scratch under strict disjointness.
        let slot_of = |name: &str| {
            plan.slots
                .iter()
                .find(|s| s.members.iter().any(|m| m == name))
                .map(|s| s.id)
        };
        assert_eq!(slot_of("hdr"), slot_of("bloom_mip0"));
        assert_ne!(slot_of("hdr"), slot_of("depth"));
    }

    #[test]
    fn touching_lifetimes_never_share() {
        let rows = vec![
            TransientRow::new("a", 1_000, 0, 1),
            TransientRow::new("b", 1_000, 1, 2),
        ];
        let plan = plan_aliases(&rows);
        assert_eq!(plan.slots.len(), 2, "touching rows need separate slots");
        assert_eq!((plan.total_bytes, plan.backing_bytes), (2_000, 2_000));
        assert_eq!(plan.reuse_pct, 0);
    }

    #[test]
    fn empty_plan_and_run_are_zero() {
        let plan = plan_aliases(&[]);
        assert_eq!((plan.total_bytes, plan.backing_bytes), (0, 0));
        assert_eq!(plan.reuse_pct, 100);
        let mut exec = CpuExecutor::new();
        let receipt = exec.run(0);
        assert_eq!(
            (receipt.draws, receipt.triangles, receipt.total_cpu_ms),
            (0, 0, 0)
        );
        assert!(receipt.per_pass_cpu_ms.is_empty());
        assert_eq!(receipt.costliest_pass(), None);
    }

    #[test]
    fn gpu_timestamps_start_none_until_q1() {
        let mut exec = CpuExecutor::new();
        exec.add_pass(CpuPassDesc::new("shadow", 2, 200), None);
        exec.add_pass(CpuPassDesc::new("forward", 5, 500), None);
        let receipt = exec.run(7);
        assert_eq!(receipt.per_pass_cpu_ms.len(), 2);
        for timing in &receipt.per_pass_cpu_ms {
            assert_eq!(timing.gpu_ms, None, "no device timestamps before Q1");
        }
    }

    #[test]
    fn costliest_pass_names_max_cpu_ms_first_wins_ties() {
        let receipt = ExecReceipt {
            frame_index: 1,
            draws: 0,
            triangles: 0,
            per_pass_cpu_ms: vec![
                ExecPassTiming::new("shadow", 3),
                ExecPassTiming::new("forward", 9),
                ExecPassTiming::new("post", 4),
            ],
            total_cpu_ms: 16,
        };
        assert_eq!(receipt.costliest_pass(), Some("forward"));
        let tied = ExecReceipt {
            frame_index: 2,
            draws: 0,
            triangles: 0,
            per_pass_cpu_ms: vec![ExecPassTiming::new("a", 5), ExecPassTiming::new("b", 5)],
            total_cpu_ms: 10,
        };
        assert_eq!(tied.costliest_pass(), Some("a"));
    }

    #[test]
    fn resolve_gpu_fills_only_with_finite_ms() {
        let mut timing = ExecPassTiming::new("forward", 2);
        timing.resolve_gpu(1.75).expect("finite ms resolves");
        assert_eq!(timing.gpu_ms, Some(1.75));
        for bad in [f64::NAN, f64::INFINITY, -0.5] {
            match timing.resolve_gpu(bad) {
                Err(ExecError::BadTimestamp(detail)) => {
                    assert!(detail.contains("forward"), "names the pass: {detail}");
                }
                Ok(()) => panic!("non-finite gpu ms must never store"),
                Err(other) => panic!("wrong error: {other:?}"),
            }
        }
        // Failed resolves never clobber the good sample.
        assert_eq!(timing.gpu_ms, Some(1.75));
    }

    #[test]
    fn resolve_gpu_ms_wires_query_pairs_and_degrades() {
        let mut receipt = ExecReceipt {
            frame_index: 1,
            draws: 0,
            triangles: 0,
            per_pass_cpu_ms: vec![
                ExecPassTiming::new("shadow", 3),
                ExecPassTiming::new("forward", 9),
            ],
            total_cpu_ms: 12,
        };
        receipt
            .resolve_gpu_ms(&[Some(0.25), None])
            .expect("finite samples resolve");
        assert_eq!(receipt.per_pass_cpu_ms[0].gpu_ms, Some(0.25));
        assert_eq!(receipt.per_pass_cpu_ms[1].gpu_ms, None);
        // Short slices leave the tail unresolved; long slices ignore extras.
        receipt
            .resolve_gpu_ms(&[Some(0.5)])
            .expect("short slice resolves its head");
        assert_eq!(receipt.per_pass_cpu_ms[0].gpu_ms, Some(0.5));
        assert_eq!(receipt.per_pass_cpu_ms[1].gpu_ms, None);
        receipt
            .resolve_gpu_ms(&[Some(0.5), Some(1.5), Some(9.0)])
            .expect("extra entries are ignored");
        assert_eq!(receipt.per_pass_cpu_ms[1].gpu_ms, Some(1.5));
        // Bad samples refuse loudly and keep the prior good sample.
        match receipt.resolve_gpu_ms(&[Some(f64::NAN), Some(2.0)]) {
            Err(ExecError::BadTimestamp(detail)) => {
                assert!(detail.contains("shadow"), "names the pass: {detail}");
            }
            other => panic!("non-finite gpu ms must fail loudly, got {other:?}"),
        }
        assert_eq!(receipt.per_pass_cpu_ms[0].gpu_ms, Some(0.5));
    }

    #[test]
    fn scoped_rollup_posts_per_pass_gpu_ms() {
        let mut exec = CpuExecutor::new();
        exec.add_pass(CpuPassDesc::new("shadow", 1, 1), None);
        exec.add_pass(CpuPassDesc::new("forward", 1, 1), None);
        let mut receipt = exec.run(11);
        let none_before = receipt
            .per_pass_cpu_ms
            .iter()
            .filter(|t| t.gpu_ms.is_none())
            .count();
        assert_eq!(none_before, 2);
        let some_before = receipt.per_pass_cpu_ms.len() - none_before;
        let (pass, ms) = GpuScope::begin(0, 10).end(10 + 1_000_000, 1.0);
        assert_eq!((pass, ms), (0, Some(1.0)));
        assert_eq!(
            GpuScope::begin(1, 5).end(3, 1.0).1,
            None,
            "inverted ticks degrade"
        );
        rollup_gpu_scopes(
            &mut receipt,
            &[(Some(10), Some(10 + 1_000_000)), (None, None)],
            1.0,
        );
        assert_eq!(receipt.per_pass_cpu_ms[0].gpu_ms, Some(1.0));
        assert_eq!(receipt.per_pass_cpu_ms[1].gpu_ms, None);
        let some_after = receipt
            .per_pass_cpu_ms
            .iter()
            .filter(|t| t.gpu_ms.is_some())
            .count();
        assert_eq!(some_after, 1);
        eprintln!(
            "scoped rollup: passes={} pass={} gpu_ms None->Some {some_before}->{some_after}",
            receipt.per_pass_cpu_ms.len(),
            receipt.per_pass_cpu_ms[0].pass,
        );
    }

    // ----- GX1 window tests (behind `window`; skip loudly without a device) -----

    /// Window device for the submit tests: shared with the graph submit
    /// tests (one device for the suite, so parallel tests never exhaust
    /// adapters). `Err` is skip-not-green, never a fake pass: each test hands
    /// it to `require_or_skip`, which names the test in its SKIPPED line.
    #[cfg(feature = "gpu")]
    fn exec_device() -> Result<
        (
            &'static wgpu::Device,
            &'static wgpu::Queue,
            &'static wgpu::AdapterInfo,
        ),
        crate::test_gpu::NoAdapter,
    > {
        crate::graph::gx1_probe()
    }

    #[cfg(feature = "gpu")]
    fn exec_frame(index: u64) -> crate::gtao::LookFrame {
        crate::graph::gx1_test_frame(index)
    }

    /// Four-pass executor plus aliasing rows: cascade at 0, HDR and depth
    /// overlapping at 1, scratch spanning 2..=3. Largest-first packs two
    /// backing slots ({hdr, cascade}, {depth, scratch}), so every byte is
    /// shared and the submit proves aliasing structurally.
    #[cfg(feature = "gpu")]
    fn exec_four() -> (CpuExecutor, Vec<TransientRow>) {
        let mut exec = CpuExecutor::new();
        exec.add_pass(CpuPassDesc::new("shadow", 64, 500_000), None);
        exec.add_pass(CpuPassDesc::new("forward", 256, 1_800_000), None);
        exec.add_pass(CpuPassDesc::new("decals", 48, 96_000), None);
        exec.add_pass(CpuPassDesc::new("resolve", 2, 2), None);
        let rows = vec![
            TransientRow::new("cascade_scratch", 4_194_304, 0, 0),
            TransientRow::new("hdr", 16_588_800, 1, 2),
            TransientRow::new("depth", 8_294_400, 1, 1),
            TransientRow::new("scratch", 2_097_152, 2, 3),
        ];
        (exec, rows)
    }

    #[test]
    #[cfg(feature = "gpu")]
    fn exec_window_submit_runs_passes_with_aliasing_clean() {
        let Some((device, queue, info)) = crate::test_gpu::require_or_skip(
            "exec_window_submit_runs_passes_with_aliasing_clean",
            exec_device(),
        ) else {
            return;
        };
        let (exec, rows) = exec_four();
        assert_eq!(exec.pass_count(), 4, "forward plus 3 graph passes");
        let mut submitter = crate::graph::GpuSubmitter::new(device);
        let receipt = exec
            .submit_gpu_window(device, queue, &exec_frame(21), &mut submitter, &rows)
            .expect("executor window submit runs clean");
        assert_eq!(receipt.passes_submitted, 4);
        assert!(
            receipt.is_clean(),
            "0 validation errors, got {}",
            receipt.validation_errors
        );
        assert!(receipt.covered_pixels > 0, "passes must write pixels");
        assert_eq!(
            receipt.transient_slots, 2,
            "two backing slots hold four rows: {receipt:?}"
        );
        assert!(
            receipt.transient_slots < rows.len(),
            "aliasing shares slots across disjoint lifetimes"
        );
        assert!(
            receipt.transient_reuse_pct >= 80,
            "reuse {}% below the 80% gate",
            receipt.transient_reuse_pct
        );
        assert!(receipt.within_transient_budget());
        assert_eq!((receipt.draws, receipt.triangles), (370, 2_396_002));
        eprintln!(
            "GX1 executor receipt on {}: passes={} draws={} slots={} reuse={}% covered={} submit_ms={:.2}",
            info.name,
            receipt.passes_submitted,
            receipt.draws,
            receipt.transient_slots,
            receipt.transient_reuse_pct,
            receipt.covered_pixels,
            receipt.submit_ms
        );
    }

    #[test]
    #[cfg(feature = "gpu")]
    fn exec_twin_matches_cpu_draws() {
        let Some((device, queue, _)) =
            crate::test_gpu::require_or_skip("exec_twin_matches_cpu_draws", exec_device())
        else {
            return;
        };
        let (mut exec, rows) = exec_four();
        let cpu = exec.run(21);
        let mut submitter = crate::graph::GpuSubmitter::new(device);
        let gpu = exec
            .submit_gpu_window(device, queue, &exec_frame(21), &mut submitter, &rows)
            .expect("gpu leg runs");
        assert_eq!(gpu.draws, cpu.draws, "twin runs the same pass list");
        assert_eq!(gpu.triangles, cpu.triangles, "twin triangles match");
        assert_eq!(gpu.passes_submitted, cpu.per_pass_cpu_ms.len());
        // Second submit reuses the graph submitter caches: no new targets or
        // bind groups on matching shape (steady-state proof for this path).
        let shape = (submitter.slot_texture_count(), submitter.bind_group_count());
        let again = exec
            .submit_gpu_window(device, queue, &exec_frame(22), &mut submitter, &rows)
            .expect("second submits");
        assert_eq!(
            (submitter.slot_texture_count(), submitter.bind_group_count()),
            shape,
            "steady-state submit creates no new targets or bind groups"
        );
        assert!(again.is_clean() && again.covered_pixels > 0);
    }

    #[test]
    #[cfg(feature = "gpu")]
    fn exec_window_rejects_bad_span_before_sizing() {
        let Some((device, queue, _)) = crate::test_gpu::require_or_skip(
            "exec_window_rejects_bad_span_before_sizing",
            exec_device(),
        ) else {
            return;
        };
        let (exec, _) = exec_four();
        let mut submitter = crate::graph::GpuSubmitter::new(device);
        // Row lifetime escapes the 4-pass range: the window method refuses
        // before compiling or sizing any target.
        let bad = vec![TransientRow::new("stale", 1024, 0, 9)];
        match exec.submit_gpu_window(device, queue, &exec_frame(21), &mut submitter, &bad) {
            Err(crate::graph::GpuSubmitError::Invalid(detail)) => {
                assert!(detail.contains("stale"), "names the row: {detail}");
            }
            other => panic!("out-of-range span must fail Invalid, got {other:?}"),
        }
        assert_eq!(
            (submitter.slot_texture_count(), submitter.bind_group_count()),
            (0, 0),
            "refused submits size nothing"
        );
    }

    #[test]
    #[cfg(feature = "gpu")]
    fn exec_invalid_frame_never_reports_unsupported() {
        let Some((device, queue, _)) = crate::test_gpu::require_or_skip(
            "exec_invalid_frame_never_reports_unsupported",
            exec_device(),
        ) else {
            return;
        };
        let (exec, rows) = exec_four();
        let mut submitter = crate::graph::GpuSubmitter::new(device);
        let bad = crate::gtao::LookFrame {
            viewport_px: [0, 64],
            ..exec_frame(21)
        };
        match exec.submit_gpu_window(device, queue, &bad, &mut submitter, &rows) {
            Err(crate::graph::GpuSubmitError::Invalid(detail)) => {
                assert!(!detail.is_empty(), "never silent");
                assert!(
                    !detail.contains("unsupported"),
                    "window path never reports Unsupported: {detail}"
                );
            }
            other => panic!("zero viewport must fail Invalid, got {other:?}"),
        }
    }

    #[test]
    #[cfg(feature = "gpu")]
    fn exec_window_timings_resolve_from_query_receipt() {
        let Some((device, queue, _)) = crate::test_gpu::require_or_skip(
            "exec_window_timings_resolve_from_query_receipt",
            exec_device(),
        ) else {
            return;
        };
        let (mut exec, rows) = exec_four();
        let cpu = exec.run(21);
        let mut submitter = crate::graph::GpuSubmitter::new(device);
        let gpu = exec
            .submit_gpu_window(device, queue, &exec_frame(21), &mut submitter, &rows)
            .expect("gpu leg runs");
        assert!(gpu.is_clean(), "no validation error with timestamps");
        assert_eq!(gpu.per_pass_gpu_ms.len(), gpu.passes_submitted);
        assert_eq!(gpu.passes_submitted, cpu.per_pass_cpu_ms.len());
        let mut wired = cpu;
        wired
            .resolve_gpu_ms(&gpu.per_pass_gpu_ms)
            .expect("device samples are finite");
        assert_eq!(wired.per_pass_cpu_ms.len(), gpu.passes_submitted);
        if timestamp_supported(device) {
            assert!(
                wired.per_pass_cpu_ms.iter().all(|timing| {
                    matches!(timing.gpu_ms, Some(ms) if ms.is_finite() && ms >= 0.0)
                }),
                "reference GPU wires Some per pass: {wired:?}"
            );
        } else {
            assert!(
                wired
                    .per_pass_cpu_ms
                    .iter()
                    .all(|timing| timing.gpu_ms.is_none()),
                "fallback wires None per pass: {wired:?}"
            );
        }
    }

    // ----- PF25 pass timings (row `pass_timing`): resolution math + receipt -----
    // One timestamp pair per compiled pass. The resolution helpers are pure,
    // so the math test below runs on the headless twin; only the device
    // receipt needs `window` (and a real adapter).

    /// Resolution math: ticks to ms through the queue period, an inverted
    /// pair reading zero, and a missing timing staying `None` (never a fake
    /// zero). Runs headless: the helpers use no wgpu type.
    #[test]
    fn pass_timing_math_reads_ticks_and_keeps_missing_none() {
        // 1e6 ns is one ms; the queue period is ns per tick.
        assert_eq!(ticks_to_ms(0, 1.0), 0.0);
        assert!((ticks_to_ms(1_000_000, 1.0) - 1.0).abs() < 1e-9);
        assert!((ticks_to_ms(2_500, 2.0) - 0.005).abs() < 1e-9);
        // The hi/lo split keeps a wide delta finite instead of saturating.
        let wide = u64::MAX >> 1;
        let ms = ticks_to_ms(wide, 1.0);
        assert!(ms.is_finite() && ms > 0.0, "wide delta stays finite: {ms}");
        // A pass reads end-minus-begin; an inverted or empty pair reads zero.
        assert!((pass_ms_from_ticks(10, 11_010, 1.0) - 0.011).abs() < 1e-9);
        assert_eq!(
            pass_ms_from_ticks(50, 20, 1.0),
            0.0,
            "inverted pair is zero"
        );
        assert_eq!(pass_ms_from_ticks(7, 7, 1.0), 0.0, "empty pair is zero");
        // A missing timing stays None: the all-None submit and a short tail.
        assert_eq!(none_gpu_timings(3), vec![None, None, None]);
        let mut bytes = Vec::with_capacity(16);
        bytes.extend_from_slice(&100u64.to_le_bytes());
        bytes.extend_from_slice(&1_100u64.to_le_bytes());
        let rows = decode_pass_bytes(&bytes, 2, 1.0);
        assert_eq!(rows.len(), 2);
        assert!(
            rows[0].is_some_and(|sample| (sample - 0.001).abs() < 1e-9),
            "a full pair resolves"
        );
        assert_eq!(rows[1], None, "a missing pair stays None, never zero");
        // resolve_gpu_ms ignores extras and leaves a None sample unresolved.
        let mut receipt = ExecReceipt {
            frame_index: 0,
            draws: 0,
            triangles: 0,
            per_pass_cpu_ms: vec![ExecPassTiming::new("a", 1), ExecPassTiming::new("b", 1)],
            total_cpu_ms: 2,
        };
        receipt
            .resolve_gpu_ms(&[Some(0.25), None])
            .expect("finite sample resolves");
        assert_eq!(receipt.per_pass_cpu_ms[0].gpu_ms, Some(0.25));
        assert_eq!(
            receipt.per_pass_cpu_ms[1].gpu_ms, None,
            "missing stays None"
        );
    }

    /// Nearest-rank percentile of already-sorted samples; 0.0 when empty.
    #[cfg(feature = "gpu")]
    fn percentile(sorted: &[f64], rank: usize) -> f64 {
        if sorted.is_empty() {
            return 0.0;
        }
        let n = sorted.len();
        let idx = (n * rank).div_ceil(100).max(1) - 1;
        sorted[idx.min(n - 1)]
    }

    /// Device receipt: submit the arena graph many frames on the real adapter,
    /// read the QuerySet back and print per-pass p50/p99 GPU ms plus the frame
    /// total, naming the slowest pass. Without `TIMESTAMP_QUERY` the samples
    /// stay `None` and the test says so instead of printing a fake number.
    #[cfg(feature = "gpu")]
    #[test]
    fn pass_timing_gpu_per_pass_p50_over_120_frames() {
        const FRAMES: u64 = 120;
        let Some((device, queue, info)) = crate::test_gpu::require_or_skip(
            "pass_timing_gpu_per_pass_p50_over_120_frames",
            crate::graph::gx1_probe(),
        ) else {
            return;
        };
        let supported = timestamp_supported(device);
        eprintln!(
            "pass_timing gpu: adapter='{}' backend={:?} driver='{} {}' TIMESTAMP_QUERY={supported} period_ns={}",
            info.name,
            info.backend,
            info.driver,
            info.driver_info,
            queue.get_timestamp_period()
        );
        let plan = crate::graph::arena_frame_graph()
            .compile()
            .expect("arena graph is acyclic");
        let names: Vec<String> = plan
            .order()
            .iter()
            .filter_map(|id| plan.pass(*id).map(|desc| desc.name.clone()))
            .collect();
        assert_eq!(
            names.len(),
            plan.order().len(),
            "every ordered pass has a name"
        );
        let mut submitter = crate::graph::GpuSubmitter::new(device);
        let mut samples: Vec<Vec<f64>> = vec![Vec::new(); names.len()];
        let mut totals: Vec<f64> = Vec::with_capacity(usize::try_from(FRAMES).unwrap_or(0));
        for index in 0..FRAMES {
            let receipt = plan
                .submit_gpu(
                    device,
                    queue,
                    &crate::graph::gx1_test_frame(index),
                    &mut submitter,
                )
                .expect("timestamp submit runs");
            assert!(receipt.is_clean(), "no validation error with timestamps");
            assert_eq!(
                receipt.per_pass_gpu_ms.len(),
                names.len(),
                "one sample per compiled pass"
            );
            let mut total = 0.0;
            for (pos, sample) in receipt.per_pass_gpu_ms.iter().enumerate() {
                if let (Some(ms), Some(rows)) = (sample, samples.get_mut(pos)) {
                    assert!(ms.is_finite() && *ms >= 0.0, "device ms is finite");
                    rows.push(*ms);
                    total += *ms;
                }
            }
            totals.push(total);
        }
        if supported {
            for (pos, rows) in samples.iter().enumerate() {
                assert_eq!(
                    rows.len(),
                    usize::try_from(FRAMES).unwrap_or(0),
                    "pass {pos} resolved every frame"
                );
            }
        } else {
            assert!(
                samples.iter().all(Vec::is_empty),
                "no TIMESTAMP_QUERY: every per-pass sample stays None"
            );
            eprintln!(
                "pass_timing gpu: adapter lacks TIMESTAMP_QUERY; per-pass ms stays None (no fake numbers)"
            );
        }
        let mut ranked: Vec<(String, f64, f64)> = Vec::with_capacity(names.len());
        for (pos, rows) in samples.iter().enumerate() {
            let mut sorted = rows.clone();
            sorted.sort_by(f64::total_cmp);
            let name = names.get(pos).cloned().unwrap_or_default();
            ranked.push((name, percentile(&sorted, 50), percentile(&sorted, 99)));
        }
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        eprintln!("pass_timing gpu: per-pass GPU ms over {FRAMES} frames, slowest first");
        for (name, p50, p99) in &ranked {
            eprintln!("pass_timing gpu:   {name:<12} p50={p50:.4} p99={p99:.4}");
        }
        totals.sort_by(f64::total_cmp);
        eprintln!(
            "pass_timing gpu: frame total p50={:.4} p99={:.4} ms",
            percentile(&totals, 50),
            percentile(&totals, 99)
        );
        match ranked.first() {
            Some((slowest, p50, _)) => {
                eprintln!("pass_timing gpu: slowest pass = {slowest} (p50={p50:.4} ms)");
                assert!(!slowest.is_empty(), "the slowest pass is named");
            }
            None => panic!("the arena graph has passes, so one is slowest"),
        }
    }
}
