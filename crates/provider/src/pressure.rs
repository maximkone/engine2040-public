//! pressure: the live feed that keeps local inference off the frame budget.
//!
//! A running window knows two things the arbiter needs: how long the last
//! frames took and how much video memory the game has left.
//! [`PressureFeed`] turns those two live readings into the one integer
//! sample [`PressureSample`] the arbiter decides on: the recent frame-time
//! p95 against the frame budget, plus the VRAM headroom. It is the missing
//! wire between a running window and [`Arbiter`]: before it, nothing could
//! hand the arbiter a live frame p95 or a live `read_vram` result.
//!
//! Every number is an integer and nothing here reads a clock, so the same
//! frame series and the same VRAM readings always give the same notices. The
//! feed never allocates after construction and never waits: the placement it
//! publishes is one atomic byte the request path reads
//! ([`PlacementReader`]), so a frame never blocks on inference.
//!
//! Units: microseconds for frame times and budgets, MB (1 MiB) for VRAM,
//! permille for load. Errors: [`PressureError`].

use super::arbiter::{
    Arbiter, ArbiterNotice, LocalRoute, Placement, PlacementReader, PressureSample, RouteError,
    VramHeadroom,
};
use super::transport::Endpoint;

/// Frames kept for the recent p95: two seconds at 60 fps.
pub const FRAME_WINDOW: usize = 120;

/// Largest frame window the feed accepts.
pub const MAX_FRAME_WINDOW: usize = 1_024;

/// Why a pressure feed was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PressureError {
    /// The frame window is zero or above [`MAX_FRAME_WINDOW`]; it is carried.
    Window(usize),
    /// The frame budget is zero: every sample would read as full pressure.
    ZeroBudget,
}

impl core::fmt::Display for PressureError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Window(n) => {
                write!(f, "frame window {n} is outside 1..={MAX_FRAME_WINDOW}")
            }
            Self::ZeroBudget => f.write_str("frame budget must be above zero"),
        }
    }
}

impl std::error::Error for PressureError {}

/// The recent frame times: the last `capacity` frames in time order.
///
/// The window keeps its frames sorted by time, each with the number of the
/// frame that brought it in, so the p95 is one index read and the per-frame
/// cost is one scan plus one memmove: no allocation after construction and
/// no sort in the frame loop. The oldest frame (the smallest number) drops
/// when the window is full.
#[derive(Clone, Debug)]
pub struct FrameWindow {
    /// The kept frames, ascending by time, each with its arrival number.
    sorted: Vec<(u32, u64)>,
    /// The number the next frame gets.
    next_seq: u64,
    capacity: usize,
    /// The p95 of the kept frames, kept in step with the window.
    p95_us: u32,
}

impl FrameWindow {
    /// A window of `capacity` frames. Zero and oversized windows are refused.
    pub fn new(capacity: usize) -> Result<Self, PressureError> {
        if capacity == 0 || capacity > MAX_FRAME_WINDOW {
            return Err(PressureError::Window(capacity));
        }
        Ok(Self {
            sorted: Vec::with_capacity(capacity),
            next_seq: 0,
            capacity,
            p95_us: 0,
        })
    }

    /// Frames kept right now.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sorted.len()
    }

    /// True when no frame was pushed yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sorted.is_empty()
    }

    /// Window capacity in frames.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// One frame time in microseconds. The oldest frame drops when the
    /// window is full.
    pub fn push(&mut self, frame_us: u32) {
        if self.sorted.len() == self.capacity {
            let oldest = self
                .sorted
                .iter()
                .enumerate()
                .min_by_key(|(_, (_, seq))| *seq)
                .map(|(index, _)| index)
                .unwrap_or(0);
            self.sorted.remove(oldest);
        }
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        let at = self.sorted.partition_point(|(value, _)| *value <= frame_us);
        self.sorted.insert(at, (frame_us, seq));
        self.p95_us = self.p95_rank();
    }

    /// The recent frame-time p95 in microseconds: the nearest rank over the
    /// kept frames (rank `ceil(0.95 n)`, 1-based). Zero when no frame was
    /// pushed yet: no frames measured is no pressure.
    #[must_use]
    pub fn p95_us(&self) -> u32 {
        self.p95_us
    }

    /// The p95 of the kept frames, recomputed from the window.
    fn p95_rank(&self) -> u32 {
        let count = self.sorted.len();
        if count == 0 {
            return 0;
        }
        let rank = (95 * count).div_ceil(100);
        self.sorted[rank - 1].0
    }
}

