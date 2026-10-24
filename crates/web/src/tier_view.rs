//! What the arena page prints about its quality tier and its frame cost, as pure functions so
//! they test without a GPU or a browser (WEB8): the `tier=` line, the one-second frame split
//! (sim, physfx, render, present) and the ladder that takes one cut after another when the
//! frame misses its budget, in Babylon's SceneOptimizer order.
//!
//! Around the ladder sit the checks that keep it honest: the startup probe's bookkeeping
//! ([`ProbeRun`]: settle first, repeat a window a compile spoiled, start over after a hidden
//! stretch), the slow slide check ([`Drift`]) and [`Pace`], which owns the one-second window,
//! the ladder and the drift check, so a hidden tab or a 10-minute series tests as plain data.
//!
//! The tier table itself is `engine_render::quality` (the page maps its knobs into
//! [`TierFacts`]); this module only formats and decides, so the native workspace builds and
//! tests it without the GPU stack.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::time::Duration;

/// The render scale never goes under this many percent (below it the picture is a smear).
pub const SCALE_FLOOR_PCT: u32 = 50;

/// The knobs of the chosen tier, as the page prints them.
#[derive(Clone, Debug, PartialEq)]
pub struct TierFacts {
    /// `potato`, `mobile`, `web`, `comfort` or `high`.
    pub name: String,
    /// Why this tier: `query`, `limits`, `denylist`, `probe` or `default`.
    pub why: String,
    /// What the browser said about the adapter (see `limits::adapter_hint`): text without
    /// spaces, or `none`. A hint to read, never an input of the tier pick (the denylist apart).
    pub adapter: String,
    /// Pipelines built while the startup probe timed frames (0 on a healthy page).
    pub late_compile: u32,
    /// Median milliseconds of the startup probe, `None` when it did not run (a `?tier=` query,
    /// a software adapter, a tab that stalled).
    pub probe_ms: Option<f64>,
    /// The device pixel ratio the page saw.
    pub dpr: f64,
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
    /// Largest texture edge the uploads keep resident, already cut to the adapter's
    /// `maxTextureDimension2D` (`engine_render::quality::texture_edge_px`, QT9); the tier line
    /// prints it as `tex_max=`.
    pub texture_px: u32,
    /// The knobs the page really applies (the rest are printed so the table is visible, but
    /// nothing draws with them), comma separated. See [`WIRED`].
    pub wired: String,
}

/// The knobs the arena page applies to what it draws: the render scale and bloom (the page),
/// the sun shadow map size, the cascade band, the AO pass, the texture size cap and the particle
/// and decal pool caps (`Presenter::apply_quality`). `taa` is printed but not applied: the
/// live frame has no temporal pass. The particle and decal pools are sized, but the live frame
/// does not draw them yet.
pub const WIRED: &str = "scale,bloom,shadow,cascades,gtao,tex,particles,decals";

/// Why the page runs below the web tier, `none` when it does not: the tier was asked for
/// (`?tier=`) or is the default. The reasons are the tier's own `why` (QT11).
#[must_use]
pub fn fallback_for(why: &str) -> &'static str {
    match why {
        "limits" => "limits",
        "denylist" => "denylist",
        "probe" => "probe",
        _ => "none",
    }
}

impl TierFacts {
    /// The line the page prints once at start:
    /// `tier=web why=default fallback=none adapter=none late_compile=0 probe_ms=na scale=100 dpr=1.00 ...`.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "tier={} why={} fallback={} adapter={} late_compile={} probe_ms={} scale={} dpr={:.2} shadow={}x{} gtao={} taa={} particles={} decals={} physfx={}/{}/{} tex_max={} wired={}",
            self.name,
            self.why,
            fallback_for(&self.why),
            self.adapter,
            self.late_compile,
            self.probe_ms
                .map_or_else(|| "na".to_string(), |ms| format!("{ms:.1}")),
            self.scale_pct,
            self.dpr,
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
    /// Frames dropped so far because they spanned a hidden stretch (QT7).
    pub hidden_skips: u32,
    /// Percent the last minute's mean frame time is over the first minute's, `None` until a
    /// full minute has followed the first (see [`Drift`]).
    pub drift_pct: Option<f64>,
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

