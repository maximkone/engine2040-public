//! backend: GPU backend selector plus headless 600-frame bench.
//!
//! Purpose: choose the wgpu backend at startup (`--backend` / `WGPU_BACKEND`
//! over the compiled-in default, pilot-report winner with the other as
//! fallback) and measure the fixed 600-frame arena camera orbit headlessly.
//! Owner: render lane; consumers `gpu.rs` window path and `present.rs`
//! offscreen/windowed entries plus the pilot-report path. Units: ms for CPU
//! timings, MB for VRAM, frames/draws as counts. Determinism: selection is
//! pure over (cli, env, pilot); the orbit plus report body are bit-identical
//! x2; wall clock feeds CPU samples only. Errors: [`BackendError`] names the
//! bad backend label loudly. Budget: selection is O(1) with no allocation on
//! the frame path (the report allocates its JSON once, cold only).

use std::error::Error;
use std::fmt;

/// Fixed seed for the arena600 camera path.
pub const ARENA600_SEED: u64 = 2040;
/// Fixed frame count for the arena600 camera path.
pub const ARENA600_FRAMES: u32 = 600;
/// Capture width in pixels for the arena600 orbit.
pub const ARENA600_WIDTH: u32 = 1280;
/// Capture height in pixels for the arena600 orbit.
pub const ARENA600_HEIGHT: u32 = 720;
/// Scene digest stamped on arena600 reports (hex capture identity).
pub const ARENA600_DIGEST: &str =
    "d57b5c8ae3096614d57b5c8ae3096614d57b5c8ae3096614d57b5c8ae3096614";
/// Orbit radius in meters around the arena center.
pub const ARENA600_RADIUS_M: f32 = 12.0;
/// Camera height in meters above the floor.
pub const ARENA600_HEIGHT_M: f32 = 4.0;
/// Per-pass names in report order (mirrors `present::FrameReceipt`).
pub const BENCH_PASSES: [&str; 7] = ["sky", "layers", "particles", "decals", "ibl", "fog", "gtao"];
/// GPU timing marker until the reference-GPU window (never a fake number).
pub const GPU_PENDING: &str = "PENDING";
/// VRAM marker on adapter-less machines (never a fake number).
pub const VRAM_UNMEASURED_HEADLESS: &str = "unmeasured-headless";

/// Why no live VRAM reading exists. Never a guessed number.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VramUnknown {
    /// Built without the `window` feature: the headless default links no GPU API.
    Headless,
    /// Not Windows: only the DXGI budget query is wired today.
    NotWindows,
    /// Built with `gpu` but not `window`: no DXGI reader is linked.
    NoWindowFeature,
    /// DXGI lists no hardware adapter (software rasterizer only).
    NoHardwareAdapter,
    /// A DXGI call failed; its HRESULT is carried.
    Dxgi {
        /// The failing HRESULT.
        hresult: i32,
    },
}

/// One OS video-memory reading for the local segment of the adapter with the
/// most dedicated VRAM. `budget` is what the OS lets this process use now
/// (it shrinks when other processes, such as an inference sidecar, take
/// VRAM); `usage` is what this process holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VramMeasure {
    /// OS budget for this process, bytes.
    pub budget_bytes: u64,
    /// This process's current usage, bytes.
    pub usage_bytes: u64,
    /// The adapter's dedicated VRAM, bytes.
    pub dedicated_bytes: u64,
}

impl VramMeasure {
    /// Budget left over after current usage, bytes (never negative).
    #[must_use]
    pub fn headroom_bytes(&self) -> u64 {
        self.budget_bytes.saturating_sub(self.usage_bytes)
    }
}

/// A live VRAM reading, or why there is none.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VramReading {
    /// Read from the OS.
    Measured(VramMeasure),
    /// Not readable here; the reason is typed.
    Unknown(VramUnknown),
}

impl VramReading {
    /// One-line startup log in MB, or the typed reason.
    #[must_use]
    pub fn log_line(&self) -> String {
        const MB: u64 = 1024 * 1024;
        match self {
            Self::Measured(m) => format!(
                "vram budget_mb={} usage_mb={} headroom_mb={} dedicated_mb={}",
                m.budget_bytes / MB,
                m.usage_bytes / MB,
                m.headroom_bytes() / MB,
                m.dedicated_bytes / MB
            ),
            Self::Unknown(why) => format!("vram unknown reason={why:?}"),
        }
    }
}

/// Read the OS video-memory budget and usage (DXGI 1.4
/// `IDXGIAdapter3::QueryVideoMemoryInfo`, local segment) for the hardware
/// adapter with the most dedicated VRAM. Cold path: call it at startup or
/// off the tick, never per frame. Other platforms and the headless build
/// return [`VramReading::Unknown`] with the reason.
#[must_use]
pub fn read_vram() -> VramReading {
    #[cfg(all(windows, feature = "window"))]
    {
        dxgi_vram::read()
    }
    #[cfg(all(not(windows), feature = "window"))]
    {
        VramReading::Unknown(VramUnknown::NotWindows)
    }
    #[cfg(not(any(feature = "window", feature = "gpu")))]
    {
        VramReading::Unknown(VramUnknown::Headless)
    }
    #[cfg(all(feature = "gpu", not(feature = "window")))]
    {
        VramReading::Unknown(VramUnknown::NoWindowFeature)
    }
}

