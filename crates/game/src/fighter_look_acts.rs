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

/// Hold one slot's pose for a hitstop tick (LF10): keep the act and march the
/// clip clock and the flinch clock with the tick, so neither phase moves. The
/// first held tick takes the blend the bridge set on the impact tick; later
/// held ticks keep the blend exactly as the previous tick left it, whatever
/// the bridge says, so the pose is the same on every held tick. The bridge
/// clock marches too, so the resume tick still maps the held act, not an
/// expired swing. Death is decided by the caller: a `Down` mapping never
/// reaches here.
fn freeze_slot(
    slot: &mut SlotAnim,
    old: &SlotAnim,
    fade: SlotFade,
    fade_before: u32,
    tick: u32,
    elapsed: u32,
    bridge_slot: &mut FighterSlot,
) {
    if old.held {
        slot.fade_left = fade_before;
    } else {
        slot.prev_clip = usize::from(fade.prev_clip);
        slot.prev_time = bridge_prev_time(fade.prev_clip, fade.prev_phase_ticks, old.run_speed);
        slot.fade_left = fade.fade_left;
    }
    slot.act_start = slot.act_start.saturating_add(elapsed);
    // The impact tick starts the flinch, so only an older flinch marches on it.
    if slot.hit_start < tick {
        slot.hit_start = slot.hit_start.saturating_add(elapsed);
    }
    slot.held = true;
    bridge_slot.march_hold(elapsed);
}

