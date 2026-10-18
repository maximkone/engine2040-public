//! feel_view: the window's side of the GF2/GF3 feel flow.
//!
//! The shipped `engine_combat` feel flow ([`FeelFlow::v1`]) turns fight events
//! into hit-stop, slow-mo and camera shake. This view feeds it the tick cues
//! the window already holds, steps it once per presented frame, and hands back
//! the three things the window applies: the clock rate for the next frame's
//! elapsed time, a screen-plane camera shake, and a receipt line.
//!
//! Caller: `bin/arena.rs` (`ArenaApp::run_sim_ticks` observes each tick's
//! cues, `tick_and_present` steps, `ticks_owed` scales the wall clock and
//! `present_frame` shakes the camera). Units: permille of real time, meters.
//! Determinism: integer feel state, one float scale at the camera edge.
//! Presentation only: the sim never reads this; slow-mo changes how many fixed
//! ticks a wall second owes, never what a tick computes, so no hash moves.
//! Errors: none; a full slow-mo table is counted in [`FeelStats::refused`].
//! Budget: one fixed-size flow step per frame, the event buffer is reused.

use engine_arena::spear::{EndWhy, SpearEvent};
use engine_audio::FightCue;
use engine_combat::feel_flow::{FeelFlow, Trigger};
use engine_combat::gamefeel::{FeelFrame, STOP_SCALE_PERMILLE};
use engine_combat::FightEvent;
use engine_render::v1::Camera;

/// Real time in permille: the clock rate with no feel effect live.
pub const REAL_TIME_PERMILLE: u32 = 1000;
/// Meters of camera travel per whole shake unit (`shake_milli / 1000`). The
/// shipped rows peak at 2.5 units times a power of 2.0 (a knockdown), so the
/// worst shake is 6 cm, under half the director's 12 cm cap.
pub const SHAKE_M_PER_UNIT: f32 = 0.012;

/// What the flow did over a run, for the exit receipt and the tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FeelStats {
    /// Frames stepped.
    pub steps: u32,
    /// Fight events that raised a feel trigger.
    pub triggers: u32,
    /// Frames whose shake was off center.
    pub shaken: u32,
    /// Largest shake component seen, in milli units.
    pub peak_shake_milli: u32,
    /// Frames the clock was frozen to the hit-stop rate or lower.
    pub stopped: u32,
    /// Frames the clock ran under real time but above the freeze.
    pub slowed: u32,
    /// Frames where a slow-mo effect found no free slot.
    pub refused: u32,
}

/// The feel flow plus the events waiting for its next step.
pub struct FeelView {
    flow: FeelFlow,
    pending: Vec<FightEvent>,
    frame: FeelFrame,
    ftime: u32,
    stats: FeelStats,
}

impl Default for FeelView {
    fn default() -> Self {
        Self::new()
    }
}

/// The fight event a cue was mapped from (the inverse of the game's cue map).
fn event_of(cue: FightCue) -> FightEvent {
    match cue {
        FightCue::Hit {
            attacker,
            victim,
            damage,
        } => FightEvent::Hit {
            attacker,
            victim,
            damage,
        },
        FightCue::Blocked {
            attacker,
            victim,
            chip,
        } => FightEvent::Blocked {
            attacker,
            victim,
            chip,
        },
        FightCue::Parried { attacker, victim } => FightEvent::Parried { attacker, victim },
        FightCue::Dodged { attacker, victim } => FightEvent::Dodged { attacker, victim },
        FightCue::StaminaOut { fighter } => FightEvent::StaminaOut { fighter },
        FightCue::Knockdown { attacker, victim } => FightEvent::Knockdown { attacker, victim },
    }
}

impl FeelView {
    /// The shipped flow, quiet: real-time clock, no shake.
    #[must_use]
    pub fn new() -> Self {
        Self {
            flow: FeelFlow::v1(),
            pending: Vec::with_capacity(32),
            frame: FeelFrame {
                time_permille: REAL_TIME_PERMILLE,
                ..FeelFrame::default()
            },
            ftime: 0,
            stats: FeelStats::default(),
        }
    }

