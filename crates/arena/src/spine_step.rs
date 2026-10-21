//! spine_step: the match stepped over spine columns (flag `spine`).
//!
//! Owner: engine-arena. The five spine systems of `engine_ecs::spine_schedule`
//! run in their fixed phases once per tick over one [`Spine`]: `Inputs`
//! lands the tape on its slots, `Ai` fills the rest from the brains and
//! plans the tick's fights, `Rules` resolves combat over the merged intents,
//! `Motion` moves every live fighter and `Extract` resolves the aftermath
//! and folds the checkpoint.
//!
//! The spine step owns the same authority the legacy [`Sim`](crate::Sim)
//! owns: one persistent pose array, one fight state, one balance and
//! stagger set per slot, one limb pool, the spear grapple, the height
//! channel and (under `sim`) one physics controller per fighter. The rule
//! libraries keep their own machines (`FightState`, `Locomotion`,
//! `DepthSet`, the bonded crate); the spine columns are this tick's
//! output, written once per tick from that authority, so a column never
//! feeds the step back.
//!
//! Checkpoints: [`hash_state_with_sim`](crate::hash_state_with_sim) over the
//! legacy shapes (the "translate back" rule), so the legacy golden path is
//! untouched and the spine path compares against it tick for tick. The
//! rules, spear and height folds follow the legacy chain, and a non-empty
//! [`MatchWorld`] folds its digest into every checkpoint, so scene
//! revisions never share a hash.
//!
//! Track: the kinematic default track and the v2 rules track (cuts and
//! limbs, balance and stagger, the spear grapple, the height channel and
//! the match observer track). Under `sim` the physics controllers own the
//! poses. Determinism: no wall clock, no hash maps, floats only as the
//! legacy step already uses them (`to_bits` into the hash). Budget: one
//! reserved checkpoint vector, no per-tick allocation on quiet ticks.

use engine_ai::{DepthBrain, Percept, Personality};
use engine_anim::locomotion::{LocoState, Locomotion};
use engine_anim::motion_net::state_code;
use engine_combat::depth::DepthSet;
use engine_combat::fight::{
    try_step_v2_with_cues_into, AttackStarted, MAX_ATTACK_STARTED_PER_TICK,
};
use engine_combat::tables::TABLES_VERSION_V2;
use engine_combat::{try_step_into, Cut, FightEvent, FightState, MAX_EVENTS_PER_TICK, TABLES_V1};
use engine_core::fixed::Fixed;
use engine_destruction::bonded::{BondedCollection, BreakEvent};
use engine_ecs::schedule::{Ctx, Schedule};
use engine_ecs::spine::{AiState, AnimState, Body, Intent, Spine, Transform, Vitals};
use engine_ecs::spine_schedule::{SpineSchedule, SpineSystem};
use engine_input::fighter::{Buttons, JUMP, THROW_SPEAR};
use engine_physics::balance::{BalanceParams, BalanceState, GaitPhase};
#[cfg(feature = "sim")]
use engine_physics::controller::Controller;
use engine_physics::limb::{LimbPool, MAX_LIMBS};
#[cfg(feature = "sim")]
use engine_physics::rapier::FIXED_DT;
use engine_physics::stagger_memory::StaggerMemory;

