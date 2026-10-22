//! authority: the step rules the legacy step and the spine step share.
//!
//! Owner: engine-arena. One match runs two ways: the legacy [`Sim`](crate::Sim)
//! and the spine step ([`crate::spine_step::SpineMatch`]). Both own the same
//! authority, so both call the same rules from this module: the height
//! channel's gravity, the integer balance, the cut resolver and the match
//! observer row. The bytes are the legacy's, so the golden path stays
//! frozen and the spine path matches it tick for tick.
//!
//! Units: meters, radians, millimetres, fixed ticks at 60 Hz.
//! Determinism: integer math plus the legacy floats, no wall clock, no
//! hash maps. Errors: a rule that cannot run returns `None` (or an empty
//! cut list) instead of panicking.

use engine_combat::exchange::{ExchangeEvent, ExchangeIntent};
use engine_combat::fight::FightState;
use engine_combat::heft::{self, Heft};
use engine_combat::sweep::MAX_SWEEPS_PER_TICK;
use engine_combat::{
    plate_struck, resolve_swing, AnatomyTable, BladeSwing, Cut, FightEvent, CUT_V1,
    MAX_CUTS_PER_SWING, MAX_ENERGY_Q16, MAX_SHARPNESS_Q16,
};
use engine_input::fighter::{ATTACK, DODGE, JUMP};
use engine_physics::balance::{self, BalanceState, Knockback};
#[cfg(feature = "sim")]
use engine_physics::controller::Controller;
use engine_physics::limb::LimbPool;
use engine_physics::stagger_memory::StaggerMemory;

use crate::armour::DeflectCue;
use crate::match_observer::TickMatchEvents;
use crate::match_wire;
use crate::{
    rest_targets, ArenaRules, FighterInput, FighterState, AIR_APEX_MM_TICK, BALANCE_BLOCK_Q8,
    BALANCE_FALL_Q8, BALANCE_HIT_Q8, BALANCE_PARRY_Q8, FIGHTER_GRAVITY, FIGHT_RULES_V1, MAX_SLOTS,
};

/// One tick of the height channel (row SG5): gravity on every slot that is
/// off the floor, a landing at floor height with the vertical speed cleared.
/// Integer mm, so a replay lands on the same height twice. Near the apex
/// (`AIR_APEX_MM_TICK`) a held jump halves gravity, the AIN6 table's
/// float-free copy; the spear swing keeps the plain arc.
///
/// Shared by the legacy step and the spine step so both tracks land on the
/// same millimetre.
pub(crate) fn fall_states(
    states: &mut [FighterState],
    rise: &mut [i32],
    fighters: usize,
    last_inputs: &[Option<FighterInput>; MAX_SLOTS],
) {
    for (slot, state) in states.iter_mut().enumerate().take(fighters) {
        let at_mm = match_wire::pos_mm(state.pos)[1];
        if rise[slot] == 0 && at_mm <= 0 {
            continue;
        }
        // The apex hang belongs to the live jump only: a held jump button
        // near the top halves gravity. Other height users (the spear rope
        // swing) keep the plain arc, so their receipts stay exact.
        let held_jump = last_inputs[slot].is_some_and(|input| input.buttons.0 & JUMP != 0);
        let gravity = if held_jump && rise[slot].abs() < AIR_APEX_MM_TICK {
            FIGHTER_GRAVITY / 2
        } else {
            FIGHTER_GRAVITY
        };
        let drop = rise[slot].saturating_sub(gravity);
        let next = at_mm.saturating_add(drop);
        if next <= 0 {
            rise[slot] = 0;
            state.pos[1] = 0.0;
        } else {
            rise[slot] = drop;
            state.pos[1] = match_wire::metres(next);
        }
    }
}

