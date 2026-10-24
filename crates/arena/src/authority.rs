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

/// One authoritative landing for a tick (FEEL7): the slot, the fall height
/// estimated from the impact speed in mm, and the health the fall table
/// charged in permille. Presentation reads it for the body squash, camera
/// dip and land cue; the health is already off the fight by then.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FallLanding {
    /// Fighter slot index, `0..fighters`.
    pub slot: u8,
    /// Fall height estimated from the impact speed: `v^2 / (2g)` in mm.
    pub fall_mm: u32,
    /// Health cost the fall table charged, in permille (`0` under rules v1).
    pub damage: u16,
}

/// The fall table (FEEL7): at or above each height, the health it costs in
/// permille. A jump apex is about 0.4 m and stays below the first row; a 4 m
/// fall costs a quarter of the pool. Rules v1 never reads it, so the golden
/// bytes stay frozen.
pub const FALL_DAMAGE_TABLE: [(u32, u16); 5] = [
    (2_000, 100),
    (4_000, 250),
    (6_000, 500),
    (8_000, 750),
    (10_000, 1_000),
];

/// The fall table's health cost for a fall of `fall_mm`: the highest row at
/// or below it, `0` below the first row.
#[must_use]
pub fn fall_damage_permille(fall_mm: u32) -> u16 {
    let mut damage = 0;
    for (height, cost) in FALL_DAMAGE_TABLE {
        if fall_mm >= height {
            damage = cost;
        }
    }
    damage
}

