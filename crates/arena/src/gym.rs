//! gym: headless observer (snapshots, live stepping env, batch step).
use super::{
    run_sim, spawn_states, ArenaConfig, FighterId, FighterInput, FighterTape, Sim,
    MAX_PARTICIPANTS, MAX_SLOTS,
};
use engine_combat::fight::{
    FighterState as CombatFighter, ATTACK_STAMINA_COST, DODGE_STAMINA_COST, MAX_STAMINA,
};
use engine_combat::FightEvent;

/// Discrete gym actions covered by [`GymFighterState::legal_mask`], in
/// mask order: attack, block, dodge. Mirrors the three known input bits
/// (`ATTACK`, `BLOCK`, `DODGE`); movement wishes stay continuous.
pub const GYM_NUM_ACTIONS: usize = 3;
/// Mask index of the attack action.
pub const GYM_ACTION_ATTACK: usize = 0;
/// Mask index of the block action.
pub const GYM_ACTION_BLOCK: usize = 1;
/// Mask index of the dodge action.
pub const GYM_ACTION_DODGE: usize = 2;

/// What a fighter is doing, one word for a gym observer. Derived from the
/// combat state with a fixed precedence (first match wins): out, knocked
/// down, staggered, attacking, dodging, blocking, idle.
///
/// Consumer: endpoint `sim.observe` / `sim.step` / `play.step` map the name
/// into their observation DTO. Data owner: derived per read from combat
/// state, never stored or hashed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GymPhase {
    /// Free to act: no attack, dodge, block, stagger or knockdown running.
    Idle,
    /// An attack is in flight (startup through recovery).
    Attacking,
    /// Block is held.
    Blocking,
    /// A dodge is in flight.
    Dodging,
    /// Hit or parry stagger: no new action until it ends.
    Staggered,
    /// Knocked down but still holding hit points.
    KnockedDown,
    /// Zero hit points: out of the fight.
    Out,
}

impl GymPhase {
    /// Classify one fighter's combat state.
    pub const fn of(state: &CombatFighter) -> Self {
        if state.hp_permille == 0 {
            Self::Out
        } else if state.knockdown > 0 {
            Self::KnockedDown
        } else if state.react > 0 {
            Self::Staggered
        } else if state.is_attacking {
            Self::Attacking
        } else if state.is_dodging {
            Self::Dodging
        } else if state.is_blocking {
            Self::Blocking
        } else {
            Self::Idle
        }
    }

    /// Stable lowercase wire name (`idle`, `attacking`, `blocking`,
    /// `dodging`, `staggered`, `knocked_down`, `out`).
    pub const fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Attacking => "attacking",
            Self::Blocking => "blocking",
            Self::Dodging => "dodging",
            Self::Staggered => "staggered",
            Self::KnockedDown => "knocked_down",
            Self::Out => "out",
        }
    }
}

/// One fighter's gym-visible snapshot: pose, health, stamina, phase plus
/// the RL readout (reward, legal mask, velocity).
///
/// Consumer: endpoint `sim.*` and `play.step` tools map this into their
/// observation DTO. Data owner: the caller owns the returned values (fresh
/// `Vec`). Units: meters for position, radians for yaw, permille for health,
/// stamina points (`0..=100`), reward `0..=1`, meters per second for
/// velocity. Observed only: nothing here feeds [`run`](crate::run) checkpoints.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GymFighterState {
    /// Fighter slot, `0..config.fighters`.
    pub id: FighterId,
    /// Feet position `[x, y, z]` in meters.
    pub pos: [f32; 3],
    /// Facing yaw in radians.
    pub yaw: f32,
    /// Health in permille, `0..=1000`.
    pub hp: u16,
    /// Stamina points, `0..=100`.
    pub stamina: u16,
    /// What the fighter is doing now.
    pub phase: GymPhase,
    /// Immediate survival reward: remaining health fraction `0..=1`.
    pub reward: f32,
    /// Legal discrete actions in [`GYM_NUM_ACTIONS`] order
    /// (`attack`, `block`, `dodge`); `false` while staggered, down or out,
    /// or without the stamina to start the move.
    pub legal_mask: [bool; GYM_NUM_ACTIONS],
    /// Planar velocity `[vx, vy, vz]` in meters per second (`vy` is `0` on
    /// the flat floor), from the locomotion machine's speed times facing.
    pub velocity: [f32; 3],
}

