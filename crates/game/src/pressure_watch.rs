//! The window's pressure watch: presented frames and the OS VRAM reading in,
//! placement and GPU layer count out (OPT27).
//!
//! The window loop owns one [`PressureWatch`]. Every presented frame pushes
//! its interval into the [`PressureFeed`], which the arbiter turns into a
//! placement; the OS VRAM probe is the cold path (a DXGI query), so it runs
//! at most once a second and the cached headroom rides every frame. The
//! budget that is left becomes a [`LayerPlan`]: the `--n-gpu-layers` count a
//! local sidecar launches with, and a typed [`LayerNotice`] when the model
//! does not fit whole.
//!
//! The tick never waits: the placement is one atomic byte, the probe runs on
//! the window thread, and the watch allocates nothing per frame.
//!
//! Units: microseconds for frame times, milliseconds for the probe clock, MB
//! for VRAM.

use engine_provider::arbiter::{
    plan_layers, Arbiter, ArbiterConfig, ArbiterError, ArbiterNotice, LayerNotice, LayerPlan,
    Placement, VramHeadroom,
};
use engine_provider::pressure::{PressureError, PressureFeed};
use engine_render::backend::{read_vram, VramReading};

/// How often the OS VRAM probe runs, milliseconds. The read is the cold
/// path: once a second is enough for a placement that dwells for frames.
pub const VRAM_EVERY_MS: u64 = 1_000;

/// The frame budget of a 60 Hz window, microseconds.
pub const FRAME_BUDGET_60HZ_US: u32 = 16_667;

/// The reference machine's local model on the GPU, MB (a 4 GB quantised
/// model beside the renderer on the 6 GB RTX 3050).
pub const LOCAL_MODEL_VRAM_MB: u32 = 4_000;

/// The reference local model's layer count (llama.cpp's `--n-gpu-layers`
/// ceiling).
pub const LOCAL_MODEL_LAYERS: u32 = 32;

/// Where the VRAM reading comes from: the OS in the window, a scripted
/// reading in tests. A plain function pointer, so the watch stays allocation
/// free and deterministic.
pub type VramProbe = fn() -> VramReading;

/// Why a watch could not start.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WatchError {
    /// The arbiter config was refused.
    Config(ArbiterError),
    /// The pressure feed was refused (a zero frame budget).
    Feed(PressureError),
}

impl core::fmt::Display for WatchError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Config(why) => write!(f, "pressure watch config: {why}"),
            Self::Feed(why) => write!(f, "pressure watch feed: {why}"),
        }
    }
}

impl std::error::Error for WatchError {}

/// One window's pressure state: frames in, receipt lines out.
#[derive(Debug)]
pub struct PressureWatch {
    feed: PressureFeed,
    cfg: ArbiterConfig,
    layers: u32,
    probe: VramProbe,
    vram: VramHeadroom,
    /// Milliseconds since the last OS probe.
    since_vram_ms: u64,
    /// The last layer notice printed, so a plan prints once, not each frame.
    last_notice: Option<LayerNotice>,
    frames: u64,
}

impl PressureWatch {
    /// A watch over `cfg`'s model, which has `of_layers` equal layers, with
    /// the OS as the VRAM probe.
    pub fn new(
        cfg: ArbiterConfig,
        of_layers: u32,
        frame_budget_us: u32,
    ) -> Result<Self, WatchError> {
        Self::with_probe(cfg, of_layers, frame_budget_us, read_vram)
    }

    /// The reference machine's local model at 60 Hz: a 4 GB model in 32
    /// layers, the OS as the VRAM probe.
    pub fn reference() -> Result<Self, WatchError> {
        Self::new(
            ArbiterConfig::reference(LOCAL_MODEL_VRAM_MB),
            LOCAL_MODEL_LAYERS,
            FRAME_BUDGET_60HZ_US,
        )
    }

    /// A watch over an explicit VRAM probe.
    pub fn with_probe(
        cfg: ArbiterConfig,
        of_layers: u32,
        frame_budget_us: u32,
        probe: VramProbe,
    ) -> Result<Self, WatchError> {
        let arbiter = Arbiter::new(cfg).map_err(WatchError::Config)?;
        let feed = PressureFeed::new(arbiter, frame_budget_us).map_err(WatchError::Feed)?;
        let vram = headroom(probe());
        let mut watch = Self {
            feed,
            cfg,
            layers: of_layers,
            probe,
            vram,
            since_vram_ms: 0,
            last_notice: None,
            frames: 0,
        };
        watch.last_notice = watch.plan().notice;
        Ok(watch)
    }

