//! arbiter: where local inference runs while the game shares one GPU.
//!
//! Owner: engine-provider. On the reference machine the renderer and a
//! local model share one 6 GB GPU (RTX 3050 Laptop). [`Arbiter::observe`]
//! takes one integer pressure sample (frame-time p95 against its budget,
//! VRAM budget and use) and places local inference on the GPU, the CPU,
//! defers it, or hands it to a configured remote. A move needs several
//! agreeing samples (hysteresis, so placement does not flap), and every
//! move emits a typed [`ArbiterNotice`]. Same config plus same sample series
//! gives the same notices: the inputs are integers and nothing reads a clock.
//!
//! The tick never waits: the decision is published to a lock-free
//! [`PlacementReader`] (one atomic byte), which the request path reads.
//! [`LocalRoute`] applies it on the local adapter: Gpu calls the sidecar,
//! Cpu calls a second loopback sidecar started with no GPU layers
//! (`llama-server --n-gpu-layers 0`), Defer parks the request in a bounded
//! queue drained once pressure falls, and Remote hands the request back so
//! the session's fallback order reaches the remote adapter. Remote is
//! chosen only when [`ArbiterConfig::remote_configured`] says one exists.
//!
//! Units: microseconds for frame times, MB for VRAM, permille for load
//! (p95 over budget). Errors: [`ArbiterError`], [`RouteError`].

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use super::config::{AdapterHandle, AdapterTarget, ConfigError};
use super::mock::MockStep;
use super::protocol::AiRequest;
use super::transport::{self, Endpoint};

/// Where local inference runs.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Placement {
    /// The loopback sidecar with GPU layers.
    Gpu,
    /// The loopback CPU sidecar (no GPU layers).
    Cpu,
    /// Not now: requests wait in a bounded queue.
    Defer,
    /// The session's configured remote adapter.
    Remote,
}

impl Placement {
    /// Pressure rung: GPU 0, CPU 1, Defer and Remote 2.
    fn rung(self) -> u8 {
        match self {
            Self::Gpu => 0,
            Self::Cpu => 1,
            Self::Defer | Self::Remote => 2,
        }
    }

    fn to_byte(self) -> u8 {
        match self {
            Self::Gpu => 0,
            Self::Cpu => 1,
            Self::Defer => 2,
            Self::Remote => 3,
        }
    }

    fn from_byte(byte: u8) -> Self {
        match byte {
            0 => Self::Gpu,
            1 => Self::Cpu,
            3 => Self::Remote,
            // Only the arbiter stores here; anything else reads as the
            // safe choice.
            _ => Self::Defer,
        }
    }
}

/// VRAM as the OS reports it, or unknown (never a guess).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VramHeadroom {
    /// OS budget for the game process and what it uses, MB.
    Known {
        /// Budget, MB.
        budget_mb: u32,
        /// Current use, MB.
        used_mb: u32,
    },
    /// No reading on this machine.
    Unknown,
}

impl VramHeadroom {
    /// One MiB in bytes: the MB unit these thresholds use.
    pub const MB: u64 = 1024 * 1024;

    /// Free MB under the budget, or `None` when unknown.
    #[must_use]
    pub fn free_mb(self) -> Option<u32> {
        match self {
            Self::Known { budget_mb, used_mb } => Some(budget_mb.saturating_sub(used_mb)),
            Self::Unknown => None,
        }
    }

    /// Headroom from a host reading in bytes, the shape the OS `read_vram`
    /// gives (budget and use). A missing half reads as unknown, never as a
    /// guess. Consumer: [`crate::pressure::PressureFeed::push_frame`].
    #[must_use]
    pub fn from_bytes(budget_bytes: Option<u64>, used_bytes: Option<u64>) -> Self {
        match (budget_bytes, used_bytes) {
            (Some(budget), Some(used)) => Self::Known {
                budget_mb: u32::try_from(budget / Self::MB).unwrap_or(u32::MAX),
                used_mb: u32::try_from(used / Self::MB).unwrap_or(u32::MAX),
            },
            _ => Self::Unknown,
        }
    }
}

/// One pressure sample, taken off the tick (frame stats plus a VRAM read).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PressureSample {
    /// Recent frame-time p95, microseconds.
    pub frame_p95_us: u32,
    /// Frame budget, microseconds. Zero reads as full pressure.
    pub frame_budget_us: u32,
    /// VRAM budget and use.
    pub vram: VramHeadroom,
}