/// Survival reward for one combat state: health fraction `0..=1`.
pub fn reward_of(fight: &CombatFighter) -> f32 {
    (f32::from(fight.hp_permille) / 1000.0).clamp(0.0, 1.0)
}

/// Legal discrete actions for one combat state, in [`GYM_NUM_ACTIONS`]
/// order (`attack`, `block`, `dodge`). Follows the fight step rules:
/// nothing is legal while the fighter cannot act; attack needs full
/// start conditions plus attack stamina; dodge needs dodge stamina;
/// block holds whenever the fighter can act.
pub fn legal_mask_of(fight: &CombatFighter) -> [bool; GYM_NUM_ACTIONS] {
    if !fight.can_act() {
        return [false, false, false];
    }
    let mut mask = [false, false, false];
    mask[GYM_ACTION_ATTACK] =
        !fight.is_attacking && !fight.is_dodging && fight.stamina >= ATTACK_STAMINA_COST;
    mask[GYM_ACTION_BLOCK] = true;
    mask[GYM_ACTION_DODGE] = !fight.is_dodging && fight.stamina >= DODGE_STAMINA_COST;
    mask
}

/// Post-step snapshot for gym observers: tick count, digest and fighters.
///
/// The digest equals the [`run`](crate::run) final hash for the same config plus tape,
/// so gym replays and `play.run` receipts agree. Observed only: nothing
/// here feeds [`run`](crate::run) checkpoints.
#[derive(Clone, PartialEq, Debug)]
pub struct GymSnapshot {
    /// Ticks simulated: the 1-indexed count `config.ticks`.
    pub tick: u32,
    /// State digest: the checkpoint `tick - 1` hash, `0` before the first tick.
    pub digest: u64,
    /// Fighter snapshots in slot order, one per participant.
    pub fighters: Vec<GymFighterState>,
}

/// Spawn fighter snapshots before the first tick: seed-expanded poses plus
/// full health. The endpoint `sim.reset` observation at tick 0.
///
/// `None` when the fighter count or extent is outside the arena ranges.
/// Deterministic over its inputs on the same machine.
///
/// # Example
///
/// ```rust
/// use engine_arena::initial_state;
///
/// let states = initial_state(7, 2, 20.0).expect("valid spawn");
/// assert_eq!(states.len(), 2);
/// assert!(states.iter().all(|state| state.hp == 1000));
/// ```
pub fn initial_state(seed: u64, fighters: u8, half_extent_m: f32) -> Option<Vec<GymFighterState>> {
    if !(2..=MAX_PARTICIPANTS).contains(&fighters) {
        return None;
    }
    if !half_extent_m.is_finite() || !(5.0..=100.0).contains(&half_extent_m) {
        return None;
    }
    let states = spawn_states(seed, fighters, half_extent_m);
    let mut out = Vec::with_capacity(states.len());
    for state in states {
        let idle = CombatFighter::default();
        out.push(GymFighterState {
            id: state.id,
            pos: state.pos,
            yaw: state.yaw,
            hp: engine_combat::fight::MAX_HP_PERMILLE,
            stamina: MAX_STAMINA,
            phase: GymPhase::Idle,
            reward: reward_of(&idle),
            legal_mask: legal_mask_of(&idle),
            velocity: [0.0, 0.0, 0.0],
        });
    }
    Some(out)
}

/// Every live fighter of `sim` in slot order: pose from the kinematic state,
/// health, stamina and phase from the combat state, plus the RL readout
/// (reward, legal mask, locomotion velocity). Observed only, never hashed.
fn fighters_of(sim: &Sim) -> Vec<GymFighterState> {
    let mut out = Vec::with_capacity(sim.fighters);
    for (slot, state) in sim.states.iter().enumerate().take(sim.fighters) {
        let fight = &sim.fight.fighters[slot];
        let loco = &sim.locos[slot];
        let facing = loco.facing();
        let speed = loco.speed();
        out.push(GymFighterState {
            id: state.id,
            pos: state.pos,
            yaw: state.yaw,
            hp: fight.hp_permille,
            stamina: fight.stamina,
            phase: GymPhase::of(fight),
            reward: reward_of(fight),
            legal_mask: legal_mask_of(fight),
            velocity: [speed * facing[0], 0.0, speed * facing[1]],
        });
    }
    out
}