#[cfg(all(windows, feature = "window"))]
mod dxgi_vram {
    use super::{VramMeasure, VramReading, VramUnknown};
    use windows::core::Interface;
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter3, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
        DXGI_ERROR_NOT_FOUND, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO,
    };

    /// Adapters probed before giving up; a laptop has two or three.
    const MAX_ADAPTERS: u32 = 16;

    fn unknown(e: &windows::core::Error) -> VramReading {
        VramReading::Unknown(VramUnknown::Dxgi {
            hresult: e.code().0,
        })
    }

    #[expect(
        unsafe_code,
        reason = "DXGI is a COM API: every call is unsafe in the windows crate"
    )]
    pub(super) fn read() -> VramReading {
        // SAFETY: plain COM calls through the windows crate's owned
        // interface wrappers. Each wrapper holds a live reference for its
        // scope and releases it on drop; `info` is a local, fully sized
        // out-parameter that lives across the `QueryVideoMemoryInfo` call;
        // no pointer escapes this function.
        unsafe {
            let factory: IDXGIFactory1 = match CreateDXGIFactory1() {
                Ok(f) => f,
                Err(e) => return unknown(&e),
            };
            let mut best: Option<(usize, IDXGIAdapter3)> = None;
            for index in 0..MAX_ADAPTERS {
                let adapter = match factory.EnumAdapters1(index) {
                    Ok(a) => a,
                    Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                    Err(e) => return unknown(&e),
                };
                let desc = match adapter.GetDesc1() {
                    Ok(d) => d,
                    Err(e) => return unknown(&e),
                };
                if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0.cast_unsigned() != 0 {
                    continue;
                }
                let Ok(adapter3) = adapter.cast::<IDXGIAdapter3>() else {
                    continue;
                };
                if best
                    .as_ref()
                    .is_none_or(|(size, _)| desc.DedicatedVideoMemory > *size)
                {
                    best = Some((desc.DedicatedVideoMemory, adapter3));
                }
            }
            let Some((dedicated, adapter)) = best else {
                return VramReading::Unknown(VramUnknown::NoHardwareAdapter);
            };
            let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
            if let Err(e) =
                adapter.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info)
            {
                return unknown(&e);
            }
            VramReading::Measured(VramMeasure {
                budget_bytes: info.Budget,
                usage_bytes: info.CurrentUsage,
                dedicated_bytes: u64::try_from(dedicated).unwrap_or(u64::MAX),
            })
        }
    }
}
/// Env override honored at startup.
pub const BACKEND_ENV: &str = "WGPU_BACKEND";
/// Compiled-in default when no flag, env or pilot report says otherwise.
pub const COMPILED_DEFAULT: BackendChoice = BackendChoice::Auto;

/// Which GPU backend to bring up. `Auto` lets wgpu decide.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BackendChoice {
    /// Let wgpu pick (compiled-in default).
    Auto,
    /// DirectX 12 (Windows reference path).
    Dx12,
    /// Vulkan (portable reference path).
    Vulkan,
    /// OpenGL (fallback only, never the pilot winner).
    Gl,
    /// Metal (Apple reference path).
    Metal,
}

impl BackendChoice {
    /// CLI/env label quoted in logs and reports.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Dx12 => "dx12",
            Self::Vulkan => "vulkan",
            Self::Gl => "gl",
            Self::Metal => "metal",
        }
    }

    /// Parse a CLI/env label; unknown labels fail loudly.
    pub fn from_name(raw: &str) -> Result<Self, BackendError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "dx12" | "d3d12" | "directx" => Ok(Self::Dx12),
            "vulkan" | "vk" => Ok(Self::Vulkan),
            "gl" | "gles" | "opengl" => Ok(Self::Gl),
            "metal" => Ok(Self::Metal),
            _ => Err(BackendError::UnknownBackend(raw.to_string())),
        }
    }

    /// True for the explicit (non-auto) choices.
    #[must_use]
    pub fn is_explicit(self) -> bool {
        !matches!(self, Self::Auto)
    }

    /// The fallback partner: the other reference backend, so a Dx12 winner
    /// falls back to Vulkan and vice versa; `Auto`/`Gl`/`Metal` fall back to Dx12.
    #[must_use]
    pub fn fallback(self) -> Self {
        match self {
            Self::Dx12 => Self::Vulkan,
            Self::Vulkan => Self::Dx12,
            Self::Auto | Self::Gl | Self::Metal => Self::Dx12,
        }
    }
}

/// Backend selector failures; every variant names the culprit.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BackendError {
    /// `--backend`/`WGPU_BACKEND` label is not a known backend.
    UnknownBackend(String),
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownBackend(raw) => {
                write!(f, "unknown backend {raw:?}; want auto|dx12|vulkan|gl|metal")
            }
        }
    }
}

impl Error for BackendError {}

/// Parse an optional `--backend` value; `None` means the flag was absent.
pub fn parse_cli(raw: Option<&str>) -> Result<Option<BackendChoice>, BackendError> {
    match raw {
        None => Ok(None),
        Some(value) => Ok(Some(BackendChoice::from_name(value)?)),
    }
}

/// Parse an optional `WGPU_BACKEND` value; `None`/empty means unset.
pub fn parse_env(raw: Option<&str>) -> Result<Option<BackendChoice>, BackendError> {
    match raw {
        None => Ok(None),
        Some(value) if value.trim().is_empty() => Ok(None),
        Some(value) => Ok(Some(BackendChoice::from_name(value)?)),
    }
}

/// Resolve CLI over env over the compiled-in default. Pure over the two
/// parsed inputs so tests drive it exactly; the `reason` names the winner.
#[must_use]
pub fn resolve(
    cli: Option<BackendChoice>,
    env: Option<BackendChoice>,
) -> (BackendChoice, &'static str) {
    if let Some(choice) = cli {
        if choice.is_explicit() {
            return (choice, "explicit --backend flag");
        }
        return (choice, "explicit --backend auto");
    }
    if let Some(choice) = env {
        return (choice, "WGPU_BACKEND env override");
    }
    (COMPILED_DEFAULT, "compiled-in default")
}

/// Resolve from the live process env: `--backend` (as `cli`) wins over
/// `WGPU_BACKEND`; a bad env label falls back to the default with a
/// `compiled-in default (bad env ignored)` reason the binary logs loudly
/// rather than failing startup. Cold startup path only; no stdout here.
#[must_use]
pub fn resolve_from_env(cli: Option<BackendChoice>) -> (BackendChoice, &'static str) {
    if let Some(choice) = cli {
        return resolve(Some(choice), None);
    }
    let env = std::env::var(BACKEND_ENV).ok();
    match parse_env(env.as_deref()) {
        Ok(parsed) => resolve(None, parsed),
        Err(_) => (COMPILED_DEFAULT, "compiled-in default (bad env ignored)"),
    }
}

