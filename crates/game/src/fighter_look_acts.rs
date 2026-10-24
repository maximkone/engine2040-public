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
use engine_arena::{GaitPhase, LiveMatch};
use engine_audio::FightCue;
use engine_combat::fight::{HITSTOP_TICKS, TABLES_V1, TABLES_V2};
use engine_combat::tables::TABLES_VERSION_V2;

use crate::anim_bridge::{BridgeClip, BridgeTuning, ClipPick, FighterSlot, Press};
use crate::impact_seq::{ImpactClass, ImpactTable};
use crate::pose_hold::hold_mask;
use crate::swing_time::swing_total;
use crate::FighterView;

use super::{
    angle_delta, run_phase_scale, Act, FighterLook, SlotAnim, FLASH_TICKS, IMPACT_TICKS,
    MAX_LOOK_FIGHTERS, MAX_TURN_PER_TICK, RUN_REF_M_PER_TICK, RUN_SPEED_M_PER_TICK,
    SHAKE_DECAY_PERMILLE_PER_TICK, TICK_HZ,
};

/// One tick's cues for the act fold: the outcome cues and, under rules v2, the
/// slots whose attack began this tick (`AttackStarted`, EG19c).
#[derive(Clone, Copy)]
struct TickCues<'a> {
    fight: &'a [FightCue],
    started: &'a [u8],
}

/// One idle bridge slot per look slot, ids `0..MAX_LOOK_FIGHTERS`. Cold path
/// only (load plus test constructors); the fold drives them in place.
pub(crate) fn bridge_slots() -> Vec<FighterSlot> {
    (0..MAX_LOOK_FIGHTERS)
        .map(|index| FighterSlot::new(u8::try_from(index).unwrap_or(u8::MAX)))
        .collect()
}

/// The ticks slot `slot`'s swing clip plays (FEEL1b): its armed weapon's
/// startup from the live match, or `default` when the slot is bare.
fn swing_ticks_for(live: Option<&LiveMatch>, slot: u8, default: u32) -> u32 {
    live.and_then(|live| live.weapon(slot))
        .map_or(default, |weapon| weapon.startup_ticks)
}