/// Read the hashed snapshot for one config plus sorted tape: tick-zero spawn
/// when the tape is empty, otherwise the last checkpoint digest plus fighter
/// states. `None` on any validation failure, never partial. Determinism
/// scope: same config plus sorted tape twice gives the same snapshot.
///
/// Consumer: endpoint `sim.observe`/`sim.step` and [`vec_step`]; data owner
/// is the caller (fresh snapshot per call, meters plus permille plus digests).
pub fn observe_state(config: &ArenaConfig, tape: &FighterTape) -> Option<GymSnapshot> {
    if config.validate().is_err() || tape.validate().is_err() {
        return None;
    }
    let fighters = usize::from(config.fighters);
    for input in &tape.0 {
        if input.tick >= config.ticks || usize::from(input.fighter.0) >= fighters {
            return None;
        }
    }
    let sim = run_sim(config, tape, false, false)?;
    let digest = sim.checkpoints.last().map_or(0, |point| point.hash);
    Some(GymSnapshot {
        tick: config.ticks,
        digest,
        fighters: fighters_of(&sim),
    })
}

/// Kind of one combat event a gym observer reads back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GymEventKind {
    /// Full damage landed.
    Hit,
    /// A held block soaked the hit down to chip damage.
    Block,
    /// A block inside the parry window: no damage, attacker staggered.
    Parry,
    /// Dodge i-frames negated the hit.
    Dodge,
    /// An attack or dodge was pressed without enough stamina.
    StaminaOut,
    /// The victim dropped to zero hit points.
    Kill,
}

impl GymEventKind {
    /// Stable lowercase wire name (`hit`, `block`, `parry`, `dodge`,
    /// `stamina_out`, `kill`).
    pub const fn name(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Block => "block",
            Self::Parry => "parry",
            Self::Dodge => "dodge",
            Self::StaminaOut => "stamina_out",
            Self::Kill => "kill",
        }
    }
}

/// One combat event with the 0-indexed tick it resolved on.
///
/// Consumer: endpoint `sim.observe` keeps the last events in a bounded ring.
/// Data owner: [`GymLive`] fills it per step; the caller copies out what it
/// keeps. Units: fixed ticks, permille for `amount`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GymEvent {
    /// 0-indexed tick the event resolved on.
    pub tick: u32,
    /// What happened.
    pub kind: GymEventKind,
    /// Attacker slot; `None` for [`GymEventKind::StaminaOut`].
    pub attacker: Option<u8>,
    /// Victim slot (the fighter itself for [`GymEventKind::StaminaOut`]).
    pub victim: u8,
    /// Damage in permille for a hit, chip damage for a block, else `0`.
    pub amount: u16,
}

impl GymEvent {
    /// Map one combat event stamped with its tick.
    pub const fn of(tick: u32, event: FightEvent) -> Self {
        let (kind, attacker, victim, amount) = match event {
            FightEvent::Hit {
                attacker,
                victim,
                damage,
            } => (GymEventKind::Hit, Some(attacker), victim, damage),
            FightEvent::Blocked {
                attacker,
                victim,
                chip,
            } => (GymEventKind::Block, Some(attacker), victim, chip),
            FightEvent::Parried { attacker, victim } => {
                (GymEventKind::Parry, Some(attacker), victim, 0)
            }
            FightEvent::Dodged { attacker, victim } => {
                (GymEventKind::Dodge, Some(attacker), victim, 0)
            }
            FightEvent::StaminaOut { fighter } => (GymEventKind::StaminaOut, None, fighter, 0),
            FightEvent::Knockdown { attacker, victim } => {
                (GymEventKind::Kill, Some(attacker), victim, 0)
            }
        };
        Self {
            tick,
            kind,
            attacker,
            victim,
            amount,
        }
    }
}

