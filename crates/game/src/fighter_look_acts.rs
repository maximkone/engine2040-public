//! fighter_look_acts: the per-tick fold plus bridge bulk of [`FighterLook`].
//!
//! Consumer: [`FighterLook::observe_tick`] (this module's entry) drives the
//! look from the same views, hp and cues the window already folds; sampling
//! stays in `slot_pose`. Data owner: this module owns nothing new, it only
//! implements the fold on the parent's slots. Units: meters, fixed ticks at
//! 60 Hz, clip seconds. Determinism: presentation only, never match state,
//! so the tick hash cannot move. Errors: none new; a refused bridge tick
//! keeps the old act.

use engine_anim::crossfade::SlotFade;
use engine_arena::GaitPhase;
use engine_audio::FightCue;
use engine_combat::fight::HITSTOP_TICKS;

use crate::anim_bridge::{BridgeClip, BridgeTuning, FighterSlot};
use crate::pose_hold::hold_mask;
use crate::FighterView;

use super::{
    angle_delta, run_phase_scale, Act, FighterLook, SlotAnim, FLASH_TICKS, IMPACT_TICKS,
    MAX_LOOK_FIGHTERS, MAX_TURN_PER_TICK, RUN_REF_M_PER_TICK, RUN_SPEED_M_PER_TICK,
    SHAKE_DECAY_PERMILLE_PER_TICK, SHAKE_HIT_PERMILLE, SHAKE_KNOCKDOWN_PERMILLE, TICK_HZ,
};

/// One idle bridge slot per look slot, ids `0..MAX_LOOK_FIGHTERS`. Cold path
/// only (load plus test constructors); the fold drives them in place.
pub(crate) fn bridge_slots() -> Vec<FighterSlot> {
    (0..MAX_LOOK_FIGHTERS)
        .map(|index| FighterSlot::new(u8::try_from(index).unwrap_or(u8::MAX)))
        .collect()
}

/// The impact pair's per-slot freeze for a tick: whatever `fold_cues`
/// recorded on the bridge, decided by the `hold_mask` owner, so bad slots
/// never freeze. Exactly one pair freezes; anything else (quiet ticks,
/// overlapping impacts) advances like today. Pure over the bridge plus views.
fn hold_frozen(
    bridge: &[FighterSlot],
    tick: u32,
    views: &[FighterView],
) -> [bool; MAX_LOOK_FIGHTERS] {
    let mut first: Option<u8> = None;
    let mut second: Option<u8> = None;
    let mut count = 0u32;
    for (index, bridge_slot) in bridge.iter().enumerate() {
        if tick < bridge_slot.hold_until() {
            count += 1;
            if first.is_none() {
                first = u8::try_from(index).ok();
            } else if second.is_none() {
                second = u8::try_from(index).ok();
            }
        }
    }
    let mut frozen = [false; MAX_LOOK_FIGHTERS];
    if count == 2 {
        if let (Some(attacker), Some(victim)) = (first, second) {
            let fighters = u8::try_from(views.len()).unwrap_or(u8::MAX);
            hold_mask(fighters, attacker, victim, true, &mut frozen);
        }
    }
    frozen
}

/// Face the nearest standing rival; with none, face the motion. Turn rate
/// capped per tick, run speed parked for falls and bad input. Pure over the
/// slot plus inputs.
fn face_rival(
    slot: &mut SlotAnim,
    mapped: Act,
    rival: Option<(f32, f32)>,
    step: (f32, f32),
    speed: f32,
) {
    slot.prev_yaw = slot.yaw;
    slot.run_speed = if mapped == Act::Down || !speed.is_finite() {
        0.0
    } else {
        speed
    };
    let aim = match rival {
        Some(dir) if dir.0 * dir.0 + dir.1 * dir.1 > 1e-6 => Some(dir),
        _ if speed > RUN_SPEED_M_PER_TICK => Some(step),
        _ => None,
    };
    if let Some((dx, dz)) = aim {
        let want = dx.atan2(dz);
        let turn = angle_delta(slot.yaw, want).clamp(-MAX_TURN_PER_TICK, MAX_TURN_PER_TICK);
        slot.yaw += turn;
    }
}