/// The press swing of `slot` (EG19c): ticks from the press to the blow (the
/// armed weapon's startup, or the rules' own for a bare slot) and the ticks the
/// swing pick lives, from the press. `clip_len` is the swing clip in ticks.
fn press_timing(live: Option<&LiveMatch>, slot: u8, clip_len: u32) -> Press {
    let (lead, recovery) = match live.and_then(|live| live.weapon(slot)) {
        Some(weapon) => (weapon.startup_ticks, weapon.recovery_ticks),
        None => live.map_or((IMPACT_TICKS, 0), |live| {
            let rows = if live.config().table_version == TABLES_VERSION_V2 {
                &TABLES_V2
            } else {
                &TABLES_V1
            };
            (rows.attack_startup_ticks, rows.attack_recovery_ticks)
        }),
    };
    let lead = lead.max(1);
    Press {
        lead,
        total: swing_total(lead, recovery, clip_len, IMPACT_TICKS),
    }
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
/// to this one) has it. An attack start on the outcome cue keeps the landed
/// `IMPACT_TICKS` backdate; one the fight cued on its press (EG19c, `attack`
/// carries the bridge's own start tick and the lead to the blow, lead > 0)
/// starts the clip on the press and times it to meet the blow
/// ([`crate::swing_time`]). Every other pick starts on its tick.
/// Presentation only.
fn follow_pick(
    slot: &mut SlotAnim,
    mapped: Act,
    fade: SlotFade,
    leaving_s: f32,
    aged: &SlotAnim,
    tick: u32,
    attack: (u32, u32),
) {
    slot.prev_clip = usize::from(fade.prev_clip);
    slot.fade_left = fade.fade_left;
    let (attack_start, lead) = attack;
    // A press tick restarts the clip even when the variant repeats.
    if mapped != slot.act || (lead > 0 && attack_start == tick) {
        slot.prev_time = leaving_s;
        slot.act = mapped;
        let attacking = matches!(mapped, Act::Attack(_));
        slot.act_start = if attacking { attack_start } else { tick };
        slot.swing_lead = if attacking { lead } else { 0 };
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
        self.observe_tick_inner(tick, views, hp, cues, &[], None);
    }

    /// [`FighterLook::observe_tick`] with the tick's press cues (EG19c):
    /// `started` names the slots whose attack the fight began this tick
    /// (`AttackStarted`, rules v2). Each starts its swing clip on this very
    /// tick, timed so the clip's impact frame meets the blow `lead` ticks later
    /// (the armed weapon's startup from `live`, the table's for a bare slot);
    /// the blow's outcome cue is then that swing's impact and starts nothing.
    /// With `started` empty this is exactly `observe_tick` (or
    /// `observe_tick_with_live` when `live` is given). The match is read for
    /// numbers only; the look never writes it or its hash.
    pub fn observe_tick_pressed(
        &mut self,
        tick: u32,
        views: &[FighterView],
        hp: &[u16],
        cues: &[FightCue],
        started: &[u8],
        live: Option<&LiveMatch>,
    ) {
        self.observe_tick_inner(tick, views, hp, cues, started, live);
    }

    /// [`FighterLook::observe_tick`] with the live match in hand (FEEL1b):
    /// each slot's swing plays for its own armed weapon's startup
    /// (`Sim::weapon`), so the pugio's clip ends nine ticks before the
    /// clava's. A bare slot keeps the look's default clip length. The match
    /// is read for numbers only; the look never writes it or its hash.
    pub fn observe_tick_with_live(
        &mut self,
        tick: u32,
        views: &[FighterView],
        hp: &[u16],
        cues: &[FightCue],
        live: &LiveMatch,
    ) {
        self.observe_tick_inner(tick, views, hp, cues, &[], Some(live));
    }

    fn observe_tick_inner(
        &mut self,
        tick: u32,
        views: &[FighterView],
        hp: &[u16],
        cues: &[FightCue],
        started: &[u8],
        live: Option<&LiveMatch>,
    ) {
        // EG39: a tick at or below the last folded one is a rollback resim.
        // Drop the fading correction if its window has passed, capture the
        // roots the viewer last saw, then restore the checkpoint through the
        // tick before the replay so the cues below fold exactly once. The
        // rewind is presentation only; the sim never calls this path.
        if let Some(correction) = self.correction {
            if correction.age(tick) >= engine_anim::net_pose::CORRECTION_FADE_TICKS {
                self.correction = None;
            }
        }
        let high = self.tick;
        let replaying = tick <= high && tick > 0 && self.checkpoint_through(tick - 1).is_some();
        if replaying {
            self.note_correction(high);
            self.rewind_to(tick);
        }
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
        self.fold_cues(tick, cues, replaying);
        self.fold_acts(
            tick,
            views,
            hp,
            TickCues {
                fight: cues,
                started,
            },
            elapsed,
            live,
        );
        self.fold_switches(&before, elapsed);
        // BG5: a run slot's phase crossed a foot plant this tick: hand the
        // audio feed the step on this exact tick. Presentation only.
        self.step_clip_events(tick, replaying);
        // LF12: step each slot's planted feet from the clip's end-of-tick
        // ankles, after the switch offsets so a plant reads the shown pose.
        self.fold_feet(elapsed);
        // View-only trail feed (LF14): age once, push attacking slots and
        // clear the rest. Presentation only, never the sim hash.
        self.feed_trails();
        if !replaying {
            // RZ7: lay and step each seen slot's cape on this tick's pose.
            // Never hashed; the sim never reads a cape. A resim holds the
            // cape at the high tick instead of re-stepping its float state.
            self.cape_tick();
            // Last stage: advance each seen slot's angular-speed cap onto this
            // tick's final pose, so no bone turns faster than 1080 deg/s on the
            // frames that read it. The limiter owns float state of its own, so
            // the resim holds it too. Presentation only, never hashed.
            self.limit_tick(elapsed);
        }
        self.save_checkpoint(tick);
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
    fn fold_cues(&mut self, tick: u32, cues: &[FightCue], replaying: bool) {
        for cue in cues {
            match *cue {
                FightCue::Hit {
                    attacker,
                    victim,
                    damage,
                } => {
                    self.start_swing(attacker, tick);
                    let push = Self::push_away(&self.slots, attacker, victim);
                    // The sim's freeze on this blow names its class: a heavy blow
                    // holds six ticks (`observe_hitstop` recorded it this tick).
                    let hold = self
                        .bridge
                        .get(usize::from(attacker))
                        .map_or(0, |bridge| bridge.hold_until().saturating_sub(tick));
                    let class = ImpactClass::of_hit(hold);
                    if self.in_reach(attacker, victim) {
                        if let Some(slot) = self.slots.get_mut(usize::from(victim)) {
                            slot.flash_until = tick + FLASH_TICKS;
                        }
                        // The camera takes the trauma the blow's class row
                        // authors (`impact.rows`): a heavy blow shakes it more.
                        self.add_shake_trauma(
                            ImpactTable::shipped().sequence(class, None).trauma_permille,
                        );
                    }
                    self.stumble_slot(victim);
                    self.remember_hit(victim, tick, push, class == ImpactClass::Heavy);
                    // FEEL9: a standing victim's upper chain takes the blow.
                    // A resim holds the ragdoll at the high tick, so re-noting
                    // the same cue would apply the blow twice.
                    if !replaying {
                        self.ragdoll_note_hit(attacker, victim, damage, cues);
                    }
                }
                FightCue::Knockdown { attacker, victim } => {
                    self.start_swing(attacker, tick);
                    if self.in_reach(attacker, victim) {
                        if let Some(slot) = self.slots.get_mut(usize::from(victim)) {
                            slot.flash_until = tick + FLASH_TICKS;
                        }
                        self.add_shake_trauma(
                            ImpactTable::shipped()
                                .sequence(ImpactClass::Knockdown, None)
                                .trauma_permille,
                        );
                    }
                    // RZ2w: the dead victim falls as a ragdoll, pushed along the blow. Noted
                    // before the floor verdict so it keeps the standing pose.
                    // The resim already holds that fall from the high tick.
                    if !replaying {
                        self.ragdoll_note_knockdown(attacker, victim, cues);
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

    /// Start the victim's additive flinch at `tick` along `push`, shaped by
    /// the reaction row for where the blow came from and how hard it landed
    /// (`hit-reactions.rows`). A new hit restarts the window; bad slots are
    /// ignored. Presentation only; the sim digest never reads this.
    fn remember_hit(&mut self, victim: u8, tick: u32, push: [f32; 2], heavy: bool) {
        let Some(slot) = self.slots.get_mut(usize::from(victim)) else {
            return;
        };
        slot.hit_start = tick;
        slot.hit_push = push;
        slot.hit_shape = crate::hit_reaction::ReactionTable::shipped().shape(
            crate::hit_reaction::Sector::of(slot.yaw, push),
            crate::hit_reaction::Weight::of(heavy, false),
        );
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
    /// the same views, hp and cues; sampling stays in `slot_pose`. A swing
    /// the fight cued on its press (`cues.started`, EG19c) starts on that tick;
    /// one that starts on its outcome cue keeps the landed `IMPACT_TICKS`
    /// backdate; every other pick starts on its tick.
    fn fold_acts(
        &mut self,
        tick: u32,
        views: &[FighterView],
        hp: &[u16],
        cues: TickCues<'_>,
        elapsed: u32,
        live: Option<&LiveMatch>,
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
        let drive = BridgeDrive {
            cues,
            live,
            tuning,
            default_swing: self.swing_ticks,
        };
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
            let rival = nearest_rival(&standing, index, old.pos);
            let step = (old.pos[0] - old.prev_pos[0], old.pos[2] - old.prev_pos[2]);
            let speed = (step.0 * step.0 + step.1 * step.1).sqrt();
            let hp_permille = hp.get(index).copied().unwrap_or(1);
            // Drive the bridge slot from the same tick inputs. A refused
            // tick (foreign slot, never here) keeps the old act.
            let Some((pick, fade, attack)) =
                self.drive_bridge(index, view, tick, hp_permille, &drive)
            else {
                continue;
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
            follow_pick(slot, mapped, fade, leaving.1, &aged, tick, attack);
        }
    }

    /// Drive one slot's bridge for the tick and read its pick back: the fight's
    /// press for this slot (EG19c) queues first, so the swing starts on its own
    /// tick; the armed startup times a swing that starts on its outcome cue
    /// (FEEL1b; bare keeps the default). Returns the pick, its fade and the
    /// swing start `(tick, lead)` the look follows: the bridge's own start and
    /// the lead to the blow for a press swing, the backdated outcome cue with
    /// no lead otherwise. `None` when the bridge refuses the tick (a foreign
    /// slot, never here), which keeps the old act.
    fn drive_bridge(
        &mut self,
        index: usize,
        view: &FighterView,
        tick: u32,
        hp_permille: u16,
        drive: &BridgeDrive<'_>,
    ) -> Option<(ClipPick, SlotFade, (u32, u32))> {
        let swing_ticks = swing_ticks_for(drive.live, view.slot, drive.default_swing);
        let press = drive
            .cues
            .started
            .contains(&view.slot)
            .then(|| press_timing(drive.live, view.slot, drive.default_swing));
        let bridge = self.bridge.get_mut(index)?;
        if let Some(press) = press {
            bridge.press(press);
        }
        bridge
            .observe_tick(
                tick,
                view,
                hp_permille,
                drive.cues.fight,
                &drive.tuning,
                swing_ticks,
            )
            .ok()?;
        let (pick, fade) = bridge.look_pick();
        let lead = bridge.press_lead(tick);
        let attack = if lead > 0 {
            (pick.act_start, lead)
        } else {
            (tick.saturating_sub(IMPACT_TICKS), 0)
        };
        Some((pick, fade, attack))
    }
}

/// What one tick's bridge drive reads besides the slot: the cues (outcome and
/// press), the match for weapon timing, the speed tuning and the look's
/// default swing length in ticks.
struct BridgeDrive<'a> {
    cues: TickCues<'a>,
    live: Option<&'a LiveMatch>,
    tuning: BridgeTuning,
    default_swing: u32,
}

/// Planar offset from `pos` to the nearest standing rival of slot `index`, or
/// `None` with nobody else standing.
fn nearest_rival(
    standing: &[(usize, [f32; 3])],
    index: usize,
    pos: [f32; 3],
) -> Option<(f32, f32)> {
    standing
        .iter()
        .filter(|(other, _)| *other != index)
        .map(|(_, at)| (at[0] - pos[0], at[2] - pos[2]))
        .min_by(|a, b| (a.0 * a.0 + a.1 * a.1).total_cmp(&(b.0 * b.0 + b.1 * b.1)))
}

#[cfg(test)]
mod tests {
    use super::super::{Act, ClipEventLane, FighterLook, SlotAnim, MAX_LOOK_FIGHTERS};
    use super::{bridge_slots, hold_frozen};
    use crate::weapon_trail::TrailRing;
    use crate::FighterView;
    use engine_arena::LiveMatch;
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
            clip_events: ClipEventLane::new(),
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
            checkpoints: super::super::empty_checkpoints(),
            rewind_cursor: 0,
            correction: None,
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

    /// EG39: a resim re-emits the same cue; the slot and the bridge must land
    /// on the act and clip clock of the no-resim run, and the hop that caused
    /// the correction must fade out instead of snapping. The F2P was the old
    /// fold counting the replay as a new swing (variant 1 instead of 0) and
    /// restarting the clock.
    #[test]
    fn resim_no_restart() {
        let views_at = |x: f32| {
            vec![
                FighterView {
                    slot: 0,
                    pos_m: [x, 0.0, 0.0],
                    yaw: 0.0,
                },
                FighterView {
                    slot: 1,
                    pos_m: [2.0, 0.0, 0.0],
                    yaw: 0.0,
                },
            ]
        };
        let hp = [1000u16; 2];
        let cues: &[FightCue] = &[hit(0, 1)];
        let empty: &[FightCue] = &[];

        // No-resim run: the corrected position from tick 4 on, one hit at 4.
        let mut clean = bare_look();
        for tick in 0..=8 {
            let x = if tick >= 4 { 0.5 } else { 0.0 };
            let cues = if tick == 4 { cues } else { empty };
            clean.observe_tick(tick, &views_at(x), &hp, cues);
        }

        // The old run, then the same four ticks replayed on top of it with the
        // corrected position.
        let mut replay = bare_look();
        for tick in 0..=8 {
            let cues = if tick == 4 { cues } else { empty };
            replay.observe_tick(tick, &views_at(0.0), &hp, cues);
        }
        for tick in 4..=8 {
            let cues = if tick == 4 { cues } else { empty };
            replay.observe_tick(tick, &views_at(0.5), &hp, cues);
        }

        assert_eq!(
            replay.slots[0].act, clean.slots[0].act,
            "the replayed swing keeps its variant"
        );
        assert_eq!(
            replay.slots[0].act_start, clean.slots[0].act_start,
            "the replayed slot keeps its clip clock"
        );
        assert_eq!(
            replay.bridge[0].pick(),
            clean.bridge[0].pick(),
            "the bridge pick matches the no-resim run"
        );

        // The 0.5 m hop is the visual error: full on the correction tick,
        // then linear to zero over CORRECTION_FADE_TICKS (12 ticks).
        let offset = replay.correction_offset_m(0);
        assert!(
            offset > 0.0,
            "the correction tick keeps a visual offset, got {offset}"
        );
        assert!(
            (offset - 0.5).abs() < 1e-3,
            "the offset is the hop the viewer saw, got {offset}"
        );
        assert_eq!(
            clean.correction_offset_m(0),
            0.0,
            "a run with no rewind has no offset"
        );

        // A second resim of the same window is idempotent: the ring still
        // holds the checkpoint before tick 4.
        let replayed = replay.bridge[0].pick();
        for tick in 4..=8 {
            let cues = if tick == 4 { cues } else { empty };
            replay.observe_tick(tick, &views_at(0.5), &hp, cues);
        }
        assert_eq!(
            replay.bridge[0].pick(),
            replayed,
            "a second resim keeps the same phase"
        );

        for tick in 9..=20 {
            replay.observe_tick(tick, &views_at(0.5), &hp, empty);
        }
        let offset = replay.correction_offset_m(0);
        assert!(
            offset < 0.01,
            "the offset is under 1 cm after 12 ticks, got {offset}"
        );
    }

    /// A rules v2 duel armed with `ids`, for the look's per-slot timing.
    fn armed_live(ids: &[&'static str]) -> LiveMatch {
        let config = engine_arena::ArenaConfig::try_new(11, 32, 2, 12.0).expect("config");
        engine_arena::MatchSpec::duel(config, engine_arena::world::MatchWorld::empty())
            .with_weapons(ids)
            .live()
            .expect("armed duel")
    }

    /// FEEL1b: each slot's swing plays for its own armed startup. The tick-0
    /// hit holds both clips three ticks (the shipped light hit-stop), so the
    /// pugio's 24-tick clip frees on tick 27 and the clava's 42-tick clip on
    /// tick 45: the club ends eighteen ticks after the dagger, as the rows say.
    #[test]
    fn feel1b_look_times_each_swing_by_its_weapon_startup() {
        let live = armed_live(&["pugio", "clava"]);
        assert_eq!(live.weapon(0).map(|w| w.startup_ticks), Some(24));
        assert_eq!(live.weapon(1).map(|w| w.startup_ticks), Some(42));
        let views = pair_views();
        let hp = [1000u16; 2];
        let mut look = bare_look();
        look.observe_tick_with_live(0, &views, &hp, &[hit(0, 1), hit(1, 0)], &live);
        assert!(matches!(look.slots[0].act, Act::Attack(_)));
        assert!(matches!(look.slots[1].act, Act::Attack(_)));
        let mut ends = [None; 2];
        for tick in 1..60 {
            look.observe_tick_with_live(tick, &views, &hp, &[], &live);
            for (slot, end) in ends.iter_mut().enumerate() {
                if end.is_none() && !matches!(look.slots[slot].act, Act::Attack(_)) {
                    *end = Some(tick);
                }
            }
        }
        assert_eq!(
            ends,
            [Some(27), Some(45)],
            "each startup ends its own clip after the hold"
        );
        assert_eq!(
            ends[1].unwrap() - ends[0].unwrap(),
            18,
            "the club lingers eighteen ticks"
        );
    }

    #[test]
    fn hitstop_hold_matches_sim_four_ticks() {
        // LF10: an impact at tick 10 must freeze exactly HITSTOP_TICKS (4)
        // ticks, matching the App hitstop count (4,3,2,1 then 0), not one
        // extra tick. The bridge expiry is exclusive, so impact tick plus
        // three quiet ticks hold; tick 14 runs free.
        let mut look = bare_look();
        look.fold_cues(10, &[hit(0, 1)], false);
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
            look.fold_cues(10, &[cue], false);
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

    /// The human's input for one tick of a chase: walk at the foe, face it,
    /// and swing once in reach (a fresh press every sixth tick).
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a bearing is within +-18000 centidegrees and a wish within +-100"
    )]
    fn chase_input(tick: u32, views: &[FighterView]) -> engine_input::FighterInput {
        use engine_input::fighter::ATTACK;
        use engine_input::{Buttons, FighterId, FighterInput};
        let slot = |index: u8| views.iter().find(|view| view.slot == index);
        let (Some(me), Some(foe)) = (slot(0), slot(1)) else {
            // The fighters appear after the first tick: stand still until then.
            return FighterInput {
                tick,
                fighter: FighterId::new(0).expect("slot 0"),
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: Buttons::NONE,
            };
        };
        let to = [foe.pos_m[0] - me.pos_m[0], foe.pos_m[2] - me.pos_m[2]];
        let dist = to[0].hypot(to[1]).max(1.0e-3);
        let near = dist <= 1.3;
        let wish = |component: f32| {
            if near {
                0
            } else {
                (component / dist * 100.0).round() as i8
            }
        };
        FighterInput {
            tick,
            fighter: FighterId::new(0).expect("slot 0"),
            mx: wish(to[0]),
            mz: wish(to[1]),
            facing: (to[0].atan2(to[1]).to_degrees() * 100.0).round() as i16,
            buttons: if near && tick.is_multiple_of(6) {
                Buttons(ATTACK)
            } else {
                Buttons::NONE
            },
        }
    }

    /// A look folded through a rules v2 duel (the gladius, startup 30) the way
    /// the window folds it: the human walks at the foe and swings in reach.
    /// Returns the press tick of the swing of slot 0 that lands first, the
    /// tick its blow lands, the look's slot-0 swing start on the tick before
    /// the blow and after the blow's own fold (the hold marches it a tick),
    /// and the lead the look timed it with.
    fn duel_swing() -> (u32, u32, u32, u32, u32) {
        let config = crate::GameConfig {
            seed: 7,
            ticks: 900,
            fighters: 2,
            half_extent_m: 12.0,
        };
        let mut app = crate::App::new(config)
            .expect("session")
            .into_duel()
            .expect("duel");
        let mut look = bare_look();
        look.swing_ticks = 40;
        let (mut press, mut seen) = (None, Vec::new());
        for tick in 0..config.ticks {
            let out = app.tick(chase_input(tick, app.fighter_views()));
            let before = look.slots[0].act_start;
            look.observe_hitstop(tick, &out.cues, app.hitstop_remaining());
            look.observe_tick_pressed(
                tick,
                app.fighter_views(),
                app.hp(),
                &out.cues,
                &out.started,
                Some(app.live_match()),
            );
            if out.started.contains(&0) {
                assert_eq!(
                    look.slots[0].act_start, tick,
                    "the clip starts on the press tick"
                );
                press = Some(tick);
            }
            let landed = out
                .cues
                .iter()
                .any(|cue| matches!(cue, FightCue::Hit { attacker: 0, .. }));
            if landed {
                // A blow of a swing the fight cued on its press (a swing that
                // starts without a cue falls back to the outcome-cue clip).
                match press.filter(|at| tick - at <= 40) {
                    Some(press) => {
                        return (
                            press,
                            tick,
                            before,
                            look.slots[0].act_start,
                            look.slots[0].swing_lead,
                        );
                    }
                    None => seen.push((tick, press)),
                }
            }
        }
        panic!(
            "no press-cued blow in {} ticks; blows (tick, last press): {seen:?}",
            config.ticks
        );
    }

    /// EG19c: under rules v2 the swing clip starts on the press tick, and its
    /// impact frame meets the blow. The F2P was the clip starting on the
    /// outcome cue, backdated twelve ticks: nothing moved for the whole
    /// startup, then the body popped to mid-swing on the hit.
    #[test]
    fn swing_on_press_meets_the_hit_tick() {
        let (press, blow, before, after, lead) = duel_swing();
        assert_eq!(
            lead, 30,
            "the gladius startup leads the blow: press {press}, blow {blow}, start {before}/{after}"
        );
        assert!(
            blow.abs_diff(press + lead) <= 2,
            "the blow lands the startup after the press: press {press}, blow {blow}"
        );
        assert_eq!(
            before, press,
            "the clip kept its start from the press to the blow"
        );
        assert_eq!(
            after,
            press + 1,
            "the blow's cue restarts nothing: the hold only marches the clock"
        );
        // The look's clip clock at the end of the blow tick (the pose a held
        // frame shows): within two ticks of the clip's impact frame.
        let since = f32::from(u16::try_from(blow - after).expect("ticks")) + 1.0;
        let phase = crate::swing_time::clip_ticks(since, lead, super::IMPACT_TICKS);
        let impact = f32::from(u16::try_from(super::IMPACT_TICKS).expect("ticks"));
        assert!(
            (phase - impact).abs() <= 2.0,
            "clip phase {phase} at the hit, impact frame {impact}"
        );
    }

    /// EG19c: the blow's outcome cue is the swing's impact; it neither
    /// restarts the clip nor picks another variant, and the press swing lives
    /// through its recovery, then hands the body back to idle.
    #[test]
    fn swing_on_press_the_blow_cue_starts_nothing() {
        let live = armed_live(&["gladius", "gladius"]);
        let gladius = live.weapon(0).expect("the gladius is armed");
        let (startup, recovery) = (gladius.startup_ticks, gladius.recovery_ticks);
        let views = pair_views();
        let hp = [1000u16; 2];
        let mut look = bare_look();
        look.swing_ticks = 40;
        look.observe_tick_pressed(0, &views, &hp, &[], &[0], Some(&live));
        let swung = look.bridge[0].pick();
        assert!(matches!(look.slots[0].act, Act::Attack(_)));
        assert_eq!(look.slots[0].act_start, 0, "the clip starts on the press");
        assert_eq!(look.slots[0].swing_lead, startup);
        assert!(
            !matches!(look.slots[1].act, Act::Attack(_)),
            "the foe stands"
        );
        let mut ended = None;
        for tick in 1..60 {
            let cues: &[FightCue] = if tick == startup { &[hit(0, 1)] } else { &[] };
            look.observe_tick_pressed(tick, &views, &hp, cues, &[], Some(&live));
            if ended.is_none() && !matches!(look.slots[0].act, Act::Attack(_)) {
                ended = Some(tick);
            }
            if ended.is_none() {
                assert_eq!(
                    look.bridge[0].pick().clip,
                    swung.clip,
                    "tick {tick}: one swing"
                );
                assert_eq!(
                    look.slots[0].act_start,
                    // The impact pair holds (no live count fed: the V1 four),
                    // and the clip clock marches with the hold.
                    if tick < startup {
                        0
                    } else {
                        (tick - (startup - 1)).min(HITSTOP_TICKS)
                    },
                    "tick {tick}: the clip keeps its start"
                );
            }
        }
        // 30 to the blow, 6 recovery, 4 tail, plus the blow's own hold.
        assert_eq!(
            ended,
            Some(startup + recovery + 4 + HITSTOP_TICKS),
            "the swing outlives the blow by its recovery"
        );
        // The foe took the hit at the blow tick; its flinch onset marches with
        // the hold, so the lean starts on the first free tick.
        assert_eq!(look.slots[1].hit_start, startup + HITSTOP_TICKS - 1);
    }

    #[test]
    fn swing_on_press_whiffs_show_and_v1_keeps_the_outcome_swing() {
        let live = armed_live(&["gladius", "gladius"]);
        let views = pair_views();
        let hp = [1000u16; 2];
        let mut whiff = bare_look();
        whiff.swing_ticks = 40;
        whiff.observe_tick_pressed(0, &views, &hp, &[], &[1], Some(&live));
        assert!(
            matches!(whiff.slots[1].act, Act::Attack(_)),
            "a whiff swings"
        );
        assert!(!matches!(whiff.slots[0].act, Act::Attack(_)));
        for tick in 1..60 {
            whiff.observe_tick_pressed(tick, &views, &hp, &[], &[], Some(&live));
        }
        assert!(!matches!(whiff.slots[1].act, Act::Attack(_)), "and it ends");
        let mut v1 = bare_look();
        for tick in 0..20 {
            v1.observe_tick_pressed(tick, &views, &hp, &[], &[], Some(&live));
        }
        v1.observe_tick_pressed(20, &views, &hp, &[hit(0, 1)], &[], Some(&live));
        assert!(matches!(v1.slots[0].act, Act::Attack(_)));
        assert_eq!(
            // The cue tick holds the pair, which marches the clip clock one tick.
            v1.slots[0].act_start,
            20 - super::IMPACT_TICKS + 1,
            "the old backdate"
        );
        assert_eq!(v1.slots[0].swing_lead, 0, "no lead: the clip plays 1:1");
    }

    #[test]
    fn hitstop_hold_without_a_live_count_keeps_the_v1_freeze() {
        // No live count fed (the old callers): the impact pair holds the V1 four.
        let mut look = bare_look();
        look.fold_cues(10, &[hit(0, 1)], false);
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