impl PressureSample {
    /// p95 over budget in permille, saturating.
    #[must_use]
    pub fn load_permille(&self) -> u32 {
        if self.frame_budget_us == 0 {
            return u32::MAX;
        }
        let load = u64::from(self.frame_p95_us) * 1000 / u64::from(self.frame_budget_us);
        u32::try_from(load).unwrap_or(u32::MAX)
    }
}

/// Thresholds and dwell for [`Arbiter`]. Each `*_exit` sits below its
/// `*_enter`, so a load between them keeps the current placement.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ArbiterConfig {
    /// VRAM the model needs on the GPU, MB.
    pub model_vram_mb: u32,
    /// VRAM always left free for the renderer, MB.
    pub vram_reserve_mb: u32,
    /// Load at or above which GPU inference moves to CPU, permille.
    pub cpu_enter_permille: u32,
    /// Load at or below which CPU inference returns to GPU, permille.
    pub cpu_exit_permille: u32,
    /// Load at or above which inference defers, permille.
    pub defer_enter_permille: u32,
    /// Load at or below which deferred inference resumes on CPU, permille.
    pub defer_exit_permille: u32,
    /// Agreeing samples before a move to a heavier rung.
    pub degrade_after: u8,
    /// Agreeing samples before a move back to a lighter rung.
    pub recover_after: u8,
    /// A remote adapter is configured; heavy pressure routes there instead
    /// of deferring.
    pub remote_configured: bool,
}

impl ArbiterConfig {
    /// Reference-machine defaults for a model of `model_vram_mb`: leave
    /// 512 MB to the renderer, move to CPU at 90% of the frame budget and
    /// back at 75%, defer at 110% and resume at 95%; degrade after 2
    /// samples, recover after 6; no remote.
    #[must_use]
    pub fn reference(model_vram_mb: u32) -> Self {
        Self {
            model_vram_mb,
            vram_reserve_mb: 512,
            cpu_enter_permille: 900,
            cpu_exit_permille: 750,
            defer_enter_permille: 1100,
            defer_exit_permille: 950,
            degrade_after: 2,
            recover_after: 6,
            remote_configured: false,
        }
    }

    /// Check the hysteresis bands and dwell counts.
    pub fn validate(&self) -> Result<(), ArbiterError> {
        if self.degrade_after == 0 || self.recover_after == 0 {
            return Err(ArbiterError::ZeroDwell);
        }
        if self.cpu_exit_permille >= self.cpu_enter_permille
            || self.defer_exit_permille >= self.defer_enter_permille
            || self.cpu_enter_permille >= self.defer_enter_permille
        {
            return Err(ArbiterError::Bands);
        }
        Ok(())
    }
}

/// Why an arbiter config was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArbiterError {
    /// A dwell count is zero: every sample could flap the placement.
    ZeroDwell,
    /// An exit threshold is not below its enter threshold, or CPU enters
    /// at or above the defer threshold.
    Bands,
}

impl core::fmt::Display for ArbiterError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ZeroDwell => f.write_str("arbiter dwell counts must be at least 1"),
            Self::Bands => f.write_str(
                "arbiter thresholds need cpu_exit < cpu_enter < defer_enter and defer_exit < defer_enter",
            ),
        }
    }
}

impl std::error::Error for ArbiterError {}

/// Why the arbiter moved.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MoveReason {
    /// Frame time rose past a threshold.
    FramePressure {
        /// Load of the deciding sample, permille.
        load_permille: u32,
    },
    /// Frame time fell back under a threshold.
    FrameRecovered {
        /// Load of the deciding sample, permille.
        load_permille: u32,
    },
    /// Not enough VRAM for the model plus the renderer reserve.
    VramShort {
        /// Free VRAM, MB.
        free_mb: u32,
        /// VRAM needed to stay or start on the GPU, MB.
        need_mb: u32,
    },
    /// No VRAM reading, so the GPU is refused.
    VramUnknown,
}

/// One placement move.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ArbiterNotice {
    /// Index of the deciding sample, counting from 0.
    pub sample: u64,
    /// Placement before.
    pub from: Placement,
    /// Placement after.
    pub to: Placement,
    /// Why.
    pub reason: MoveReason,
}

/// Lock-free view of the current placement. Cloned into the request path;
/// reading it is one atomic load and never waits.
#[derive(Clone, Debug)]
pub struct PlacementReader {
    shared: Arc<AtomicU8>,
}

