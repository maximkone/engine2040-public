//! fighter_look_rewind: the look's rollback checkpoint ring and the fading
//! visual correction the frame draws after a resim (EG39).
//!
//! Consumer: [`FighterLook::observe_tick`](super::FighterLook::observe_tick)
//! saves one checkpoint per folded tick and rewinds to it when a resim repeats
//! a tick; `slot_sample_raw` adds the fading offset to the drawn root. Data
//! owner: this module owns the rewind ring and the correction vector; the slot
//! anim stays in `fighter_look.rs`. Units: meters, fixed ticks at 60 Hz.
//! Determinism: presentation only; nothing here is hashed. Errors: none; a
//! rewind deeper than the ring falls back to the old fold.

use crate::anim_bridge::FighterSlot;
use crate::weapon_trail::TrailRing;
use engine_anim::net_pose::{correction_fade_permille, CORRECTION_FADE_TICKS};

use super::{slot_root, FighterLook, SlotAnim, MAX_LOOK_FIGHTERS};

/// Ticks of presentation history kept for a rollback rewind (EG39): 16
/// covers a 150 ms round-trip (9 ticks) with margin, so a 4-tick resim always
/// finds the checkpoint through its first tick minus one.
const LOOK_REWIND_TICKS: usize = 16;

/// One tick of presentation state the rollback rewind restores (EG39): the
/// slot anim, the bridge pick/fade, the blade trails and the camera trauma.
/// Float layers that own their own simulation (capes, limit lanes, ragdoll,
/// limbs) are not copied; a short resim holds them at the high tick.
#[derive(Clone)]
pub(super) struct LookCheckpoint {
    tick: u32,
    slots: [SlotAnim; MAX_LOOK_FIGHTERS],
    bridge: Vec<FighterSlot>,
    trails: TrailRing,
    shake_trauma_permille: u32,
}

impl LookCheckpoint {
    fn empty() -> Self {
        Self {
            tick: u32::MAX,
            slots: [SlotAnim::new(); MAX_LOOK_FIGHTERS],
            bridge: Vec::new(),
            trails: TrailRing::new(),
            shake_trauma_permille: 0,
        }
    }
}

/// The visual correction a rollback leaves behind: the roots drawn just
/// before the replayed ticks and the high tick they were drawn at. The frame
/// eases from those roots to the corrected ones over
/// [`CORRECTION_FADE_TICKS`] ticks. Presentation only; the sim hash never
/// reads it.
#[derive(Clone, Copy)]
pub(super) struct Correction {
    tick: u32,
    from: [[f32; 3]; MAX_LOOK_FIGHTERS],
}

impl Correction {
    /// Ticks since the correction was captured at `tick`; 0 on the correction
    /// tick and `CORRECTION_FADE_TICKS` once the fade is over.
    pub(super) fn age(&self, tick: u32) -> u32 {
        tick.saturating_sub(self.tick)
    }
}

/// One empty rewind slot per tick of history; the constructor and the test
/// literals share this shape.
pub(super) fn empty_checkpoints() -> Vec<LookCheckpoint> {
    (0..LOOK_REWIND_TICKS)
        .map(|_| LookCheckpoint::empty())
        .collect()
}

impl FighterLook {
    /// The checkpoint through `tick`, if the rewind ring still holds it.
    pub(super) fn checkpoint_through(&self, tick: u32) -> Option<&LookCheckpoint> {
        self.checkpoints
            .iter()
            .find(|checkpoint| checkpoint.tick == tick)
    }