use crate::authority;
use crate::controller::{self, ControlView, SlotController, SlotView};
use crate::engage;
use crate::match_observer::TickMatchEvents;
use crate::match_wire;
use crate::spear;
use crate::world::MatchWorld;
use crate::{
    brain_wish_to_world, canon_zero, crate_seed, fold_rise, fold_world, hash_state_with_sim,
    interrupt_for_slot, loco_wish, nearest_chunk, spawn_states, strike_pose, wall_limit,
    yaw_from_centideg, ArenaConfig, ArenaRules, Checkpoint, FighterId, FighterInput, FighterState,
    FighterTape, AI_PERIOD_TICKS, BALANCE_BODY_MASS_KG, CRATE_CENTER_M, CRATE_HALF_M, CRATE_PIECES,
    FIGHTER_RADIUS_M, FIGHT_RULES_V2, JUMP_RISE_MM_TICK, MAX_SLOTS,
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
    /// Authoritative poses, the legacy `Sim::states` shape, one per slot.
    states: [FighterState; MAX_SLOTS],
    /// Poses at the tick start, so the body column's root delta is the
    /// legacy expression over the same two poses.
    before: [FighterState; MAX_SLOTS],
    /// Merged inputs of the last stepped tick (the legacy `last_inputs`).
    last_inputs: [Option<FighterInput>; MAX_SLOTS],
    /// Fresh jump press of this tick, per slot.
    jump_edges: [bool; MAX_SLOTS],
    /// Jump release of this tick, per slot.
    jump_releases: [bool; MAX_SLOTS],
    /// Merged button bytes of this tick; the spear bit is masked off
    /// unless a spear match is armed.
    buttons: [u8; MAX_SLOTS],
    fight: FightState,
    locos: Vec<Locomotion>,
    brains: Vec<DepthBrain>,
    destruction: BondedCollection,
    /// Combat events of this tick (the combat scratch and the tick log are
    /// the same buffer; every combat entry point clears it first).
    events: Vec<FightEvent>,
    breaks: Vec<BreakEvent>,
    /// Severed joints of the last stepped tick.
    cuts: Vec<Cut>,
    checkpoints: Vec<Checkpoint>,
    /// Last checkpoint hash before the world digest folded in (the legacy
    /// shape; equals the legacy hash while no obstacle moved anything).
    base_hash: u64,
    /// Fight rules this match runs under (the legacy `rules`).
    rules: ArenaRules,
    /// Per-slot integer balance, stepped only under `rules.balance`.
    balance: Vec<BalanceState>,
    /// Per-slot stagger memory, stepped only under `rules.balance`.
    stagger: Vec<StaggerMemory>,
    /// Combat-depth poise meters, stepped beside the fight step.
    depth: DepthSet,
    /// Severed-limb pool, stepped only under `rules.balance`.
    limbs: LimbPool,
    /// Spear grapple state, `Some` only for a spear match.
    spear: Option<spear::SpearState>,
    /// Height channel vertical speeds, mm per tick; empty while off.
    rise: Vec<i32>,
    /// True once the height channel is armed.
    height: bool,
    /// Press-tick attack cues of the last stepped tick.
    last_cues: Vec<AttackStarted>,
    /// Match observer track (the GP1/GP2 resolver call-site).
    match_track: match_wire::MatchTrack,
    /// Whether the match track resolves every tick.
    match_live: bool,
    /// Match observer row of the last stepped tick.
    match_row: Option<TickMatchEvents>,
    /// Physics controllers, one per fighter (`sim` only).
    #[cfg(feature = "sim")]
    controllers: Vec<Controller>,
    /// Set by a system that met an impossible state; the step returns `None`.
    failed: bool,
    /// Optional outside decider of some slots (the controller hook).
    controller: Option<Box<dyn SlotController>>,
}

impl Core {
    /// Pin the fight rules before the first tick. An unknown version or a
    /// match already past tick 0 refuses (`false`) and keeps v1. Rules v2
    /// seeds one temperament per brain, so identical brains do not close in
    /// lockstep under the symmetric v2 hit rule.
    fn set_rules(&mut self, rules: ArenaRules) -> bool {
        if !rules.is_known() || self.tick != 0 {
            return false;
        }
        self.rules = rules;
        if rules.version == FIGHT_RULES_V2 {
            for (slot, brain) in self.brains.iter_mut().enumerate() {
                let id = u8::try_from(slot).unwrap_or(u8::MAX);
                brain.set_personality(Personality::seeded(self.config.seed, id));
            }
        }
        true
    }

    /// Arm the height channel from any tick (a fresh jump press). Refuses
    /// on the `sim` leg, whose controller owns the pose.
    fn arm_jump(&mut self) -> bool {
        if cfg!(feature = "sim") {
            return false;
        }
        if !self.height {
            self.rise = vec![0; self.fighters];
            self.height = true;
        }
        true
    }
}

/// The match's authority, built like the legacy one: the same checks, the
/// same seed-expanded spawns settled out of any obstacle, the same rules
/// the config selects and (under `sim`) the same settled controllers.
fn build_core(config: &ArenaConfig, tape: &FighterTape, world: MatchWorld) -> Option<Core> {
    if config.validate().is_err() || tape.validate().is_err() {
        return None;
    }
    let fighters = usize::from(config.fighters);
    for input in &tape.0 {
        if input.tick >= config.ticks || usize::from(input.fighter.0) >= fighters {
            return None;
        }
    }
    let states = settled_spawns(config, &world)?;
    let mut core = new_core(config, tape, world, &states)?;
    // Table version 2 runs under rules v2; version 1 pins the legacy rules.
    // The same call the legacy build makes.
    if !core.set_rules(config.rules()) {
        return None;
    }
    Some(core)
}

