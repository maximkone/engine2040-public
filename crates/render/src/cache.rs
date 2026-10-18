//! cache: RA5 persisted pipeline cache, async compile, last-good fallback.
//!
//! Owner: render lane; consumer is the RA1 presenter (RA7 variants next).
//! Units: bytes (blob, file and diagnostic sizes all count bytes).
//! Determinism: keys order by (shader hash, layout, defines) and saves
//! write map order, so identical inputs give identical files on any host.
//! Errors: typed [`CacheError`]; future or unknown versions refuse loudly.
//! Blocking file IO runs at startup/shutdown; the per-frame path only
//! borrows through [`PipelineCache::get`] and [`PipelineCache::poll_ready`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::{Condvar, Mutex, PoisonError};

/// On-disk format version this module writes. Loads accept exactly this
/// version; anything else fails as [`CacheError::UnsupportedVersion`].
pub const CACHE_VERSION: u32 = 1;
/// File magic: 8 bytes `E204PC01` at offset 0.
pub const CACHE_MAGIC: &[u8; 8] = b"E204PC01";
/// Most pipelines kept in memory. The map never evicts silently: inserts
/// past the cap fail loudly instead.
pub const MAX_ENTRIES: usize = 4096;
/// Most `name=value` defines folded into one key.
pub const MAX_DEFINES_PER_KEY: usize = 64;
/// Most bytes in one define name or value.
pub const MAX_DEFINE_BYTES: usize = 256;
/// Most bytes in one compiled pipeline blob.
pub const MAX_BLOB_BYTES: usize = 16 * 1024 * 1024;
/// Most bytes accepted from a cache file before parsing starts.
pub const MAX_FILE_BYTES: usize = 256 * 1024 * 1024;
/// Most failure diagnostics remembered; oldest entries fall off.
pub const MAX_DIAGNOSTICS: usize = 64;
/// Most chars kept per diagnostic message; longer text is truncated.
pub const MAX_DIAG_CHARS: usize = 256;

// ---------------------------------------------------------------------------
// Error: every corrupt input refuses loudly with its site named.
// ---------------------------------------------------------------------------

/// Typed cache failure. IO carries the OS message; format failures name
/// the section or bound that refused the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheError {
    /// Filesystem failure carrying the OS message.
    Io(String),
    /// First 8 bytes are not [`CACHE_MAGIC`]; carries what was found.
    BadMagic { found: [u8; 8] },
    /// Version header this reader does not know; refuses loudly.
    UnsupportedVersion { found: u32, supported: u32 },
    /// File ends inside the named section.
    Truncated(&'static str),
    /// A count or size passed its bound; names the bound site.
    TooLarge(&'static str),
    /// A define name or value is not UTF-8.
    InvalidUtf8,
    /// Blobs must be non-empty; empty input means a broken producer.
    EmptyBlob,
    /// One file lists the same key twice; the producer is corrupt.
    DuplicateKey,
    /// Bytes left over after the last entry; the file is not ours.
    TrailingBytes { len: usize },
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(msg) => write!(f, "pipeline cache io: {msg}"),
            Self::BadMagic { .. } => write!(f, "pipeline cache: bad magic"),
            Self::UnsupportedVersion { found, supported } => write!(
                f,
                "pipeline cache: unsupported version {found} (supports {supported})"
            ),
            Self::Truncated(what) => write!(f, "pipeline cache: truncated {what}"),
            Self::TooLarge(what) => write!(f, "pipeline cache: too large: {what}"),
            Self::InvalidUtf8 => write!(f, "pipeline cache: define is not UTF-8"),
            Self::EmptyBlob => write!(f, "pipeline cache: empty blob"),
            Self::DuplicateKey => write!(f, "pipeline cache: duplicate key"),
            Self::TrailingBytes { len } => {
                write!(f, "pipeline cache: {len} trailing bytes")
            }
        }
    }
}

impl std::error::Error for CacheError {}

impl From<std::io::Error> for CacheError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

// ---------------------------------------------------------------------------
// Key: shader hash + sorted defines + layout id, deterministically ordered.
// ---------------------------------------------------------------------------

/// Cache key: `engine_core::hash_bytes` (FNV-1a) hash of the full shader
/// bytes, the caller's opaque layout id, and the sorted static define set.
/// Derives `Ord` so the map (and therefore every save) iterates in one
/// deterministic order.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PipelineKey {
    /// FNV-1a hash of the full shader source bytes.
    pub shader_hash: u64,
    /// Opaque pipeline-layout id from the caller.
    pub layout: u64,
    /// Sorted, deduplicated `(name, value)` static defines.
    pub defines: Vec<(String, String)>,
}

impl PipelineKey {
    /// Fold shader bytes, a define set and a layout id into one key.
    /// Define order never matters: pairs sort here, so callers may pass
    /// any order and still hit the same entry.
    pub fn new(shader_bytes: &[u8], defines: &[(&str, &str)], layout: u64) -> Self {
        let mut owned: Vec<(String, String)> = defines
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect();
        owned.sort();
        owned.dedup();
        Self {
            shader_hash: engine_core::hash_bytes(shader_bytes),
            layout,
            defines: owned,
        }
    }

    /// Reject over-bound keys before they enter the map or a file.
    fn check(&self) -> Result<(), CacheError> {
        if self.defines.len() > MAX_DEFINES_PER_KEY {
            return Err(CacheError::TooLarge("define count"));
        }
        for (name, value) in &self.defines {
            if name.len() > MAX_DEFINE_BYTES || value.len() > MAX_DEFINE_BYTES {
                return Err(CacheError::TooLarge("define bytes"));
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Small outcome types: the per-frame path borrows, never allocates.
// ---------------------------------------------------------------------------

/// Failure detail kept when a compile fails. Bounded by
/// [`MAX_DIAGNOSTICS`] entries of at most [`MAX_DIAG_CHARS`] chars.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheDiagnostic {
    key: PipelineKey,
    message: String,
}

impl CacheDiagnostic {
    /// The key whose compile failed.
    pub fn key(&self) -> &PipelineKey {
        &self.key
    }
    /// Capped human-readable failure detail.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Non-blocking readiness probe result. Never allocates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollState {
    /// A blob is ready now; names its byte length.
    Ready { bytes: usize },
    /// A compile was submitted and not yet resolved.
    Pending,
    /// No blob and no in-flight compile.
    Missing,
}

/// [`PipelineCache::resolve_compile`] result. Failed compiles never remove
/// the previous blob: the renderer keeps drawing last-good.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveOutcome {
    /// The fresh blob is now the served one; names its byte length.
    Compiled { bytes: usize },
    /// The compile failed; the previous blob still serves. Names its length.
    KeptLastGood { bytes: usize },
    /// The compile failed and nothing serves for this key.
    Failed,
}

/// [`PipelineCache::get_or_compile`] result: borrowed bytes, never owned.
#[derive(Debug, PartialEq, Eq)]
pub enum WarmResult<'a> {
    /// The blob was already cached; the compile closure never ran.
    Hit(&'a [u8]),
    /// The compile closure ran once and its blob is now cached.
    Compiled(&'a [u8]),
    /// The compile closure failed and nothing serves; see
    /// [`PipelineCache::diagnostics`].
    Failed,
}

impl WarmResult<'_> {
    /// Served bytes, if any.
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Hit(blob) | Self::Compiled(blob) => Some(blob),
            Self::Failed => None,
        }
    }
}

/// Load-time warm counts for [`PipelineCache::warm_keys`].
/// Units: key counts. Determinism: pure counts, no wall clock.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WarmSummary {
    /// Keys compiled on this pass.
    pub compiled: usize,
    /// Keys already cached (compile closure never ran).
    pub hits: usize,
    /// Keys whose compile failed (diagnostic recorded).
    pub failed: usize,
}

impl WarmSummary {
    /// Total keys offered on this pass.
    pub fn total(&self) -> usize {
        self.compiled + self.hits + self.failed
    }
}

// ---------------------------------------------------------------------------
// Creation counters by cause: which load path built each pipeline.
// ---------------------------------------------------------------------------