    /// Push one presented frame: its interval in microseconds and the
    /// milliseconds since the previous frame. Returns the receipt line on the
    /// first frame and whenever this frame moved the placement or changed the
    /// layer plan.
    pub fn frame(&mut self, frame_us: u32, dt_ms: u64) -> Option<String> {
        self.frames = self.frames.saturating_add(1);
        self.since_vram_ms = self.since_vram_ms.saturating_add(dt_ms);
        if self.since_vram_ms >= VRAM_EVERY_MS {
            self.since_vram_ms = 0;
            self.vram = headroom((self.probe)());
        }
        let notice = self.feed.push_frame(frame_us, self.vram);
        let plan = self.plan();
        let replanned = plan.notice != self.last_notice;
        self.last_notice = plan.notice;
        // The first presented frame always prints, so even a quiet run has
        // its receipt line.
        if notice.is_none() && !replanned && self.frames > 1 {
            return None;
        }
        Some(self.line(notice))
    }

    /// The receipt line: recent p95, the load, the placement, the GPU layer
    /// plan and the free VRAM, plus why the model is short of the GPU.
    #[must_use]
    pub fn line(&self, notice: Option<ArbiterNotice>) -> String {
        let plan = self.plan();
        let free = match self.vram.free_mb() {
            Some(free) => format!("vram_free_mb={free}"),
            None => "vram_free_mb=unknown".to_owned(),
        };
        let moved = match notice {
            Some(moved) => format!(
                "moved={}->{}",
                placement_label(moved.from),
                placement_label(moved.to)
            ),
            None => "moved=none".to_owned(),
        };
        let mut line = format!(
            "arena pressure: p95_us={} load={} placement={} layers={}/{} {free} {moved}",
            self.p95_us(),
            self.load_permille(),
            placement_label(self.placement()),
            plan.gpu_layers,
            plan.of_layers,
        );
        if let Some(why) = plan.notice {
            line.push_str(" (");
            line.push_str(&why.line());
            line.push(')');
        }
        line
    }

    /// The GPU layer plan the cached VRAM reading gives.
    #[must_use]
    pub fn plan(&self) -> LayerPlan {
        plan_layers(
            self.vram,
            self.cfg.model_vram_mb,
            self.cfg.vram_reserve_mb,
            self.layers,
        )
    }

    /// The placement the arbiter published.
    #[must_use]
    pub fn placement(&self) -> Placement {
        self.feed.placement()
    }

    /// The recent frame-time p95, microseconds.
    #[must_use]
    pub fn p95_us(&self) -> u32 {
        self.feed.p95_us()
    }

    /// The recent load against the frame budget, permille.
    #[must_use]
    pub fn load_permille(&self) -> u32 {
        self.feed.load_permille()
    }

    /// Frames pushed so far.
    #[must_use]
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// The cached OS headroom.
    #[must_use]
    pub fn vram(&self) -> VramHeadroom {
        self.vram
    }
}

/// The OS reading as arbiter headroom; a missing reading stays unknown.
fn headroom(reading: VramReading) -> VramHeadroom {
    match reading {
        VramReading::Measured(measure) => {
            VramHeadroom::from_bytes(Some(measure.budget_bytes), Some(measure.usage_bytes))
        }
        VramReading::Unknown(_) => VramHeadroom::Unknown,
    }
}