/// Incremental headless match with a state readout: one cold authority
/// `Sim` stepped exactly once per [`GymLive::step`] call, so a session that
/// asks once per tick pays constant cost instead of re-running the match
/// from tick 0 (the prefix runners and [`observe_state`] do that).
///
/// Same sim, same step and same hash path as [`run`](crate::run): after `n`
/// steps over a tape, [`GymLive::snapshot`] equals [`observe_state`] over the
/// same config with `ticks = n` and the tape's first `n` ticks. `config.ticks`
/// only caps how far the env may step; it never enters the hash, so one env
/// built with a long cap serves every shorter prefix. Unlike
/// [`LiveMatch`](crate::LiveMatch) it exposes the post-step fighter state
/// (hp, stamina, phase, pose) and skips the dataset recorder leg.
///
/// Consumer: endpoint `sim.step`, `sim.observe` and `play.step` hold one per
/// door session. Data owner: this env owns its sim; the caller owns the tape
/// and hands in one tick's inputs per call. Units: fixed ticks at
/// [`TICK_HZ`](crate::TICK_HZ), meters, radians, permille. Determinism scope:
/// same as [`run`](crate::run). Error policy: a rejected input slice returns
/// `None` and steps nothing; a failed step poisons the env and the caller
/// rebuilds from tick 0.
pub struct GymLive {
    sim: Sim,
    /// One-tick tape scratch: the caller's inputs for the stepped tick only.
    tape: FighterTape,
    /// Events of the last stepped tick, reused every step.
    events: Vec<GymEvent>,
    digest: u64,
    poisoned: bool,
}

impl GymLive {
    /// Fresh env at tick 0 over a validated `config`; `None` when the config
    /// is invalid (same rule as [`run`](crate::run)).
    pub fn new(config: &ArenaConfig) -> Option<Self> {
        let mut sim = Sim::build(config, &FighterTape::default(), false, false)?;
        // The live log keeps one row: drop the whole-match reservations.
        sim.checkpoints = Vec::with_capacity(1);
        sim.events = Vec::with_capacity(1);
        sim.breaks = Vec::with_capacity(1);
        Some(Self {
            sim,
            tape: FighterTape(Vec::with_capacity(MAX_SLOTS)),
            events: Vec::with_capacity(MAX_SLOTS),
            digest: 0,
            poisoned: false,
        })
    }

    /// Next tick [`GymLive::step`] simulates (ticks stepped so far).
    #[must_use]
    pub const fn tick(&self) -> u32 {
        self.sim.tick
    }

    /// Step tick [`GymLive::tick`] with `inputs`: that tick's human inputs,
    /// `(fighter)`-sorted with no duplicates, each stamped with the stepped
    /// tick and a slot below the fighter count (brains fill the rest, as in
    /// the prefix runners). Returns the tick's combat events. A slice
    /// breaking those rules, a finished env or a poisoned one returns `None`
    /// and steps nothing.
    pub fn step(&mut self, inputs: &[FighterInput]) -> Option<&[GymEvent]> {
        let tick = self.sim.tick;
        if self.poisoned || tick >= self.sim.config.ticks {
            return None;
        }
        self.tape.0.clear();
        self.tape.0.extend_from_slice(inputs);
        let fighters = self.sim.fighters;
        let stray =
            |input: &FighterInput| input.tick != tick || usize::from(input.fighter.0) >= fighters;
        if self.tape.0.iter().any(stray) || self.tape.validate().is_err() {
            return None;
        }
        if self.advance(tick).is_none() {
            self.poisoned = true;
            return None;
        }
        Some(&self.events)
    }

    /// One sim step plus the one-row log rotation and the event readout.
    fn advance(&mut self, tick: u32) -> Option<()> {
        self.sim.cursor = 0;
        self.sim.step(&self.tape)?;
        self.digest = self.sim.checkpoints.pop()?.hash;
        let row = self.sim.events.pop()?;
        self.sim.breaks.clear();
        self.events.clear();
        for event in &row.events {
            self.events.push(GymEvent::of(tick, *event));
        }
        Some(())
    }

    /// The state after the last step: ticks stepped, digest (the last
    /// checkpoint hash, `0` before the first step) and every fighter.
    #[must_use]
    pub fn snapshot(&self) -> GymSnapshot {
        GymSnapshot {
            tick: self.sim.tick,
            digest: self.digest,
            fighters: fighters_of(&self.sim),
        }
    }
}