/// EG34: feed one tick's combat outcomes into the integer balance, in event
/// order: a hit stumbles the victim (a fall when it lands inside an open
/// stagger window, so a combo floors), a block chips the stance, a parry
/// staggers the attacker and a knockdown falls the victim. EG46: each landed
/// blow also feeds that slot's [`StaggerMemory`] on bone 0 through
/// [`BalanceState::apply_memory_hit`]. `None` only on an out-of-range impulse
/// (unreachable: the table stays inside the ceiling).
///
/// FEEL2: a landed blow from an armed swinger moves the body it lands on.
/// The hit impulse comes from the weapon's momentum (mass times the swing's
/// blade travel, [`engine_combat::heft::hit_impulse_q8`]), not the constant
/// [`BALANCE_HIT_Q8`], and the victim takes the matching knockback velocity
/// along the attacker-to-victim direction. An unarmed blow keeps the table
/// row's constant and launches nothing, so a match that never arms a weapon
/// steps exactly as before.
///
/// Shared by the legacy step and the spine step.
pub(crate) fn feed_balance(
    balance: &mut [BalanceState],
    stagger: &mut [StaggerMemory],
    knockback: &mut [Knockback],
    states: &[FighterState],
    fight: &FightState,
    rules: &ArenaRules,
    events: &[FightEvent],
) -> Option<()> {
    for event in events {
        let (slot, impulse) = match *event {
            FightEvent::Hit {
                attacker, victim, ..
            } => {
                let staggered = balance
                    .get(usize::from(victim))
                    .is_some_and(BalanceState::staggered);
                // An armed swinger's blow carries its weapon's momentum; an
                // unarmed blow keeps the table row's constant.
                let armed = blow_impulse(fight, attacker);
                let impulse = match armed {
                    Some(impulse) if !staggered => impulse,
                    _ if staggered => BALANCE_FALL_Q8,
                    _ => BALANCE_HIT_Q8,
                };
                if let Some(impulse) = armed {
                    let index = usize::from(victim);
                    if index != usize::from(attacker) {
                        launch_knockback(knockback, states, rules, index, impulse, attacker);
                    }
                }
                (victim, impulse)
            }
            FightEvent::Blocked { victim, .. } => (victim, BALANCE_BLOCK_Q8),
            FightEvent::Parried { attacker, .. } => (attacker, BALANCE_PARRY_Q8),
            FightEvent::Knockdown { victim, .. } => (victim, BALANCE_FALL_Q8),
            FightEvent::Dodged { .. } | FightEvent::StaminaOut { .. } => continue,
        };
        let index = usize::from(slot);
        if let (Some(state), Some(memory)) = (balance.get_mut(index), stagger.get_mut(index)) {
            state.apply_memory_hit(memory, impulse, 0).ok()?;
        }
    }
    Some(())
}

/// The hit impulse the attacker's weapon carries, or `None` for an unarmed
/// blow (the table row's constant) or a slot out of the match.
fn blow_impulse(fight: &FightState, attacker: u8) -> Option<u16> {
    let heft: Heft = fight.fighter(attacker)?.heft?;
    let impulse = heft::hit_impulse_q8(Some(heft), swing_travel_mm(heft.startup_ticks));
    (impulse > 0).then_some(impulse)
}

/// The blade's travel in one tick of the swing, millimetres: the strike
/// pose's span (reach minus base) over the weapon's startup ticks. The
/// heavier weapons start later, so each covers less ground per tick; the
/// momentum is the product, and it still grows with the weight.
fn swing_travel_mm(startup_ticks: u32) -> u32 {
    let span = u32::try_from(match_wire::BLADE_REACH_MM - match_wire::BLADE_BASE_MM).unwrap_or(0);
    span / startup_ticks.max(1)
}

/// Launch the victim's knockback along the attacker-to-victim direction, at
/// the speed the impulse scales to. A zero or unmeasurable direction (the
/// two bodies on one point) leaves the victim exactly where it stood, and a
/// match whose own collision pass never runs (the `sim` leg, whose
/// controllers own the poses) never launches one.
fn launch_knockback(
    knockback: &mut [Knockback],
    states: &[FighterState],
    rules: &ArenaRules,
    index: usize,
    impulse_q8: u16,
    attacker: u8,
) {
    if !rules.collides() {
        return;
    }
    let (Some(from), Some(to)) = (states.get(usize::from(attacker)), states.get(index)) else {
        return;
    };
    let dir = direction_q10(match_wire::pos_mm(from.pos), match_wire::pos_mm(to.pos));
    if dir == [0, 0] {
        return;
    }
    if let Some(carrier) = knockback.get_mut(index) {
        carrier.launch(balance::knockback_velocity_q10(impulse_q8, dir));
    }
}

