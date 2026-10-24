//! Incremental live match (EG1) and presentation (EG1b): one `Sim` step
//! per fixed tick.
//!
//! A fixed-step loop must cost the same every tick (Gregory, Game Engine
//! Architecture 7.2 and 7.5.2.2). The prefix runners ([`run_prefix`],
//! [`run_events`], [`run_dataset`]) rebuild the match from tick 0 on every
//! call, so a session that asks them once per tick pays O(n) per tick and
//! O(n^2) per match. [`LiveMatch`] owns one cold authority `Sim` and steps
//! it exactly once per call through the same recorder leg as
//! [`run_dataset`] (`DatasetLeg`), returning that tick's checkpoint, combat
//! event row and dataset rows: byte-identical to the prefix runners' last
//! tick, at constant cost. [`LivePresentation`] does the same for the
//! presentation legs ([`run_loco_poses`], [`run_presentation`]) over its own
//! recording sim.
//!
//! Consumer: the packaged game session (`engine-game` `App`), one call per
//! accepted tick. Data owner: the live match owns its sim; the caller owns
//! the tape and hands in one tick's inputs per call. Units: fixed ticks at
//! [`TICK_HZ`](crate::TICK_HZ). Determinism scope: same as [`run`]; the
//! checkpoint hash path is the shared `Sim::step`, untouched. Error policy:
//! a rejected input slice returns `None` and steps nothing; a step failure
//! (unreachable for validated inputs) poisons the match, and the caller
//! rebuilds from tick 0.
//!
//! [`run`]: crate::run
//! [`run_prefix`]: crate::run_prefix
//! [`run_events`]: crate::run_events
//! [`run_dataset`]: crate::run_dataset
//! [`run_loco_poses`]: crate::run_loco_poses
//! [`run_presentation`]: crate::run_presentation

use engine_ai::legion::Formation;
use engine_ai::orders::SquadOrder;
use engine_anim::contract::{LocalPose, RootDelta, RootTable};
use engine_anim::pose::extract_root;
use engine_input::{FighterInput, FighterTape};
use engine_nav::orca::{Agent, Orca, Params, Wall};

use crate::legion_bind::{Contubernium, Facing, Line, CONTUBERNIUM};
use crate::recorder::{on_fork_step, DamageLedger, Row, StepRecord};
use crate::world::MatchWorld;
#[cfg(not(feature = "sim"))]
use crate::MatchSnapshot;
use crate::{
    ArenaConfig, ArenaRules, BalanceState, Checkpoint, FighterState, MatchSpec, Sim, TickBreaks,
    TickCuts, TickEvents, MAX_SLOTS,
};
use engine_combat::depth::PoiseMeter;
use engine_combat::heft::Heft;

/// Dataset recorder leg: pre-step poses, fight state and damage counters
/// around one `Sim::step`, then one [`Row`] per fighter. Shared by
/// [`run_dataset`](crate::run_dataset) (whole match) and [`LiveMatch`] (one
/// tick per call), so both record through the same per-tick code. Scratch is
/// hoisted: the steady path allocates nothing beyond the sim's own tick log.
pub(crate) struct DatasetLeg {
    ledger: DamageLedger,
    before_states: Vec<FighterState>,
    before_damage: Vec<u16>,
}

impl DatasetLeg {
    /// Fresh leg for `fighters` slots (zero damage dealt).
    pub(crate) fn new(fighters: usize) -> Self {
        Self {
            ledger: DamageLedger::new(fighters),
            before_states: Vec::with_capacity(fighters),
            before_damage: Vec::with_capacity(fighters),
        }
    }

    /// Step `sim` one tick over `tape` and append that tick's rows (one per
    /// fighter, slot order) to `rows`. `None` when the step or a row fails.
    pub(crate) fn step(
        &mut self,
        sim: &mut Sim,
        tape: &FighterTape,
        rows: &mut Vec<Row>,
    ) -> Option<()> {
        let tick = sim.tick;
        self.before_states.clear();
        self.before_states.extend_from_slice(sim.states.as_slice());
        let before_fight = sim.fight;
        self.ledger.snapshot_into(&mut self.before_damage);
        sim.step(tape)?;
        let inputs = sim.last_inputs();
        let tick_row = sim.events.last()?;
        let killed = self.ledger.apply(&tick_row.events);
        for (slot, cell) in inputs.iter().enumerate().take(sim.fighters) {
            rows.push(on_fork_step(&StepRecord {
                tick,
                states: &self.before_states,
                fight: &before_fight,
                input: (*cell)?,
                damage_before: self.before_damage.get(slot).copied()?,
                damage_after: self.ledger.damage(slot),
                killed,
            })?);
        }
        Some(())
    }
}

/// One stepped tick, borrowed from the [`LiveMatch`] until the next step.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LiveTick<'a> {
    /// Post-step checkpoint: the last row of [`run_prefix`](crate::run_prefix)
    /// over `0..=tick`.
    pub checkpoint: Checkpoint,
    /// Combat events of the tick: the last row of
    /// [`run_events`](crate::run_events) over `0..=tick`.
    pub events: &'a TickEvents,
    /// Destruction breaks of the tick: the last row of
    /// [`run_breaks`](crate::run_breaks) over `0..=tick`.
    pub breaks: &'a TickBreaks,
    /// Severed-joint cuts of the tick: the last row of
    /// [`run_cuts`](crate::run_cuts) over `0..=tick`.
    pub cuts: &'a TickCuts,
    /// The tick's dataset rows, one per fighter in slot order: the last
    /// `fighters` rows of [`run_dataset`](crate::run_dataset) over `0..=tick`.
    pub rows: &'a [Row],
}

/// One fighter's combat-depth poise, read from the meter: the points left,
/// the meter's cap and whether it is holding a stagger trip.
///
/// Data owner: this crate's [`DepthSet`](engine_combat::depth::DepthSet). Units: poise points against the
/// meter's own cap (100 under the shipped v2 rows). Presentation only; the
/// digest path never reads this, so the hash cannot move. Determinism
/// scope: same-machine read of tick output, never hashed here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PoiseReading {
    /// Points left on the meter.
    pub current: u16,
    /// The meter's cap.
    pub max: u16,
    /// True while the meter holds a stagger trip (a poise break).
    pub staggered: bool,
}

impl PoiseReading {
    /// Read one meter into a plain value.
    fn of(meter: &PoiseMeter) -> Self {
        Self {
            current: meter.current,
            max: meter.max,
            staggered: meter.staggered,
        }
    }
}

/// Engine-observe reason name for a past-deadline provider reply. Names, not
/// raw codes: the provider loop resolves this through the real
/// `engine-observe` registry (`provider.timeout`), so this crate never
/// mirrors that registry (provider `fallback.rs` precedent).
const PROVIDER_TIMEOUT_REASON: &str = "provider.timeout";

/// One past-deadline provider reply refused without ever applying late. The
/// tick already ran the scripted brain; this row is diagnostics only and
/// never enters the tick or the hash.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct LateFallback {
    /// The refused reply's tick, already stepped when the reply arrived.
    tick: u32,
    /// Engine-observe reason name (`PROVIDER_TIMEOUT_REASON`).
    reason: &'static str,
}

/// ORCA look-around radius in mm: every ring neighbour inside 8 m constrains.
const STEER_NEIGHBOR_MM: i32 = 8_000;
/// Neighbours that may constrain one slot (the ORCA cap is 16).
const STEER_NEIGHBORS: usize = 8;
/// ORCA time horizon in ms: 1.5 s of closing speed is avoided.
const STEER_HORIZON_MS: i32 = 1_500;
/// ORCA step length in ms, the rate that resolves an overlapped pair.
const STEER_DT_MS: i32 = 17;
/// Planar radius of the near-field ring in mm: slots inside it steer with ORCA.
const STEER_RING_MM: i32 = 15_000;
/// Speed cap in mm/s: the walk top speed, with no headroom to waste.
const STEER_MAX_SPEED_MM_S: i32 = 5_000;

/// Armed ORCA crowd channel (row TC4): one planar avoidance velocity per
/// AI-driven slot inside the 15 m ring, applied on top of the walk before the
/// collision pass, so crossing bodies pass instead of shoving.
///
/// Data owner: this crate. Units: positions and velocities in whole
/// millimetres and millimetres per second, the applied correction in whole
/// millimetres per fixed tick, converted back to the arena's float metres.
/// Determinism: the batch is slot ordered and ORCA is integer math, so the
/// same state gives the same velocities; every lane is hoisted, so a warm
/// tick allocates nothing. Consumer: [`Sim::set_steering`](crate::Sim).
pub(crate) struct Steering {
    orca: Orca,
    params: Params,
    /// Four arena boundary walls with inward normals.
    walls: [Wall; 4],
    /// Submitted batch, in slot order; only `..count` lanes are live.
    agents: [Agent; MAX_SLOTS],
    /// New velocity per submitted lane.
    out: [[i32; 2]; MAX_SLOTS],
    /// Slot behind each submitted lane.
    lanes: [usize; MAX_SLOTS],
    /// Last velocity each slot was given, in mm/s: the next tick's current
    /// velocity, so ORCA sees the correction it handed out.
    vel: [[i32; 2]; MAX_SLOTS],
    /// Walk delta each slot asked for on the current tick, in metres.
    pub(crate) requests: [[f32; 2]; MAX_SLOTS],
    /// Test-only: correction applied to each slot on the last tick, in
    /// millimetres, so a test can separate it from the collision push.
    #[cfg(all(test, not(feature = "sim")))]
    applied: [[i32; 2]; MAX_SLOTS],
}

impl Steering {
    /// Channel for an arena of `half_extent_m` (the wall line ORCA keeps off).
    pub(crate) fn armed(half_extent_m: f32) -> Self {
        let edge = crate::match_wire::pos_mm([half_extent_m, 0.0, 0.0])[0];
        Self {
            orca: Orca::new(),
            params: Params {
                neighbor_mm: STEER_NEIGHBOR_MM,
                max_neighbors: STEER_NEIGHBORS,
                horizon_ms: STEER_HORIZON_MS,
                dt_ms: STEER_DT_MS,
                max_speed_mm_s: STEER_MAX_SPEED_MM_S,
            },
            walls: [
                Wall::new([edge, 0], [-1, 0]),
                Wall::new([-edge, 0], [1, 0]),
                Wall::new([0, edge], [0, -1]),
                Wall::new([0, -edge], [0, 1]),
            ],
            agents: [Agent {
                pos: [0, 0],
                vel: [0, 0],
                pref: [0, 0],
                radius_mm: crate::COLLIDE_RADIUS_MM,
            }; MAX_SLOTS],
            out: [[0; 2]; MAX_SLOTS],
            lanes: [0; MAX_SLOTS],
            vel: [[0; 2]; MAX_SLOTS],
            requests: [[0.0; 2]; MAX_SLOTS],
            #[cfg(all(test, not(feature = "sim")))]
            applied: [[0; 2]; MAX_SLOTS],
        }
    }

    /// One tick of the channel: submit every slot inside the ring, step ORCA,
    /// then add each driven, living slot's correction to its position.
    ///
    /// `requested` is the tick's walk delta in metres; `driven` marks the
    /// slots the arena's own AI (brain or controller) decided this tick, so a
    /// tape human is steered around but never steered. `limit_m` is the wall
    /// clamp the walk already honoured.
    pub(crate) fn steer(
        &mut self,
        states: &mut [FighterState],
        alive: &[bool; MAX_SLOTS],
        driven: &[bool; MAX_SLOTS],
        requested: &[[f32; 2]; MAX_SLOTS],
        limit_m: f32,
    ) -> Option<()> {
        #[cfg(all(test, not(feature = "sim")))]
        {
            self.applied = [[0; 2]; MAX_SLOTS];
        }
        let tick_hz = i32::try_from(crate::TICK_HZ).ok()?;
        let ring = i64::from(STEER_RING_MM);
        let radius = crate::COLLIDE_RADIUS_MM;
        let mut count = 0;
        for (slot, state) in states.iter().enumerate() {
            let pos = crate::match_wire::pos_mm(state.pos);
            let (x, z) = (i64::from(pos[0]), i64::from(pos[2]));
            if x * x + z * z > ring * ring {
                self.vel[slot] = [0; 2];
                continue;
            }
            let want = mm_pair(requested[slot]);
            let pref = [want[0] * tick_hz, want[1] * tick_hz];
            // A dead body stays put and is only an obstacle; a tape human
            // walks its own path and is an obstacle that never yields.
            let (vel, pref) = if !alive[slot] {
                ([0; 2], [0; 2])
            } else if driven[slot] {
                (self.vel[slot], pref)
            } else {
                (pref, [0; 2])
            };
            self.agents[count] = Agent {
                pos: [pos[0], pos[2]],
                vel,
                pref,
                radius_mm: radius,
            };
            self.lanes[count] = slot;
            count += 1;
        }
        self.orca
            .step(
                &self.agents[..count],
                &self.walls,
                &self.params,
                &mut self.out[..count],
            )
            .ok()?;
        for lane in 0..count {
            let slot = self.lanes[lane];
            let new = self.out[lane];
            self.vel[slot] = new;
            if !driven[slot] || !alive[slot] {
                continue;
            }
            let want = mm_pair(requested[slot]);
            let correction = [new[0] / tick_hz - want[0], new[1] / tick_hz - want[1]];
            if correction == [0; 2] {
                continue;
            }
            let state = &mut states[slot];
            state.pos[0] = crate::canon_zero(state.pos[0] + correction[0] as f32 / 1000.0)
                .clamp(-limit_m, limit_m);
            state.pos[2] = crate::canon_zero(state.pos[2] + correction[1] as f32 / 1000.0)
                .clamp(-limit_m, limit_m);
            #[cfg(all(test, not(feature = "sim")))]
            {
                self.applied[slot] = correction;
            }
        }
        Some(())
    }
}