/// Freeze one slot's clip clock for a hitstop tick: keep the act and march
/// `act_start` with the tick, so the clip phase stays put. Fades still blend,
/// and the bridge clock marches too, so the resume tick still maps the held
/// act, not an expired swing. Death is decided by the caller: a `Down`
/// mapping never reaches here.
fn freeze_slot(
    slot: &mut SlotAnim,
    fade: SlotFade,
    old_run_speed: f32,
    elapsed: u32,
    bridge_slot: &mut FighterSlot,
) {
    slot.prev_clip = usize::from(fade.prev_clip);
    slot.prev_time = bridge_prev_time(fade.prev_clip, fade.prev_phase_ticks, old_run_speed);
    slot.fade_left = fade.fade_left;
    slot.act_start = slot.act_start.saturating_add(elapsed);
    bridge_slot.march_hold(elapsed);
}

/// The impact's slots from one tick's fight cues: the first Hit or Parried
/// cue's `(attacker, victim)`. `None` on a quiet tick. Mirrors the
/// pose_hold impact pair across the sim/presentation boundary: the look only
/// ever sees fight cues, never combat events, so the pair is read here and
/// the hold_mask owner decides the freeze.
fn cue_impact_slots(cues: &[FightCue]) -> Option<(u8, u8)> {
    cues.iter().find_map(|cue| match *cue {
        FightCue::Hit {
            attacker, victim, ..
        } => Some((attacker, victim)),
        FightCue::Parried { attacker, victim } => Some((attacker, victim)),
        _ => None,
    })
}

/// Act for one bridge clip cue, in the look's act order (idle, run, three
/// swings, down). Pure; the clip order is pinned by the bridge index test.
pub(crate) fn bridge_act(clip: BridgeClip) -> Act {
    match clip {
        BridgeClip::Idle => Act::Idle,
        BridgeClip::Run => Act::Run,
        BridgeClip::Attack0 => Act::Attack(0),
        BridgeClip::Attack1 => Act::Attack(1),
        BridgeClip::Attack2 => Act::Attack(2),
        BridgeClip::Down => Act::Down,
    }
}

/// Leaving-clip seconds for one bridge fade: phase ticks over 60 Hz, scaled
/// for the run clip through [`run_phase_scale`] like `clip_time`. Pure.
fn bridge_prev_time(prev_clip: u8, prev_phase_ticks: u32, run_speed: f32) -> f32 {
    let phase = f32::from(u16::try_from(prev_phase_ticks).unwrap_or(u16::MAX));
    if prev_clip == BridgeClip::Run.index() {
        phase / TICK_HZ * run_phase_scale(run_speed)
    } else {
        phase / TICK_HZ
    }
}

impl FighterLook {
    /// Fold one simulated tick into the presentation state: positions,
    /// facing toward the nearest standing rival, swings from cues (the
    /// attacker starts its swing so the impact lands on the cue tick), hit
    /// tint on victims, knockdown on zero hp. Never touches match state.
    pub fn observe_tick(
        &mut self,
        tick: u32,
        views: &[FighterView],
        hp: &[u16],
        cues: &[FightCue],
    ) {
        // View-only shake decay (LF11): 1000 permille per second, integer
        // only, before the tick's impacts land. Same ticks give the same
        // trauma twice; presentation only, never the sim hash.
        let elapsed = tick.saturating_sub(self.tick);
        self.shake_trauma_permille = self
            .shake_trauma_permille
            .saturating_sub(elapsed.saturating_mul(SHAKE_DECAY_PERMILLE_PER_TICK));
        self.tick = tick;
        self.fold_positions(views);
        self.fold_cues(tick, cues);
        self.fold_acts(tick, views, hp, cues, elapsed);
        // View-only trail feed (LF14): age once, push attacking slots and
        // clear the rest. Presentation only, never the sim hash.
        self.feed_trails();
    }

    /// Shift each seen slot's position (previous, current) for the frame lerp.
    fn fold_positions(&mut self, views: &[FighterView]) {
        for view in views {
            let Some(slot) = self.slots.get_mut(usize::from(view.slot)) else {
                continue;
            };
            if !view.pos_m.iter().all(|v| v.is_finite()) {
                continue;
            }
            if slot.seen {
                slot.prev_pos = slot.pos;
            } else {
                slot.prev_pos = view.pos_m;
                slot.seen = true;
            }
            slot.pos = view.pos_m;
        }
    }

