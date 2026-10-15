//! spine_step: the match stepped over spine columns (flag `spine`).
//!
//! Owner: engine-arena. The five spine systems of `engine_ecs::spine_schedule`
//! run in their fixed phases once per tick over one [`Spine`]:
//! `Inputs` lands the tape on its slots, `Ai` fills the rest from the
//! brains, `Rules` resolves combat over the merged intents, `Motion` moves
//! every live fighter (locomotion root delta, arena clamp, obstacle
//! resolution) and `Extract` steps the destruction prop and folds the
//! checkpoint. Pose, body, vitals, intent and gait live in the spine
//! columns; the rule libraries keep their own machines (`FightState`,
//! `Locomotion`, `DepthBrain`, the bonded crate), so the spine step moves
//! no rule and parity with the legacy step is byte-exact on an empty world.
//!
//! Checkpoints: [`hash_state_with_sim`](crate::hash_state_with_sim) over the
//! legacy shapes rebuilt from the columns (the "translate back" rule), so
//! the legacy golden path is untouched and the spine path compares against
//! it tick for tick. A non-empty [`MatchWorld`] folds its digest into every
//! checkpoint, so scene revisions never share a hash.
//!
//! Track: the kinematic default track only (no physics controllers, `sim`
//! tag byte off). Determinism: no wall clock, no hash maps, floats only as
//! the legacy step already uses them (`to_bits` into the hash). Budget: one
//! reserved checkpoint vector, no per-tick allocation on quiet ticks.

use engine_ai::{DepthBrain, Percept};
use engine_anim::locomotion::{LocoState, Locomotion};
use engine_anim::motion_net::state_code;
use engine_combat::{try_step_into, FightEvent, FightState, MAX_EVENTS_PER_TICK};
use engine_core::fixed::Fixed;
use engine_destruction::bonded::{BondedCollection, BreakEvent};
use engine_ecs::schedule::{Ctx, Schedule};
use engine_ecs::spine::{AiState, AnimState, Body, Intent, Spine, Transform, Vitals};
use engine_ecs::spine_schedule::{SpineSchedule, SpineSystem};
use engine_input::fighter::Buttons;

use crate::controller::{self, ControlView, SlotController, SlotView};
use crate::world::MatchWorld;
use crate::{
    canon_zero, crate_seed, hash_state_with_sim, interrupt_for_slot, loco_wish, nearest_chunk,
    spawn_states, wall_limit, yaw_from_centideg, ArenaConfig, Checkpoint, FighterId, FighterInput,
    FighterState, FighterTape, AI_PERIOD_TICKS, CRATE_CENTER_M, CRATE_HALF_M, CRATE_PIECES,
    FIGHTER_RADIUS_M, MAX_SLOTS,
};

/// One fixed tick in seconds: `1 / TICK_HZ` (pinned by a test).
const DT_S: f32 = 1.0 / 60.0;

/// Capsule radius as the 16.16 raw value of 0.4 m (rounded).
const RADIUS_FIXED: Fixed = Fixed(26_214);
/// Capsule height as the 16.16 raw value of 1.8 m (rounded).
const HEIGHT_FIXED: Fixed = Fixed(117_965);

/// Everything one spine tick reads or writes; the schedule's world.
struct Core {
    config: ArenaConfig,
    fighters: usize,
    tick: u32,
    spine: Spine,
    world: MatchWorld,
    tape: Vec<FighterInput>,
    cursor: usize,
    /// Merged inputs of this tick, tape where present, brain elsewhere.
    row: [FighterInput; MAX_SLOTS],
    /// Which slots the tape covered this tick.
    from_tape: [bool; MAX_SLOTS],
    fight: FightState,
    locos: Vec<Locomotion>,
    brains: Vec<DepthBrain>,
    destruction: BondedCollection,
    /// Combat events of this tick (scratch reserved once).
    events: Vec<FightEvent>,
    breaks: Vec<BreakEvent>,
    checkpoints: Vec<Checkpoint>,
    /// Last checkpoint hash before the world digest folded in (the legacy
    /// shape; equals the legacy hash while no obstacle moved anything).
    base_hash: u64,
    /// Set by a system that met an impossible state; the step returns `None`.
    failed: bool,
    /// Optional outside decider of some slots (the controller hook).
    controller: Option<Box<dyn SlotController>>,
    /// Facing of each slot's last merged input (what the controller view reports).
    last_facing: [i16; MAX_SLOTS],
}

