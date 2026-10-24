//! dynamic_res: the potato tier's render-scale controller (SW171).
//!
//! Owner: render. Units: microseconds for frame times, permille for the scale
//! (1000 full, 750 the row's single drop). Presentation only: the scale sets
//! how big the scene target is before the upscale, so the sim never reads it
//! and the golden `d57b5c8ae3096614` never moves.
//!
//! Determinism: integers only, no clock, no libm, no heap. The p95 window is a
//! fixed 60-slot ring, the sort is a stable insertion sort over at most 60
//! slots, and every change is gated by [`MIN_STEP_FRAMES`], so the same
//! frame-time run always gives the same scale steps.
//! Errors: none; every function is total.
//!
//! Rule (row): the frame p95 over the last 60 frames over the tier budget drops
//! the scale to 0.75; 120 frames under it bring it back to 1.0; never more than
//! one step per 60 frames.

/// Render scale at full resolution, permille.
pub const SCALE_FULL_PERMILLE: u32 = 1000;
/// The potato tier's one drop, permille: the row's 0.75.
pub const SCALE_LOW_PERMILLE: u32 = 750;
/// Frames in the p95 window (row: the last 60 frames).
pub const P95_WINDOW: usize = 60;
/// Consecutive frames at or under budget before the scale climbs (row: 120).
pub const UP_FRAMES: u32 = 120;
/// Frames between two scale changes (row: at most one step per 60 frames).
pub const MIN_STEP_FRAMES: u32 = 60;

/// The potato tier's render-scale controller: a fixed 60-frame p95 window over
/// the tier's frame budget, one drop to [`SCALE_LOW_PERMILLE`] and one climb
/// back to [`SCALE_FULL_PERMILLE`], both gated to one step per
/// [`MIN_STEP_FRAMES`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DynamicRes {
    /// Tier budget in microseconds: the p95 reference a frame must beat.
    budget_us: u32,
    /// Scale now, permille.
    scale_permille: u32,
    /// The last [`P95_WINDOW`] frame times, oldest overwritten.
    ring: [u32; P95_WINDOW],
    /// Next slot the ring writes.
    at: usize,
    /// Live samples in the ring, saturating at the window length.
    filled: u32,
    /// Frames since the last scale change.
    since_step: u32,
    /// Consecutive frames at or under budget, the climb's counter.
    under: u32,
}

impl DynamicRes {
    /// A controller at full scale for `budget_us`; a zero budget reads as one
    /// microsecond, so the p95 test is never trivially true.
    #[must_use]
    pub const fn new(budget_us: u32) -> Self {
        Self {
            budget_us: if budget_us == 0 { 1 } else { budget_us },
            scale_permille: SCALE_FULL_PERMILLE,
            ring: [0; P95_WINDOW],
            at: 0,
            filled: 0,
            since_step: MIN_STEP_FRAMES,
            under: 0,
        }
    }

    /// Feed one frame time in microseconds and return the new scale in permille
    /// when this frame changed it. The drop reads the p95 of the last 60
    /// frames; the climb reads 120 straight frames at or under budget.
    pub fn step(&mut self, frame_us: u32) -> Option<u32> {
        self.ring[self.at] = frame_us;
        self.at = (self.at + 1) % P95_WINDOW;
        self.filled = self
            .filled
            .saturating_add(1)
            .min(u32::try_from(P95_WINDOW).unwrap_or(u32::MAX));
        self.since_step = self.since_step.saturating_add(1);
        self.under = if frame_us <= self.budget_us {
            self.under.saturating_add(1)
        } else {
            0
        };
        if self.since_step < MIN_STEP_FRAMES {
            return None;
        }
        if self.scale_permille > SCALE_LOW_PERMILLE && self.window_full() {
            if self.p95_us().is_some_and(|p95| p95 > self.budget_us) {
                return Some(self.change_to(SCALE_LOW_PERMILLE));
            }
        }
        if self.scale_permille < SCALE_FULL_PERMILLE && self.under >= UP_FRAMES {
            return Some(self.change_to(SCALE_FULL_PERMILLE));
        }
        None
    }

    /// Take a new scale and reopen the step gate.
    fn change_to(&mut self, permille: u32) -> u32 {
        self.scale_permille = permille;
        self.since_step = 0;
        self.under = 0;
        permille
    }

    /// True once the p95 window holds a full 60 frames.
    #[must_use]
    pub fn window_full(&self) -> bool {
        usize::try_from(self.filled).unwrap_or(0) >= P95_WINDOW
    }

    /// The 95th percentile of the frames held, microseconds: `None` before any
    /// frame. The rank is `ceil(0.95 n) - 1`, so 60 frames give the 57th.
    #[must_use]
    pub fn p95_us(&self) -> Option<u32> {
        let live = usize::try_from(self.filled).unwrap_or(0).min(P95_WINDOW);
        if live == 0 {
            return None;
        }
        let mut buf = self.ring;
        for i in 1..live {
            let mut j = i;
            while j > 0 && buf[j - 1] > buf[j] {
                buf.swap(j - 1, j);
                j -= 1;
            }
        }
        let rank = (95 * live).div_ceil(100) - 1;
        buf.get(rank).copied()
    }

    /// Scale now, permille.
    #[must_use]
    pub const fn scale_permille(&self) -> u32 {
        self.scale_permille
    }

    /// Scale now, percent of the output edge: what the upscaler takes.
    #[must_use]
    pub fn scale_pct(&self) -> u8 {
        match u8::try_from(self.scale_permille / 10) {
            Ok(pct) if pct <= 100 => pct,
            _ => 100,
        }
    }