fn placement_label(placement: Placement) -> &'static str {
    match placement {
        Placement::Gpu => "gpu",
        Placement::Cpu => "cpu",
        Placement::Defer => "defer",
        Placement::Remote => "remote",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_render::backend::{VramMeasure, VramUnknown};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    /// A 4 GB model in 32 equal layers, 125 MB each.
    const MODEL_MB: u32 = 4_000;
    const LAYERS: u32 = 32;
    const MIB: u64 = 1024 * 1024;

    static PROBES: AtomicUsize = AtomicUsize::new(0);
    /// 0 = roomy (5 GB free), 1 = short (1 GB free).
    static MODE: AtomicUsize = AtomicUsize::new(0);

    fn scripted_probe() -> VramReading {
        PROBES.fetch_add(1, Ordering::SeqCst);
        let used_mb = if MODE.load(Ordering::SeqCst) == 0 {
            500
        } else {
            5_000
        };
        VramReading::Measured(VramMeasure {
            budget_bytes: 6_000 * MIB,
            usage_bytes: used_mb * MIB,
            dedicated_bytes: 6_144 * MIB,
        })
    }

    fn watch() -> PressureWatch {
        PROBES.store(0, Ordering::SeqCst);
        PressureWatch::with_probe(
            ArbiterConfig::reference(MODEL_MB),
            LAYERS,
            FRAME_BUDGET_60HZ_US,
            scripted_probe,
        )
        .expect("watch")
    }

    #[test]
    fn the_os_probe_runs_once_a_second_not_once_a_frame() {
        MODE.store(0, Ordering::SeqCst);
        let mut watch = watch();
        assert_eq!(PROBES.load(Ordering::SeqCst), 1, "one probe at start");
        // 30 frames of 16 ms are 480 ms: no new reading.
        for _ in 0..30 {
            watch.frame(16_000, 16);
        }
        assert_eq!(PROBES.load(Ordering::SeqCst), 1, "under a second: cached");
        // 40 more cross the second: exactly one more reading.
        for _ in 0..40 {
            watch.frame(16_000, 16);
        }
        assert_eq!(PROBES.load(Ordering::SeqCst), 2, "one probe per second");
        assert_eq!(watch.frames(), 70);
    }

    #[test]
    fn a_heavy_window_moves_the_placement_and_prints_one_line_per_change() {
        MODE.store(0, Ordering::SeqCst);
        let mut watch = watch();
        // Quiet frames earn the GPU (recover_after 6).
        let mut lines = Vec::new();
        for _ in 0..8 {
            if let Some(line) = watch.frame(10_000, 5) {
                lines.push(line);
            }
        }
        assert_eq!(watch.placement(), Placement::Gpu);
        assert_eq!(lines.len(), 2, "first frame plus the move: {lines:?}");
        assert!(lines[1].contains("moved=cpu->gpu"), "{}", lines[1]);
        // 959 permille of the 60 Hz budget: past cpu_enter, under defer.
        let mut last = String::new();
        for _ in 0..2 {
            if let Some(line) = watch.frame(16_000, 5) {
                last = line;
            }
        }
        assert_eq!(watch.placement(), Placement::Cpu);
        assert!(last.contains("placement=cpu"), "{last}");
        assert!(last.contains("load=959"), "{last}");
        assert!(last.contains("moved=gpu->cpu"), "{last}");
    }

    #[test]
    fn a_short_budget_reaches_the_line_as_a_layer_count() {
        MODE.store(1, Ordering::SeqCst);
        let mut watch = watch();
        // 1 GB free minus the 512 MB reserve: 488 usable, 3 layers of 125.
        assert_eq!(watch.vram().free_mb(), Some(1_000));
        assert_eq!(watch.plan().gpu_layers, 3);
        watch.frame(10_000, 16);
        let line = watch.line(None);
        assert!(line.contains("layers=3/32"), "{line}");
        assert!(line.contains("vram_free_mb=1000"), "{line}");
        assert!(line.contains("MB short of the whole model"), "{line}");
    }

    #[test]
    fn a_reading_change_prints_the_new_plan_once() {
        MODE.store(0, Ordering::SeqCst);
        let mut watch = watch();
        assert!(watch.plan().is_whole(), "5.5 GB free carries 4 GB");
        watch.frame(16_000, 16);
        MODE.store(1, Ordering::SeqCst);
        // Cross the probe second: the short reading arrives with its line.
        let mut lines = Vec::new();
        for _ in 0..70 {
            if let Some(line) = watch.frame(16_000, 16) {
                lines.push(line);
            }
        }
        assert_eq!(lines.len(), 2, "the first frame plus the change: {lines:?}");
        assert!(lines[1].contains("layers=3/32"), "{}", lines[1]);
        assert_eq!(watch.plan().gpu_layers, 3);
    }

    #[test]
    fn an_unreadable_probe_stays_unknown_and_never_guesses() {
        static HITS: AtomicU64 = AtomicU64::new(0);
        fn unknown() -> VramReading {
            HITS.fetch_add(1, Ordering::SeqCst);
            VramReading::Unknown(VramUnknown::Headless)
        }
        let mut watch = PressureWatch::with_probe(
            ArbiterConfig::reference(MODEL_MB),
            LAYERS,
            FRAME_BUDGET_60HZ_US,
            unknown,
        )
        .expect("watch");
        assert_eq!(watch.vram(), VramHeadroom::Unknown);
        assert!(watch.plan().is_whole());
        assert_eq!(watch.plan().notice, Some(LayerNotice::Unknown));
        // No reading refuses the GPU, so the placement stays on the CPU.
        for _ in 0..8 {
            watch.frame(10_000, 16);
        }
        assert_eq!(watch.placement(), Placement::Cpu);
        assert!(watch.line(None).contains("vram_free_mb=unknown"));
    }

    #[test]
    fn a_zero_frame_budget_is_refused() {
        let refused = PressureWatch::with_probe(
            ArbiterConfig::reference(MODEL_MB),
            LAYERS,
            0,
            scripted_probe,
        );
        assert!(matches!(refused, Err(WatchError::Feed(_))));
        let bad_cfg = ArbiterConfig {
            degrade_after: 0,
            ..ArbiterConfig::reference(MODEL_MB)
        };
        let refused =
            PressureWatch::with_probe(bad_cfg, LAYERS, FRAME_BUDGET_60HZ_US, scripted_probe);
        assert!(matches!(refused, Err(WatchError::Config(_))));
    }
}