/// One match over spine columns. Build with [`SpineMatch::build`], advance
/// with [`SpineMatch::step`].
pub struct SpineMatch {
    core: Core,
    schedule: Schedule<Core>,
}

impl SpineMatch {
    /// Validated constructor: same config and tape checks as the legacy
    /// build, then seed-expanded spawns settled out of any obstacle. `None`
    /// on any failure (bad config, bad tape, a world that blocks a spawn).
    pub fn build(config: &ArenaConfig, tape: &FighterTape, world: MatchWorld) -> Option<Self> {
        if config.validate().is_err() || tape.validate().is_err() {
            return None;
        }
        let fighters = usize::from(config.fighters);
        for input in &tape.0 {
            if input.tick >= config.ticks || usize::from(input.fighter.0) >= fighters {
                return None;
            }
        }
        let limit = wall_limit(config.half_extent_m);
        let mut spine = Spine::with_capacity(fighters);
        for state in spawn_states(config.seed, config.fighters, config.half_extent_m) {
            let at = [state.pos[0], state.pos[2]];
            let placed = if world.is_empty() {
                at
            } else {
                world.settle(at, FIGHTER_RADIUS_M, limit)?
            };
            let transform = Transform {
                pos: [placed[0], 0.0, placed[1]],
                yaw: 0.0,
            };
            let body = Body {
                vel: [Fixed::ZERO; 2],
                radius: RADIUS_FIXED,
                height: HEIGHT_FIXED,
            };
            let fresh = FightState::new(config.fighters).ok()?.fighters[0];
            let vitals = Vitals {
                hp_permille: fresh.hp_permille,
                stamina: fresh.stamina,
                poise: 0,
                state: 0,
            };
            spine.spawn(transform, body, vitals).ok()?;
        }
        let brains = (0..fighters)
            .map(|slot| {
                let mut brain = DepthBrain::new(FighterId(slot_id(slot)), AI_PERIOD_TICKS);
                brain.set_half_extent_m(config.half_extent_m);
                brain
            })
            .collect();
        let destruction = BondedCollection::crate_prop(
            CRATE_CENTER_M[0],
            CRATE_CENTER_M[1],
            CRATE_CENTER_M[2],
            CRATE_HALF_M,
            CRATE_PIECES,
            crate_seed(config),
        )
        .ok()?;
        let idle = FighterInput {
            tick: 0,
            fighter: FighterId(0),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        };
        let core = Core {
            config: *config,
            fighters,
            tick: 0,
            spine,
            world,
            tape: tape.0.clone(),
            cursor: 0,
            row: [idle; MAX_SLOTS],
            from_tape: [false; MAX_SLOTS],
            fight: FightState::new(config.fighters).ok()?,
            locos: (0..fighters).map(|_| Locomotion::new()).collect(),
            brains,
            destruction,
            events: Vec::with_capacity(MAX_EVENTS_PER_TICK),
            breaks: Vec::new(),
            checkpoints: Vec::with_capacity(usize::try_from(config.ticks).unwrap_or(0)),
            base_hash: 0,
            failed: false,
            controller: None,
            last_facing: [0; MAX_SLOTS],
        };
        let mut builder = SpineSchedule::new();
        builder.register(SpineSystem::Inputs, sys_inputs).ok()?;
        builder.register(SpineSystem::Ai, sys_ai).ok()?;
        builder.register(SpineSystem::Rules, sys_rules).ok()?;
        builder.register(SpineSystem::Motion, sys_motion).ok()?;
        builder.register(SpineSystem::Extract, sys_extract).ok()?;
        Some(Self {
            core,
            schedule: builder.build().ok()?,
        })
    }

