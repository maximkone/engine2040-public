//! feel_view: the window's side of the GF2/GF3 feel flow, now driven by the
//! authored hit sequences of [`crate::impact_seq`].
//!
//! The engine's feel state ([`GameFeel`]: shake, nudge, slow motion) takes the
//! tick cues the window already holds, one authored [`Sequence`] per impact
//! class and weapon (`content/design/impact.rows`) fires every channel of a
//! blow at once, and the view hands back the three things the window applies:
//! the clock rate for the next frame's elapsed time, a screen-plane camera
//! shake with a kick along the blow, and a receipt line. Hit-stop is a
//! composable [`StopStack`] beside the slow-mo, not one timer the next blow
//! resets: overlapping stops last the longest and run the lowest, and when
//! they end the clock returns to the rate slow motion left it at.
//!
//! Caller: `bin/arena.rs` (`ArenaApp::run_sim_ticks` observes each tick's
//! cues, `tick_and_present` steps, `ticks_owed` scales the wall clock and
//! `present_frame` shakes the camera). Units: permille of real time, meters.
//! The view runs on a fixed 60 Hz clock whatever the monitor refreshes at
//! ([`FeelView::step_ms`]): a four-frame stop lasts 67 ms at 60 or 144 Hz.
//! Determinism: integer feel state, one float scale at the camera edge.
//! Presentation only: the sim never reads this; slow-mo changes how many fixed
//! ticks a wall second owes, never what a tick computes, so no hash moves.
//! Errors: none; a full slow-mo table is counted in [`FeelStats::refused`].
//! Budget: one fixed-size step per 60th second, the event buffer is reused.

use engine_arena::spear::{EndWhy, SpearEvent};
use engine_audio::FightCue;
use engine_combat::feel_flow::Trigger;
use engine_combat::gamefeel::{FeelFrame, GameFeel, STOP_SCALE_PERMILLE};
use engine_combat::FightEvent;
use engine_render::v1::Camera;

use crate::hitstop::freeze_frames;
use crate::impact_seq::{ImpactClass, ImpactTable, Sequence, StopStack};
use crate::FighterView;

/// Real time in permille: the clock rate with no feel effect live.
pub const REAL_TIME_PERMILLE: u32 = 1000;
/// Meters of camera travel per whole shake unit (`shake_milli / 1000`). The
/// shipped rows peak at 2.5 units times a power of 2.0 (a knockdown), so the
/// worst shake is 6 cm, under half the director's 12 cm cap.
pub const SHAKE_M_PER_UNIT: f32 = 0.012;
/// One feel step in wall milliseconds: the feel flow's own 60 Hz.
pub const FEEL_STEP_MS: f64 = 1000.0 / 60.0;
/// Most feel steps one presented frame may catch up, so a stalled frame never
/// replays a second of shake.
const MAX_CATCHUP_STEPS: u32 = 6;
/// Share of a step's worth of slack that rounds a frame up to a step, so a
/// 60 Hz monitor with a jittery vsync steps once per frame, not 0, 2, 1.
const STEP_SLACK: f64 = 0.25;
/// Slow-mo id the knockdown row uses (a repeated id extends, never stacks).
const BLOW_SLOWMO_ID: u8 = 1;

/// What the flow did over a run, for the exit receipt and the tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FeelStats {
    /// Feel steps taken (the fixed 60 Hz clock).
    pub steps: u32,
    /// Fight events that raised a feel trigger.
    pub triggers: u32,
    /// Steps whose shake was off center.
    pub shaken: u32,
    /// Largest shake component seen, in milli units.
    pub peak_shake_milli: u32,
    /// Steps the clock was frozen to the hit-stop rate or lower.
    pub stopped: u32,
    /// Steps the clock ran under real time but above the freeze.
    pub slowed: u32,
    /// Slow-mo effects that found no free slot.
    pub refused: u32,
    /// Blows that kicked the camera along their axis.
    pub kicked: u32,
    /// Largest camera kick fired, in millimeters.
    pub peak_kick_mm: u32,
}