impl Sim {
    /// Arm the ORCA crowd channel (row TC4) before the first tick: every
    /// AI-driven slot inside the 15 m ring then takes a planar avoidance
    /// velocity on top of its walk, before the collision pass. Off by
    /// default, so the golden and the arena digest never move.
    ///
    /// The channel is a rules v2 crowd aid, so it refuses (`false`) under the
    /// v1 rows. It also refuses past tick 0, on the Rapier `sim` leg (whose
    /// controllers own the poses) and when already armed.
    pub fn set_steering(&mut self) -> bool {
        if cfg!(feature = "sim")
            || self.tick != 0
            || self.rules.version != crate::FIGHT_RULES_V2
            || self.steer.is_some()
        {
            return false;
        }
        self.steer = Some(Steering::armed(self.config.half_extent_m));
        true
    }

    /// One armed tick: hand every slot inside the ring to ORCA, then add each
    /// driven, living slot's correction before the collision pass. A tape
    /// human and a dead body are obstacles that never yield.
    pub(crate) fn steer_crowd(&mut self, from_tape: &[bool; MAX_SLOTS]) -> Option<()> {
        let mut driven = [false; MAX_SLOTS];
        let mut alive = [false; MAX_SLOTS];
        for slot in 0..self.fighters {
            driven[slot] = !from_tape[slot];
            alive[slot] = self.fight.fighters[slot].hp_permille != 0;
        }
        let limit = crate::wall_limit(self.config.half_extent_m);
        let steering = self.steer.as_mut()?;
        let requests = steering.requests;
        steering.steer(&mut self.states, &alive, &driven, &requests, limit)
    }
}

/// Planar metres to whole millimetres (truncated toward zero, like the wire
/// poses, so a walk delta and its preferred speed agree exactly).
fn mm_pair(meters: [f32; 2]) -> [i32; 2] {
    let mm = crate::match_wire::pos_mm([meters[0], 0.0, meters[1]]);
    [mm[0], mm[2]]
}

/// Incremental live match: one cold authority `Sim` stepped exactly once per
/// [`LiveMatch::step`] call (see the module docs). The tick log stays one row
/// deep, so memory is flat over any match length.
pub struct LiveMatch {
    sim: Sim,
    leg: DatasetLeg,
    /// One-tick tape scratch: the caller's inputs for the stepped tick only.
    tape: FighterTape,
    /// Provider replies staged by [`LiveMatch::stamp`], drained at the tick
    /// boundary into the stepped tick; ticks already stepped never linger.
    staged: Vec<FighterInput>,
    /// Inputs that actually ran, in tick order: the replay tape read through
    /// [`LiveMatch::recorded_tape`].
    recorded: Vec<FighterInput>,
    /// Merge scratch for one tick's staged plus live inputs; cleared and
    /// refilled by [`LiveMatch::step`] so steady ticks allocate nothing new.
    merge: Vec<FighterInput>,
    rows: Vec<Row>,
    checkpoint: Checkpoint,
    events: TickEvents,
    breaks: TickBreaks,
    cuts: TickCuts,
    steps: u64,
    poisoned: bool,
    /// Past-deadline replies refused in arrival order, one row per late
    /// reply. Diagnostics only; never enters the tick or the hash.
    fallbacks: Vec<LateFallback>,
    /// The contubernium under rules v2: slot 0 the player and his seven
    /// comrades in the Roman line. `None` until
    /// [`LiveMatch::arm_contubernium`] arms it; the frozen v1 match never
    /// arms it, so its checkpoints stay byte for byte.
    contubernium: Option<Contubernium>,
}

impl LiveMatch {
    /// Fresh match at tick 0 over a validated `config` under the frozen v1
    /// rules; `None` when the config is invalid (same rule as
    /// [`run`](crate::run)).
    pub fn new(config: &ArenaConfig) -> Option<Self> {
        Self::with_rules(config, ArenaRules::V1)
    }

    /// Fresh match at tick 0 under explicit fight rules (EG3); `None` when
    /// the config is invalid or the rules version is unknown.
    pub fn with_rules(config: &ArenaConfig, rules: ArenaRules) -> Option<Self> {
        Self::with_rules_world(config, rules, MatchWorld::empty())
    }

    /// [`LiveMatch::with_rules`] over a static obstacle world: the kit walls
    /// the fight collides with. [`MatchWorld::empty`] is the legacy open
    /// sand byte for byte, so existing callers and fixtures stay frozen.
    ///
    /// Consumer: the game's live window session (`engine-game`), which loads
    /// the arena kit's physical placement into this world. Units: meters.
    /// Determinism: the same world gives the same checkpoints; the world
    /// digest rides every checkpoint. Errors: `None` on an invalid config,
    /// an unknown rules version or a world that blocks a spawn.
    pub fn with_rules_world(
        config: &ArenaConfig,
        rules: ArenaRules,
        world: MatchWorld,
    ) -> Option<Self> {
        let mut sim = Sim::build_world(config, &FighterTape::default(), false, false, world)?;
        if !sim.set_rules(rules) {
            return None;
        }
        // The live log keeps one row: drop the whole-match reservations.
        sim.checkpoints = Vec::with_capacity(1);
        sim.events = Vec::with_capacity(1);
        sim.breaks = Vec::with_capacity(1);
        let fighters = sim.fighters;
        Some(Self {
            sim,
            leg: DatasetLeg::new(fighters),
            tape: FighterTape(Vec::with_capacity(MAX_SLOTS)),
            staged: Vec::new(),
            recorded: Vec::new(),
            merge: Vec::new(),
            rows: Vec::with_capacity(fighters),
            checkpoint: Checkpoint { tick: 0, hash: 0 },
            events: TickEvents {
                tick: 0,
                events: Vec::new(),
            },
            breaks: TickBreaks {
                tick: 0,
                events: Vec::new(),
            },
            cuts: TickCuts {
                tick: 0,
                cuts: Vec::new(),
            },
            steps: 0,
            poisoned: false,
            fallbacks: Vec::new(),
            contubernium: None,
        })
    }

    /// Fresh match at tick 0 from one [`MatchSpec`]: its config, rules and
    /// world. The one constructor the window, the net server and the net
    /// client share; [`MatchSpec::v1`] gives [`LiveMatch::new`] byte for byte.
    #[must_use]
    pub fn from_spec(spec: &MatchSpec) -> Option<Self> {
        let mut live = Self::with_rules_world(&spec.config, spec.rules, spec.world.clone())?;
        if spec.spear && !live.sim.set_spear() {
            return None;
        }
        if spec.height && !live.sim.set_height() {
            return None;
        }
        // FEEL1b: the spec names one weapon row per slot; a refused list
        // builds no match, so the window, the net server and the net client
        // never play half armed.
        if !spec.weapons.is_empty() && live.arm_weapons(&spec.weapons).is_err() {
            return None;
        }
        Some(live)
    }

    /// Arm one weapon row per fighter, in slot order, before the first tick
    /// (FEEL1b): [`Sim::arm_weapons`] over the shipped rows. The rows ride
    /// [`MatchSpec`], so every peer of one spec arms the
    /// same list.
    ///
    /// # Errors
    ///
    /// As [`Sim::arm_weapons`]: rules v1, a started match, a count that is
    /// not the slot count or an unknown row. Nothing is armed then.
    pub fn arm_weapons(&mut self, ids: &[&str]) -> Result<(), crate::heft::ArmError> {
        self.sim.arm_weapons(ids)
    }

    /// The weapon slot `slot` swings (FEEL1b), or `None` when it is bare.
    /// Presentation reads the swing startup here; the look never hashes it.
    #[must_use]
    pub fn weapon(&self, slot: u8) -> Option<Heft> {
        self.sim.weapon(slot)
    }

    /// Match configuration.
    #[must_use]
    pub const fn config(&self) -> ArenaConfig {
        self.sim.config
    }

    /// Fight rules this match runs under.
    #[must_use]
    pub const fn rules(&self) -> ArenaRules {
        self.sim.rules
    }

    /// Arm the contubernium: eight men in the Roman line, slot 0 the
    /// player and seven comrades beside him, the frontage and the rank gap
    /// from the shipped Rome line table. Only rules v2 arms it, and only
    /// a match with a full contubernium of bodies; the frozen v1 match
    /// never arms it, so its checkpoints stay byte for byte.
    ///
    /// The line starts at `anchor_mm` with all eight men on that spot:
    /// the fall-in seats them by the slot nearest to where they stand.
    /// False when the rules or the body count refuse.
    pub fn arm_contubernium(&mut self, anchor_mm: (i64, i64), facing: Facing) -> bool {
        if self.sim.rules.version != crate::FIGHT_RULES_V2 || self.sim.fighters < CONTUBERNIUM {
            return false;
        }
        let Ok(line) = Line::shipped() else {
            return false;
        };
        let places = [(anchor_mm.0, anchor_mm.1); CONTUBERNIUM];
        let Ok(bind) = Contubernium::new(line, anchor_mm, facing, Formation::Line, places) else {
            return false;
        };
        self.contubernium = Some(bind);
        true
    }

    /// The armed contubernium, or `None` when the match never armed one.
    #[must_use]
    pub fn contubernium(&self) -> Option<&Contubernium> {
        self.contubernium.as_ref()
    }

    /// Arm the ORCA crowd channel (row TC4) before the first tick: every
    /// AI-driven slot inside the 15 m ring then takes a planar avoidance
    /// velocity before the collision pass. Rules v2 only; off by default,
    /// and off leaves every tick and checkpoint byte for byte.
    pub fn arm_steering(&mut self) -> bool {
        self.sim.set_steering()
    }

    /// Give the contubernium a squad order: `fall in` closes the line up,
    /// `hold` stops it, `move` marches it on the point the order names.
    /// False when no contubernium is armed or the order is outside the
    /// drill; a refused order changes nothing.
    pub fn set_squad_order(&mut self, order: SquadOrder) -> bool {
        let Some(bind) = self.contubernium.as_mut() else {
            return false;
        };
        bind.apply_order(order).is_ok()
    }

    /// Next tick [`LiveMatch::step`] simulates (ticks stepped so far).
    #[must_use]
    pub const fn tick(&self) -> u32 {
        self.sim.tick
    }

    /// `Sim` steps performed since construction: exactly one per
    /// [`LiveMatch::step`] call that passed input validation (EG1 cost proof).
    #[must_use]
    pub const fn steps(&self) -> u64 {
        self.steps
    }

    /// True after a step failed mid-tick; every later step returns `None`
    /// and the caller rebuilds from tick 0.
    #[must_use]
    pub const fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    /// Read-only integer balance verdict for `slot`: the slot entry or `None`
    /// for a bad slot. Forwards to [`Sim::balance`](crate::Sim::balance),
    /// which owns the per-slot states. Presentation only; the digest path
    /// never reads this, so the hash cannot move.
    ///
    /// Consumer: the window fighter look polls this each tick and feeds
    /// `fighter_look::observe_balance`. Data owner:
    /// [`Sim::balance`](crate::Sim::balance). Units: slot index.
    /// Determinism scope: same-machine read of hashed state, never hashed here.
    #[must_use]
    pub fn balance(&self, slot: u8) -> Option<&BalanceState> {
        self.sim.balance(slot)
    }

    /// Hit-stop ticks left in the live fight after the last stepped tick: the
    /// count the impact set for the strike that landed. Forwards to
    /// [`Sim::hitstop`](crate::Sim::hitstop), which owns the fight state.
    /// Presentation only; never hashed here.
    ///
    /// Caller: `crates/game/src/lib.rs` (`App::simulate_tick` and
    /// `App::rebuild_live`) mirror it after each step. Units: fixed ticks.
    #[must_use]
    pub fn hitstop(&self) -> u32 {
        self.sim.hitstop()
    }