/// The live feed: frame times and VRAM readings in, placement out.
///
/// The host (the window or the game loop) calls [`PressureFeed::push_frame`]
/// once per frame, off the tick, and the placement the arbiter published is
/// what the request path reads.
#[derive(Debug)]
pub struct PressureFeed {
    arbiter: Arbiter,
    frames: FrameWindow,
    frame_budget_us: u32,
}

impl PressureFeed {
    /// A feed over the default frame window.
    pub fn new(arbiter: Arbiter, frame_budget_us: u32) -> Result<Self, PressureError> {
        Self::with_window(arbiter, frame_budget_us, FRAME_WINDOW)
    }

    /// A feed over an explicit frame window. A zero frame budget is refused:
    /// it would read as full pressure on every sample.
    pub fn with_window(
        arbiter: Arbiter,
        frame_budget_us: u32,
        window: usize,
    ) -> Result<Self, PressureError> {
        if frame_budget_us == 0 {
            return Err(PressureError::ZeroBudget);
        }
        Ok(Self {
            arbiter,
            frames: FrameWindow::new(window)?,
            frame_budget_us,
        })
    }

    /// Feed one frame time and the VRAM reading; returns the notice when the
    /// placement moved. Integer math and one atomic store: no lock, no
    /// allocation, safe per frame.
    pub fn push_frame(&mut self, frame_us: u32, vram: VramHeadroom) -> Option<ArbiterNotice> {
        self.frames.push(frame_us);
        self.arbiter.observe(self.sample(vram))
    }

    /// The recent frame-time p95 in microseconds.
    #[must_use]
    pub fn p95_us(&self) -> u32 {
        self.frames.p95_us()
    }

    /// The load of the recent frames against the budget, in permille.
    #[must_use]
    pub fn load_permille(&self) -> u32 {
        self.sample(VramHeadroom::Unknown).load_permille()
    }

    /// The placement the arbiter published.
    #[must_use]
    pub fn placement(&self) -> Placement {
        self.arbiter.placement()
    }

    /// A reader for the request path: one atomic load per request, never a
    /// wait.
    #[must_use]
    pub fn reader(&self) -> PlacementReader {
        self.arbiter.reader()
    }

    /// The arbiter behind the feed.
    #[must_use]
    pub fn arbiter(&self) -> &Arbiter {
        &self.arbiter
    }

    /// The arbiter behind the feed.
    pub fn arbiter_mut(&mut self) -> &mut Arbiter {
        &mut self.arbiter
    }

    /// The frame budget in microseconds.
    #[must_use]
    pub fn frame_budget_us(&self) -> u32 {
        self.frame_budget_us
    }

    /// The request path for this feed: a [`LocalRoute`] on the same reader
    /// the feed publishes, so the sidecar the request path calls always
    /// matches the placement the feed decided. `cpu` is the loopback CPU
    /// sidecar, or `None` to park every CPU and deferred placement.
    pub fn route(&self, cpu: Option<Endpoint>, bound: usize) -> Result<LocalRoute, RouteError> {
        LocalRoute::new(self.reader(), cpu, bound)
    }