    /// Let `controller` decide the slots it claims from now on (see [`crate::controller`]).
    #[must_use]
    pub fn with_controller(mut self, controller: Box<dyn SlotController>) -> Self {
        self.core.controller = Some(controller);
        self
    }

    /// Next tick to simulate (equals the checkpoint count).
    pub fn tick(&self) -> u32 {
        self.core.tick
    }

    /// Run one fixed tick; the checkpoint it folded, or `None` when a rule
    /// refused (unreachable after validation, never a silent skip).
    pub fn step(&mut self) -> Option<Checkpoint> {
        if self.core.failed || self.core.tick >= self.core.config.ticks {
            return None;
        }
        self.schedule.run(&mut self.core, DT_S);
        if self.core.failed {
            return None;
        }
        self.core.checkpoints.last().copied()
    }

    /// The spine columns after the last step.
    pub fn spine(&self) -> &Spine {
        &self.core.spine
    }

    /// Combat state after the last step (vitals source).
    pub fn fight(&self) -> &FightState {
        &self.core.fight
    }

    /// Locomotion machines after the last step (gait source).
    pub fn locos(&self) -> &[Locomotion] {
        &self.core.locos
    }

    /// The obstacles this match is played around.
    pub fn world(&self) -> &MatchWorld {
        &self.core.world
    }

    /// Last checkpoint hash without the world digest: the legacy shape the
    /// parity harness compares against.
    pub fn base_hash(&self) -> u64 {
        self.core.base_hash
    }

    /// Checkpoints folded so far.
    pub fn checkpoints(&self) -> &[Checkpoint] {
        &self.core.checkpoints
    }
}

/// [`run_spine`] with a [`SlotController`] deciding the slots it claims (tape inputs still win).
pub fn run_spine_controlled(
    config: &ArenaConfig,
    tape: &FighterTape,
    world: &MatchWorld,
    controller: Box<dyn SlotController>,
) -> Vec<Checkpoint> {
    let Some(sim) = SpineMatch::build(config, tape, world.clone()) else {
        return Vec::new();
    };
    let mut sim = sim.with_controller(controller);
    while sim.tick() < config.ticks {
        if sim.step().is_none() {
            return Vec::new();
        }
    }
    sim.core.checkpoints
}

/// Spine match to completion: one checkpoint per tick, empty on any failure.
pub fn run_spine(config: &ArenaConfig, tape: &FighterTape, world: &MatchWorld) -> Vec<Checkpoint> {
    let Some(mut sim) = SpineMatch::build(config, tape, world.clone()) else {
        return Vec::new();
    };
    while sim.tick() < config.ticks {
        if sim.step().is_none() {
            return Vec::new();
        }
    }
    sim.core.checkpoints
}

// ---------------------------------------------------------------------------
// Systems, in `SpineSystem::ORDER`.
// ---------------------------------------------------------------------------

/// `Inputs` (PreUpdate): the tape slice of this tick lands on its slots.
fn sys_inputs(core: &mut Core, _: &mut Ctx) {
    core.from_tape = [false; MAX_SLOTS];
    while core.cursor < core.tape.len() && core.tape[core.cursor].tick == core.tick {
        let input = core.tape[core.cursor];
        let slot = usize::from(input.fighter.0);
        if slot < core.fighters {
            core.row[slot] = input;
            core.from_tape[slot] = true;
        }
        core.cursor += 1;
    }
}