/// The fresh authority of one match: the columns, the fight state, the
/// machines and the buffers, wired exactly as the legacy build wires them.
fn new_core(
    config: &ArenaConfig,
    tape: &FighterTape,
    world: MatchWorld,
    states: &[FighterState],
) -> Option<Core> {
    let fighters = usize::from(config.fighters);
    let spine = spawn_columns(states, config.fighters)?;
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
    // Depth tables follow the match table version: V1 keeps poise damage 0
    // and never staggers, V2 uses the shipped poise rows.
    let depth_tables = if config.table_version == TABLES_VERSION_V2 {
        engine_combat::fight::TABLES_V2
    } else {
        TABLES_V1
    };
    let cue_cap = fighters.min(MAX_ATTACK_STARTED_PER_TICK);
    let limb_cap = fighters.clamp(1, MAX_LIMBS);
    #[cfg(feature = "sim")]
    let controllers = crate::build_controllers(config, states)?;
    let mut state_array = [FighterState {
        id: FighterId(0),
        pos: [0.0; 3],
        yaw: 0.0,
    }; MAX_SLOTS];
    state_array[..fighters].copy_from_slice(&states[..fighters]);
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
        states: state_array,
        before: state_array,
        last_inputs: [None; MAX_SLOTS],
        jump_edges: [false; MAX_SLOTS],
        jump_releases: [false; MAX_SLOTS],
        buttons: [0; MAX_SLOTS],
        fight: FightState::new(config.fighters).ok()?,
        locos: (0..fighters).map(|_| Locomotion::new()).collect(),
        brains,
        destruction,
        events: Vec::with_capacity(MAX_EVENTS_PER_TICK),
        breaks: Vec::new(),
        cuts: Vec::new(),
        checkpoints: Vec::with_capacity(usize::try_from(config.ticks).unwrap_or(0)),
        base_hash: 0,
        rules: ArenaRules::V1,
        balance: (0..fighters)
            .map(|_| BalanceState::new(BalanceParams::default()))
            .collect(),
        stagger: (0..fighters).map(|_| StaggerMemory::new()).collect(),
        depth: DepthSet::new(depth_tables, config.fighters),
        limbs: LimbPool::new(limb_cap).ok()?,
        spear: None,
        rise: Vec::new(),
        height: false,
        last_cues: Vec::with_capacity(cue_cap),
        match_track: match_wire::MatchTrack::new(config.fighters)?,
        match_live: false,
        match_row: None,
        #[cfg(feature = "sim")]
        controllers,
        failed: false,
        controller: None,
    };
    Some(core)
}

/// The seed-expanded start poses, settled out of the world's boxes so a
/// seed that lands on a kit wall starts free (mirrors the legacy
/// `Sim::build_world`).
fn settled_spawns(config: &ArenaConfig, world: &MatchWorld) -> Option<Vec<FighterState>> {
    let mut states = spawn_states(config.seed, config.fighters, config.half_extent_m);
    if !world.is_empty() {
        let limit = wall_limit(config.half_extent_m);
        for state in states.iter_mut() {
            let placed = world.settle([state.pos[0], state.pos[2]], FIGHTER_RADIUS_M, limit)?;
            state.pos[0] = placed[0];
            state.pos[2] = placed[1];
        }
    }
    Some(states)
}

/// The spine columns of a fresh match: one slot per fighter, the pose
/// column from the start poses, the body column at rest and the vitals
/// column from a fresh fight.
fn spawn_columns(states: &[FighterState], fighters: u8) -> Option<Spine> {
    let mut spine = Spine::with_capacity(usize::from(fighters));
    for state in states {
        let transform = Transform {
            pos: state.pos,
            yaw: state.yaw,
        };
        let body = Body {
            vel: [Fixed::ZERO; 2],
            radius: RADIUS_FIXED,
            height: HEIGHT_FIXED,
        };
        let fresh = FightState::new(fighters).ok()?.fighters[0];
        let vitals = Vitals {
            hp_permille: fresh.hp_permille,
            stamina: fresh.stamina,
            poise: 0,
            state: 0,
        };
        spine.spawn(transform, body, vitals).ok()?;
    }
    Some(spine)
}

/// One match over spine columns. Build with [`SpineMatch::build`], advance
/// with [`SpineMatch::step`].
pub struct SpineMatch {
    core: Core,
    schedule: Schedule<Core>,
}