    /// The sample the last frame produced.
    fn sample(&self, vram: VramHeadroom) -> PressureSample {
        PressureSample {
            frame_p95_us: self.frames.p95_us(),
            frame_budget_us: self.frame_budget_us,
            vram,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::arbiter::{ArbiterConfig, Hold, MoveReason};
    use crate::mock::MockStep;
    use crate::policy::{AiSessionPolicy, ProviderCapabilities};
    use crate::protocol::{AdapterKind, AiError, PrivacyMode};
    use crate::stub::StubServer;
    use crate::transport::parse_endpoint;
    use crate::{LocalAdapter, ProviderAdapter};

    /// The 60 fps frame budget in microseconds, rounded to a whole
    /// millisecond so a frame time maps to an exact permille load.
    const FRAME_BUDGET_US: u32 = 16_000;

    const ROOMY: VramHeadroom = VramHeadroom::Known {
        budget_mb: 5_000,
        used_mb: 1_500,
    };

    /// A frame time at `permille` of the budget.
    fn at(permille: u32) -> u32 {
        u32::try_from(u64::from(permille) * u64::from(FRAME_BUDGET_US) / 1_000).unwrap_or(u32::MAX)
    }

    fn policy() -> AiSessionPolicy {
        AiSessionPolicy {
            adapters: vec![AdapterKind::Local],
            mode: PrivacyMode::LocalOnly,
            destinations: Vec::new(),
            scopes: Vec::new(),
            request_limit: 64,
            max_concurrency: 1,
            spend_limit: None,
            fallback: crate::policy::Fallback {
                order: Vec::new(),
                scripted_ok: true,
            },
        }
    }

    fn request(id: u64) -> crate::protocol::AiRequest {
        let mut ask = crate::mock::fixtures::request(id, 3);
        ask.model.adapter = AdapterKind::Local;
        ask.model.model = String::from("sidecar-1");
        ask
    }

    fn new_feed(cfg: ArbiterConfig, window: usize) -> PressureFeed {
        PressureFeed::with_window(
            Arbiter::new(cfg).expect("arbiter config"),
            FRAME_BUDGET_US,
            window,
        )
        .expect("feed")
    }

    #[test]
    fn frame_window_p95_is_the_nearest_rank_over_the_kept_frames() {
        // OPT27: the live frame p95 the arbiter decides on. The window keeps
        // the newest frames and reads the nearest rank (ceil(0.95 n)).
        let mut window = FrameWindow::new(100).expect("window");
        assert_eq!(window.p95_us(), 0, "no frames measured is no pressure");
        for frame in 1..=100u32 {
            window.push(frame);
        }
        assert_eq!(window.len(), 100);
        assert_eq!(window.p95_us(), 95, "rank 95 of 100");
        // The oldest frame drops; the p95 follows the window.
        window.push(101);
        assert_eq!(window.len(), 100);
        assert_eq!(window.p95_us(), 96, "rank 95 of 2..=101");
        // A short window keeps the newest frames only.
        let mut short = FrameWindow::new(4).expect("window");
        for frame in [10, 20, 30, 40] {
            short.push(frame);
        }
        assert_eq!(short.p95_us(), 40, "rank 4 of 4 is the newest worst frame");
        short.push(5);
        assert_eq!(short.p95_us(), 40, "the worst of the kept frames stays");
        // Bad windows are refused by name.
        assert_eq!(FrameWindow::new(0).err(), Some(PressureError::Window(0)));
        assert_eq!(
            FrameWindow::new(MAX_FRAME_WINDOW + 1).err(),
            Some(PressureError::Window(MAX_FRAME_WINDOW + 1))
        );
        assert!(PressureError::Window(0)
            .to_string()
            .contains("frame window"));
    }

    #[test]
    fn live_frames_move_the_placement_with_typed_notices() {
        // OPT27: the feed hands the arbiter a live frame p95 and a live VRAM
        // reading. Quiet frames earn the GPU, hot frames take it away, and
        // every move carries a typed reason.
        let mut cfg = ArbiterConfig::reference(1_500);
        cfg.degrade_after = 1;
        cfg.recover_after = 1;
        let mut feed = new_feed(cfg, 20);
        assert_eq!(feed.placement(), Placement::Cpu, "cold start is CPU");
        assert_eq!(feed.frame_budget_us(), FRAME_BUDGET_US);
        assert_eq!(feed.load_permille(), 0);

        let mut notices = Vec::new();
        for _ in 0..4 {
            if let Some(notice) = feed.push_frame(at(600), ROOMY) {
                notices.push(notice);
            }
        }
        assert_eq!(feed.placement(), Placement::Gpu);
        assert_eq!(
            notices,
            vec![ArbiterNotice {
                sample: 0,
                from: Placement::Cpu,
                to: Placement::Gpu,
                reason: MoveReason::FrameRecovered { load_permille: 600 },
            }]
        );
        assert_eq!(feed.p95_us(), at(600));
        assert_eq!(feed.load_permille(), 600);

        // Hot frames: the placement leaves the GPU with a typed reason.
        let mut notices = Vec::new();
        for _ in 0..4 {
            if let Some(notice) = feed.push_frame(at(1_200), ROOMY) {
                notices.push(notice);
            }
        }
        assert_eq!(feed.placement(), Placement::Defer);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].from, Placement::Gpu);
        assert_eq!(notices[0].to, Placement::Defer);
        assert_eq!(
            notices[0].reason,
            MoveReason::FramePressure {
                load_permille: 1_200
            }
        );

        // The same series twice gives the same notices: no clock, no guess.
        let series: Vec<u32> = (0..12)
            .map(|index| if index < 6 { at(600) } else { at(1_200) })
            .collect();
        let play = |frames: &[u32]| {
            let mut cfg = ArbiterConfig::reference(1_500);
            cfg.degrade_after = 1;
            cfg.recover_after = 1;
            let mut feed = new_feed(cfg, 20);
            frames
                .iter()
                .filter_map(|frame| feed.push_frame(*frame, ROOMY))
                .collect::<Vec<_>>()
        };
        assert_eq!(play(&series), play(&series));
        assert!(!play(&series).is_empty());

        // A bad feed is refused by name.
        let arbiter = Arbiter::new(ArbiterConfig::reference(1_500)).expect("arbiter config");
        assert_eq!(
            PressureFeed::new(arbiter, 0).err(),
            Some(PressureError::ZeroBudget)
        );
        let arbiter = Arbiter::new(ArbiterConfig::reference(1_500)).expect("arbiter config");
        assert_eq!(
            PressureFeed::with_window(arbiter, FRAME_BUDGET_US, 0).err(),
            Some(PressureError::Window(0))
        );
    }

