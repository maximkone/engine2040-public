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
            .to_string(),
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

/// True while the device records per-pass GPU timestamps. Never fails:
/// callers post `None` timings when false.
#[cfg(feature = "window")]
pub(crate) fn timestamp_supported(device: &wgpu::Device) -> bool {
    device.features().contains(wgpu::Features::TIMESTAMP_QUERY)
}

/// Tick delta to ms via the queue period (ns per tick). Pure.
#[cfg(feature = "window")]
pub(crate) fn ticks_to_ms(delta_ticks: u64, period: f32) -> f64 {
    let hi = u32::try_from(delta_ticks >> 32).unwrap_or(u32::MAX);
    let lo = u32::try_from(delta_ticks & 0xffff_ffff).unwrap_or(u32::MAX);
    (f64::from(hi) * 4_294_967_296.0 + f64::from(lo)) * f64::from(period) / 1_000_000.0
}

/// One pass ms from a begin/end tick pair; inverted pairs read zero. Pure.
#[cfg(feature = "window")]
pub(crate) fn pass_ms_from_ticks(begin_ticks: u64, end_ticks: u64, period: f32) -> f64 {
    ticks_to_ms(end_ticks.saturating_sub(begin_ticks), period)
}

/// Query pair for one compiled-order position. Pure.
#[cfg(feature = "window")]
pub(crate) fn query_pair(pos: usize) -> (u32, u32) {
    let begin = u32::try_from(pos.saturating_mul(2)).unwrap_or(u32::MAX);
    (begin, begin.saturating_add(1))
}

/// Query count for a submit: two timestamps per pass. Pure.
#[cfg(feature = "window")]
pub(crate) fn timestamp_query_count(passes: usize) -> u32 {
    u32::try_from(passes.saturating_mul(2)).unwrap_or(u32::MAX)
}

/// All-`None` timings for a submit that skipped or lost its queries. Pure.
#[cfg(feature = "window")]
pub(crate) fn none_gpu_timings(passes: usize) -> Vec<Option<f64>> {
    let mut rows = Vec::with_capacity(passes);
    for _ in 0..passes {
        rows.push(None);
    }
    rows
}

/// Decode a resolved buffer (`2 * passes` little-endian u64) to per-pass
/// ms; short tails and bad samples degrade to `None`. Pure.
#[cfg(feature = "window")]
pub(crate) fn decode_pass_bytes(bytes: &[u8], passes: usize, period: f32) -> Vec<Option<f64>> {
    let mut rows = Vec::with_capacity(passes);
    for pos in 0..passes {
        let base = pos.saturating_mul(16);
        let sample = bytes.get(base..base.saturating_add(16)).and_then(|pair| {
            let begin = u64::from_le_bytes(pair[0..8].try_into().ok()?);
            let end = u64::from_le_bytes(pair[8..16].try_into().ok()?);
            let ms = pass_ms_from_ticks(begin, end, period);
            if ms.is_finite() && ms >= 0.0 {
                Some(ms)
            } else {
                None
            }
        });
        rows.push(sample);
    }
    rows
}

/// Live per-submit timestamp state: pair set plus resolve and staging
/// buffers. The resolve buffer carries `QUERY_RESOLVE | COPY_SRC` and the
/// staging buffer `MAP_READ | COPY_DST`: `MAP_READ` only combines with its
/// opposite `COPY_DST`, so resolving straight into a mapped buffer is a
/// validation error. Evidence only, never hashed.
#[cfg(feature = "window")]
pub(crate) struct TimestampSetup {
    /// Pair set holding `2 * passes` timestamps.
    pub query_set: wgpu::QuerySet,
    /// Resolve destination for `resolve_query_set`.
    pub resolve_buf: wgpu::Buffer,
    /// Mapped staging copy of the resolved ticks.
    pub staging_buf: wgpu::Buffer,
    /// Timestamp count (`2 * passes`).
    pub query_count: u32,
    /// Queue timestamp period in ns per tick.
    pub period: f32,
}

/// Size the per-submit timestamp state, or `None` when the device or period
/// cannot record. `None` never fails startup: the submit records without
/// queries and posts `None` per pass.
#[cfg(feature = "window")]
pub(crate) fn timestamp_setup(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    passes: usize,
) -> Option<TimestampSetup> {
    let query_count = timestamp_query_count(passes);
    let period = if timestamp_supported(device) {
        queue.get_timestamp_period()
    } else {
        0.0
    };
    if query_count == 0 || !period.is_finite() || period <= 0.0 {
        return None;
    }
    let size = u64::from(query_count).saturating_mul(8);
    Some(TimestampSetup {
        query_set: device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("ts-pass-pairs"),
            ty: wgpu::QueryType::Timestamp,
            count: query_count,
        }),
        resolve_buf: device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ts-resolve"),
            size,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }),
        staging_buf: device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ts-staging"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        query_count,
        period,
    })
}