    /// The tier budget this controller holds, microseconds.
    #[must_use]
    pub const fn budget_us(&self) -> u32 {
        self.budget_us
    }

    /// The scene target `width` by `height` at the current scale, at least one
    /// pixel each: the size the loop renders before the upscale.
    #[must_use]
    pub const fn scaled(&self, width: u32, height: u32) -> (u32, u32) {
        let w = width * self.scale_permille / SCALE_FULL_PERMILLE;
        let h = height * self.scale_permille / SCALE_FULL_PERMILLE;
        (if w == 0 { 1 } else { w }, if h == 0 { 1 } else { h })
    }

    /// The one-line receipt a run prints: `scale=0.75 p95_us=… budget_us=…`.
    /// Cold path only.
    #[must_use]
    pub fn receipt_line(&self) -> String {
        let p95 = self.p95_us().unwrap_or(0);
        format!(
            "scale={}.{:02} p95_us={p95} budget_us={}",
            self.scale_permille / 1000,
            (self.scale_permille % 1000) / 10,
            self.budget_us
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frames a 60 fps budget allows, microseconds.
    const BUDGET: u32 = 16_000;
    /// A frame over it, microseconds.
    const SLOW: u32 = 20_000;
    /// A frame under it, microseconds.
    const FAST: u32 = 8_000;

    #[test]
    fn scale_drops_to_seventy_five_after_sixty_frames_over_budget() {
        let mut res = DynamicRes::new(BUDGET);
        for frame in 1..60 {
            assert_eq!(res.step(SLOW), None, "no step at frame {frame}");
            assert_eq!(res.scale_permille(), SCALE_FULL_PERMILLE);
        }
        assert_eq!(res.p95_us(), Some(SLOW), "the window reads the slow frames");
        assert_eq!(res.step(SLOW), Some(SCALE_LOW_PERMILLE), "0.75 at frame 60");
        assert_eq!(res.scale_permille(), SCALE_LOW_PERMILLE);
        assert_eq!(res.scale_pct(), 75);
        // The scale never goes below the one drop, however slow the run.
        for _ in 0..120 {
            assert_eq!(res.step(SLOW), None, "one drop only");
        }
        assert_eq!(res.scale_permille(), SCALE_LOW_PERMILLE);
    }

    #[test]
    fn scale_recovers_after_a_hundred_and_twenty_frames_under_budget() {
        let mut res = DynamicRes::new(BUDGET);
        let mut steps = Vec::new();
        let mut frame = 0;
        for _ in 0..60 {
            frame += 1;
            if let Some(step) = res.step(SLOW) {
                steps.push((frame, step));
            }
        }
        for _ in 0..(UP_FRAMES - 1) {
            frame += 1;
            assert_eq!(res.step(FAST), None, "no climb before 120 frames");
        }
        frame += 1;
        assert_eq!(
            res.step(FAST),
            Some(SCALE_FULL_PERMILLE),
            "1.0 at frame 180"
        );
        steps.push((frame, SCALE_FULL_PERMILLE));
        assert_eq!(
            steps,
            vec![(60, SCALE_LOW_PERMILLE), (180, SCALE_FULL_PERMILLE)],
            "one step per 60 frames, and the row's two"
        );
    }

    #[test]
    fn alternating_input_never_flaps() {
        let mut res = DynamicRes::new(BUDGET);
        for _ in 0..60 {
            let _ = res.step(SLOW);
        }
        assert_eq!(res.scale_permille(), SCALE_LOW_PERMILLE);
        for frame in 0..600 {
            let frame_us = if frame % 2 == 0 { SLOW } else { FAST };
            assert_eq!(res.step(frame_us), None, "no step at {frame}");
        }
        assert_eq!(
            res.scale_permille(),
            SCALE_LOW_PERMILLE,
            "an alternating load holds 0.75"
        );
    }

    #[test]
    fn the_same_run_gives_the_same_scale() {
        let feed = |res: &mut DynamicRes| {
            let mut out = Vec::new();
            for frame in 0..400u32 {
                let frame_us = match frame % 7 {
                    0 | 1 | 2 => SLOW,
                    3 => BUDGET,
                    _ => FAST,
                };
                if let Some(step) = res.step(frame_us) {
                    out.push((frame, step));
                }
            }
            out
        };
        let mut a = DynamicRes::new(BUDGET);
        let mut b = DynamicRes::new(BUDGET);
        assert_eq!(feed(&mut a), feed(&mut b), "the same run replays alike");
        assert_eq!(a, b, "and the controllers land on the same state");
    }

    #[test]
    fn target_and_receipt_name_the_scale() {
        let mut res = DynamicRes::new(BUDGET);
        assert_eq!(res.receipt_line(), "scale=1.00 p95_us=0 budget_us=16000");
        assert_eq!(res.scaled(1920, 1080), (1920, 1080));
        for _ in 0..60 {
            let _ = res.step(SLOW);
        }
        assert_eq!(
            res.receipt_line(),
            "scale=0.75 p95_us=20000 budget_us=16000"
        );
        assert_eq!(res.scaled(1920, 1080), (1440, 810), "three quarters each");
        assert_eq!(res.scaled(1, 1), (1, 1), "never an empty target");
        assert_eq!(
            DynamicRes::new(0).budget_us(),
            1,
            "a zero budget reads as one microsecond"
        );
    }
}