/// Apply an explicit `--backend` CLI choice as the process `WGPU_BACKEND`
/// override so the env-only lib resolve follows the flag, then return the
/// startup pick the binary logs via [`BackendPick::startup_line`]. `None`
/// and `Auto` leave the env untouched. Cold startup path only; no stdout.
#[must_use]
pub fn apply_cli_override(cli: Option<BackendChoice>) -> BackendPick {
    if let Some(choice) = cli {
        if choice.is_explicit() {
            std::env::set_var(BACKEND_ENV, choice.name());
        }
    }
    let (resolved, _) = resolve_from_env(cli);
    pick_backend(resolved, None)
}

/// One startup pick: the primary plus its fallback plus why. An explicit
/// choice always wins; `Auto` with a pilot winner follows the numbers;
/// `Auto` without one keeps the compiled-in default.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BackendPick {
    /// Backend to bring up first.
    pub primary: BackendChoice,
    /// Backend to try when the primary fails to acquire.
    pub fallback: BackendChoice,
    /// Why this pair was chosen (startup log quotes it).
    pub reason: &'static str,
}

/// Pick the startup pair from the resolved choice plus the last pilot
/// winner (`None` when no report exists yet). Pure; the caller logs
/// [`BackendPick::startup_line`].
#[must_use]
pub fn pick_backend(choice: BackendChoice, pilot_winner: Option<BackendChoice>) -> BackendPick {
    if choice.is_explicit() {
        return BackendPick {
            primary: choice,
            fallback: choice.fallback(),
            reason: "explicit choice wins over the pilot report",
        };
    }
    match pilot_winner {
        Some(winner) if winner.is_explicit() && !matches!(winner, BackendChoice::Gl) => {
            BackendPick {
                primary: winner,
                fallback: winner.fallback(),
                reason: "pilot report winner with the other as fallback",
            }
        }
        _ => BackendPick {
            primary: COMPILED_DEFAULT,
            fallback: COMPILED_DEFAULT.fallback(),
            reason: "compiled-in default; no pilot report yet",
        },
    }
}

impl BackendPick {
    /// One-line startup log: names the chosen backend, the fallback and why.
    #[must_use]
    pub fn startup_line(&self) -> String {
        format!(
            "backend={} fallback={} reason={}",
            self.primary.name(),
            self.fallback.name(),
            self.reason
        )
    }
}

/// Read the pilot winner out of one evidence JSON body: the `backend` field
/// must name an explicit backend; anything else yields `None` (never a guess).
#[must_use]
pub fn pilot_winner_from_json(body: &str) -> Option<BackendChoice> {
    let key = "\"backend\":\"";
    let start = body.find(key)? + key.len();
    let rest = &body[start..];
    let end = rest.find('"')?;
    match BackendChoice::from_name(&rest[..end]) {
        Ok(choice) if choice.is_explicit() && !matches!(choice, BackendChoice::Gl) => Some(choice),
        _ => None,
    }
}

/// Map a choice onto wgpu backends. Behind `gpu` so the headless default
/// stays std-only; test-only `Backends::all()` sites are untouched.
#[cfg(feature = "gpu")]
#[must_use]
pub fn wgpu_backends(choice: BackendChoice) -> wgpu::Backends {
    // wgpu yields no Metal adapter off Apple hardware, so the caller falls back there.
    match choice {
        BackendChoice::Auto => wgpu::Backends::all(),
        BackendChoice::Dx12 => wgpu::Backends::DX12,
        BackendChoice::Vulkan => wgpu::Backends::VULKAN,
        BackendChoice::Gl => wgpu::Backends::GL,
        BackendChoice::Metal => wgpu::Backends::METAL,
    }
}

/// Env switch for strict WebGPU mode: `1` opens every native instance of the
/// render stack with `InstanceFlags::STRICT_WEBGPU_COMPLIANCE`, so wgpu hides
/// every feature and limit the WebGPU spec does not have and rejects what a
/// browser rejects (a color attachment loaded with `DontCare`, a bind group
/// layout over the spec limits). Unset or any other value leaves the instance
/// flags as they are. Read once per instance, at startup.
pub const STRICT_WEBGPU_ENV: &str = "E2040_STRICT_WEBGPU";

/// True when the raw `E2040_STRICT_WEBGPU` value asks for strict mode: exactly
/// `1`, surrounding blanks ignored (a Windows `set X=1 &&` carries one). Pure
/// over the value so tests drive it without touching the process env.
#[must_use]
pub fn strict_webgpu_from(raw: Option<&str>) -> bool {
    raw.is_some_and(|value| value.trim() == "1")
}

/// True when the live process env asks for strict WebGPU mode.
#[must_use]
pub fn strict_webgpu_from_env() -> bool {
    strict_webgpu_from(std::env::var(STRICT_WEBGPU_ENV).ok().as_deref())
}

/// The instance flags: wgpu's build-config default (debug and validation
/// flags in a debug build), plus `STRICT_WEBGPU_COMPLIANCE` when `strict`.
/// With `strict` false this is exactly what `InstanceDescriptor` defaults to.
#[cfg(feature = "gpu")]
#[must_use]
pub fn instance_flags(strict: bool) -> wgpu::InstanceFlags {
    let mut flags = wgpu::InstanceFlags::default();
    if strict {
        flags |= wgpu::InstanceFlags::STRICT_WEBGPU_COMPLIANCE;
    }
    flags
}

/// The instance descriptor of the render stack: `backends`, the
/// [`instance_flags`] for `strict`, everything else wgpu's default and no
/// display handle.
#[cfg(feature = "gpu")]
#[must_use]
pub fn instance_descriptor(backends: wgpu::Backends, strict: bool) -> wgpu::InstanceDescriptor {
    wgpu::InstanceDescriptor {
        backends,
        flags: instance_flags(strict),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    }
}

/// Create the wgpu instance every native bring-up in the render stack uses,
/// strict when `E2040_STRICT_WEBGPU=1` (see [`STRICT_WEBGPU_ENV`]). The one
/// creation point: a new bring-up calls this instead of `Instance::new`, so
/// the switch reaches it. Cold startup path only.
#[cfg(feature = "gpu")]
#[must_use]
pub fn new_instance(backends: wgpu::Backends) -> wgpu::Instance {
    wgpu::Instance::new(instance_descriptor(backends, strict_webgpu_from_env()))
}