/// Follow the bridge pick on a free tick: a new act starts its clip and its
/// blend from the frozen leaving clip, with a run phase of zero; the same act
/// (including a restarted swing via `start_swing`) carries the bridge blend
/// plus the frozen leaving clip, so the switch frame still shows the old clip
/// whole, and keeps its run phase as `aged` (the slot's last tick marched on
/// to this one) has it. Attack starts keep the landed `IMPACT_TICKS`
/// backdate; every other pick starts on its tick. Presentation only.
fn follow_pick(
    slot: &mut SlotAnim,
    mapped: Act,
    fade: SlotFade,
    leaving_s: f32,
    aged: &SlotAnim,
    tick: u32,
) {
    slot.prev_clip = usize::from(fade.prev_clip);
    slot.fade_left = fade.fade_left;
    if mapped != slot.act {
        slot.prev_time = leaving_s;
        slot.act = mapped;
        slot.act_start = match mapped {
            Act::Attack(_) => tick.saturating_sub(IMPACT_TICKS),
            _ => tick,
        };
        slot.run_phase_s = 0.0;
    } else {
        slot.prev_time = bridge_prev_time(fade.prev_clip, fade.prev_phase_ticks, aged.run_speed);
        slot.run_phase_s = aged.run_phase_s;
    }
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
/// for the run clip through [`run_phase_scale`] at the recorded speed. The
/// bridge counts ticks, not rates, so this mirror stays a whole-phase guess
/// that the pose never reads. Pure.
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
        // Each slot as the last tick left it, for the clip-switch offsets
        // (LF9): the cue fold below may already restart a swing.
        let before: [SlotAnim; MAX_LOOK_FIGHTERS] =
            std::array::from_fn(|index| self.slots.get(index).copied().unwrap_or(SlotAnim::new()));
        self.shake_trauma_permille = self
            .shake_trauma_permille
            .saturating_sub(elapsed.saturating_mul(SHAKE_DECAY_PERMILLE_PER_TICK));
        self.tick = tick;
        self.fold_positions(views);
        self.fold_cues(tick, cues);
        self.fold_acts(tick, views, hp, cues, elapsed);
        self.fold_switches(&before, elapsed);
        // LF12: step each slot's planted feet from the clip's end-of-tick
        // ankles, after the switch offsets so a plant reads the shown pose.
        self.fold_feet(elapsed);
        // View-only trail feed (LF14): age once, push attacking slots and
        // clear the rest. Presentation only, never the sim hash.
        self.feed_trails();
        // RZ7: lay and step each seen slot's cape on this tick's pose. Never
        // hashed; the sim never reads a cape.
        self.cape_tick();
        // Last stage: advance each seen slot's angular-speed cap onto this
        // tick's final pose, so no bone turns faster than 1080 deg/s on the
        // frames that read it. Presentation only, never hashed.
        self.limit_tick(elapsed);
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
                    // RZ2w: the dead victim falls as a ragdoll, pushed along the blow. Noted
                    // before the floor verdict so it keeps the standing pose.
                    self.ragdoll_note_knockdown(attacker, victim, cues);
                    self.floor_slot(victim);
                }
                FightCue::Blocked { attacker, .. }
                | FightCue::Parried { attacker, .. }
                | FightCue::Dodged { attacker, .. } => self.start_swing(attacker, tick),
                FightCue::StaminaOut { .. } => {}
            }
        }
        // LF10 hitstop hold: the impact pair's clip clocks freeze while the
        // sim holds. The live count (`observe_hitstop`, FEEL3) has already
        // recorded the pair's expiry for this tick; without one, the V1 freeze
        // holds (exclusive end, impact tick plus four). `fold_acts` freezes
        // exactly that pair. Presentation only; the sim hash never reads this.
        if let Some((attacker, victim)) = cue_impact_slots(cues) {
            let until = tick.saturating_add(HITSTOP_TICKS);
            for index in [attacker, victim] {
                if let Some(slot) = self.bridge.get_mut(usize::from(index)) {
                    if slot.hold_until() <= tick {
                        slot.hold_through(until);
                    }
                }
            }
        }
    }

    /// Record the live fight's freeze for this tick's impact pair (FEEL3): the
    /// sim's hit-stop count for the strike that landed, read from the App
    /// (`App::hitstop_remaining`) after the tick ran. Call it before
    /// [`observe_tick`](Self::observe_tick) for the same tick, so the pair holds
    /// exactly `remaining` ticks (3 light, 6 heavy, 8 parry under v2). A tick
    /// with no impact pair, or a zero count, records nothing. Presentation only.
    pub fn observe_hitstop(&mut self, tick: u32, cues: &[FightCue], remaining: u32) {
        if remaining == 0 {
            return;
        }
        let Some((attacker, victim)) = cue_impact_slots(cues) else {
            return;
        };
        let until = tick.saturating_add(remaining);
        for index in [attacker, victim] {
            if let Some(slot) = self.bridge.get_mut(usize::from(index)) {
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
        if len2 <= 1e-12 {
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

    /// Clip plus frozen time for the act `leaving` is being left from, at the
    /// switch tick with `alpha` 0; `leaving` is the slot's last tick with its
    /// run phase marched on to this one. Presentation only; the sim hash
    /// never reads it.
    fn prev_clip_time(&self, slot_index: usize, leaving: &SlotAnim) -> (usize, f32) {
        if let Some(rig) = self.rigs.get(self.rig_for_slot(slot_index)) {
            self.clip_time(rig, leaving, 0.0)
        } else {
            let clip = match leaving.act {
                Act::Idle => 0,
                Act::Run => 1,
                Act::Attack(variant) => 2 + usize::from(variant.min(2)),
                Act::Down => 5,
            };
            let since = f32::from(
                u16::try_from(self.tick.saturating_sub(leaving.act_start)).unwrap_or(u16::MAX),
            );
            let time = match leaving.act {
                Act::Run => leaving.run_clip_s(0.0),
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
        let fades_before: [u32; MAX_LOOK_FIGHTERS] =
            std::array::from_fn(|index| self.slots.get(index).map_or(0, |slot| slot.fade_left));
        for slot in &mut self.slots {
            slot.fade_left = slot.fade_left.saturating_sub(1);
            // The switch-offset clock runs with the clip clock: a slot held
            // last tick keeps its age (LF9).
            if !slot.held {
                slot.iz_age = slot.iz_age.saturating_add(elapsed);
            }
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
            // The slot as its last tick left it, run phase marched on to this
            // tick at the rate the last frame of that tick played at, so the
            // phase meets the frame it follows (a held tick never reads it).
            // Then the frozen leaving clip at the switch tick, read before any
            // overwrite; the immutable borrow ends before the mutable update
            // below.
            let mut aged = old;
            aged.march_run_phase(elapsed);
            let leaving = self.prev_clip_time(index, &aged);
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
            let held = frozen.get(index).copied().unwrap_or(false) && mapped != Act::Down;
            if held {
                // Hitstop hold: the facing and the run speed stand still too,
                // so the whole pose is the impact tick's. Death still lands
                // at once: a `Down` mapping is never held.
                slot.prev_yaw = slot.yaw;
                let Some(bridge_slot) = self.bridge.get_mut(index) else {
                    continue;
                };
                freeze_slot(
                    slot,
                    &old,
                    fade,
                    fades_before.get(index).copied().unwrap_or(0),
                    tick,
                    elapsed,
                    bridge_slot,
                );
                continue;
            }
            slot.held = false;
            // Face the nearest standing rival; with none, face the motion.
            face_rival(slot, mapped, rival, step, speed);
            follow_pick(slot, mapped, fade, leaving.1, &aged, tick);
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
            inertia: Vec::new(),
            limit: Vec::new(),
            ragdoll: crate::fighter_look::ragdoll_look::RagdollLook::empty(),
            limbs: crate::fighter_look::limb_look::LimbLook::empty(),
            capes: crate::fighter_look::Capes::empty(),
            hero: 0,
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

    #[test]
    fn hitstop_hold_follows_the_live_count() {
        // FEEL3: the look holds the freeze of the strike that landed, the sim's
        // own count: a light 3, a heavy 6 and a parry 8 (v2 rows). The live
        // count is fed for the tick before the cue fold, as the window does.
        use engine_combat::depth::{INPUT_HEAVY, INPUT_LIGHT};
        use engine_combat::fight::{hitstop_for_impact, TABLES_V2};
        let light = hitstop_for_impact(&TABLES_V2, INPUT_LIGHT, false);
        let heavy = hitstop_for_impact(&TABLES_V2, INPUT_HEAVY, false);
        let parry = hitstop_for_impact(&TABLES_V2, INPUT_LIGHT, true);
        assert_eq!([light, heavy, parry], [3, 6, 8], "the sim's v2 rows");
        let parried = FightCue::Parried {
            attacker: 0,
            victim: 1,
        };
        let views = pair_views();
        for (cue, count) in [(hit(0, 1), light), (hit(0, 1), heavy), (parried, parry)] {
            let mut look = bare_look();
            look.observe_hitstop(10, &[cue], count);
            look.fold_cues(10, &[cue]);
            let held: Vec<u32> = (10..30)
                .filter(|tick| {
                    let frozen = hold_frozen(&look.bridge, *tick, &views);
                    frozen[0] && frozen[1]
                })
                .collect();
            assert_eq!(
                held,
                (10..10 + count).collect::<Vec<u32>>(),
                "a count of {count} holds exactly {count} ticks"
            );
        }
    }

    #[test]
    fn hitstop_hold_without_a_live_count_keeps_the_v1_freeze() {
        // No live count fed (the old callers): the impact pair holds the V1 four.
        let mut look = bare_look();
        look.fold_cues(10, &[hit(0, 1)]);
        assert_eq!(look.bridge[0].hold_until(), 10 + HITSTOP_TICKS);
        // A zero live count records nothing, so the fold falls back to the V1 freeze.
        let mut zero = bare_look();
        zero.observe_hitstop(10, &[hit(0, 1)], 0);
        assert_eq!(
            zero.bridge[0].hold_until(),
            0,
            "a zero count records nothing"
        );
    }
}
