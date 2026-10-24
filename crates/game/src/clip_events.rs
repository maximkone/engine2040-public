//! clip_events: the run clip's authored footstep events reach the audio feed.
//!
//! Consumer: the look builds one [`ClipClock`] per running slot after each
//! folded tick and steps the lane; [`crate::window_audio_space`] plays the
//! lane's [`ClipFootstep`]s on the tick the clip crossed them. Data owner:
//! `engine-anim` owns the event system — [`EventBank`] holds the run clip's
//! authored plant ticks, [`Router`] names the callee and [`CallbackLog`]
//! records the routed call — so the frame-exact event lane finally has a live
//! consumer outside its own crate. This module only maps a clip phase onto
//! those ticks. Units: fixed ticks at 60 Hz; a clock's phase is in ticks of
//! its own clip cycle, and the authored track is in canonical ticks of
//! [`RUN_CYCLE_TICKS`], so a body of any clip length plants at the same
//! fraction of its stride. Determinism: the same clocks give the same steps;
//! a rewind or a long phase jump restarts the clock instead of firing from
//! the gap. Presentation only: nothing here reads or writes the sim, so the
//! golden `d57b5c8ae3096614` cannot move. Errors: none propagate. Budget: at
//! most two bank lookups per clock per tick, no heap after warmup.

use engine_anim::events::{CallbackLog, Callee, Router, FOOTSTEP_CODE};
use engine_anim::graph::{EventBank, FiredEvent, NodeId, TimedEvent};

/// Slots the lane tracks: the canonical roster cap. The audio space re-exports
/// it so its stride and step-skip arrays stay the same size.
pub const MAX_SLOTS: usize = 32;
/// The look's run clip index in the clip-file order (idle, run, three swings,
/// down); the look reads it to pick each rig's run length.
pub const RUN_CLIP: u8 = 1;
/// Canonical ticks in one run cycle: the authored track's time base. A clip of
/// any duration maps its phase onto this cycle. A `u16` so the fraction below
/// converts with `f32::from`, no `as`.
pub const RUN_CYCLE_TICKS: u16 = 48;
/// Foot plants of the shipped run clip, in canonical ticks: one eighth and
/// five eighths into the stride.
pub const RUN_PLANTS: [u16; 2] = [6, 30];
/// A phase jump longer than half a cycle is a cut or a resim, not a stride:
/// the lane restarts that slot's clock instead of firing every plant it
/// skipped.
const RESTART_TURNS: f32 = 0.5;

/// One slot's run clip after a folded tick, as the look shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipClock {
    /// Fighter slot the clock belongs to.
    pub slot: u8,
    /// Clip index the phase belongs to ([`RUN_CLIP`] for a run slot).
    pub clip: u8,
    /// Phase into the clip, in ticks of the clip's own cycle.
    pub phase_ticks: f32,
    /// The clip's cycle length, in ticks.
    pub cycle_ticks: f32,
}

/// One run-clip footstep the lane fired: the slot, the sim tick and the routed
/// cue code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipFootstep {
    /// Fighter slot that planted the foot.
    pub slot: u8,
    /// Fixed tick the clip crossed the plant on.
    pub tick: u32,
    /// Routed cue code ([`FOOTSTEP_CODE`] for every plant in the bank).
    pub code: u32,
}

/// A slot's phase at its last step: the clip and its fraction of the cycle.
#[derive(Clone, Copy, Debug)]
struct Was {
    clip: u8,
    turns: f32,
}

/// The run clip's event lane: the authored plant track, its router, the
/// per-tick callback log and each slot's last phase. One owner for the
/// look-to-audio event hand-off.
#[derive(Clone, Debug)]
pub struct ClipEventLane {
    bank: EventBank,
    router: Router,
    log: CallbackLog,
    prev: [Option<Was>; MAX_SLOTS],
    last_tick: Option<u32>,
    clocks: Vec<ClipClock>,
    fired: Vec<FiredEvent>,
    steps: Vec<ClipFootstep>,
}

impl Default for ClipEventLane {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipEventLane {
    /// An empty lane with the run clip's plant track authored (one eighth and
    /// five eighths of the canonical cycle) and the footstep code routed to
    /// the audio callee.
    #[must_use]
    pub fn new() -> Self {
        let mut bank = EventBank::new();
        bank.insert(
            u32::from(RUN_CLIP),
            RUN_PLANTS
                .iter()
                .map(|&tick| TimedEvent {
                    tick: u32::from(tick),
                    code: FOOTSTEP_CODE,
                })
                .collect(),
        );
        Self {
            bank,
            router: Router::with_footstep_hit(),
            log: CallbackLog::with_capacity(MAX_SLOTS * 2),
            prev: [None; MAX_SLOTS],
            last_tick: None,
            clocks: Vec::with_capacity(MAX_SLOTS),
            fired: Vec::with_capacity(MAX_SLOTS * 2),
            steps: Vec::with_capacity(MAX_SLOTS * 2),
        }
    }

