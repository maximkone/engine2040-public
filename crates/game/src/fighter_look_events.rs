//! fighter_look_events: the look's run phase feeds the clip event lane (BG5).
//!
//! Consumer: [`FighterLook::observe_tick`](crate::fighter_look::FighterLook::observe_tick)
//! steps the lane after the act fold; the window reads
//! [`FighterLook::clip_footsteps`] right after and the audio feed plays each
//! step on the exact tick. Data owner: the std lane in
//! [`crate::clip_events`] owns the authored plant track and the phase memory;
//! the look only says where its run clip is. Units: clip seconds become ticks
//! at the fixed rate, and the cycle length is the rig's own run clip. A body
//! of any clip length plants at the same fraction of its stride.
//! Determinism: presentation only, never hashed; a resim holds the lane at the
//! high tick like the cape and the limiter. Errors: none; a slot with no rig
//! or no run clip simply has no clock.

use super::{Act, FighterLook, TICK_HZ};
use crate::clip_events::{ClipClock, ClipFootstep, RUN_CLIP};

impl FighterLook {
    /// Fill the lane's clocks from the slots that show the run clip and step
    /// it for `tick`. A `replaying` tick (a rollback resim) keeps the high
    /// tick's phase and plays none.
    pub(crate) fn step_clip_events(&mut self, tick: u32, replaying: bool) {
        self.clip_events.begin();
        for (index, slot) in self.slots.iter().enumerate() {
            if !slot.seen || slot.act != Act::Run {
                continue;
            }
            let Ok(slot_id) = u8::try_from(index) else {
                continue;
            };
            let Some(rig) = self.rigs.get(self.rig_for_slot(index)) else {
                continue;
            };
            let Some(run) = rig.clips.get(usize::from(RUN_CLIP)) else {
                continue;
            };
            let cycle_ticks = run.duration * TICK_HZ;
            let phase_ticks = slot.run_clip_s(0.0) * TICK_HZ;
            if !(cycle_ticks.is_finite() && cycle_ticks > 0.0 && phase_ticks.is_finite()) {
                continue;
            }
            self.clip_events.add_clock(ClipClock {
                slot: slot_id,
                clip: RUN_CLIP,
                phase_ticks,
                cycle_ticks,
            });
        }
        self.clip_events.step(tick, replaying);
    }

    /// The run clip's footstep events the last free tick stepped (BG5): empty
    /// when none fired, on a replay tick, or for a look with no rigs.
    /// Presentation only.
    #[must_use]
    pub fn clip_footsteps(&self) -> &[ClipFootstep] {
        self.clip_events.steps()
    }
}