    /// `tier=web fps=60.0 frame=16.7ms sim=0.31 physfx=0.00 render=2.10 present=0.40 scale=100
    /// dpr=1.00 drift=na hidden_skips=0`. `scale_pct` and `dpr` are the page's right now.
    #[must_use]
    pub fn line(&self, tier: &str, scale_pct: u32, dpr: f64) -> String {
        format!(
            "tier={tier} fps={:.1} frame={:.1}ms sim={:.2} physfx={:.2} render={:.2} present={:.2} scale={scale_pct} dpr={dpr:.2} drift={} hidden_skips={}",
            self.fps(),
            self.frame_ms,
            self.sim_ms,
            self.physfx_ms,
            self.render_ms,
            self.present_ms,
            self.drift_pct
                .map_or_else(|| "na".to_string(), |pct| format!("{pct:.1}")),
            self.hidden_skips
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
    /// Hidden stretches dropped over the whole run; a window closing does not clear it.
    skipped: u32,
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

    /// The tab was hidden and came back: the frame that spans the gap is not a slow frame and
    /// neither is anything the window held before it. Drops them all and counts the skip.
    pub fn drop_gap(&mut self) {
        *self = Self {
            skipped: self.skipped + 1,
            ..Self::default()
        };
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
            hidden_skips: self.skipped,
            drift_pct: None,
        };
        *self = Self {
            started_ms: Some(now_ms),
            skipped: self.skipped,
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
    /// Windows in a row with a mean under [`HEADROOM_FRACTION`] of the budget.
    headroom: u32,
    /// Windows since the last restore (`u32::MAX`: none yet).
    since_restore: u32,
    failed_restores: u32,
    restores: u32,
    /// Windows left in which no cut is given back (set by [`Ladder::force_cut`]).
    restore_hold: u32,
}

/// A window counts as headroom when its mean frame time is under this share of the budget.
pub const HEADROOM_FRACTION: f64 = 0.8;

/// Windows of headroom in a row (ten seconds) before the ladder gives one cut back.
pub const HEADROOM_WINDOWS: u32 = 10;

/// A cut that follows a restore within this many windows (thirty seconds) means the restore
/// missed.
pub const RESTORE_RETRY_WINDOWS: u32 = 30;

/// Restores that missed before the ladder stops trying them for the rest of the run.
pub const MAX_FAILED_RESTORES: u32 = 2;

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
            headroom: 0,
            since_restore: u32::MAX,
            failed_restores: 0,
            restores: 0,
            restore_hold: 0,
        }
    }

    /// Book-keeping of a cut just taken: the headroom count starts over, and a cut soon after a
    /// restore marks that restore as missed.
    fn took_cut(&mut self) {
        self.headroom = 0;
        if self.since_restore < RESTORE_RETRY_WINDOWS {
            self.failed_restores += 1;
        }
    }

    /// The cut taken last, `None` when none stands.
    fn last_cut(&self) -> Option<Cut> {
        let last = self.steps.checked_sub(1)?;
        if self.repeats_used > 0 {
            Some(Cut {
                step: last,
                repeat: self.repeats_used - 1,
            })
        } else if self.next > 0 {
            Some(Cut {
                step: self.next - 1,
                repeat: 0,
            })
        } else {
            None
        }
    }

    /// After a window that took no cut: the cut to give back once ten windows in a row ran under
    /// 80 percent of the budget. Two restores that missed (a cut within thirty windows of the
    /// restore) end restoring for the run. The caller undoes the returned cut.
    pub fn restore_due(&mut self) -> Option<Cut> {
        if self.headroom < HEADROOM_WINDOWS
            || self.failed_restores >= MAX_FAILED_RESTORES
            || self.restore_hold > 0
        {
            return None;
        }
        let cut = self.last_cut()?;
        if cut.step + 1 == self.steps && self.repeats_used > 0 {
            self.repeats_used = cut.repeat;
        } else {
            self.next = cut.step;
        }
        self.headroom = 0;
        self.since_restore = 0;
        self.restores += 1;
        Some(cut)
    }

    /// Restores given so far, for the `restore=<n>` the page prints.
    #[must_use]
    pub fn restores(&self) -> u32 {
        self.restores
    }