    /// Start the tick's clocks: the look adds one [`ClipClock`] per slot that
    /// shows the run clip.
    pub fn begin(&mut self) {
        self.clocks.clear();
    }

    /// Add one slot's run clock for this tick; out-of-cap slots are dropped.
    pub fn add_clock(&mut self, clock: ClipClock) {
        if usize::from(clock.slot) < MAX_SLOTS {
            self.clocks.push(clock);
        }
    }

    /// Step one tick: fire the plants each clock crossed since its last step
    /// and return the steps. A `replaying` tick (a rollback resim) keeps the
    /// last free tick's phase and plays none: the steps list stays empty.
    pub fn step(&mut self, tick: u32, replaying: bool) -> &[ClipFootstep] {
        self.steps.clear();
        if replaying {
            return &self.steps;
        }
        // A tick at or behind the lane's tape is a rewind: forget every phase,
        // so the first replayed tick never fires from the jump back.
        if self.last_tick.is_some_and(|last| tick <= last) {
            self.prev = [None; MAX_SLOTS];
        }
        self.last_tick = Some(tick);
        self.fired.clear();
        let mut present = 0u64;
        for index in 0..self.clocks.len() {
            let clock = self.clocks[index];
            let slot = usize::from(clock.slot);
            present |= 1 << slot;
            let now = phase_turns(clock);
            let Some(was) = self.prev[slot] else {
                self.prev[slot] = Some(Was {
                    clip: clock.clip,
                    turns: now,
                });
                continue;
            };
            self.prev[slot] = Some(Was {
                clip: clock.clip,
                turns: now,
            });
            if was.clip != clock.clip {
                continue;
            }
            let forward = (now - was.turns).rem_euclid(1.0);
            if forward > RESTART_TURNS {
                continue;
            }
            self.fire_crossed(tick, clock.clip, clock.slot, was.turns, now);
        }
        // A slot with no clock this tick (act change, unseen) starts fresh
        // next time: no plant fires across the gap.
        for (slot, was) in self.prev.iter_mut().enumerate() {
            if present & (1 << slot) == 0 {
                *was = None;
            }
        }
        // One tick's tape: the log is cleared each step, so it never grows
        // past this tick's calls. Collect cannot fail: every fired event is
        // stamped this tick.
        self.log.clear();
        if !self.fired.is_empty() && self.log.collect(tick, &self.fired, &self.router).is_ok() {
            for call in self.log.calls() {
                if call.callee != Callee::Audio || call.code != FOOTSTEP_CODE {
                    continue;
                }
                let Some(slot) = call
                    .node
                    .0
                    .checked_sub(1)
                    .and_then(|slot| u8::try_from(slot).ok())
                else {
                    continue;
                };
                self.steps.push(ClipFootstep {
                    slot,
                    tick: call.tick,
                    code: call.code,
                });
            }
        }
        &self.steps
    }

    /// The steps of the last free tick, in slot order.
    #[must_use]
    pub fn steps(&self) -> &[ClipFootstep] {
        &self.steps
    }