/// `Ai` (PreUpdate): brains fill the slots the tape left empty, then every
/// slot's merged intent lands in the AI column.
fn sys_ai(core: &mut Core, _: &mut Ctx) {
    for slot in 0..core.fighters {
        if !core.from_tape[slot] {
            let percept = percept(core, slot);
            core.row[slot] = core.brains[slot].decide(&percept);
        }
    }
    if let Some(mut decider) = core.controller.take() {
        let mut views = [SlotView::default(); MAX_SLOTS];
        let transforms = core.spine.transforms();
        for (slot, view) in views.iter_mut().enumerate().take(core.fighters) {
            *view = controller::slot_view(
                transforms[slot].pos,
                &core.fight.fighters[slot],
                core.last_facing[slot],
            );
        }
        let view = ControlView {
            tick: core.tick,
            seed: core.config.seed,
            slots: &views[..core.fighters],
        };
        let n = core.fighters;
        controller::drive(&mut *decider, &view, &mut core.row[..n], &core.from_tape);
        // Script effects land on fight state here, one system before `sys_rules`
        // resolves combat; an empty drain keeps the legacy spine run exactly.
        let effects = decider.take_effects();
        controller::apply_script_effects(&mut core.fight, &effects);
        core.controller = Some(decider);
    }
    for (last, input) in core
        .last_facing
        .iter_mut()
        .zip(&core.row)
        .take(core.fighters)
    {
        *last = input.facing;
    }
    let tick = core.tick;
    let (row, from_tape) = (core.row, core.from_tape);
    core.spine.for_each_mut(|slot, view| {
        let input = row[slot];
        *view.ai = AiState {
            intent: Intent {
                mx: input.mx,
                mz: input.mz,
                facing: input.facing,
                buttons: input.buttons.0,
            },
            clock: tick,
            source: if from_tape[slot] {
                AiState::FROM_TAPE
            } else {
                AiState::FROM_BRAIN
            },
        };
    });
}

/// `Rules` (Update): strict combat over the merged intents, then the vitals
/// column mirrors the combat state.
fn sys_rules(core: &mut Core, _: &mut Ctx) {
    let n = core.fighters;
    if try_step_into(&core.row[..n], &mut core.fight, &mut core.events).is_err() {
        core.failed = true;
        return;
    }
    let fight = core.fight;
    core.spine.for_each_mut(|slot, view| {
        let f = fight.fighters[slot];
        let mut state = 0u8;
        if f.is_attacking {
            state |= Vitals::ATTACKING;
        }
        if f.is_blocking {
            state |= Vitals::BLOCKING;
        }
        if f.is_dodging {
            state |= Vitals::DODGING;
        }
        if f.knockdown > 0 {
            state |= Vitals::DOWN;
        }
        if f.hp_permille == 0 {
            state |= Vitals::OUT;
        }
        *view.vitals = Vitals {
            hp_permille: f.hp_permille,
            stamina: f.stamina,
            poise: 0,
            state,
        };
    });
}

/// `Motion` (Update): each live fighter's wish drives its locomotion; the
/// requested root delta resolves once (arena clamp, then obstacles) and
/// moves the pose. Knocked-out fighters hold pose.
fn sys_motion(core: &mut Core, _: &mut Ctx) {
    let limit = wall_limit(core.config.half_extent_m);
    let tick = core.tick;
    let row = core.row;
    for (slot, &input) in row.iter().enumerate().take(core.fighters) {
        if core.fight.fighters[slot].hp_permille == 0 {
            // Held pose: no motion this tick.
            if let Some(view) = core.spine.slot_mut(slot) {
                view.body.vel = [Fixed::ZERO; 2];
            }
            continue;
        }
        let (speed, dir) = loco_wish(&input);
        let interrupt = interrupt_for_slot(&core.events, slot_id(slot));
        let Ok(root) = core.locos[slot].update_root(speed, dir, tick, interrupt) else {
            core.failed = true;
            return;
        };
        let requested = root.dp;
        let Some(view) = core.spine.slot_mut(slot) else {
            core.failed = true;
            return;
        };
        let pos = view.transform.pos;
        view.transform.yaw = yaw_from_centideg(input.facing);
        let to = [
            (pos[0] + requested[0]).clamp(-limit, limit),
            (pos[2] + requested[1]).clamp(-limit, limit),
        ];
        let placed = if core.world.is_empty() {
            to
        } else {
            core.world
                .resolve([pos[0], pos[2]], to, FIGHTER_RADIUS_M, limit)
        };
        view.transform.pos[0] = canon_zero(placed[0]);
        view.transform.pos[2] = canon_zero(placed[1]);
        view.body.vel = [
            raw_fixed(view.transform.pos[0] - pos[0]),
            raw_fixed(view.transform.pos[2] - pos[2]),
        ];
        let loco = &core.locos[slot];
        *view.anim = AnimState {
            loco: loco_disc(loco.state),
            phase: loco.phase(),
            facing: loco.facing(),
            airborne: loco.is_airborne(),
            interrupt: loco.interrupt_active(),
            pose_ref: 0,
        };
    }
}