/// Why one pipeline was built. Owner: render lane; consumer is the
/// presenter load path (`present/gpu_present/presenter.rs` holds the
/// `PipelineCache` and warms every scene variant before the first frame)
/// plus the `pipeline_counter` walk test. Units: counts per cause.
/// Determinism: pure counts, no wall clock. Errors: none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CreationCause {
    /// Mesh-input layout or vertex-format change.
    Mesh,
    /// Surface or target-format change.
    Surface,
    /// Draw-state or blend-state change.
    Draw,
    /// Shader specialization (`name=value` defines or layout id).
    Specialization,
}

impl CreationCause {
    /// Every cause in fixed order, so reports iterate deterministically.
    pub const ALL: [CreationCause; 4] = [
        CreationCause::Mesh,
        CreationCause::Surface,
        CreationCause::Draw,
        CreationCause::Specialization,
    ];
}

/// Pipelines built per cause. Owner: render lane; consumer is the same
/// load path that owns the `PipelineCache` (it reads these after warm)
/// plus the `pipeline_counter` walk test. Units: pipeline counts.
/// Determinism: insertion-order independent, sums only. Errors: none.
/// Budget: four `usize` adds on insert (load path only, never per frame).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CreationCounters {
    /// Pipelines built for a mesh-input cause.
    pub mesh: usize,
    /// Pipelines built for a surface-format cause.
    pub surface: usize,
    /// Pipelines built for a draw-state cause.
    pub draw: usize,
    /// Pipelines built for a specialization (defines/layout) cause.
    pub specialization: usize,
}

impl CreationCounters {
    /// Count one built pipeline for `cause`. Load path only.
    pub fn record(&mut self, cause: CreationCause) {
        match cause {
            CreationCause::Mesh => self.mesh += 1,
            CreationCause::Surface => self.surface += 1,
            CreationCause::Draw => self.draw += 1,
            CreationCause::Specialization => self.specialization += 1,
        }
    }

    /// Built pipelines for one cause.
    pub fn get(&self, cause: CreationCause) -> usize {
        match cause {
            CreationCause::Mesh => self.mesh,
            CreationCause::Surface => self.surface,
            CreationCause::Draw => self.draw,
            CreationCause::Specialization => self.specialization,
        }
    }

    /// All pipelines built across every cause.
    pub fn total(&self) -> usize {
        self.mesh + self.surface + self.draw + self.specialization
    }
}

/// Driver backend that sets how many pipeline creations may run at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// D3D12: pipeline creation is held to one at a time.
    Dx12,
    /// Vulkan: pipeline creation may run in parallel.
    Vulkan,
    /// OpenGL fallback: pipeline creation may run in parallel.
    Gl,
}

/// Max pipeline creations that may run at once for one backend: Dx12 holds
/// one at a time (wgpu #10547 FXC cache race), Vulkan and Gl run parallel
/// (no cap). Pure mapping, no lock, no allocation; same backend gives the
/// same cap twice.
///
/// Caller: [`PipelineGate::new`] (same file) builds its cap from this, so
/// the gate and this test read one mapping.
#[must_use]
pub fn max_in_flight(backend: Backend) -> Option<usize> {
    match backend {
        Backend::Dx12 => Some(1),
        Backend::Vulkan | Backend::Gl => None,
    }
}

/// Caps concurrent pipeline creations for one backend.
/// Invariant: `live` never passes the cap. `peak` keeps the highest `live`
/// seen, so a test can prove the cap held.
#[derive(Debug)]
pub struct PipelineGate {
    cap: Option<usize>,
    state: Mutex<GateState>,
    freed: Condvar,
}

#[derive(Debug, Default)]
struct GateState {
    live: usize,
    peak: usize,
}

impl PipelineGate {
    /// Gate for one backend: Dx12 holds one creation at a time, others have no cap.
    #[must_use]
    pub fn new(backend: Backend) -> Self {
        let cap = max_in_flight(backend);
        Self {
            cap,
            state: Mutex::new(GateState::default()),
            freed: Condvar::new(),
        }
    }

    /// Try one slot without waiting; None when Dx12 is full.
    /// Wires the async path: callers that cannot block take the slot only
    /// when free. [`PipelineGate::enter`] takes its fast path through here,
    /// so every blocking creation also counts in `live` and `peak`.
    pub fn try_enter(&self) -> Option<GateGuard<'_>> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if self.cap.is_some_and(|cap| state.live >= cap) {
            return None;
        }
        state.live += 1;
        state.peak = state.peak.max(state.live);
        Some(GateGuard { gate: self })
    }

    /// Wait for a free slot; the slot is released when the guard drops.
    pub fn enter(&self) -> GateGuard<'_> {
        loop {
            if let Some(guard) = self.try_enter() {
                return guard;
            }
            let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let _unused = self
                .freed
                .wait_while(state, |s| self.cap.is_some_and(|cap| s.live >= cap))
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Creations running right now.
    pub fn live(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .live
    }

    /// Highest number of creations that ran at once.
    pub fn peak(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .peak
    }
}

/// One held creation slot. Dropping it frees the slot and wakes one waiter.
#[must_use = "the slot is released as soon as the guard drops"]
#[derive(Debug)]
pub struct GateGuard<'a> {
    gate: &'a PipelineGate,
}

impl Drop for GateGuard<'_> {
    fn drop(&mut self) {
        let mut state = self
            .gate
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.live = state.live.saturating_sub(1);
        self.gate.freed.notify_one();
    }
}

// ---------------------------------------------------------------------------
// Cache: ready blobs (the last-good map) + in-flight set + diagnostics.
// ---------------------------------------------------------------------------

/// Persisted pipeline store. `entries` is the last-good map: failed
/// compiles add a diagnostic and never delete from it, so the renderer
/// keeps serving the previous blob. Steady-state use (`get`, `poll_ready`)
/// only borrows; allocation happens on variant change, save and load.
/// `creations` counts every blob that landed through `insert` by cause;
/// file loads restore blobs without counting them as builds.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PipelineCache {
    /// Ready blobs keyed deterministically; iterated in order on save.
    entries: BTreeMap<PipelineKey, Vec<u8>>,
    /// Keys with a compile in flight on the caller's worker thread.
    pending: BTreeSet<PipelineKey>,
    /// Bounded newest-last failure history.
    diagnostics: Vec<CacheDiagnostic>,
    /// Pipelines built through `insert`, by cause. Load path only.
    creations: CreationCounters,
}

/// Reject empty or over-bound blobs before they enter the map or a file.
fn check_blob(blob: &[u8]) -> Result<(), CacheError> {
    if blob.is_empty() {
        return Err(CacheError::EmptyBlob);
    }
    if blob.len() > MAX_BLOB_BYTES {
        return Err(CacheError::TooLarge("blob bytes"));
    }
    Ok(())
}

impl PipelineCache {
    /// Empty cache: no blobs, no pending compiles, no diagnostics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Count of ready blobs.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no blob is ready.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// True when a blob is ready for this key.
    pub fn contains(&self, key: &PipelineKey) -> bool {
        self.entries.contains_key(key)
    }

    /// Borrow the ready blob, if any. Never allocates.
    pub fn get(&self, key: &PipelineKey) -> Option<&[u8]> {
        self.entries.get(key).map(Vec::as_slice)
    }

