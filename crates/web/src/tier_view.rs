//! What the arena page prints about its quality tier and its frame cost, as pure functions so
//! they test without a GPU or a browser (WEB8): the `tier=` line, the one-second frame split
//! (sim, physfx, render, present) and the ladder that takes one cut after another when the
//! frame misses its budget, in Babylon's SceneOptimizer order.
//!
//! The tier table itself is `engine_render::quality` (the page maps its knobs into
//! [`TierFacts`]); this module only formats and decides, so the native workspace builds and
//! tests it without the GPU stack.

use std::fmt::Write as _;

/// The knobs of the chosen tier, as the page prints them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TierFacts {
    /// `potato`, `web`, `comfort` or `high`.
    pub name: String,
    /// Why this tier: `query`, `limits` or `default`.
    pub why: String,
    pub scale_pct: u32,
    pub shadow_px: u32,
    pub cascades: u32,
    /// `off`, `half` or `full`.
    pub gtao: String,
    pub taa: bool,
    pub particles: u32,
    pub decals: u32,
    pub physfx_bodies: u32,
    pub physfx_ragdolls: u32,
    pub physfx_contacts: u32,
    pub texture_px: u32,
    /// The knobs the page really applies (the rest are printed so the table is visible, but
    /// nothing draws with them yet), comma separated.
    pub wired: String,
}

impl TierFacts {
    /// The line the page prints once at start: `tier=web why=default scale=100 ...`.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "tier={} why={} scale={} shadow={}x{} gtao={} taa={} particles={} decals={} physfx={}/{}/{} tex={} wired={}",
            self.name,
            self.why,
            self.scale_pct,
            self.shadow_px,
            self.cascades,
            self.gtao,
            if self.taa { "on" } else { "off" },
            self.particles,
            self.decals,
            self.physfx_bodies,
            self.physfx_ragdolls,
            self.physfx_contacts,
            self.texture_px,
            self.wired
        )
    }
}

/// The averages of one window of frames, in milliseconds per frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Split {
    pub frames: u32,
    /// Time between animation frames.
    pub frame_ms: f64,
    /// The page's own simulation step (the fighters' pose and the draw list).
    pub sim_ms: f64,
    /// The physical layer (ragdolls, debris). Not wired into the page yet: always 0.
    pub physfx_ms: f64,
    /// Staging and encoding plus `queue.submit`.
    pub render_ms: f64,
    /// Waiting for the swapchain image plus the `present` call.
    pub present_ms: f64,
}

impl Split {
    /// Frames per second from the mean frame time; 0 when no frame was measured.
    #[must_use]
    pub fn fps(&self) -> f64 {
        if self.frame_ms > 0.0 {
            1000.0 / self.frame_ms
        } else {
            0.0
        }
    }

    /// `tier=web fps=60.0 frame=16.7ms sim=0.31 physfx=0.00 render=2.10 present=0.40`
    #[must_use]
    pub fn line(&self, tier: &str) -> String {
        format!(
            "tier={tier} fps={:.1} frame={:.1}ms sim={:.2} physfx={:.2} render={:.2} present={:.2}",
            self.fps(),
            self.frame_ms,
            self.sim_ms,
            self.physfx_ms,
            self.render_ms,
            self.present_ms
        )
    }
}

/// Collects frames and hands back their averages once per second.
#[derive(Clone, Copy, Debug, Default)]
pub struct SplitWindow {
    started_ms: Option<f64>,
    frames: u32,
    frame_ms: f64,
    sim_ms: f64,
    physfx_ms: f64,
    render_ms: f64,
    present_ms: f64,
}

/// Length of a window, milliseconds.
pub const WINDOW_MS: f64 = 1000.0;

impl SplitWindow {
    /// Add one frame. `now_ms` starts the window on the first frame.
    pub fn add(
        &mut self,
        now_ms: f64,
        frame_ms: f64,
        sim_ms: f64,
        render_ms: f64,
        present_ms: f64,
    ) {
        self.started_ms.get_or_insert(now_ms);
        let clean = |v: f64| if v.is_finite() && v > 0.0 { v } else { 0.0 };
        self.frames += 1;
        self.frame_ms += clean(frame_ms);
        self.sim_ms += clean(sim_ms);
        self.render_ms += clean(render_ms);
        self.present_ms += clean(present_ms);
    }

    /// The averages when a second has passed since the window opened (and the window starts
    /// over), `None` before that or when it holds no frame.
    pub fn take_if_due(&mut self, now_ms: f64) -> Option<Split> {
        let started = self.started_ms?;
        if now_ms - started < WINDOW_MS || self.frames == 0 {
            return None;
        }
        let n = f64::from(self.frames);
        let split = Split {
            frames: self.frames,
            frame_ms: self.frame_ms / n,
            sim_ms: self.sim_ms / n,
            physfx_ms: self.physfx_ms / n,
            render_ms: self.render_ms / n,
            present_ms: self.present_ms / n,
        };
        *self = Self {
            started_ms: Some(now_ms),
            ..Self::default()
        };
        Some(split)
    }
}