    #[test]
    fn a_vram_reading_feeds_the_arbiter_and_never_guesses() {
        // OPT27: the read_vram seam. Bytes become the MB the thresholds use;
        // a missing half reads as unknown, never as a guess.
        let known =
            VramHeadroom::from_bytes(Some(5 * 1024 * 1024 * 1024), Some(1024 * 1024 * 1024));
        assert_eq!(
            known,
            VramHeadroom::Known {
                budget_mb: 5_120,
                used_mb: 1_024,
            }
        );
        assert_eq!(known.free_mb(), Some(4_096));
        assert_eq!(
            VramHeadroom::from_bytes(None, Some(1024 * 1024 * 1024)),
            VramHeadroom::Unknown
        );
        assert_eq!(
            VramHeadroom::from_bytes(Some(1024 * 1024 * 1024), None),
            VramHeadroom::Unknown
        );

        // A short reading refuses the GPU even when the frames are quiet:
        // the placement stays on the CPU and no move is announced.
        let mut cfg = ArbiterConfig::reference(1_500);
        cfg.degrade_after = 1;
        cfg.recover_after = 1;
        let mut feed = new_feed(cfg, 20);
        let short =
            VramHeadroom::from_bytes(Some(2 * 1024 * 1024 * 1024), Some(1_900 * 1024 * 1024));
        for _ in 0..4 {
            assert!(
                feed.push_frame(at(400), short).is_none(),
                "a refused GPU is no move"
            );
        }
        assert_eq!(feed.placement(), Placement::Cpu, "no room for the model");

        // With room the GPU is earned; a squeezed reading leaves it at once,
        // and the reason names the VRAM gap.
        let mut feed = new_feed(cfg, 20);
        for _ in 0..4 {
            feed.push_frame(at(400), ROOMY);
        }
        assert_eq!(feed.placement(), Placement::Gpu);
        let squeezed =
            VramHeadroom::from_bytes(Some(5_000 * 1024 * 1024), Some(4_700 * 1024 * 1024));
        let notice = feed.push_frame(at(400), squeezed).expect("leaves at once");
        assert_eq!(
            notice.reason,
            MoveReason::VramShort {
                free_mb: 300,
                need_mb: 512,
            }
        );
        assert_eq!(notice.to, Placement::Cpu);
        assert_eq!(feed.placement(), Placement::Cpu);
    }

