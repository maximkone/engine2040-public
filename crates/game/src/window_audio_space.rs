//! window_audio_space: what the `arena` window tells the audio feed each tick
//! so the room has a place, a floor and a mood (EG21w).
//!
//! Consumer: [`crate::windowed::feed_tick_audio`], called once per fixed tick
//! by `arena_frame::run_sim_ticks`. Data owner: the game owns the camera pose,
//! the fighter positions and the tick outcome; `engine-audio` owns the feed,
//! the listener maths and the cue bank. This module only maps one onto the
//! other. Units: centimeters `[east, up, south]` on the feed (the game's
//! meters times 100), degrees clockwise from north for the listener, `0.0..=1.0`
//! for gains.
//!
//! What goes out each tick:
//! - the listener at the camera (position and yaw), so a hit on the left of
//!   the picture sounds on the left and a far one is quieter;
//! - one named play per fight event, placed where the blow landed (victim for
//!   impacts, attacker for a dodged swing) from the fighter views;
//! - a footstep per stride of each walking fighter, on the floor's
//!   [`Surface`] cue;
//! - the tension level, on the ticks it changes (rises at once, falls after a
//!   hold, see [`TensionTracker`]), and a kill stinger on a knockdown;
//! - the crowd bed, started once and then ridden by `outcome.bed_gain` every
//!   tick it moves.
//!
//! Errors: none propagate. A full ring is counted by the feed
//! (`dropped_full`), a cue the bank lacks is counted by the feed
//! (`unknown_cue`) and again in [`SpaceStats`], never skipped silently.
//! Determinism: same outcomes, views and camera give the same commands.
//! Presentation only: nothing here reads or writes the simulation, so the
//! golden `d57b5c8ae3096614` cannot move. Budget: a handful of feed writes
//! per tick, no heap.

use engine_audio::combat_cues::{cue_for_event, footstep, Surface};
use engine_audio::{cue_names, FeedError, FeedTx, FightCue, Tension, TensionTracker};
use engine_spatial::to_cm;

use crate::{FighterView, TickOutcome};

/// Slots tracked for footsteps (the roster cap).
pub const MAX_SLOTS: usize = 16;
/// Meters a fighter travels between footsteps.
pub const STRIDE_M: f32 = 0.8;
/// A move longer than this in one tick is a teleport (respawn, rematch), not
/// a stride: the stride meter restarts instead of firing a step.
const TELEPORT_M: f32 = 1.5;
/// Height of a blow above the feet in cm: chest, so the 3D distance to the
/// camera counts the body, not the sand.
pub const CHEST_CM: i32 = 100;
/// Smallest bed-gain move worth a feed write.
const BED_EPSILON: f32 = 1.0 / 256.0;

/// Which parts of the room [`AudioSpace::feed`] drives. All on in the window;
/// tests switch parts off to hear one part alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpaceParts {
    /// Fight-event plays (hit, block, parry, knockdown, swing).
    pub events: bool,
    /// Stride footsteps.
    pub footsteps: bool,
    /// Tension levels and the kill stinger.
    pub tension: bool,
    /// The crowd bed and its gain.
    pub bed: bool,
}

impl SpaceParts {
    /// Everything on: the window.
    pub const ALL: Self = Self {
        events: true,
        footsteps: true,
        tension: true,
        bed: true,
    };
    /// Only the fight-event plays and the listener.
    pub const EVENTS_ONLY: Self = Self {
        events: true,
        footsteps: false,
        tension: false,
        bed: false,
    };
}

/// What the feed writes of this module did, for the exit line and tests.
/// The feed's own `sent`, `applied`, `dropped_full` and `unknown_cue` stay the
/// truth for the queue; these say what this module asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpaceStats {
    /// Plays placed at a fighter's position.
    pub positioned: u64,
    /// Plays with no position because no view named the fighter.
    pub flat: u64,
    /// Footsteps played.
    pub footsteps: u64,
    /// Tension levels sent.
    pub tension_sends: u64,
    /// Bed gain writes sent.
    pub bed_writes: u64,
    /// Writes the bank refused for a missing cue name (also counted by the
    /// feed as `unknown_cue`).
    pub unknown_cue: u64,
    /// Writes the ring refused (full or closed).
    pub refused: u64,
    /// Tension levels sent to a bank that carries no music: the bed cannot
    /// play them, so the level is spoken to nobody (loud, not silent).
    pub tension_unheard: u64,
}

/// Window-side audio presentation state: where the listener is, what the
/// music was last told, whether the crowd bed runs, and each fighter's stride.
#[derive(Clone, Debug)]
pub struct AudioSpace {
    parts: SpaceParts,
    surface: Surface,
    listener_cm: [i32; 3],
    yaw_deg: i32,
    tension: TensionTracker,
    tension_told: bool,
    bed: BedState,
    bed_gain_sent: f32,
    stride: [Stride; MAX_SLOTS],
    stats: SpaceStats,
}