/// The ladder: when the frame misses its budget for [`MISSES_BEFORE_CUT`] windows in a row it
/// takes the next cut, in the order the caller names (`engine_render::quality::WEB_CUT_ORDER`).
/// The last step may repeat (the render scale goes down a notch each time) up to
/// `last_repeats` times, then the ladder is spent.
#[derive(Clone, Copy, Debug)]
pub struct Ladder {
    steps: usize,
    last_repeats: u32,
    budget_ms: f64,
    next: usize,
    repeats_used: u32,
    misses: u32,
}

/// Windows over budget in a row before one cut.
pub const MISSES_BEFORE_CUT: u32 = 2;

/// A window slower than this (under 2 frames per second) is a throttled or stalled page (a tab
/// the browser slows to one frame a second, a window behind another), not a slow picture: no
/// cut fixes it, so the ladder ignores it instead of stripping the quality for good.
pub const STALL_MS: f64 = 500.0;

/// The step the ladder takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cut {
    /// Index into the cut order.
    pub step: usize,
    /// 0 for the first time this step is taken, 1 for the second, ...
    pub repeat: u32,
}

impl Ladder {
    /// A ladder of `steps` cuts; the last one may run `last_repeats` more times.
    #[must_use]
    pub fn new(steps: usize, last_repeats: u32, budget_ms: f64) -> Self {
        Self {
            steps,
            last_repeats,
            budget_ms,
            next: 0,
            repeats_used: 0,
            misses: 0,
        }
    }

    /// Feed one window's mean frame time. A cut comes back after two misses in a row; a window
    /// at or under budget clears the count.
    pub fn observe(&mut self, frame_ms: f64) -> Option<Cut> {
        if frame_ms > STALL_MS {
            return None;
        }
        if self.steps == 0 || frame_ms.is_nan() || frame_ms <= self.budget_ms {
            self.misses = 0;
            return None;
        }
        self.misses += 1;
        if self.misses < MISSES_BEFORE_CUT {
            return None;
        }
        self.misses = 0;
        let last = self.steps - 1;
        if self.next < last {
            let cut = Cut {
                step: self.next,
                repeat: 0,
            };
            self.next += 1;
            return Some(cut);
        }
        if self.repeats_used > self.last_repeats {
            return None;
        }
        let cut = Cut {
            step: last,
            repeat: self.repeats_used,
        };
        self.repeats_used += 1;
        self.next = last;
        Some(cut)
    }

    /// True when no cut is left.
    #[must_use]
    pub fn spent(&self) -> bool {
        self.steps == 0 || (self.next + 1 >= self.steps && self.repeats_used > self.last_repeats)
    }
}

/// The render scale after the `repeat`-th scale cut: 15 points lower each time, never under
/// `floor`.
#[must_use]
pub fn scale_after_cut(start_pct: u32, repeat: u32, floor_pct: u32) -> u32 {
    let cut = 15u32.saturating_mul(repeat + 1);
    start_pct.saturating_sub(cut).max(floor_pct.min(start_pct))
}