impl PlacementReader {
    /// The placement the arbiter last published.
    #[must_use]
    pub fn placement(&self) -> Placement {
        Placement::from_byte(self.shared.load(Ordering::Acquire))
    }
}

/// Decides where local inference runs from a series of pressure samples.
#[derive(Debug)]
pub struct Arbiter {
    cfg: ArbiterConfig,
    current: Placement,
    /// A move waiting for its dwell: target, reason, agreeing samples.
    pending: Option<(Placement, MoveReason, u8)>,
    samples: u64,
    shared: Arc<AtomicU8>,
}

impl Arbiter {
    /// Start on the CPU: the model is not on the GPU yet and earns it after
    /// `recover_after` quiet samples with room.
    pub fn new(cfg: ArbiterConfig) -> Result<Self, ArbiterError> {
        cfg.validate()?;
        Ok(Self {
            cfg,
            current: Placement::Cpu,
            pending: None,
            samples: 0,
            shared: Arc::new(AtomicU8::new(Placement::Cpu.to_byte())),
        })
    }

    /// Current placement.
    #[must_use]
    pub fn placement(&self) -> Placement {
        self.current
    }

    /// A reader for the request path.
    #[must_use]
    pub fn reader(&self) -> PlacementReader {
        PlacementReader {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Build the serving adapter for `target` under this arbiter's placement:
    /// the local arm applies the route on its request path, so a request under
    /// frame pressure parks or dials the CPU sidecar instead of always dialing
    /// the GPU sidecar. The route reads this arbiter's published placement
    /// (one atomic load per request, never a wait on the tick); `cpu` is the
    /// loopback CPU sidecar endpoint, or `None` to park every CPU and deferred
    /// placement.
    pub fn serving_adapter(
        &self,
        target: &AdapterTarget,
        script: Vec<MockStep>,
        cpu: Option<Endpoint>,
    ) -> Result<AdapterHandle, ConfigError> {
        super::serving::serving_adapter(target, script, self, cpu)
    }

    /// Feed one sample; returns the notice when the placement moved. Integer
    /// math and one atomic store: no lock, no allocation, safe per frame.
    pub fn observe(&mut self, sample: PressureSample) -> Option<ArbiterNotice> {
        let index = self.samples;
        self.samples = self.samples.saturating_add(1);
        let (target, reason, immediate) = self.target(sample);
        if target == self.current {
            self.pending = None;
            return None;
        }
        let dwell = if target.rung() > self.current.rung() {
            self.cfg.degrade_after
        } else {
            self.cfg.recover_after
        };
        let count = match self.pending {
            Some((waiting, _, n)) if waiting == target => n.saturating_add(1),
            _ => 1,
        };
        if !immediate && count < dwell {
            self.pending = Some((target, reason, count));
            return None;
        }
        let notice = ArbiterNotice {
            sample: index,
            from: self.current,
            to: target,
            reason,
        };
        self.current = target;
        self.pending = None;
        self.shared.store(target.to_byte(), Ordering::Release);
        Some(notice)
    }

    /// The placement this sample asks for, why, and whether it must apply
    /// now (VRAM pressure on the GPU cannot wait out a dwell).
    fn target(&self, sample: PressureSample) -> (Placement, MoveReason, bool) {
        let cfg = &self.cfg;
        let load = sample.load_permille();
        let rung = match self.current.rung() {
            0 if load >= cfg.defer_enter_permille => 2,
            0 if load >= cfg.cpu_enter_permille => 1,
            0 => 0,
            1 if load >= cfg.defer_enter_permille => 2,
            1 if load <= cfg.cpu_exit_permille => 0,
            1 => 1,
            _ if load <= cfg.defer_exit_permille => 1,
            _ => 2,
        };
        let frame_reason = if rung > self.current.rung() {
            MoveReason::FramePressure {
                load_permille: load,
            }
        } else {
            MoveReason::FrameRecovered {
                load_permille: load,
            }
        };
        // Leaving the GPU for memory is immediate, wherever frames point:
        // the destination still follows frame pressure, the reason stays
        // VRAM. Refusing entry from off the GPU keeps the current rung.
        if self.current == Placement::Gpu {
            if let Some(why) = self.vram_refusal(sample.vram) {
                let dest = match rung {
                    2 if cfg.remote_configured => Placement::Remote,
                    2 => Placement::Defer,
                    _ => Placement::Cpu,
                };
                return (dest, why, true);
            }
        }
        match rung {
            0 => match self.vram_refusal(sample.vram) {
                Some(why) => (Placement::Cpu, why, false),
                None => (Placement::Gpu, frame_reason, false),
            },
            1 => (Placement::Cpu, frame_reason, false),
            _ if cfg.remote_configured => (Placement::Remote, frame_reason, false),
            _ => (Placement::Defer, frame_reason, false),
        }
    }

    /// VRAM needed to run on the GPU, MB. On the GPU the model's VRAM is
    /// already resident, so staying needs only the renderer reserve;
    /// joining needs the model plus the reserve. Saturating: a huge model
    /// never wraps to a small need. Caller: [`Arbiter::vram_refusal`].
    #[must_use]
    pub fn vram_need_mb(&self) -> u32 {
        if self.current == Placement::Gpu {
            self.cfg.vram_reserve_mb
        } else {
            self.cfg
                .model_vram_mb
                .saturating_add(self.cfg.vram_reserve_mb)
        }
    }

    /// Why the GPU is refused, if it is. On the GPU the model's VRAM is
    /// already in use, so staying needs only the reserve; joining needs the
    /// model plus the reserve. The gap is the VRAM half of the hysteresis.
    fn vram_refusal(&self, vram: VramHeadroom) -> Option<MoveReason> {
        let need_mb = self.vram_need_mb();
        match vram.free_mb() {
            None => Some(MoveReason::VramUnknown),
            Some(free_mb) if free_mb < need_mb => Some(MoveReason::VramShort { free_mb, need_mb }),
            Some(_) => None,
        }
    }
}

/// Most requests one [`LocalRoute`] may hold deferred.
pub const MAX_DEFERRED: usize = 64;

/// Why a [`LocalRoute`] was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RouteError {
    /// The CPU sidecar endpoint is not loopback.
    NonLoopbackCpu,
    /// The defer bound is zero or above [`MAX_DEFERRED`]; it is carried.
    Bound(usize),
}

impl core::fmt::Display for RouteError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NonLoopbackCpu => f.write_str("cpu sidecar endpoint must be loopback"),
            Self::Bound(n) => write!(f, "defer bound {n} is outside 1..={MAX_DEFERRED}"),
        }
    }
}