/// Read back a resolved timestamp buffer into per-pass ms. Any map failure
/// degrades to `None`: timestamps never fail the frame pixels already proved.
#[cfg(feature = "window")]
pub(crate) fn read_pass_ms(
    ts_buf: Option<&wgpu::Buffer>,
    ts_slice: Option<&wgpu::BufferSlice<'_>>,
    ts_rx: std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
    passes: usize,
    period: f32,
) -> Vec<Option<f64>> {
    let Some(buf) = ts_buf else {
        return none_gpu_timings(passes);
    };
    let Some(slice) = ts_slice else {
        buf.unmap();
        return none_gpu_timings(passes);
    };
    if !matches!(ts_rx.recv(), Ok(Ok(()))) {
        buf.unmap();
        return none_gpu_timings(passes);
    }
    let rows = match slice.get_mapped_range() {
        Ok(mapped) => decode_pass_bytes(&mapped, passes, period),
        Err(_) => none_gpu_timings(passes),
    };
    buf.unmap();
    rows
}

// ----- GX1: window submit through the graph submitter (one encode owner) -----
// The encode (pass pipeline, look bind groups, RA2-aliased slot targets)
// lives in `graph.rs`; this method builds the executor pass list into that
// same compiled shape and drives it, so WIN1 and ED1 run one graph on both
// legs. Budget and steady-state story are the graph submitter's (one owner:
// `graph::GPU_SUBMIT_BUDGET_MS` plus its reuse test); this path adds no new
// budget and no new error variant.

/// Window submit over the executor pass list (GX1).
#[cfg(feature = "window")]
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
            if row.first >= count || row.last >= count || row.first > row.last {
                return Err(GpuSubmitError::Invalid(format!(
                    "transient `{}` span [{}, {}] misses the {count}-pass range",
                    row.name, row.first, row.last
                )));
            }
        }
        let mut graph = FrameGraph::new();
        for (index, pass) in self.passes.iter().enumerate() {
            let mut desc = PassDesc::new(&pass.name).work(pass.draws, pass.triangles);
            for row in transients {
                if row.first == index {
                    let first = u32::try_from(row.first).map_err(|_| {
                        GpuSubmitError::Invalid(format!("pass id {} exceeds u32", row.first))
                    })?;
                    let last = u32::try_from(row.last).map_err(|_| {
                        GpuSubmitError::Invalid(format!("pass id {} exceeds u32", row.last))
                    })?;
                    desc = desc.transient_span(&row.name, row.size_bytes, first, last);
                }
            }
            graph.add_pass(desc);
        }
        for index in 1..count {
            graph
                .add_edge(index - 1, index)
                .map_err(|e| GpuSubmitError::Invalid(format!("linear chain: {e}")))?;
        }
        let compiled = graph
            .compile()
            .map_err(|e| GpuSubmitError::Invalid(format!("compile: {e}")))?;
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
    /// adapters). `None` is skip-not-green, never a fake pass.
    #[cfg(feature = "window")]
    fn exec_device() -> Option<(
        &'static wgpu::Device,
        &'static wgpu::Queue,
        &'static wgpu::AdapterInfo,
    )> {
        crate::graph::gx1_test_device()
    }

    #[cfg(feature = "window")]
    fn exec_frame(index: u64) -> crate::gtao::LookFrame {
        crate::graph::gx1_test_frame(index)
    }

    /// Four-pass executor plus aliasing rows: cascade at 0, HDR and depth
    /// overlapping at 1, scratch spanning 2..=3. Largest-first packs two
    /// backing slots ({hdr, cascade}, {depth, scratch}), so every byte is
    /// shared and the submit proves aliasing structurally.
    #[cfg(feature = "window")]
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
    #[cfg(feature = "window")]
    fn exec_window_submit_runs_passes_with_aliasing_clean() {
        let Some((device, queue, info)) = exec_device() else {
            eprintln!("SKIPPED exec window submit: no wgpu adapter on this machine");
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
    #[cfg(feature = "window")]
    fn exec_twin_matches_cpu_draws() {
        let Some((device, queue, _)) = exec_device() else {
            eprintln!("SKIPPED exec twin: no wgpu adapter on this machine");
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
    #[cfg(feature = "window")]
    fn exec_window_rejects_bad_span_before_sizing() {
        let Some((device, queue, _)) = exec_device() else {
            eprintln!("SKIPPED exec bad span: no wgpu adapter on this machine");
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
    #[cfg(feature = "window")]
    fn exec_invalid_frame_never_reports_unsupported() {
        let Some((device, queue, _)) = exec_device() else {
            eprintln!("SKIPPED exec invalid frame: no wgpu adapter on this machine");
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
    #[cfg(feature = "window")]
    fn exec_window_timings_resolve_from_query_receipt() {
        let Some((device, queue, _)) = exec_device() else {
            eprintln!("SKIPPED exec gpu timings: no wgpu adapter on this machine");
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
}