/// `tier=web cut=shadows scale=100`: the loud line a cut prints.
#[must_use]
pub fn cut_line(tier: &str, name: &str, repeat: u32, applied: bool, scale_pct: u32) -> String {
    let mut line = format!("tier={tier} cut={name}");
    if repeat > 0 {
        let _ = write!(line, " repeat={repeat}");
    }
    let _ = write!(
        line,
        " applied={} scale={scale_pct}",
        if applied { "yes" } else { "no" }
    );
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> TierFacts {
        TierFacts {
            name: "web".into(),
            why: "default".into(),
            scale_pct: 100,
            shadow_px: 1024,
            cascades: 2,
            gtao: "half".into(),
            taa: false,
            particles: 2048,
            decals: 128,
            physfx_bodies: 192,
            physfx_ragdolls: 4,
            physfx_contacts: 512,
            texture_px: 1024,
            wired: "scale,bloom".into(),
        }
    }

    #[test]
    fn the_tier_line_starts_with_tier_equals_name_and_is_one_line() {
        let line = facts().line();
        assert!(
            line.starts_with("tier=web why=default scale=100 "),
            "{line}"
        );
        assert!(line.contains("shadow=1024x2 gtao=half taa=off"), "{line}");
        assert!(
            line.contains("physfx=192/4/512 tex=1024 wired=scale,bloom"),
            "{line}"
        );
        assert!(!line.contains('\n'));
    }

    #[test]
    fn the_split_line_carries_the_tier_and_all_four_fields() {
        let s = Split {
            frames: 60,
            frame_ms: 16.7,
            sim_ms: 0.3,
            physfx_ms: 0.0,
            render_ms: 2.1,
            present_ms: 0.4,
        };
        let line = s.line("web");
        assert_eq!(
            line,
            "tier=web fps=59.9 frame=16.7ms sim=0.30 physfx=0.00 render=2.10 present=0.40"
        );
        for field in ["sim=", "physfx=", "render=", "present="] {
            assert!(line.contains(field), "{field} missing from {line}");
        }
        assert_eq!(Split::default().fps(), 0.0);
    }

    #[test]
    fn a_window_averages_its_frames_once_per_second_and_starts_over() {
        let mut w = SplitWindow::default();
        assert!(w.take_if_due(0.0).is_none(), "an empty window is never due");
        for i in 0..4u32 {
            let now = 100.0 + f64::from(i) * 400.0;
            w.add(now, 20.0 + f64::from(i) * 10.0, 1.0, 2.0, 0.5);
            let due = w.take_if_due(now);
            if i < 3 {
                assert!(due.is_none(), "frame {i} is inside the second");
            } else {
                let s = due.expect("the fourth frame closes the second");
                assert_eq!(s.frames, 4);
                assert!((s.frame_ms - 35.0).abs() < 1e-9, "{}", s.frame_ms);
                assert!((s.sim_ms - 1.0).abs() < 1e-9);
                assert!((s.render_ms - 2.0).abs() < 1e-9);
                assert!((s.present_ms - 0.5).abs() < 1e-9);
                assert_eq!(s.physfx_ms, 0.0);
            }
        }
        // The next window starts at the frame that closed the last one.
        w.add(1500.0, 10.0, 0.0, 0.0, 0.0);
        assert!(w.take_if_due(2000.0).is_none());
        assert_eq!(w.take_if_due(2400.0).map(|s| s.frames), Some(1));
    }

    #[test]
    fn bad_readings_count_as_zero_not_nan() {
        let mut w = SplitWindow::default();
        w.add(0.0, f64::NAN, -1.0, f64::INFINITY, 0.0);
        let s = w.take_if_due(2000.0).expect("due");
        assert_eq!(
            (s.frame_ms, s.sim_ms, s.render_ms, s.present_ms),
            (0.0, 0.0, 0.0, 0.0)
        );
        assert!(!s.line("web").contains("NaN"));
    }

    #[test]
    fn the_ladder_takes_a_cut_after_two_misses_in_a_row_in_order() {
        let mut l = Ladder::new(6, 2, 33.4);
        assert_eq!(l.observe(16.0), None);
        assert_eq!(l.observe(50.0), None, "one miss is noise");
        assert_eq!(l.observe(16.0), None, "a good window clears the count");
        assert_eq!(l.observe(50.0), None);
        assert_eq!(l.observe(50.0), Some(Cut { step: 0, repeat: 0 }));
        let mut steps = Vec::new();
        while let Some(c) = {
            l.observe(60.0);
            l.observe(60.0)
        } {
            steps.push((c.step, c.repeat));
        }
        assert_eq!(
            steps,
            [(1, 0), (2, 0), (3, 0), (4, 0), (5, 0), (5, 1), (5, 2)],
            "the last step repeats twice more, then the ladder is spent"
        );
        assert!(l.spent());
        assert_eq!(l.observe(60.0), None);
        assert_eq!(l.observe(60.0), None);
    }

    #[test]
    fn a_stalled_window_is_ignored_not_counted() {
        let mut l = Ladder::new(6, 0, 33.4);
        for _ in 0..20 {
            assert_eq!(
                l.observe(1000.0),
                None,
                "a throttled tab is not a slow picture"
            );
        }
        // It neither counts as a miss nor clears one.
        assert_eq!(l.observe(50.0), None);
        assert_eq!(l.observe(1000.0), None);
        assert_eq!(l.observe(50.0), Some(Cut { step: 0, repeat: 0 }));
    }

    #[test]
    fn a_frame_on_budget_never_cuts_and_a_nan_is_not_a_miss() {
        let mut l = Ladder::new(6, 0, 33.4);
        for _ in 0..50 {
            assert_eq!(l.observe(33.4), None);
            assert_eq!(l.observe(f64::NAN), None);
        }
        assert!(!l.spent());
        assert!(Ladder::new(0, 0, 10.0).spent());
    }

    #[test]
    fn the_scale_goes_down_fifteen_points_a_cut_and_stops_at_the_floor() {
        assert_eq!(scale_after_cut(100, 0, 60), 85);
        assert_eq!(scale_after_cut(100, 1, 60), 70);
        assert_eq!(scale_after_cut(100, 2, 60), 60);
        assert_eq!(scale_after_cut(100, 9, 60), 60);
        assert_eq!(
            scale_after_cut(50, 0, 60),
            50,
            "never above where it started"
        );
    }

    #[test]
    fn a_cut_line_names_the_tier_the_cut_and_whether_it_changed_the_picture() {
        assert_eq!(
            cut_line("web", "shadows", 0, false, 100),
            "tier=web cut=shadows applied=no scale=100"
        );
        assert_eq!(
            cut_line("web", "render_scale", 1, true, 70),
            "tier=web cut=render_scale repeat=1 applied=yes scale=70"
        );
    }
}