    /// Push one [`FiredEvent`] per banked cue whose canonical tick sits in the
    /// phase range `from..=to` (two ranges when the phase wrapped), stamped
    /// with the sim `tick`.
    fn fire_crossed(&mut self, tick: u32, clip: u8, slot: u8, from: f32, to: f32) {
        let wrapped = to < from;
        for plant in RUN_PLANTS {
            let at = f32::from(plant) / f32::from(RUN_CYCLE_TICKS);
            let crossed = if wrapped {
                at > from || at <= to
            } else {
                at > from && at <= to
            };
            if !crossed {
                continue;
            }
            for event in self.bank.at(u32::from(clip), u32::from(plant)) {
                self.fired.push(FiredEvent {
                    node: NodeId(1 + u32::from(slot)),
                    tick,
                    code: event.code,
                });
            }
        }
    }
}

/// `clock`'s phase as a fraction of its own cycle, in `0.0..1.0`. A
/// non-finite or empty cycle reads as zero.
fn phase_turns(clock: ClipClock) -> f32 {
    if !clock.phase_ticks.is_finite() || !(clock.cycle_ticks.is_finite() && clock.cycle_ticks > 0.0)
    {
        return 0.0;
    }
    (clock.phase_ticks / clock.cycle_ticks).rem_euclid(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clock at `phase` ticks of a `cycle`-tick run clip on slot 0.
    fn clock(phase: f32, cycle: f32) -> ClipClock {
        ClipClock {
            slot: 0,
            clip: RUN_CLIP,
            phase_ticks: phase,
            cycle_ticks: cycle,
        }
    }

    /// Step a fresh lane over `ticks` with the phase advancing one canonical
    /// tick per sim tick; returns the fired steps.
    fn run_tape(ticks: std::ops::RangeInclusive<u32>) -> Vec<ClipFootstep> {
        let mut lane = ClipEventLane::new();
        let mut steps = Vec::new();
        for tick in ticks {
            lane.begin();
            lane.add_clock(clock(tick as f32, f32::from(RUN_CYCLE_TICKS)));
            steps.extend_from_slice(lane.step(tick, false));
        }
        steps
    }

    #[test]
    fn the_run_clip_plants_fire_on_their_exact_tick() {
        let steps = run_tape(0..=60);
        let ticks: Vec<u32> = steps.iter().map(|step| step.tick).collect();
        // One eighth and five eighths of a 48-tick cycle, then the next cycle.
        assert_eq!(ticks, vec![6, 30, 54], "{steps:?}");
        assert!(steps.iter().all(|step| step.slot == 0));
        assert!(steps.iter().all(|step| step.code == FOOTSTEP_CODE));
    }

    #[test]
    fn the_same_tape_replays_equal() {
        let first = run_tape(0..=60);
        let second = run_tape(0..=60);
        assert_eq!(first, second, "cue tape must replay x2");
    }

    #[test]
    fn a_clip_with_no_event_fires_none() {
        let mut lane = ClipEventLane::new();
        let mut fired = 0;
        for tick in 0..=60u32 {
            lane.begin();
            lane.add_clock(ClipClock {
                slot: 0,
                clip: 2,
                phase_ticks: tick as f32,
                cycle_ticks: f32::from(RUN_CYCLE_TICKS),
            });
            fired += lane.step(tick, false).len();
        }
        assert_eq!(fired, 0, "an unauthored clip never fires");
    }

    #[test]
    fn a_shorter_clip_keeps_the_plant_fraction() {
        // A 40-tick run clip: the plants sit at the same eighth and five
        // eighths of the stride, so they fire at phase 5 and 25.
        let mut lane = ClipEventLane::new();
        let mut ticks = Vec::new();
        for tick in 0..=40u32 {
            lane.begin();
            lane.add_clock(clock(tick as f32, 40.0));
            ticks.extend(lane.step(tick, false).iter().map(|step| step.tick));
        }
        assert_eq!(ticks, vec![5, 25], "plant phase, not clip ticks");
    }

    #[test]
    fn a_phase_jump_restarts_instead_of_spraying_plants() {
        let mut lane = ClipEventLane::new();
        lane.begin();
        lane.add_clock(clock(0.0, f32::from(RUN_CYCLE_TICKS)));
        assert!(lane.step(1, false).is_empty());
        // 40 canonical ticks in one step is a cut, not a stride: without the
        // restart rule this would fire both plants at once.
        lane.begin();
        lane.add_clock(clock(40.0, f32::from(RUN_CYCLE_TICKS)));
        assert!(lane.step(2, false).is_empty(), "no spray from a jump");
        // The clock keeps counting from where it landed.
        lane.begin();
        lane.add_clock(clock(41.0, f32::from(RUN_CYCLE_TICKS)));
        assert!(lane.step(3, false).is_empty());
    }

    #[test]
    fn a_rewind_resets_the_phase_memory() {
        let mut lane = ClipEventLane::new();
        for tick in 0..=10u32 {
            lane.begin();
            lane.add_clock(clock(tick as f32, f32::from(RUN_CYCLE_TICKS)));
            lane.step(tick, false);
        }
        // A resim tick behind the tape (and a different phase) fires nothing
        // from the jump back.
        lane.begin();
        lane.add_clock(clock(47.0, f32::from(RUN_CYCLE_TICKS)));
        assert!(lane.step(3, false).is_empty());
    }

    #[test]
    fn a_replaying_tick_keeps_the_last_steps() {
        let mut lane = ClipEventLane::new();
        lane.begin();
        lane.add_clock(clock(6.0, f32::from(RUN_CYCLE_TICKS)));
        // First step seeds the phase: no plant on the first sight.
        assert!(lane.step(0, false).is_empty());
        lane.begin();
        lane.add_clock(clock(0.0, f32::from(RUN_CYCLE_TICKS)));
        // A resim tick holds the high tick's steps instead of firing anew.
        assert!(lane.step(9, true).is_empty());
    }
}