    /// Queue one sim tick's cues for the next [`Self::step`].
    pub fn observe(&mut self, cues: &[FightCue]) {
        self.pending.extend(cues.iter().copied().map(event_of));
    }

    /// Queue one sim tick's spear events: a throw nudges the camera, a spear that sticks hits
    /// like a blow (freeze and shake), a world anchor lands like a parry, a tether that ends
    /// with damage hits again (the slam harder), and a fighter put down is a knockdown.
    pub fn observe_spear(&mut self, events: &[SpearEvent]) {
        for event in events {
            let mapped = match *event {
                SpearEvent::Thrown { thrower, .. } => Some(FightEvent::Blocked {
                    attacker: thrower,
                    victim: thrower,
                    chip: 0,
                }),
                SpearEvent::Hit {
                    thrower,
                    victim: Some(victim),
                    ..
                } => Some(FightEvent::Hit {
                    attacker: thrower,
                    victim,
                    damage: 30,
                }),
                SpearEvent::Hit { thrower, .. } => Some(FightEvent::Parried {
                    attacker: thrower,
                    victim: thrower,
                }),
                SpearEvent::Deflected {
                    thrower,
                    victim,
                    parried: true,
                } => Some(FightEvent::Parried {
                    attacker: thrower,
                    victim,
                }),
                SpearEvent::Deflected {
                    thrower, victim, ..
                } => Some(FightEvent::Blocked {
                    attacker: thrower,
                    victim,
                    chip: 0,
                }),
                SpearEvent::TetherEnd {
                    thrower,
                    victim,
                    why,
                    damage_permille,
                } => match (why, victim) {
                    (EndWhy::Down, Some(victim)) => Some(FightEvent::Knockdown {
                        attacker: thrower,
                        victim,
                    }),
                    (_, Some(victim)) if damage_permille > 0 => Some(FightEvent::Hit {
                        attacker: thrower,
                        victim,
                        damage: damage_permille,
                    }),
                    _ => None,
                },
                SpearEvent::Missed { .. } | SpearEvent::TetherPull { .. } => None,
            };
            self.pending.extend(mapped);
        }
    }

    /// One presented frame: fire the queued events through the flow and keep
    /// its frame. The queue is emptied.
    pub fn step(&mut self) -> FeelFrame {
        self.stats.triggers += self
            .pending
            .iter()
            .filter(|event| Trigger::of(**event).is_some())
            .count()
            .try_into()
            .unwrap_or(u32::MAX);
        let (frame, status) = self.flow.step(&self.pending, self.ftime);
        self.pending.clear();
        self.ftime = self.ftime.wrapping_add(1);
        self.frame = frame;
        let stats = &mut self.stats;
        stats.steps += 1;
        stats.refused += u32::from(status.is_err());
        let (sx, sy) = frame.shake_milli;
        stats.shaken += u32::from((sx, sy) != (0, 0));
        stats.peak_shake_milli = stats
            .peak_shake_milli
            .max(sx.unsigned_abs().max(sy.unsigned_abs()));
        if frame.time_permille <= STOP_SCALE_PERMILLE {
            stats.stopped += 1;
        } else if frame.time_permille < REAL_TIME_PERMILLE {
            stats.slowed += 1;
        }
        frame
    }

    /// The last stepped frame.
    #[must_use]
    pub fn frame(&self) -> FeelFrame {
        self.frame
    }

    /// What the flow has done so far.
    #[must_use]
    pub fn stats(&self) -> FeelStats {
        self.stats
    }

    /// Wall milliseconds scaled by the clock rate of the last frame: hit-stop
    /// and slow-mo owe the fixed-tick accumulator fewer ticks.
    #[must_use]
    pub fn scale_elapsed_ms(&self, elapsed_ms: f64) -> f64 {
        elapsed_ms * f64::from(self.frame.time_permille) / f64::from(REAL_TIME_PERMILLE)
    }