    /// Swings for attackers (impact on the cue tick), tint for victims.
    /// Balance mirror (EG45): the same blows that feed the sim's integer
    /// balance lean the look (Hit stumbles, Knockdown floors). Read-only;
    /// never writes match state. A fall never downgrades to a stumble.
    /// A `Hit` also starts the victim's additive flinch (EG18): the push
    /// runs away from the attacker in world space, decayed by the shared
    /// react-window envelope in [`slot_pose`](Self::slot_pose).
    fn fold_cues(&mut self, tick: u32, cues: &[FightCue]) {
        for cue in cues {
            match *cue {
                FightCue::Hit {
                    attacker, victim, ..
                } => {
                    self.start_swing(attacker, tick);
                    let push = Self::push_away(&self.slots, attacker, victim);
                    if self.in_reach(attacker, victim) {
                        if let Some(slot) = self.slots.get_mut(usize::from(victim)) {
                            slot.flash_until = tick + FLASH_TICKS;
                        }
                        self.add_shake_trauma(SHAKE_HIT_PERMILLE);
                    }
                    self.stumble_slot(victim);
                    self.remember_hit(victim, tick, push);
                }
                FightCue::Knockdown { attacker, victim } => {
                    self.start_swing(attacker, tick);
                    if self.in_reach(attacker, victim) {
                        if let Some(slot) = self.slots.get_mut(usize::from(victim)) {
                            slot.flash_until = tick + FLASH_TICKS;
                        }
                        self.add_shake_trauma(SHAKE_KNOCKDOWN_PERMILLE);
                    }
                    self.floor_slot(victim);
                }
                FightCue::Blocked { attacker, .. }
                | FightCue::Parried { attacker, .. }
                | FightCue::Dodged { attacker, .. } => self.start_swing(attacker, tick),
                FightCue::StaminaOut { .. } => {}
            }
        }
        // LF10 hitstop hold: the impact pair's clip clocks freeze while the
        // sim holds. Record the pair's hold expiry on the bridge (exclusive
        // end, impact tick plus three quiet ticks for four held ticks);
        // `fold_acts` freezes exactly that pair. Presentation only; the sim
        // hash never reads this.
        if let Some((attacker, victim)) = cue_impact_slots(cues) {
            let until = tick.saturating_add(HITSTOP_TICKS);
            if let Some(slot) = self.bridge.get_mut(usize::from(attacker)) {
                slot.hold_through(until);
            }
            if let Some(slot) = self.bridge.get_mut(usize::from(victim)) {
                slot.hold_through(until);
            }
        }
    }

    /// Mark `slot` stumbling unless it already lies flat or kneels: impacts
    /// never downgrade an in-flight fall, matching the sim. Bad slots are
    /// ignored. Presentation only.
    pub(crate) fn stumble_slot(&mut self, slot: u8) {
        let Some(anim) = self.slots.get_mut(usize::from(slot)) else {
            return;
        };
        match anim.balance {
            GaitPhase::Fallen | GaitPhase::Catching => {}
            GaitPhase::Steady | GaitPhase::Stumbling => {
                anim.balance = GaitPhase::Stumbling;
            }
        }
    }

    /// Mark `slot` fallen (lies flat). Bad slots are ignored.
    /// Presentation only.
    pub(crate) fn floor_slot(&mut self, slot: u8) {
        let Some(anim) = self.slots.get_mut(usize::from(slot)) else {
            return;
        };
        anim.balance = GaitPhase::Fallen;
    }

    /// World-space push away from the attacker for the victim's flinch:
    /// the victim minus the attacker on the sand plane, unit length.
    /// Degenerate (missing, unseen, non-finite or co-located) reads as
    /// zero, so the flinch stays silent instead of poisoning the pose.
    /// Pure; presentation only.
    fn push_away(slots: &[SlotAnim], attacker: u8, victim: u8) -> [f32; 2] {
        let (Some(a), Some(v)) = (
            slots.get(usize::from(attacker)),
            slots.get(usize::from(victim)),
        ) else {
            return [0.0, 0.0];
        };
        if !a.seen || !v.seen {
            return [0.0, 0.0];
        }
        let dx = v.pos[0] - a.pos[0];
        let dz = v.pos[2] - a.pos[2];
        if !dx.is_finite() || !dz.is_finite() {
            return [0.0, 0.0];
        }
        let len2 = dx * dx + dz * dz;
        if !(len2 > 1e-12) {
            return [0.0, 0.0];
        }
        let len = len2.sqrt();
        if !len.is_finite() || len <= 0.0 {
            return [0.0, 0.0];
        }
        [dx / len, dz / len]
    }