    /// Read-only combat-depth poise for `slot`: the meter's points, its cap
    /// and whether it is holding a stagger trip. `None` for a bad slot.
    /// Forwards to [`Sim::poise`](crate::Sim::poise), which owns the
    /// per-slot [`DepthSet`](engine_combat::depth::DepthSet). No copy, no lock.
    /// Presentation only; the digest path never reads this, so the hash
    /// cannot move. V1 meters stay full (never stagger); V2 meters drain on
    /// hits and chips and trip a stagger at zero.
    ///
    /// Consumer: the browser page (`crates/web/src/fight.rs`) paints the
    /// poise pips and names a poise break. Data owner: this crate's
    /// [`DepthSet`](engine_combat::depth::DepthSet). Units: poise points against the meter's own cap.
    /// Determinism scope: same-machine read of tick output, never hashed here.
    #[must_use]
    pub fn poise(&self, slot: u8) -> Option<PoiseReading> {
        self.sim.poise(slot).map(PoiseReading::of)
    }

    /// Read-only stamina points for `slot` in `0..=MAX_STAMINA`, or `None`
    /// for a bad slot. Forwards to [`Sim::stamina`](crate::Sim::stamina),
    /// which owns the fight state. No copy, no lock.
    /// Presentation only; the digest path never reads this, so the hash
    /// cannot move.
    ///
    /// Consumer: the browser page paints the stamina bar. Data owner: this
    /// crate's fight state. Units: stamina points against a 100 cap.
    /// Determinism scope: same-machine read of hashed state, never hashed here.
    #[must_use]
    pub fn stamina(&self, slot: u8) -> Option<u16> {
        self.sim.stamina(slot)
    }

    /// Read-only fall landings from the last stepped tick (FEEL7), slot
    /// order: each landed slot's fall height and the health the fall table
    /// charged. Forwards to [`Sim::landings`](crate::Sim::landings); the
    /// array is refreshed every step, so an entry lives one tick.
    ///
    /// Consumer: `crates/game/src/lib.rs::App::simulate_tick` copies them
    /// into the tick outcome for the body squash, camera dip and land cue.
    /// Data owner: this crate's height channel. Units: slot,
    /// millimetres, permille. Determinism scope: read of tick output.
    #[must_use]
    pub fn landings(&self) -> &[Option<crate::FallLanding>; MAX_SLOTS] {
        self.sim.landings()
    }

    /// Read-only press-tick attack cues from the last stepped tick (EG19b):
    /// one `AttackStarted` per fresh attack start in fighter-ID order under
    /// tables v2; empty under v1 rows, on quiet ticks and past the press
    /// tick. Forwards to [`Sim::cues`](crate::Sim::cues), which owns the
    /// per-tick cue buffer. Presentation only; the digest path never reads
    /// this, so the hash cannot move.
    ///
    /// Caller: `crates/game/src/lib.rs::App::simulate_tick` reads these each
    /// accepted tick and maps each cue into an audio `FightCue` (EG19c starts
    /// the swing clip from them). Data owner:
    /// [`Sim::cues`](crate::Sim::cues). Units: fighter slots in press order.
    /// Determinism scope: same-machine read of tick output, never hashed here.
    #[must_use]
    pub fn cues(&self) -> &[engine_combat::fight::AttackStarted] {
        self.sim.cues()
    }

    /// What the spear grapple did on the last stepped tick, in emission order
    /// (throws, hits, misses, deflections, tether pulls and ends). Empty for a
    /// match without `MatchSpec::spear`. Presentation only: map these onto
    /// cues and effects; the digest path never reads them.
    #[must_use]
    pub fn spear_events(&self) -> &[crate::SpearEvent] {
        self.sim.spear.as_ref().map_or(&[], |spear| spear.events())
    }

    /// Per-slot spear state after the last stepped tick: the flying spear
    /// (position, velocity), the tether (victim, goal, age, speed), the
    /// cooldown and body momentum. Empty for a match without
    /// `MatchSpec::spear`. Presentation only; fighter poses come from the
    /// normal path because the sim moves the bodies.
    #[must_use]
    pub fn spear_view(&self) -> &[crate::SpearSlot] {
        self.sim.spear.as_ref().map_or(&[], |spear| spear.slots())
    }

    /// Step tick [`LiveMatch::tick`] with `inputs`: the human inputs for that
    /// tick, `(fighter)`-sorted with no duplicates, every `input.tick` equal
    /// to the stepped tick and every slot below `config.fighters` (brains
    /// fill the rest, exactly as in the prefix runners). Replies staged with
    /// [`LiveMatch::stamp`] for this tick merge in: staged slots the caller
    /// does not drive run as recorded, so stepping with an empty slice replays
    /// a fully stamped tick. A slice breaking those rules, a finished match
    /// or a poisoned one returns `None` and steps nothing.
    pub fn step(&mut self, inputs: &[FighterInput]) -> Option<LiveTick<'_>> {
        if self.poisoned || self.sim.tick >= self.sim.config.ticks {
            return None;
        }
        if !accepts_tick(&self.sim, inputs) {
            return None;
        }
        let tick = self.sim.tick;
        let Self {
            sim,
            tape,
            staged,
            recorded,
            merge,
            ..
        } = self;
        merge.clear();
        for staged_input in staged.iter() {
            if staged_input.tick == tick
                && !inputs
                    .iter()
                    .any(|live| live.fighter.0 == staged_input.fighter.0)
            {
                merge.push(*staged_input);
            }
        }
        merge.extend_from_slice(inputs);
        merge.sort_by_key(|input| input.fighter.0);
        if !accepts_tick(sim, merge) {
            return None;
        }
        // Ticks already stepped never linger: everything staged for this
        // tick either ran or was superseded by a live call driving its slot.
        staged.retain(|staged_input| staged_input.tick != tick);
        load_tick(sim, tape, merge);
        recorded.extend_from_slice(merge);
        self.rows.clear();
        self.steps += 1;
        if self.advance().is_none() {
            self.poisoned = true;
            return None;
        }
        self.step_contubernium();
        Some(LiveTick {
            checkpoint: self.checkpoint,
            events: &self.events,
            breaks: &self.breaks,
            cuts: &self.cuts,
            rows: &self.rows,
        })
    }

    /// Stage one provider reply for its tick. True when staged: the tick is
    /// the next one or a later one inside the match, the slot is live and
    /// the fields validate. Past-due ticks refuse with false and log the
    /// fallback with its engine-observe reason (`provider.timeout`); that
    /// tick already ran the scripted brain and never applies late.
    /// Out-of-range ticks, bad slots and invalid fields refuse with false
    /// and change nothing; refusals never poison the match. A second reply
    /// for the same tick and slot replaces the first, so the newest provider
    /// answer wins.
    /// Caller: the AL1 live-mind loop stages each validated provider reply
    /// here one or more ticks ahead; [`LiveMatch::step`] drains them at the
    /// tick boundary.
    pub fn stamp(&mut self, input: FighterInput) -> bool {
        if input.tick < self.sim.tick {
            if usize::from(input.fighter.0) < self.sim.fighters && input.validate().is_ok() {
                self.fallbacks.push(LateFallback {
                    tick: input.tick,
                    reason: PROVIDER_TIMEOUT_REASON,
                });
            }
            return false;
        }
        if input.tick >= self.sim.config.ticks
            || usize::from(input.fighter.0) >= self.sim.fighters
            || input.validate().is_err()
        {
            return false;
        }
        if let Some(slot) = self
            .staged
            .iter_mut()
            .find(|staged| staged.tick == input.tick && staged.fighter.0 == input.fighter.0)
        {
            *slot = input;
        } else {
            self.staged.push(input);
        }
        true
    }

    /// Inputs that ran so far, in tick order: staged replies plus live calls
    /// merged per tick by [`LiveMatch::step`]. Replaying them through
    /// [`run`](crate::run) reproduces the stepped checkpoints.
    /// Caller: the AL1 verifier replays with the provider off.
    #[must_use]
    pub fn recorded_tape(&self) -> FighterTape {
        FighterTape(self.recorded.clone())
    }

    /// Non-consuming snapshot of the running match at its current tick: the
    /// same bytes [`run_prefix`](crate::run_prefix) would capture there,
    /// taken from the held sim without disturbing it. `None` when the match
    /// runs a non-empty obstacle world or rules other than v1:
    /// [`MatchSnapshot`] carries neither the world nor the v2 balance,
    /// stagger and limb state, so a resume would silently drop them (both
    /// ride every checkpoint). Callers rebuild such a match from its
    /// [`MatchSpec`] and the tape.
    ///
    /// Pair with [`LiveMatch::from_snapshot`] to continue a second leg from
    /// the same tick while the original keeps stepping. Consumer: the net
    /// session's re-based prediction suffix. Units: fixed ticks.
    #[cfg(not(feature = "sim"))]
    #[must_use]
    pub fn snapshot(&self) -> Option<MatchSnapshot> {
        if !self.sim.world.is_empty()
            || self.sim.rules != ArenaRules::V1
            || self.sim.spear.is_some()
            || self.sim.height
        {
            return None;
        }
        Some(MatchSnapshot {
            config: self.sim.config,
            tick: self.sim.tick,
            states: self.sim.states.clone(),
            wishes: self.sim.wishes,
            buttons: self.sim.buttons,
            fight: self.sim.fight,
            match_state: *self.sim.match_track.state(),
            match_version: self.sim.match_track.version(),
            phased: *self.sim.match_track.phased(),
            brains: self.sim.brains.clone(),
            locos: self.sim.locos.clone(),
            destruction: self.sim.destruction.clone(),
            rise: Vec::new(),
            height: false,
        })
    }

    /// Fresh stepper resuming `snapshot` at its tick under the frozen v1
    /// rules, the same leg [`resume`](crate::resume) takes wrapped in the
    /// one-step-at-a-time stepper: `None` when the snapshot is misfiled or
    /// its config is invalid. The snapshot is consumed (single-use, like
    /// [`resume`](crate::resume)); the source match stays untouched and keeps
    /// stepping.
    ///
    /// Std-only track: the `sim` leg cannot copy its live controllers out of
    /// a running match (they are not `Clone`), so it keeps the single-use
    /// [`run_prefix`](crate::run_prefix)/[`resume`](crate::resume) path. Snapshots carry no obstacle world, so
    /// the resume runs the legacy open arena, exactly like
    /// [`resume`](crate::resume).
    #[cfg(not(feature = "sim"))]
    #[must_use]
    pub fn from_snapshot(snapshot: MatchSnapshot) -> Option<Self> {
        let config = snapshot.config();
        let mut sim = Sim::from_snapshot_parts(snapshot, &config, &FighterTape::default())?;
        // The live log keeps one row: drop the whole-match reservations.
        sim.checkpoints = Vec::with_capacity(1);
        sim.events = Vec::with_capacity(1);
        sim.breaks = Vec::with_capacity(1);
        let fighters = sim.fighters;
        Some(Self {
            sim,
            leg: DatasetLeg::new(fighters),
            tape: FighterTape(Vec::with_capacity(MAX_SLOTS)),
            staged: Vec::new(),
            recorded: Vec::new(),
            merge: Vec::new(),
            rows: Vec::with_capacity(fighters),
            checkpoint: Checkpoint { tick: 0, hash: 0 },
            events: TickEvents {
                tick: 0,
                events: Vec::new(),
            },
            breaks: TickBreaks {
                tick: 0,
                events: Vec::new(),
            },
            cuts: TickCuts {
                tick: 0,
                cuts: Vec::new(),
            },
            steps: 0,
            poisoned: false,
            fallbacks: Vec::new(),
            contubernium: None,
        })
    }

    /// One shared-leg step plus the one-row log rotation.
    fn advance(&mut self) -> Option<()> {
        self.leg.step(&mut self.sim, &self.tape, &mut self.rows)?;
        self.checkpoint = self.sim.checkpoints.pop()?;
        self.events = self.sim.events.pop()?;
        self.breaks = self.sim.breaks.pop()?;
        self.cuts = self.sim.cuts.pop()?;
        Some(())
    }

    /// Step the contubernium one tick when the match armed one. The line
    /// stands beside the sim and never touches it, so the checkpoint hash
    /// cannot move. A refused step disarms the line and leaves the match
    /// running.
    fn step_contubernium(&mut self) {
        let Some(bind) = self.contubernium.as_mut() else {
            return;
        };
        if bind.step(self.sim.tick).is_err() {
            self.contubernium = None;
        }
    }
}