/// One blow waiting for the next step: the fight event, the sim's hold for it
/// when known, and the blow's axis on the sand when the window gave positions.
#[derive(Clone, Copy, Debug)]
struct Pending {
    event: FightEvent,
    hold: Option<u32>,
    axis: Option<[f32; 2]>,
    /// The table's handle for the attacker's weapon, when it has override rows.
    weapon: Option<u8>,
}

/// The camera kick in flight: pushed along the blow, easing back.
#[derive(Clone, Copy, Debug, Default)]
struct Kick {
    axis: [f32; 2],
    amp_m: f32,
    total: u32,
    left: u32,
}

impl Kick {
    /// Offset along the blow in meters now: full at the blow, easing out
    /// quadratically so the camera settles without a snap.
    fn offset_m(&self) -> f32 {
        if self.left == 0 || self.total == 0 {
            return 0.0;
        }
        let left = f32::from(u16::try_from(self.left).unwrap_or(u16::MAX));
        let total = f32::from(u16::try_from(self.total).unwrap_or(u16::MAX));
        let ease = left / total;
        self.amp_m * ease * ease
    }
}

/// The feel state plus the events waiting for its next step.
pub struct FeelView {
    feel: GameFeel,
    stops: StopStack,
    table: &'static ImpactTable,
    pending: Vec<Pending>,
    /// The sim's hold count after the last tick it was read at; zero is none.
    last_hold: u32,
    kick: Kick,
    frame: FeelFrame,
    ftime: u32,
    /// Wall time owed to the feel clock, in steps (can dip under zero by the
    /// slack).
    owed_steps: f64,
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

/// The impact class an event plays, or `None` for the quiet ones (a dodge, a
/// stamina-out). A clean hit is light or heavy by the sim's hold.
fn class_of(event: FightEvent, hold: Option<u32>) -> Option<ImpactClass> {
    match event {
        FightEvent::Hit { .. } => Some(ImpactClass::of_hit(hold.unwrap_or(0))),
        FightEvent::Parried { .. } => Some(ImpactClass::Parry),
        FightEvent::Blocked { .. } => Some(ImpactClass::Block),
        FightEvent::Knockdown { .. } => Some(ImpactClass::Knockdown),
        FightEvent::Dodged { .. } | FightEvent::StaminaOut { .. } => None,
    }
}

/// The slot whose weapon struck: the attacker of a blow.
fn attacker_of(event: FightEvent) -> Option<u8> {
    match event {
        FightEvent::Hit { attacker, .. }
        | FightEvent::Blocked { attacker, .. }
        | FightEvent::Parried { attacker, .. }
        | FightEvent::Knockdown { attacker, .. } => Some(attacker),
        FightEvent::Dodged { .. } | FightEvent::StaminaOut { .. } => None,
    }
}

/// Unit blow axis on the sand, attacker toward victim, from the tick's views.
/// `None` when either fighter is missing or they stand on one spot.
fn blow_axis(views: &[FighterView], attacker: u8, victim: u8) -> Option<[f32; 2]> {
    let find = |slot: u8| views.iter().find(|view| view.slot == slot);
    let (from, to) = (find(attacker)?, find(victim)?);
    let (dx, dz) = (to.pos_m[0] - from.pos_m[0], to.pos_m[2] - from.pos_m[2]);
    let len = dx.hypot(dz);
    (len.is_finite() && len > 1.0e-4).then(|| [dx / len, dz / len])
}

impl FeelView {
    /// The shipped table, quiet: real-time clock, no shake.
    #[must_use]
    pub fn new() -> Self {
        Self::with_table(ImpactTable::shipped())
    }

    /// A view over another table (tests, mods).
    #[must_use]
    pub fn with_table(table: &'static ImpactTable) -> Self {
        Self {
            feel: GameFeel::default(),
            stops: StopStack::new(),
            table,
            pending: Vec::with_capacity(32),
            last_hold: 0,
            kick: Kick::default(),
            frame: FeelFrame {
                time_permille: REAL_TIME_PERMILLE,
                ..FeelFrame::default()
            },
            ftime: 0,
            owed_steps: 0.0,
            stats: FeelStats::default(),
        }
    }