impl std::error::Error for RouteError {}

/// Why a request was not run now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hold {
    /// Parked in the defer queue; it runs from `run_deferred` later.
    Deferred,
    /// The defer queue is full; refused and counted.
    QueueFull,
    /// Placement is Remote: the session's fallback serves it.
    Remote,
}

/// What the route decided for one request: the placement it carries.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum Admit {
    /// Call the GPU sidecar.
    Gpu,
    /// Call the CPU sidecar at this endpoint.
    Cpu(Endpoint),
    /// Do not call now.
    Held(Hold),
}

/// The arbiter's decision applied on the local adapter's request path.
#[derive(Clone, Debug)]
pub struct LocalRoute {
    reader: PlacementReader,
    cpu: Option<Endpoint>,
    deferred: VecDeque<AiRequest>,
    bound: usize,
    refused_full: u64,
    last: Option<(Placement, Option<Hold>)>,
}

impl LocalRoute {
    /// Route by `reader`. `cpu` is the loopback CPU sidecar; without one a
    /// Cpu placement defers. `bound` caps the defer queue, `1..=MAX_DEFERRED`.
    pub fn new(
        reader: PlacementReader,
        cpu: Option<Endpoint>,
        bound: usize,
    ) -> Result<Self, RouteError> {
        if bound == 0 || bound > MAX_DEFERRED {
            return Err(RouteError::Bound(bound));
        }
        if cpu
            .as_ref()
            .is_some_and(|e| !transport::is_loopback(e.host()))
        {
            return Err(RouteError::NonLoopbackCpu);
        }
        Ok(Self {
            reader,
            cpu,
            deferred: VecDeque::with_capacity(bound),
            bound,
            refused_full: 0,
            last: None,
        })
    }

    /// Requests waiting in the defer queue.
    #[must_use]
    pub fn deferred_len(&self) -> usize {
        self.deferred.len()
    }

    /// Requests refused because the defer queue was full.
    #[must_use]
    pub fn refused_full(&self) -> u64 {
        self.refused_full
    }