    /// Start the victim's additive flinch at `tick` along `push`.
    /// A new hit restarts the window; bad slots are ignored.
    /// Presentation only; the sim digest never reads this.
    fn remember_hit(&mut self, victim: u8, tick: u32, push: [f32; 2]) {
        let Some(slot) = self.slots.get_mut(usize::from(victim)) else {
            return;
        };
        slot.hit_start = tick;
        slot.hit_push = push;
    }

    /// Clip plus frozen time for the act being left, at the switch tick
    /// with `alpha` 0. Presentation only; the sim hash never reads it.
    fn prev_clip_time(
        &self,
        slot_index: usize,
        act: Act,
        act_start: u32,
        run_speed: f32,
    ) -> (usize, f32) {
        if let Some(rig) = self.rigs.get(self.rig_for_slot(slot_index)) {
            self.clip_time(rig, act, act_start, run_speed, 0.0)
        } else {
            let clip = match act {
                Act::Idle => 0,
                Act::Run => 1,
                Act::Attack(variant) => 2 + usize::from(variant.min(2)),
                Act::Down => 5,
            };
            let since =
                f32::from(u16::try_from(self.tick.saturating_sub(act_start)).unwrap_or(u16::MAX));
            let time = match act {
                Act::Run => since / TICK_HZ * run_phase_scale(run_speed),
                _ => since / TICK_HZ,
            };
            (clip, time)
        }
    }