    /// Feed one window's mean frame time. A cut comes back after two misses in a row; a window
    /// at or under budget clears the count.
    pub fn observe(&mut self, frame_ms: f64) -> Option<Cut> {
        if frame_ms > STALL_MS {
            return None;
        }
        self.since_restore = self.since_restore.saturating_add(1);
        self.restore_hold = self.restore_hold.saturating_sub(1);
        if frame_ms.is_finite() && frame_ms <= self.budget_ms * HEADROOM_FRACTION {
            self.headroom += 1;
        } else {
            self.headroom = 0;
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
        self.take_next_cut()
    }

    /// The next cut in order, whatever the last windows said; `None` when the ladder is spent.
    fn take_next_cut(&mut self) -> Option<Cut> {
        let last = self.steps.checked_sub(1)?;
        if self.next < last {
            let cut = Cut {
                step: self.next,
                repeat: 0,
            };
            self.next += 1;
            self.took_cut();
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
        self.took_cut();
        Some(cut)
    }

    /// The extra cut of the drift check (QT8): the next cut now, and no cut is given back for
    /// the next `hold_windows` windows. `None` (and no hold) when the ladder is spent.
    pub fn force_cut(&mut self, hold_windows: u32) -> Option<Cut> {
        let cut = self.take_next_cut()?;
        self.restore_hold = hold_windows;
        self.misses = 0;
        Some(cut)
    }

    /// True when no cut is left.
    #[must_use]
    pub fn spent(&self) -> bool {
        self.steps == 0 || (self.next + 1 >= self.steps && self.repeats_used > self.last_repeats)
    }
}

/// Windows (seconds) in the baseline and in the rolling mean the drift check compares to it.
pub const DRIFT_WINDOWS: usize = 60;

/// Drift over this many percent counts as a slide.
pub const DRIFT_LIMIT_PCT: f64 = 20.0;

/// Windows (three minutes) the drift must stay over the limit before the extra cut.
pub const DRIFT_CUT_WINDOWS: u32 = 180;

/// Windows (five minutes) without a restore after the drift check cut.
pub const DRIFT_NO_RESTORE_WINDOWS: u32 = 300;

/// A float count as a number without a cast.
fn count_f64(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// The slow slide check (QT8): a heap that fills, a thermal throttle, a leak. The first minute of
/// windows sets a baseline; after it a rolling mean of the last minute is compared with the
/// baseline. When that drift stays over [`DRIFT_LIMIT_PCT`] for [`DRIFT_CUT_WINDOWS`] windows in
/// a row, [`Drift::observe`] says so once and starts counting again. Stalled windows
/// ([`STALL_MS`]) are not frame times and are left out.
#[derive(Clone, Debug, Default)]
pub struct Drift {
    base_sum: f64,
    base_windows: usize,
    recent: VecDeque<f64>,
    over: u32,
    pct: Option<f64>,
}

impl Drift {
    /// Feed one window's mean frame time. True when the drift has stayed over the limit for three
    /// minutes: the page takes one extra cut.
    pub fn observe(&mut self, frame_ms: f64) -> bool {
        if !frame_ms.is_finite() || frame_ms <= 0.0 || frame_ms > STALL_MS {
            return false;
        }
        if self.base_windows < DRIFT_WINDOWS {
            self.base_sum += frame_ms;
            self.base_windows += 1;
            return false;
        }
        if self.recent.len() == DRIFT_WINDOWS {
            self.recent.pop_front();
        }
        self.recent.push_back(frame_ms);
        if self.recent.len() < DRIFT_WINDOWS {
            return false;
        }
        let base = self.base_sum / count_f64(self.base_windows);
        let mean = self.recent.iter().sum::<f64>() / count_f64(self.recent.len());
        let pct = (mean - base) / base * 100.0;
        self.pct = Some(pct);
        if pct > DRIFT_LIMIT_PCT {
            self.over += 1;
        } else {
            self.over = 0;
        }
        if self.over < DRIFT_CUT_WINDOWS {
            return false;
        }
        self.over = 0;
        true
    }

    /// The latest drift in percent, `None` until the rolling minute is full.
    #[must_use]
    pub fn pct(&self) -> Option<f64> {
        self.pct
    }
}

/// One page's frame pacing: the one-second window, the ladder and the drift check. The page
/// feeds it frames and asks it once a frame what the closed second decided.
#[derive(Clone, Debug)]
pub struct Pace {
    window: SplitWindow,
    ladder: Option<Ladder>,
    drift: Drift,
}

/// What one closed window decided.
#[derive(Clone, Copy, Debug)]
pub struct Tick {
    /// The window's averages, with the drift and the hidden skips filled in.
    pub split: Split,
    /// A cut to take (`false`) or to give back (`true`).
    pub step: Option<(Cut, bool)>,
}

impl Pace {
    /// `ladder` is `None` when cutting is off (`?ladder=off`, a software adapter that no cut
    /// helps); the window and the drift reading still run.
    #[must_use]
    pub fn new(ladder: Option<Ladder>) -> Self {
        Self {
            window: SplitWindow::default(),
            ladder,
            drift: Drift::default(),
        }
    }

    /// Add one frame to the window.
    pub fn frame(
        &mut self,
        now_ms: f64,
        frame_ms: f64,
        sim_ms: f64,
        render_ms: f64,
        present_ms: f64,
    ) {
        self.window
            .add(now_ms, frame_ms, sim_ms, render_ms, present_ms);
    }

    /// The tab came back from hidden (QT7): the frame that spans the gap is dropped from the
    /// window and counted, so a long gap is never taken for a slow frame.
    pub fn resumed(&mut self) {
        self.window.drop_gap();
    }

    /// Hidden stretches dropped so far.
    #[must_use]
    pub fn hidden_skips(&self) -> u32 {
        self.window.skipped
    }

    /// Restores the ladder has given back, for the `restores=<n>` the page prints.
    #[must_use]
    pub fn restores(&self) -> u32 {
        self.ladder.as_ref().map_or(0, Ladder::restores)
    }

    /// Once a frame: the closed second, if one closed. `may_cut` is false while a capture is
    /// on its way or the tab is hidden: the second is reported, but nothing is cut and the
    /// drift check does not see it.
    pub fn tick(&mut self, now_ms: f64, may_cut: bool) -> Option<Tick> {
        let mut split = self.window.take_if_due(now_ms)?;
        let mut step = None;
        if may_cut {
            if let Some(ladder) = &mut self.ladder {
                step = match ladder.observe(split.frame_ms) {
                    Some(cut) => Some((cut, false)),
                    None => ladder.restore_due().map(|back| (back, true)),
                };
            }
            if self.drift.observe(split.frame_ms) && step.is_none() {
                step = self
                    .ladder
                    .as_mut()
                    .and_then(|l| l.force_cut(DRIFT_NO_RESTORE_WINDOWS))
                    .map(|cut| (cut, false));
            }
        }
        split.drift_pct = self.drift.pct();
        Some(Tick { split, step })
    }
}

/// Frames the startup probe draws and does not time, so the browser's first compiles are over
/// before the clock starts (QT2).
pub const PROBE_SETTLE_FRAMES: u32 = 30;

/// The startup probe's bookkeeping (QT1, QT2, QT7), with no GPU and no clock of its own: the
/// page feeds it each frame's milliseconds and the ledger's running count of pipeline builds.
///
/// - The first [`PROBE_SETTLE_FRAMES`] frames are not timed (and not longer than half of the
///   window's time budget).
/// - A window then collects `frames` samples, or frames up to `max_ms` of time.
/// - A pipeline built while a window collects (a `late_compile`) spoils it: the window is thrown
///   away and runs once more, never twice.
/// - [`ProbeRun::restart`] (the tab was hidden and came back) starts over from the settle.
#[derive(Clone, Debug)]
pub struct ProbeRun {
    frames: usize,
    min_frames: usize,
    max_ms: f64,
    settle_left: u32,
    settle_ms: f64,
    samples: Vec<f64>,
    window_ms: f64,
    builds_seen: usize,
    window_builds: u32,
    late_compile: u32,
    repeated: bool,
}

/// What a finished probe hands back.
#[derive(Clone, Debug, PartialEq)]
pub struct ProbeDone {
    /// The frame times of the last window; empty when fewer than `min_frames` fitted in the
    /// time (a stalled or throttled tab says nothing about the GPU).
    pub samples: Vec<f64>,
    /// Pipelines built while probing, over all windows.
    pub late_compile: u32,
    /// True when a spoiled window was run again.
    pub repeated: bool,
}

impl ProbeRun {
    /// A probe of `frames` samples per window, at least `min_frames` of them, `max_ms` of frame
    /// time at most.
    #[must_use]
    pub fn new(frames: usize, min_frames: usize, max_ms: f64) -> Self {
        Self {
            frames,
            min_frames,
            max_ms,
            settle_left: PROBE_SETTLE_FRAMES,
            settle_ms: 0.0,
            samples: Vec::new(),
            window_ms: 0.0,
            builds_seen: 0,
            window_builds: 0,
            late_compile: 0,
            repeated: false,
        }
    }

    /// Start over from the settle: the tab was hidden or came back, so what the window holds
    /// says nothing about the GPU. Builds already counted stay counted.
    pub fn restart(&mut self) {
        self.settle_left = PROBE_SETTLE_FRAMES;
        self.settle_ms = 0.0;
        self.samples.clear();
        self.window_ms = 0.0;
        self.window_builds = 0;
    }

    /// Give up waiting (the tab stayed hidden): no samples, so no figure, and the builds seen.
    #[must_use]
    pub fn abandon(&self) -> ProbeDone {
        ProbeDone {
            samples: Vec::new(),
            late_compile: self.late_compile.saturating_add(self.window_builds),
            repeated: self.repeated,
        }
    }

    /// Feed one probe frame: its milliseconds and the ledger's builds so far. `Some` when the
    /// probe is over.
    pub fn frame(&mut self, ms: f64, builds_total: usize) -> Option<ProbeDone> {
        let ms = if ms.is_finite() && ms > 0.0 { ms } else { 0.0 };
        let new_builds =
            u32::try_from(builds_total.saturating_sub(self.builds_seen)).unwrap_or(u32::MAX);
        self.builds_seen = builds_total;
        if self.settle_left > 0 {
            // Builds while settling are the warm-up's tail, not the window's.
            self.settle_left -= 1;
            self.settle_ms += ms;
            if self.settle_ms >= self.max_ms / 2.0 {
                self.settle_left = 0;
            }
            return None;
        }
        self.window_builds = self.window_builds.saturating_add(new_builds);
        self.samples.push(ms);
        self.window_ms += ms;
        if self.samples.len() < self.frames && self.window_ms < self.max_ms {
            return None;
        }
        self.late_compile = self.late_compile.saturating_add(self.window_builds);
        if self.window_builds > 0 && !self.repeated {
            self.repeated = true;
            self.samples.clear();
            self.window_ms = 0.0;
            self.window_builds = 0;
            return None;
        }
        let samples = if self.samples.len() >= self.min_frames {
            std::mem::take(&mut self.samples)
        } else {
            Vec::new()
        };
        Some(ProbeDone {
            samples,
            late_compile: self.late_compile,
            repeated: self.repeated,
        })
    }
}

/// Microseconds as milliseconds, with no float cast.
#[must_use]
pub fn us_to_ms(us: u64) -> f64 {
    Duration::from_micros(us).as_secs_f64() * 1000.0
}

/// Whole milliseconds of a float reading; a bad or negative reading is 0.
#[must_use]
pub fn whole_ms(ms: f64) -> u64 {
    if !ms.is_finite() || ms <= 0.0 {
        return 0;
    }
    let secs = (ms / 1000.0).min(1.0e9);
    u64::try_from(Duration::from_secs_f64(secs).as_millis()).unwrap_or(u64::MAX)
}

/// The render scale after the `repeat`-th scale cut: 15 points lower each time, never under
/// `floor`.
#[must_use]
pub fn scale_after_cut(start_pct: u32, repeat: u32, floor_pct: u32) -> u32 {
    let cut = 15u32.saturating_mul(repeat + 1);
    start_pct.saturating_sub(cut).max(floor_pct.min(start_pct))
}

/// The render scale once the `repeat`-th scale cut is given back: the scale before that cut
/// (the tier's own for the first one).
#[must_use]
pub fn scale_after_restore(start_pct: u32, repeat: u32, floor_pct: u32) -> u32 {
    match repeat.checked_sub(1) {
        Some(before) => scale_after_cut(start_pct, before, floor_pct),
        None => start_pct,
    }
}

/// `tier=web restore=shadows applied=no scale=100 restores=1`: the line a given-back cut prints.
#[must_use]
pub fn restore_line(
    tier: &str,
    name: &str,
    repeat: u32,
    applied: bool,
    scale_pct: u32,
    restores: u32,
) -> String {
    let line = cut_line(tier, name, repeat, applied, scale_pct);
    format!(
        "{} restores={restores}",
        line.replacen("cut=", "restore=", 1)
    )
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
            adapter: "nvidia/ampere".into(),
            late_compile: 0,
            probe_ms: Some(12.34),
            dpr: 1.0,
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
            wired: WIRED.into(),
        }
    }

    #[test]
    fn the_tier_line_starts_with_tier_equals_name_and_is_one_line() {
        let line = facts().line();
        assert!(
            line.starts_with(
                "tier=web why=default fallback=none adapter=nvidia/ampere late_compile=0 probe_ms=12.3 scale=100 dpr=1.00 "
            ),
            "{line}"
        );
        assert!(line.contains("shadow=1024x2 gtao=half taa=off"), "{line}");
        assert!(
            line.contains("physfx=192/4/512 tex_max=1024 wired=scale,bloom,shadow,"),
            "{line}"
        );
        assert!(!line.contains('\n'));
    }

    #[test]
    fn a_probe_that_did_not_run_prints_na_and_the_wired_list_names_every_applied_knob() {
        let mut f = facts();
        f.probe_ms = None;
        assert!(
            f.line().contains(" probe_ms=na scale=100 dpr=1.00 "),
            "{}",
            f.line()
        );
        let wired: Vec<&str> = WIRED.split(',').collect();
        for knob in ["shadow", "cascades", "gtao", "tex", "particles", "decals"] {
            assert!(wired.contains(&knob), "{knob} is applied but not listed");
        }
        assert!(
            !wired.contains(&"taa"),
            "the live frame has no temporal pass"
        );
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
            hidden_skips: 2,
            drift_pct: Some(2.75),
        };
        let line = s.line("web", 85, 2.0);
        assert_eq!(
            line,
            "tier=web fps=59.9 frame=16.7ms sim=0.30 physfx=0.00 render=2.10 present=0.40 scale=85 dpr=2.00 drift=2.8 hidden_skips=2"
        );
        for field in ["sim=", "physfx=", "render=", "present="] {
            assert!(line.contains(field), "{field} missing from {line}");
        }
        assert_eq!(Split::default().fps(), 0.0);
        assert!(
            Split::default()
                .line("web", 100, 1.0)
                .contains(" drift=na "),
            "no drift reading before the second minute"
        );
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
        assert!(!s.line("web", 100, 1.0).contains("NaN"));
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

    /// Feed `n` windows of `ms`; the last cut or restore that came back.
    fn windows(l: &mut Ladder, n: u32, ms: f64) -> (Option<Cut>, Option<Cut>) {
        let mut out = (None, None);
        for _ in 0..n {
            match l.observe(ms) {
                Some(cut) => out.0 = Some(cut),
                None => {
                    if let Some(back) = l.restore_due() {
                        out.1 = Some(back);
                    }
                }
            }
        }
        out
    }

    #[test]
    fn ten_windows_of_headroom_give_the_last_cut_back_and_not_before() {
        let mut l = Ladder::new(6, 2, 33.4);
        assert_eq!(windows(&mut l, 4, 50.0).0, Some(Cut { step: 1, repeat: 0 }));
        assert_eq!(
            windows(&mut l, 9, 20.0),
            (None, None),
            "nine windows are not ten"
        );
        assert_eq!(windows(&mut l, 1, 20.0).1, Some(Cut { step: 1, repeat: 0 }));
        assert_eq!(l.restores(), 1);
        // Not on the heels of the cut it gave back, and not with the frame inside the dead band.
        assert_eq!(windows(&mut l, 9, 20.0).1, None);
        assert_eq!(
            windows(&mut l, 1, 30.0).1,
            None,
            "30 ms is under budget but not under 80 percent"
        );
        assert_eq!(
            windows(&mut l, 10, 20.0).1,
            Some(Cut { step: 0, repeat: 0 })
        );
        assert_eq!(
            windows(&mut l, 30, 20.0).1,
            None,
            "nothing is left to give back"
        );
    }

    #[test]
    fn a_restore_that_misses_twice_is_not_tried_again() {
        let mut l = Ladder::new(6, 2, 33.4);
        windows(&mut l, 2, 50.0);
        for round in 0..2 {
            assert!(windows(&mut l, 10, 20.0).1.is_some(), "restore {round}");
            // Slow again right after: the cut comes back and the restore counts as missed.
            assert!(windows(&mut l, 2, 50.0).0.is_some(), "cut {round}");
        }
        assert_eq!(
            windows(&mut l, 40, 20.0).1,
            None,
            "two misses end restoring"
        );
        assert_eq!(l.restores(), 2);
    }

    #[test]
    fn a_restore_that_holds_does_not_count_as_missed() {
        let mut l = Ladder::new(6, 2, 33.4);
        windows(&mut l, 2, 50.0);
        assert!(windows(&mut l, 10, 20.0).1.is_some());
        // A cut more than 30 windows after the restore is a new slowdown, not a missed restore.
        assert_eq!(windows(&mut l, 35, 25.0), (None, None));
        for round in 0..4 {
            windows(&mut l, 2, 50.0);
            assert!(
                windows(&mut l, 10, 20.0).1.is_some(),
                "restoring is still on in round {round}"
            );
            windows(&mut l, 35, 25.0);
        }
    }

    #[test]
    fn the_repeated_last_step_comes_back_in_reverse_order() {
        let mut l = Ladder::new(2, 2, 33.4);
        let mut taken = Vec::new();
        while !l.spent() {
            if let (Some(c), _) = windows(&mut l, 2, 60.0) {
                taken.push((c.step, c.repeat));
            }
        }
        assert_eq!(taken, [(0, 0), (1, 0), (1, 1), (1, 2)]);
        let mut back = Vec::new();
        while let (_, Some(c)) = windows(&mut l, 10, 5.0) {
            back.push((c.step, c.repeat));
        }
        assert_eq!(back, [(1, 2), (1, 1), (1, 0), (0, 0)]);
        assert_eq!(l.restores(), 4);
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
    fn repeated_scale_cuts_stop_at_fifty_percent() {
        assert_eq!(SCALE_FLOOR_PCT, 50);
        for start in [100, 67] {
            let steps: Vec<u32> = (0..12)
                .map(|repeat| scale_after_cut(start, repeat, SCALE_FLOOR_PCT))
                .collect();
            assert!(steps.windows(2).all(|w| w[1] <= w[0]), "{steps:?}");
            assert_eq!(steps.last(), Some(&50), "{start}: {steps:?}");
            assert!(steps.iter().all(|s| *s >= 50), "{start}: {steps:?}");
        }
        assert_eq!(scale_after_cut(100, 3, SCALE_FLOOR_PCT), 50);
        assert_eq!(scale_after_cut(67, 0, SCALE_FLOOR_PCT), 52);
        assert_eq!(scale_after_cut(67, 1, SCALE_FLOOR_PCT), 50);
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
    fn microseconds_and_float_milliseconds_convert_without_casts() {
        assert!((us_to_ms(1500) - 1.5).abs() < 1e-12);
        assert_eq!(us_to_ms(0), 0.0);
        assert_eq!(whole_ms(1996.4), 1996);
        assert_eq!(whole_ms(0.9), 0);
        for bad in [f64::NAN, f64::INFINITY, -5.0] {
            assert_eq!(whole_ms(bad), 0, "{bad}");
        }
    }

    #[test]
    fn a_given_back_scale_cut_returns_to_the_scale_before_it() {
        assert_eq!(scale_after_restore(100, 0, 60), 100);
        assert_eq!(scale_after_restore(100, 1, 60), 85);
        assert_eq!(scale_after_restore(100, 2, 60), 70);
        assert_eq!(
            restore_line("web", "render_scale", 1, true, 85, 2),
            "tier=web restore=render_scale repeat=1 applied=yes scale=85 restores=2"
        );
        assert_eq!(
            restore_line("web", "shadows", 0, false, 100, 1),
            "tier=web restore=shadows applied=no scale=100 restores=1"
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

    #[test]
    fn only_the_tiers_below_web_name_a_fallback() {
        assert_eq!(fallback_for("limits"), "limits");
        assert_eq!(fallback_for("denylist"), "denylist");
        assert_eq!(fallback_for("probe"), "probe");
        assert_eq!(fallback_for("default"), "none");
        assert_eq!(fallback_for("query"), "none");
        let mut f = facts();
        f.name = "potato".into();
        f.why = "denylist".into();
        assert!(
            f.line()
                .starts_with("tier=potato why=denylist fallback=denylist adapter="),
            "{}",
            f.line()
        );
    }

    #[test]
    fn a_forced_cut_takes_the_next_step_and_holds_restores_for_its_windows() {
        let mut l = Ladder::new(6, 2, 33.4);
        assert_eq!(l.force_cut(300), Some(Cut { step: 0, repeat: 0 }));
        assert_eq!(
            windows(&mut l, 299, 20.0),
            (None, None),
            "the hold lasts 300 windows"
        );
        assert_eq!(windows(&mut l, 1, 20.0).1, Some(Cut { step: 0, repeat: 0 }));
        let mut spent = Ladder::new(1, 0, 33.4);
        assert!(spent.force_cut(300).is_some());
        assert_eq!(spent.force_cut(300), None, "nothing is left to cut");
        assert_eq!(Ladder::new(0, 0, 33.4).force_cut(300), None);
    }

    // ---- QT8: drift ----

    /// `(windows, ms)` runs as one list of window means.
    fn series(parts: &[(usize, f64)]) -> Vec<f64> {
        parts
            .iter()
            .flat_map(|(n, ms)| std::iter::repeat_n(*ms, *n))
            .collect()
    }

    /// The window indexes where the drift check says "cut".
    fn fires(windows: &[f64]) -> Vec<usize> {
        let mut d = Drift::default();
        windows
            .iter()
            .enumerate()
            .filter_map(|(i, ms)| d.observe(*ms).then_some(i))
            .collect()
    }

    #[test]
    fn drift_has_no_reading_until_the_minute_after_the_first_is_full() {
        let mut d = Drift::default();
        for _ in 0..(2 * DRIFT_WINDOWS - 1) {
            assert!(!d.observe(16.0));
            assert_eq!(d.pct(), None);
        }
        assert!(!d.observe(16.0));
        assert_eq!(d.pct(), Some(0.0));
    }

    #[test]
    fn a_steady_ten_minutes_and_a_mild_slide_never_cut() {
        assert_eq!(fires(&series(&[(600, 16.0)])), Vec::<usize>::new());
        // 12.5 percent over the first minute is under the limit.
        let mut d = Drift::default();
        for ms in series(&[(120, 16.0), (480, 18.0)]) {
            assert!(!d.observe(ms));
        }
        assert!((d.pct().expect("a reading") - 12.5).abs() < 1e-9);
    }

    #[test]
    fn a_slide_over_twenty_percent_for_three_minutes_cuts_once_per_three_minutes() {
        // 31 percent over the baseline from the third minute on. The rolling minute passes 20
        // percent when 39 of its 60 windows are slow (index 158); 180 windows later it fires.
        let slide = series(&[(120, 16.0), (480, 21.0)]);
        assert_eq!(fires(&slide), [337, 517]);
    }

    #[test]
    fn a_two_minute_bump_is_not_a_slide() {
        // The rolling mean stays over the limit for 117 windows, under the 180 it needs.
        let bump = series(&[(120, 16.0), (100, 25.0), (380, 16.0)]);
        assert_eq!(fires(&bump), Vec::<usize>::new());
    }

    #[test]
    fn stalled_and_bad_windows_are_not_frame_times() {
        let mut d = Drift::default();
        for _ in 0..(2 * DRIFT_WINDOWS) {
            d.observe(16.0);
        }
        let before = d.pct();
        for bad in [STALL_MS + 1.0, f64::NAN, f64::INFINITY, -1.0, 0.0] {
            assert!(!d.observe(bad), "{bad}");
        }
        assert_eq!(d.pct(), before);
    }

    /// Drive `pace` for `secs` seconds of frames of `frame_ms` each on a fake clock; the ticks
    /// that closed, in order.
    fn run_seconds(pace: &mut Pace, clock: &mut f64, secs: f64, frame_ms: f64) -> Vec<Tick> {
        let end = *clock + secs * 1000.0;
        let mut ticks = Vec::new();
        while *clock < end {
            *clock += frame_ms;
            pace.frame(*clock, frame_ms, 0.0, 0.0, 0.0);
            ticks.extend(pace.tick(*clock, true));
        }
        ticks
    }

    /// Frames of `frame_ms` on the fake clock until one window closes; that window's tick.
    fn next_tick(pace: &mut Pace, clock: &mut f64, frame_ms: f64) -> Tick {
        loop {
            *clock += frame_ms;
            pace.frame(*clock, frame_ms, 0.0, 0.0, 0.0);
            if let Some(tick) = pace.tick(*clock, true) {
                return tick;
            }
        }
    }

    #[test]
    fn a_synthetic_ten_minute_run_cuts_on_the_slide_and_holds_restores_for_five_minutes() {
        let mut pace = Pace::new(Some(Ladder::new(6, 2, 33.4)));
        let mut clock = 0.0;
        // Two minutes at 16 ms set the baseline and fill the rolling minute; none of it is a cut.
        let mut last = None;
        for _ in 0..125 {
            let tick = next_tick(&mut pace, &mut clock, 16.0);
            assert!(tick.step.is_none());
            last = Some(tick);
        }
        assert!(last.is_some_and(|t| t.split.drift_pct.is_some()));
        // Then 21 ms frames: under the 33.4 ms budget, so only the drift check can cut.
        let mut windows = 0;
        let cut = loop {
            windows += 1;
            assert!(windows < 400, "the slide never cut");
            if let Some(step) = next_tick(&mut pace, &mut clock, 21.0).step {
                break step;
            }
        };
        assert_eq!(cut, (Cut { step: 0, repeat: 0 }, false));
        assert!(windows < 400 - 50, "the cut came after {windows} windows");
        // The frame is back to 4 ms at once: plenty of headroom, yet nothing is given back until
        // the 300th window after the cut, and then the cut comes back.
        for k in 1..=300 {
            let step = next_tick(&mut pace, &mut clock, 4.0).step;
            if k < 300 {
                assert_eq!(step, None, "window {k} after the cut");
            } else {
                assert_eq!(step, Some((Cut { step: 0, repeat: 0 }, true)));
            }
        }
        assert_eq!(pace.restores(), 1);
    }

    // ---- QT7: a hidden gap is not a miss ----

    #[test]
    fn a_thirty_second_gap_is_no_slow_frame_and_no_cut_and_it_restarts_the_probe() {
        let mut pace = Pace::new(Some(Ladder::new(6, 2, 33.4)));
        let mut probe = ProbeRun::new(30, 8, 4000.0);
        let mut clock = 0.0;
        let before = run_seconds(&mut pace, &mut clock, 3.0, 16.0);
        assert!(before.iter().all(|t| t.step.is_none()));
        // The probe is part way through its settle frames when the tab hides.
        for _ in 0..PROBE_SETTLE_FRAMES + 5 {
            assert!(probe.frame(10.0, 24).is_none());
        }
        // 30 s without a frame. The page sees the tab come back, drops the frame that spans the
        // gap (it never reaches the window) and restarts the probe.
        clock += 30_000.0;
        pace.resumed();
        probe.restart();
        let after = run_seconds(&mut pace, &mut clock, 20.0, 16.0);
        assert!(!after.is_empty());
        assert!(
            after.iter().all(|t| t.step.is_none()),
            "a gap is not a slow second"
        );
        for t in &after {
            assert!(t.split.frame_ms < 17.0, "{}", t.split.frame_ms);
            assert_eq!(t.split.hidden_skips, 1);
        }
        assert_eq!(pace.hidden_skips(), 1);
        // The probe is back at its settle: the 30 settle frames and the 30 timed ones follow.
        for _ in 0..PROBE_SETTLE_FRAMES + 29 {
            assert!(probe.frame(10.0, 24).is_none());
        }
        assert!(probe.frame(10.0, 24).is_some());
    }

    #[test]
    fn two_gaps_count_two_skips_and_the_count_survives_a_closing_window() {
        let mut pace = Pace::new(None);
        let mut clock = 0.0;
        run_seconds(&mut pace, &mut clock, 2.0, 16.0);
        pace.resumed();
        run_seconds(&mut pace, &mut clock, 2.0, 16.0);
        pace.resumed();
        let last = run_seconds(&mut pace, &mut clock, 2.0, 16.0);
        assert_eq!(last.last().map(|t| t.split.hidden_skips), Some(2));
    }

    #[test]
    fn a_second_that_may_not_cut_is_reported_and_cuts_nothing() {
        let mut pace = Pace::new(Some(Ladder::new(6, 2, 33.4)));
        let mut clock = 0.0;
        for _ in 0..6 {
            let end = clock + 1000.0;
            while clock < end {
                clock += 50.0;
                pace.frame(clock, 50.0, 0.0, 0.0, 0.0);
                if let Some(t) = pace.tick(clock, false) {
                    assert!(t.step.is_none(), "a hidden or capturing page does not cut");
                }
            }
        }
    }

    // ---- QT2: the probe waits for the compiles ----

    /// Feed `ProbeRun` frames of `ms` with a constant number of builds until it finishes.
    fn finish(run: &mut ProbeRun, ms: f64, builds: usize) -> (usize, ProbeDone) {
        for n in 1..=1000 {
            if let Some(done) = run.frame(ms, builds) {
                return (n, done);
            }
        }
        panic!("the probe never finished");
    }

    #[test]
    fn a_probe_settles_for_thirty_frames_then_times_thirty() {
        let mut run = ProbeRun::new(30, 8, 4000.0);
        let (frames, done) = finish(&mut run, 9.0, 24);
        assert_eq!(frames, 60);
        assert_eq!(done.samples.len(), 30);
        assert_eq!((done.late_compile, done.repeated), (0, false));
    }

    #[test]
    fn a_late_compile_repeats_the_probe_once() {
        let mut run = ProbeRun::new(30, 8, 4000.0);
        // 30 settle frames, builds still moving: they are the warm-up's tail, not late compiles.
        for k in 0..PROBE_SETTLE_FRAMES {
            assert!(run
                .frame(9.0, 20 + usize::try_from(k).unwrap_or(0) / 10)
                .is_none());
        }
        // The first window: a build lands in frame 5 and the slow frame it caused is in the samples.
        let mut done = None;
        for k in 0..30 {
            let ms = if k == 5 { 80.0 } else { 9.0 };
            let builds = if k < 5 { 22 } else { 23 };
            done = run.frame(ms, builds);
        }
        assert_eq!(done, None, "a spoiled window is not an answer");
        // The second window is clean (11 ms) and is the one that counts.
        for _ in 0..29 {
            assert!(run.frame(11.0, 23).is_none());
        }
        let done = run.frame(11.0, 23).expect("the repeat finishes");
        assert_eq!(done.samples, vec![11.0; 30]);
        assert_eq!((done.late_compile, done.repeated), (1, true));
    }

    #[test]
    fn a_second_spoiled_window_is_not_run_a_third_time() {
        let mut run = ProbeRun::new(30, 8, 4000.0);
        let mut builds = 24;
        for _ in 0..PROBE_SETTLE_FRAMES {
            run.frame(9.0, builds);
        }
        let mut done = None;
        for window in 0..2 {
            for k in 0..30 {
                if k == 3 {
                    builds += 1;
                }
                done = run.frame(9.0, builds);
            }
            assert_eq!(done.is_some(), window == 1, "window {window}");
        }
        let done = done.expect("done after the second window");
        assert_eq!((done.late_compile, done.repeated), (2, true));
        assert_eq!(done.samples.len(), 30);
    }

    #[test]
    fn a_slow_card_ends_the_probe_on_time_and_a_stalled_tab_gives_no_samples() {
        // 500 ms frames: the 4 s window closes after 8 of them, the settle after 4 (half of 4 s).
        let mut run = ProbeRun::new(30, 8, 4000.0);
        let (frames, done) = finish(&mut run, 500.0, 24);
        assert_eq!((frames, done.samples.len()), (4 + 8, 8));
        // 2 s frames: two fit, which is too few to say anything.
        let mut run = ProbeRun::new(30, 8, 4000.0);
        let (_, done) = finish(&mut run, 2000.0, 24);
        assert!(done.samples.is_empty());
    }

    #[test]
    fn an_abandoned_probe_has_no_samples_and_keeps_the_builds_it_saw() {
        let mut run = ProbeRun::new(30, 8, 4000.0);
        for _ in 0..PROBE_SETTLE_FRAMES {
            run.frame(9.0, 24);
        }
        run.frame(9.0, 24);
        run.frame(9.0, 26);
        let done = run.abandon();
        assert!(done.samples.is_empty());
        assert_eq!((done.late_compile, done.repeated), (2, false));
    }

    #[test]
    fn restarting_a_probe_forgets_its_window_and_settles_again() {
        let mut run = ProbeRun::new(30, 8, 4000.0);
        for _ in 0..PROBE_SETTLE_FRAMES + 10 {
            assert!(run.frame(9.0, 24).is_none());
        }
        run.restart();
        let (frames, done) = finish(&mut run, 7.0, 24);
        assert_eq!(frames, 60);
        assert_eq!(
            done.samples,
            vec![7.0; 30],
            "nothing from before the restart"
        );
    }
}