/// Same per-input rules the prefix runners apply to the tape, scoped to one
/// tick: valid fields, strict fighter order, slot in range, tick equal to
/// the sim's next tick. Shared by both live steppers.
fn accepts_tick(sim: &Sim, inputs: &[FighterInput]) -> bool {
    let mut prev: Option<u8> = None;
    for input in inputs {
        if input.tick != sim.tick
            || usize::from(input.fighter.0) >= sim.fighters
            || input.validate().is_err()
            || prev.is_some_and(|last| input.fighter.0 <= last)
        {
            return false;
        }
        prev = Some(input.fighter.0);
    }
    true
}

/// Load one accepted tick's inputs into the one-tick tape scratch and rewind
/// the sim's tape cursor onto it (no allocation once the scratch is sized).
fn load_tick(sim: &mut Sim, tape: &mut FighterTape, inputs: &[FighterInput]) {
    tape.0.clear();
    tape.0.extend_from_slice(inputs);
    sim.cursor = 0;
}

/// One fighter's root sample after a tick: planar `[x, z]` plus yaw, the
/// same values `Sim::root_tables` reads from the recorded trajectory.
type RootSample = ([f32; 2], f32);

/// One fighter's pending root-table edit for a stepped tick: the corrected
/// previous delta (it used to wrap to tick 0) plus the new wrap delta.
type RootEdit = (Option<RootDelta>, RootDelta);

/// Incremental presentation (EG1b): one recording `Sim` stepped exactly once
/// per [`LivePresentation::step_into`] call, appending that tick to
/// caller-held pose tracks and root tables. Byte-equal to
/// [`run_loco_poses`](crate::run_loco_poses) plus
/// [`run_presentation`](crate::run_presentation) over `0..=tick`, which stay
/// the parity oracle, at constant cost per tick.
///
/// Pose tracks are prefix-stable, so each tick appends one pose per fighter.
/// Root tables are loops ([`extract_root`]: delta `k` spans sample `k` to
/// `(k + 1) % n`), so a new sample only rewrites the old wrap delta into a
/// plain step and appends the new wrap; both come from `extract_root` over a
/// two-sample window, so the bytes match the whole-table extraction.
///
/// Consumer: the packaged game's rolling presentation cursor. Data owner: the
/// caller owns the tracks and tables; this owns one recording sim whose own
/// history stays one tick deep. Presentation only: never hashed, and the
/// authority sim ([`LiveMatch`]) is a separate sim.
pub struct LivePresentation {
    sim: Sim,
    tape: FighterTape,
    first: Vec<RootSample>,
    last: Vec<RootSample>,
    edits: Vec<RootEdit>,
    steps: u64,
    poisoned: bool,
}

impl LivePresentation {
    /// Fresh recording sim at tick 0 over a validated `config` under the
    /// frozen v1 rules; `None` when the config is invalid (same rule as
    /// [`run`](crate::run)).
    pub fn new(config: &ArenaConfig) -> Option<Self> {
        Self::with_rules(config, ArenaRules::V1)
    }

    /// Fresh recording sim under explicit fight rules (EG3): presentation
    /// must run the authority's rules, since hits move fighters. `None` when
    /// the config is invalid or the rules version is unknown.
    pub fn with_rules(config: &ArenaConfig, rules: ArenaRules) -> Option<Self> {
        Self::with_rules_world(config, rules, MatchWorld::empty())
    }

    /// [`LivePresentation::with_rules`] over a static obstacle world: the
    /// pose cursor must resolve the same kit walls the authority does, or the
    /// drawn fighter walks through the wall the sim just blocked.
    ///
    /// Consumer: the game's live window session (`engine-game`).
    pub fn with_rules_world(
        config: &ArenaConfig,
        rules: ArenaRules,
        world: MatchWorld,
    ) -> Option<Self> {
        let mut sim = Sim::build_world(config, &FighterTape::default(), true, false, world)?;
        if !sim.set_rules(rules) {
            return None;
        }
        let fighters = sim.fighters;
        // Logs and histories stay one tick deep: drop the whole-match
        // reservations (the recorded pose tracks live with the caller).
        sim.checkpoints = Vec::with_capacity(1);
        sim.events = Vec::with_capacity(1);
        sim.breaks = Vec::with_capacity(1);
        sim.history = Some(Vec::with_capacity(1));
        sim.loco_history = Some((0..fighters).map(|_| Vec::with_capacity(2)).collect());
        // RC36: recording ticks carry pose work, so they fork the per-fighter
        // leg over the process pool; authority ticks stay inline and heap-free.
        sim.pool = Some(engine_ecs::pool::shared());
        Some(Self {
            sim,
            tape: FighterTape(Vec::with_capacity(MAX_SLOTS)),
            first: Vec::with_capacity(fighters),
            last: Vec::with_capacity(fighters),
            edits: Vec::with_capacity(fighters),
            steps: 0,
            poisoned: false,
        })
    }

    /// Fresh recording sim for one [`MatchSpec`]: its rules, world and spear
    /// flag, so the drawn poses follow the authority (a tether moves bodies).
    /// `None` when the spec does not build.
    #[must_use]
    pub fn from_spec(spec: &MatchSpec) -> Option<Self> {
        let mut live = Self::with_rules_world(&spec.config, spec.rules, spec.world.clone())?;
        let armed = !spec.spear || live.sim.set_spear();
        armed.then_some(live)
    }

    /// Next tick [`LivePresentation::step_into`] simulates.
    #[must_use]
    pub const fn tick(&self) -> u32 {
        self.sim.tick
    }

    /// `Sim` steps performed since construction: one per accepted
    /// [`LivePresentation::step_into`] call (EG1b cost proof).
    #[must_use]
    pub const fn steps(&self) -> u64 {
        self.steps
    }

    /// Step tick [`LivePresentation::tick`] with `inputs` (same rules as
    /// [`LiveMatch::step`]) and append it: one pose onto each
    /// `pose_tracks[slot]`, one root edit onto each `root_tables[slot]`.
    /// Both slices hold one entry per fighter, each covering exactly the
    /// ticks stepped so far; otherwise (or on rejected inputs, a finished or
    /// poisoned sim) nothing steps and nothing is touched. A failed step
    /// poisons the sim and leaves the caller's data untouched.
    pub fn step_into(
        &mut self,
        inputs: &[FighterInput],
        pose_tracks: &mut [Vec<LocalPose>],
        root_tables: &mut [RootTable],
    ) -> Option<()> {
        let fighters = self.sim.fighters;
        let covered = usize::try_from(self.sim.tick).ok()?;
        if self.poisoned
            || self.sim.tick >= self.sim.config.ticks
            || !accepts_tick(&self.sim, inputs)
            || pose_tracks.len() != fighters
            || root_tables.len() != fighters
            || pose_tracks.iter().any(|track| track.len() != covered)
            || root_tables
                .iter()
                .any(|table| table.deltas.len() != covered)
        {
            return None;
        }
        load_tick(&mut self.sim, &mut self.tape, inputs);
        self.steps += 1;
        if self.advance().is_none() {
            self.poisoned = true;
            return None;
        }
        // Every fallible read happened in `advance`: apply all or nothing.
        let poses = self.sim.loco_history.as_ref()?;
        for ((track, table), ((fix, wrap), history)) in pose_tracks
            .iter_mut()
            .zip(root_tables.iter_mut())
            .zip(self.edits.iter().zip(poses))
        {
            track.push(history.last()?.clone());
            if let (Some(fix), Some(old)) = (fix, table.deltas.last_mut()) {
                *old = *fix;
            }
            table.deltas.push(*wrap);
        }
        Some(())
    }

    /// One recording step, then the per-fighter root edits and the one-tick
    /// trims. `None` on any failure (the caller poisons).
    fn advance(&mut self) -> Option<()> {
        let tick = self.sim.tick;
        self.sim.step(&self.tape)?;
        self.sim.checkpoints.clear();
        self.sim.events.clear();
        self.sim.breaks.clear();
        if let Some(history) = self.sim.history.as_mut() {
            history.clear();
        }
        let poses = self.sim.loco_history.as_mut()?;
        for track in poses.iter_mut() {
            let keep_from = track.len().checked_sub(1)?;
            track.drain(..keep_from);
        }
        self.edits.clear();
        for (slot, state) in self.sim.states.iter().enumerate().take(self.sim.fighters) {
            let sample = ([state.pos[0], state.pos[2]], state.yaw);
            let edit = if tick == 0 {
                self.first.push(sample);
                self.last.push(sample);
                (None, window_delta(sample, sample, 0, true)?)
            } else {
                let prev = *self.last.get(slot)?;
                let first = *self.first.get(slot)?;
                let fix = window_delta(prev, sample, tick - 1, false)?;
                let wrap = window_delta(sample, first, tick, false)?;
                *self.last.get_mut(slot)? = sample;
                (Some(fix), wrap)
            };
            self.edits.push(edit);
        }
        (self.edits.len() == self.sim.fighters).then_some(())
    }
}