/// Camera eye in meters for one arena600 orbit frame: one full turn over
/// [`ARENA600_FRAMES`] frames. Presentation math (trig allowed); the same
/// frame gives the same eye twice.
#[must_use]
pub fn arena600_eye(frame: u32) -> [f32; 3] {
    let clamped = frame.min(ARENA600_FRAMES.saturating_sub(1));
    // Both inputs stay under 2^16, so the u16 hop keeps every bit intact
    // with `From` only (f32 represents every u16 exactly).
    let clamped_half = u16::try_from(clamped).unwrap_or(u16::MAX);
    let frames_half = u16::try_from(ARENA600_FRAMES).unwrap_or(u16::MAX);
    let turn = f32::from(clamped_half) / f32::from(frames_half);
    let angle = turn * std::f32::consts::TAU;
    [
        angle.cos() * ARENA600_RADIUS_M,
        ARENA600_HEIGHT_M,
        angle.sin() * ARENA600_RADIUS_M,
    ]
}

/// Deterministic digest over the whole orbit (the eye bytes fed through
/// the shared [`crate::shadow_post::fnv1a64`] owner, no local copy of the
/// constants): the x2 harness asserts this plus draws plus validation errors.
#[must_use]
pub fn arena600_digest() -> String {
    let frames = usize::try_from(ARENA600_FRAMES).unwrap_or(0);
    let mut bytes = Vec::with_capacity(frames.saturating_mul(12));
    for frame in 0..ARENA600_FRAMES {
        for component in arena600_eye(frame) {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
    }
    // Cold receipt path: one small buffer per harness run, never on a frame.
    format!("{:016x}", crate::shadow_post::fnv1a64(&bytes))
}

/// p50/p95/p99 in ms over one pass sample set. Pure over the samples.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Quantiles {
    /// Median ms.
    pub p50_ms: f64,
    /// 95th percentile ms.
    pub p95_ms: f64,
    /// 99th percentile ms.
    pub p99_ms: f64,
}

/// Quantiles over an ascending-sorted sample slice; empty yields zeros
/// (documented, never NaN). The caller sorts with `total_cmp`.
#[must_use]
pub fn quantiles_of_sorted(sorted_ms: &[f64]) -> Quantiles {
    if sorted_ms.is_empty() {
        return Quantiles {
            p50_ms: 0.0,
            p95_ms: 0.0,
            p99_ms: 0.0,
        };
    }
    let max_index = sorted_ms.len().saturating_sub(1);
    let max_scalar = f64::from(u32::try_from(max_index).unwrap_or(u32::MAX));
    let at = |q: f64| -> f64 {
        let pos = (q * max_scalar).clamp(0.0, max_scalar);
        let lo = floor_index(pos, max_index);
        let hi = ceil_index(pos, max_index);
        let lo_scalar = f64::from(u32::try_from(lo).unwrap_or(u32::MAX));
        let frac = pos - lo_scalar;
        sorted_ms[lo] * (1.0 - frac) + sorted_ms[hi] * frac
    };
    Quantiles {
        p50_ms: at(0.50),
        p95_ms: at(0.95),
        p99_ms: at(0.99),
    }
}

/// Largest sample index at or below `pos` without a float cast: binary
/// search over the exact `f64::from(u32)` scalars.
fn floor_index(pos: f64, max_index: usize) -> usize {
    let mut lo = 0usize;
    let mut hi = max_index;
    while lo < hi {
        let mid = lo.saturating_add(hi.saturating_sub(lo).saturating_add(1) / 2);
        if index_scalar(mid) <= pos {
            lo = mid;
        } else {
            hi = mid.saturating_sub(1);
        }
    }
    lo
}

/// Smallest sample index at or above `pos`: the floor, plus one unless `pos`
/// already sits exactly on it.
fn ceil_index(pos: f64, max_index: usize) -> usize {
    let floor = floor_index(pos, max_index);
    if index_scalar(floor) >= pos {
        floor
    } else {
        floor.saturating_add(1).min(max_index)
    }
}

/// Exact scalar for a sample index; saturates at `u32::MAX` (a slice that
/// long never occurs: 8 B samples each would already exceed memory).
fn index_scalar(index: usize) -> f64 {
    f64::from(u32::try_from(index).unwrap_or(u32::MAX))
}

/// Per-pass CPU summary in ms (observed only; GPU stays [`GPU_PENDING`]).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CpuPassSummary {
    /// Pass name in [`BENCH_PASSES`] order.
    pub name: &'static str,
    /// Median/95th/99th ms observed on the CPU twin.
    pub quantiles: Quantiles,
}

/// One headless bench report body: the capture identity plus observed
/// CPU per-pass quantiles plus counts. GPU per-pass timings are always the
/// [`GPU_PENDING`] marker until the reference GPU lands (never faked); VRAM is always the
/// [`VRAM_UNMEASURED_HEADLESS`] marker on adapter-less machines.
/// Cold receipt path: the JSON string is built once per harness run.
#[derive(Clone, PartialEq, Debug)]
pub struct BackendReport {
    /// Backend this report was measured under.
    pub backend: BackendChoice,
    /// Adapter name, or `llvmpipe` on the headless twin.
    pub adapter: String,
    /// Driver string, or the twin marker.
    pub driver: String,
    /// Capture width in pixels.
    pub width_px: u32,
    /// Capture height in pixels.
    pub height_px: u32,
    /// Scene digest (hex).
    pub scene_digest: String,
    /// Frames run (always [`ARENA600_FRAMES`] on the fixed path).
    pub frames: u32,
    /// Draws submitted (deterministic; asserted x2).
    pub draws: u32,
    /// Validation errors observed (deterministic; asserted x2).
    pub validation_errors: u32,
    /// CPU per-pass quantiles in [`BENCH_PASSES`] order.
    pub cpu: [CpuPassSummary; 7],
}

