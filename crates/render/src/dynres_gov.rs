//! dynres_gov: pure integer dynamic-resolution governor.
//!
//! Owner: render lane. Consumer: crates/render/src/aa_scale.rs:760 ScaleController::update reads scale_pct (aa_scale.rs:798) each frame for the WIN1 viewport TAA-off path; a governor pull lowers scale and render size, presentation only.
//! Units: microseconds for `budget_us`/`gpu_us`, percent for scale.
//! Determinism: integer only, no clock; same inputs give same percents.
//! Errors: none; config clamps, never panics, never allocates.

/// Short history kept for the perceived signal.
const WINDOW_LEN: usize = 4;
/// Percent points added per cooldown window on sustained headroom.
const RISE_STEP_PCT: u8 = 5;

/// Integer dynamic-resolution governor.
///
/// `budget_us` is the GPU reference, `min_pct`/`max_pct` bound the scale,
/// `deadband_pct` is the hold band around the budget, `cooldown_frames`
/// gates any rise, `panic_over_budget_frames` is straight over-budget
/// frames before the floor drop. Presentation only, never sim state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DynRes {
    /// GPU budget in microseconds, the reference signal.
    pub budget_us: u32,
    /// Scale floor in percent, reached on panic drop.
    pub min_pct: u8,
    /// Scale ceiling in percent, start and recovery target.
    pub max_pct: u8,
    /// Hold band around the budget in percent of budget.
    pub deadband_pct: u8,
    /// Sustained non-over frames before one rise step.
    pub cooldown_frames: u32,
    /// Straight over-budget frames before the floor drop.
    pub panic_over_budget_frames: u32,
    /// Current scale in percent.
    current_pct: u8,
    /// Ring of recent `gpu_us` readings.
    window: [u32; WINDOW_LEN],
    /// Next write index into `window`.
    pos: usize,
    /// Readings stored, capped at `WINDOW_LEN`.
    filled: usize,
    /// Consecutive over-budget frames.
    over_streak: u32,
    /// Consecutive non-over frames since the last change.
    cooldown: u32,
}

impl DynRes {
    /// Fresh governor at the ceiling with an empty history window.
    pub fn new(
        budget_us: u32,
        min_pct: u8,
        max_pct: u8,
        deadband_pct: u8,
        cooldown_frames: u32,
        panic_over_budget_frames: u32,
    ) -> Self {
        let low = min_pct.min(max_pct);
        let high = min_pct.max(max_pct);
        Self {
            budget_us,
            min_pct,
            max_pct,
            deadband_pct,
            cooldown_frames,
            panic_over_budget_frames,
            current_pct: high,
            window: [0; WINDOW_LEN],
            pos: 0,
            filled: 0,
            over_streak: 0,
            cooldown: 0,
        }
        .with_floor(low)
    }

    /// Snap helper keeping the builder small; pins start inside bounds.
    fn with_floor(mut self, low: u8) -> Self {
        if self.current_pct < low {
            self.current_pct = low;
        }
        self
    }

    /// Perceived signal: integer mean of the history window.
    fn perceived(&self) -> u64 {
        let mut sum: u64 = 0;
        for value in self.window.iter().take(self.filled) {
            sum += u64::from(*value);
        }
        let count: u64 = u64::try_from(self.filled).unwrap_or(1);
        if count == 0 {
            return 0;
        }
        sum / count
    }

    /// Hold band around the budget, both in microseconds.
    fn band(&self) -> (u64, u64) {
        let budget: u64 = u64::from(self.budget_us);
        let dead: u64 = u64::from(self.deadband_pct);
        let tol: u64 = budget.saturating_mul(dead) / 100;
        (budget.saturating_sub(tol), budget.saturating_add(tol))
    }