impl SpineMatch {
    /// Validated constructor: same config and tape checks as the legacy
    /// build, then seed-expanded spawns settled out of any obstacle, the
    /// rules the config selects and (under `sim`) one settled controller
    /// per fighter. `None` on any failure (bad config, bad tape, a world
    /// that blocks a spawn).
    pub fn build(config: &ArenaConfig, tape: &FighterTape, world: MatchWorld) -> Option<Self> {
        let core = build_core(config, tape, world)?;
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

    /// Pin the fight rules of this match. `None` for an unknown version or
    /// a match already past tick 0.
    #[must_use]
    pub fn with_rules(mut self, rules: ArenaRules) -> Option<Self> {
        if !self.core.set_rules(rules) {
            return None;
        }
        Some(self)
    }

    /// Arm the spear grapple (the legacy `Sim::set_spear`). `None` past
    /// tick 0 and on the `sim` leg, whose controllers own the poses the
    /// tether would have to move.
    #[must_use]
    pub fn with_spear(mut self) -> Option<Self> {
        if cfg!(feature = "sim") || self.core.tick != 0 {
            return None;
        }
        self.core.spear = Some(spear::SpearState::new(self.core.fighters));
        Some(self)
    }

    /// Arm the height channel (the legacy `Sim::set_height`): fighters then
    /// fall under gravity and land on the floor. `None` past tick 0 and on
    /// the `sim` leg.
    #[must_use]
    pub fn with_height(mut self) -> Option<Self> {
        if cfg!(feature = "sim") || self.core.tick != 0 {
            return None;
        }
        self.core.rise = vec![0; self.core.fighters];
        self.core.height = true;
        Some(self)
    }

    /// Arm the match observer track at `version` (the legacy
    /// `Sim::set_match_version`). `None` for an unknown version.
    #[must_use]
    pub fn with_match(mut self, version: u32) -> Option<Self> {
        if !self.core.match_track.set_version(version) {
            return None;
        }
        self.core.match_live = true;
        Some(self)
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

    /// The fight rules this match runs under.
    #[must_use]
    pub fn rules(&self) -> &ArenaRules {
        &self.core.rules
    }

    /// Authoritative poses after the last step, the legacy `Sim::states`
    /// shape.
    #[must_use]
    pub fn states(&self) -> &[FighterState] {
        &self.core.states[..self.core.fighters]
    }

    /// Per-slot integer balance after the last step.
    #[must_use]
    pub fn balance(&self) -> &[BalanceState] {
        &self.core.balance
    }

    /// Per-slot stagger memory after the last step.
    #[must_use]
    pub fn stagger(&self) -> &[StaggerMemory] {
        &self.core.stagger
    }

    /// Combat events of the last stepped tick, in emission order.
    #[must_use]
    pub fn events(&self) -> &[FightEvent] {
        &self.core.events
    }

    /// Severed joints of the last stepped tick.
    #[must_use]
    pub fn cuts(&self) -> &[Cut] {
        &self.core.cuts
    }

    /// Match observer row of the last stepped tick, `None` without the
    /// observer track.
    #[must_use]
    pub fn match_row(&self) -> Option<&TickMatchEvents> {
        self.core.match_row.as_ref()
    }

    /// The spear grapple state, `None` without a spear match.
    #[must_use]
    pub fn spear_state(&self) -> Option<&spear::SpearState> {
        self.core.spear.as_ref()
    }

    /// Spear events of the last stepped tick, in emission order.
    #[must_use]
    pub fn spear_events(&self) -> &[spear::SpearEvent] {
        self.core
            .spear
            .as_ref()
            .map_or(&[], spear::SpearState::events)
    }

    /// Height channel vertical speeds, mm per tick; empty while off.
    #[must_use]
    pub fn rise(&self) -> &[i32] {
        &self.core.rise
    }

    /// True while the height channel is armed.
    #[must_use]
    pub fn height(&self) -> bool {
        self.core.height
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

/// `Inputs` (PreUpdate): the tape slice of this tick lands on its slots and
/// the tick-start poses are snapshotted for the body column.
fn sys_inputs(core: &mut Core, _: &mut Ctx) {
    core.before = core.states;
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

/// `Ai` (PreUpdate): rules v2 plans the tick's fights, brains fill the
/// slots the tape left empty, the outside controller may take slots of its
/// own, then every slot's merged intent lands in the AI column.
fn sys_ai(core: &mut Core, _: &mut Ctx) {
    let n = core.fighters;
    let v2 = core.rules.version == FIGHT_RULES_V2;
    let plan = v2.then(|| engage::Plan::build(&core.states, &core.fight, &core.rules.teams, n));
    for slot in 0..n {
        if core.from_tape[slot] {
            continue;
        }
        let mut percept = percept(core, slot);
        if let Some(plan) = &plan {
            percept = plan.aim(percept, slot, &core.states, &core.fight);
        }
        let decided = core.brains[slot].decide(&percept);
        // Rules v2: the brain's intent wish is facing-relative (forward,
        // side); v1 keeps the frozen world-axis reading.
        core.row[slot] = if v2 {
            brain_wish_to_world(decided, core.brains[slot].intent(), &percept)
        } else {
            decided
        };
    }
    if let Some(mut decider) = core.controller.take() {
        let mut views = [SlotView::default(); MAX_SLOTS];
        for (slot, view) in views.iter_mut().enumerate().take(n) {
            let facing = core.last_inputs[slot].map_or(0, |input| input.facing);
            *view =
                controller::slot_view(core.states[slot].pos, &core.fight.fighters[slot], facing);
        }
        let view = ControlView {
            tick: core.tick,
            seed: core.config.seed,
            slots: &views[..n],
        };
        controller::drive(&mut *decider, &view, &mut core.row[..n], &core.from_tape);
        // Script effects land on fight state here, one system before
        // `sys_rules` resolves combat; an empty drain keeps the legacy
        // spine run exactly.
        let effects = decider.take_effects();
        controller::apply_script_effects(&mut core.fight, &effects);
        core.controller = Some(decider);
    }
    // The jump edge is read against the previous latch before it moves.
    for slot in 0..n {
        let input = core.row[slot];
        let before = core.last_inputs[slot].is_some_and(|prev| prev.buttons.0 & JUMP != 0);
        let now = input.buttons.0 & JUMP != 0;
        core.jump_edges[slot] = now && !before;
        core.jump_releases[slot] = !now && before;
        core.last_inputs[slot] = Some(input);
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

/// `Rules` (Update): strict combat over the merged intents, then the
/// balance, poise and match observer tracks read the tick's outcomes.
fn sys_rules(core: &mut Core, _: &mut Ctx) {
    let n = core.fighters;
    if core.rules.version == FIGHT_RULES_V2 {
        // Rules v2: pre-move poses in integer mm, fixed-point facing from
        // the absolute IN1 facing, one blade per slot along it.
        let mut poses = [strike_pose(0, &core.states[0], &core.row[0], 0); MAX_SLOTS];
        for (slot, input) in core.row.iter().enumerate().take(n) {
            let id = u8::try_from(slot).unwrap_or(u8::MAX);
            poses[slot] = strike_pose(id, &core.states[slot], input, core.rules.teams[slot]);
        }
        let tables = if core.config.table_version == TABLES_VERSION_V2 {
            &engine_combat::fight::TABLES_V2
        } else {
            &TABLES_V1
        };
        if try_step_v2_with_cues_into(
            &core.row[..n],
            &poses[..n],
            &mut core.fight,
            tables,
            &mut core.events,
            &mut core.last_cues,
        )
        .is_err()
        {
            core.failed = true;
            return;
        }
    } else if try_step_into(&core.row[..n], &mut core.fight, &mut core.events).is_err() {
        core.failed = true;
        return;
    }
    if core.rules.balance
        && authority::feed_balance(&mut core.balance, &mut core.stagger, &core.events).is_none()
    {
        core.failed = true;
        return;
    }
    // Combat-depth poise beside the fight step: route this tick's fight
    // events into victim poise, then advance the meters. Presentation only,
    // never hashed, so V1 bouts replay byte-identical.
    if core.rules.balance {
        core.depth.observe(&core.events);
    }
    core.depth.tick();
    // The match track resolves beside the fight path only while
    // `match_live` holds, over the pre-move poses.
    if core.match_live {
        let row = authority::match_observer_row(
            &mut core.match_track,
            &core.states,
            &core.row,
            n,
            core.tick,
        );
        core.match_row = Some(row);
    }
}

/// `Motion` (Update): each live fighter's wish drives its locomotion; the
/// requested root delta resolves once (arena clamp, then obstacles) and
/// moves the pose. Knocked-out fighters hold pose. Then the height channel,
/// the spear grapple and the collision pass run, in the legacy order.
fn sys_motion(core: &mut Core, _: &mut Ctx) {
    let n = core.fighters;
    let limit = wall_limit(core.config.half_extent_m);
    let tick = core.tick;
    for slot in 0..n {
        if !move_slot(core, slot, limit, tick) {
            core.failed = true;
            return;
        }
    }
    after_motion(core, n);
}

/// One slot's motion: the merged wish drives the locomotion machine, the
/// requested root delta resolves once and moves the pose. A knocked-out
/// fighter holds its pose. `false` when a rule refused.
fn move_slot(core: &mut Core, slot: usize, limit: f32, tick: u32) -> bool {
    let input = core.row[slot];
    // The spear bit only exists in a spear match; elsewhere it is masked out
    // so a stray press cannot reach the hash.
    core.buttons[slot] = if core.spear.is_some() {
        input.buttons.0
    } else {
        input.buttons.0 & !THROW_SPEAR
    };
    if core.fight.fighters[slot].hp_permille == 0 {
        // Held pose: no motion this tick.
        return true;
    }
    // Airborne follows the controller's grounded flag from the last tick
    // (`sim` only); the default leg stays grounded on the flat floor.
    #[cfg(feature = "sim")]
    core.locos[slot].set_airborne(!core.controllers[slot].grounded());
    let (speed, dir) = loco_wish(&input);
    // A fallen or catching fighter is down: its wish stops and its root
    // holds (the locomotion machine still eases to idle).
    let down = core.rules.balance
        && core
            .balance
            .get(slot)
            .is_some_and(|state| matches!(state.gait(), GaitPhase::Fallen | GaitPhase::Catching));
    let speed = if down { 0.0 } else { speed };
    // A fresh press on the floor takes off through the height channel;
    // releasing early cuts the rise (the hold cut). The first press arms the
    // channel, so a default match and its hashes before the press stay
    // floor-plane.
    if core.jump_edges[slot] && !down && core.arm_jump() {
        let grounded = core.rise[slot] == 0 && core.states[slot].pos[1] <= 0.0;
        if grounded {
            core.rise[slot] = JUMP_RISE_MM_TICK;
        }
    }
    if core.jump_releases[slot] && core.height && core.rise[slot] > 0 {
        core.rise[slot] /= 2;
    }
    let interrupt = interrupt_for_slot(&core.events, slot_id(slot));
    let Ok(root) = core.locos[slot].update_root(speed, dir, tick, interrupt) else {
        return false;
    };
    let requested = if down { [0.0, 0.0] } else { root.dp };
    core.states[slot].yaw = yaw_from_centideg(input.facing);
    #[cfg(feature = "sim")]
    {
        // Single resolution: the blocked remainder dies here and is never
        // reapplied on a later tick.
        let resolved = core.controllers[slot].move_and_slide(requested, FIXED_DT);
        let (dx, dz) = (resolved.displacement[0], resolved.displacement[1]);
        let state = &mut core.states[slot];
        // Backstop clamp: controller walls sit one skin inside this limit, so
        // it never fires; it keeps the shared wall inset live on both legs.
        state.pos[0] = canon_zero(state.pos[0] + dx).clamp(-limit, limit);
        state.pos[2] = canon_zero(state.pos[2] + dz).clamp(-limit, limit);
    }
    #[cfg(not(feature = "sim"))]
    {
        // Single resolution without physics: one clamp, then the static
        // world push-out when a kit world is supplied (the spine step's
        // rule). The blocked remainder is never reapplied on a later tick.
        let state = &mut core.states[slot];
        let from = [state.pos[0], state.pos[2]];
        let raw = [state.pos[0] + requested[0], state.pos[2] + requested[1]];
        let to = [raw[0].clamp(-limit, limit), raw[1].clamp(-limit, limit)];
        let placed = if core.world.is_empty() {
            to
        } else {
            core.world.resolve(from, to, FIGHTER_RADIUS_M, limit)
        };
        state.pos[0] = canon_zero(placed[0]);
        state.pos[2] = canon_zero(placed[1]);
    }
    true
}

/// The tail of the tick's motion: the height channel, the spear grapple and
/// the collision pass, in the legacy order.
fn after_motion(core: &mut Core, n: usize) {
    // Height channel (row SG5): gravity and the landing, after the walk and
    // before the spear, so a swing constrains a body that has already fallen
    // this tick. Off means the floor plane, untouched.
    if core.height {
        authority::fall_states(&mut core.states[..n], &mut core.rise, n, &core.last_inputs);
    }
    // Spear grapple: throws, flights, tether physics and the bodies it
    // drags, between the walking and the collision pass. Frozen with
    // everything else while hit-stop holds.
    if core.fight.hitstop == 0 {
        if let Some(mut grapple) = core.spear.take() {
            let tables = if core.config.table_version == TABLES_VERSION_V2 {
                &engine_combat::fight::TABLES_V2
            } else {
                &TABLES_V1
            };
            grapple.step(&mut spear::Env {
                inputs: &core.row[..n],
                states: &mut core.states[..n],
                fight: &mut core.fight,
                rules: &core.rules,
                world: &core.world,
                tables,
                half_extent_m: core.config.half_extent_m,
                rise: &mut core.rise,
            });
            core.spear = Some(grapple);
        }
    }
    // One collision pass after every slot moved (default leg). The body-push
    // shove needs the balance states, so the pass takes them.
    if core.rules.collides() {
        crate::collide::collide_states_shove(
            &mut core.states[..n],
            &mut core.balance[..n],
            &core.rules,
            core.config.half_extent_m,
            n,
        );
    }
}

/// `Extract` (PostUpdate): the landed hits resolve to cuts, the balance and
/// the limb pool step, sword hits strike the crate prop, the prop steps and
/// the checkpoint folds over the legacy shapes.
fn sys_extract(core: &mut Core, _: &mut Ctx) {
    let n = core.fighters;
    // V2 landed hits resolve to cuts before the limb pool steps, so a limb
    // loses its first tick immediately. V1 never enters this path.
    core.cuts = if core.rules.version == FIGHT_RULES_V2 {
        authority::resolve_cuts(&core.states[..n], core.tick, &mut core.limbs, &core.events)
    } else {
        Vec::new()
    };
    // Observe each body's post-move centre, then advance balance. The
    // stagger memory steps beside balance, the limb pool behind it.
    if core.rules.balance {
        for slot in 0..n {
            let pos = core.states[slot].pos;
            if core.balance[slot]
                .observe(&[pos], &[BALANCE_BODY_MASS_KG])
                .is_err()
            {
                core.failed = true;
                return;
            }
            core.balance[slot].step();
        }
        for memory in core.stagger.iter_mut() {
            memory.step();
        }
        core.limbs.step();
    }
    // Every sword hit strikes the chunk nearest the victim, one sword unit
    // above the fracture gate; the prop steps each tick and the drained
    // breaks join the hash below.
    for event in &core.events {
        if let FightEvent::Hit { victim, .. } = *event {
            let slot = usize::from(victim);
            if slot < n {
                let pos = core.states[slot].pos;
                if let Some(chunk) = nearest_chunk(&core.destruction, pos) {
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

    #[cfg(feature = "sim")]
    let physics_hash = authority::controller_hash(&core.controllers);
    #[cfg(not(feature = "sim"))]
    let physics_hash = 0u64;

    let mut hash = hash_state_with_sim(
        core.tick,
        &core.states[..n],
        &core.buttons[..n],
        &core.fight,
        &core.events,
        physics_hash,
        &core.locos,
        destruction_hash,
        &core.breaks,
        cfg!(feature = "sim"),
    );
    hash = core
        .rules
        .fold(hash, n, &core.balance, &core.stagger, &core.limbs);
    // The spear state folds after the rules and before the world; a match
    // without one leaves the hash untouched.
    hash = core.spear.as_ref().map_or(hash, |spear| spear.fold(hash));
    // The height channel folds only when armed, so the golden bytes stay
    // exactly as they were on a floor-plane match.
    if core.height {
        hash = fold_rise(&core.rise[..n], hash);
    }
    core.base_hash = hash;
    // A supplied world folds after the rules: the empty world digests to
    // zero and leaves the golden bytes untouched.
    let hash = fold_world(core.world.digest(), hash);
    core.checkpoints.push(Checkpoint {
        tick: core.tick,
        hash,
    });
    write_columns(core, n);
    if core.match_live {
        core.match_track.advance();
    }
    core.tick += 1;
}

/// Write this tick's columns from the authority: the pose column from the
/// authoritative poses, the body column from the tick-start snapshot, the
/// vitals from the fight state and the gait from the locomotion machines.
fn write_columns(core: &mut Core, n: usize) {
    let states = core.states;
    let before = core.before;
    let fight = core.fight;
    let mut anims = [AnimState::default(); MAX_SLOTS];
    for (slot, anim) in anims.iter_mut().enumerate().take(n) {
        let loco = &core.locos[slot];
        *anim = AnimState {
            loco: loco_disc(loco.state),
            phase: loco.phase(),
            facing: loco.facing(),
            airborne: loco.is_airborne(),
            interrupt: loco.interrupt_active(),
            pose_ref: 0,
        };
    }
    core.spine.for_each_mut(|slot, view| {
        if slot >= n {
            return;
        }
        view.transform.pos = states[slot].pos;
        view.transform.yaw = states[slot].yaw;
        view.body.vel = [
            raw_fixed(states[slot].pos[0] - before[slot].pos[0]),
            raw_fixed(states[slot].pos[2] - before[slot].pos[2]),
        ];
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
        *view.anim = anims[slot];
    });
}

/// One fighter's [`Percept`]: own pose, nearest foe (strict `<`, ties keep
/// the lowest slot), separation and the foe's attack flag. Same mapping as
/// the legacy step, read from the authoritative poses.
fn percept(core: &Core, slot: usize) -> Percept {
    let me = core.states[slot];
    let mut foe = (slot + 1) % core.fighters;
    let mut best = f32::INFINITY;
    for (index, other) in core.states.iter().enumerate().take(core.fighters) {
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
    let foe_state = core.states[foe];
    let dist_m = if best.is_finite() { best.sqrt() } else { 0.0 };
    Percept::new(
        me.pos,
        me.yaw,
        foe_state.pos,
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

/// Fighter slot as the `u8` id the contracts carry (slots stay below 32).
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

    /// The same config under the v2 move tables (rules v2).
    fn config_v2(seed: u64, ticks: u32, fighters: u8) -> ArenaConfig {
        ArenaConfig {
            table_version: 2,
            ..ArenaConfig::try_new(seed, ticks, fighters, 12.0).expect("valid config")
        }
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
            core::f32::consts::PI,
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

    /// A full-size match (the participant cap) must build a spine.
    ///
    /// 2026-10-21 (lead): the spine's own slot cap was 16 while the
    /// participant cap is 32, so `spawn_columns` reserved 16 slots, the 17th
    /// `Spine::spawn` refused with `TooManySlots`, and the spine step
    /// answered `None` - which `run_spine` turns into an empty checkpoint
    /// list. The legacy path seated the same fight, so the two paths
    /// disagreed on a match size the engine claims to support.
    #[test]
    fn a_full_size_match_builds_a_spine() {
        let cfg = config(9, 60, MAX_SLOTS as u8);
        let tape = FighterTape(Vec::new());
        let legacy = run(&cfg, &tape);
        let spine = run_spine(&cfg, &tape, &MatchWorld::empty());
        assert_eq!(legacy.len(), 60, "the legacy path seats the full match");
        assert_eq!(
            spine.len(),
            60,
            "the spine path seats the full match too"
        );
        assert_eq!(spine, legacy, "both paths agree on the full match");
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
    #[expect(
        clippy::needless_range_loop,
        reason = "slot indexes the same column of every tick row"
    )]
    fn stuck_slots(history: &[Vec<[f32; 2]>], kill: Option<u32>, ticks: u32) -> Vec<usize> {
        let step = crate::fork::STUCK_SPEED_MPS * DT_S;
        let last = usize::try_from(kill.unwrap_or_else(|| ticks.saturating_sub(1))).unwrap_or(0);
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

    /// The kinematic leg resolves every root delta against the kit world,
    /// so a fighter never stands inside a wall. The `sim` leg resolves
    /// through the physics controller, which owns the arena walls only,
    /// exactly like the legacy step; the claim is the kinematic track's.
    #[cfg(not(feature = "sim"))]
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

    /// The v2 track reaches the spine: the legacy rules fold, the balance
    /// and the limb pool step, and the checkpoint moves off the v1 bytes.
    #[test]
    fn spine_v2_rules_fold_into_the_checkpoint() {
        let cfg = config(31, 120, 2);
        let v1 = run(&cfg, &FighterTape(Vec::new()));
        let v2_cfg = config_v2(31, 120, 2);
        let v2 = run(&v2_cfg, &FighterTape(Vec::new()));
        assert_ne!(v1.last(), v2.last(), "rules v2 must move the hash");
        let spine = run_spine(&v2_cfg, &FighterTape(Vec::new()), &MatchWorld::empty());
        assert_eq!(spine, v2, "the spine must fold the same v2 bytes");
    }

    /// The sim leg: the physics controllers own the poses, so the spine
    /// must agree with the legacy step tick for tick.
    #[cfg(feature = "sim")]
    #[test]
    fn spine_sim_controllers_match_the_legacy_step() {
        let cfg = config(41, 60, 2);
        let tape = FighterTape(Vec::new());
        let mut legacy = crate::Sim::build(&cfg, &tape, false, false).expect("legacy builds");
        let mut spine = SpineMatch::build(&cfg, &tape, MatchWorld::empty()).expect("spine builds");
        while spine.tick() < cfg.ticks {
            legacy.step(&tape).expect("legacy steps");
            let stepped = spine.step().expect("spine steps");
            let mine = legacy
                .checkpoints
                .last()
                .copied()
                .expect("legacy checkpoint");
            assert_eq!(mine.hash, stepped.hash, "tick {}", spine.tick());
            for slot in 0..usize::from(cfg.fighters) {
                let (a, b) = (&legacy.states[slot], &spine.states()[slot]);
                assert_eq!(a.pos, b.pos, "slot {slot} tick {}", spine.tick());
                assert_eq!(a.yaw, b.yaw, "slot {slot} tick {}", spine.tick());
            }
        }
    }
}