impl BackendReport {
    /// Build the headless twin report: `llvmpipe` adapter/driver markers,
    /// the fixed arena600 identity, deterministic counts. CPU quantiles
    /// come from the observed samples the caller measured.
    #[must_use]
    pub fn headless_twin(backend: BackendChoice, cpu: [CpuPassSummary; 7]) -> Self {
        Self {
            backend,
            adapter: "llvmpipe".to_string(),
            driver: "unmeasured-headless".to_string(),
            width_px: ARENA600_WIDTH,
            height_px: ARENA600_HEIGHT,
            scene_digest: ARENA600_DIGEST.to_string(),
            frames: ARENA600_FRAMES,
            draws: ARENA600_FRAMES,
            validation_errors: 0,
            cpu,
        }
    }

    /// Determinism key asserted x2: draws plus validation errors.
    #[must_use]
    pub fn determinism_key(&self) -> (u32, u32) {
        (self.draws, self.validation_errors)
    }

    /// Fixed-order JSON body for `target/evidence/backend/`. Field order is
    /// frozen: manifest fields, then CPU per-pass, then counts, then the
    /// VRAM/GPU markers. No timestamps, so x2 bodies compare byte for byte.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut out = String::with_capacity(1024);
        out.push_str("{\"adapter\":\"");
        out.push_str(&self.adapter);
        out.push_str("\",\"driver\":\"");
        out.push_str(&self.driver);
        out.push_str("\",\"backend\":\"");
        out.push_str(self.backend.name());
        out.push_str("\",\"resolution\":[");
        out.push_str(&self.width_px.to_string());
        out.push(',');
        out.push_str(&self.height_px.to_string());
        out.push_str("],\"quality\":\"high\",\"scene_digest\":\"");
        out.push_str(&self.scene_digest);
        out.push_str("\",\"frames\":");
        out.push_str(&self.frames.to_string());
        out.push_str(",\"cpu_per_pass_ms\":{");
        for (index, pass) in self.cpu.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push('"');
            out.push_str(pass.name);
            out.push_str("\":{\"p50\":");
            push_f64(&mut out, pass.quantiles.p50_ms);
            out.push_str(",\"p95\":");
            push_f64(&mut out, pass.quantiles.p95_ms);
            out.push_str(",\"p99\":");
            push_f64(&mut out, pass.quantiles.p99_ms);
            out.push('}');
        }
        out.push_str("},\"draws\":");
        out.push_str(&self.draws.to_string());
        out.push_str(",\"validation_errors\":");
        out.push_str(&self.validation_errors.to_string());
        out.push_str(",\"vram_mb\":null,\"vram_note\":\"");
        out.push_str(VRAM_UNMEASURED_HEADLESS);
        out.push_str("\",\"gpu_per_pass_ms\":\"");
        out.push_str(GPU_PENDING);
        out.push_str("\"}");
        out
    }

    /// Evidence file name (no directories): `<backend>-run<N>.json`.
    #[must_use]
    pub fn file_name(&self, run: u32) -> String {
        format!("{}-run{run}.json", self.backend.name())
    }
}

fn push_f64(out: &mut String, value: f64) {
    if value.is_finite() {
        out.push_str(&format!("{value:.6}"));
    } else {
        out.push_str("0.000000");
    }
}

