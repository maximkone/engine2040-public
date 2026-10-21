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
use engine_combat::sweep::MAX_SWEEPS_PER_TICK;
use engine_combat::{
    resolve_swing, AnatomyTable, BladeSwing, Cut, FightEvent, CUT_V1, MAX_CUTS_PER_SWING,
    MAX_ENERGY_Q16, MAX_SHARPNESS_Q16,
};
use engine_input::fighter::{ATTACK, DODGE, JUMP};
use engine_physics::balance::BalanceState;
#[cfg(feature = "sim")]
use engine_physics::controller::Controller;
use engine_physics::limb::LimbPool;
use engine_physics::stagger_memory::StaggerMemory;

use crate::match_observer::TickMatchEvents;
use crate::match_wire;
use crate::{
    rest_targets, FighterInput, FighterState, AIR_APEX_MM_TICK, BALANCE_BLOCK_Q8, BALANCE_FALL_Q8,
    BALANCE_HIT_Q8, BALANCE_PARRY_Q8, FIGHTER_GRAVITY, MAX_SLOTS,
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
/// Shared by the legacy step and the spine step.
pub(crate) fn feed_balance(
    balance: &mut [BalanceState],
    stagger: &mut [StaggerMemory],
    events: &[FightEvent],
) -> Option<()> {
    for event in events {
        let (slot, impulse) = match *event {
            FightEvent::Hit { victim, .. } => {
                let staggered = balance
                    .get(usize::from(victim))
                    .is_some_and(BalanceState::staggered);
                let impulse = if staggered {
                    BALANCE_FALL_Q8
                } else {
                    BALANCE_HIT_Q8
                };
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

/// Resolve one tick's landed hits into severed joints. V2 only: the cut event
/// names the joint and the limb pool carries it into the hash; the blade is
/// the attacker's mapped sweep at torso height, and the victim's capsules use
/// the standing rest layout at its feet. Presentation owns the pose and
/// blood; this path owns the integer cut.
///
/// Shared by the legacy step and the spine step; `states` holds exactly the
/// active slots.
pub(crate) fn resolve_cuts(
    states: &[FighterState],
    tick: u32,
    limbs: &mut LimbPool,
    events: &[FightEvent],
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
        let mut out = [Cut {
            swing: 0,
            attacker: 0,
            victim: 0,
            joint: 1,
            flags: 0,
            energy_q16: 0,
            protection_q16: 0,
        }; MAX_CUTS_PER_SWING];
        let count = resolve_swing(&blade, &targets, &anatomy, 0, &CUT_V1, &mut out);
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