    /// The placement the latest request carried and its hold, if held.
    #[must_use]
    pub fn last(&self) -> Option<(Placement, Option<Hold>)> {
        self.last
    }

    /// Decide `request` from the published placement (one atomic load).
    pub(crate) fn admit(&mut self, request: &AiRequest) -> Admit {
        let placement = self.reader.placement();
        let admit = if self.deferred.iter().any(|r| r.id == request.id) {
            // A retry of a parked request stays parked; it never queues twice.
            Admit::Held(Hold::Deferred)
        } else {
            match (placement, &self.cpu) {
                (Placement::Gpu, _) => Admit::Gpu,
                (Placement::Cpu, Some(cpu)) => Admit::Cpu(cpu.clone()),
                (Placement::Remote, _) => Admit::Held(Hold::Remote),
                (Placement::Cpu | Placement::Defer, _) => Admit::Held(self.park(request)),
            }
        };
        let hold = match &admit {
            Admit::Held(hold) => Some(*hold),
            _ => None,
        };
        self.last = Some((placement, hold));
        admit
    }

    fn park(&mut self, request: &AiRequest) -> Hold {
        if self.deferred.len() >= self.bound {
            self.refused_full = self.refused_full.saturating_add(1);
            return Hold::QueueFull;
        }
        self.deferred.push_back(request.clone());
        Hold::Deferred
    }