/// The planar unit direction from `from` to `to`, in Q10 (1024 is one):
/// integer only, `isqrt` for the length, truncation toward zero so a
/// mirrored pair gives a mirrored direction.
fn direction_q10(from: [i32; 3], to: [i32; 3]) -> [i32; 2] {
    let dx = i64::from(to[0]) - i64::from(from[0]);
    let dz = i64::from(to[2]) - i64::from(from[2]);
    let len2 = u64::try_from(dx * dx + dz * dz).unwrap_or(0);
    let len = i64::try_from(len2.isqrt()).unwrap_or(0);
    if len <= 0 {
        return [0, 0];
    }
    let q10 = |delta: i64| i32::try_from(delta * 1024 / len).unwrap_or(0);
    [q10(dx), q10(dz)]
}

/// Cut resolve for a track that wears no armour (the spine step): the same
/// cuts as [`resolve_cuts`], with the plate cues dropped.
pub(crate) fn resolve_cuts_bare(
    states: &[FighterState],
    tick: u32,
    limbs: &mut LimbPool,
    events: &[FightEvent],
) -> Vec<Cut> {
    resolve_cuts(states, tick, limbs, events, &mut Vec::new())
}

/// Resolve one tick's landed hits into severed joints. V2 only: the cut event
/// names the joint and the limb pool carries it into the hash; the blade is
/// the attacker's mapped sweep at torso height, and the victim's capsules use
/// the standing rest layout at its feet. Presentation owns the pose and
/// blood; this path owns the integer cut.
///
/// FEEL4: the victim's worn bits (`FighterState::worn`) reach the resolver.
/// A blade whose edge reaches a plated joint is stopped by the plate and
/// pushes one [`DeflectCue`] into `deflects` instead of cutting; any other
/// blow resolves with the worn bits as the joint guard. Unworn victims
/// resolve exactly as before.
///
/// Shared by the legacy step and the spine step; `states` holds exactly the
/// active slots.
pub(crate) fn resolve_cuts(
    states: &[FighterState],
    tick: u32,
    limbs: &mut LimbPool,
    events: &[FightEvent],
    deflects: &mut Vec<DeflectCue>,
) -> Vec<Cut> {
    let anatomy = AnatomyTable::rome_kit();
    let fighters = states.len();
    let mut cuts = Vec::new();
    for event in events {
        let FightEvent::Hit {
            attacker, victim, ..
        } = *event
        else {
            continue;
        };
        let (a, v) = (usize::from(attacker), usize::from(victim));
        if a >= fighters || v >= fighters {
            continue;
        }
        let from = match_wire::pos_mm(states[a].pos);
        let to = match_wire::pos_mm(states[v].pos);
        let facing = ((to[0] - from[0]).signum(), (to[2] - from[2]).signum());
        let sweep = match_wire::sweep_for_attacker(attacker, from, facing);
        let blade = BladeSwing {
            attacker,
            victim,
            swing: tick.wrapping_mul(8).wrapping_add(u32::from(attacker)),
            p0_mm: [sweep.p0_mm[0], 1_000, sweep.p0_mm[2]],
            p1_mm: [sweep.p1_mm[0], 1_000, sweep.p1_mm[2]],
            blade_half_mm: sweep.blade_half_mm,
            // A landed blade is a full-force cut for the first authority
            // slice; damage scaling and edge wear land with the feel pass.
            energy_q16: MAX_ENERGY_Q16,
            sharpness_q16: MAX_SHARPNESS_Q16,
            wear_q16: 0,
        };
        let targets = rest_targets(to);
        let worn = states[v].worn;
        if let Some(joint) = plate_struck(&blade, &targets, &anatomy, worn, &CUT_V1) {
            deflects.push(DeflectCue {
                attacker,
                victim,
                joint: joint.as_u8(),
            });
            continue;
        }
        let mut out = [Cut {
            swing: 0,
            attacker: 0,
            victim: 0,
            joint: 1,
            flags: 0,
            energy_q16: 0,
            protection_q16: 0,
        }; MAX_CUTS_PER_SWING];
        let count = resolve_swing(&blade, &targets, &anatomy, worn, &CUT_V1, &mut out);
        if count > 0 {
            cuts.push(out[0]);
        }
    }
    if !cuts.is_empty() && crate::limbs::spawn_limbs(limbs, &cuts, 0).is_err() {
        cuts.clear();
    }
    cuts
}

