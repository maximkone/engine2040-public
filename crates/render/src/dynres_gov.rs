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