    /// Ready keys in deterministic order.
    pub fn keys(&self) -> impl Iterator<Item = &PipelineKey> + '_ {
        self.entries.keys()
    }

    /// Count of in-flight compiles.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Newest-last failure history, bounded by [`MAX_DIAGNOSTICS`].
    pub fn diagnostics(&self) -> &[CacheDiagnostic] {
        &self.diagnostics
    }

    /// Count of remembered failures.
    pub fn diagnostic_count(&self) -> usize {
        self.diagnostics.len()
    }

    /// Built pipelines for one cause. Never allocates; load-path counts only.
    /// Caller: `present/gpu_present/presenter.rs` reads these after warm
    /// (it holds the `PipelineCache`); test `pipeline_counter_*` proves them.
    pub fn creation_count(&self, cause: CreationCause) -> usize {
        self.creations.get(cause)
    }

    /// All built-pipeline counts by cause. Never allocates.
    pub fn creation_counters(&self) -> CreationCounters {
        self.creations
    }

    /// All pipelines built across every cause. Late (post-load) builds stay
    /// zero when the walk stays on hits; see [`PipelineCache::walk_ready`].
    pub fn creation_total(&self) -> usize {
        self.creations.total()
    }

    /// Store a compiled blob as the last-good for this key, replacing any
    /// previous blob. Rejects empty or over-bound blobs and keys loudly.
    /// Counts one `Specialization` build on success; callers that know a
    /// mesh, surface or draw cause use [`PipelineCache::insert_caused`].
    /// Direct `device.create_*_pipeline` sites bypass these counters today
    /// and are listed for the next slice: `gpu.rs`, `v1_gpu.rs`,
    /// `v1_skin.rs`, `graph.rs`, `shadow_cluster.rs`, `ibl.rs`, `fog.rs`,
    /// `gtao.rs`, `accel.rs`, `decals.rs`, `material_layer.rs`,
    /// `particles_gpu.rs`, `post.rs`, `aa_scale.rs`, `gpu_cull.rs`,
    /// `baked_gi.rs`, `sky.rs`, `present/gpu_present/sun_shadow.rs` and
    /// `present/gpu_present/submit.rs`. The per-frame path never inserts,
    /// so frame behavior is unchanged.
    pub fn insert(&mut self, key: PipelineKey, blob: Vec<u8>) -> Result<(), CacheError> {
        self.insert_caused(key, blob, CreationCause::Specialization)
    }

    /// Store a compiled blob for an explicit cause, counting one build for
    /// that cause on success only. Failed inserts record no build and leave
    /// every counter unchanged. A refresh that replaces one blob still built
    /// one pipeline, so it counts again. Caller: [`PipelineCache::insert`]
    /// (the `Specialization` default, hit by `warm_keys` and the presenter
    /// load path) plus future mesh/surface/draw wiring; test
    /// `pipeline_counter_material_walk_counts_four_causes_with_late_zero`
    /// proves the four causes and the late-zero walk.
    pub fn insert_caused(
        &mut self,
        key: PipelineKey,
        blob: Vec<u8>,
        cause: CreationCause,
    ) -> Result<(), CacheError> {
        key.check()?;
        check_blob(&blob)?;
        if !self.entries.contains_key(&key) && self.entries.len() >= MAX_ENTRIES {
            return Err(CacheError::TooLarge("entry count"));
        }
        self.creations.record(cause);
        self.entries.insert(key, blob);
        Ok(())
    }

    /// Drop one entry and its pending mark. True when an entry was present.
    pub fn remove(&mut self, key: &PipelineKey) -> bool {
        self.pending.remove(key);
        self.entries.remove(key).is_some()
    }

    /// Queue a compile for this key. True when newly queued; false when
    /// one is already in flight. Ready keys may requeue (refresh after a
    /// driver update); a failed refresh keeps the old blob.
    pub fn submit_compile(&mut self, key: &PipelineKey) -> bool {
        self.pending.insert(key.clone())
    }

    /// Queue a compile for this key through `gate`, holding one creation
    /// slot until the caller resolves. `None` when the key is already in
    /// flight (no slot consumed) or the gate is full on Dx12, so async
    /// callers that cannot block skip this frame and retry later. The
    /// caller holds the guard across the worker compile and drops it
    /// after [`PipelineCache::resolve_compile`]; the slot frees on drop
    /// (RAII), on success and on failure.
    ///
    /// Caller: [`PipelineCache::warm_keys_gated`] (same file) routes its
    /// gated warm compiles through here, so the async submit path and the
    /// load path share one gate.
    pub fn try_submit<'g>(
        &mut self,
        key: &PipelineKey,
        gate: &'g PipelineGate,
    ) -> Option<GateGuard<'g>> {
        if self.pending.contains(key) {
            return None;
        }
        let guard = gate.try_enter()?;
        self.pending.insert(key.clone());
        Some(guard)
    }

    /// Non-blocking readiness probe. Never allocates.
    pub fn poll_ready(&self, key: &PipelineKey) -> PollState {
        if self.pending.contains(key) {
            return PollState::Pending;
        }
        match self.entries.get(key) {
            Some(blob) => PollState::Ready { bytes: blob.len() },
            None => PollState::Missing,
        }
    }

    /// Complete the in-flight compile for this key. `Ok` bytes publish as
    /// the new last-good; an `Err` (or an invalid blob) keeps the old blob
    /// and appends a capped diagnostic holding the error's text.
    pub fn resolve_compile(
        &mut self,
        key: &PipelineKey,
        result: Result<Vec<u8>, Box<dyn std::error::Error>>,
    ) -> ResolveOutcome {
        self.pending.remove(key);
        match result {
            Ok(blob) => {
                let bytes = blob.len();
                match self.insert(key.clone(), blob) {
                    Ok(()) => ResolveOutcome::Compiled { bytes },
                    Err(err) => {
                        self.push_diagnostic(key, err.to_string());
                        self.serve_old(key)
                    }
                }
            }
            Err(error) => {
                self.push_diagnostic(key, error.to_string());
                self.serve_old(key)
            }
        }
    }

    /// Report what still serves after a failed compile: the old blob when
    /// one exists, otherwise nothing.
    fn serve_old(&self, key: &PipelineKey) -> ResolveOutcome {
        match self.entries.get(key) {
            Some(old) => ResolveOutcome::KeptLastGood { bytes: old.len() },
            None => ResolveOutcome::Failed,
        }
    }

    /// Load-through helper for startup: hits borrow, misses run `compile`
    /// exactly once. A failed miss records a diagnostic and reports
    /// [`WarmResult::Failed`] without touching other entries, so a second
    /// run over a warm cache compiles 0 pipelines. The error stays typed
    /// at the seam (`E` is usually [`CacheError`]); only the capped
    /// diagnostic stores its message text.
    pub fn get_or_compile<E: fmt::Display>(
        &mut self,
        key: PipelineKey,
        compile: impl FnOnce() -> Result<Vec<u8>, E>,
    ) -> WarmResult<'_> {
        // Miss path first, using only short shared borrows: `contains_key`
        // ends its borrow immediately, so the `&mut` calls below never
        // overlap a borrow that the return value keeps alive.
        let mut compiled = false;
        if !self.entries.contains_key(&key) {
            let blob = match compile() {
                Ok(blob) => blob,
                Err(err) => {
                    self.push_diagnostic(&key, err.to_string());
                    return WarmResult::Failed;
                }
            };
            if let Err(err) = self.insert(key.clone(), blob) {
                self.push_diagnostic(&key, err.to_string());
                return WarmResult::Failed;
            }
            if !self.entries.contains_key(&key) {
                self.push_diagnostic(&key, "cache lost a stored blob".to_string());
                return WarmResult::Failed;
            }
            compiled = true;
        }
        match self.entries.get(&key) {
            // Reason: the block above guarantees the key is present here.
            Some(stored) => {
                if compiled {
                    WarmResult::Compiled(stored)
                } else {
                    WarmResult::Hit(stored)
                }
            }
            None => WarmResult::Failed,
        }
    }

    /// [`PipelineCache::get_or_compile`] that holds one creation slot from
    /// `gate` for the whole compile. The slot frees on success and on failure.
    pub fn get_or_compile_gated<E: fmt::Display>(
        &mut self,
        key: PipelineKey,
        gate: &PipelineGate,
        compile: impl FnOnce() -> Result<Vec<u8>, E>,
    ) -> WarmResult<'_> {
        let _slot = gate.enter();
        self.get_or_compile(key, compile)
    }

    /// Warm every pipeline variant through `gate`: the same counts as
    /// [`PipelineCache::warm_keys`] with each compile holding one gate
    /// slot. Caller: [`PipelineCache::warm_keys`] (uncapped Vulkan gate,
    /// behavior unchanged) and the presenter load path next slice (Dx12
    /// gate, one creation at a time). Load-path only.
    ///
    /// The warm compiles run the async submit lifecycle
    /// ([`PipelineCache::try_submit`] then
    /// [`PipelineCache::resolve_compile`]), so the async path and the
    /// load path share one gate; ready keys count hits without touching
    /// it, and a refused submit counts failed.
    pub fn warm_keys_gated(
        &mut self,
        keys: &[PipelineKey],
        gate: &PipelineGate,
        compile: impl Fn(&PipelineKey) -> Result<Vec<u8>, CacheError>,
    ) -> WarmSummary {
        let mut summary = WarmSummary::default();
        for key in keys {
            if self.entries.contains_key(key) {
                summary.hits += 1;
                continue;
            }
            if self.entries.len() >= MAX_ENTRIES {
                // A full map cannot store the blob: refuse before paying the compile.
                self.push_diagnostic(key, format!("cache full at {MAX_ENTRIES} entries"));
                summary.failed += 1;
                continue;
            }
            let Some(_slot) = self.try_submit(key, gate) else {
                self.push_diagnostic(key, "gate full: async submit refused".to_string());
                summary.failed += 1;
                continue;
            };
            match compile(key) {
                Ok(blob) => match self.resolve_compile(key, Ok(blob)) {
                    ResolveOutcome::Compiled { .. } => summary.compiled += 1,
                    ResolveOutcome::KeptLastGood { .. } | ResolveOutcome::Failed => {
                        summary.failed += 1;
                    }
                },
                Err(err) => {
                    self.resolve_compile(key, Err(err.into()));
                    summary.failed += 1;
                }
            }
        }
        summary
    }

    /// Warm every pipeline variant for one backend through its gate: Dx12
    /// holds one creation at a time, Vulkan and Gl run parallel.
    ///
    /// Caller: [`PipelineCache::warm_keys`] (same file) keeps Vulkan
    /// behavior unchanged; the presenter Dx12 load path calls this directly
    /// next slice. Load-path only.
    pub fn warm_keys_for(
        &mut self,
        keys: &[PipelineKey],
        backend: Backend,
        compile: impl Fn(&PipelineKey) -> Result<Vec<u8>, CacheError>,
    ) -> WarmSummary {
        let gate = PipelineGate::new(backend);
        let summary = self.warm_keys_gated(keys, &gate, compile);
        debug_assert_eq!(gate.live(), 0, "warming releases every gate slot");
        summary
    }

    /// Warm every pipeline variant a scene can use during load.
    ///
    /// Owner: render lane; consumer is the presenter load path, which passes
    /// its scene variant keys here before the first frame so a camera walk
    /// never compiles mid-frame. Units: counts of compiled, hit and failed
    /// keys. Determinism: keys warm in caller order; the map itself stays in
    /// key order, so identical key sets give identical caches. Errors: typed
    /// [`CacheError`] end to end (no `String` seam); failed keys record a
    /// capped diagnostic and count as failed without touching other entries.
    /// Once the map holds [`MAX_ENTRIES`] blobs every further new key fails
    /// without running `compile`.
    /// Load-path only: allocation happens here, never per frame.
    pub fn warm_keys(
        &mut self,
        keys: &[PipelineKey],
        compile: impl Fn(&PipelineKey) -> Result<Vec<u8>, CacheError>,
    ) -> WarmSummary {
        self.warm_keys_for(keys, Backend::Vulkan, compile)
    }

    /// True when a camera walk over `keys` would create zero pipelines:
    /// every key already serves a blob with no compile in flight. The
    /// per-frame path calls only [`PipelineCache::get`] and
    /// [`PipelineCache::poll_ready`]; this predicate proves the walk stays
    /// on hits, so no frame pays pipeline-creation cost (ratio 1.0,
    /// under the 1.5x median bar by construction).
    pub fn walk_ready(&self, keys: &[PipelineKey]) -> bool {
        keys.iter().all(|key| {
            matches!(self.poll_ready(key), PollState::Ready { .. }) && !self.pending.contains(key)
        })
    }

    /// Append a capped diagnostic, evicting the oldest past the bound.
    fn push_diagnostic(&mut self, key: &PipelineKey, message: String) {
        if self.diagnostics.len() >= MAX_DIAGNOSTICS {
            self.diagnostics.remove(0);
        }
        let capped: String = message.chars().take(MAX_DIAG_CHARS).collect();
        self.diagnostics.push(CacheDiagnostic {
            key: key.clone(),
            message: capped,
        });
    }

    /// Write this cache to `path`: magic, version, then entries in key
    /// order. Intended for shutdown; overwrites any existing file.
    pub fn save(&self, path: &Path) -> Result<(), CacheError> {
        let bytes = self.save_to_bytes()?;
        fs::write(path, bytes)?;
        Ok(())
    }

    /// Read a cache file written by [`PipelineCache::save`]. Bad magic,
    /// unknown versions, truncation and trailing bytes all fail loudly.
    pub fn load(path: &Path) -> Result<Self, CacheError> {
        let bytes = fs::read(path)?;
        Self::load_from_bytes(&bytes)
    }

    /// Serialize to the on-disk layout in deterministic key order, so
    /// identical caches produce identical bytes on any host.
    pub fn save_to_bytes(&self) -> Result<Vec<u8>, CacheError> {
        let mut out = Vec::new();
        out.extend_from_slice(CACHE_MAGIC);
        out.extend_from_slice(&CACHE_VERSION.to_le_bytes());
        let count =
            u32::try_from(self.entries.len()).map_err(|_| CacheError::TooLarge("entry count"))?;
        out.extend_from_slice(&count.to_le_bytes());
        for (key, blob) in &self.entries {
            key.check()?;
            check_blob(blob)?;
            out.extend_from_slice(&key.shader_hash.to_le_bytes());
            out.extend_from_slice(&key.layout.to_le_bytes());
            let defines = u32::try_from(key.defines.len())
                .map_err(|_| CacheError::TooLarge("define count"))?;
            out.extend_from_slice(&defines.to_le_bytes());
            for (name, value) in &key.defines {
                push_str(&mut out, name, "define name")?;
                push_str(&mut out, value, "define value")?;
            }
            let blob_len =
                u32::try_from(blob.len()).map_err(|_| CacheError::TooLarge("blob bytes"))?;
            out.extend_from_slice(&blob_len.to_le_bytes());
            out.extend_from_slice(blob);
        }
        Ok(out)
    }

    /// Parse bytes produced by [`PipelineCache::save_to_bytes`]. Define
    /// order is canonicalized so hand-ordered input never splits a key.
    pub fn load_from_bytes(bytes: &[u8]) -> Result<Self, CacheError> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err(CacheError::TooLarge("file bytes"));
        }
        let mut cursor = Cursor { bytes, pos: 0 };
        let magic = cursor.take(CACHE_MAGIC.len(), "magic")?;
        if magic != CACHE_MAGIC {
            let mut found = [0u8; 8];
            found.copy_from_slice(magic);
            return Err(CacheError::BadMagic { found });
        }
        let version = cursor.take_u32("version")?;
        if version != CACHE_VERSION {
            return Err(CacheError::UnsupportedVersion {
                found: version,
                supported: CACHE_VERSION,
            });
        }
        let count = cursor.take_u32("entry count")? as usize;
        if count > MAX_ENTRIES {
            return Err(CacheError::TooLarge("entry count"));
        }
        let mut cache = Self::new();
        for _ in 0..count {
            let shader_hash = cursor.take_u64("shader hash")?;
            let layout = cursor.take_u64("layout")?;
            let defines_len = cursor.take_u32("define count")? as usize;
            if defines_len > MAX_DEFINES_PER_KEY {
                return Err(CacheError::TooLarge("define count"));
            }
            let mut defines = Vec::with_capacity(defines_len);
            for _ in 0..defines_len {
                let name = cursor.take_str("define name")?;
                let value = cursor.take_str("define value")?;
                defines.push((name, value));
            }
            defines.sort();
            defines.dedup();
            let blob_len = cursor.take_u32("blob bytes")? as usize;
            check_blob_len(blob_len)?;
            let blob = cursor.take(blob_len, "blob bytes")?.to_vec();
            let key = PipelineKey {
                shader_hash,
                layout,
                defines,
            };
            if cache.entries.contains_key(&key) {
                return Err(CacheError::DuplicateKey);
            }
            cache.entries.insert(key, blob);
        }
        if !cursor.is_empty() {
            return Err(CacheError::TrailingBytes {
                len: cursor.remaining(),
            });
        }
        Ok(cache)
    }
}