/// Match observer call-site for one tick: the track honors its version flag
/// (V0 legacy intakes, V1 phased gates); authority stays on `try_step_into`,
/// so these bytes join the owned tick log only and never the checkpoint hash.
/// All scratch lives in fixed stack/owned buffers; the owned tick rows clone
/// exact-size on event ticks only, so quiet ticks move nothing.
///
/// Shared by the legacy step and the spine step; `states` holds exactly the
/// active slots. The observer path owns one row per tick (two small
/// allocations on an event tick), exactly as the legacy observer always did.
pub(crate) fn match_observer_row(
    track: &mut match_wire::MatchTrack,
    states: &[FighterState],
    row: &[FighterInput; MAX_SLOTS],
    fighters: usize,
    tick: u32,
) -> TickMatchEvents {
    let mut intents = [ExchangeIntent::Idle; MAX_SLOTS];
    for (slot, input) in row.iter().enumerate().take(fighters) {
        intents[slot] = match_wire::exchange_intent(input);
    }
    let mut poses = [match_wire::sweep_fighter(0, &states[0], &row[0]); MAX_SLOTS];
    for (slot, input) in row.iter().enumerate().take(fighters) {
        poses[slot] = match_wire::sweep_fighter(slot_id(slot), &states[slot], input);
    }
    let mut sweeps = [match_wire::sweep_for_attacker(0, [0, 0, 0], (0, 1)); MAX_SWEEPS_PER_TICK];
    let mut sweep_n: usize = 0;
    for (slot, input) in row.iter().enumerate().take(fighters) {
        if input.buttons.0 & ATTACK != 0 && sweep_n < MAX_SWEEPS_PER_TICK {
            let facing = match_wire::facing_vector(input.mx, input.mz, input.facing);
            let origin = match_wire::pos_mm(states[slot].pos);
            sweeps[sweep_n] = match_wire::sweep_for_attacker(slot_id(slot), origin, facing);
            sweep_n += 1;
        }
    }
    track.resolve(&intents, &poses[..fighters], &sweeps[..sweep_n]);
    // Locked riposte: a dodge-bit press while this slot holds an open parry
    // window attempts the riposte against the lowest-ID foe through the match
    // state; denials are silently skipped. The call is pure resolver work (no
    // heap); only a landed riposte grows the tick log below, never a quiet
    // tick.
    let mut landed = [ExchangeEvent::Parried {
        attacker: 0,
        victim: 0,
    }; MAX_SLOTS];
    let mut landed_n: usize = 0;
    for (slot, input) in row.iter().enumerate().take(fighters) {
        if input.buttons.0 & DODGE != 0 {
            let victim = u8::from(slot == 0);
            if let Ok(event) = track.riposte(slot_id(slot), victim) {
                landed[landed_n] = event;
                landed_n += 1;
            }
        }
    }
    // Single-alloc staging for the owned tick row: the track slices plus
    // landed ripostes size both rows exactly, so an event tick allocates once
    // per row (no regrow past the old exact-size `to_vec` buffer) and quiet
    // ticks move nothing. The track keeps its buffers for the next tick.
    let track_exchange = track.exchange_events();
    let track_sweep = track.sweep_events();
    let mut exchange = Vec::with_capacity(track_exchange.len() + landed_n);
    exchange.extend_from_slice(track_exchange);
    exchange.extend_from_slice(&landed[..landed_n]);
    let mut sweep = Vec::with_capacity(track_sweep.len());
    sweep.extend_from_slice(track_sweep);
    TickMatchEvents {
        tick,
        exchange,
        sweep,
    }
}