/// One tick of the height channel (row SG5): gravity on every slot that is
/// off the floor, a landing at floor height with the vertical speed cleared,
/// and (FEEL7, rules v2 only) the fall table's health cost. Integer mm, so a
/// replay lands on the same height twice. Near the apex
/// (`AIR_APEX_MM_TICK`) a held jump halves gravity, the AIN6 table's
/// float-free copy; the spear swing keeps the plain arc.
///
/// The landing's height comes from the impact speed just before the floor
/// snap: `v^2 / (2g)`. The floor snap discards the last few mm, so this is
/// the fall the body actually carried, not the last tick's overshoot.
///
/// Shared by the legacy step and the spine step so both tracks land on the
/// same millimetre and charge the same table row.
pub(crate) fn fall_states(
    states: &mut [FighterState],
    rise: &mut [i32],
    fighters: usize,
    last_inputs: &[Option<FighterInput>; MAX_SLOTS],
    fight: &mut FightState,
    rules: &ArenaRules,
) -> [Option<FallLanding>; MAX_SLOTS] {
    let mut landings: [Option<FallLanding>; MAX_SLOTS] = [None; MAX_SLOTS];
    let charge_falls = rules.version == crate::FIGHT_RULES_V2;
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
            // The speed at the floor snap is the fall the body carried.
            let impact = drop.unsigned_abs();
            let fall_mm = impact
                .saturating_mul(impact)
                .saturating_add(u32::try_from(FIGHTER_GRAVITY).unwrap_or(0))
                / u32::try_from(2 * FIGHTER_GRAVITY).unwrap_or(1);
            let damage = if charge_falls {
                fall_damage_permille(fall_mm)
            } else {
                0
            };
            if damage > 0 {
                if let Some(fighter) = fight.fighters.get_mut(slot) {
                    fighter.hp_permille = fighter.hp_permille.saturating_sub(damage);
                }
            }
            if fall_mm > 0 {
                landings[slot] = Some(FallLanding {
                    slot: slot_id(slot),
                    fall_mm,
                    damage,
                });
            }
        } else {
            rise[slot] = drop;
            state.pos[1] = match_wire::metres(next);
        }
    }
    landings
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
#[cfg(feature = "spine")]
pub(crate) fn resolve_cuts_bare(
    states: &[FighterState],
    fight: &FightState,
    tick: u32,
    limbs: &mut LimbPool,
    events: &[FightEvent],
) -> Vec<Cut> {
    resolve_cuts(states, fight, tick, limbs, events, &mut Vec::new())
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
/// BG15: the attacker's kit brings its steel to the blow — `fight`'s armed
/// row for the attacking slot sets `wear_q16`, so the resolver's effective
/// energy scales by the dulled edge. A slot swinging bare hands, or a match
/// with no row armed, reads `0` and cuts exactly as it did.
///
/// Shared by the legacy step and the spine step; `states` holds exactly the
/// active slots.
pub(crate) fn resolve_cuts(
    states: &[FighterState],
    fight: &FightState,
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
        // The kit's own dulled edge (BG15); no armed row is a fresh edge.
        let wear_q16 = fight
            .fighters
            .get(a)
            .and_then(|swinger| swinger.heft)
            .map_or(0, |heft| heft.edge_wear_q16());
        let blade = BladeSwing {
            attacker,
            victim,
            swing: tick.wrapping_mul(8).wrapping_add(u32::from(attacker)),
            p0_mm: [sweep.p0_mm[0], 1_000, sweep.p0_mm[2]],
            p1_mm: [sweep.p1_mm[0], 1_000, sweep.p1_mm[2]],
            blade_half_mm: sweep.blade_half_mm,
            // A landed blade is a full-force cut for the first authority
            // slice; the edge wear it brings is the kit's own (BG15).
            energy_q16: MAX_ENERGY_Q16,
            sharpness_q16: MAX_SHARPNESS_Q16,
            wear_q16,
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

impl crate::Sim {
    /// One tick of the height channel on `sim` (row SG5, FEEL7): gravity, the
    /// landing and the fall table's charge. The landing rows move to
    /// [`Sim::landings`](crate::Sim::landings) for the presentation.
    pub(crate) fn fall(&mut self) {
        self.landings = fall_states(
            &mut self.states,
            &mut self.rise,
            self.fighters,
            &self.last_inputs,
            &mut self.fight,
            &self.rules,
        );
    }

    /// Read-only fall landings from the last stepped tick (FEEL7), slot
    /// order: the fall height and the health the fall table charged. Cleared
    /// each tick; presentation only.
    #[must_use]
    pub fn landings(&self) -> &[Option<FallLanding>; MAX_SLOTS] {
        &self.landings
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spawn_states;

    /// Drop one fighter from `height_m` on the floor under `rules` and return
    /// the landing tick's row plus the fight health left.
    fn drop_from(height_m: f32, rules: ArenaRules) -> (FallLanding, u16) {
        let mut states = spawn_states(2040, 2, 20.0);
        let mut rise = vec![0; states.len()];
        states[0].pos[1] = height_m;
        let mut fight = FightState::new(2).expect("two fighters");
        let inputs = [None; MAX_SLOTS];
        let mut landed = None;
        for _ in 0..120 {
            let tick = fall_states(&mut states, &mut rise, 2, &inputs, &mut fight, &rules);
            if let Some(landing) = tick[0] {
                landed = Some(landing);
                break;
            }
        }
        (
            landed.expect("the fighter reaches the floor"),
            fight.fighters[0].hp_permille,
        )
    }

    #[test]
    fn feel7_a_four_metre_fall_charges_the_tables_quarter_and_v1_never_charges() {
        assert_eq!(fall_damage_permille(1_999), 0);
        assert_eq!(fall_damage_permille(4_000), 250);
        assert_eq!(fall_damage_permille(10_000), 1_000);
        let (landing, hp) = drop_from(4.0, ArenaRules::v2_free_for_all());
        assert!(
            (3_900..=4_100).contains(&landing.fall_mm),
            "impact speed reads a 4 m fall, got {} mm",
            landing.fall_mm
        );
        assert_eq!(landing.damage, 250);
        assert_eq!(hp, 1_000 - 250, "rules v2 charges the fall table");
        let (landing, hp) = drop_from(4.0, ArenaRules::V1);
        assert_eq!(landing.damage, 0);
        assert_eq!(hp, 1_000, "rules v1 and the golden stay untouched");
    }

    /// Eight swings of the same geometry over every severable joint of the
    /// Rome kit at rest, at one edge wear, one blade strength and one worn
    /// set: the joints severed and the energy those blows carry. BG15's cut
    /// test, so the same geometry decides at every wear.
    fn blows(wear_q16: u32, energy_q16: u32, worn: u8) -> (usize, u64) {
        use engine_combat::{JointId, JointTarget, MAX_TARGETS_PER_SWING};
        let anatomy = AnatomyTable::rome_kit();
        let targets: Vec<JointTarget> = JointId::all()
            .iter()
            .filter(|joint| joint.is_severable())
            .map(|&joint| JointTarget {
                joint,
                q0_mm: [-20, -80, 0],
                q1_mm: [20, 80, 0],
                radius_mm: 60,
            })
            .collect();
        assert!(
            targets.len() <= MAX_TARGETS_PER_SWING,
            "the kit's severable joints must fit one swing"
        );
        let blank = Cut {
            swing: 0,
            attacker: 0,
            victim: 1,
            joint: 1,
            flags: 0,
            energy_q16: 0,
            protection_q16: 0,
        };
        let mut severs = 0;
        let mut delivered = 0;
        for swing in 0..8u32 {
            let blade = BladeSwing {
                attacker: 0,
                victim: 1,
                swing,
                p0_mm: [-100, 0, 0],
                p1_mm: [100, 0, 0],
                blade_half_mm: 10,
                energy_q16,
                sharpness_q16: MAX_SHARPNESS_Q16,
                wear_q16,
            };
            let mut out = [blank; MAX_CUTS_PER_SWING];
            let n = resolve_swing(&blade, &targets, &anatomy, worn, &CUT_V1, &mut out);
            severs += n;
            delivered += out
                .iter()
                .take(n)
                .map(|hit| u64::from(hit.energy_q16))
                .sum::<u64>();
        }
        (severs, delivered)
    }

    /// A mid blade: with the table's full-force sharpness its fresh edge
    /// bites through a bare joint every time (851.968 Q16 over the 590.336
    /// tissue) and cannot bite a shield arm at all (1.114.624 protected), so
    /// the plated joints sit in the ramp where wear decides. The full-force
    /// blade cannot show this: it saturates every joint, dull or not.
    const MID_ENERGY_Q16: u32 = 425_984;

    #[test]
    fn bg15_damage_drops_proportional_to_the_edge_wear() {
        use engine_combat::{effective_q16, ARMOR_ALL, MAX_WEAR_Q16};
        // The scaling is exact: half the edge dulled is half the energy.
        let swing = |wear: u32| BladeSwing {
            attacker: 0,
            victim: 1,
            swing: 7,
            p0_mm: [-100, 0, 0],
            p1_mm: [100, 0, 0],
            blade_half_mm: 10,
            energy_q16: MAX_ENERGY_Q16,
            sharpness_q16: MAX_SHARPNESS_Q16,
            wear_q16: wear,
        };
        let fresh = effective_q16(&swing(0));
        assert_eq!(effective_q16(&swing(MAX_WEAR_Q16 / 2)), fresh / 2);
        assert_eq!(effective_q16(&swing(MAX_WEAR_Q16 / 4)), fresh * 3 / 4);
        assert_eq!(effective_q16(&swing(MAX_WEAR_Q16)), 0);

        // The delivered damage follows it exactly: the same joints bitten,
        // half the energy in every one of them; a dead edge carries nothing.
        let (fresh_joints, fresh_damage) = blows(0, MAX_ENERGY_Q16, 0);
        let (half_joints, half_damage) = blows(MAX_WEAR_Q16 / 2, MAX_ENERGY_Q16, 0);
        let (dead_joints, dead_damage) = blows(MAX_WEAR_Q16, MAX_ENERGY_Q16, 0);
        assert!(
            fresh_joints > 20,
            "a fresh edge must still sever, got {fresh_joints}"
        );
        assert_eq!(
            half_joints, fresh_joints,
            "at full force a dulled edge still bites the same joints"
        );
        assert_eq!(
            half_damage * 2,
            fresh_damage,
            "half the edge is half the delivered damage"
        );
        assert_eq!(dead_joints, 0, "a fully dulled edge severs nothing");
        assert_eq!(dead_damage, 0, "a fully dulled edge carries no energy");

        // And against plate the dulled edge loses joints outright, not just
        // force: the same mid blade bites fewer as the edge dulls.
        let (steel, quarter, half) = (
            blows(0, MID_ENERGY_Q16, ARMOR_ALL).0,
            blows(MAX_WEAR_Q16 / 4, MID_ENERGY_Q16, ARMOR_ALL).0,
            blows(MAX_WEAR_Q16 / 2, MID_ENERGY_Q16, ARMOR_ALL).0,
        );
        println!(
            "arena wear cuts: fresh={steel} quarter={quarter} half={half} over 8 swings on the plated kit"
        );
        assert!(
            steel > 0,
            "a fresh mid blade bites the plated kit, got {steel}"
        );
        assert!(
            quarter < steel,
            "a quarter-dulled edge severs fewer joints: {quarter} against {steel}"
        );
        assert!(
            half < quarter,
            "a half-dulled edge severs fewer still: {half} against {quarter}"
        );
    }

    #[test]
    fn bg15_the_kit_wear_reaches_the_resolver_not_zero() {
        use engine_combat::heft::Heft;
        use engine_combat::{effective_q16, MAX_WEAR_Q16};
        // The shipped kit row carries its steel's condition, and the wear the
        // resolver gets is that row's, never a hardcoded zero.
        let gladius = Heft::from_rows(crate::heft::WEAPONS_ROWS, "gladius").expect("shipped row");
        assert_eq!(gladius.edge_wear_permille, 120);
        let wear = gladius.edge_wear_q16();
        assert!(wear > 0, "the kit gladius arrives dulled, got {wear}");
        assert!(
            wear <= MAX_WEAR_Q16,
            "the row's wear stays inside the resolver's range: {wear}"
        );
        // The energy the kit's blade actually delivers, against what the same
        // blade would deliver fresh: the row's wear is in the number.
        let blade = |wear_q16: u32| BladeSwing {
            attacker: 0,
            victim: 1,
            swing: 3,
            p0_mm: [-100, 0, 0],
            p1_mm: [100, 0, 0],
            blade_half_mm: 10,
            energy_q16: MAX_ENERGY_Q16,
            sharpness_q16: MAX_SHARPNESS_Q16,
            wear_q16,
        };
        let (kit_joints, kit_damage) = blows(wear, MAX_ENERGY_Q16, 0);
        let (fresh_joints, fresh_damage) = blows(0, MAX_ENERGY_Q16, 0);
        assert_eq!(
            u64::try_from(kit_joints).unwrap(),
            u64::try_from(fresh_joints).unwrap(),
            "the kit's edge still bites the same joints"
        );
        assert_eq!(
            kit_damage,
            u64::try_from(fresh_joints).unwrap() * effective_q16(&blade(wear)),
            "each blow carries the kit row's dulled energy"
        );
        assert!(
            kit_damage < fresh_damage,
            "the kit's own dulled edge cuts weaker steel than fresh: {kit_damage} against {fresh_damage}"
        );
        assert!(
            effective_q16(&blade(wear)) < effective_q16(&blade(0)),
            "the resolver sees the kit's wear, not zero"
        );
        // A row that names no wear is a fresh edge, so an older mod table
        // arms exactly as it did before BG15.
        let unmarked = Heft::from_rows(
            "table weapons version 2\nold mass_g=1000 damage_permille=120 speed_ticks=15 recovery_ticks=8 stagger_permille=400",
            "old",
        )
        .expect("a row without the new key parses");
        assert_eq!(unmarked.edge_wear_permille, 0);
        assert_eq!(unmarked.edge_wear_q16(), 0);
    }
}