    /// Feed one GPU reading, return the new scale when it changes.
    ///
    /// Holds inside the deadband, counts cooldown before any rise, drops
    /// to the floor after straight over-budget frames then clears history
    /// for a slow return. Integer only, no clock. Same inputs give the
    /// same percents twice.
    pub fn step(&mut self, gpu_us: u32) -> Option<u8> {
        let low: u8 = self.min_pct.min(self.max_pct);
        let high: u8 = self.min_pct.max(self.max_pct);
        if self.current_pct > high {
            self.current_pct = high;
            return Some(self.current_pct);
        }
        if self.current_pct < low {
            self.current_pct = low;
        }
        self.window[self.pos] = gpu_us;
        self.pos = (self.pos + 1) % WINDOW_LEN;
        if self.filled < WINDOW_LEN {
            self.filled += 1;
        }
        let seen: u64 = self.perceived();
        let (_lower, upper) = self.band();
        if seen > upper {
            self.cooldown = 0;
            self.over_streak = self.over_streak.saturating_add(1);
            let need: u32 = if self.panic_over_budget_frames == 0 {
                1
            } else {
                self.panic_over_budget_frames
            };
            if self.over_streak >= need {
                self.window = [0; WINDOW_LEN];
                self.pos = 0;
                self.filled = 0;
                self.over_streak = 0;
                self.cooldown = 0;
                if self.current_pct != low {
                    self.current_pct = low;
                    return Some(self.current_pct);
                }
            }
            return None;
        }
        self.over_streak = 0;
        self.cooldown = self.cooldown.saturating_add(1);
        let need: u32 = if self.cooldown_frames == 0 {
            1
        } else {
            self.cooldown_frames
        };
        if self.cooldown >= need {
            if self.current_pct < high {
                let next: u8 = self.current_pct.saturating_add(RISE_STEP_PCT);
                self.current_pct = if next > high { high } else { next };
                self.cooldown = 0;
                return Some(self.current_pct);
            }
            self.cooldown = 0;
        }
        None
    }

    /// Current scale in percent, clamped inside the configured bounds.
    ///
    /// Reads the governor output for the next frame even when [`Self::step`]
    /// returns `None` (hold). Integer only, no clock.
    pub fn scale_pct(&self) -> u8 {
        let low: u8 = self.min_pct.min(self.max_pct);
        let high: u8 = self.min_pct.max(self.max_pct);
        self.current_pct.clamp(low, high)
    }
}

/// The render scales the live window frame steps between, percent of the output edge, highest
/// first. A short ladder keeps the frame targets from being rebuilt on every small move.
pub const LIVE_RUNGS_PCT: [u8; 4] = [100, 85, 75, 67];
/// GPU budget in ms the live picker holds by default: under one 60 fps frame, with slack left
/// for the CPU half and the present.
pub const LIVE_TARGET_MS: f32 = 14.0;

const LIVE_RUNGS: usize = LIVE_RUNGS_PCT.len();
/// Samples the median is taken over.
const LIVE_MEDIAN: usize = 5;
/// Frames after a change whose GPU samples are ignored (they were taken before it landed).
const LIVE_SETTLE: u32 = 4;
/// Changes the picker remembers for its receipt; the newest are kept.
const LIVE_LOG: usize = 16;

/// Tuning of [`LiveDynRes`]. All counts are frames.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LiveDynResConfig {
    /// GPU ms the picker holds.
    pub target_ms: f32,
    /// A step up needs the predicted cost one rung up under `target_ms * up_fraction`: the gap
    /// between this and the step-down line is the dead band that stops oscillation.
    pub up_fraction: f32,
    /// Consecutive over-budget frames before a step down.
    pub down_frames: u32,
    /// Consecutive fits before a step up.
    pub up_frames: u32,
    /// Frames a step down waits after any change.
    pub hold_down_frames: u32,
    /// Frames a step up waits after any change.
    pub hold_up_frames: u32,
}

impl Default for LiveDynResConfig {
    fn default() -> Self {
        Self {
            target_ms: LIVE_TARGET_MS,
            up_fraction: 0.85,
            down_frames: 6,
            up_frames: 60,
            hold_down_frames: 12,
            hold_up_frames: 90,
        }
    }
}

/// One scale change of the live picker.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LiveScaleStep {
    /// Frame count (from 1) of the observation that changed the scale.
    pub frame: u32,
    /// Scale before, percent.
    pub from_pct: u8,
    /// Scale after, percent.
    pub to_pct: u8,
}