/// The delta from sample `a` to sample `b` stamped `tick`, through
/// [`extract_root`] itself so the float bytes match the whole-table run.
/// `single` extracts the one-sample loop (tick 0: zero motion).
fn window_delta(a: RootSample, b: RootSample, tick: u32, single: bool) -> Option<RootDelta> {
    let table = if single {
        extract_root(&[a.0], &[a.1], tick).ok()?
    } else {
        extract_root(&[a.0, b.0], &[a.1, b.1], tick).ok()?
    };
    table.deltas.first().copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(feature = "sim"))]
    use crate::world::Obstacle;
    use crate::{run_breaks, run_dataset, run_events, run_prefix};
    use engine_input::{fighter::ATTACK, Buttons, FighterId};

    /// Human slot 0 circle-strafes and swings every 30 ticks; brains fill
    /// the rest. Inputs grouped per tick for the live stepper.
    fn script(config: &ArenaConfig) -> FighterTape {
        let mut inputs = Vec::new();
        for tick in 0..config.ticks {
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

    /// FEEL1b: the live match arms one row per slot before tick 0, and the
    /// law of the sim holds through the wrapper: rules v2 only, a full list,
    /// known rows, before the first step.
    #[test]
    fn live_match_arms_one_weapon_row_per_slot_before_tick_zero() {
        let config = ArenaConfig::try_new(7, 60, 2, 20.0).expect("valid config");
        let mut duel = MatchSpec::duel(config, MatchWorld::empty())
            .live()
            .expect("duel");
        assert!(duel.weapon(0).is_none(), "bare until armed");
        duel.arm_weapons(&["pugio", "clava"]).expect("the rows arm");
        assert_eq!(duel.weapon(0).map(|w| w.startup_ticks), Some(12));
        assert_eq!(duel.weapon(1).map(|w| w.startup_ticks), Some(21));
        assert_eq!(duel.weapon(2), None, "off the match is bare");
        duel.step(&[]).expect("the duel steps");
        assert!(
            duel.arm_weapons(&["pugio", "clava"]).is_err(),
            "arming is before the first tick"
        );
        let mut v1 = MatchSpec::v1(config).live().expect("v1");
        assert!(
            v1.arm_weapons(&["pugio", "clava"]).is_err(),
            "v1 cannot arm"
        );
        let mut short = MatchSpec::duel(config, MatchWorld::empty())
            .live()
            .expect("duel");
        assert!(short.arm_weapons(&["pugio"]).is_err(), "one row per slot");
        assert!(
            short.arm_weapons(&["no-such-row", "clava"]).is_err(),
            "unknown rows are refused"
        );
    }

    /// FEEL1b: the same two armed slots swing their own startup. The pugio
    /// hits on tick 12, the clava on tick 21; each bout presses once so the
    /// first blow does not interrupt the other's swing.
    #[test]
    fn an_armed_duel_lands_the_clava_nine_ticks_after_the_pugio() {
        let config = ArenaConfig::try_new(7, 60, 2, 20.0).expect("valid config");
        let mut first = [0u32; 2];
        for (attacker, slot) in [(0u8, 0usize), (1, 1)] {
            let mut live = MatchSpec::duel(config, MatchWorld::empty())
                .with_weapons(&["pugio", "clava"])
                .live()
                .expect("armed duel");
            // Arm's length, the shipped duel's reach: the attacker stands at
            // the origin facing +z, the victim 1 m ahead, so either slot can
            // be the swinger without turning.
            let (swinger, victim) = (usize::from(attacker), usize::from(attacker ^ 1));
            live.sim.states[swinger].pos = [0.0, 0.0, 0.0];
            live.sim.states[victim].pos = [0.0, 0.0, 1.0];
            let mut hit = None;
            for tick in 0..40 {
                let inputs: Vec<FighterInput> = (0..2u8)
                    .map(|fighter| FighterInput {
                        tick,
                        fighter: FighterId(fighter),
                        mx: 0,
                        mz: 0,
                        facing: 0,
                        buttons: if fighter == attacker && tick == 0 {
                            Buttons(ATTACK)
                        } else {
                            Buttons::NONE
                        },
                    })
                    .collect();
                let stepped = live.step(&inputs).expect("the duel steps");
                if stepped.events.events.iter().any(|event| {
                    matches!(
                        event,
                        crate::FightEvent::Hit { attacker: who, .. } if *who == attacker
                    )
                }) {
                    hit = Some(stepped.events.tick);
                    break;
                }
            }
            first[slot] = hit.expect("the armed blow lands");
        }
        assert_eq!(first[0], 12, "the pugio lands on its startup");
        assert_eq!(first[1], 21, "the clava lands on its startup");
        assert_eq!(first[1] - first[0], 9, "the club lands nine ticks later");
    }

    #[test]
    fn live_match_equals_whole_match_runners_tick_for_tick() {
        for fighters in [2u8, 8] {
            let config = ArenaConfig::try_new(2040, 600, fighters, 20.0).expect("valid config");
            let tape = script(&config);
            let (_, checkpoints) = run_prefix(&config, &tape, config.ticks).expect("prefix");
            let events = run_events(&config, &tape);
            let breaks = run_breaks(&config, &tape);
            let dataset = run_dataset(&config, &tape).expect("dataset");
            let stride = usize::from(fighters);
            let mut live = LiveMatch::new(&config).expect("live match");
            for tick in 0..config.ticks {
                let index = tick as usize;
                let stepped = live.step(tick_inputs(&tape, tick)).expect("live step");
                assert_eq!(stepped.checkpoint, checkpoints[index], "checkpoint {tick}");
                assert_eq!(stepped.events, &events[index], "events {tick}");
                assert_eq!(stepped.breaks, &breaks[index], "breaks {tick}");
                assert_eq!(
                    stepped.rows,
                    &dataset.rows[index * stride..(index + 1) * stride],
                    "rows {tick}"
                );
            }
            assert!(
                events.iter().any(|row| !row.events.is_empty()),
                "match fights"
            );
            assert_eq!(
                live.steps(),
                u64::from(config.ticks),
                "one sim step per tick"
            );
            assert_eq!(live.tick(), config.ticks);
            assert!(live.step(&[]).is_none(), "finished match steps nothing");
            assert_eq!(live.steps(), u64::from(config.ticks));
        }
    }

    #[cfg(not(feature = "sim"))]
    #[test]
    fn live_snapshot_resumes_bit_identical_without_consuming_the_match() {
        // The non-consuming snapshot continues a second leg from the same
        // tick byte-identically while the source match keeps stepping.
        let config = ArenaConfig::try_new(2040, 600, 2, 20.0).expect("valid config");
        let tape = script(&config);
        let (_, checkpoints) = run_prefix(&config, &tape, config.ticks).expect("prefix");
        let mut live = LiveMatch::new(&config).expect("live match");
        for tick in 0..300 {
            live.step(tick_inputs(&tape, tick)).expect("live step");
        }
        let mut first = LiveMatch::from_snapshot(live.snapshot().expect("open arena snapshots"))
            .expect("resume at 300");
        let mut second =
            LiveMatch::from_snapshot(live.snapshot().expect("snapshot twice")).expect("resume 2");
        assert_eq!((first.tick(), second.tick()), (300, 300));
        // The resumed leg replays the suffix byte-identically to the oracle.
        for tick in 300..config.ticks {
            let stepped = first.step(tick_inputs(&tape, tick)).expect("suffix step");
            assert_eq!(
                stepped.checkpoint, checkpoints[tick as usize],
                "suffix {tick}"
            );
        }
        // The second resume and the untouched original hold the same bytes:
        // taking the snapshot consumed nothing.
        let mut again = Vec::new();
        for tick in 300..320 {
            let stepped = second
                .step(tick_inputs(&tape, tick))
                .expect("second suffix");
            again.push(stepped.checkpoint);
        }
        assert_eq!(again, checkpoints[300..320], "second resume agrees");
        for tick in 300..config.ticks {
            let stepped = live.step(tick_inputs(&tape, tick)).expect("live continues");
            assert_eq!(
                stepped.checkpoint, checkpoints[tick as usize],
                "live {tick}"
            );
        }
        assert_eq!(live.steps(), u64::from(config.ticks), "one step per tick");
        // A world-armed match refuses to snapshot: MatchSnapshot has no world.
        let world =
            MatchWorld::try_new(vec![Obstacle::aligned([19.0, 19.0], [0.5, 0.5])]).expect("world");
        let armed =
            LiveMatch::with_rules_world(&config, ArenaRules::V1, world).expect("world match");
        assert!(armed.snapshot().is_none(), "the world would be dropped");
        // A rules-v2 match refuses too: the snapshot has no balance state.
        let duel = MatchSpec::duel(config, MatchWorld::empty())
            .live()
            .expect("duel match");
        assert!(duel.snapshot().is_none(), "v2 state would be dropped");
    }

    #[test]
    fn live_presentation_equals_prefix_presentation_runners_tick_for_tick() {
        // EG1b parity: after every tick the appended tracks and tables equal
        // run_loco_poses plus run_presentation over 0..=tick (root tables
        // are not prefix-stable, so each tick checks against a fresh replay).
        for fighters in [2u8, 8] {
            let config = ArenaConfig::try_new(2040, 600, fighters, 20.0).expect("valid config");
            let tape = script(&config);
            let stride = usize::from(fighters);
            let mut live = LivePresentation::new(&config).expect("live presentation");
            let mut tracks: Vec<Vec<LocalPose>> = vec![Vec::new(); stride];
            let mut tables: Vec<RootTable> = vec![RootTable::default(); stride];
            for tick in 0..config.ticks {
                live.step_into(tick_inputs(&tape, tick), &mut tracks, &mut tables)
                    .expect("presentation step");
                let short = ArenaConfig {
                    ticks: tick + 1,
                    ..config
                };
                // The runners reject tape rows past `short.ticks`: replay
                // the tape prefix the session held at this tick.
                let held = tape.0.partition_point(|input| input.tick <= tick);
                let prefix = FighterTape(tape.0[..held].to_vec());
                let poses = crate::run_loco_poses(&short, &prefix);
                assert_eq!(poses.len(), stride, "oracle replays at {tick}");
                assert_eq!(tracks, poses, "poses {tick}");
                assert_eq!(
                    tables,
                    crate::run_presentation(&short, &prefix),
                    "roots {tick}"
                );
            }
            assert_eq!(
                live.steps(),
                u64::from(config.ticks),
                "one sim step per tick"
            );
            assert!(live.step_into(&[], &mut tracks, &mut tables).is_none());
        }
    }

    #[test]
    fn live_presentation_rejects_mismatched_buffers_untouched() {
        let config = ArenaConfig::try_new(7, 8, 2, 12.0).expect("valid config");
        let mut live = LivePresentation::new(&config).expect("live presentation");
        let mut tracks: Vec<Vec<LocalPose>> = vec![Vec::new(); 2];
        let mut short_tables: Vec<RootTable> = vec![RootTable::default(); 1];
        assert!(live
            .step_into(&[], &mut tracks, &mut short_tables)
            .is_none());
        assert_eq!((live.tick(), live.steps()), (0, 0));
        let mut tables: Vec<RootTable> = vec![RootTable::default(); 2];
        live.step_into(&[], &mut tracks, &mut tables)
            .expect("tick 0");
        // A caller cache that fell behind is refused, never half-appended.
        let mut stale_tracks: Vec<Vec<LocalPose>> = vec![Vec::new(); 2];
        assert!(live
            .step_into(&[], &mut stale_tracks, &mut tables)
            .is_none());
        assert_eq!((live.tick(), live.steps()), (1, 1));
        assert!(stale_tracks.iter().all(Vec::is_empty));
        assert_eq!(tables[0].deltas.len(), 1);
    }
    /// Whole-match v2 run through one sim (the oracle for the live stepper).
    fn whole_v2(config: &ArenaConfig, tape: &FighterTape, rules: ArenaRules) -> Sim {
        let mut sim = Sim::build(config, tape, false, false).expect("valid build");
        assert!(sim.set_rules(rules));
        while sim.tick < config.ticks {
            sim.step(tape).expect("valid step");
        }
        sim
    }

    fn hits(events: &[TickEvents]) -> usize {
        events
            .iter()
            .flat_map(|row| row.events.iter())
            .filter(|event| matches!(event, crate::FightEvent::Hit { .. }))
            .count()
    }

    #[test]
    fn v2_live_match_digest_twice_equal_and_apart_from_v1() {
        // EG3: v2 digest x2 equal, the live stepper equals the whole-match
        // v2 sim tick for tick, and v1 checkpoints never collide with v2.
        for fighters in [2u8, 8] {
            let config = ArenaConfig::try_new(2040, 600, fighters, 20.0).expect("valid config");
            let tape = script(&config);
            let rules = ArenaRules::v2_free_for_all();
            let oracle = whole_v2(&config, &tape, rules);
            let run_live = || {
                let mut live = LiveMatch::with_rules(&config, rules).expect("v2 match");
                let mut checkpoints = Vec::new();
                for tick in 0..config.ticks {
                    let stepped = live.step(tick_inputs(&tape, tick)).expect("v2 step");
                    assert_eq!(stepped.events, &oracle.events[tick as usize]);
                    checkpoints.push(stepped.checkpoint);
                }
                checkpoints
            };
            let (first, second) = (run_live(), run_live());
            assert_eq!(first, second, "v2 digest x2");
            assert_eq!(first, oracle.checkpoints);
            let v1 = crate::run(&config, &tape);
            assert!(first.iter().zip(&v1).all(|(a, b)| a.hash != b.hash));
        }
    }

    #[test]
    fn v2_arena_far_foe_untouched_while_v1_hits_across_the_arena() {
        // Two idle humans at their spawns; slot 0 swings on tick 0.
        let config = ArenaConfig::try_new(2040, 12, 2, 20.0).expect("valid config");
        let mut inputs = Vec::new();
        for tick in 0..config.ticks {
            for slot in 0..2u8 {
                let buttons = if tick == 0 && slot == 0 {
                    Buttons(ATTACK)
                } else {
                    Buttons::NONE
                };
                inputs.push(FighterInput {
                    tick,
                    fighter: FighterId(slot),
                    mx: 0,
                    mz: 0,
                    facing: 0,
                    buttons,
                });
            }
        }
        let tape = FighterTape(inputs);
        let spawn = crate::spawn_states(config.seed, config.fighters, config.half_extent_m);
        let (dx, dz) = (
            spawn[1].pos[0] - spawn[0].pos[0],
            spawn[1].pos[2] - spawn[0].pos[2],
        );
        assert!(dx * dx + dz * dz > 4.0, "spawns stand over 2 m apart");
        assert_eq!(
            hits(&crate::run_events(&config, &tape)),
            1,
            "v1 hits anyway"
        );
        let v2 = whole_v2(&config, &tape, ArenaRules::v2_free_for_all());
        assert_eq!(hits(&v2.events), 0, "v2 needs reach");
    }

    #[test]
    fn v2_teammates_never_strike_each_other_in_a_brain_match() {
        let config = ArenaConfig::try_new(2040, 600, 4, 20.0).expect("valid config");
        let tape = FighterTape::default();
        let ffa = whole_v2(&config, &tape, ArenaRules::v2_free_for_all());
        assert!(hits(&ffa.events) > 0, "brains still land v2 hits");
        let one_team = whole_v2(&config, &tape, ArenaRules::v2_teams([0; MAX_SLOTS]));
        assert_eq!(hits(&one_team.events), 0, "one team, no hits");
        let mut bad = ArenaRules::v2_free_for_all();
        bad.version = 3;
        assert!(LiveMatch::with_rules(&config, bad).is_none());
    }
    /// Winner of a v2 brain duel from the sim's spawns: the slot still up on
    /// the first knockdown tick, `None` for no knockdown or a double
    /// knockout on one tick; plus the per-tick hp pairs.
    fn v2_duel(seed: u64, ticks: u32) -> (Option<u8>, Vec<(u16, u16)>) {
        let config = ArenaConfig::try_new(seed, ticks, 2, 20.0).expect("valid config");
        let tape = FighterTape::default();
        let mut sim = Sim::build(&config, &tape, false, false).expect("build");
        assert!(sim.set_rules(ArenaRules::v2_free_for_all()));

        let mut hp = Vec::with_capacity(ticks as usize);
        let mut winner = None;
        let mut decided = false;
        while sim.tick < config.ticks {
            sim.step(&tape).expect("step");
            let mut down = [false; 2];
            for event in &sim.events.last().expect("row").events {
                if let crate::FightEvent::Knockdown { victim, .. } = *event {
                    down[usize::from(victim)] = true;
                }
            }
            if !decided && (down[0] || down[1]) {
                decided = true;
                winner = match down {
                    [true, false] => Some(1),
                    [false, true] => Some(0),
                    _ => None,
                };
            }
            hp.push((
                sim.fight.fighters[0].hp_permille,
                sim.fight.fighters[1].hp_permille,
            ));
        }
        (winner, hp)
    }

    #[test]
    fn v2_brains_close_distance_and_land_hits() {
        // EG3 brain axes: the depth brain's wish is facing-relative; read as
        // world axes both fighters walked east into the wall. V2 actuates
        // it in the foe frame, so two brains meet and trade within 20 s.
        let config = ArenaConfig::try_new(2040, 1200, 2, 20.0).expect("valid config");
        let duel = whole_v2(
            &config,
            &FighterTape::default(),
            ArenaRules::v2_free_for_all(),
        );
        assert!(hits(&duel.events) > 0, "v2 brains land hits");
        let landed = duel
            .events
            .iter()
            .position(|row| {
                row.events
                    .iter()
                    .any(|event| matches!(event, crate::FightEvent::Hit { .. }))
            })
            .expect("a hit lands");
        assert!(landed < 1200, "first hit at tick {landed}");
    }

    #[test]
    fn v2_mirrored_duel_is_symmetric_tick_for_tick() {
        // EG3 symmetric resolution: two scripted fighters, point-mirrored
        // and pressing mirrored inputs (close in, then swing every 20 ticks
        // with a guard between swings), keep identical hp and stun on both
        // slots every tick. v1 resolves the trades in slot order.
        let config = ArenaConfig::try_new(2040, 240, 2, 20.0).expect("valid config");
        let mut inputs = Vec::new();
        for tick in 0..config.ticks {
            let (wish, buttons) = if tick < 40 {
                (50, Buttons::NONE)
            } else if tick % 20 == 0 {
                (0, Buttons(ATTACK))
            } else if tick % 20 >= 10 {
                (0, Buttons(engine_input::fighter::BLOCK))
            } else {
                (0, Buttons::NONE)
            };
            for (slot, sign, facing) in [(0u8, 1i8, 9_000i16), (1, -1, -9_000)] {
                inputs.push(FighterInput {
                    tick,
                    fighter: FighterId(slot),
                    mx: wish * sign,
                    mz: 0,
                    facing,
                    buttons,
                });
            }
        }
        let tape = FighterTape(inputs);
        let place = |slot: usize| {
            if slot == 0 {
                [-2.0, 0.0, 0.0]
            } else {
                [2.0, 0.0, 0.0]
            }
        };
        let mut hits = 0;
        let mirrored = |sim: &Sim| {
            let (a, b) = (sim.fight.fighters[0], sim.fight.fighters[1]);
            (a.hp_permille, a.react, a.knockdown) == (b.hp_permille, b.react, b.knockdown)
        };
        placed_run(
            &config,
            &tape,
            ArenaRules::v2_free_for_all(),
            place,
            |sim| {
                assert!(mirrored(sim), "v2 diverged at tick {}", sim.tick);
                hits += sim.events.last().map_or(0, |row| row.events.len());
            },
        );
        assert!(hits > 0, "the duel trades blows");
        let mut v1_split = false;
        placed_run(&config, &tape, ArenaRules::V1, place, |sim| {
            v1_split |= !mirrored(sim);
        });
        assert!(v1_split, "v1 favours a slot");
    }
    #[test]
    #[ignore = "slow: 400 brain duels; run with --ignored for the win-split bar"]
    fn v2_win_split_over_seeds_has_no_slot_advantage() {
        // Binomial bar over decisive duels: |slot-0 wins - n/2| stays inside
        // the two-sided p > 0.01 band (2.576 * sqrt(n) / 2). Measured
        // 2026-10-10: v1 slot 0 took about 99% of decisive bouts purely
        // from slot-order resolution; under v2 the identical brains close
        // in lockstep and every duel ends in a same-tick double knockout
        // (no slot ever wins alone). A real win split needs brain diversity.
        let (mut slot0, mut decisive, mut draws) = (0u32, 0u32, 0u32);
        for seed in 0..400u64 {
            match v2_duel(seed, 3600) {
                (Some(won), _) => {
                    decisive += 1;
                    slot0 += u32::from(won == 0);
                }
                (None, hp) => {
                    draws += u32::from(hp.last().is_some_and(|(a, b)| *a == 0 && *b == 0));
                }
            }
        }
        println!("v2 duels: slot 0 won {slot0} of {decisive} decisive, {draws} double knockouts");
        let n = f64::from(decisive);
        let off = (f64::from(slot0) - n / 2.0).abs();
        assert!(
            off <= 2.576 * n.sqrt() / 2.0,
            "slot 0 won {slot0} of {decisive}"
        );
        assert!(decisive + draws > 0, "duels must finish");
    }
    /// Every slot human and idle (or walking `+x` at full wish for slot 0
    /// when `walk`) for `config.ticks` ticks: no brains move anyone.
    #[cfg(not(feature = "sim"))]
    fn still_tape(config: &ArenaConfig, walk: bool) -> FighterTape {
        let mut inputs = Vec::new();
        for tick in 0..config.ticks {
            for slot in 0..config.fighters {
                inputs.push(FighterInput {
                    tick,
                    fighter: FighterId(slot),
                    mx: if walk && slot == 0 { 100 } else { 0 },
                    mz: 0,
                    facing: 0,
                    buttons: Buttons::NONE,
                });
            }
        }
        FighterTape(inputs)
    }

    /// Run `rules` over `tape` from spawns overridden by `place`, checking
    /// `each` after every tick; returns the sim.
    fn placed_run(
        config: &ArenaConfig,
        tape: &FighterTape,
        rules: ArenaRules,
        place: impl Fn(usize) -> [f32; 3],
        mut each: impl FnMut(&Sim),
    ) -> Sim {
        let mut sim = Sim::build(config, tape, false, false).expect("valid build");
        assert!(sim.set_rules(rules));
        for (slot, state) in sim.states.iter_mut().enumerate() {
            state.pos = place(slot);
        }
        while sim.tick < config.ticks {
            sim.step(tape).expect("valid step");
            each(&sim);
        }
        sim
    }

    // The default-leg collision pass never runs under `sim` (`ArenaRules::collides`),
    // so the push-apart checks below only hold without it.
    #[cfg(not(feature = "sim"))]
    fn min_gap_m(states: &[FighterState]) -> f32 {
        let mut best = f32::INFINITY;
        for (i, a) in states.iter().enumerate() {
            for b in &states[i + 1..] {
                let (dx, dz) = (b.pos[0] - a.pos[0], b.pos[2] - a.pos[2]);
                best = best.min((dx * dx + dz * dz).sqrt());
            }
        }
        best
    }

    #[cfg(not(feature = "sim"))]
    #[test]
    fn push_apart_spreads_eight_fighters_from_one_point_in_30_ticks() {
        let config = ArenaConfig::try_new(2040, 30, 8, 20.0).expect("valid config");
        let tape = still_tape(&config, false);
        let two_radii = 2.0 * crate::COLLIDE_RADIUS_MM as f32 / 1000.0;
        let v2 = placed_run(
            &config,
            &tape,
            ArenaRules::v2_free_for_all(),
            |_| [1.0, 0.0, 2.0],
            |_| {},
        );
        let gap = min_gap_m(&v2.states);
        assert!(gap >= two_radii - 0.002, "closest pair {gap} m apart");
        // F2P: without collision (v1) the eight stay on one point.
        let v1 = placed_run(&config, &tape, ArenaRules::V1, |_| [1.0, 0.0, 2.0], |_| {});
        assert_eq!(min_gap_m(&v1.states), 0.0);
    }

    #[cfg(not(feature = "sim"))]
    #[test]
    fn push_apart_keeps_every_centre_out_of_the_pillars() {
        let pillar = crate::Pillar::new([-1_000, -1_000], [1_000, 1_000]);
        let side = crate::Pillar::new([-6_000, 3_000], [-5_000, 4_000]);
        let rules = ArenaRules::v2_free_for_all()
            .with_pillar(pillar)
            .and_then(|rules| rules.with_pillar(side))
            .expect("room for two pillars");
        let inside = |state: &FighterState| {
            let mm = crate::match_wire::pos_mm(state.pos);
            [pillar, side].iter().any(|p| {
                mm[0] > p.min_mm[0]
                    && mm[0] < p.max_mm[0]
                    && mm[2] > p.min_mm[1]
                    && mm[2] < p.max_mm[1]
            })
        };
        // Slot 0 walks +x straight at the centre pillar for 2 s.
        let config = ArenaConfig::try_new(7, 120, 2, 20.0).expect("valid config");
        let walk = still_tape(&config, true);
        let place = |slot: usize| {
            if slot == 0 {
                [-4.0, 0.0, 0.0]
            } else {
                [8.0, 0.0, 8.0]
            }
        };
        let walked = placed_run(&config, &walk, rules, place, |sim| {
            assert!(!sim.states.iter().any(inside), "a centre entered a pillar");
        });
        assert!(
            walked.states[0].pos[0] < -1.0,
            "the pillar blocked the walk"
        );
        // Eight brains brawl around both pillars for 10 s.
        let brawl = ArenaConfig::try_new(2040, 600, 8, 20.0).expect("valid config");
        whole_v2_checked(&brawl, rules, |sim| {
            assert!(!sim.states.iter().any(inside), "a brain entered a pillar");
        });
    }

    /// Whole-match v2 over the default (brain-filled) tape, checking `each`.
    #[cfg(not(feature = "sim"))]
    fn whole_v2_checked(
        config: &ArenaConfig,
        rules: ArenaRules,
        mut each: impl FnMut(&Sim),
    ) -> Sim {
        let tape = FighterTape::default();
        let mut sim = Sim::build(config, &tape, false, false).expect("valid build");
        assert!(sim.set_rules(rules));
        while sim.tick < config.ticks {
            sim.step(&tape).expect("valid step");
            each(&sim);
        }
        sim
    }

    #[cfg(not(feature = "sim"))]
    #[test]
    fn push_apart_digest_twice_equal_and_tagged() {
        let config = ArenaConfig::try_new(2040, 600, 8, 20.0).expect("valid config");
        let rules = ArenaRules::v2_free_for_all()
            .with_pillar(crate::Pillar::new([-500, -500], [500, 500]))
            .expect("room for a pillar");
        let run = |rules: ArenaRules| whole_v2_checked(&config, rules, |_| {}).checkpoints;
        let (first, second) = (run(rules), run(rules));
        assert_eq!(first, second, "collision digest x2");
        let open = ArenaRules {
            collision: false,
            ..rules
        };
        assert_ne!(first.last(), run(open).last(), "the collision tag folds");
    }
    /// Slot 0 holds attack facing +x; slot 1 stands 1 m ahead facing it and
    /// tries to walk away (+x) the whole time.
    fn combo_tape(config: &ArenaConfig) -> FighterTape {
        let mut inputs = Vec::new();
        for tick in 0..config.ticks {
            inputs.push(FighterInput {
                tick,
                fighter: FighterId(0),
                mx: 0,
                mz: 0,
                facing: 9_000,
                buttons: Buttons(ATTACK),
            });
            inputs.push(FighterInput {
                tick,
                fighter: FighterId(1),
                mx: 40,
                mz: 0,
                facing: -9_000,
                buttons: Buttons::NONE,
            });
        }
        FighterTape(inputs)
    }

    #[test]
    fn balance_wire_combo_floors_the_victim_and_hashes_twice() {
        // EG34: integer balance in the v2 step. The first hit stumbles slot 1,
        // the follow-up inside the stagger window floors it, and a fallen or
        // catching fighter cannot move even while it pushes its wish.
        let config = ArenaConfig::try_new(2040, 120, 2, 20.0).expect("valid config");
        let tape = combo_tape(&config);
        let place = |slot: usize| {
            if slot == 0 {
                [0.0, 0.0, 0.0]
            } else {
                [1.0, 0.0, 0.0]
            }
        };
        let rules = ArenaRules {
            collision: false,
            ..ArenaRules::v2_free_for_all()
        };
        let mut fell = false;
        let mut last = None;
        let first = placed_run(&config, &tape, rules, place, |sim| {
            let down = matches!(
                sim.balance[1].gait(),
                engine_physics::balance::GaitPhase::Fallen
                    | engine_physics::balance::GaitPhase::Catching
            );
            let pos = sim.states[1].pos;
            if down && last.is_some_and(|was: (bool, [f32; 3])| was.0) {
                assert_eq!(Some(pos), last.map(|was| was.1), "down fighters hold still");
            }
            fell |= down;
            last = Some((down, pos));
        });
        assert!(fell, "the combo floors slot 1");
        assert!(first.balance[1].fall_count() >= 1);
        let second = placed_run(&config, &tape, rules, place, |_| {});
        assert_eq!(first.checkpoints, second.checkpoints, "balance digest x2");
        let unwired = placed_run(
            &config,
            &tape,
            ArenaRules {
                balance: false,
                ..rules
            },
            place,
            |_| {},
        );
        assert_ne!(
            first.checkpoints, unwired.checkpoints,
            "the balance tag folds"
        );
        // V1 never wires balance: the frozen hashes cannot see it.
        let v1 = placed_run(&config, &tape, ArenaRules::V1, place, |_| {});
        assert!(v1.balance.iter().all(|state| state.shove_count() == 0));
    }

    #[test]
    fn live_balance_forwards_sim_verdict() {
        // EG45: the live read forwards the authority verdict. A v2 combo
        // floors slot 1, so the live read reports Fallen or Catching, a bad
        // slot reads None, and two runs agree.
        let config = ArenaConfig::try_new(2040, 120, 2, 20.0).expect("valid config");
        let tape = combo_tape(&config);
        let rules = ArenaRules {
            collision: false,
            ..ArenaRules::v2_free_for_all()
        };
        let place = |slot: usize| {
            if slot == 0 {
                [0.0, 0.0, 0.0]
            } else {
                [1.0, 0.0, 0.0]
            }
        };
        let oracle = placed_run(&config, &tape, rules, place, |_| {});
        let oracle_gait = oracle.balance[1].gait();
        assert!(
            matches!(
                oracle_gait,
                engine_physics::balance::GaitPhase::Fallen
                    | engine_physics::balance::GaitPhase::Catching
            ),
            "the combo floors slot 1"
        );
        let run = || {
            let mut live = LiveMatch::with_rules(&config, rules).expect("v2 match");
            live.sim.states[0].pos = [0.0, 0.0, 0.0];
            live.sim.states[1].pos = [1.0, 0.0, 0.0];
            let mut checkpoints = Vec::with_capacity(config.ticks as usize);
            let mut fell = false;
            for tick in 0..config.ticks {
                let stepped = live.step(tick_inputs(&tape, tick)).expect("live step");
                checkpoints.push(stepped.checkpoint);
                let gait = live.balance(1).expect("slot 1 verdict").gait();
                fell |= matches!(
                    gait,
                    engine_physics::balance::GaitPhase::Fallen
                        | engine_physics::balance::GaitPhase::Catching
                );
                assert!(live.balance(99).is_none(), "bad slot reads None");
            }
            let final_gait = live.balance(1).expect("final verdict").gait();
            (final_gait, checkpoints, fell)
        };
        let (first_gait, first_checks, first_fell) = run();
        assert!(first_fell, "the live combo floors slot 1");
        assert_eq!(first_gait, oracle_gait, "live forwards the sim gait");
        let (second_gait, second_checks, _) = run();
        assert_eq!(first_gait, second_gait, "balance x2");
        assert_eq!(first_checks, second_checks, "digest x2");
    }

    #[test]
    fn live_match_surfaces_press_cues_in_fighter_id_order() {
        use engine_combat::fight::AttackStarted;
        use engine_combat::tables::TABLES_VERSION_V2;
        use engine_input::fighter::ATTACK;

        let base = ArenaConfig::try_new(2040, 32, 2, 20.0).expect("valid config");
        let config = ArenaConfig {
            table_version: TABLES_VERSION_V2,
            ..base
        };
        let rules = ArenaRules::v2_free_for_all();
        let mut inputs = Vec::new();
        for tick in 0..config.ticks {
            for fighter in 0..config.fighters {
                let buttons = if tick == 0 {
                    Buttons(ATTACK)
                } else {
                    Buttons::NONE
                };
                inputs.push(FighterInput {
                    tick,
                    fighter: FighterId(fighter),
                    mx: 0,
                    mz: 0,
                    facing: 0,
                    buttons,
                });
            }
        }
        let tape = FighterTape(inputs);
        let mut live = LiveMatch::with_rules(&config, rules).expect("v2 match");
        live.step(tick_inputs(&tape, 0)).expect("live step");
        assert_eq!(
            live.cues(),
            [AttackStarted { attacker: 0 }, AttackStarted { attacker: 1 }].as_slice(),
            "press tick surfaces both cues in fighter-ID order"
        );
        live.step(tick_inputs(&tape, 1)).expect("live step");
        assert!(live.cues().is_empty(), "cues never linger a tick");
        let v1 = ArenaConfig::try_new(2040, 32, 2, 20.0).expect("valid config");
        let mut live_v1 = LiveMatch::new(&v1).expect("live match");
        live_v1.step(tick_inputs(&tape, 0)).expect("live step");
        assert!(live_v1.cues().is_empty(), "v1 stream stays frozen");
    }

    #[test]
    fn live_match_rejects_bad_slices_without_stepping() {
        let config = ArenaConfig::try_new(7, 8, 2, 12.0).expect("valid config");
        let mut live = LiveMatch::new(&config).expect("live match");
        let good = FighterInput {
            tick: 0,
            fighter: FighterId(0),
            mx: 10,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
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
        ] {
            assert!(live.step(bad).is_none());
            assert_eq!((live.tick(), live.steps()), (0, 0));
            assert!(!live.is_poisoned());
        }
        assert!(live.step(&[good]).is_some());
        assert_eq!((live.tick(), live.steps()), (1, 1));
        assert!(LiveMatch::new(&ArenaConfig {
            fighters: 1,
            ..config
        })
        .is_none());
    }

    #[test]
    fn live_mind_stamped_replay_matches_tape() {
        // AL1 slice 1: provider replies are validated, stamped for tick N+k
        // and drained at the tick boundary; replay with the provider off
        // reads the recorded tape to the same hash.
        let config = ArenaConfig::try_new(2040, 600, 2, 20.0).expect("valid config");
        let latency = 3u32;
        let mut live = LiveMatch::new(&config).expect("live match");
        let reply_for = |tick: u32| FighterInput {
            tick,
            fighter: FighterId(0),
            mx: match (tick / 15) % 4 {
                0 => 100,
                2 => -100,
                _ => 0,
            },
            mz: match (tick / 15) % 4 {
                1 => 100,
                3 => -100,
                _ => 0,
            },
            facing: 0,
            buttons: if tick.is_multiple_of(30) {
                Buttons(ATTACK)
            } else {
                Buttons::NONE
            },
        };
        for tick in 0..latency.min(config.ticks) {
            assert!(live.stamp(reply_for(tick)), "stamp tick {tick}");
        }
        let mut checkpoints = Vec::with_capacity(config.ticks as usize);
        for tick in 0..config.ticks {
            let future = tick + latency;
            if future < config.ticks {
                assert!(live.stamp(reply_for(future)), "stamp tick {future}");
            }
            let stepped = live.step(&[]).expect("live step");
            checkpoints.push(stepped.checkpoint);
        }
        let past_due = FighterInput {
            tick: 0,
            fighter: FighterId(0),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        };
        assert!(!live.stamp(past_due), "past-due stamp refused");
        let out_of_range = FighterInput {
            tick: config.ticks,
            fighter: FighterId(0),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        };
        assert!(!live.stamp(out_of_range), "out-of-range stamp refused");
        assert!(!live.is_poisoned(), "refusals never poison");
        let tape = live.recorded_tape();
        assert_eq!(
            tape.0.len(),
            config.ticks as usize,
            "one stamped input per tick"
        );
        let replay = crate::run(&config, &tape);
        assert_eq!(checkpoints, replay, "replay with provider off matches");
        assert_eq!(replay, crate::run(&config, &tape), "replay x2");
    }

    use engine_provider::{fixtures, validate_proposal, MockAdapter, MockOutcome, ProviderAdapter};

    /// AL1 reply mapping: a validated mock reply becomes one fighter-0 input
    /// for `tick`. The mock's canned proposal has no fighter fields, so the
    /// request id picks the canned move (ids 1..=4). Pure: no clock, no state.
    fn mind_input(outcome: &MockOutcome, tick: u32) -> Option<FighterInput> {
        let proposal = outcome.result.as_ref().ok()?.proposal.as_ref()?;
        validate_proposal(proposal).ok()?;
        let (mx, mz, buttons) = match proposal.request.0 {
            1 => (100, 0, Buttons::NONE),
            2 => (0, 100, Buttons(ATTACK)),
            3 => (-100, 0, Buttons::NONE),
            4 => (0, -100, Buttons::NONE),
            _ => return None,
        };
        Some(FighterInput {
            tick,
            fighter: FighterId(0),
            mx,
            mz,
            facing: 0,
            buttons,
        })
    }

    #[test]
    fn live_mind_provider_replay_matches_live_hash() {
        // AL1 slice 1 on the mock adapter: three provider replies at different
        // delays are stamped at issue and drained at the tick boundary. The
        // recorded tape replays with the provider off to the live hash, twice.
        // A late reply (its tick already run) is refused, leaves no tape row
        // and the bout equals the on-time bout. Gap: nothing logs the fallback
        // (AL1b, engine-observe reason); the refusal is the only record.
        let config = ArenaConfig::try_new(2040, 600, 2, 20.0).expect("valid config");
        // One bout driven by the mock provider. Canned reply n (request id n + 1)
        // is issued at tick issue and stamped for issue + delay, with (issue,
        // delay) from the plan. With late, one more reply (id 4) is issued at
        // tick at for tick target, which already ran, and must be refused.
        let provider_bout = |late: Option<(u32, u32)>| -> (Vec<Checkpoint>, FighterTape) {
            let plan = [(0u32, 2u32), (40, 3), (80, 5)];
            let policy = fixtures::policy();
            let caps = fixtures::caps();
            let mut provider = MockAdapter::new(Vec::new());
            let mut live = LiveMatch::new(&config).expect("live match");
            let mut checkpoints = Vec::with_capacity(config.ticks as usize);
            for tick in 0..config.ticks {
                for (n, &(issue, delay)) in plan.iter().enumerate() {
                    if issue == tick {
                        let request = fixtures::request(n as u64 + 1, 0);
                        let outcome = provider.call(&request, &policy, &caps);
                        let input = mind_input(&outcome, issue + delay).expect("canned reply");
                        assert!(live.stamp(input), "stamp for tick {}", issue + delay);
                    }
                }
                if let Some((at, target)) = late.filter(|&(at, _)| at == tick) {
                    let request = fixtures::request(4, 0);
                    let outcome = provider.call(&request, &policy, &caps);
                    let input = mind_input(&outcome, target).expect("canned late reply");
                    assert!(
                        !live.stamp(input),
                        "late reply at {at} for tick {target} refused"
                    );
                }
                let stepped = live.step(&[]).expect("live step");
                checkpoints.push(stepped.checkpoint);
            }
            assert!(!live.is_poisoned(), "refusals never poison");
            (checkpoints, live.recorded_tape())
        };
        let (live_checks, tape) = provider_bout(None);
        assert_eq!(
            tape.0.iter().map(|input| input.tick).collect::<Vec<_>>(),
            [2u32, 43, 85],
            "each reply drains at issue + delay"
        );
        let live_hash = live_checks.last().expect("600 ticks").hash;
        let first = crate::run(&config, &tape);
        let second = crate::run(&config, &tape);
        assert_eq!(first, second, "replay x2");
        assert_eq!(first, live_checks, "replay with provider off matches live");
        assert_eq!(first.last().expect("600 ticks").hash, live_hash);
        let (late_checks, late_tape) = provider_bout(Some((50, 10)));
        assert_eq!(late_tape.0, tape.0, "late reply leaves no tape row");
        assert_eq!(
            late_checks, live_checks,
            "late fallback equals the on-time bout"
        );
    }

    #[test]
    fn live_mind_late() {
        // AL1b slice 1: a mock provider 3 ticks late never applies late. Each
        // past-deadline reply is refused, logged as fallback with its
        // engine-observe reason, and the bout equals the scripted-only bout
        // plus its provider-off replay.
        let config = ArenaConfig::try_new(2040, 600, 2, 20.0).expect("valid config");
        let policy = fixtures::policy();
        let caps = fixtures::caps();
        let scripted_checks = {
            let mut live = LiveMatch::new(&config).expect("live match");
            let mut checks = Vec::with_capacity(config.ticks as usize);
            for tick in 0..config.ticks {
                let stepped = live.step(&[]).expect("live step");
                assert_eq!(stepped.checkpoint.tick, tick, "checkpoint {tick}");
                checks.push(stepped.checkpoint);
            }
            assert!(live.fallbacks.is_empty(), "scripted bout logs nothing");
            checks
        };
        let mut provider = MockAdapter::new(Vec::new());
        let mut live = LiveMatch::new(&config).expect("live match");
        let mut checks = Vec::with_capacity(config.ticks as usize);
        let mut expected_fallbacks = 0usize;
        for tick in 0..config.ticks {
            if tick >= 3 {
                let target = tick - 3;
                let request = fixtures::request(u64::from(target % 4 + 1), 0);
                let outcome = provider.call(&request, &policy, &caps);
                let input = mind_input(&outcome, target).expect("canned late reply");
                assert!(
                    !live.stamp(input),
                    "reply 3 ticks late at {tick} for tick {target} refused"
                );
                expected_fallbacks += 1;
                assert_eq!(
                    live.fallbacks.len(),
                    expected_fallbacks,
                    "late reply logged as fallback each time"
                );
                let logged = live.fallbacks.last().expect("logged fallback");
                assert_eq!(logged.tick, target, "fallback names the refused tick");
                assert_eq!(
                    logged.reason, PROVIDER_TIMEOUT_REASON,
                    "fallback carries its engine-observe reason"
                );
                assert_eq!(
                    logged.reason, "provider.timeout",
                    "reason is the engine-observe code name"
                );
            }
            let stepped = live.step(&[]).expect("live step");
            checks.push(stepped.checkpoint);
        }
        assert!(!live.is_poisoned(), "refusals never poison");
        assert_eq!(
            live.fallbacks.len(),
            expected_fallbacks,
            "every late reply logged once"
        );
        assert!(
            live.fallbacks
                .iter()
                .all(|fallback| fallback.reason == PROVIDER_TIMEOUT_REASON),
            "every fallback carries the engine-observe reason"
        );
        assert_eq!(
            checks, scripted_checks,
            "late bout equals the scripted-only bout"
        );
        let tape = live.recorded_tape();
        assert!(tape.0.is_empty(), "late replies leave no tape rows");
        let first = crate::run(&config, &tape);
        assert_eq!(first, checks, "provider-off replay matches the late bout");
        assert_eq!(first, crate::run(&config, &tape), "replay x2");
        assert_eq!(
            first, scripted_checks,
            "replay equals the scripted-only bout"
        );
    }
}

/// The spear grapple through the live match: determinism, replay and the
/// inert-when-off promise. Scenario tests live in `crate::spear`.
#[cfg(all(test, not(feature = "sim")))]
mod spear_tests {
    use super::*;
    use engine_core::fixed::Fixed;
    use engine_input::fighter::THROW_SPEAR;
    use engine_input::{Buttons, FighterId};

    const TICKS: u32 = 600;

    fn spec(spear: bool) -> MatchSpec {
        let config = ArenaConfig {
            seed: 0x5EA2,
            ticks: TICKS,
            fighters: 2,
            half_extent_m: 12.0,
            table_version: 2,
        };
        let spec = MatchSpec::duel(config, MatchWorld::empty());
        if spear {
            spec.with_spear()
        } else {
            spec
        }
    }

    /// Facing in centidegrees (0 is +z, 9000 is +x) from `me` toward `foe`,
    /// through the integer `Fixed::atan2` (no platform trig).
    fn aim(me: [f32; 3], foe: [f32; 3]) -> i16 {
        let q16 = |v: f32| Fixed::from_bits((v * 65_536.0) as i64);
        let angle = Fixed::atan2(q16(foe[0] - me[0]), q16(foe[2] - me[2]));
        i16::try_from(angle.to_bits() * 18_000 / Fixed::PI.to_bits()).unwrap_or(0)
    }

    /// Both fighters stand still, face each other and throw every 50 ticks
    /// (slot 1 half a period later); slot 1 guards on a cycle. Aimed from the
    /// live poses; returns the hashes, the recorded tape and every spear event.
    fn duel(spec: &MatchSpec, throw: bool) -> (Vec<u64>, FighterTape, Vec<crate::SpearEvent>) {
        let mut live = spec.live().expect("spec builds");
        let (mut hashes, mut seen) = (Vec::new(), Vec::new());
        for tick in 0..TICKS {
            let poses = [live.sim.states[0].pos, live.sim.states[1].pos];
            let inputs: Vec<FighterInput> = (0..2u8)
                .map(|slot| {
                    let me = usize::from(slot);
                    let phase = tick + u32::from(slot) * 25;
                    let mut buttons = 0;
                    if throw && phase % 50 == 0 {
                        buttons |= THROW_SPEAR;
                    }
                    if slot == 1 && (tick / 40) % 3 == 2 {
                        buttons |= engine_input::fighter::BLOCK;
                    }
                    FighterInput {
                        tick,
                        fighter: FighterId(slot),
                        mx: 0,
                        mz: 0,
                        facing: aim(poses[me], poses[1 - me]),
                        buttons: Buttons(buttons),
                    }
                })
                .collect();
            hashes.push(live.step(&inputs).expect("step").checkpoint.hash);
            seen.extend_from_slice(live.spear_events());
        }
        (hashes, live.recorded_tape(), seen)
    }

    #[test]
    fn spear_duel_600_ticks_digest_identical_twice_and_across_tape_replay() {
        let spec = spec(true);
        let (first, tape, events) = duel(&spec, true);
        let (second, _, _) = duel(&spec, true);
        assert_eq!(first, second, "x2 identical");
        let thrown = |e: &&crate::SpearEvent| matches!(e, crate::SpearEvent::Thrown { .. });
        let tethers = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    crate::SpearEvent::Hit {
                        victim: Some(_),
                        ..
                    }
                )
            })
            .count();
        assert!(events.iter().filter(thrown).count() >= 4, "spears flew");
        assert!(tethers >= 2, "the spear was in play: {tethers} tethers");
        assert!(events.iter().any(|e| matches!(
            e,
            crate::SpearEvent::TetherEnd {
                damage_permille: 1..,
                ..
            }
        )));
        let mut replay = spec.live().expect("replay");
        let mut again = Vec::new();
        for tick in 0..TICKS {
            let row: Vec<FighterInput> =
                tape.0.iter().filter(|i| i.tick == tick).copied().collect();
            again.push(replay.step(&row).expect("replay step").checkpoint.hash);
        }
        assert_eq!(first, again, "tape replay reproduces the digest");
    }

    #[test]
    fn spear_off_ignores_the_button_and_keeps_the_old_digest() {
        let off = spec(false);
        assert!(off.live().expect("builds").spear_view().is_empty());
        let (pressed, ..) = duel(&off, true);
        let (quiet, ..) = duel(&off, false);
        assert_eq!(
            pressed, quiet,
            "the bit changes nothing when the spear is off"
        );
        let (armed, ..) = duel(&spec(true), true);
        assert_ne!(pressed, armed, "switching it on plays another match");
    }

    #[test]
    fn spear_matches_have_no_snapshot_and_report_their_view() {
        let mut live = spec(true).live().expect("builds");
        assert!(live.snapshot().is_none(), "rollback rebuilds from the spec");
        assert_eq!(live.spear_view().len(), 2);
        live.step(&[]).expect("step");
        assert!(live.spear_events().is_empty());
    }

    /// Walks slots 0..4 south and slots 4..8 north, so the two lines pass
    /// straight through each other in four one-metre lanes.
    #[cfg(not(feature = "sim"))]
    struct Crossing;

    #[cfg(not(feature = "sim"))]
    impl crate::controller::SlotController for Crossing {
        fn controls(&self, slot: u8) -> bool {
            usize::from(slot) < 8
        }

        fn control(&mut self, _: &crate::controller::ControlView<'_>, row: &mut [FighterInput]) {
            for (slot, input) in row.iter_mut().enumerate() {
                input.mx = 0;
                input.mz = if slot < 4 { -100 } else { 100 };
                input.facing = 0;
                input.buttons = Buttons::NONE;
            }
        }
    }

    /// Eight slots on two facing lines of four lanes, each walking straight
    /// through the other line for the whole run; returns the checkpoints and
    /// the summed collision push in millimetres (the displacement beyond the
    /// walk and the ORCA correction).
    #[cfg(not(feature = "sim"))]
    fn orca_crossing(armed: bool) -> (Vec<Checkpoint>, f64) {
        const TICKS: u32 = 120;
        let config = ArenaConfig::try_new(2040, TICKS, 8, 20.0).expect("valid config");
        let mut sim = Sim::build(&config, &FighterTape::default(), false, false).expect("build");
        assert!(sim.set_rules(ArenaRules::v2_free_for_all()));
        if armed {
            assert!(sim.set_steering(), "v2 arms the ORCA channel");
        }
        for (slot, state) in sim.states.iter_mut().enumerate() {
            let x = [-1.5, -0.5, 0.5, 1.5][slot % 4];
            let z = if slot < 4 { 4.0 } else { -4.0 };
            state.pos = [x, 0.0, z];
        }
        sim.trace = Some(Vec::new());
        let mut push_mm = 0.0f64;
        let mut checkpoints = Vec::with_capacity(TICKS as usize);
        while sim.tick < config.ticks {
            let before: Vec<[f32; 3]> = sim.states.iter().map(|state| state.pos).collect();
            sim.step_with(&FighterTape::default(), Some(&mut Crossing))
                .expect("crossing step");
            let trace = sim
                .trace
                .as_ref()
                .and_then(|rows| rows.last())
                .expect("row");
            let applied = sim
                .steer
                .as_ref()
                .map(|steering| steering.applied)
                .unwrap_or([[0; 2]; MAX_SLOTS]);
            for (slot, state) in sim.states.iter().enumerate() {
                let moved = [
                    state.pos[0] - before[slot][0],
                    state.pos[2] - before[slot][2],
                ];
                let push = [
                    moved[0] - trace[slot].resolved[0] - applied[slot][0] as f32 / 1000.0,
                    moved[1] - trace[slot].resolved[1] - applied[slot][1] as f32 / 1000.0,
                ];
                push_mm += f64::from((push[0] * push[0] + push[1] * push[1]).sqrt());
            }
            checkpoints.push(*sim.checkpoints.last().expect("checkpoint"));
        }
        (checkpoints, push_mm)
    }

    #[cfg(not(feature = "sim"))]
    #[test]
    fn orca_steering_halves_push_apart_for_eight_crossing_slots() {
        let (first, armed_push) = orca_crossing(true);
        let (again, _) = orca_crossing(true);
        assert_eq!(first, again, "the armed run hashes twice the same");
        let (_, plain_push) = orca_crossing(false);
        assert!(plain_push > 0.0, "the unarmed crossing shoves bodies");
        assert!(
            armed_push <= 0.5 * plain_push,
            "ORCA left {armed_push:.1} mm of push against {plain_push:.1} mm unarmed"
        );
        println!("orca crossing push: {armed_push:.1} mm armed vs {plain_push:.1} mm unarmed");
    }

    #[cfg(not(feature = "sim"))]
    #[test]
    fn orca_live_match_arms_the_same_step_as_the_whole_match() {
        let config = ArenaConfig::try_new(2040, 240, 8, 20.0).expect("valid config");
        let rules = ArenaRules::v2_free_for_all();
        let tape = FighterTape::default();
        // Every body overlaps every other one, so ORCA must change the very
        // first tick; a loose spawn could keep the two hashes equal.
        let cluster = |sim: &mut Sim| {
            for (slot, state) in sim.states.iter_mut().enumerate() {
                state.pos = [
                    [-0.3, -0.1, 0.1, 0.3][slot % 4],
                    0.0,
                    if slot < 4 { -0.1 } else { 0.1 },
                ];
            }
        };
        let mut whole = Sim::build(&config, &tape, false, false).expect("build");
        assert!(whole.set_rules(rules));
        assert!(whole.set_steering(), "v2 arms it");
        assert!(!whole.set_steering(), "arming once is enough");
        cluster(&mut whole);
        while whole.tick < config.ticks {
            whole.step(&tape).expect("step");
        }
        let mut live = LiveMatch::with_rules(&config, rules).expect("live match");
        assert!(live.arm_steering(), "the live match arms the sim channel");
        assert!(!live.arm_steering(), "armed once");
        cluster(&mut live.sim);
        let mut live_checkpoints = Vec::new();
        for _ in 0..config.ticks {
            live_checkpoints.push(live.step(&[]).expect("live step").checkpoint);
        }
        assert_eq!(live_checkpoints, whole.checkpoints, "same step, same hash");
        let mut plain = LiveMatch::with_rules(&config, rules).expect("live match");
        cluster(&mut plain.sim);
        let mut plain_checkpoints = Vec::new();
        for _ in 0..config.ticks {
            plain_checkpoints.push(plain.step(&[]).expect("live step").checkpoint);
        }
        assert_ne!(live_checkpoints, plain_checkpoints, "the channel folds in");
        // v1 rows have no crowd channel, and arming is before the first tick.
        let mut v1 = Sim::build(&config, &tape, false, false).expect("build");
        assert!(!v1.set_steering(), "v1 rows refuse the channel");
        let mut late = LiveMatch::with_rules(&config, rules).expect("live match");
        late.step(&[]).expect("first tick");
        assert!(!late.arm_steering(), "arming is before the first tick");
    }
}