/// `Extract` (PostUpdate): sword hits strike the crate prop, the prop steps,
/// and the checkpoint folds over the legacy shapes rebuilt from the columns.
fn sys_extract(core: &mut Core, _: &mut Ctx) {
    for event in &core.events {
        if let FightEvent::Hit { victim, .. } = *event {
            let slot = usize::from(victim);
            if let Some(t) = core.spine.transforms().get(slot) {
                if let Some(chunk) = nearest_chunk(&core.destruction, t.pos) {
                    if let Some(mass) = core.destruction.chunk_mass(chunk) {
                        let impulse = mass * core.destruction.break_threshold() + 1.0;
                        let _ = core.destruction.apply_impact(chunk, impulse, core.tick);
                    }
                }
            }
        }
    }
    core.destruction.step(DT_S);
    core.breaks = core.destruction.drain_events();
    let destruction_hash = core.destruction.state_hash();

    let n = core.fighters;
    let mut states = [FighterState {
        id: FighterId(0),
        pos: [0.0; 3],
        yaw: 0.0,
    }; MAX_SLOTS];
    let mut buttons = [0u8; MAX_SLOTS];
    for slot in 0..n {
        let t = core.spine.transforms()[slot];
        states[slot] = FighterState {
            id: FighterId(slot_id(slot)),
            pos: t.pos,
            yaw: t.yaw,
        };
        buttons[slot] = core.spine.ai()[slot].intent.buttons;
    }
    let mut hash = hash_state_with_sim(
        core.tick,
        &states[..n],
        &buttons[..n],
        &core.fight,
        &core.events,
        0,
        &core.locos,
        destruction_hash,
        &core.breaks,
        false,
    );
    core.base_hash = hash;
    let world = core.world.digest();
    if world != 0 {
        const PRIME: u64 = 0x1000_0000_01b3;
        for byte in world.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    core.checkpoints.push(Checkpoint {
        tick: core.tick,
        hash,
    });
    core.tick += 1;
}

/// One fighter's [`Percept`]: own pose, nearest foe (strict `<`, ties keep
/// the lowest slot), separation and the foe's attack flag. Same mapping as
/// the legacy step, read from the spine columns.
fn percept(core: &Core, slot: usize) -> Percept {
    let transforms = core.spine.transforms();
    let me = transforms[slot];
    let mut foe = (slot + 1) % core.fighters;
    let mut best = f32::INFINITY;
    for (index, other) in transforms.iter().enumerate().take(core.fighters) {
        if index == slot {
            continue;
        }
        let dx = other.pos[0] - me.pos[0];
        let dz = other.pos[2] - me.pos[2];
        let dist_sq = dx * dx + dz * dz;
        if dist_sq < best {
            best = dist_sq;
            foe = index;
        }
    }
    let dist_m = if best.is_finite() { best.sqrt() } else { 0.0 };
    Percept::new(
        me.pos,
        me.yaw,
        transforms[foe].pos,
        core.fight.fighters[foe].is_attacking,
        dist_m,
        core.tick,
    )
}

/// Locomotion state byte: the animation crate's state code (`0..=8`, the
/// same discriminants the checkpoint hash uses).
fn loco_disc(state: LocoState) -> u8 {
    u8::try_from(state_code(state)).unwrap_or(u8::MAX)
}

/// Fighter slot as the `u8` id the contracts carry (slots stay below 16).
fn slot_id(slot: usize) -> u8 {
    u8::try_from(slot).unwrap_or(u8::MAX)
}

/// A meter value as 16.16 raw bits, truncating toward zero, saturating on
/// overflow and infinity, zero for NaN, subnormals and zero. Integer
/// arithmetic on the `f32` bit pattern: no float-to-int cast.
pub(crate) fn raw_fixed(meters: f32) -> Fixed {
    let bits = meters.to_bits();
    let negative = bits >> 31 == 1;
    let exp = i32::try_from((bits >> 23) & 0xff).unwrap_or(0);
    let mantissa = u64::from((bits & 0x7f_ffff) | 0x80_0000);
    if exp == 0 || (exp == 255 && bits & 0x7f_ffff != 0) {
        return Fixed::ZERO;
    }
    // value = mantissa * 2^(exp - 150); raw = value * 2^16.
    let shift = exp - 134;
    let magnitude = if exp == 255 || shift > 39 {
        i64::MAX
    } else if shift >= 0 {
        i64::try_from(mantissa << u32::try_from(shift).unwrap_or(0)).unwrap_or(i64::MAX)
    } else {
        let right = u32::try_from(-shift).unwrap_or(64);
        i64::try_from(mantissa.checked_shr(right).unwrap_or(0)).unwrap_or(0)
    };
    let saturated = magnitude == i64::MAX;
    Fixed(match (negative, saturated) {
        (false, _) => magnitude,
        (true, true) => i64::MIN,
        (true, false) => -magnitude,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run;
    use crate::world::Obstacle;

    fn config(seed: u64, ticks: u32, fighters: u8) -> ArenaConfig {
        ArenaConfig::try_new(seed, ticks, fighters, 12.0).expect("valid config")
    }

    #[test]
    fn raw_fixed_truncates_like_the_float_cast() {
        for v in [
            0.0f32,
            -0.0,
            1.0,
            -1.0,
            0.0833333,
            -0.0833333,
            3.14159,
            1234.5,
            1.0e-6,
            -1.0e-6,
            0.4,
            65535.9,
            1.0e12,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        ] {
            let want = if v.is_nan() { 0 } else { (v * 65_536.0) as i64 };
            assert_eq!(raw_fixed(v), Fixed(want), "value {v}");
        }
    }

    #[test]
    fn one_tick_is_a_sixtieth_of_a_second() {
        assert_eq!(crate::TICK_HZ, 60);
        assert_eq!(DT_S, 1.0 / 60.0);
    }

    #[test]
    fn empty_world_matches_the_legacy_checkpoints() {
        for seed in [1u64, 2, 7, 2040] {
            let cfg = config(seed, 300, 2);
            let tape = FighterTape(Vec::new());
            let legacy = run(&cfg, &tape);
            let spine = run_spine(&cfg, &tape, &MatchWorld::empty());
            assert_eq!(legacy.len(), 300);
            assert_eq!(spine, legacy, "seed {seed}");
        }
    }

    #[test]
    fn a_wall_changes_the_hash_even_untouched() {
        let cfg = config(5, 60, 2);
        let tape = FighterTape(Vec::new());
        let far = Obstacle::aligned([10.0, 10.0], [0.5, 0.5]);
        let world = MatchWorld::try_new(vec![far]).expect("valid");
        let plain = run_spine(&cfg, &tape, &MatchWorld::empty());
        let walled = run_spine(&cfg, &tape, &world);
        assert_eq!(plain.len(), walled.len());
        assert_ne!(plain.last(), walled.last());
        // Same world twice is the same hash.
        assert_eq!(walled, run_spine(&cfg, &tape, &world));
    }

    #[test]
    fn schedule_runs_in_spine_order_and_the_spine_snapshot_roundtrips() {
        let cfg = config(9, 30, 3);
        let mut sim =
            SpineMatch::build(&cfg, &FighterTape(Vec::new()), MatchWorld::empty()).expect("builds");
        while sim.tick() < 30 {
            sim.step().expect("steps");
        }
        let snap = sim.spine().snapshot();
        let back = Spine::restore(&snap.bytes).expect("restores");
        assert_eq!(back.snapshot(), snap);
    }

    /// Run a tapeless (AI vs AI) match and return every tick's planar poses
    /// plus the first tick a fighter reached zero health.
    fn pose_history(cfg: &ArenaConfig, world: &MatchWorld) -> (Vec<Vec<[f32; 2]>>, Option<u32>) {
        let mut sim =
            SpineMatch::build(cfg, &FighterTape(Vec::new()), world.clone()).expect("builds");
        let mut history = vec![planar(&sim)];
        let mut kill = None;
        while sim.tick() < cfg.ticks {
            sim.step().expect("steps");
            history.push(planar(&sim));
            if kill.is_none() && sim.spine().vitals().iter().any(|v| v.hp_permille == 0) {
                kill = Some(sim.tick() - 1);
            }
        }
        (history, kill)
    }

    fn planar(sim: &SpineMatch) -> Vec<[f32; 2]> {
        sim.spine()
            .transforms()
            .iter()
            .map(|t| [t.pos[0], t.pos[2]])
            .collect()
    }

    /// Slots that hold below the fork stuck speed for 180 consecutive
    /// pre-kill samples (the balance tournament's stuck definition).
    fn stuck_slots(history: &[Vec<[f32; 2]>], kill: Option<u32>, ticks: u32) -> Vec<usize> {
        let step = crate::fork::STUCK_SPEED_MPS * DT_S;
        let last = usize::try_from(kill.unwrap_or(ticks.saturating_sub(1))).unwrap_or(0);
        let mut out = Vec::new();
        for slot in 0..history[0].len() {
            let mut run = 0u32;
            for t in 0..last {
                let (a, b) = (history[t][slot], history[t + 1][slot]);
                let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                if dx * dx + dz * dz < step * step {
                    run += 1;
                    if run >= 180 {
                        out.push(slot);
                        break;
                    }
                } else {
                    run = 0;
                }
            }
        }
        out
    }

    #[test]
    fn stuck_legs_stay_empty_on_spine_fixtures() {
        for s in 0..20u64 {
            let cfg = config(7000 + s, 600, 2);
            let (history, kill) = pose_history(&cfg, &MatchWorld::empty());
            assert_eq!(
                stuck_slots(&history, kill, 600),
                Vec::<usize>::new(),
                "seed {s}"
            );
        }
    }

    #[test]
    fn walled_fights_never_put_a_fighter_inside_the_wall() {
        // A 2 m x 0.4 m wall across the arena center.
        let wall = Obstacle::aligned([0.0, 0.0], [1.0, 0.2]);
        let world = MatchWorld::try_new(vec![wall]).expect("valid");
        let mut clean = 0;
        let mut moved = 0;
        for s in 0..20u64 {
            let cfg = config(7000 + s, 600, 2);
            let (history, _) = pose_history(&cfg, &world);
            let inside = history
                .iter()
                .flatten()
                .any(|p| world.overlaps(*p, FIGHTER_RADIUS_M));
            if !inside {
                clean += 1;
            }
            // The wall changed what was played on this seed.
            let open = run_spine(&cfg, &FighterTape(Vec::new()), &MatchWorld::empty());
            let walled = run_spine(&cfg, &FighterTape(Vec::new()), &world);
            if open.last() != walled.last() {
                moved += 1;
            }
        }
        assert_eq!(clean, 20, "fighters inside the wall");
        assert_eq!(moved, 20, "the wall must change every seed's final hash");
    }
}