    /// Queue one sim tick's cues for the next step.
    pub fn observe(&mut self, cues: &[FightCue]) {
        self.observe_with(cues, &[], &[]);
    }

    /// [`Self::observe`] with the tick's views, so each blow knows its axis on
    /// the sand and the camera kicks along it, and with each slot's weapon
    /// (`weapons.rows` ids in slot order), so a blow plays its weapon's row. A
    /// slot with `None`, or a weapon the table has no row for, plays the class
    /// default.
    pub fn observe_with(
        &mut self,
        cues: &[FightCue],
        views: &[FighterView],
        weapons: &[Option<&str>],
    ) {
        for &cue in cues {
            let event = event_of(cue);
            let weapon = attacker_of(event)
                .and_then(|slot| weapons.get(usize::from(slot)).copied().flatten())
                .and_then(|name| self.table.weapon_id(name));
            let axis = match event {
                FightEvent::Hit {
                    attacker, victim, ..
                }
                | FightEvent::Parried { attacker, victim }
                | FightEvent::Knockdown { attacker, victim }
                | FightEvent::Blocked {
                    attacker, victim, ..
                } => blow_axis(views, attacker, victim),
                FightEvent::Dodged { .. } | FightEvent::StaminaOut { .. } => None,
            };
            self.pending.push(Pending {
                event,
                hold: None,
                axis,
                weapon,
            });
        }
    }

    /// The live fight's hit-stop count for the tick just queued (FEEL3, read from
    /// `App::hitstop_remaining` after the tick ran): the blows queued since the
    /// last count take it as their hold, so a row that follows the sim stops the
    /// clock this many frames, the same as the sim freezes, and a heavy blow is
    /// told from a light one. A carry spear stick never freezes the sim (no
    /// count), so a zero count keeps the four-frame stick; a yank's catch (row
    /// SG9) freezes it five ticks, and the clock holds that long.
    ///
    /// QL7 caps the presentation freeze at
    /// [`FREEZE_MAX_FRAMES`](crate::hitstop::FREEZE_MAX_FRAMES): a parry's
    /// eight sim ticks stop the clock six frames (100 ms). The sim's own count
    /// is only read, never rewritten.
    ///
    /// The hold's receipt line is [`hold_line`](crate::hitstop::hold_line) over
    /// [`Self::last_hold`]; a caller prints it on the frame a hold starts (a
    /// zero hold before it), never per tick, so no print lives in the library.
    pub fn observe_hitstop(&mut self, remaining: u32) {
        let frames = freeze_frames(remaining);
        self.last_hold = frames;
        if frames == 0 {
            return;
        }
        for pending in &mut self.pending {
            if pending.hold.is_none() {
                pending.hold = Some(frames);
            }
        }
    }

    /// The capped freeze of the last count read, in frames at 60 Hz: `0` when
    /// no hold is live, at most [`FREEZE_MAX_FRAMES`](crate::hitstop::FREEZE_MAX_FRAMES).
    #[must_use]
    pub const fn last_hold(&self) -> u32 {
        self.last_hold
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
            if let Some(event) = mapped {
                // A spear stick has no sim count of its own; it takes the one
                // read for this tick (a yank's catch holds five ticks).
                self.pending.push(Pending {
                    event,
                    hold: (self.last_hold > 0).then_some(self.last_hold),
                    axis: None,
                    weapon: None,
                });
            }
        }
    }