/// The physics controllers' hash fold: one FNV-1a word per controller, in
/// slot order. The `sim` tag byte keeps the default and `sim` checkpoints
/// apart; this fold rides it.
///
/// Shared by the legacy step and the spine step so both fold the same bytes.
#[cfg(feature = "sim")]
pub(crate) fn controller_hash(controllers: &[Controller]) -> u64 {
    let mut hash: u64 = engine_core::hash::FNV64_OFFSET;
    for controller in controllers {
        hash ^= controller.state_hash();
        hash = hash.wrapping_mul(crate::HASH_PRIME);
    }
    hash
}

/// Fighter slot as the `u8` id the contracts carry (slots stay below 32).
fn slot_id(slot: usize) -> u8 {
    u8::try_from(slot).unwrap_or(u8::MAX)
}

/// Checkpoint tag byte for the momentum knockback carrier (FEEL2).
const KNOCKBACK_TAG: u8 = 0xB6;

/// Fold the rules into a v1 checkpoint hash: version, the active slots'
/// team ids, then (when collision runs) the collision tag byte `0xC0`
/// plus each pillar footprint, then (when balance is wired) the balance
/// tag byte `0xB1` plus every slot's [`BalanceState::state_hash`], then
/// the stagger tag byte `0xB2` plus every slot's
/// [`StaggerMemory::state_hash`], then the limb tag byte `0xB3` plus
/// [`LimbPool::state_hash`], and last (FEEL2, still under balance) the
/// knockback tag byte `0xB6` plus every slot's
/// [`Knockback::state_hash`] — but only when at least one slot carries
/// knockback, so a match that never lands an armed blow leaves the frozen
/// bytes exactly as they were. Plain v1 returns the hash untouched
/// (golden-frozen bytes).
///
/// Shared by the legacy step and the spine step so both tracks fold the
/// same bytes; the bytes are the legacy `ArenaRules::fold`'s, moved here so
/// the two tracks cannot drift.
pub(crate) fn fold_rules(
    hash: u64,
    rules: &ArenaRules,
    fighters: usize,
    balance: &[BalanceState],
    stagger: &[StaggerMemory],
    limbs: &LimbPool,
    knockback: &[Knockback],
) -> u64 {
    if rules.version == FIGHT_RULES_V1 && !rules.collides() && !rules.balance {
        return hash;
    }
    let mut digest = engine_core::hash::Fnv64::with_state(hash);
    digest.write_u32(rules.version);
    digest.write(rules.teams.get(..fighters).unwrap_or(&[]));
    if rules.collides() {
        digest.write_u8(0xC0);
        for pillar in rules.pillars.iter().flatten() {
            for lane in pillar.min_mm.iter().chain(&pillar.max_mm) {
                digest.write_i32(*lane);
            }
        }
    }
    if rules.balance {
        digest.write_u8(0xB1);
        for state in balance.iter().take(fighters) {
            digest.write_u64(state.state_hash());
        }
        digest.write_u8(0xB2);
        for memory in stagger.iter().take(fighters) {
            digest.write_u64(memory.state_hash());
        }
        digest.write_u8(0xB3);
        digest.write_u64(limbs.state_hash());
        // FEEL2: the knockback carrier folds only while a body still slides,
        // so an unarmed (or already-settled) match folds no new bytes.
        if knockback
            .iter()
            .take(fighters)
            .any(|carrier| !carrier.at_rest())
        {
            digest.write_u8(KNOCKBACK_TAG);
            for carrier in knockback.iter().take(fighters) {
                digest.write_u64(carrier.state_hash());
            }
        }
    }
    digest.finish()
}