/// The crowd bed's life: not started, running, or given up on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BedState {
    Idle,
    Running,
    /// The bank has no such cue (counted once); do not ask again each tick.
    Absent,
}

/// One fighter's walked distance since its last footstep.
#[derive(Clone, Copy, Debug, Default)]
struct Stride {
    last_xz: Option<[f32; 2]>,
    walked_m: f32,
}

impl Default for AudioSpace {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioSpace {
    /// The window's state: everything on, sand underfoot, the listener at the
    /// arena center facing north until the first camera arrives.
    #[must_use]
    pub fn new() -> Self {
        Self::with_parts(SpaceParts::ALL)
    }

    /// As [`Self::new`] driving only `parts`.
    #[must_use]
    pub fn with_parts(parts: SpaceParts) -> Self {
        Self {
            parts,
            surface: Surface::Sand,
            listener_cm: [0; 3],
            yaw_deg: 0,
            tension: TensionTracker::new(),
            tension_told: false,
            bed: BedState::Idle,
            bed_gain_sent: 1.0,
            stride: [Stride::default(); MAX_SLOTS],
            stats: SpaceStats::default(),
        }
    }

    /// The floor under the fighters (picks the footstep cue).
    pub fn set_surface(&mut self, surface: Surface) {
        self.surface = surface;
    }

    /// Put the listener at the camera: `eye_m` is where it sits, `target_m`
    /// what it looks at, both `[x east, y up, z south]` meters. A non-finite
    /// pose keeps the last good one; a camera looking straight down keeps the
    /// last yaw.
    pub fn follow_camera(&mut self, eye_m: [f32; 3], target_m: [f32; 3]) {
        if eye_m.iter().chain(&target_m).any(|v| !v.is_finite()) {
            return;
        }
        self.listener_cm = [to_cm(eye_m[0]), to_cm(eye_m[1]), to_cm(eye_m[2])];
        let (dx, dz) = (target_m[0] - eye_m[0], target_m[2] - eye_m[2]);
        if dx.abs() + dz.abs() > 1e-4 {
            // Clockwise from north (north is -z): facing east is 90.
            // `to_cm` rounds hundredths, so hundredths of a degree in, whole
            // degrees out.
            let hundredths = dx.atan2(-dz).to_degrees() / 100.0;
            self.yaw_deg = to_cm(hundredths).rem_euclid(360);
        }
    }

    /// Listener position now, `[east, up, south]` cm.
    #[must_use]
    pub fn listener_cm(&self) -> [i32; 3] {
        self.listener_cm
    }

    /// Listener yaw now, degrees clockwise from north, `0..360`.
    #[must_use]
    pub fn yaw_deg(&self) -> i32 {
        self.yaw_deg
    }

    /// The music level last reported by the tracker.
    #[must_use]
    pub fn tension_level(&self) -> Tension {
        self.tension.level()
    }

    /// What this module has asked of the feed so far.
    #[must_use]
    pub fn stats(&self) -> SpaceStats {
        self.stats
    }

    /// Queue one tick: listener, events, footsteps, tension, bed. `views` are
    /// the fighter positions after the tick (`App::fighter_views`). Never
    /// fails and never blocks; refusals are counted.
    pub fn feed(
        &mut self,
        tx: &mut FeedTx,
        tick: u32,
        outcome: &TickOutcome,
        views: &[FighterView],
    ) {
        let sent = tx.listener(tick, self.listener_cm, self.yaw_deg);
        self.note(sent);
        if self.parts.events {
            self.feed_events(tx, tick, &outcome.cues, views);
        }
        if self.parts.footsteps {
            self.feed_footsteps(tx, tick, views);
        }
        if self.parts.tension {
            self.feed_tension(tx, tick, outcome);
        }
        if self.parts.bed {
            self.feed_bed(tx, tick, outcome.bed_gain);
        }
    }

    /// Count a refused write; the feed counts the same refusal on its side.
    fn note(&mut self, sent: Result<(), FeedError>) {
        match sent {
            Ok(()) => {}
            Err(FeedError::UnknownCue) => self.stats.unknown_cue += 1,
            Err(FeedError::Full | FeedError::Closed) => self.stats.refused += 1,
        }
    }