    /// Fire one blow's authored sequence on every channel.
    fn fire(&mut self, blow: Pending) {
        self.stats.triggers += u32::from(Trigger::of(blow.event).is_some());
        let hold = blow
            .hold
            .or_else(|| (self.last_hold > 0).then_some(self.last_hold));
        let Some(class) = class_of(blow.event, hold) else {
            return;
        };
        let seq: Sequence = self.table.sequence_of(class, blow.weapon);
        if seq.shake_ticks > 0 && seq.shake_power_permille > 0 {
            self.feel
                .shake
                .start(seq.shake_ticks, seq.shake_power_permille);
        }
        if seq.bump_milli > 0 {
            self.feel
                .bump
                .add_angle(0, i32::try_from(seq.bump_milli).unwrap_or(i32::MAX));
        }
        if seq.stops() {
            self.stops.push(seq.stop_for(hold), seq.stop_rate_permille);
        }
        if seq.slowmo_ticks > 0
            && self
                .feel
                .time
                .add_slow_mo(BLOW_SLOWMO_ID, seq.slowmo_ticks, seq.slowmo_permille)
                .is_err()
        {
            self.stats.refused += 1;
        }
        if seq.kick_mm > 0 && seq.kick_ticks > 0 {
            if let Some(axis) = blow.axis {
                self.kick = Kick {
                    axis,
                    amp_m: seq.kick_m(),
                    total: seq.kick_ticks,
                    left: seq.kick_ticks,
                };
                self.stats.kicked += 1;
                self.stats.peak_kick_mm = self.stats.peak_kick_mm.max(seq.kick_mm);
            }
        }
    }

    /// One feel step (a 60th of a second): fire the queued blows, step the
    /// state, ease the kick, age the stops. The queue is emptied.
    fn step_once(&mut self) -> FeelFrame {
        let mut blows = std::mem::take(&mut self.pending);
        for blow in blows.drain(..) {
            self.fire(blow);
        }
        self.pending = blows;
        let mut frame = self.feel.step(self.ftime);
        // The state's own stop timer is never started: the stack owns every
        // stop, so the rate here is the slow motion alone, and the stack rides
        // on top of it and hands the slow-mo rate back when it ends.
        let stop = self.stops.rate_permille();
        frame.time_permille = frame.time_permille.saturating_mul(stop) / REAL_TIME_PERMILLE;
        self.stops.advance();
        self.kick.left = self.kick.left.saturating_sub(1);
        self.ftime = self.ftime.wrapping_add(1);
        let stats = &mut self.stats;
        stats.steps += 1;
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
        self.frame = frame;
        frame
    }

    /// One presented frame as exactly one feel step: the queued blows fire and
    /// the state moves a 60th of a second. The tests and fixed-step callers use
    /// it; the window uses [`Self::step_ms`], which keeps the 60 Hz clock on any
    /// monitor.
    pub fn step(&mut self) -> FeelFrame {
        self.step_once()
    }