    #[test]
    fn a_loaded_local_model_keeps_the_frame_budget_and_degrades_with_typed_notices() {
        // OPT27, the window-build budget check (provider half): with a local
        // model loaded, quiet frames keep it on the GPU and hot frames take
        // it off without the tick ever waiting. The GPU and window half of
        // the check needs the render lane and the reference GPU; what this
        // proves is the provider half: the placement, the typed degradation
        // and the parked work running later with the same answer.
        let body = crate::stub::chat_ok(
            r#"{"actions": [{"noop": true}], "diagnostics": ["opt27:budget"]}"#,
            None,
        );
        let gpu = StubServer::spawn_completion(body.clone());
        let cpu = StubServer::spawn_completion(body);
        let mut cfg = ArbiterConfig::reference(1_000);
        cfg.degrade_after = 1;
        cfg.recover_after = 1;
        let mut feed = new_feed(cfg, 8);
        let route = feed
            .route(Some(parse_endpoint(&cpu.url()).expect("cpu url parses")), 2)
            .expect("route");
        let mut adapter = LocalAdapter::new(
            vec![MockStep::Succeed],
            parse_endpoint(&gpu.url()).expect("gpu url parses"),
        )
        .expect("loopback accepted")
        .with_route(route);
        let policy = policy();
        let caps = ProviderCapabilities::local_http();

        // Quiet frames earn the GPU and the loaded model serves there.
        for _ in 0..8 {
            feed.push_frame(at(500), ROOMY);
        }
        assert_eq!(feed.placement(), Placement::Gpu);
        let served = adapter.call(&request(1), &policy, &caps);
        assert!(served.result.is_ok(), "the gpu sidecar served: {served:?}");
        assert!(gpu.hits() >= 1, "the local model ran on the gpu sidecar");
        assert_eq!(cpu.hits(), 0);

        // Hot frames: the placement leaves the GPU. The tick asks, the
        // request parks, and the answer comes as a typed refusal.
        for _ in 0..8 {
            feed.push_frame(at(1_500), ROOMY);
        }
        assert_eq!(feed.placement(), Placement::Defer);
        let (gpu_before, cpu_before) = (gpu.hits(), cpu.hits());
        let start = Instant::now();
        for id in 2..62u64 {
            let held = adapter.call(&request(id), &policy, &caps);
            assert_eq!(held.result, Err(AiError::Unavailable));
            assert!(!held.mutated, "a held request changes nothing");
            assert!(held.events.is_empty());
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_micros(u64::from(FRAME_BUDGET_US)),
            "60 frames of provider work took {elapsed:?}, over one frame budget"
        );
        assert_eq!((gpu.hits(), cpu.hits()), (gpu_before, cpu_before));
        let route = adapter.route().expect("route set");
        assert_eq!(route.deferred_len(), 2, "the defer queue is bounded");
        assert_eq!(route.refused_full(), 58, "a full queue refuses and counts");
        assert_eq!(
            route.last(),
            Some((Placement::Defer, Some(Hold::QueueFull))),
            "the degradation is typed"
        );

        // Pressure eases: the parked work runs off the tick, oldest first,
        // on the sidecar the placement names. The window holds the last 8
        // frames, so the quiet p95 lands after 8 quiet frames and the
        // placement walks back up one rung per sample.
        let hits_before = gpu.hits() + cpu.hits();
        for _ in 0..12 {
            feed.push_frame(at(500), ROOMY);
        }
        assert_eq!(feed.placement(), Placement::Gpu);
        let ran = adapter.run_deferred(&policy, &caps);
        assert_eq!(ran.len(), 2);
        for outcome in &ran {
            let result = outcome.result.as_ref().expect("parked work ran");
            let proposal = result.proposal.as_ref().expect("a proposal");
            crate::mock::validate_proposal(proposal).expect("the proposal validates");
        }
        assert!(
            gpu.hits() + cpu.hits() > hits_before,
            "the parked work ran on a sidecar"
        );
        assert_eq!(
            adapter.route().map(LocalRoute::deferred_len),
            Some(0),
            "the queue drained"
        );
    }
}