/// Picks the live frame's render scale from measured GPU ms, one of [`LIVE_RUNGS_PCT`].
///
/// A median of the last few samples drives it. Over the target for `down_frames` in a row steps
/// down to the first rung the pixel-count model says fits (or the floor); a step up goes one
/// rung at a time, only when the model says the rung above fits with `up_fraction` to spare and
/// only after `up_frames` such frames, so a load that sits near the line cannot make it flap.
/// After any change it holds (`hold_*_frames`) and ignores the samples already in flight.
/// Presentation only: no clock, no allocation per frame, never hashed.
#[derive(Clone, Debug)]
pub struct LiveDynRes {
    cfg: LiveDynResConfig,
    rung: usize,
    ring: [f32; LIVE_MEDIAN],
    filled: usize,
    pos: usize,
    over: u32,
    fits: u32,
    since_change: u32,
    settle: u32,
    frames: u32,
    hist: [u32; LIVE_RUNGS],
    steps: [Option<LiveScaleStep>; LIVE_LOG],
    changes: u32,
}

impl LiveDynRes {
    /// Start at the top rung.
    pub fn new(cfg: LiveDynResConfig) -> Self {
        Self {
            cfg,
            rung: 0,
            ring: [0.0; LIVE_MEDIAN],
            filled: 0,
            pos: 0,
            over: 0,
            fits: 0,
            since_change: u32::MAX / 2,
            settle: 0,
            frames: 0,
            hist: [0; LIVE_RUNGS],
            steps: [None; LIVE_LOG],
            changes: 0,
        }
    }

    /// Current scale in percent.
    pub fn scale_pct(&self) -> u8 {
        LIVE_RUNGS_PCT[self.rung]
    }

    /// Frames observed.
    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// Scale changes so far.
    pub fn changes(&self) -> u32 {
        self.changes
    }

    /// Frames spent at each rung, in [`LIVE_RUNGS_PCT`] order.
    pub fn histogram(&self) -> [u32; LIVE_RUNGS] {
        self.hist
    }