/// One headless env for a vectorized step: config plus its sorted tape.
///
/// Consumer: endpoint `sim.vec_step` maps one batch spec per env into this
/// value, then reads the per-env snapshots back. Data owner: the caller
/// owns the returned snapshots (fresh `Vec`s per env). Units: meters,
/// fixed ticks, permille, `u64` digests. Threading: `Send + Sync`, no
/// shared mutable state between envs. Determinism scope: same config plus
/// sorted tape twice gives the same snapshot on the same machine.
#[derive(Clone, PartialEq, Debug)]
pub struct BatchEnv {
    /// Match config for this env (`Copy + Send + Sync`, never mutated).
    pub config: ArenaConfig,
    /// Sorted input tape for this env.
    pub tape: FighterTape,
}

/// Vectorized headless step: one call steps all N worlds.
///
/// Each env clones nothing shared: every worker reads only its own
/// [`BatchEnv`] and runs the same [`observe_state`] path as a single step,
/// so the single-step behavior is untouched. Results return in input order
/// (slot order, never completion order); an invalid env yields `None` at
/// its index without poisoning the rest. Determinism is per seed: same
/// seed plus sorted tape gives the same digest across runs.
///
/// Consumer: endpoint batch DTOs plus the `headless_ticks` bench.
/// Threading: one `std::thread` scope per call with at most one worker per
/// core, each stepping a contiguous run of envs into its own output slots
/// (the pool lives here in arena, never in endpoint; std only, so the
/// headless default graph gains no dependency). No shared mutable state
/// between envs.
/// Error policy: invalid envs map to `None`, never partial; a panic inside
/// an env is a bug and propagates to the caller.
///
/// # Example
///
/// ```rust
/// use engine_arena::{vec_step, BatchEnv, ArenaConfig};
/// use engine_input::FighterTape;
///
/// let config = ArenaConfig::try_new(2040, 4, 2, 20.0).expect("valid config");
/// let envs = vec![
///     BatchEnv { config, tape: FighterTape::default() },
///     BatchEnv { config, tape: FighterTape::default() },
/// ];
/// let snaps = vec_step(&envs);
/// assert_eq!(snaps.len(), 2);
/// assert_eq!(snaps[0], snaps[1]);
/// ```
pub fn vec_step(envs: &[BatchEnv]) -> Vec<Option<GymSnapshot>> {
    // One output slot per env; each env runs a whole match, so this is per batch, not per tick.
    let mut out: Vec<Option<GymSnapshot>> = vec![None; envs.len()]; // alloc-ok: one result slot per env per batch
                                                                    // At most one worker per core, each owning a contiguous run of envs and
                                                                    // their output slots, so results land in input order with no locks.
    let workers = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .clamp(1, envs.len().max(1));
    let chunk = envs.len().div_ceil(workers).max(1);
    std::thread::scope(|scope| {
        for (batch, slots) in envs.chunks(chunk).zip(out.chunks_mut(chunk)) {
            scope.spawn(move || {
                for (env, slot) in batch.iter().zip(slots) {
                    *slot = observe_state(&env.config, &env.tape);
                }
            });
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run;
    use engine_input::{fighter::ATTACK, Buttons};

    /// Human slot 0 circle-strafes and swings every 30 ticks; slot 1 gets a
    /// sparse scripted nudge and the brains fill the rest.
    fn script(ticks: u32) -> FighterTape {
        let mut inputs = Vec::new();
        for tick in 0..ticks {
            let (mx, mz) = match (tick / 15) % 4 {
                0 => (100, 0),
                1 => (0, 100),
                2 => (-100, 0),
                _ => (0, -100),
            };
            let buttons = if tick.is_multiple_of(30) {
                Buttons(ATTACK)
            } else {
                Buttons::NONE
            };
            inputs.push(FighterInput {
                tick,
                fighter: FighterId(0),
                mx,
                mz,
                facing: 0,
                buttons,
            });
            if tick.is_multiple_of(7) {
                inputs.push(FighterInput {
                    tick,
                    fighter: FighterId(1),
                    mx: -50,
                    mz: 25,
                    facing: 9_000,
                    buttons: Buttons::NONE,
                });
            }
        }
        FighterTape(inputs)
    }

    fn tick_inputs(tape: &FighterTape, tick: u32) -> &[FighterInput] {
        let start = tape.0.partition_point(|input| input.tick < tick);
        let end = tape.0.partition_point(|input| input.tick <= tick);
        &tape.0[start..end]
    }

    #[test]
    fn live_env_matches_prefix_observe_tick_for_tick() {
        // The env's cap is long; every shorter prefix must still agree.
        let cap = ArenaConfig::try_new(2040, 600, 2, 20.0).expect("valid config");
        let tape = script(600);
        let mut live = GymLive::new(&cap).expect("live env");
        assert_eq!(live.snapshot().tick, 0);
        assert_eq!(live.snapshot().digest, 0, "no digest before the first step");
        assert!(
            live.snapshot()
                .fighters
                .iter()
                .all(|fighter| fighter.hp == 1000 && fighter.phase == GymPhase::Idle),
            "tick zero is full health and idle"
        );
        for tick in 0..60 {
            live.step(tick_inputs(&tape, tick)).expect("live step");
            let count = tick + 1;
            let prefix = ArenaConfig::try_new(2040, count, 2, 20.0).expect("valid prefix");
            let head = FighterTape(
                tape.0
                    .iter()
                    .copied()
                    .filter(|input| input.tick < count)
                    .collect(),
            );
            let want = observe_state(&prefix, &head).expect("prefix observe");
            assert_eq!(live.snapshot(), want, "snapshot after {count} ticks");
        }
        assert_eq!(live.tick(), 60);
    }

    #[test]
    fn live_env_events_equal_the_event_runner() {
        let config = ArenaConfig::try_new(2040, 600, 2, 20.0).expect("valid config");
        let tape = script(600);
        let want = crate::run_events(&config, &tape);
        let mut live = GymLive::new(&config).expect("live env");
        let mut seen = 0usize;
        for tick in 0..config.ticks {
            let got = live
                .step(tick_inputs(&tape, tick))
                .expect("live step")
                .to_vec();
            let expected: Vec<GymEvent> = want[tick as usize]
                .events
                .iter()
                .map(|event| GymEvent::of(tick, *event))
                .collect();
            assert_eq!(got, expected, "events at tick {tick}");
            seen += got.len();
        }
        assert!(seen > 0, "the scripted match fights");
        let points = run(&config, &tape);
        assert_eq!(
            live.snapshot().digest,
            points.last().expect("600 checkpoints").hash,
            "live digest equals the run final hash"
        );
        assert!(live.step(&[]).is_none(), "a finished env steps nothing");
        assert_eq!(live.tick(), config.ticks);
    }

    #[test]
    fn live_env_rejects_bad_slices_without_stepping() {
        let config = ArenaConfig::try_new(7, 8, 2, 12.0).expect("valid config");
        let mut live = GymLive::new(&config).expect("live env");
        let good = FighterInput {
            tick: 0,
            fighter: FighterId(0),
            mx: 10,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        };
        let other = FighterInput {
            fighter: FighterId(1),
            ..good
        };
        let wrong_tick = FighterInput { tick: 1, ..good };
        let bad_slot = FighterInput {
            fighter: FighterId(2),
            ..good
        };
        let bad_wish = FighterInput { mx: 101, ..good };
        for bad in [
            &[wrong_tick][..],
            &[bad_slot][..],
            &[bad_wish][..],
            &[good, good][..],
            &[other, good][..],
        ] {
            assert!(live.step(bad).is_none());
            assert_eq!(live.tick(), 0, "a refused slice steps nothing");
        }
        assert!(live.step(&[good]).is_some());
        assert_eq!(live.tick(), 1);
        let one_fighter = ArenaConfig {
            fighters: 1,
            ..config
        };
        assert!(GymLive::new(&one_fighter).is_none());
    }

    #[test]
    fn phase_follows_the_documented_precedence() {
        let mut state = CombatFighter::default();
        assert_eq!(GymPhase::of(&state), GymPhase::Idle);
        state.is_blocking = true;
        assert_eq!(GymPhase::of(&state), GymPhase::Blocking);
        state.is_dodging = true;
        assert_eq!(GymPhase::of(&state), GymPhase::Dodging);
        state.is_attacking = true;
        assert_eq!(GymPhase::of(&state), GymPhase::Attacking);
        state.react = 3;
        assert_eq!(GymPhase::of(&state), GymPhase::Staggered);
        state.knockdown = 9;
        assert_eq!(GymPhase::of(&state), GymPhase::KnockedDown);
        state.hp_permille = 0;
        assert_eq!(GymPhase::of(&state), GymPhase::Out);
        let names = [
            GymPhase::Idle,
            GymPhase::Attacking,
            GymPhase::Blocking,
            GymPhase::Dodging,
            GymPhase::Staggered,
            GymPhase::KnockedDown,
            GymPhase::Out,
        ]
        .map(GymPhase::name);
        assert_eq!(
            names,
            [
                "idle",
                "attacking",
                "blocking",
                "dodging",
                "staggered",
                "knocked_down",
                "out"
            ]
        );
    }

    #[test]
    fn combat_events_map_to_gym_events() {
        let cases = [
            (
                FightEvent::Hit {
                    attacker: 0,
                    victim: 1,
                    damage: 120,
                },
                GymEventKind::Hit,
                Some(0),
                1,
                120,
            ),
            (
                FightEvent::Blocked {
                    attacker: 1,
                    victim: 0,
                    chip: 15,
                },
                GymEventKind::Block,
                Some(1),
                0,
                15,
            ),
            (
                FightEvent::Parried {
                    attacker: 1,
                    victim: 0,
                },
                GymEventKind::Parry,
                Some(1),
                0,
                0,
            ),
            (
                FightEvent::Dodged {
                    attacker: 0,
                    victim: 1,
                },
                GymEventKind::Dodge,
                Some(0),
                1,
                0,
            ),
            (
                FightEvent::StaminaOut { fighter: 1 },
                GymEventKind::StaminaOut,
                None,
                1,
                0,
            ),
            (
                FightEvent::Knockdown {
                    attacker: 0,
                    victim: 1,
                },
                GymEventKind::Kill,
                Some(0),
                1,
                0,
            ),
        ];
        for (event, kind, attacker, victim, amount) in cases {
            assert_eq!(
                GymEvent::of(41, event),
                GymEvent {
                    tick: 41,
                    kind,
                    attacker,
                    victim,
                    amount
                }
            );
        }
        let kinds = [
            GymEventKind::Hit,
            GymEventKind::Block,
            GymEventKind::Parry,
            GymEventKind::Dodge,
            GymEventKind::StaminaOut,
            GymEventKind::Kill,
        ]
        .map(GymEventKind::name);
        assert_eq!(
            kinds,
            ["hit", "block", "parry", "dodge", "stamina_out", "kill"]
        );
    }

    #[test]
    fn gym_obs_carries_reward_legal_mask_and_velocity() {
        let spawned = initial_state(2040, 2, 20.0).expect("valid spawn");
        assert_eq!(spawned.len(), 2);
        for fighter in &spawned {
            assert_eq!(fighter.reward, 1.0, "spawn holds full health");
            assert_eq!(fighter.legal_mask, [true, true, true]);
            assert_eq!(fighter.velocity, [0.0, 0.0, 0.0]);
        }
        let config = ArenaConfig::try_new(2040, 8, 2, 20.0).expect("valid config");
        let snap = observe_state(&config, &FighterTape::default()).expect("observe serves");
        assert_eq!(snap.fighters.len(), 2);
        for fighter in &snap.fighters {
            assert!(
                (0.0..=1.0).contains(&fighter.reward),
                "reward stays a health fraction"
            );
            assert_eq!(
                fighter.reward,
                f32::from(fighter.hp) / 1000.0,
                "reward mirrors health"
            );
            assert_eq!(fighter.legal_mask.len(), GYM_NUM_ACTIONS);
            assert!(
                fighter.velocity.iter().all(|lane| lane.is_finite()),
                "velocity stays finite meters per second"
            );
            assert_eq!(fighter.velocity[1], 0.0, "flat floor keeps vy at zero");
        }
        // A drained fighter cannot afford attack or dodge, but block stays
        // legal while it can act; nothing is legal while staggered.
        let mut tired = CombatFighter {
            stamina: ATTACK_STAMINA_COST - 1,
            ..CombatFighter::default()
        };
        assert_eq!(
            legal_mask_of(&tired),
            [false, true, true],
            "attack cost gates attack below 25 stamina"
        );
        tired.stamina = DODGE_STAMINA_COST - 1;
        assert_eq!(
            legal_mask_of(&tired),
            [false, true, false],
            "low stamina gates attack and dodge only"
        );
        tired.react = 3;
        assert_eq!(legal_mask_of(&tired), [false, false, false]);
        assert_eq!(reward_of(&CombatFighter::default()), 1.0);
    }
}