    /// Knockdowns, facing toward the nearest standing rival, run or idle
    /// once a swing has played out. The act pick plus the fade come from
    /// the per-slot bridge (`anim_bridge::FighterSlot`), driven here from
    /// the same views, hp and cues; sampling stays in `slot_pose`. Attack
    /// starts keep the landed `IMPACT_TICKS` backdate (EG19c owns the press
    /// timing); every other pick starts on its tick.
    fn fold_acts(
        &mut self,
        tick: u32,
        views: &[FighterView],
        hp: &[u16],
        cues: &[FightCue],
        elapsed: u32,
    ) {
        for slot in &mut self.slots {
            slot.fade_left = slot.fade_left.saturating_sub(1);
        }
        // LF10 hitstop hold: freeze the impact pair's clip clocks while the
        // sim holds; the `hold_mask` owner decides the freeze through
        // `hold_frozen`, so bad slots never freeze.
        let frozen = hold_frozen(&self.bridge, tick, views);
        let standing: Vec<(usize, [f32; 3])> = views
            .iter()
            .filter(|view| hp.get(usize::from(view.slot)).copied().unwrap_or(1) > 0)
            .map(|view| (usize::from(view.slot), view.pos_m))
            .collect();
        let tuning = BridgeTuning::new(RUN_SPEED_M_PER_TICK, RUN_REF_M_PER_TICK);
        let Ok(tuning) = tuning else {
            // Unreachable with the look's own speed constants; keep the old
            // acts rather than picking from an unvalidated tuning.
            return;
        };
        let swing_ticks = self.swing_ticks;
        for view in views {
            let index = usize::from(view.slot);
            let Some(old) = self.slots.get(index).copied() else {
                continue;
            };
            // Frozen leaving clip at the switch tick, read before any overwrite.
            // Immutable borrow ends before the mutable update below.
            let leaving = self.prev_clip_time(index, old.act, old.act_start, old.run_speed);
            let rival = standing
                .iter()
                .filter(|(other, _)| *other != index)
                .map(|(_, pos)| (pos[0] - old.pos[0], pos[2] - old.pos[2]))
                .min_by(|a, b| (a.0 * a.0 + a.1 * a.1).total_cmp(&(b.0 * b.0 + b.1 * b.1)));
            let step = (old.pos[0] - old.prev_pos[0], old.pos[2] - old.prev_pos[2]);
            let speed = (step.0 * step.0 + step.1 * step.1).sqrt();
            let hp_permille = hp.get(index).copied().unwrap_or(1);
            // Drive the bridge slot from the same tick inputs. A refused
            // tick (foreign slot, never here) keeps the old act.
            let (pick, fade) = match self.bridge.get_mut(index) {
                Some(slot) => {
                    match slot.observe_tick(tick, view, hp_permille, cues, &tuning, swing_ticks) {
                        Ok(_) => slot.look_pick(),
                        Err(_) => continue,
                    }
                }
                None => continue,
            };
            let mapped = bridge_act(pick.clip);
            let Some(slot) = self.slots.get_mut(index) else {
                continue;
            };
            // Face the nearest standing rival; with none, face the motion.
            face_rival(slot, mapped, rival, step, speed);
            let held = frozen.get(index).copied().unwrap_or(false) && mapped != Act::Down;
            if held {
                // Hitstop hold: keep the act and march the clocks with the
                // tick, so the clip phase stays put. Death still lands at
                // once: a `Down` mapping is never held.
                let Some(bridge_slot) = self.bridge.get_mut(index) else {
                    continue;
                };
                freeze_slot(slot, fade, old.run_speed, elapsed, bridge_slot);
            } else if mapped != slot.act {
                slot.prev_clip = usize::from(fade.prev_clip);
                slot.prev_time = leaving.1;
                slot.act = mapped;
                slot.act_start = match mapped {
                    Act::Attack(_) => tick.saturating_sub(IMPACT_TICKS),
                    _ => tick,
                };
                slot.fade_left = fade.fade_left;
            } else {
                // Same clip (including a restarted swing via `start_swing`):
                // carry the bridge fade plus the frozen leaving clip so the
                // switch frame still shows the old clip whole.
                slot.prev_clip = usize::from(fade.prev_clip);
                slot.prev_time =
                    bridge_prev_time(fade.prev_clip, fade.prev_phase_ticks, old.run_speed);
                slot.fade_left = fade.fade_left;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{FighterLook, SlotAnim, MAX_LOOK_FIGHTERS};
    use super::{bridge_slots, hold_frozen};
    use crate::weapon_trail::TrailRing;
    use crate::FighterView;
    use engine_audio::FightCue;
    use engine_combat::fight::HITSTOP_TICKS;

    fn hit(attacker: u8, victim: u8) -> FightCue {
        FightCue::Hit {
            attacker,
            victim,
            damage: 100,
        }
    }

    fn bare_look() -> FighterLook {
        FighterLook {
            rigs: Vec::new(),
            ring: None,
            slots: vec![SlotAnim::new(); MAX_LOOK_FIGHTERS],
            bridge: bridge_slots(),
            trails: TrailRing::new(),
            tick: 0,
            swing_ticks: 12,
            bundle_shas: Vec::new(),
            truth: Vec::new(),
            shake_trauma_permille: 0,
            uploaded_digest: crate::fighter_look::empty_upload_digest(),
        }
    }

    fn pair_views() -> Vec<FighterView> {
        vec![
            FighterView {
                slot: 0,
                pos_m: [0.0, 0.0, 0.0],
                yaw: 0.0,
            },
            FighterView {
                slot: 1,
                pos_m: [2.0, 0.0, 0.0],
                yaw: 0.0,
            },
        ]
    }

    #[test]
    fn hitstop_hold_matches_sim_four_ticks() {
        // LF10: an impact at tick 10 must freeze exactly HITSTOP_TICKS (4)
        // ticks, matching the App hitstop count (4,3,2,1 then 0), not one
        // extra tick. The bridge expiry is exclusive, so impact tick plus
        // three quiet ticks hold; tick 14 runs free.
        let mut look = bare_look();
        look.fold_cues(10, &[hit(0, 1)]);
        let until = look.bridge[0].hold_until();
        assert_eq!(
            until,
            10 + HITSTOP_TICKS,
            "hold expiry matches the sim count"
        );
        assert_eq!(
            look.bridge[1].hold_until(),
            10 + HITSTOP_TICKS,
            "victim holds as long as the attacker"
        );
        let views = pair_views();
        for tick in [10u32, 11, 12, 13] {
            let frozen = hold_frozen(&look.bridge, tick, &views);
            assert!(frozen[0] && frozen[1], "tick {tick} holds the pair");
            assert!(!frozen[2], "tick {tick} frees the rest");
        }
        let free = hold_frozen(&look.bridge, 14, &views);
        assert!(
            !free[0] && !free[1],
            "tick 14 runs free when the sim count hits 0"
        );
    }
}