    /// The oldest parked request and where it may run, if the placement now
    /// lets it run.
    pub(crate) fn next_ready(&mut self) -> Option<(AiRequest, Admit)> {
        let placement = self.reader.placement();
        let admit = match (placement, &self.cpu) {
            (Placement::Gpu, _) => Admit::Gpu,
            (Placement::Cpu, Some(cpu)) => Admit::Cpu(cpu.clone()),
            _ => return None,
        };
        let out = self.deferred.pop_front().map(|request| (request, admit));
        if out.is_some() {
            self.last = Some((placement, None));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOMY: VramHeadroom = VramHeadroom::Known {
        budget_mb: 5_000,
        used_mb: 1_500,
    };

    fn at(load_permille: u32, vram: VramHeadroom) -> PressureSample {
        PressureSample {
            frame_p95_us: load_permille * 16,
            frame_budget_us: 16_000,
            vram,
        }
    }

    fn run(arbiter: &mut Arbiter, series: &[PressureSample]) -> Vec<ArbiterNotice> {
        series.iter().filter_map(|s| arbiter.observe(*s)).collect()
    }

    /// Quiet, then pressure, then overload, then easing back down.
    fn scripted() -> Vec<PressureSample> {
        let mut s = Vec::new();
        s.extend([at(600, ROOMY); 6]); // earns the GPU
        s.extend([at(950, ROOMY); 2]); // CPU
        s.extend([at(1_200, ROOMY); 2]); // defer
        s.extend([at(1_000, ROOMY); 5]); // inside the defer band: stays
        s.extend([at(900, ROOMY); 6]); // back to CPU
        s.extend([at(800, ROOMY); 6]); // inside the CPU band: stays
        s.extend([at(600, ROOMY); 6]); // back to GPU
        s
    }

    fn moves(notices: &[ArbiterNotice]) -> Vec<(Placement, Placement)> {
        notices.iter().map(|n| (n.from, n.to)).collect()
    }

    #[test]
    fn frame_pressure_moves_gpu_cpu_defer_and_back() {
        let mut arbiter = Arbiter::new(ArbiterConfig::reference(1_500)).expect("config");
        let reader = arbiter.reader();
        assert_eq!(reader.placement(), Placement::Cpu, "cold start is CPU");
        let notices = run(&mut arbiter, &scripted());
        use Placement::{Cpu, Defer, Gpu};
        assert_eq!(
            moves(&notices),
            vec![
                (Cpu, Gpu),
                (Gpu, Cpu),
                (Cpu, Defer),
                (Defer, Cpu),
                (Cpu, Gpu)
            ]
        );
        // Each move waited its dwell: sample indexes of the deciding samples.
        let at_sample: Vec<u64> = notices.iter().map(|n| n.sample).collect();
        assert_eq!(at_sample, vec![5, 7, 9, 20, 32]);
        assert_eq!(
            notices[1].reason,
            MoveReason::FramePressure { load_permille: 950 }
        );
        assert_eq!(
            notices[3].reason,
            MoveReason::FrameRecovered { load_permille: 900 }
        );
        assert_eq!(reader.placement(), Gpu, "the snapshot follows the arbiter");
    }

    #[test]
    fn load_hovering_on_a_threshold_does_not_flap() {
        let mut arbiter = Arbiter::new(ArbiterConfig::reference(1_500)).expect("config");
        run(&mut arbiter, &[at(600, ROOMY); 6]);
        assert_eq!(arbiter.placement(), Placement::Gpu);
        // Alternating just over and just under the CPU threshold never
        // collects two agreeing samples.
        let hover: Vec<PressureSample> = (0..40)
            .map(|i| at(if i % 2 == 0 { 920 } else { 880 }, ROOMY))
            .collect();
        assert!(run(&mut arbiter, &hover).is_empty());
        assert_eq!(arbiter.placement(), Placement::Gpu);
        // One overload sample alone does not defer either.
        assert_eq!(arbiter.observe(at(1_300, ROOMY)), None);
        assert_eq!(arbiter.observe(at(600, ROOMY)), None);
        assert_eq!(arbiter.placement(), Placement::Gpu);
    }

    #[test]
    fn low_vram_headroom_refuses_the_gpu() {
        let short = VramHeadroom::Known {
            budget_mb: 5_000,
            used_mb: 3_300,
        };
        let mut arbiter = Arbiter::new(ArbiterConfig::reference(1_500)).expect("config");
        // 1 700 MB free < 1 500 model + 512 reserve: quiet frames never
        // earn the GPU.
        assert!(run(&mut arbiter, &[at(400, short); 30]).is_empty());
        assert_eq!(arbiter.placement(), Placement::Cpu);
        // Unknown VRAM is refused too.
        assert!(run(&mut arbiter, &[at(400, VramHeadroom::Unknown); 30]).is_empty());
        assert_eq!(arbiter.placement(), Placement::Cpu);
        // With room it joins; once there, VRAM falling under the reserve
        // leaves at once, without waiting a dwell.
        run(&mut arbiter, &[at(400, ROOMY); 6]);
        assert_eq!(arbiter.placement(), Placement::Gpu);
        let squeezed = VramHeadroom::Known {
            budget_mb: 5_000,
            used_mb: 4_700,
        };
        let notice = arbiter.observe(at(400, squeezed)).expect("leaves at once");
        assert_eq!(
            notice.reason,
            MoveReason::VramShort {
                free_mb: 300,
                need_mb: 512
            }
        );
        assert_eq!(notice.to, Placement::Cpu);
        let notice = {
            run(&mut arbiter, &[at(400, ROOMY); 6]);
            arbiter.observe(at(400, VramHeadroom::Unknown))
        };
        assert_eq!(notice.map(|n| n.reason), Some(MoveReason::VramUnknown));
    }

    #[test]
    fn vram_short_on_gpu_forces_immediate_leave_even_when_frames_are_hot() {
        // GPU + hot frames + short VRAM: memory cannot wait out a dwell,
        // so the first hot sample leaves at once with the VRAM reason;
        // the destination still follows frame pressure (defer here).
        let mut arbiter = Arbiter::new(ArbiterConfig::reference(1_500)).expect("config");
        run(&mut arbiter, &[at(400, ROOMY); 6]);
        assert_eq!(arbiter.placement(), Placement::Gpu);
        let squeezed = VramHeadroom::Known {
            budget_mb: 5_000,
            used_mb: 4_700,
        };
        let notice = arbiter
            .observe(at(1_500, squeezed))
            .expect("leaves at once");
        assert_eq!(notice.to, Placement::Defer);
        assert_eq!(
            notice.reason,
            MoveReason::VramShort {
                free_mb: 300,
                need_mb: 512
            }
        );
        assert_eq!(arbiter.placement(), Placement::Defer);
    }

    #[test]
    fn same_series_twice_gives_the_same_notices() {
        let mut series = scripted();
        series.extend([at(400, VramHeadroom::Unknown); 3]);
        series.extend([at(2_000, ROOMY); 4]);
        let first = run(
            &mut Arbiter::new(ArbiterConfig::reference(1_500)).expect("config"),
            &series,
        );
        let second = run(
            &mut Arbiter::new(ArbiterConfig::reference(1_500)).expect("config"),
            &series,
        );
        assert!(!first.is_empty());
        assert_eq!(first, second);
    }

    #[test]
    fn heavy_pressure_routes_remote_only_when_configured() {
        let mut cfg = ArbiterConfig::reference(1_500);
        let mut local_only = Arbiter::new(cfg).expect("config");
        run(&mut local_only, &[at(1_500, ROOMY); 3]);
        assert_eq!(local_only.placement(), Placement::Defer);
        cfg.remote_configured = true;
        let mut with_remote = Arbiter::new(cfg).expect("config");
        run(&mut with_remote, &[at(1_500, ROOMY); 3]);
        assert_eq!(with_remote.placement(), Placement::Remote);
        assert_eq!(with_remote.reader().placement(), Placement::Remote);
        // A zero frame budget reads as full pressure, never a divide.
        let zero = PressureSample {
            frame_p95_us: 1,
            frame_budget_us: 0,
            vram: ROOMY,
        };
        assert_eq!(zero.load_permille(), u32::MAX);
    }

    #[test]
    fn bad_configs_are_refused_by_name() {
        let mut cfg = ArbiterConfig::reference(1_500);
        cfg.recover_after = 0;
        assert_eq!(Arbiter::new(cfg).err(), Some(ArbiterError::ZeroDwell));
        let mut cfg = ArbiterConfig::reference(1_500);
        cfg.cpu_exit_permille = cfg.cpu_enter_permille;
        assert_eq!(cfg.validate(), Err(ArbiterError::Bands));
        let mut cfg = ArbiterConfig::reference(1_500);
        cfg.defer_enter_permille = cfg.cpu_enter_permille;
        assert_eq!(cfg.validate(), Err(ArbiterError::Bands));
        assert!(ArbiterError::Bands.to_string().contains("cpu_exit"));
        let reader = Arbiter::new(ArbiterConfig::reference(1))
            .expect("config")
            .reader();
        assert_eq!(
            LocalRoute::new(reader.clone(), None, 0).err(),
            Some(RouteError::Bound(0))
        );
        assert_eq!(
            LocalRoute::new(reader.clone(), None, MAX_DEFERRED + 1).err(),
            Some(RouteError::Bound(MAX_DEFERRED + 1))
        );
        let far = transport::parse_endpoint("http://10.0.0.9:8081").expect("parses");
        assert_eq!(
            LocalRoute::new(reader, Some(far), 4).err(),
            Some(RouteError::NonLoopbackCpu)
        );
    }

    #[test]
    fn defer_queue_parks_fifo_refuses_full_and_drains_in_order() {
        // OPT27 slice 1: LocalRoute::admit parks under Defer in FIFO
        // order, never queues one id twice, refuses past the bound while
        // counting refused_full, and next_ready drains oldest-first once
        // the placement lets it run. Consumer: LocalAdapter (local.rs)
        // admits every request and drains through next_ready.
        let mut cfg = ArbiterConfig::reference(1_500);
        cfg.degrade_after = 1;
        cfg.recover_after = 1;
        let mut arbiter = Arbiter::new(cfg).expect("config");
        arbiter.observe(at(1_500, ROOMY));
        assert_eq!(arbiter.placement(), Placement::Defer);
        let mut route = LocalRoute::new(arbiter.reader(), None, 2).expect("route");
        let first = crate::mock::fixtures::request(11, 3);
        let second = crate::mock::fixtures::request(12, 3);
        let third = crate::mock::fixtures::request(13, 3);
        assert_eq!(route.admit(&first), Admit::Held(Hold::Deferred));
        assert_eq!(route.admit(&first), Admit::Held(Hold::Deferred));
        assert_eq!(route.deferred_len(), 1, "one id never queues twice");
        assert_eq!(route.admit(&second), Admit::Held(Hold::Deferred));
        assert_eq!(route.deferred_len(), 2);
        assert_eq!(route.admit(&third), Admit::Held(Hold::QueueFull));
        assert_eq!(route.refused_full(), 1);
        assert_eq!(route.deferred_len(), 2);
        assert_eq!(
            route.last(),
            Some((Placement::Defer, Some(Hold::QueueFull)))
        );
        // Under Defer nothing is ready: the queue waits for recovery.
        assert!(route.next_ready().is_none());
        // Quiet frames earn the GPU back; the drain is oldest-first.
        for _ in 0..8 {
            arbiter.observe(at(400, ROOMY));
        }
        assert_eq!(arbiter.placement(), Placement::Gpu);
        let (ready_first, _) = route.next_ready().expect("oldest drains first");
        assert_eq!(ready_first.id, crate::protocol::RequestId(11));
        let (ready_second, _) = route.next_ready().expect("second drains next");
        assert_eq!(ready_second.id, crate::protocol::RequestId(12));
        assert!(route.next_ready().is_none());
        assert_eq!(route.deferred_len(), 0);
    }

    #[test]
    fn drained_requests_refresh_last_notice() {
        // OPT27 slice 2: draining through next_ready publishes the running
        // placement, so last() never reports a stale hold after the queue
        // empties. Consumer: LocalAdapter::run_deferred drains oldest-first.
        let mut cfg = ArbiterConfig::reference(1_500);
        cfg.degrade_after = 1;
        cfg.recover_after = 1;
        let mut arbiter = Arbiter::new(cfg).expect("config");
        arbiter.observe(at(1_500, ROOMY));
        assert_eq!(arbiter.placement(), Placement::Defer);
        let mut route = LocalRoute::new(arbiter.reader(), None, 2).expect("route");
        let first = crate::mock::fixtures::request(21, 3);
        let second = crate::mock::fixtures::request(22, 3);
        assert_eq!(route.admit(&first), Admit::Held(Hold::Deferred));
        assert_eq!(route.admit(&second), Admit::Held(Hold::Deferred));
        assert_eq!(route.last(), Some((Placement::Defer, Some(Hold::Deferred))));
        for _ in 0..8 {
            arbiter.observe(at(400, ROOMY));
        }
        assert_eq!(arbiter.placement(), Placement::Gpu);
        let (ready, _) = route.next_ready().expect("oldest drains first");
        assert_eq!(ready.id, crate::protocol::RequestId(21));
        assert_eq!(
            route.last(),
            Some((Placement::Gpu, None)),
            "draining publishes the running placement"
        );
    }

    #[test]
    fn vram_need_is_reserve_on_gpu_and_model_plus_reserve_off_it() {
        // OPT27 slice 3: the VRAM half of the hysteresis as one named,
        // saturating helper. Consumer: Arbiter::vram_refusal gates GPU
        // entry (model + reserve) vs staying (reserve only); LocalAdapter
        // (local.rs) admits every request through that gate.
        let cfg = ArbiterConfig::reference(1_500);
        let cold = Arbiter::new(cfg).expect("config");
        assert_eq!(cold.vram_need_mb(), 1_500 + 512);
        let mut gpu = Arbiter::new(cfg).expect("config");
        for _ in 0..6 {
            gpu.observe(at(400, ROOMY));
        }
        assert_eq!(gpu.placement(), Placement::Gpu);
        assert_eq!(gpu.vram_need_mb(), 512);
        let huge = ArbiterConfig {
            model_vram_mb: u32::MAX,
            ..cfg
        };
        let saturating = Arbiter::new(huge).expect("config");
        assert_eq!(
            saturating.vram_need_mb(),
            u32::MAX,
            "a huge model never wraps to a small need"
        );
    }

    #[test]
    fn remote_placement_holds_without_parking_and_waits() {
        // OPT27 guard: under Remote the route holds for the session fallback
        // without parking, so the tick never waits; the queue stays empty and
        // next_ready waits. Consumer: LocalAdapter (local.rs) admits every
        // request and drains through next_ready.
        let mut cfg = ArbiterConfig::reference(1_500);
        cfg.degrade_after = 1;
        cfg.recover_after = 1;
        cfg.remote_configured = true;
        let mut arbiter = Arbiter::new(cfg).expect("config");
        arbiter.observe(at(1_500, ROOMY));
        assert_eq!(arbiter.placement(), Placement::Remote);
        let mut route = LocalRoute::new(arbiter.reader(), None, 2).expect("route");
        let first = crate::mock::fixtures::request(31, 3);
        let second = crate::mock::fixtures::request(32, 3);
        assert_eq!(route.admit(&first), Admit::Held(Hold::Remote));
        assert_eq!(route.admit(&first), Admit::Held(Hold::Remote));
        assert_eq!(route.admit(&second), Admit::Held(Hold::Remote));
        assert_eq!(route.deferred_len(), 0, "remote never parks");
        assert_eq!(route.refused_full(), 0);
        assert_eq!(route.last(), Some((Placement::Remote, Some(Hold::Remote))));
        assert!(route.next_ready().is_none(), "remote waits off the tick");
    }
}