    /// Shake the camera by the last frame's offset, in its own screen plane:
    /// eye and target move together so the view direction holds. A degenerate
    /// camera (view parallel to up) stays untouched.
    pub fn shake_camera(&self, camera: &mut Camera) {
        let (sx, sy) = self.frame.shake_milli;
        if (sx, sy) == (0, 0) {
            return;
        }
        let forward = (camera.target - camera.eye).normalize_or_zero();
        let right = forward.cross(camera.up).normalize_or_zero();
        if right.length_squared() == 0.0 {
            return;
        }
        let up = right.cross(forward);
        let per_milli = SHAKE_M_PER_UNIT / 1000.0;
        let milli = |v: i32| f32::from(i16::try_from(v).unwrap_or(0)) * per_milli;
        let offset = right * milli(sx) + up * milli(sy);
        camera.eye += offset;
        camera.target += offset;
    }

    /// One receipt line for the exit report.
    #[must_use]
    pub fn line(&self) -> String {
        let s = self.stats;
        format!(
            "arena feel: steps={} triggers={} shaken={} peak_shake_milli={} stopped={} slowed={} refused={}",
            s.steps, s.triggers, s.shaken, s.peak_shake_milli, s.stopped, s.slowed, s.refused
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit() -> FightCue {
        FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 120,
        }
    }

    #[test]
    fn quiet_view_runs_real_time_and_leaves_the_camera() {
        let mut view = FeelView::new();
        view.step();
        assert_eq!(view.scale_elapsed_ms(16.0), 16.0);
        let base = Camera::new([0.0, 2.0, 5.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0], 0.9);
        let mut shaken = base.clone();
        view.shake_camera(&mut shaken);
        assert_eq!(shaken, base);
    }

    #[test]
    fn a_hit_freezes_the_clock_four_frames_and_shakes_the_camera() {
        let mut view = FeelView::new();
        view.observe(&[hit()]);
        view.step();
        assert!(view.scale_elapsed_ms(1000.0) <= 100.0, "hit-stop rate");
        let base = Camera::new([0.0, 2.0, 5.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0], 0.9);
        let mut shaken = base.clone();
        view.shake_camera(&mut shaken);
        assert_ne!(shaken.eye, base.eye, "the eye moved");
        let (was, now) = (base.target - base.eye, shaken.target - shaken.eye);
        assert!(
            (now - was).length() < 1.0e-5,
            "eye and target move together"
        );
        assert!(
            (shaken.eye - base.eye).length() <= 2.0 * SHAKE_M_PER_UNIT * 2.5 * 2.0,
            "shake stays small"
        );
        for _ in 0..3 {
            view.step();
        }
        assert_eq!(view.stats().stopped, 4, "four frozen frames");
        view.step();
        assert_eq!(view.scale_elapsed_ms(16.0), 16.0, "time resumes");
        assert_eq!(view.stats().triggers, 1);
    }

    #[test]
    fn a_knockdown_slows_time_without_freezing_it() {
        let mut view = FeelView::new();
        view.observe(&[FightCue::Knockdown {
            attacker: 0,
            victim: 1,
        }]);
        for _ in 0..10 {
            view.step();
        }
        let stats = view.stats();
        assert!(stats.slowed > 0 && stats.stopped == 0, "{stats:?}");
        assert!(view.scale_elapsed_ms(1000.0) < 1000.0);
    }

    #[test]
    fn cues_that_raise_no_trigger_stay_quiet() {
        let mut view = FeelView::new();
        view.observe(&[
            FightCue::Dodged {
                attacker: 0,
                victim: 1,
            },
            FightCue::StaminaOut { fighter: 0 },
        ]);
        view.step();
        assert_eq!(view.stats().triggers, 0);
        assert_eq!(view.stats().shaken, 0);
    }
}