    /// Restore the presentation state through `tick - 1` and park the look
    /// on that tick, so the normal fold replays `tick` with `elapsed` 1.
    /// False when the ring no longer holds the checkpoint (a rewind deeper
    /// than [`LOOK_REWIND_TICKS`]); the caller then folds on the high state.
    pub(super) fn rewind_to(&mut self, tick: u32) -> bool {
        let Some(index) = self
            .checkpoints
            .iter()
            .position(|checkpoint| checkpoint.tick == tick.saturating_sub(1))
        else {
            return false;
        };
        let checkpoint = &self.checkpoints[index];
        for (slot, saved) in self.slots.iter_mut().zip(checkpoint.slots.iter()) {
            *slot = *saved;
        }
        self.bridge.clone_from(&checkpoint.bridge);
        self.trails.clone_from(&checkpoint.trails);
        self.shake_trauma_permille = checkpoint.shake_trauma_permille;
        self.tick = checkpoint.tick;
        true
    }

    /// Capture the roots drawn at `high` before a rewind, so the frame eases
    /// from where the viewer last saw each slot to the replayed root. A second
    /// correction before the fade ends starts from the drawn root, not the
    /// corrected one, so two resims in a row never snap.
    pub(super) fn note_correction(&mut self, high: u32) {
        let from = std::array::from_fn(|index| self.drawn_root(index, 1.0));
        self.correction = Some(Correction { tick: high, from });
    }

    /// The fading offset drawn for `slot` this tick: the root the viewer last
    /// saw minus the corrected root, scaled by the tick-indexed fade. Zero at
    /// rest, on an unseen slot and after [`CORRECTION_FADE_TICKS`].
    pub(super) fn correction_offset(&self, slot: usize) -> [f32; 3] {
        let Some(correction) = self.correction else {
            return [0.0; 3];
        };
        let Some(anim) = self.slots.get(slot) else {
            return [0.0; 3];
        };
        let age = correction.age(self.tick);
        let fade = correction_fade_permille(age, CORRECTION_FADE_TICKS);
        if fade == 0 {
            return [0.0; 3];
        }
        let (base, _) = slot_root(anim, 1.0);
        let from = correction.from[slot];
        let scale = f32::from(u16::try_from(fade).unwrap_or(1000)) / 1000.0;
        [
            (from[0] - base[0]) * scale,
            (from[1] - base[1]) * scale,
            (from[2] - base[2]) * scale,
        ]
    }

    /// The root drawn for `slot` at `alpha`: the tick's root plus the fading
    /// correction offset. Used to capture the viewer's last picture before a
    /// rewind; the frame adds the same vector in `slot_sample_raw`.
    fn drawn_root(&self, slot: usize, alpha: f32) -> [f32; 3] {
        let base = self
            .slots
            .get(slot)
            .map_or([0.0; 3], |anim| slot_root(anim, alpha).0);
        let offset = self.correction_offset(slot);
        [
            base[0] + offset[0],
            base[1] + offset[1],
            base[2] + offset[2],
        ]
    }

    /// Length in meters of the visual correction drawn for `slot` at the
    /// current tick: 0 when no rewind is fading. Receipts and tests only; the
    /// frame reads the full vector through `slot_sample_raw`.
    #[must_use]
    pub fn correction_offset_m(&self, slot: usize) -> f32 {
        let offset = self.correction_offset(slot);
        (offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2]).sqrt()
    }

    /// Save the state through `tick` for a later rewind, replacing an entry a
    /// replay already rewrote. `clone_from` keeps the ring allocation-free
    /// after its first pass.
    pub(super) fn save_checkpoint(&mut self, tick: u32) {
        if self.checkpoints.is_empty() {
            return;
        }
        let index = self
            .checkpoints
            .iter()
            .position(|checkpoint| checkpoint.tick == tick)
            .unwrap_or(self.rewind_cursor);
        let checkpoint = &mut self.checkpoints[index];
        checkpoint.tick = tick;
        for (saved, slot) in checkpoint.slots.iter_mut().zip(self.slots.iter()) {
            *saved = *slot;
        }
        checkpoint.bridge.clone_from(&self.bridge);
        checkpoint.trails.clone_from(&self.trails);
        checkpoint.shake_trauma_permille = self.shake_trauma_permille;
        self.rewind_cursor = (index + 1) % self.checkpoints.len();
    }
}