    /// The newest changes, oldest first (at most 16).
    pub fn steps(&self) -> impl Iterator<Item = LiveScaleStep> + '_ {
        let n = usize::try_from(self.changes).unwrap_or(usize::MAX);
        let first = if n > LIVE_LOG { n % LIVE_LOG } else { 0 };
        (0..n.min(LIVE_LOG)).filter_map(move |i| self.steps[(first + i) % LIVE_LOG])
    }

    fn median(&self) -> f32 {
        let mut s = self.ring;
        let n = self.filled.min(LIVE_MEDIAN);
        s[..n].sort_by(f32::total_cmp);
        s[n / 2]
    }

    /// Model: GPU cost follows the pixel count, so `ms` at `from` percent costs about
    /// `ms * (to / from)^2` at `to` percent. It over-states the cost of a higher rung (some of
    /// the frame does not scale), which keeps the step-up line safe.
    fn predict(ms: f32, from_pct: u8, to_pct: u8) -> f32 {
        let r = f32::from(to_pct) / f32::from(from_pct.max(1));
        ms * r * r
    }

    fn change_to(&mut self, rung: usize) {
        let from = self.scale_pct();
        self.rung = rung;
        let slot = usize::try_from(self.changes).unwrap_or(usize::MAX) % LIVE_LOG;
        self.steps[slot] = Some(LiveScaleStep {
            frame: self.frames,
            from_pct: from,
            to_pct: self.scale_pct(),
        });
        self.changes += 1;
        self.since_change = 0;
        self.over = 0;
        self.fits = 0;
        self.filled = 0;
        self.pos = 0;
        self.settle = LIVE_SETTLE;
    }

    /// Feed one frame's GPU ms (it may be a few frames old; the read is late by design) and
    /// return the scale in percent for the next frame. A non-finite or negative sample counts the
    /// frame but is not used.
    pub fn observe(&mut self, gpu_ms: f32) -> u8 {
        self.frames = self.frames.saturating_add(1);
        self.hist[self.rung] = self.hist[self.rung].saturating_add(1);
        self.since_change = self.since_change.saturating_add(1);
        if self.settle > 0 {
            self.settle -= 1;
            return self.scale_pct();
        }
        if !gpu_ms.is_finite() || gpu_ms < 0.0 {
            return self.scale_pct();
        }
        self.ring[self.pos] = gpu_ms;
        self.pos = (self.pos + 1) % LIVE_MEDIAN;
        self.filled = (self.filled + 1).min(LIVE_MEDIAN);
        if self.filled < 3 {
            return self.scale_pct();
        }
        let ms = self.median();
        let cur = self.scale_pct();
        if ms > self.cfg.target_ms {
            self.fits = 0;
            self.over += 1;
            if self.over >= self.cfg.down_frames
                && self.since_change >= self.cfg.hold_down_frames
                && self.rung + 1 < LIVE_RUNGS
            {
                let mut next = self.rung + 1;
                while next + 1 < LIVE_RUNGS
                    && Self::predict(ms, cur, LIVE_RUNGS_PCT[next]) > self.cfg.target_ms
                {
                    next += 1;
                }
                self.change_to(next);
            }
        } else {
            self.over = 0;
            let up_fits = self.rung > 0
                && Self::predict(ms, cur, LIVE_RUNGS_PCT[self.rung - 1])
                    <= self.cfg.target_ms * self.cfg.up_fraction;
            if up_fits {
                self.fits += 1;
                if self.fits >= self.cfg.up_frames && self.since_change >= self.cfg.hold_up_frames {
                    self.change_to(self.rung - 1);
                }
            } else {
                self.fits = 0;
            }
        }
        self.scale_pct()
    }

    /// One receipt line: the target, the frames, the change count and the scale histogram.
    pub fn receipt_line(&self) -> String {
        use std::fmt::Write as _;
        let mut hist = String::new();
        for (i, (pct, n)) in LIVE_RUNGS_PCT.iter().zip(self.hist).enumerate() {
            let _ = write!(hist, "{}{pct}:{n}", if i > 0 { "," } else { "" });
        }
        let mut steps = String::new();
        for (i, s) in self.steps().enumerate() {
            let _ = write!(
                steps,
                "{}{}:{}>{}",
                if i > 0 { "," } else { "" },
                s.frame,
                s.from_pct,
                s.to_pct
            );
        }
        format!(
            "dynres: on target_ms={:.1} frames={} changes={} final={} hist={hist} steps={}",
            self.cfg.target_ms,
            self.frames,
            self.changes,
            self.scale_pct(),
            if steps.is_empty() { "none" } else { &steps },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::DynRes;
    use crate::aa_scale::{ScaleController, ScalePolicy};
    use engine_core::CounterRng;

    fn gov() -> DynRes {
        DynRes::new(10_000, 50, 100, 10, 3, 3)
    }

    #[test]
    fn scale_dynres_steady_holds_100() {
        let mut gov = gov();
        for _ in 0..10 {
            assert_eq!(gov.step(10_000), None);
        }
    }

    #[test]
    fn scale_dynres_spike_drops_to_floor() {
        let mut gov = gov();
        for _ in 0..3 {
            assert_eq!(gov.step(10_000), None);
        }
        let mut last: Option<u8> = None;
        for _ in 0..3 {
            last = gov.step(30_000).or(last);
        }
        assert_eq!(last, Some(50));
    }

    #[test]
    fn scale_dynres_recovers_slowly() {
        let mut gov = gov();
        for _ in 0..3 {
            let _ = gov.step(30_000);
        }
        let mut cur: u8 = 50;
        let windows: u32 = 40 * 3;
        for _ in 0..windows {
            if let Some(next) = gov.step(10_000) {
                cur = next;
            }
            if cur == 100 {
                break;
            }
        }
        assert_eq!(cur, 100);
    }

    #[test]
    fn scale_dynres_stable_x2() {
        let seq: [u32; 12] = [
            10_000, 10_000, 30_000, 30_000, 30_000, 10_000, 10_000, 10_000, 10_000, 10_000, 10_000,
            10_000,
        ];
        let mut first = gov();
        let mut second = gov();
        let mut a: Vec<Option<u8>> = Vec::new();
        let mut b: Vec<Option<u8>> = Vec::new();
        for sample in seq {
            a.push(first.step(sample));
            b.push(second.step(sample));
        }
        assert_eq!(a, b);
    }

    #[test]
    fn scale_dynres_panic_clears_history() {
        let mut gov = gov();
        for _ in 0..3 {
            let _ = gov.step(30_000);
        }
        assert_eq!(gov.scale_pct(), 50);
        // Old spike readings must not linger: exactly one cooldown window of
        // in-budget frames raises the scale one step.
        assert_eq!(gov.step(10_000), None);
        assert_eq!(gov.step(10_000), None);
        assert_eq!(gov.step(10_000), Some(55));
    }

    #[test]
    fn scale_dynres_inside_deadband_never_moves() {
        // 10% of 10_000 us is 1_000 us: both edges hold and the scale stays.
        let mut hold = gov();
        for sample in [11_000, 9_000, 10_500, 9_500, 11_000, 9_000, 10_000] {
            assert_eq!(hold.step(sample), None, "sample {sample}");
        }
        assert_eq!(hold.scale_pct(), 100);
    }

    #[test]
    fn scale_dynres_swapped_bounds_clamp() {
        let mut gov = DynRes::new(10_000, 100, 50, 10, 3, 3);
        assert_eq!(gov.scale_pct(), 100);
        let mut last: Option<u8> = None;
        for _ in 0..3 {
            last = gov.step(30_000).or(last);
        }
        assert_eq!(last, Some(50));
        assert_eq!(gov.scale_pct(), 50);
    }

    /// OPT20 fixture: frames of one seeded GPU load (jitter, one long spike,
    /// two short ones) through the live controller. Cost model: a fixed
    /// post/upscale part plus the scene part times the pixel share (scale
    /// squared). `adaptive` false holds full resolution.
    fn seeded_load_run(adaptive: bool) -> (Vec<f64>, ScaleController) {
        const FRAMES: u64 = 600;
        let rng = CounterRng::new(2040);
        let unit =
            |word: u64| f64::from(u32::try_from(word >> 32).expect("32 bits")) / 4_294_967_296.0;
        let mut ctl = ScaleController::new(ScalePolicy::default()).expect("policy builds");
        let mut frame_ms = Vec::new();
        for frame in 0..FRAMES {
            let load = match frame {
                120..=199 => 1.9,
                330..=335 | 450..=455 => 1.7,
                _ => 1.0,
            };
            let jitter = 0.94 + 0.12 * unit(rng.sample(frame));
            let scale = if adaptive {
                f64::from(ctl.scale())
            } else {
                1.0
            };
            let ms = 1.0 + 11.0 * load * jitter * scale * scale;
            frame_ms.push(ms);
            ctl.update(ms).expect("frame runs");
        }
        (frame_ms, ctl)
    }

    fn p95(frame_ms: &[f64]) -> f64 {
        let mut sorted = frame_ms.to_vec();
        sorted.sort_by(f64::total_cmp);
        sorted[(sorted.len() * 95).div_ceil(100) - 1]
    }

    #[test]
    fn dynres_seeded_load_spike_holds_p95_at_the_frame_budget() {
        let (fixed, _) = seeded_load_run(false);
        assert!(
            p95(&fixed) > 16.7,
            "full resolution must miss the budget: p95 {:.2} ms",
            p95(&fixed)
        );
        let (adaptive, ctl) = seeded_load_run(true);
        eprintln!(
            "OPT20 dynres: p95 {:.2} ms fixed, {:.2} ms adaptive, {} resolution changes",
            p95(&fixed),
            p95(&adaptive),
            ctl.changes().len()
        );
        assert!(
            p95(&adaptive) <= 16.7,
            "p95 {:.2} ms over the 16.7 ms budget",
            p95(&adaptive)
        );
        assert_eq!(
            ctl.scale(),
            1.0,
            "the load cleared, full resolution is back"
        );
    }

    #[test]
    fn dynres_resolution_changes_are_logged_inside_the_bounds() {
        let (_, ctl) = seeded_load_run(true);
        let log = ctl.changes();
        let first = log.first().expect("the spike changes the resolution");
        assert!(
            (121..=124).contains(&first.frame),
            "first change on frame {}, spike starts at 121",
            first.frame
        );
        assert!(first.to < first.from, "the first change is a step down");
        let policy = ctl.policy();
        for change in log {
            assert!((policy.min_scale..=policy.max_scale).contains(&change.to));
            assert!(
                (change.to - change.from).abs() > 0.0,
                "a logged change moves"
            );
        }
        assert!(log.windows(2).all(|pair| pair[0].frame < pair[1].frame));
        assert!(
            log.iter().any(|change| change.to > change.from),
            "recovery logged"
        );
        assert!(
            log.len() < 40,
            "{} changes in 600 frames is flapping",
            log.len()
        );
    }

    #[test]
    fn dynres_seeded_run_is_identical_twice() {
        let (frames_a, ctl_a) = seeded_load_run(true);
        let (frames_b, ctl_b) = seeded_load_run(true);
        assert_eq!(frames_a, frames_b);
        assert_eq!(ctl_a.changes(), ctl_b.changes());
    }

    #[test]
    fn scale_controller_spike_floors_through_the_governor() {
        let policy = ScalePolicy::default();
        let mut ctl = ScaleController::new(policy).expect("policy builds");
        assert_eq!(ctl.scale(), 1.0);
        // Steady at budget: the consumer holds full rate.
        for _ in 0..10 {
            ctl.update(policy.target_ms).expect("steady frame runs");
        }
        assert_eq!(ctl.scale(), 1.0);
        // A 3x spike: hysteresis alone would reach only one `step` below
        // full after `over_frames`; the governor panic drop floors it.
        for _ in 0..policy.over_frames {
            ctl.update(3.0 * policy.target_ms)
                .expect("spike frame runs");
        }
        assert_eq!(ctl.scale(), policy.min_scale);
        assert!(ctl.steps_down >= 1);
        let (rw, rh) = ctl.render_size(1920, 1080).expect("size runs");
        assert_eq!((rw, rh), (960, 540), "floor halves the render size");
    }
}

#[cfg(test)]
mod live_tests {
    use super::{LiveDynRes, LiveDynResConfig, LIVE_RUNGS_PCT};
    use engine_core::CounterRng;

    /// A stand-in GPU: 15 percent of the frame does not scale, the rest follows the pixel count.
    fn cost(base_ms: f32, pct: u8) -> f32 {
        let r = f32::from(pct) / 100.0;
        base_ms * (0.15 + 0.85 * r * r)
    }

    /// Drive the picker for `frames` frames; `load` gives the full-scale cost of each frame.
    /// Returns every frame's cost at the scale it ran at and the picker.
    fn run(frames: u32, mut load: impl FnMut(u32) -> f32) -> (Vec<f32>, LiveDynRes) {
        let mut ctl = LiveDynRes::new(LiveDynResConfig::default());
        let rng = CounterRng::new(2040);
        let mut pct = ctl.scale_pct();
        let mut seen = Vec::new();
        for f in 0..frames {
            let word = u32::try_from(rng.sample(u64::from(f)) >> 40).expect("24 bits");
            let noise =
                0.95 + 0.1 * (f32::from(u16::try_from(word >> 8).expect("16 bits")) / 65536.0);
            let ms = cost(load(f), pct) * noise;
            seen.push(ms);
            pct = ctl.observe(ms);
        }
        (seen, ctl)
    }

    fn p95(v: &[f32]) -> f32 {
        let mut s = v.to_vec();
        s.sort_by(f32::total_cmp);
        s[(s.len() * 95 / 100).min(s.len() - 1)]
    }

    #[test]
    fn a_load_inside_the_budget_never_moves_the_scale() {
        let (_, ctl) = run(1200, |_| 11.0);
        assert_eq!(ctl.changes(), 0);
        assert_eq!(ctl.scale_pct(), 100);
        assert_eq!(ctl.histogram()[0], 1200);
    }

    #[test]
    fn a_spike_steps_down_to_a_rung_that_fits_and_recovers_after() {
        let (seen, ctl) = run(1400, |f| if (300..700).contains(&f) { 24.0 } else { 11.0 });
        assert!(ctl.changes() >= 2, "it must go down and come back");
        let steps: Vec<_> = ctl.steps().collect();
        assert!(
            steps[0].frame >= 300 && steps[0].frame <= 330,
            "first drop at {}",
            steps[0].frame
        );
        assert!(steps[0].to_pct < 100);
        // Inside the spike, after the picker settled, the frame fits the budget.
        let settled = &seen[400..700];
        assert!(
            p95(settled) <= LiveDynResConfig::default().target_ms * 1.05,
            "p95 {}",
            p95(settled)
        );
        assert_eq!(
            ctl.scale_pct(),
            100,
            "back at full scale once the load is gone"
        );
    }

    #[test]
    fn a_steady_load_that_fits_only_lower_settles_without_flapping() {
        let (_, ctl) = run(4000, |_| 19.0);
        let last = ctl.steps().last().expect("it stepped down");
        assert!(last.frame < 120, "settled by frame {}", last.frame);
        assert!(
            ctl.changes() <= 3,
            "{} changes under a steady load",
            ctl.changes()
        );
        assert!(ctl.scale_pct() < 100);
    }

    #[test]
    fn changes_keep_their_distance_in_either_direction() {
        let cfg = LiveDynResConfig::default();
        let (_, ctl) = run(6000, |f| {
            let wave = if (f / 150) % 2 == 1 { 14.0 } else { 0.0 };
            9.0 + wave + f32::from(u8::try_from(f % 7).expect("under 7"))
        });
        let steps: Vec<_> = ctl.steps().collect();
        for pair in steps.windows(2) {
            let gap = pair[1].frame - pair[0].frame;
            let need = if pair[1].to_pct > pair[1].from_pct {
                cfg.hold_up_frames
            } else {
                cfg.hold_down_frames
            };
            assert!(
                gap >= need,
                "{pair:?} only {gap} frames apart, needs {need}"
            );
        }
    }

    #[test]
    fn the_scale_never_leaves_the_ladder() {
        let (_, ctl) = run(2000, |_| 500.0);
        assert_eq!(ctl.scale_pct(), *LIVE_RUNGS_PCT.last().expect("rungs"));
        assert!(LIVE_RUNGS_PCT.windows(2).all(|w| w[0] > w[1]));
    }

    #[test]
    fn bad_samples_count_the_frame_but_move_nothing() {
        let mut ctl = LiveDynRes::new(LiveDynResConfig::default());
        for bad in [f32::NAN, f32::INFINITY, -1.0] {
            for _ in 0..50 {
                assert_eq!(ctl.observe(bad), 100);
            }
        }
        assert_eq!(ctl.frames(), 150);
        assert_eq!(ctl.changes(), 0);
    }

    #[test]
    fn the_histogram_sums_to_the_frames_and_the_receipt_names_it() {
        let (_, ctl) = run(900, |f| if f > 200 { 28.0 } else { 11.0 });
        assert_eq!(ctl.histogram().iter().sum::<u32>(), ctl.frames());
        let line = ctl.receipt_line();
        assert!(
            line.starts_with("dynres: on target_ms=14.0 frames=900 changes="),
            "{line}"
        );
        assert!(
            line.contains("hist=100:") && line.contains(",67:"),
            "{line}"
        );
        assert!(
            line.contains("steps=") && !line.contains("steps=none"),
            "{line}"
        );
    }

    #[test]
    fn the_same_run_gives_the_same_receipt_twice() {
        let load = |f: u32| if (f / 200) % 2 == 1 { 26.0 } else { 12.0 };
        assert_eq!(
            run(2000, load).1.receipt_line(),
            run(2000, load).1.receipt_line()
        );
    }
}