    fn feed_events(
        &mut self,
        tx: &mut FeedTx,
        tick: u32,
        cues: &[FightCue],
        views: &[FighterView],
    ) {
        for &event in cues {
            let (attacker, victim) = event_slots(event);
            let (a_pos, v_pos) = (position_of(views, attacker), position_of(views, victim));
            // One fighter missing: the blow sits where the other stands. Both
            // missing: nowhere to put it, so it plays flat (counted).
            let here = v_pos.or(a_pos);
            let play = cue_for_event(
                event,
                a_pos.or(here).unwrap_or_default(),
                v_pos.or(here).unwrap_or_default(),
            );
            let Some(mut play) = play else {
                continue;
            };
            if here.is_none() {
                play.pos_cm = None;
                self.stats.flat += 1;
            } else {
                self.stats.positioned += 1;
            }
            let sent = tx.play(tick, play.name, play.gain, play.pos_cm);
            self.note(sent);
            if matches!(event, FightCue::Knockdown { .. }) && self.parts.tension {
                let sent = tx.stinger(tick, cue_names::STINGER_KILL);
                self.note(sent);
            }
        }
    }

    fn feed_footsteps(&mut self, tx: &mut FeedTx, tick: u32, views: &[FighterView]) {
        for view in views {
            let Some(stride) = self.stride.get_mut(usize::from(view.slot)) else {
                continue;
            };
            let here = [view.pos_m[0], view.pos_m[2]];
            let Some(last) = stride.last_xz.replace(here) else {
                continue;
            };
            let step = (here[0] - last[0]).hypot(here[1] - last[1]);
            if !step.is_finite() || step > TELEPORT_M {
                stride.walked_m = 0.0;
                continue;
            }
            stride.walked_m += step;
            if stride.walked_m < STRIDE_M {
                continue;
            }
            stride.walked_m -= STRIDE_M;
            let play = footstep(self.surface, feet_cm(view));
            let sent = tx.play(tick, play.name, play.gain, play.pos_cm);
            self.stats.footsteps += 1;
            self.note(sent);
        }
    }

    fn feed_tension(&mut self, tx: &mut FeedTx, tick: u32, outcome: &TickOutcome) {
        // The bed is silent until asked: say the starting level once.
        let mut send = if self.tension_told {
            None
        } else {
            self.tension_told = true;
            Some(self.tension.level())
        };
        if let Some(level) = self.tension.update(outcome.layers.intensity) {
            send = Some(level);
        }
        let Some(level) = send else {
            return;
        };
        let sent = tx.tension(tick, level);
        self.stats.tension_sends += 1;
        if tx.bank().music().is_none() {
            self.stats.tension_unheard += 1;
        }
        self.note(sent);
    }

    fn feed_bed(&mut self, tx: &mut FeedTx, tick: u32, bed_gain: f32) {
        if self.bed == BedState::Absent {
            return;
        }
        if self.bed == BedState::Idle {
            // The loop starts at full level; the trim below rides it.
            match tx.play(tick, cue_names::CROWD, 1.0, None) {
                Ok(()) => {
                    self.bed = BedState::Running;
                    self.bed_gain_sent = 1.0;
                }
                Err(FeedError::UnknownCue) => {
                    self.bed = BedState::Absent;
                    self.stats.unknown_cue += 1;
                    return;
                }
                Err(FeedError::Full | FeedError::Closed) => {
                    self.stats.refused += 1;
                    return;
                }
            }
        }
        let gain = if bed_gain.is_finite() {
            bed_gain.clamp(0.0, 1.0)
        } else {
            1.0
        };
        // Tiny moves are not worth a write, but the exact ends always are, so
        // a recovered bed lands on 1.0 and a muted one on 0.0.
        let moved = (gain - self.bed_gain_sent).abs();
        let at_end = gain == 0.0 || gain == 1.0;
        if moved == 0.0 || (moved < BED_EPSILON && !at_end) {
            return;
        }
        let sent = tx.trim(tick, cue_names::CROWD, gain);
        if sent.is_ok() {
            self.bed_gain_sent = gain;
            self.stats.bed_writes += 1;
        }
        self.note(sent);
    }
}

/// Feet of `view` on the feed's axes.
fn feet_cm(view: &FighterView) -> [i32; 3] {
    [
        to_cm(view.pos_m[0]),
        to_cm(view.pos_m[1]),
        to_cm(view.pos_m[2]),
    ]
}

/// Where a blow on `slot` lands: its chest.
fn position_of(views: &[FighterView], slot: u8) -> Option<[i32; 3]> {
    let view = views.iter().find(|view| view.slot == slot)?;
    let feet = feet_cm(view);
    Some([feet[0], feet[1].saturating_add(CHEST_CM), feet[2]])
}

/// Attacker and victim slots of an event (the fighter itself for a stamina
/// whiff, which has no cue anyway).
fn event_slots(event: FightCue) -> (u8, u8) {
    match event {
        FightCue::Hit {
            attacker, victim, ..
        }
        | FightCue::Blocked {
            attacker, victim, ..
        }
        | FightCue::Parried { attacker, victim }
        | FightCue::Dodged { attacker, victim }
        | FightCue::Knockdown { attacker, victim } => (attacker, victim),
        FightCue::StaminaOut { fighter } => (fighter, fighter),
    }
}

#[cfg(test)]
#[path = "window_audio_space_tests.rs"]
mod tests;