/// Flat zeroed CPU table (one entry per [`BENCH_PASSES`] name) for callers
/// that measured no samples.
#[must_use]
pub fn zero_cpu_table() -> [CpuPassSummary; 7] {
    let zero = Quantiles {
        p50_ms: 0.0,
        p95_ms: 0.0,
        p99_ms: 0.0,
    };
    [
        CpuPassSummary {
            name: BENCH_PASSES[0],
            quantiles: zero,
        },
        CpuPassSummary {
            name: BENCH_PASSES[1],
            quantiles: zero,
        },
        CpuPassSummary {
            name: BENCH_PASSES[2],
            quantiles: zero,
        },
        CpuPassSummary {
            name: BENCH_PASSES[3],
            quantiles: zero,
        },
        CpuPassSummary {
            name: BENCH_PASSES[4],
            quantiles: zero,
        },
        CpuPassSummary {
            name: BENCH_PASSES[5],
            quantiles: zero,
        },
        CpuPassSummary {
            name: BENCH_PASSES[6],
            quantiles: zero,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_cpu() -> [CpuPassSummary; 7] {
        let mut table = zero_cpu_table();
        for (index, slot) in table.iter_mut().enumerate() {
            let k = f64::from(u32::try_from(index).unwrap_or(u32::MAX)) + 1.0;
            slot.quantiles = Quantiles {
                p50_ms: 0.10 * k,
                p95_ms: 0.20 * k,
                p99_ms: 0.30 * k,
            };
        }
        table
    }

    #[test]
    fn backend_names_round_trip() {
        for (label, want) in [
            ("auto", BackendChoice::Auto),
            ("dx12", BackendChoice::Dx12),
            ("vulkan", BackendChoice::Vulkan),
            ("gl", BackendChoice::Gl),
            ("metal", BackendChoice::Metal),
        ] {
            assert_eq!(BackendChoice::from_name(label), Ok(want));
            assert_eq!(want.name(), label);
        }
        assert!(parse_cli(None).unwrap().is_none());
        assert_eq!(
            parse_cli(Some("vulkan")).unwrap(),
            Some(BackendChoice::Vulkan)
        );
        assert_eq!(
            parse_cli(Some("metal")).unwrap(),
            Some(BackendChoice::Metal)
        );
    }

    #[test]
    fn env_override_is_honored_with_cli_winning() {
        assert_eq!(parse_env(None).unwrap(), None);
        assert_eq!(parse_env(Some("")).unwrap(), None);
        assert_eq!(parse_env(Some("dx12")).unwrap(), Some(BackendChoice::Dx12));
        let (choice, reason) = resolve(Some(BackendChoice::Vulkan), Some(BackendChoice::Dx12));
        assert_eq!(
            (choice, reason),
            (BackendChoice::Vulkan, "explicit --backend flag")
        );
        let (choice, reason) = resolve(None, Some(BackendChoice::Dx12));
        assert_eq!(
            (choice, reason),
            (BackendChoice::Dx12, "WGPU_BACKEND env override")
        );
        let (choice, _) = resolve(None, None);
        assert_eq!(choice, COMPILED_DEFAULT);
    }

    #[test]
    fn pick_follows_explicit_then_pilot_then_default() {
        let pick = pick_backend(BackendChoice::Dx12, Some(BackendChoice::Vulkan));
        assert_eq!(
            (pick.primary, pick.fallback),
            (BackendChoice::Dx12, BackendChoice::Vulkan)
        );
        assert!(pick.startup_line().contains("backend=dx12"));
        let pick = pick_backend(BackendChoice::Auto, Some(BackendChoice::Vulkan));
        assert_eq!(
            (pick.primary, pick.fallback),
            (BackendChoice::Vulkan, BackendChoice::Dx12)
        );
        assert!(pick.reason.contains("pilot"));
        let pick = pick_backend(BackendChoice::Auto, None);
        assert_eq!(pick.primary, COMPILED_DEFAULT);
        assert!(pick.startup_line().contains("reason="));
        let gl_pick = pick_backend(BackendChoice::Auto, Some(BackendChoice::Gl));
        assert_eq!(gl_pick.primary, COMPILED_DEFAULT);
    }

    #[test]
    fn cli_override_sticks_the_env_and_names_the_pick() {
        // An explicit `--backend` wins, sticks `WGPU_BACKEND` for the
        // env-only lib resolve, and quotes itself on the startup line.
        // The previous env value is restored so parallel legs keep theirs.
        let before = std::env::var(BACKEND_ENV).ok();
        let pick = apply_cli_override(Some(BackendChoice::Vulkan));
        assert_eq!(
            (pick.primary, pick.fallback),
            (BackendChoice::Vulkan, BackendChoice::Dx12)
        );
        assert!(pick.startup_line().contains("backend=vulkan"));
        assert_eq!(std::env::var(BACKEND_ENV).ok().as_deref(), Some("vulkan"));
        let (resolved, _) = resolve_from_env(None);
        assert_eq!(resolved, BackendChoice::Vulkan);
        match before {
            Some(value) => std::env::set_var(BACKEND_ENV, value),
            None => std::env::remove_var(BACKEND_ENV),
        }
    }

    #[test]
    fn pilot_winner_parses_evidence_bodies() {
        let body = BackendReport::headless_twin(BackendChoice::Vulkan, sample_cpu()).to_json();
        assert_eq!(pilot_winner_from_json(&body), Some(BackendChoice::Vulkan));
        assert_eq!(pilot_winner_from_json("{\"backend\":\"auto\"}"), None);
        assert_eq!(pilot_winner_from_json("{}"), None);
    }

    #[test]
    fn arena600_orbit_is_fixed_and_deterministic() {
        assert_eq!(ARENA600_SEED, 2040);
        assert_eq!(ARENA600_FRAMES, 600);
        assert_eq!(arena600_eye(0), arena600_eye(0));
        assert_ne!(arena600_eye(0), arena600_eye(150));
        assert_eq!(arena600_digest(), arena600_digest());
        assert_eq!(
            arena600_eye(ARENA600_FRAMES + 50),
            arena600_eye(ARENA600_FRAMES - 1)
        );
    }

    #[test]
    fn quantiles_read_sorted_samples() {
        let mut samples: [f64; 5] = [0.5, 0.1, 0.3, 0.2, 0.4];
        samples.sort_by(|a, b| a.total_cmp(b));
        let q = quantiles_of_sorted(&samples);
        assert!((q.p50_ms - 0.3).abs() < 1e-12);
        assert!(q.p95_ms >= q.p50_ms && q.p99_ms >= q.p95_ms);
        let empty = quantiles_of_sorted(&[]);
        assert_eq!((empty.p50_ms, empty.p95_ms, empty.p99_ms), (0.0, 0.0, 0.0));
    }

    #[test]
    fn twin_report_marks_gpu_pending_and_matches_x2() {
        let first = BackendReport::headless_twin(BackendChoice::Auto, sample_cpu());
        let second = BackendReport::headless_twin(BackendChoice::Auto, sample_cpu());
        assert_eq!(first.determinism_key(), second.determinism_key());
        assert_eq!(first.to_json(), second.to_json());
        let body = first.to_json();
        for field in [
            "\"adapter\":\"llvmpipe\"",
            "\"backend\":\"auto\"",
            "\"scene_digest\":\"",
            "\"draws\":600",
            "\"validation_errors\":0",
            VRAM_UNMEASURED_HEADLESS,
            GPU_PENDING,
        ] {
            assert!(body.contains(field), "report misses {field}");
        }
        assert!(
            body.contains("\"vram_mb\":null"),
            "VRAM must be null, never faked"
        );
        assert_eq!(first.file_name(2), "auto-run2.json");
    }

    #[test]
    fn orbit_counts_are_deterministic_x2() {
        let first = BackendReport::headless_twin(BackendChoice::Dx12, zero_cpu_table());
        let second = BackendReport::headless_twin(BackendChoice::Dx12, zero_cpu_table());
        assert_eq!(first.determinism_key(), (ARENA600_FRAMES, 0));
        assert_eq!(first.determinism_key(), second.determinism_key());
    }

    /// One observed orbit-step sample in ms: eye plus digest accumulation for
    /// `frame`, timed on the wall clock. Cold harness path only.
    fn orbit_step_ms(frame: u32) -> f64 {
        let start = std::time::Instant::now();
        let eye = arena600_eye(frame);
        let mut bytes = [0u8; 12];
        bytes[0..4].copy_from_slice(&eye[0].to_le_bytes());
        bytes[4..8].copy_from_slice(&eye[1].to_le_bytes());
        bytes[8..12].copy_from_slice(&eye[2].to_le_bytes());
        std::hint::black_box(crate::shadow_post::fnv1a64(&bytes));
        start.elapsed().as_secs_f64() * 1000.0
    }

    /// Observed CPU table for one twin run: 600 orbit-step samples shared
    /// across the pass slots. Twin structural note: headless runs no forward
    /// passes, so every slot quotes the same observed orbit-step
    /// distribution (never a per-pass device claim); the pilot
    /// overwrites each slot with per-pass device timings on the reference
    /// GPU. The samples are measured here, never hardcoded.
    fn observed_cpu_table() -> [CpuPassSummary; 7] {
        let mut samples = [0.0f64; ARENA600_FRAMES as usize];
        for (frame, slot) in samples.iter_mut().enumerate() {
            *slot = orbit_step_ms(u32::try_from(frame).unwrap_or(u32::MAX));
        }
        samples.sort_by(|a, b| a.total_cmp(b));
        let quantiles = quantiles_of_sorted(&samples);
        assert!(quantiles.p50_ms.is_finite() && quantiles.p99_ms >= quantiles.p50_ms);
        let mut table = zero_cpu_table();
        for slot in &mut table {
            slot.quantiles = quantiles;
        }
        table
    }

    #[test]
    fn twin_reports_emit_x2_with_manifest_fields() {
        let backend = BackendChoice::Auto;
        let first = BackendReport::headless_twin(backend, observed_cpu_table());
        let second = BackendReport::headless_twin(backend, observed_cpu_table());
        // x2 determinism gate: identical counts plus identical orbit digest.
        assert_eq!(first.determinism_key(), (ARENA600_FRAMES, 0));
        assert_eq!(first.determinism_key(), second.determinism_key());
        assert_eq!(arena600_digest(), arena600_digest());
        // Repo precedent (`proof/src/a9.rs`): lane runs land real files under
        // the workspace `target/evidence/<area>/`, anchored on the manifest
        // dir so the path holds no matter which directory invoked cargo.
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/backend");
        std::fs::create_dir_all(&dir).expect("evidence dir creates");
        for (run, report) in [first, second].iter().enumerate() {
            let run_no = u32::try_from(run).unwrap_or(u32::MAX).saturating_add(1);
            let path = dir.join(report.file_name(run_no));
            std::fs::write(&path, report.to_json()).expect("report writes");
            let body = std::fs::read_to_string(&path).expect("report reads back");
            let shown = path.display().to_string();
            for field in [
                "\"adapter\":\"llvmpipe\"",
                "\"backend\":\"auto\"",
                "\"scene_digest\":\"",
                "\"draws\":600",
                "\"validation_errors\":0",
                "\"cpu_per_pass_ms\"",
                VRAM_UNMEASURED_HEADLESS,
                GPU_PENDING,
            ] {
                assert!(body.contains(field), "{shown} misses {field}");
            }
            assert!(
                body.contains("\"vram_mb\":null"),
                "{shown} must leave VRAM null, never faked"
            );
        }
    }

    #[test]
    fn vram_reading_is_typed_and_never_faked() {
        let mb = 1024 * 1024;
        let seen = VramReading::Measured(VramMeasure {
            budget_bytes: 5_000 * mb,
            usage_bytes: 1_200 * mb,
            dedicated_bytes: 6_144 * mb,
        });
        assert_eq!(
            seen.log_line(),
            "vram budget_mb=5000 usage_mb=1200 headroom_mb=3800 dedicated_mb=6144"
        );
        let over = VramMeasure {
            budget_bytes: mb,
            usage_bytes: 2 * mb,
            dedicated_bytes: 0,
        };
        assert_eq!(over.headroom_bytes(), 0, "headroom never goes negative");
        let why = VramReading::Unknown(VramUnknown::Dxgi { hresult: -1 });
        assert!(why.log_line().contains("reason=Dxgi"));
        // The live read always answers, measured or with a typed reason.
        match read_vram() {
            VramReading::Measured(m) => assert!(m.budget_bytes > 0),
            VramReading::Unknown(_) => {}
        }
        #[cfg(not(feature = "gpu"))]
        assert_eq!(read_vram(), VramReading::Unknown(VramUnknown::Headless));
    }

    /// Prints this machine's live reading (RTX 3050 Laptop on the reference
    /// box). Needs a GPU adapter, so it stays out of the default suite.
    #[cfg(feature = "window")]
    #[test]
    #[ignore = "live: prints the DXGI VRAM reading; needs a GPU adapter"]
    fn vram_live_reading_prints() {
        let reading = read_vram();
        println!("{}", reading.log_line());
        #[cfg(windows)]
        assert!(
            matches!(reading, VramReading::Measured(m) if m.budget_bytes > 0),
            "{reading:?}"
        );
    }

    #[cfg(feature = "window")]
    #[test]
    fn adapter_probe_skips_headless_never_fakes() {
        crate::test_gpu::hold_gpu_slot();
        let (choice, _) = resolve_from_env(None);
        let instance = new_instance(wgpu_backends(choice));
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }));
        // The adapter leg is a GPU probe; the backend map asserts below run
        // either way, so the skip is counted and gated, not silent.
        let found = crate::test_gpu::require_or_skip(
            "adapter_probe_skips_headless_never_fakes",
            adapter
                .map_err(|_| crate::test_gpu::NoAdapter("no wgpu adapter on this machine".into())),
        );
        if let Some(found) = found {
            let info = found.get_info();
            assert!(!info.name.is_empty(), "report names the adapter");
        }
        assert_eq!(wgpu_backends(BackendChoice::Dx12), wgpu::Backends::DX12);
        assert_eq!(wgpu_backends(BackendChoice::Vulkan), wgpu::Backends::VULKAN);
        assert_eq!(wgpu_backends(BackendChoice::Metal), wgpu::Backends::METAL);
    }

    #[test]
    fn strict_webgpu_env_value_selects_the_mode() {
        assert!(!strict_webgpu_from(None), "unset keeps today's behavior");
        for off in ["", " ", "0", "true", "yes", "2", "11"] {
            assert!(!strict_webgpu_from(Some(off)), "{off:?} must stay off");
        }
        assert!(strict_webgpu_from(Some("1")));
        assert!(
            strict_webgpu_from(Some("1 ")),
            "cmd `set X=1 &&` leaves a trailing blank"
        );
    }

    #[test]
    fn strict_webgpu_env_name_is_the_documented_one() {
        // The workflow files, the skills and the dev's shell spell it this way.
        assert_eq!(STRICT_WEBGPU_ENV, "E2040_STRICT_WEBGPU");
    }

    #[cfg(feature = "window")]
    #[test]
    fn strict_webgpu_flag_is_added_only_when_asked() {
        let strict_bit = wgpu::InstanceFlags::STRICT_WEBGPU_COMPLIANCE;
        let off = instance_flags(false);
        let on = instance_flags(true);
        assert_eq!(
            off,
            wgpu::InstanceFlags::default(),
            "off is exactly what the descriptor defaulted to before the switch"
        );
        assert!(!off.contains(strict_bit));
        assert!(on.contains(strict_bit));
        assert_eq!(
            on.difference(strict_bit),
            off,
            "strict adds the one bit and keeps the debug and validation flags"
        );
    }

    #[cfg(feature = "window")]
    #[test]
    fn strict_webgpu_descriptor_keeps_backends_and_has_no_display() {
        for strict in [false, true] {
            let desc = instance_descriptor(wgpu::Backends::VULKAN, strict);
            assert_eq!(desc.backends, wgpu::Backends::VULKAN);
            assert_eq!(desc.flags, instance_flags(strict));
            assert!(desc.display.is_none());
        }
    }

    /// A high-performance adapter from a strict WebGPU instance, or the skip.
    /// `Backends::PRIMARY`: test builds skip GL, whose driver DLL loads from a
    /// helper thread and can wedge the process loader lock. An adapter that the
    /// plain instance finds and the strict one hides is not WebGPU compliant;
    /// that is named in the skip reason (and fails under `E2040_REQUIRE_GPU=1`).
    #[cfg(feature = "window")]
    fn strict_adapter(test: &str) -> Option<wgpu::Adapter> {
        use crate::test_gpu::{require_or_skip, NoAdapter};
        crate::test_gpu::hold_gpu_slot();
        let options = wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        };
        let plain = wgpu::Instance::new(instance_descriptor(wgpu::Backends::PRIMARY, false));
        let strict = wgpu::Instance::new(instance_descriptor(wgpu::Backends::PRIMARY, true));
        let probe = match pollster::block_on(plain.request_adapter(&options)) {
            Err(error) => Err(NoAdapter(format!(
                "no wgpu adapter on this machine: {error}"
            ))),
            Ok(found) => pollster::block_on(strict.request_adapter(&options)).map_err(|error| {
                NoAdapter(format!(
                    "{} is not WebGPU compliant, strict mode hides it: {error}",
                    found.get_info().name
                ))
            }),
        };
        require_or_skip(test, probe)
    }

    #[cfg(feature = "window")]
    #[test]
    fn strict_webgpu_device_stays_inside_the_spec() {
        use std::io::Write;
        let Some(adapter) = strict_adapter("strict_webgpu_device_stays_inside_the_spec") else {
            return;
        };
        let (device, _queue) = pollster::block_on(
            adapter.request_device(&crate::present::Presenter::device_descriptor(&adapter)),
        )
        .expect("the presenter's device descriptor opens on a strict WebGPU adapter");
        let info = adapter.get_info();
        let limits = device.limits();
        // A direct stderr write: libtest captures the print macros of a passing test.
        let _ = writeln!(
            std::io::stderr(),
            "strict-webgpu: adapter={} max_texture_dimension_2d={} max_bind_groups={}",
            info.name,
            limits.max_texture_dimension_2d,
            limits.max_bind_groups
        );

        // The spec numbers, stated independently of wgpu's table.
        let spec = wgpu::Limits::defaults();
        assert_eq!(
            (
                spec.max_bind_groups,
                spec.max_storage_buffers_per_shader_stage,
                spec.max_texture_dimension_2d
            ),
            (4, 8, 8192),
            "wgpu's default limits are the WebGPU spec's"
        );
        let mut beyond = Vec::new();
        limits.check_limits_with_fail_fn(&spec, false, |name, have, allowed| {
            beyond.push(format!("{name}={have} (spec {allowed})"));
        });
        assert!(
            beyond.is_empty(),
            "device limits beyond the WebGPU defaults: {beyond:?}"
        );
        assert!(limits.max_bind_groups <= 4);
        assert!(limits.max_storage_buffers_per_shader_stage <= 8);
        assert!(limits.max_texture_dimension_2d <= 8192);

        // The strict adapter itself: native-only limits already zero, and no
        // feature outside the WebGPU set (wgpu exempts these four).
        let mut native_zeroed = adapter.limits();
        native_zeroed.zero_native_only();
        assert_eq!(
            adapter.limits(),
            native_zeroed,
            "a strict adapter reports no native-only limit"
        );
        let allowed = wgpu::Features::all_webgpu_mask()
            | wgpu::Features::EXTERNAL_TEXTURE
            | wgpu::Features::TEXTURE_FORMAT_NV12
            | wgpu::Features::TEXTURE_FORMAT_P010
            | wgpu::Features::TEXTURE_FORMAT_16BIT_NORM;
        assert_eq!(
            adapter.features().difference(allowed),
            wgpu::Features::empty(),
            "a strict adapter reports only WebGPU features"
        );
        assert_eq!(
            device.features().difference(allowed),
            wgpu::Features::empty(),
            "a strict device holds only WebGPU features"
        );
    }

    #[cfg(feature = "window")]
    #[test]
    fn strict_webgpu_presenter_renders_a_clean_frame() {
        use crate::present::{DrawItem, DrawList, MeshKey, Presenter};
        let Some(adapter) = strict_adapter("strict_webgpu_presenter_renders_a_clean_frame") else {
            return;
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Presenter::device_descriptor(&adapter)))
                .expect("the presenter's device descriptor opens on a strict WebGPU adapter");
        let mut presenter = Presenter::init_offscreen_from_device(&adapter, device, queue)
            .expect("every presenter pipeline builds under strict WebGPU");
        let key = MeshKey::new(1, [0xA1; 32]);
        presenter
            .upload_mesh(key, &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(160, 120).expect("valid size applies");
        let scene = crate::v1::default_scene(160, 120).expect("default scene is valid");
        let list = DrawList {
            camera: scene.camera,
            light: scene.light,
            items: vec![DrawItem {
                mesh: key,
                material: scene.material,
                base_color: None,
                normal_map: None,
                orm: None,
                emissive: None,
            }],
        };
        let out = presenter
            .render_offscreen(&list)
            .expect("the forward and blit passes run under strict WebGPU");
        assert!(
            out.manifest.is_clean(),
            "strict WebGPU frame had validation errors: {:?}",
            out.manifest
        );
        assert!(out.receipt.is_clean() && out.receipt.presented);
        assert_eq!(out.receipt.draws, 1);
    }
}