    /// One presented frame of `elapsed_ms` wall time: the feel clock owes
    /// `elapsed_ms` of its fixed 60 Hz, takes that many steps (none on a frame
    /// shorter than a step, several on a long one, never more than a handful),
    /// and returns the frame it ends on. A frame that carries a queued blow
    /// always steps at least once, so the stop and the shake begin on the frame
    /// of the blow, not the next one. A 144 Hz monitor sees a four-frame stop
    /// last four feel steps, 67 ms, as a 60 Hz one does.
    pub fn step_ms(&mut self, elapsed_ms: f64) -> FeelFrame {
        let elapsed = if elapsed_ms.is_finite() {
            elapsed_ms.max(0.0)
        } else {
            0.0
        };
        self.owed_steps += elapsed / FEEL_STEP_MS;
        let mut steps = 0;
        while steps < MAX_CATCHUP_STEPS && self.owed_steps + STEP_SLACK >= 1.0 {
            self.owed_steps -= 1.0;
            steps += 1;
        }
        if steps == MAX_CATCHUP_STEPS {
            // A stall: forget the rest of the debt, do not replay it.
            self.owed_steps = 0.0;
        } else if steps == 0 && !self.pending.is_empty() {
            // A blow steps on its own frame; the step it borrows is repaid by
            // the frames after it.
            self.owed_steps -= 1.0;
            steps = 1;
        }
        for _ in 0..steps {
            self.step_once();
        }
        self.frame
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
    /// eye and target move together so the view direction holds. The blow's
    /// kick pushes both along the blow axis on the sand the same way. A
    /// degenerate camera (view parallel to up) stays untouched by the shake.
    pub fn shake_camera(&self, camera: &mut Camera) {
        let (sx, sy) = self.frame.shake_milli;
        if (sx, sy) != (0, 0) {
            let forward = (camera.target - camera.eye).normalize_or_zero();
            let right = forward.cross(camera.up).normalize_or_zero();
            if right.length_squared() != 0.0 {
                let up = right.cross(forward);
                let per_milli = SHAKE_M_PER_UNIT / 1000.0;
                let milli = |v: i32| f32::from(i16::try_from(v).unwrap_or(0)) * per_milli;
                let offset = right * milli(sx) + up * milli(sy);
                camera.eye += offset;
                camera.target += offset;
            }
        }
        let kick = self.kick.offset_m();
        if kick > 0.0 {
            let (dx, dz) = (self.kick.axis[0] * kick, self.kick.axis[1] * kick);
            camera.eye = [camera.eye.x + dx, camera.eye.y, camera.eye.z + dz].into();
            camera.target = [camera.target.x + dx, camera.target.y, camera.target.z + dz].into();
        }
    }

    /// One receipt line for the exit report.
    #[must_use]
    pub fn line(&self) -> String {
        let s = self.stats;
        format!(
            "arena feel: steps={} triggers={} shaken={} peak_shake_milli={} stopped={} slowed={} refused={} kicked={} peak_kick_mm={}",
            s.steps,
            s.triggers,
            s.shaken,
            s.peak_shake_milli,
            s.stopped,
            s.slowed,
            s.refused,
            s.kicked,
            s.peak_kick_mm
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

    fn views(distance: f32) -> Vec<FighterView> {
        vec![
            FighterView {
                slot: 0,
                pos_m: [0.0, 0.0, 0.0],
                yaw: 0.0,
            },
            FighterView {
                slot: 1,
                pos_m: [0.0, 0.0, distance],
                yaw: 0.0,
            },
        ]
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
    fn the_live_count_sets_the_freeze_frames() {
        // FEEL3 + QL7: a light blow freezes the clock 3 frames and a heavy one
        // 6; a parry's 8 sim ticks are capped at 6 (100 ms), and the clock runs
        // free on the frame after the hold.
        for (count, want) in [(3_u32, 3_u32), (6, 6), (8, 6)] {
            let mut view = FeelView::new();
            view.observe(&[hit()]);
            view.observe_hitstop(count);
            for _ in 0..count {
                view.step();
            }
            assert_eq!(view.stats().stopped, want, "count {count} frozen {want}");
            view.step();
            assert_eq!(view.stats().stopped, want, "count {count} then free");
        }
    }

    #[test]
    fn observe_hitstop_caps_the_hold_at_six_frames() {
        // QL7: the presentation freeze stops at six frames however long the sim
        // freezes, and every row queued for the tick takes the capped count.
        let mut view = FeelView::new();
        view.observe(&[hit()]);
        view.observe(&[hit()]);
        view.observe_hitstop(8);
        assert_eq!(view.last_hold(), 6, "a parry's 8 frames stop at 6");
        assert_eq!(view.pending.len(), 2, "two blows wait for the step");
        for pending in &view.pending {
            assert_eq!(pending.hold, Some(6), "every queued row holds the cap");
        }
        for _ in 0..6 {
            view.step();
        }
        assert_eq!(view.stops.frames_left(), 0, "six frames, none left");
        assert_eq!(view.stats().stopped, 6, "100 ms of freeze, not 133");
        view.step();
        assert_eq!(view.stats().stopped, 6, "the clock runs free after the cap");
        assert_eq!(
            crate::hitstop::hold_line(view.last_hold()),
            "feel: hitstop hold=6 ms=100",
            "the receipt names the capped freeze"
        );

        // A light blow sits under the cap and keeps its own count.
        let mut light = FeelView::new();
        light.observe(&[hit()]);
        light.observe_hitstop(3);
        assert_eq!(light.last_hold(), 3, "a light blow keeps 3 frames");
        assert_eq!(light.pending[0].hold, Some(3));
        light.observe_hitstop(0);
        assert_eq!(light.last_hold(), 0, "a zero count is no hold");
    }

    #[test]
    fn a_zero_live_count_keeps_the_four_frame_stop() {
        // A spear stick never freezes the sim, so it has no count and keeps the
        // four-frame stick it always had.
        let mut view = FeelView::new();
        view.observe_hitstop(0);
        view.observe(&[hit()]);
        for _ in 0..4 {
            view.step();
        }
        assert_eq!(view.stats().stopped, 4);
        view.step();
        assert_eq!(view.stats().stopped, 4, "time resumes after four");
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

    #[test]
    fn each_blow_keeps_its_own_hold_when_two_ticks_share_a_frame() {
        // A light blow (hold 3) on one tick and a heavy one (hold 6) on the
        // next, both before one step. The old view kept only the last count
        // for both; each blow now plays the class its own tick's hold names.
        let mut view = FeelView::new();
        view.observe(&[hit()]);
        view.observe_hitstop(3);
        view.observe(&[hit()]);
        view.observe_hitstop(6);
        view.step();
        assert_eq!(view.stops.frames_left(), 5, "the heavy blow's 6, one spent");
        assert_eq!(view.stops.rate_permille(), 60, "the heavy row's rate");
        assert_eq!(view.stats().triggers, 2);
    }

    #[test]
    fn a_second_blow_inside_the_stop_extends_it_and_never_restarts_the_count() {
        let mut view = FeelView::new();
        view.observe(&[hit()]);
        view.observe_hitstop(3);
        view.step();
        view.step();
        // One frame of the first stop is left when the second lands.
        view.observe(&[hit()]);
        view.observe_hitstop(3);
        let mut frozen = 0;
        for _ in 0..6 {
            view.step();
            frozen += u32::from(view.frame().time_permille <= STOP_SCALE_PERMILLE);
        }
        assert_eq!(
            frozen, 3,
            "the second blow's three frames, no leftover on top"
        );
    }

    #[test]
    fn a_stop_restores_the_slow_motion_it_interrupted() {
        // Knockdown slow motion (a third speed) runs; a hit stops the clock for
        // three frames on top; the clock goes back to the slow rate, not to real
        // time (Wallop Lite: the previous scale is restored).
        let mut view = FeelView::new();
        view.observe(&[FightCue::Knockdown {
            attacker: 0,
            victim: 1,
        }]);
        for _ in 0..12 {
            view.step();
        }
        let slow = view.frame().time_permille;
        assert!(
            slow < 1000 && slow > STOP_SCALE_PERMILLE,
            "slow motion: {slow}"
        );
        view.observe(&[hit()]);
        view.observe_hitstop(3);
        view.step();
        let stopped = view.frame().time_permille;
        assert!(stopped < slow / 4, "{stopped} stopped under {slow}");
        for _ in 0..3 {
            view.step();
        }
        let after = view.frame().time_permille;
        assert!(
            after > STOP_SCALE_PERMILLE && after < 1000,
            "back on the slow-mo rate, not real time: {after}"
        );
    }

    #[test]
    fn a_weapon_row_changes_the_blow() {
        // The dagger's blow and the club's differ in shake and kick (the rows
        // never soften with mass); a slot with no weapon plays the default.
        let blow = |weapon: Option<&str>| {
            let mut view = FeelView::new();
            view.observe_with(&[hit()], &views(2.0), &[weapon, None]);
            view.step();
            (view.stats().peak_shake_milli, view.stats().peak_kick_mm)
        };
        let (dagger, bare, club) = (blow(Some("pugio")), blow(None), blow(Some("clava")));
        assert!(
            dagger.1 < bare.1 && bare.1 < club.1,
            "{dagger:?} {bare:?} {club:?}"
        );
        assert!(dagger.0 < club.0, "{dagger:?} {club:?}");
        assert_eq!(
            blow(Some("no-such-weapon")),
            bare,
            "an unknown weapon plays the default"
        );
    }

    #[test]
    fn the_kick_pushes_along_the_blow_and_eases_back() {
        let mut view = FeelView::new();
        view.observe_with(&[hit()], &views(2.0), &[]);
        view.step();
        let base = Camera::new([0.0, 1.5, -4.0], [0.0, 1.0, 1.0], [0.0, 1.0, 0.0], 0.9);
        let mut kicked = base.clone();
        view.shake_camera(&mut kicked);
        // The blow runs attacker (z = 0) toward victim (z = 2): +z.
        let kick_z = kicked.eye.z - base.eye.z;
        assert!(
            kick_z > 0.0 && kick_z <= 0.02 + 0.03,
            "kick {kick_z} m along the blow"
        );
        assert!(
            ((kicked.eye.y - base.eye.y) - (kicked.target.y - base.target.y)).abs() < 1.0e-6,
            "eye and target rise together"
        );
        // It eases back to nothing inside the row's ticks.
        let mut last = kick_z;
        for _ in 0..12 {
            view.step();
            let mut now = base.clone();
            view.shake_camera(&mut now);
            let z = now.eye.z - base.eye.z;
            assert!(z.abs() <= last.abs() + 0.05, "the kick never grows");
            last = z;
        }
        assert!(last.abs() < 0.03, "settled: {last}");
        assert_eq!(view.stats().kicked, 1);
    }

    #[test]
    fn a_blow_without_positions_kicks_nothing() {
        let mut view = FeelView::new();
        view.observe(&[hit()]);
        view.step();
        assert_eq!(view.stats().kicked, 0);
        // Fighters on one spot have no axis either.
        view.observe_with(&[hit()], &views(0.0), &[]);
        view.step();
        assert_eq!(view.stats().kicked, 0);
    }

    #[test]
    fn the_feel_clock_is_60_hz_on_any_monitor() {
        // A hit's four-frame stop lasts the same wall time at 60, 144 and 30
        // Hz: the clock owes steps by elapsed time, not by frames presented.
        for hz in [30.0_f64, 60.0, 120.0, 144.0, 240.0] {
            let dt = 1000.0 / hz;
            let mut view = FeelView::new();
            view.observe(&[hit()]);
            view.observe_hitstop(4);
            let mut stopped_ms = 0.0;
            for _ in 0..(hz as u32) {
                let frame = view.step_ms(dt);
                if frame.time_permille <= STOP_SCALE_PERMILLE {
                    stopped_ms += dt;
                }
            }
            let want = 4.0 * FEEL_STEP_MS;
            assert!(
                (stopped_ms - want).abs() <= dt.max(FEEL_STEP_MS) + 1e-6,
                "{hz} Hz: stopped {stopped_ms:.1} ms, want about {want:.1}"
            );
            let steps = view.stats().steps;
            assert!(
                (59..=61).contains(&steps),
                "{hz} Hz: {steps} feel steps in one second"
            );
        }
    }

    #[test]
    fn a_blow_steps_the_feel_on_its_own_frame_even_between_steps() {
        let mut view = FeelView::new();
        // A 240 Hz frame is a quarter step: nothing owed, nothing queued.
        view.step_ms(1000.0 / 240.0);
        assert_eq!(view.stats().steps, 0);
        view.observe(&[hit()]);
        let frame = view.step_ms(1000.0 / 240.0);
        assert_eq!(view.stats().steps, 1, "the blow forces its step");
        assert!(
            frame.time_permille <= STOP_SCALE_PERMILLE,
            "the stop begins at once"
        );
    }

    #[test]
    fn a_stalled_frame_catches_up_a_handful_of_steps_not_a_second() {
        let mut view = FeelView::new();
        view.step_ms(1000.0);
        assert_eq!(view.stats().steps, MAX_CATCHUP_STEPS);
        view.step_ms(f64::NAN);
        view.step_ms(-5.0);
        assert_eq!(
            view.stats().steps,
            MAX_CATCHUP_STEPS,
            "bad time owes nothing"
        );
    }

    #[test]
    fn the_receipt_line_names_the_kick() {
        let mut view = FeelView::new();
        view.observe_with(&[hit()], &views(2.0), &[]);
        view.step();
        let line = view.line();
        assert!(line.starts_with("arena feel: steps=1 triggers=1"), "{line}");
        assert!(line.contains("kicked=1 peak_kick_mm=20"), "{line}");
    }
}