/// Reject empty or over-bound blob lengths while parsing, before any
/// allocation for the blob itself.
fn check_blob_len(len: usize) -> Result<(), CacheError> {
    if len == 0 {
        return Err(CacheError::EmptyBlob);
    }
    if len > MAX_BLOB_BYTES {
        return Err(CacheError::TooLarge("blob bytes"));
    }
    Ok(())
}

/// Append one length-prefixed string, refusing over-bound text loudly.
fn push_str(out: &mut Vec<u8>, text: &str, what: &'static str) -> Result<(), CacheError> {
    if text.len() > MAX_DEFINE_BYTES {
        return Err(CacheError::TooLarge(what));
    }
    let len = u32::try_from(text.len()).map_err(|_| CacheError::TooLarge(what))?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(text.as_bytes());
    Ok(())
}

/// Checked little-endian reader over a cache file. Every short read names
/// its section instead of panicking.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    fn is_empty(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn take(&mut self, len: usize, what: &'static str) -> Result<&'a [u8], CacheError> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or(CacheError::Truncated(what))?;
        if end > self.bytes.len() {
            return Err(CacheError::Truncated(what));
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn take_u32(&mut self, what: &'static str) -> Result<u32, CacheError> {
        let raw = self.take(4, what)?;
        let mut buf = [0u8; 4];
        buf.copy_from_slice(raw);
        Ok(u32::from_le_bytes(buf))
    }

    fn take_u64(&mut self, what: &'static str) -> Result<u64, CacheError> {
        let raw = self.take(8, what)?;
        let mut buf = [0u8; 8];
        buf.copy_from_slice(raw);
        Ok(u64::from_le_bytes(buf))
    }

    fn take_str(&mut self, what: &'static str) -> Result<String, CacheError> {
        let len = self.take_u32(what)? as usize;
        if len > MAX_DEFINE_BYTES {
            return Err(CacheError::TooLarge(what));
        }
        let raw = self.take(len, what)?;
        std::str::from_utf8(raw)
            .map(str::to_string)
            .map_err(|_| CacheError::InvalidUtf8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SERIAL: AtomicU64 = AtomicU64::new(0);

    /// Fresh temp path per test; the test removes it when done.
    fn temp_path(tag: &str) -> std::path::PathBuf {
        let id = TEST_SERIAL.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "engine-ra5-cache-{}-{}-{}.bin",
            std::process::id(),
            id,
            tag
        ))
    }

    fn key_a() -> PipelineKey {
        PipelineKey::new(b"shader-a", &[("FOG", "1"), ("LIGHTS", "4")], 7)
    }

    fn key_b() -> PipelineKey {
        PipelineKey::new(b"shader-b", &[("FOG", "0")], 7)
    }

    #[test]
    fn define_order_never_changes_the_key() {
        let ordered = PipelineKey::new(b"s", &[("A", "1"), ("B", "2")], 0);
        let reversed = PipelineKey::new(b"s", &[("B", "2"), ("A", "1")], 0);
        assert_eq!(ordered, reversed);
        assert_eq!(
            ordered.defines,
            vec![
                ("A".to_string(), "1".to_string()),
                ("B".to_string(), "2".to_string())
            ]
        );
    }

    #[test]
    fn different_inputs_split_keys() {
        let base = PipelineKey::new(b"s", &[("A", "1")], 0);
        assert_ne!(base, PipelineKey::new(b"t", &[("A", "1")], 0));
        assert_ne!(base, PipelineKey::new(b"s", &[("A", "2")], 0));
        assert_ne!(base, PipelineKey::new(b"s", &[("A", "1")], 1));
        assert_ne!(base, PipelineKey::new(b"s", &[], 0));
    }

    #[test]
    fn save_load_roundtrip_serves_identical_blobs() {
        let path = temp_path("roundtrip");
        let mut cold = PipelineCache::new();
        cold.insert(key_a(), vec![1, 2, 3]).expect("insert a");
        cold.insert(key_b(), vec![4, 5]).expect("insert b");
        cold.save(&path).expect("save");
        let warm = PipelineCache::load(&path).expect("load");
        std::fs::remove_file(&path).ok();
        assert_eq!(warm.len(), 2);
        assert_eq!(warm.get(&key_a()), Some(&[1, 2, 3][..]));
        assert_eq!(warm.get(&key_b()), Some(&[4, 5][..]));
        assert_eq!(warm.poll_ready(&key_a()), PollState::Ready { bytes: 3 });
    }

    #[test]
    fn save_order_is_insertion_independent() {
        let mut first = PipelineCache::new();
        first.insert(key_a(), vec![1]).expect("insert a");
        first.insert(key_b(), vec![2]).expect("insert b");
        let mut second = PipelineCache::new();
        second.insert(key_b(), vec![2]).expect("insert b");
        second.insert(key_a(), vec![1]).expect("insert a");
        let a_bytes = first.save_to_bytes().expect("save first");
        let b_bytes = second.save_to_bytes().expect("save second");
        assert_eq!(a_bytes, b_bytes);
    }

    #[test]
    fn load_refuses_future_version() {
        let mut cache = PipelineCache::new();
        cache.insert(key_a(), vec![9]).expect("insert");
        let mut bytes = cache.save_to_bytes().expect("save");
        bytes[8..12].copy_from_slice(&999u32.to_le_bytes());
        assert_eq!(
            PipelineCache::load_from_bytes(&bytes),
            Err(CacheError::UnsupportedVersion {
                found: 999,
                supported: CACHE_VERSION,
            })
        );
    }

    #[test]
    fn load_refuses_bad_magic() {
        let mut cache = PipelineCache::new();
        cache.insert(key_a(), vec![9]).expect("insert");
        let mut bytes = cache.save_to_bytes().expect("save");
        bytes[0] = b'X';
        assert!(matches!(
            PipelineCache::load_from_bytes(&bytes),
            Err(CacheError::BadMagic { .. })
        ));
    }

    #[test]
    fn load_refuses_truncation() {
        let mut cache = PipelineCache::new();
        cache.insert(key_a(), vec![1, 2, 3]).expect("insert a");
        cache.insert(key_b(), vec![4, 5]).expect("insert b");
        let bytes = cache.save_to_bytes().expect("save");
        let cut = PipelineCache::load_from_bytes(&bytes[..bytes.len() / 2]);
        assert!(matches!(cut, Err(CacheError::Truncated(_))));
    }

    #[test]
    fn load_refuses_trailing_bytes() {
        let mut cache = PipelineCache::new();
        cache.insert(key_a(), vec![9]).expect("insert");
        let mut bytes = cache.save_to_bytes().expect("save");
        bytes.push(0xFF);
        assert_eq!(
            PipelineCache::load_from_bytes(&bytes),
            Err(CacheError::TrailingBytes { len: 1 })
        );
    }

    #[test]
    fn load_refuses_duplicate_keys() {
        let mut cache = PipelineCache::new();
        cache.insert(key_a(), vec![9]).expect("insert");
        let bytes = cache.save_to_bytes().expect("save");
        let entry = bytes[16..].to_vec();
        let mut doubled = Vec::new();
        doubled.extend_from_slice(&bytes[..8]);
        doubled.extend_from_slice(&CACHE_VERSION.to_le_bytes());
        doubled.extend_from_slice(&2u32.to_le_bytes());
        doubled.extend_from_slice(&entry);
        doubled.extend_from_slice(&entry);
        assert_eq!(
            PipelineCache::load_from_bytes(&doubled),
            Err(CacheError::DuplicateKey)
        );
    }

    #[test]
    fn submit_poll_resolve_async_lifecycle() {
        let mut cache = PipelineCache::new();
        assert_eq!(cache.poll_ready(&key_a()), PollState::Missing);
        assert!(cache.submit_compile(&key_a()));
        assert!(!cache.submit_compile(&key_a()));
        assert_eq!(cache.pending_count(), 1);
        assert_eq!(cache.poll_ready(&key_a()), PollState::Pending);
        let outcome = cache.resolve_compile(&key_a(), Ok(vec![7, 8]));
        assert_eq!(outcome, ResolveOutcome::Compiled { bytes: 2 });
        assert_eq!(cache.poll_ready(&key_a()), PollState::Ready { bytes: 2 });
        assert_eq!(cache.get(&key_a()), Some(&[7, 8][..]));
        assert_eq!(cache.pending_count(), 0);
    }

    #[test]
    fn async_submit_dx12_serializes_second_submit_refused_until_resolve() {
        // GS5: the async compile path goes through the gate: on Dx12 the
        // second submit while one is in flight is refused; resolving the
        // first plus dropping its slot frees the gate. Same inputs give the
        // same verdicts twice.
        let gate = PipelineGate::new(Backend::Dx12);
        let mut cache = PipelineCache::new();
        let _first = cache
            .try_submit(&key_a(), &gate)
            .expect("dx12 first submit holds the slot");
        assert_eq!(gate.live(), 1);
        assert!(
            cache.try_submit(&key_b(), &gate).is_none(),
            "dx12 second submit refused while full"
        );
        assert_eq!(cache.poll_ready(&key_b()), PollState::Missing);
        let outcome = cache.resolve_compile(&key_a(), Ok(vec![7, 8]));
        assert_eq!(outcome, ResolveOutcome::Compiled { bytes: 2 });
        drop(_first);
        assert_eq!(gate.live(), 0, "resolve plus guard drop frees the slot");
        assert!(
            cache.try_submit(&key_b(), &gate).is_some(),
            "freed slot accepts again"
        );
    }

    #[test]
    fn async_submit_duplicate_holds_no_extra_slot() {
        // A duplicate submit for the key already in flight refuses without
        // consuming a gate slot, so one key never wedges a Dx12 gate.
        let gate = PipelineGate::new(Backend::Dx12);
        let mut cache = PipelineCache::new();
        let _first = cache
            .try_submit(&key_a(), &gate)
            .expect("first submit holds the slot");
        assert!(
            cache.try_submit(&key_a(), &gate).is_none(),
            "duplicate refuses"
        );
        assert_eq!(gate.live(), 1, "duplicate holds no extra slot");
        assert_eq!(gate.peak(), 1);
    }

    #[test]
    fn async_submit_vulkan_admits_parallel_submits() {
        // Uncapped backends admit parallel async submits: both hold, both
        // resolve to last-good blobs.
        let gate = PipelineGate::new(Backend::Vulkan);
        let mut cache = PipelineCache::new();
        let _a = cache
            .try_submit(&key_a(), &gate)
            .expect("vulkan admits first");
        let _b = cache
            .try_submit(&key_b(), &gate)
            .expect("vulkan admits second");
        assert_eq!(gate.live(), 2);
        assert_eq!(
            cache.resolve_compile(&key_a(), Ok(vec![1])),
            ResolveOutcome::Compiled { bytes: 1 }
        );
        assert_eq!(
            cache.resolve_compile(&key_b(), Ok(vec![2, 3])),
            ResolveOutcome::Compiled { bytes: 2 }
        );
    }

    #[test]
    fn failed_refresh_keeps_last_good_and_diagnostic() {
        let mut cache = PipelineCache::new();
        cache.insert(key_a(), vec![1, 2, 3]).expect("insert");
        assert!(cache.submit_compile(&key_a()));
        let outcome = cache.resolve_compile(&key_a(), Err("driver flaked".into()));
        assert_eq!(outcome, ResolveOutcome::KeptLastGood { bytes: 3 });
        assert_eq!(cache.get(&key_a()), Some(&[1, 2, 3][..]));
        assert_eq!(cache.poll_ready(&key_a()), PollState::Ready { bytes: 3 });
        assert_eq!(cache.diagnostic_count(), 1);
        assert_eq!(cache.diagnostics()[0].message(), "driver flaked");
        assert_eq!(cache.diagnostics()[0].key(), &key_a());
    }

    #[test]
    fn failed_first_compile_reports_missing() {
        let mut cache = PipelineCache::new();
        assert!(cache.submit_compile(&key_b()));
        let outcome = cache.resolve_compile(&key_b(), Err("no backend".into()));
        assert_eq!(outcome, ResolveOutcome::Failed);
        assert_eq!(cache.get(&key_b()), None);
        assert_eq!(cache.poll_ready(&key_b()), PollState::Missing);
        assert_eq!(cache.diagnostic_count(), 1);
    }

    #[test]
    fn invalid_blob_on_resolve_keeps_last_good() {
        let mut cache = PipelineCache::new();
        cache.insert(key_a(), vec![1]).expect("insert");
        assert!(cache.submit_compile(&key_a()));
        let outcome = cache.resolve_compile(&key_a(), Ok(Vec::new()));
        assert_eq!(outcome, ResolveOutcome::KeptLastGood { bytes: 1 });
        assert_eq!(cache.get(&key_a()), Some(&[1][..]));
    }

    #[test]
    fn second_load_compiles_zero() {
        fn tick(calls: &Cell<u32>) -> Result<Vec<u8>, String> {
            calls.set(calls.get() + 1);
            Ok(vec![9u8; 8])
        }
        let path = temp_path("warm");
        let calls = Cell::new(0u32);
        let mut cold = PipelineCache::new();
        assert!(matches!(
            cold.get_or_compile(key_a(), || tick(&calls)),
            WarmResult::Compiled(_)
        ));
        assert!(matches!(
            cold.get_or_compile(key_b(), || tick(&calls)),
            WarmResult::Compiled(_)
        ));
        assert_eq!(calls.get(), 2);
        cold.save(&path).expect("save");

        let mut warm = PipelineCache::load(&path).expect("load");
        std::fs::remove_file(&path).ok();
        assert!(matches!(
            warm.get_or_compile(key_a(), || tick(&calls)),
            WarmResult::Hit(_)
        ));
        assert!(matches!(
            warm.get_or_compile(key_b(), || tick(&calls)),
            WarmResult::Hit(_)
        ));
        assert_eq!(calls.get(), 2);
        assert_eq!(warm.get(&key_a()), Some(&[9u8; 8][..]));
    }

    #[test]
    fn failed_get_or_compile_records_diagnostic() {
        let mut cache = PipelineCache::new();
        let result = cache.get_or_compile(key_a(), || Err("backend down".to_string()));
        assert_eq!(result, WarmResult::Failed);
        assert_eq!(result.bytes(), None);
        assert_eq!(cache.diagnostic_count(), 1);
        assert!(cache.is_empty());
    }

    #[test]
    fn blob_and_key_bounds_refused() {
        let mut cache = PipelineCache::new();
        assert_eq!(
            cache.insert(key_a(), Vec::new()),
            Err(CacheError::EmptyBlob)
        );
        assert_eq!(
            cache.insert(key_a(), vec![0u8; MAX_BLOB_BYTES + 1]),
            Err(CacheError::TooLarge("blob bytes"))
        );
        let long = "x".repeat(MAX_DEFINE_BYTES + 1);
        let big_key = PipelineKey {
            shader_hash: 1,
            layout: 0,
            defines: vec![(long, "1".to_string())],
        };
        assert_eq!(
            cache.insert(big_key, vec![1]),
            Err(CacheError::TooLarge("define bytes"))
        );
        assert!(cache.is_empty());
    }

    #[test]
    fn diagnostics_bound_and_cap() {
        let mut cache = PipelineCache::new();
        for i in 0..(MAX_DIAGNOSTICS + 6) {
            let key = PipelineKey::new(b"s", &[("I", "0")], u64::from(i as u32));
            assert!(cache.submit_compile(&key));
            let message = format!("failure number {i} with extra detail");
            assert_eq!(
                cache.resolve_compile(&key, Err(message.into())),
                ResolveOutcome::Failed
            );
        }
        assert_eq!(cache.diagnostic_count(), MAX_DIAGNOSTICS);
        let long = "y".repeat(MAX_DIAG_CHARS + 40);
        cache.push_diagnostic(&key_a(), long);
        let last = cache.diagnostics().last().expect("last diagnostic");
        assert_eq!(last.message().chars().count(), MAX_DIAG_CHARS);
    }

    #[test]
    fn remove_drops_entry_and_pending() {
        let mut cache = PipelineCache::new();
        cache.insert(key_a(), vec![1]).expect("insert");
        assert!(cache.submit_compile(&key_a()));
        assert!(cache.remove(&key_a()));
        assert!(!cache.remove(&key_a()));
        assert_eq!(cache.get(&key_a()), None);
        assert_eq!(cache.poll_ready(&key_a()), PollState::Missing);
        assert_eq!(cache.pending_count(), 0);
    }

    #[test]
    fn warm_all_variants_then_walk_creates_zero_pipelines() {
        // The shipped forward-PBR variant set a scene can use at load.
        let keys = [
            PipelineKey::new(b"forward-pbr-shader", &[], 7),
            PipelineKey::new(b"forward-pbr-shader", &[("SHADOWS", "1")], 7),
            PipelineKey::new(b"forward-pbr-shader", &[("IBL", "1")], 7),
            PipelineKey::new(b"forward-pbr-shader", &[("IBL", "1"), ("SHADOWS", "1")], 7),
        ];
        fn compile(key: &PipelineKey) -> Result<Vec<u8>, CacheError> {
            Ok(vec![key.shader_hash.to_le_bytes()[0]; 8])
        }
        let mut cache = PipelineCache::new();
        assert!(!cache.walk_ready(&keys));
        // First load pass compiles every variant exactly once.
        let first = cache.warm_keys(&keys, compile);
        assert_eq!(
            first,
            WarmSummary {
                compiled: 4,
                hits: 0,
                failed: 0
            }
        );
        assert_eq!(first.total(), 4);
        assert!(cache.walk_ready(&keys));
        // Second pass over the warm cache compiles zero: the camera walk
        // across every material stays on hits, so no frame pays
        // pipeline-creation cost (creation ratio 0, under the 1.5x bar).
        let second = cache.warm_keys(&keys, compile);
        assert_eq!(
            second,
            WarmSummary {
                compiled: 0,
                hits: 4,
                failed: 0
            }
        );
        assert!(cache.walk_ready(&keys));
        // A failed variant counts loudly without touching the warm set.
        let mut cold = PipelineCache::new();
        let bad = cold.warm_keys(&keys[..1], |_| {
            Err(CacheError::Io("backend down".to_string()))
        });
        assert_eq!(bad.failed, 1);
        assert_eq!(cold.diagnostic_count(), 1);
        assert!(!cold.walk_ready(&keys[..1]));
    }

    /// RA17 storm set: every manifest variant (32, the gate) times 128 shader
    /// identities is 4096 keys, exactly [`MAX_ENTRIES`].
    fn storm_keys() -> Vec<PipelineKey> {
        use crate::variants::{VariantManifest, MAX_VARIANTS};
        let mut manifest = VariantManifest::new();
        for v in 0..MAX_VARIANTS {
            let flag = format!("V{v}");
            manifest
                .add_variant(&format!("storm/v{v:02}"), &[(flag.as_str(), "1")])
                .expect("32 variants fit the gate");
        }
        let mut keys = Vec::with_capacity(MAX_ENTRIES);
        for name in manifest.names() {
            let defines: Vec<(&str, &str)> = manifest
                .get(name)
                .expect("listed variant")
                .defines
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            for shader in 0..MAX_ENTRIES / MAX_VARIANTS {
                keys.push(PipelineKey::new(
                    format!("storm-shader-{shader}").as_bytes(),
                    &defines,
                    7,
                ));
            }
        }
        keys
    }

    fn storm_blob(key: &PipelineKey) -> Vec<u8> {
        key.shader_hash.to_le_bytes().to_vec()
    }

    #[test]
    fn variant_storm_gate_stops_at_32_variants_and_4096_entries() {
        use crate::variants::{VariantError, VariantManifest, MAX_VARIANTS};
        let mut manifest = VariantManifest::new();
        for v in 0..MAX_VARIANTS {
            manifest
                .add_variant(&format!("storm/v{v:02}"), &[])
                .expect("32 variants fit the gate");
        }
        assert_eq!(
            manifest.add_variant("storm/extra", &[]),
            Err(VariantError::OverCap {
                variant: "storm/extra".to_string(),
                cap: MAX_VARIANTS
            })
        );

        let keys = storm_keys();
        assert_eq!(keys.len(), MAX_ENTRIES);
        let distinct: BTreeSet<&PipelineKey> = keys.iter().collect();
        assert_eq!(distinct.len(), MAX_ENTRIES);

        // One load of 4097 keys: the first 4096 compile, the 4097th is refused
        // before its compile runs (it used to trip a debug assert).
        let mut over = keys.clone();
        over.push(PipelineKey::new(b"storm-shader-over", &[("V0", "1")], 7));
        let compiles = Cell::new(0usize);
        let mut cache = PipelineCache::new();
        let summary = cache.warm_keys(&over, |key| {
            compiles.set(compiles.get() + 1);
            Ok(storm_blob(key))
        });
        assert_eq!(
            summary,
            WarmSummary {
                compiled: MAX_ENTRIES,
                hits: 0,
                failed: 1
            }
        );
        assert_eq!((compiles.get(), cache.len()), (MAX_ENTRIES, MAX_ENTRIES));
        assert_eq!(cache.diagnostic_count(), 1);
        assert!(cache.diagnostics()[0].message().contains("cache full"));
        assert!(cache.walk_ready(&keys));
        assert!(!cache.contains(&over[MAX_ENTRIES]));
    }

    #[test]
    fn variant_storm_second_run_compiles_zero_even_after_a_restart() {
        let keys = storm_keys();
        let compiles = Cell::new(0usize);
        let compile = |key: &PipelineKey| {
            compiles.set(compiles.get() + 1);
            Ok(storm_blob(key))
        };
        let mut cache = PipelineCache::new();
        assert_eq!(cache.warm_keys(&keys, compile).compiled, MAX_ENTRIES);
        assert_eq!(compiles.get(), MAX_ENTRIES);

        // Second run in the same process: all hits, no compile.
        let again = cache.warm_keys(&keys, compile);
        assert_eq!(
            again,
            WarmSummary {
                compiled: 0,
                hits: MAX_ENTRIES,
                failed: 0
            }
        );
        assert_eq!(compiles.get(), MAX_ENTRIES);

        // Second run after a save and load: still no compile, no counted build.
        let bytes = cache.save_to_bytes().expect("save 4096 entries");
        let mut restarted = PipelineCache::load_from_bytes(&bytes).expect("load 4096 entries");
        assert_eq!(restarted.len(), MAX_ENTRIES);
        assert_eq!(restarted.save_to_bytes().expect("save again"), bytes);
        let warm = restarted.warm_keys(&keys, compile);
        assert_eq!((warm.compiled, warm.hits, warm.failed), (0, MAX_ENTRIES, 0));
        assert_eq!(compiles.get(), MAX_ENTRIES);
        assert_eq!(restarted.creation_total(), 0);
        assert!(restarted.walk_ready(&keys));
    }

    #[test]
    fn variant_storm_failed_refreshes_keep_every_last_good_blob() {
        let keys = storm_keys();
        let mut cache = PipelineCache::new();
        let first = cache.warm_keys(&keys, |key| Ok(storm_blob(key)));
        assert_eq!(first.compiled, MAX_ENTRIES);
        let before = cache.save_to_bytes().expect("save before the storm");

        // A driver update refreshes 200 keys and every compile fails (half
        // with an error, half with an empty blob): each key keeps serving.
        for (i, key) in keys.iter().take(200).enumerate() {
            assert!(cache.submit_compile(key));
            let result = if i % 2 == 0 {
                Err("driver flaked".into())
            } else {
                Ok(Vec::new())
            };
            assert_eq!(
                cache.resolve_compile(key, result),
                ResolveOutcome::KeptLastGood { bytes: 8 }
            );
        }
        assert_eq!(cache.save_to_bytes().expect("save after the storm"), before);
        assert_eq!(cache.diagnostic_count(), MAX_DIAGNOSTICS);
        assert_eq!(cache.pending_count(), 0);
        assert!(cache.walk_ready(&keys));

        // A good refresh then replaces just that blob.
        assert!(cache.submit_compile(&keys[0]));
        assert_eq!(
            cache.resolve_compile(&keys[0], Ok(vec![1, 2, 3])),
            ResolveOutcome::Compiled { bytes: 3 }
        );
        assert_eq!(cache.get(&keys[0]), Some(&[1, 2, 3][..]));
        assert_eq!(cache.get(&keys[1]), Some(storm_blob(&keys[1]).as_slice()));
        assert_eq!(cache.len(), MAX_ENTRIES);
    }

    #[test]
    fn cache_gate_dx12_serializes_eight_threads() {
        use std::sync::{Arc, Barrier};
        use std::time::Duration;
        let gate = Arc::new(PipelineGate::new(Backend::Dx12));
        let start = Arc::new(Barrier::new(8));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let gate = Arc::clone(&gate);
            let start = Arc::clone(&start);
            handles.push(std::thread::spawn(move || {
                start.wait();
                let _guard = gate.enter();
                std::thread::sleep(Duration::from_millis(5));
            }));
        }
        for handle in handles {
            handle.join().expect("dx12 worker joins");
        }
        assert_eq!(gate.peak(), 1, "dx12 holds at most one pipeline creation");
        assert_eq!(gate.live(), 0, "dx12 gate releases every guard");
    }

    #[test]
    fn cache_gate_vulkan_allows_parallel_eight_threads() {
        use std::sync::{Arc, Barrier};
        let gate = Arc::new(PipelineGate::new(Backend::Vulkan));
        let start = Arc::new(Barrier::new(8));
        let held = Arc::new(Barrier::new(8));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let gate = Arc::clone(&gate);
            let start = Arc::clone(&start);
            let held = Arc::clone(&held);
            handles.push(std::thread::spawn(move || {
                start.wait();
                let _guard = gate.enter();
                held.wait();
            }));
        }
        for handle in handles {
            handle.join().expect("vulkan worker joins");
        }
        assert_eq!(gate.peak(), 8, "vulkan holds all eight creations at once");
        assert_eq!(gate.live(), 0, "vulkan gate releases every guard");
    }

    #[test]
    fn cache_gate_gl_allows_parallel_eight_threads() {
        use std::sync::{Arc, Barrier};
        let gate = Arc::new(PipelineGate::new(Backend::Gl));
        let start = Arc::new(Barrier::new(8));
        let held = Arc::new(Barrier::new(8));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let gate = Arc::clone(&gate);
            let start = Arc::clone(&start);
            let held = Arc::clone(&held);
            handles.push(std::thread::spawn(move || {
                start.wait();
                let _guard = gate.enter();
                held.wait();
            }));
        }
        for handle in handles {
            handle.join().expect("gl worker joins");
        }
        assert_eq!(gate.peak(), 8, "gl holds all eight creations at once");
        assert_eq!(gate.live(), 0, "gl gate releases every guard");
    }

    #[test]
    fn cache_gate_releases_on_failed_compile() {
        let gate = PipelineGate::new(Backend::Dx12);
        let mut cache = PipelineCache::new();
        let result: WarmResult<'_> =
            cache.get_or_compile_gated(key_a(), &gate, || Err("backend down".to_string()));
        assert_eq!(result, WarmResult::Failed);
        assert_eq!(cache.diagnostic_count(), 1);
        assert_eq!(gate.live(), 0, "failed compile still releases the gate");
    }

    #[test]
    fn warm_keys_gated_routes_compiles_through_gate() {
        let gate = PipelineGate::new(Backend::Dx12);
        let mut cache = PipelineCache::new();
        let keys = [key_a(), key_b()];
        let first = cache.warm_keys_gated(&keys, &gate, |_| Ok(vec![9; 4]));
        assert_eq!(first.compiled, 2);
        assert_eq!(first.hits, 0);
        assert_eq!(gate.peak(), 1, "sequential gated warm holds one slot");
        assert_eq!(gate.live(), 0, "gated warm releases every slot");
        let second = cache.warm_keys_gated(&keys, &gate, |_| Ok(vec![9; 4]));
        assert_eq!(second.compiled, 0);
        assert_eq!(second.hits, 2);
        assert_eq!(gate.live(), 0);
    }

    #[test]
    fn warm_keys_for_routes_backend_gate() {
        let keys = [key_a(), key_b()];
        let mut dx = PipelineCache::new();
        let first = dx.warm_keys_for(&keys, Backend::Dx12, |_| Ok(vec![9; 4]));
        assert_eq!(first.compiled, 2);
        assert_eq!(first.hits, 0);
        let second = dx.warm_keys_for(&keys, Backend::Dx12, |_| Ok(vec![9; 4]));
        assert_eq!(second.compiled, 0);
        assert_eq!(second.hits, 2);
        let mut vk = PipelineCache::new();
        let vfirst = vk.warm_keys_for(&keys, Backend::Vulkan, |_| Ok(vec![9; 4]));
        assert_eq!(vfirst.compiled, 2);
        assert_eq!(vfirst.hits, 0);
    }

    #[test]
    fn cache_gate_try_enter_nonblocking() {
        for _ in 0..2 {
            let gate = PipelineGate::new(Backend::Dx12);
            let held = gate.try_enter();
            assert!(held.is_some(), "dx12 first try holds one slot");
            assert!(
                gate.try_enter().is_none(),
                "dx12 second try refuses while full"
            );
            assert_eq!(gate.live(), 1);
            drop(held);
            let again = gate.try_enter();
            assert!(again.is_some(), "dx12 try succeeds after release");
            assert_eq!(gate.live(), 1);
            drop(again);
            assert_eq!(gate.live(), 0);
            assert_eq!(gate.peak(), 1, "dx12 try never exceeds one");
        }
        let vgate = PipelineGate::new(Backend::Vulkan);
        let a = vgate.try_enter();
        let b = vgate.try_enter();
        assert!(a.is_some() && b.is_some(), "vulkan try always succeeds");
        assert_eq!(vgate.peak(), 2);
        drop(a);
        drop(b);
        assert_eq!(vgate.live(), 0);
    }

    #[test]
    fn backend_cap_dx12_is_one_others_uncapped() {
        // GS5: Dx12 serializes pipeline creation (one at a time);
        // Vulkan and Gl run creations in parallel (no cap).
        assert_eq!(max_in_flight(Backend::Dx12), Some(1));
        assert_eq!(max_in_flight(Backend::Vulkan), None);
        assert_eq!(max_in_flight(Backend::Gl), None);
    }

    #[test]
    fn pipeline_counter_material_walk_counts_four_causes_with_late_zero() {
        let keys = [
            PipelineKey::new(b"forward-pbr-shader", &[], 7),
            PipelineKey::new(b"forward-pbr-shader", &[("SHADOWS", "1")], 7),
            PipelineKey::new(b"forward-pbr-shader", &[("IBL", "1")], 7),
            PipelineKey::new(b"forward-pbr-shader", &[("IBL", "1"), ("SHADOWS", "1")], 7),
        ];
        let causes = [
            CreationCause::Mesh,
            CreationCause::Surface,
            CreationCause::Draw,
            CreationCause::Specialization,
        ];
        let mut cache = PipelineCache::new();
        for (index, (key, cause)) in keys.iter().zip(causes.iter()).enumerate() {
            let blob = vec![(index as u8) + 1; 4];
            cache
                .insert_caused(key.clone(), blob, *cause)
                .expect("caused insert lands");
        }
        assert_eq!(cache.creation_count(CreationCause::Mesh), 1);
        assert_eq!(cache.creation_count(CreationCause::Surface), 1);
        assert_eq!(cache.creation_count(CreationCause::Draw), 1);
        assert_eq!(cache.creation_count(CreationCause::Specialization), 1);
        assert_eq!(cache.creation_total(), 4);
        assert_eq!(cache.creation_counters().total(), 4);
        assert!(cache.walk_ready(&keys), "warm walk stays on hits");
        fn compile(key: &PipelineKey) -> Result<Vec<u8>, CacheError> {
            Ok(vec![key.shader_hash.to_le_bytes()[0]; 8])
        }
        let second = cache.warm_keys(&keys, compile);
        assert_eq!(second.compiled, 0, "late builds stay zero after load");
        assert_eq!(second.hits, 4);
        assert_eq!(cache.creation_total(), 4, "warm hits build nothing late");
        println!(
            "late=0 mesh={} surface={} draw={} specialization={} total={}",
            cache.creation_count(CreationCause::Mesh),
            cache.creation_count(CreationCause::Surface),
            cache.creation_count(CreationCause::Draw),
            cache.creation_count(CreationCause::Specialization),
            cache.creation_total(),
        );
    }

    #[test]
    fn pipeline_counter_failed_insert_counts_no_build() {
        let mut cache = PipelineCache::new();
        assert_eq!(
            cache.insert_caused(key_a(), Vec::new(), CreationCause::Mesh),
            Err(CacheError::EmptyBlob)
        );
        assert_eq!(cache.creation_count(CreationCause::Mesh), 0);
        assert_eq!(cache.creation_total(), 0);
        assert!(cache.is_empty());
    }
}
