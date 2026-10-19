//! arena: headless deterministic match spine (contracts C1 + AR2).
//!
//! `FighterState`, `ArenaConfig`, `Checkpoint` and fixture-v2 seed expansion
//! are the shared types (AR1). AR2 integrates the lower lanes through their
//! public APIs only: fight rules (`engine-combat` strict [`try_step`]),
//! gladiator brains (`engine-ai` [`Brain`] plus [`Percept`]), root-motion
//! presentation (`engine-anim` [`extract_root`] plus [`sample_pose`]) and
//! the bonded crate prop (`engine-destruction` [`BondedCollection`]: sword
//! hits fracture it, unsupported islands fall). Lower crates never import
//! arena.
//!
//! The default build is std-only: [`run`] drives locomotion
//! ([`Locomotion`]) tick-root intent plus
//! combat [`try_step`], brain fill, anim presentation and the FNV checkpoint
//! hash, with no physics dependency. The opt-in `sim` feature resolves each
//! requested root delta through one physics controller (engine-physics,
//! `sim` feature) per fighter
//! (`move_and_slide` exactly once per tick; floor plus walls mirror the
//! default wall clamp): the controller hash folds into checkpoints behind a
//! `sim` tag byte so default and `sim` hashes never compare equal.
//! NET1/LOC2/FK1 ship with `sim` enabled.
//!
//! Authority per tick: tape inputs cover human slots and one [`Brain`] per
//! fighter fills the slots without a tape input; the merged inputs resolve
//! through combat [`try_step`] (loud rejection, never tool-style silent
//! skip), then each slot's wish drives its [`Locomotion`] (LOC1 public API:
//! tick-root intent plus presentation pose); combat hit events map to
//! locomotion interrupts (hit-stop freezes gait: any hit/parry freezes every
//! slot, a blocked chip freezes its victim); the requested root delta
//! resolves through collision exactly once (controller under `sim`, single
//! wall clamp otherwise; blocked remainder is never reapplied) and advances
//! the authoritative pose (knocked-out fighters hold pose); the FNV
//! checkpoint hash covers the canonical bytes plus combat hp/stamina, combat
//! events, locomotion state, a `sim` tag byte and (under `sim`) the
//! controller hashes. Human wishes arrive as [`FighterInput`] (the IN1
//! [`Bindings`](engine_input::Bindings) output form: quantized wishes plus
//! absolute facing, fixed ticks only, never a render camera or frame time).
//! Presentation ([`run_presentation`], [`run_loco_poses`],
//! [`presentation_pose`]) reads the recorded trajectory/poses through anim
//! only and never feeds the hash.
//!
//! Units: meters, radians, fixed ticks at [`TICK_HZ`]. Determinism scope:
//! same-machine reproducibility (floats enter the hash only as `to_bits`;
//! under `sim` the physics world's own quantized hash folds in too); same
//! config plus tape twice gives identical checkpoints, and snapshot/restore
//! resimulates identical.
//! Error policy: invalid config or tape rejects the whole run as empty (or
//! `None`), never panics and never skips a single bad input silently.
//!
//! [`try_step`]: engine_combat::try_step
//! [`Brain`]: engine_ai::Brain
//! [`Percept`]: engine_ai::Percept
//! [`extract_root`]: engine_anim::pose::extract_root
//! [`sample_pose`]: engine_anim::pose::sample_pose

pub mod collide;
pub mod controller;
mod engage;
pub mod eqs_bind;
pub mod fork;
pub mod gameworld;
pub mod gym;
pub mod limbs;
pub mod live;
mod match_observer;
mod match_wire;
#[cfg(all(feature = "spine", not(feature = "sim")))]
pub mod parity;
pub mod probe;
pub mod recorder;
pub mod seed;
pub mod spear;
pub mod spec;
#[cfg(feature = "spine")]
pub mod spine_step;
pub mod sync;
pub mod world;
pub mod zones;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_core::alloc_probe::CountingAlloc =
    engine_core::alloc_probe::CountingAlloc;

pub use collide::{Pillar, COLLIDE_RADIUS_MM, MAX_PILLARS};
pub use controller::{ControlView, ScriptEffect, SlotController, SlotView};
pub use engine_combat::fight::{FIGHT_RULES_V1, FIGHT_RULES_V2};
pub use engine_input::{BoutTape, FighterId, FighterInput, FighterTape, MAX_PARTICIPANTS};
/// Neutral body names for below-arena ledgers: aliases of the input types.
pub use engine_input::{FighterId as BodyId, FighterInput as BodyInput, FighterTape as BodyTape};
pub use gym::{
    initial_state, legal_mask_of, observe_state, reward_of, vec_step, BatchEnv, GymFighterState,
    GymSnapshot, GYM_ACTION_ATTACK, GYM_ACTION_BLOCK, GYM_ACTION_DODGE, GYM_NUM_ACTIONS,
};
pub use live::{LiveMatch, LivePresentation, LiveTick};
pub use spear::{EndWhy, MissWhy, SpearEvent, SpearSlot};
pub use spec::{MatchSpec, DEFAULT_INPUT_DELAY_TICKS};

use core::f32::consts::PI;

use engine_ai::{DepthBrain, Intent, Percept, Personality};
use engine_anim::clip::TICK_DT;
use engine_anim::contract::{AnimError, LocalPose, RootTable, Skeleton};
use engine_anim::locomotion::{CombatInterrupt, Locomotion, LOCO_LEG_JOINTS};
use engine_anim::motion_net::state_code;
use engine_anim::pose::{extract_root, sample_pose};
use engine_anim::warp_ground::GroundWarp;
use engine_anim::Clip;
use engine_combat::arena::MatchState as CombatMatchState;
use engine_combat::depth::{DepthSet, PoiseMeter};
use engine_combat::exchange::{ExchangeEvent, ExchangeIntent, PhasedFighter};
use engine_combat::fight::TABLES_V2;
use engine_combat::fight::{
    try_step_v2_with_cues_into, AttackStarted, StrikePose, MAX_ATTACK_STARTED_PER_TICK,
};
use engine_combat::sweep::WeaponSweep;
use engine_combat::sweep::MAX_SWEEPS_PER_TICK;
use engine_combat::tables::{TABLES_VERSION_V1, TABLES_VERSION_V2};
use engine_combat::{try_step_into, FightEvent, FightState, MAX_EVENTS_PER_TICK, TABLES_V1};
use engine_core::fixed::Fixed;
use engine_destruction::bonded::{event_bytes as break_bytes, BondedCollection, BreakEvent};
use engine_input::fighter::{ATTACK, DODGE, THROW_SPEAR};
use engine_physics::balance::BalanceParams;
/// Integer balance verdicts the window reads (`Sim::balance`): re-exported so
/// the presentation leg names the return without a physics edge.
pub use engine_physics::balance::{BalanceState, GaitPhase};
/// RZ7 capes: the CPU Verlet cloth sheet the window pins at each fighter's
/// shoulders and hangs on the arena banners. Re-exported like the balance
/// verdicts above so the presentation leg cloths a fighter without a physics
/// edge; it is presentation only and the sim never reads it.
pub use engine_physics::cloth;
#[cfg(feature = "sim")]
use engine_physics::controller::{Controller, ControllerSnapshot};
use engine_physics::limb::{LimbPool, MAX_LIMBS};
#[cfg(feature = "sim")]
use engine_physics::rapier::FIXED_DT;
use engine_physics::stagger_memory::StaggerMemory;
pub use match_observer::{
    run_match_bytes, run_match_bytes_v1, run_match_bytes_versioned, run_match_events,
    run_match_events_v1, run_match_events_versioned, TickMatchEvents,
};
pub use match_wire::{MATCH_WIRE_VERSION_V0, MATCH_WIRE_VERSION_V1};
use recorder::{Dataset, DATASET_VERSION};
use world::MatchWorld;

/// Top speed at full wish (`100`) in meters per second.
pub const TOP_SPEED_MPS: f32 = 5.0;

/// Fixed steps per second for every headless match.
/// Single owner of the rate is `engine_core::TICK_HZ`.
pub const TICK_HZ: u32 = engine_core::TICK_HZ as u32;
/// True when compiled with the `sim` physics track. Checkpoint hashes carry
/// a track tag byte, so default and `sim` builds of the same match hash
/// differently by design; budget harnesses (PF1) key their baselines on this.
#[cfg(feature = "sim")]
pub const SIM_TRACK: bool = true;
/// True when compiled with the `sim` physics track (see above).
#[cfg(not(feature = "sim"))]
pub const SIM_TRACK: bool = false;
/// Kinematic capsule radius in meters.
pub const FIGHTER_RADIUS_M: f32 = 0.4;
/// Kinematic capsule height in meters.
pub const FIGHTER_HEIGHT_M: f32 = 1.8;

/// Perception period for AI-filled slots, in fixed ticks. The arena maps a
/// fresh [`Percept`] every tick; each [`DepthBrain`]
/// owns its clock and repeats its last intent between perception ticks.
pub const AI_PERIOD_TICKS: u32 = 6;
/// Physics capsule cylinder mid-section height in meters (`sim` only: the
/// Rapier settling track spawns one capsule per fighter).
#[cfg(feature = "sim")]
pub const CAPSULE_CYL_H_M: f32 = 1.0;
/// Physics capsule center rest height over the floor in meters (`sim` only).
#[cfg(feature = "sim")]
pub const CAPSULE_REST_Y_M: f32 = 0.5 * CAPSULE_CYL_H_M + FIGHTER_RADIUS_M;

/// Slot count sized by the fighter contract.
const MAX_SLOTS: usize = MAX_PARTICIPANTS as usize;

/// Match configuration: all values validated before a run.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ArenaConfig {
    /// Initialization seed; expanded with [`seed::expand`].
    pub seed: u64,
    /// Fixed steps, `1..=MAX_TICKS`.
    pub ticks: u32,
    /// Fighters, `2..=MAX_PARTICIPANTS`.
    pub fighters: u8,
    /// Arena half-extent in meters, `5.0..=100.0`.
    pub half_extent_m: f32,
    /// Move-table version, `1..=2`: `1` ships (startup 4), `2` reads
    /// (startup 15, parry 7). Default `1`; unknown refused.
    pub table_version: u32,
}

/// Bound replay work per match.
pub const MAX_TICKS: u32 = 100_000;

/// Fight rules a match runs under (EG3). [`ArenaRules::V1`] is the frozen
/// legacy hit rule every existing runner, tape and checkpoint uses: an impact
/// strikes every living foe at any range and a block covers every direction.
/// Rules v2 ([`ArenaRules::v2_free_for_all`], [`ArenaRules::v2_teams`]) gate
/// each impact through the sweep capsule test over the slots' integer-mm
/// poses and integer facing: out-of-reach or behind-the-attacker foes and
/// teammates are not struck, and a block or parry needs the shield cone to
/// face the attacker. V2 checkpoints fold the version plus the team ids
/// after the v1 bytes, so v1 and v2 hashes never compare equal and v1 hashes
/// stay byte-frozen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ArenaRules {
    /// [`FIGHT_RULES_V1`] or [`FIGHT_RULES_V2`].
    pub version: u32,
    /// Team id per slot; equal ids never strike each other (v2 only).
    pub teams: [u8; MAX_SLOTS],
    /// Fighter collision in the default leg (EG20, [`collide`]): capsules
    /// push apart, pillars block and the wall is round. Rules v2 turns it
    /// on; v1 never collides. The `sim` leg resolves motion through its
    /// physics controllers instead and skips this pass.
    pub collision: bool,
    /// Static full-height pillars the collision pass keeps fighters out of,
    /// in slot order of placement; `None` entries are empty.
    pub pillars: [Option<Pillar>; MAX_PILLARS],
    /// Integer balance in the step (EG34): every landed blow feeds the
    /// victim's [`BalanceState`] (stagger, stumble, fall, catch), a fallen or
    /// catching fighter cannot move, and each balance hash folds into the
    /// checkpoint. Rules v2 turns it on; v1 never wires it. The Rapier
    /// ragdoll stays client-local, seeded from the same events, never hashed.
    pub balance: bool,
}

impl ArenaRules {
    /// The frozen legacy rules (each slot its own team; teams unread).
    pub const V1: Self = Self {
        version: FIGHT_RULES_V1,
        teams: solo_teams(),
        collision: false,
        pillars: [None; MAX_PILLARS],
        balance: false,
    };

    /// Rules v2, every slot on its own team (the arena's free-for-all).
    #[must_use]
    pub const fn v2_free_for_all() -> Self {
        Self::v2_teams(solo_teams())
    }

    /// Rules v2 with explicit team ids per slot.
    #[must_use]
    pub const fn v2_teams(teams: [u8; MAX_SLOTS]) -> Self {
        Self {
            version: FIGHT_RULES_V2,
            teams,
            collision: true,
            pillars: [None; MAX_PILLARS],
            balance: true,
        }
    }

    /// These rules plus one more pillar; `None` when all
    /// [`MAX_PILLARS`] slots are taken.
    #[must_use]
    pub fn with_pillar(mut self, pillar: Pillar) -> Option<Self> {
        let free = self.pillars.iter_mut().find(|slot| slot.is_none())?;
        *free = Some(pillar);
        Some(self)
    }

    /// True for a known version (v1 or v2).
    #[must_use]
    pub const fn is_known(&self) -> bool {
        self.version == FIGHT_RULES_V1 || self.version == FIGHT_RULES_V2
    }

    /// True when the default-leg collision pass runs (never under `sim`).
    const fn collides(&self) -> bool {
        self.collision && !cfg!(feature = "sim")
    }

    /// Fold the rules into a v1 checkpoint hash: version, the active slots'
    /// team ids, then (when collision runs) the collision tag byte `0xC0`
    /// plus each pillar footprint, then (when balance is wired) the balance
    /// tag byte `0xB1` plus every slot's [`BalanceState::state_hash`], then
    /// the stagger tag byte `0xB2` plus every slot's
    /// [`StaggerMemory::state_hash`], then the limb tag byte `0xB3` plus
    /// [`LimbPool::state_hash`]. Plain v1 returns the hash untouched
    /// (golden-frozen bytes).
    fn fold(
        &self,
        hash: u64,
        fighters: usize,
        balance: &[BalanceState],
        stagger: &[StaggerMemory],
        limbs: &LimbPool,
    ) -> u64 {
        if self.version == FIGHT_RULES_V1 && !self.collides() && !self.balance {
            return hash;
        }
        let mut digest = engine_core::hash::Fnv64::with_state(hash);
        digest.write_u32(self.version);
        digest.write(self.teams.get(..fighters).unwrap_or(&[]));
        if self.collides() {
            digest.write_u8(0xC0);
            for pillar in self.pillars.iter().flatten() {
                for lane in pillar.min_mm.iter().chain(&pillar.max_mm) {
                    digest.write_i32(*lane);
                }
            }
        }
        if self.balance {
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
        }
        digest.finish()
    }
}

/// Rules-v2 strike pose for one slot: the standard arena hurt capsule and
/// shield cone at the slot's position in integer mm, facing from the
/// absolute IN1 `facing` (centidegrees, 0 is +z, 9000 is +x, the body yaw's
/// source) through the deterministic fixed-point LUT ([`Fixed::sin_cos`]),
/// and a blade from 200 mm to 1500 mm ahead along that facing.
fn strike_pose(id: u8, state: &FighterState, input: &FighterInput, team: u8) -> StrikePose {
    let (fx, fz) = facing_q16(input.facing);
    let mut body = match_wire::sweep_fighter(id, state, input);
    body.facing_x = fx;
    body.facing_z = fz;
    let at = |reach: i32| {
        let along = |unit: i32, base: i32| {
            let offset = i64::from(unit) * i64::from(reach) / Fixed::SCALE;
            base.saturating_add(i32::try_from(offset).unwrap_or(0))
        };
        [
            along(fx, body.pos_mm[0]),
            body.pos_mm[1],
            along(fz, body.pos_mm[2]),
        ]
    };
    StrikePose {
        body,
        blade: WeaponSweep {
            attacker: id,
            p0_mm: at(match_wire::BLADE_BASE_MM),
            p1_mm: at(match_wire::BLADE_REACH_MM),
            blade_half_mm: match_wire::BLADE_HALF_MM,
        },
        team,
    }
}

/// Rules v2: the depth brain's wish in world axes. The brain scores an
/// [`Intent`] whose emitted wish is facing-relative (`mx` forward toward the
/// foe, `mz` to the right), and its steering post-pass then reads that wish
/// as world axes (wall escapes, far-foe seeks), so neither reading of the
/// emitted pair is world motion. V2 rebuilds the wish from the intent in the
/// foe frame instead: forward along the integer foe delta, side along its
/// right-hand perpendicular, with the emit table's magnitudes (advance 100,
/// attack 30, retreat 80 back, dodge 100 aside, reposition 60 plus 80
/// aside; the side keeps the brain's sign). The brain's strike band was
/// tuned for v1's rangeless hit, so an attack scored beyond the blade's reach
/// closes in at full wish instead and holds the swing until the foe is in
/// reach. Facing and the other buttons pass through. Integer-only (`isqrt`);
/// coincident fighters stand still.
fn brain_wish_to_world(decided: FighterInput, intent: Intent, percept: &Percept) -> FighterInput {
    let me = match_wire::pos_mm(percept.self_pos);
    let foe = match_wire::pos_mm(percept.foe_pos);
    let (dx, dz) = (
        i64::from(foe[0]) - i64::from(me[0]),
        i64::from(foe[2]) - i64::from(me[2]),
    );
    let len = i64::try_from(u64::try_from(dx * dx + dz * dz).unwrap_or(0).isqrt()).unwrap_or(0);
    let out_of_reach = len > i64::from(match_wire::BLADE_REACH_MM);
    let side = if decided.mz < 0 { -1 } else { 1 };
    let (forward, aside): (i64, i64) = match intent {
        Intent::Attack if out_of_reach => (100, 0),
        Intent::Attack => (30, 0),
        Intent::Block => (0, 0),
        Intent::Dodge => (0, 100 * side),
        Intent::Advance => (100, 0),
        Intent::Retreat => (-80, 0),
        Intent::Reposition => (60, 80 * side),
    };
    let buttons = if intent == Intent::Attack && out_of_reach {
        engine_input::Buttons(decided.buttons.0 & !ATTACK)
    } else {
        decided.buttons
    };
    if len == 0 {
        return FighterInput {
            mx: 0,
            mz: 0,
            buttons,
            ..decided
        };
    }
    let axis = |v: i64| i8::try_from((v / len).clamp(-100, 100)).unwrap_or(0);
    FighterInput {
        mx: axis(forward * dx + aside * dz),
        mz: axis(forward * dz - aside * dx),
        buttons,
        ..decided
    }
}
/// Unit facing `(sin, cos)` of a centidegree heading in Q16 (65536 is 1.0):
/// never the zero vector, integer-only, identical on every platform.
fn facing_q16(centideg: i16) -> (i32, i32) {
    let angle = Fixed::from_bits(i64::from(centideg) * Fixed::PI.to_bits() / 18_000);
    let (sin, cos) = angle.sin_cos();
    let clamp = |v: Fixed| i32::try_from(v.to_bits()).unwrap_or(0);
    (clamp(sin), clamp(cos))
}

/// EG34 balance impulse of a landed hit, Q8 (stumbles at default tuning).
const BALANCE_HIT_Q8: u16 = 384;
/// EG34 balance impulse of a fall: a knockdown, or a hit inside an open
/// stagger window (falls at default tuning).
const BALANCE_FALL_Q8: u16 = 640;
/// EG34 balance impulse a blocked blow leaves (under the stagger line).
const BALANCE_BLOCK_Q8: u16 = 128;
/// EG34 balance impulse on a parried attacker (staggers at default tuning).
const BALANCE_PARRY_Q8: u16 = 256;
/// EG34 body mass the balance centre observes, kg (one root body).
const BALANCE_BODY_MASS_KG: f32 = 80.0;

/// Team id `slot` for every slot: nobody shares a team.
const fn solo_teams() -> [u8; MAX_SLOTS] {
    let mut teams = [0u8; MAX_SLOTS];
    let (mut slot, mut id) = (0usize, 0u8);
    while slot < MAX_SLOTS {
        teams[slot] = id;
        slot += 1;
        id = id.wrapping_add(1);
    }
    teams
}

impl ArenaConfig {
    /// Build plus validate in one step; same checks as [`validate`](Self::validate).
    ///
    /// Consumer: any headless entry point (`run`, forks, prefix/resume).
    /// Data owner: the caller owns the returned config (pure value, `Copy`).
    /// Units: ticks are fixed steps at [`TICK_HZ`], extent is meters.
    /// Threading: `Send + Sync`, no shared state. Determinism scope: the
    /// stored seed expands identically on every machine via [`seed::expand`].
    /// Errors: the typed culprit field, identical to `validate`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use engine_arena::{ArenaConfig, ArenaError};
    ///
    /// let config = ArenaConfig::try_new(7, 60, 2, 20.0)?;
    /// assert_eq!(config.ticks, 60);
    /// assert_eq!(
    ///     ArenaConfig::try_new(7, 0, 2, 20.0),
    ///     Err(ArenaError::BadTicks(0))
    /// );
    /// # Ok::<(), ArenaError>(())
    /// ```
    pub fn try_new(
        seed: u64,
        ticks: u32,
        fighters: u8,
        half_extent_m: f32,
    ) -> Result<Self, ArenaError> {
        let config = Self {
            seed,
            ticks,
            fighters,
            half_extent_m,
            table_version: TABLES_VERSION_V1,
        };
        config.validate()?;
        Ok(config)
    }

    /// Range-check every field; NaN or out-of-range fails.
    pub fn validate(self) -> Result<(), ArenaError> {
        if self.ticks == 0 || self.ticks > MAX_TICKS {
            return Err(ArenaError::BadTicks(self.ticks));
        }
        if self.fighters < 2 || self.fighters > MAX_PARTICIPANTS {
            return Err(ArenaError::BadFighters(self.fighters));
        }
        if !self.half_extent_m.is_finite() || self.half_extent_m < 5.0 || self.half_extent_m > 100.0
        {
            return Err(ArenaError::BadExtent);
        }
        if self.table_version != TABLES_VERSION_V1 && self.table_version != TABLES_VERSION_V2 {
            return Err(ArenaError::BadTableVersion(self.table_version));
        }
        Ok(())
    }

    /// Fight rules this config selects: table version 2 only exists under
    /// rules v2 (the v1 step never reads the tables), so v2 tables pick
    /// [`ArenaRules::v2_free_for_all`] and every other version keeps the
    /// frozen [`ArenaRules::V1`]. Every sim built from the config (the
    /// live match, the LAN authority's prefix replays) agrees on it.
    #[must_use]
    pub const fn rules(&self) -> ArenaRules {
        if self.table_version == TABLES_VERSION_V2 {
            ArenaRules::v2_free_for_all()
        } else {
            ArenaRules::V1
        }
    }
}

/// One fighter snapshot: kinematic capsule pose in meters/radians.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FighterState {
    /// Which fighter this snapshot describes.
    pub id: FighterId,
    /// Feet position `[x, y, z]` in meters.
    pub pos: [f32; 3],
    /// Facing yaw in radians.
    pub yaw: f32,
}

/// Post-step checkpoint: tick plus the match state hash.
///
/// Small per-tick handle (`Copy`, tick plus hash) for the rollback pattern:
/// the [`MatchSnapshot`] owner holds the full restore state while handles
/// name where to resume, so an 8-tick rewind restores the snapshot then
/// replays instead of rebuilding from tick zero. No new snapshot format:
/// see [`run_prefix`] plus [`resume`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Checkpoint {
    /// Zero-based fixed tick.
    pub tick: u32,
    /// State hash after the tick.
    pub hash: u64,
}

/// Combat events emitted on one fixed tick, in fighter-ID order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TickEvents {
    /// Zero-based fixed tick these events belong to.
    pub tick: u32,
    /// Combat outcomes for the tick, possibly empty.
    pub events: Vec<FightEvent>,
}

/// Destruction break events emitted on one fixed tick, in collection
/// emission order (snapped bonds in index order, then freed chunks).
#[derive(Clone, PartialEq, Debug)]
pub struct TickBreaks {
    /// Zero-based fixed tick these breaks belong to.
    pub tick: u32,
    /// Bond snaps plus freed chunks for the tick, possibly empty.
    pub events: Vec<BreakEvent>,
}

/// Opaque mid-match state for rollback/resimulation. Captures every
/// future-affecting byte: tick cursor, kinematic poses, latched wishes and
/// buttons, combat state, the observer intake flag plus phased cursors,
/// locomotion machines and per-slot brains (plus, under `sim`, per-fighter
/// controller snapshots and ownership of the live controllers).
///
/// Snapshots are single-use: [`resume`] consumes the snapshot. Under `sim`
/// the controller snapshot/restore pair is same-world exact (PH1 shape) but
/// not fresh-world exact — Rapier's pipeline-internal contact state is not
/// part of the public snapshot — so resuming into rebuilt controllers would
/// diverge on the first step. Moving the live controllers through the
/// snapshot keeps the continuation bit-identical.
///
/// Checkpoint pairing: [`Checkpoint`] handles stay small (tick plus hash)
/// while this snapshot stays the single restore owner for the net rollback
/// consumer; handles name the resume point, never a second format.
pub struct MatchSnapshot {
    config: ArenaConfig,
    tick: u32,
    states: Vec<FighterState>,
    wishes: [[i8; 2]; MAX_SLOTS],
    buttons: [u8; MAX_SLOTS],
    fight: FightState,
    match_state: CombatMatchState,
    match_version: u32,
    phased: [PhasedFighter; MAX_SLOTS],
    brains: Vec<DepthBrain>,
    locos: Vec<Locomotion>,
    destruction: BondedCollection,
    #[cfg(feature = "sim")]
    controller_snaps: Vec<ControllerSnapshot>,
    #[cfg(feature = "sim")]
    controllers: Vec<Controller>,
}

impl MatchSnapshot {
    /// Zero-based fixed tick the snapshot resumes from.
    pub const fn tick(&self) -> u32 {
        self.tick
    }

    /// Match configuration the snapshot belongs to.
    pub const fn config(&self) -> ArenaConfig {
        self.config
    }

    /// Observer intake flag carried with the snapshot (V0 legacy, V1 phased).
    pub const fn match_version(&self) -> u32 {
        self.match_version
    }
}

/// Arena contract failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ArenaError {
    /// Tick count `0` or above `MAX_TICKS`, culprit carried.
    BadTicks(u32),
    /// Fighter count outside `2..=MAX_PARTICIPANTS`, culprit carried.
    BadFighters(u8),
    /// Half-extent NaN or outside `5.0..=100.0` meters.
    BadExtent,
    /// Move-table version outside `1..=2`, culprit carried.
    BadTableVersion(u32),
}

/// Headless match v2 with AR2 integration: fixed-tick kinematic root intent
/// plus physics capsules, combat, brains and anim presentation.
///
/// Validates `config` first, then the tape (sorted, ranged, plus
/// arena-level bounds: every tape tick below `config.ticks` and every
/// fighter slot below `config.fighters`). Any failure returns an empty
/// vector, never panics and never skips one bad input silently. Otherwise
/// simulates `config.ticks` steps at [`TICK_HZ`] and returns one
/// [`Checkpoint`] per tick.
///
/// Consumer: NET1 rollback, playtest forks, the packaged game.
/// Data owner: this crate owns fighter poses for the tick loop.
/// Units: meters, radians, fixed ticks. Determinism scope:
/// same-machine reproducibility (floats enter the hash only as `to_bits`,
/// plus the physics world's own quantized hash).
/// Error type: none returned; invalid input maps to an empty vector.
pub fn run(config: &ArenaConfig, tape: &FighterTape) -> Vec<Checkpoint> {
    run_sim(config, tape, false, false)
        .map(|sim| sim.checkpoints)
        .unwrap_or_default()
}

/// [`run`] with a [`SlotController`] deciding the slots it claims: tape inputs still win, the
/// brains fill the rest, then the controller overwrites its own slots once per tick (see
/// [`controller`]). Same input rules and empty-vector error policy as [`run`]. A controller
/// that claims no slot gives exactly [`run`]'s checkpoints.
pub fn run_controlled(
    config: &ArenaConfig,
    tape: &FighterTape,
    controller: &mut dyn SlotController,
) -> Vec<Checkpoint> {
    let Some(mut sim) = Sim::build(config, tape, false, false) else {
        return Vec::new();
    };
    while sim.tick < config.ticks {
        if sim.step_with(tape, Some(&mut *controller)).is_none() {
            return Vec::new();
        }
    }
    sim.checkpoints
}

/// Per-tick combat events for a valid run; empty on invalid input. The event
/// count and payload bytes feed the checkpoint hash, so a scripted attack
/// tape that lands hits diverges from an idle tape in [`run`] too.
pub fn run_events(config: &ArenaConfig, tape: &FighterTape) -> Vec<TickEvents> {
    run_sim(config, tape, false, false)
        .map(|sim| sim.events)
        .unwrap_or_default()
}

/// Per-tick destruction breaks for a valid run; empty on invalid input.
/// One [`TickBreaks`] per tick, so the length matches [`run_events`]. A
/// sword hit ([`FightEvent::Hit`]) fractures the bonded crate prop on the
/// same tick; the break sets are identical across reruns.
pub fn run_breaks(config: &ArenaConfig, tape: &FighterTape) -> Vec<TickBreaks> {
    run_sim(config, tape, false, false)
        .map(|sim| sim.breaks)
        .unwrap_or_default()
}

/// Presentation root tables for a valid run: one [`RootTable`] per fighter
/// (ticks shared, so tables stay per-slot to keep tick order strict),
/// extracted with [`extract_root`] from the authoritative trajectory.
/// Empty on invalid input. Presentation only: nothing here feeds the hash.
pub fn run_presentation(config: &ArenaConfig, tape: &FighterTape) -> Vec<RootTable> {
    match run_sim(config, tape, true, false) {
        Some(sim) => sim.root_tables(),
        None => Vec::new(),
    }
}

/// Presentation locomotion poses for a valid run: one pose vector per
/// fighter, one [`LocalPose`] per tick on the LOC1 presentation skeleton.
/// These are the per-tick [`LocoOutput`](engine_anim::locomotion::LocoOutput)
/// poses recorded during the run (knocked-out ticks repeat the last pose),
/// each post-processed by its slot's [`GroundWarp`]; the root stays
/// authoritative. Empty on invalid input. Presentation only: nothing here feeds the hash.
pub fn run_loco_poses(config: &ArenaConfig, tape: &FighterTape) -> Vec<Vec<LocalPose>> {
    match run_sim(config, tape, true, false) {
        Some(sim) => sim.loco_history.unwrap_or_default(),
        None => Vec::new(),
    }
}
/// Sample a presentation pose at a fixed tick through [`sample_pose`]:
/// `time` is `tick / TICK_HZ` seconds. The caller supplies the skeleton and
/// clip (authored data lives outside the match); the match never samples for
/// authority. Presentation only: never hashed.
pub fn presentation_pose(
    skeleton: &Skeleton,
    clip: &Clip,
    tick: u32,
) -> Result<LocalPose, AnimError> {
    sample_pose(skeleton, clip, tick as f32 / TICK_HZ as f32)
}

/// Record one [`Dataset`] over a live match: every tick records one row per
/// fighter (pre-step observation, decided input, damage outcome) through
/// [`recorder::on_fork_step`], so script, brain and human-tape teachers
/// share one schema. `None` on invalid config/tape, never partial.
///
/// Consumer: DT2 student trainer; the sibling fork entrypoint is
/// [`Fork::run_recorded`](fork::Fork::run_recorded).
///
/// [`Dataset`]: recorder::Dataset
pub fn run_dataset(config: &ArenaConfig, tape: &FighterTape) -> Option<Dataset> {
    let mut sim = Sim::build(config, tape, false, false)?;
    let mut rows = Vec::with_capacity(config.ticks as usize * sim.fighters);
    // Recorder scratch lives in the shared leg (hoisted pre-step poses plus
    // damage counters), the same per-tick code `LiveMatch` steps once per
    // call, so the steady path allocates nothing after the first tick
    // reserves capacity (row entries already live in fixed stack storage).
    let mut leg = live::DatasetLeg::new(sim.fighters);
    while sim.tick < config.ticks {
        leg.step(&mut sim, tape, &mut rows)?;
    }
    Some(Dataset {
        version: DATASET_VERSION,
        rows,
    })
}

/// Simulate the `0..ticks` prefix of a match and capture the snapshot that
/// resumes it. Under `sim` the snapshot owns the live physics world, so
/// resuming is a same-world continuation and resimulates bit-identical.
/// Returns `None` when `ticks > config.ticks` or the config/tape is invalid.
/// Pair with [`resume`] for rollback/resimulation proofs.
pub fn run_prefix(
    config: &ArenaConfig,
    tape: &FighterTape,
    ticks: u32,
) -> Option<(MatchSnapshot, Vec<Checkpoint>)> {
    run_prefix_versioned(config, tape, ticks, MATCH_WIRE_VERSION_V0)
}

/// Simulate the `0..ticks` prefix behind the intake version flag
/// ([`MATCH_WIRE_VERSION_V0`] legacy, [`MATCH_WIRE_VERSION_V1`] phased) and
/// capture the snapshot that resumes it. The snapshot carries the version
/// plus the phased cursors, so the suffix match rows resimulate exactly like
/// the uninterrupted observer run at the same version. Returns `None` when
/// `ticks > config.ticks`, the config/tape is invalid, or the version is
/// unknown. Pair with [`resume`] for rollback/resimulation proofs.
///
/// Consumer: rollback/resimulation proofs pinning the intake path. Units:
/// fixed ticks at [`TICK_HZ`]. Determinism scope: same config plus tape plus
/// version gives identical prefix rows and snapshot bytes.
pub fn run_prefix_versioned(
    config: &ArenaConfig,
    tape: &FighterTape,
    ticks: u32,
    version: u32,
) -> Option<(MatchSnapshot, Vec<Checkpoint>)> {
    if ticks > config.ticks {
        return None;
    }
    // Prefix snapshots keep the match track live so the suffix match rows
    // resimulate exactly like the uninterrupted observer run.
    let mut sim = Sim::build_versioned(config, tape, false, true, version)?;
    while sim.tick < ticks {
        sim.step(tape)?;
    }
    #[cfg(feature = "sim")]
    let snapshot = {
        let controller_snaps = sim.controllers.iter().map(|c| c.snapshot()).collect();
        MatchSnapshot {
            config: *config,
            tick: sim.tick,
            states: sim.states.clone(),
            wishes: sim.wishes,
            buttons: sim.buttons,
            fight: sim.fight,
            match_state: *sim.match_track.state(),
            match_version: sim.match_track.version(),
            phased: *sim.match_track.phased(),
            brains: sim.brains.clone(),
            locos: sim.locos.clone(),
            destruction: sim.destruction,
            controller_snaps,
            controllers: sim.controllers,
        }
    };
    #[cfg(not(feature = "sim"))]
    let snapshot = MatchSnapshot {
        config: *config,
        tick: sim.tick,
        states: sim.states.clone(),
        wishes: sim.wishes,
        buttons: sim.buttons,
        fight: sim.fight,
        match_state: *sim.match_track.state(),
        match_version: sim.match_track.version(),
        phased: *sim.match_track.phased(),
        brains: sim.brains.clone(),
        locos: sim.locos.clone(),
        destruction: sim.destruction,
    };
    Some((snapshot, sim.checkpoints))
}

/// Resume a [`MatchSnapshot`] with the same config and tape through
/// `config.ticks`, returning the suffix checkpoints. Consumes the snapshot
/// (under `sim` its live world continues in place: same-world restore,
/// bit-identical to the uninterrupted run), so one snapshot resumes once. A
/// config mismatch or invalid config/tape rejects the whole resume as empty,
/// never partial.
pub fn resume(
    snapshot: MatchSnapshot,
    config: &ArenaConfig,
    tape: &FighterTape,
) -> Vec<Checkpoint> {
    if snapshot.config != *config {
        return Vec::new();
    }
    resume_inner(snapshot, config, tape)
        .map(|sim| sim.checkpoints)
        .unwrap_or_default()
}

/// Live match state: everything one fixed tick reads or writes.
///
/// One movement authority per slot: the merged [`FighterInput`] wish drives
/// the slot's [`Locomotion`], whose requested root delta resolves through
/// collision exactly once per tick (one controller under `sim`, one wall
/// clamp otherwise) and advances the authoritative pose by the resolved
/// delta only. Combat hit events feed locomotion interrupts on the same
/// tick, so hit-stop freezes gait phase alongside combat clocks.
///
/// Consumer: headless runners plus the window look (`Sim::balance` read).
/// Data owner: this crate owns fighter poses for the tick loop.
pub struct Sim {
    config: ArenaConfig,
    fighters: usize,
    tick: u32,
    states: Vec<FighterState>,
    wishes: [[i8; 2]; MAX_SLOTS],
    buttons: [u8; MAX_SLOTS],
    fight: FightState,
    match_track: match_wire::MatchTrack,
    brains: Vec<DepthBrain>,
    locos: Vec<Locomotion>,
    /// Per-slot ground-feel warps, one per fighter on presentation runs
    /// (`loco_history` is `Some`) and empty on authority runs. Each recorded
    /// locomotion pose rides its slot warp; the root stays authoritative and
    /// history never feeds the hash, so checkpoints and snapshots never see
    /// the warp.
    warps: Vec<GroundWarp>,
    destruction: BondedCollection,
    /// Static obstacle world this sim resolves every tick (default
    /// [`MatchWorld::empty`], the legacy open arena). A non-empty world
    /// settles the spawns, resolves each slot's root delta after the clamp
    /// and folds its digest into every checkpoint, so two scene revisions
    /// never share a hash. Consumer: the game's live window session.
    world: MatchWorld,
    #[cfg(feature = "sim")]
    controllers: Vec<Controller>,
    cursor: usize,
    checkpoints: Vec<Checkpoint>,
    events: Vec<TickEvents>,
    /// Combat event scratch, reserved once to MAX_EVENTS_PER_TICK.
    /// `try_step_into` clears and reuses its buffer each tick; the tick log
    /// below clones exact-size on event ticks only, so the scratch never
    /// regrows. Quiet ticks move an empty Vec (no heap alloc).
    combat_scratch: Vec<FightEvent>,
    /// Press-tick attack cues under readable-timing rows (EG19b): the v2
    /// combat leg clears and fills this each tick, so it always reads the
    /// last stepped tick; empty under v1 rows and on quiet ticks. Reserved
    /// once to the per-tick cue bound, so steady ticks grow nothing. Rules
    /// pin before the first tick, so the v1 leg only runs on matches whose
    /// buffer never fills and no stale cue can linger. Read through
    /// [`Sim::cues`]; presentation only, never hashed, never logged.
    last_cues: Vec<AttackStarted>,
    breaks: Vec<TickBreaks>,
    match_log: Vec<TickMatchEvents>,
    /// Whether this sim resolves the GP1/GP2 match track each tick. The
    /// hashed/measured authority path (`run`, combat/destruction logs, the
    /// dataset recorder) leaves it off so the per-tick resolver call-site
    /// costs zero heap there; the match observers (`run_match_events`,
    /// `run_match_bytes`, prefix snapshots, forks) turn it on.
    match_live: bool,
    history: Option<Vec<Vec<FighterState>>>,
    loco_history: Option<Vec<Vec<LocalPose>>>,
    /// Merged per-slot inputs from the last stepped tick (tape where
    /// present, brain decisions elsewhere). The DT1 recorder reads these;
    /// authority never does, so the hash is untouched.
    last_inputs: [Option<FighterInput>; MAX_SLOTS],
    /// Fight rules (EG3): [`ArenaRules::V1`] on every legacy runner.
    rules: ArenaRules,
    /// Per-slot integer balance (EG34), stepped only under
    /// [`ArenaRules::balance`]; empty on snapshot and fork sims (v1 only).
    balance: Vec<BalanceState>,
    /// Per-slot stagger memory (EG46), stepped only under
    /// [`ArenaRules::balance`]; empty on snapshot and fork sims (v1 only).
    stagger: Vec<StaggerMemory>,
    /// Combat-depth poise meters stepped beside the fight step: every hit
    /// moves poise through [`DepthSet::observe`] and trips one stagger
    /// through [`PoiseMeter::staggered`]. Presentation only, never hashed,
    /// so V1 bouts replay byte-identical.
    depth: DepthSet,
    /// Severed-limb pool (EG46b), stepped only under
    /// [`ArenaRules::balance`]; empty on snapshot and fork sims (v1 only).
    /// A cut reaches the authority through [`LimbPool::state_hash`].
    limbs: LimbPool,
    /// Spear grapple state, `Some` only for a match built with
    /// `MatchSpec::spear`; `None` leaves every tick and hash untouched.
    spear: Option<spear::SpearState>,
    #[cfg(test)]
    trace: Option<Vec<Vec<TickMotion>>>,
    /// Test-only oracle switch: build the melee plan through the pre-hoist
    /// pairwise path (see `engage::Plan::build_pairwise`), so a test can run
    /// a whole match through the oracle and compare events, poses and hashes.
    #[cfg(test)]
    pairwise_plan: bool,
    /// Test-only oracle switch: resolve the EG20 collision pass through the
    /// pre-shortcut fixed pass count (see `collide::resolve_fixed_passes`),
    /// so a test can run a whole match through the oracle and compare
    /// events, poses and hashes.
    #[cfg(test)]
    full_collide_passes: bool,
}

impl Sim {
    /// Validated constructor: config, tape order/ranges and arena-level
    /// bounds, then seed-expanded spawns, a fresh fight, one locomotion
    /// machine plus one brain per slot (plus, under `sim`, one settled
    /// controller per fighter with floor plus walls). `match_live` arms the
    /// per-tick GP1/GP2 resolver call-site; the hashed path passes `false`
    /// so steady ticks cost zero heap there.
    fn build(
        config: &ArenaConfig,
        tape: &FighterTape,
        record: bool,
        match_live: bool,
    ) -> Option<Self> {
        Self::build_world(config, tape, record, match_live, MatchWorld::empty())
    }

    /// [`Sim::build`] over a static obstacle world: the kit walls a match
    /// collides with. An empty world is the legacy open arena, byte for byte
    /// (no settle, no resolve, no digest fold), so every existing caller
    /// stays frozen; a non-empty world settles each spawn out of the boxes
    /// and folds its digest into every checkpoint, exactly as the spine step
    /// does. Consumer: the game's live window session (`engine-game`).
    fn build_world(
        config: &ArenaConfig,
        tape: &FighterTape,
        record: bool,
        match_live: bool,
        world: MatchWorld,
    ) -> Option<Self> {
        if config.validate().is_err() {
            return None;
        }
        if tape.validate().is_err() {
            return None;
        }
        let fighters = usize::from(config.fighters);
        for input in &tape.0 {
            if input.tick >= config.ticks || usize::from(input.fighter.0) >= fighters {
                return None;
            }
        }
        let mut states = spawn_states(config.seed, config.fighters, config.half_extent_m);
        // A supplied world places every spawn out of its boxes first, so a
        // seed that lands on a kit wall starts free (mirrors `SpineMatch`).
        if !world.is_empty() {
            let limit = wall_limit(config.half_extent_m);
            for state in states.iter_mut() {
                let placed = world.settle([state.pos[0], state.pos[2]], FIGHTER_RADIUS_M, limit)?;
                state.pos[0] = placed[0];
                state.pos[2] = placed[1];
            }
        }
        let fight = FightState::new(config.fighters).ok()?;
        let match_track = match_wire::MatchTrack::new(config.fighters)?;
        let brains = (0..fighters)
            .map(|slot| {
                let mut brain = DepthBrain::new(FighterId(slot as u8), AI_PERIOD_TICKS);
                brain.set_half_extent_m(config.half_extent_m);
                brain
            })
            .collect();
        let locos = (0..fighters).map(|_| Locomotion::new()).collect();
        // DX1: one bonded crate prop per match, seeded off the spawn stream
        // so the same config wires the same crate; a bad bake rejects the
        // run before the first tick, never partially.
        let destruction = BondedCollection::crate_prop(
            CRATE_CENTER_M[0],
            CRATE_CENTER_M[1],
            CRATE_CENTER_M[2],
            CRATE_HALF_M,
            CRATE_PIECES,
            crate_seed(config),
        )
        .ok()?;
        #[cfg(feature = "sim")]
        let controllers = build_controllers(config, &states)?;
        let ticks = config.ticks as usize;
        let history = if record {
            Some(Vec::with_capacity(ticks))
        } else {
            None
        };
        let loco_history = if record {
            Some((0..fighters).map(|_| Vec::with_capacity(ticks)).collect())
        } else {
            None
        };
        let warps = if record {
            (0..fighters).map(|_| GroundWarp::new()).collect()
        } else {
            Vec::new()
        };
        // Depth tables follow the match table version: V1 keeps poise
        // damage 0 and never staggers, V2 uses the shipped poise rows.
        let depth_tables = if config.table_version == TABLES_VERSION_V2 {
            TABLES_V2
        } else {
            TABLES_V1
        };
        let depth = DepthSet::new(depth_tables, config.fighters);
        // Heap gate: presize the limb pool plus the cue buffer to the active
        // fighter count. At most one cue per fighter per tick, so `fighters`
        // never reallocates on the steady path; the V1 authority never cuts,
        // so a fighter-scaled limb pool bills `fighters` bodies instead of
        // `MAX_LIMBS` (17408 B) while V2 cuts for this count still fit.
        let cue_cap = fighters.min(MAX_ATTACK_STARTED_PER_TICK);
        let limb_cap = fighters.clamp(1, MAX_LIMBS);
        let mut sim = Self {
            config: *config,
            fighters,
            tick: 0,
            states,
            wishes: [[0, 0]; MAX_SLOTS],
            buttons: [0; MAX_SLOTS],
            fight,
            match_track,
            brains,
            locos,
            warps,
            destruction,
            world,
            #[cfg(feature = "sim")]
            controllers,
            cursor: 0,
            checkpoints: Vec::with_capacity(ticks),
            events: Vec::with_capacity(ticks),
            combat_scratch: Vec::with_capacity(MAX_EVENTS_PER_TICK),
            last_cues: Vec::with_capacity(cue_cap),
            breaks: Vec::with_capacity(ticks),
            // Cold authority sims keep no match rows at all: even the outer
            // reservation would bill 600 rows of `Vec` pairs into the
            // allocation budget, so only live observers reserve it.
            match_log: if match_live {
                Vec::with_capacity(ticks)
            } else {
                Vec::new()
            },
            match_live,
            history,
            loco_history,
            last_inputs: [None; MAX_SLOTS],
            rules: ArenaRules::V1,
            balance: (0..fighters)
                .map(|_| BalanceState::new(BalanceParams::default()))
                .collect(),
            stagger: (0..fighters).map(|_| StaggerMemory::new()).collect(),
            depth,
            limbs: LimbPool::new(limb_cap).ok()?,
            spear: None,
            #[cfg(test)]
            trace: None,
            #[cfg(test)]
            pairwise_plan: false,
            #[cfg(test)]
            full_collide_passes: false,
        };
        // Table version 2 runs under rules v2; version 1 pins the legacy rules.
        sim.set_rules(config.rules()).then_some(sim)
    }

    /// Pin the fight rules before the first tick. An unknown version or a
    /// sim already past tick 0 refuses (`false`) and keeps v1.
    fn set_rules(&mut self, rules: ArenaRules) -> bool {
        if !rules.is_known() || self.tick != 0 {
            return false;
        }
        self.rules = rules;
        if rules.version == FIGHT_RULES_V2 {
            // Identical brains close in lockstep under the symmetric v2 hit
            // rule: each slot gets a temperament drawn from the match seed.
            for (slot, brain) in self.brains.iter_mut().enumerate() {
                let id = u8::try_from(slot).unwrap_or(u8::MAX);
                brain.set_personality(Personality::seeded(self.config.seed, id));
            }
        }
        true
    }

    /// Arm the spear grapple (`docs/plan/spear-grapple.md`) before the first
    /// tick. Refuses (`false`) past tick 0 and on the Rapier `sim` leg, whose
    /// controllers own the poses the tether would have to move.
    fn set_spear(&mut self) -> bool {
        if cfg!(feature = "sim") || self.tick != 0 {
            return false;
        }
        self.spear = Some(spear::SpearState::new(self.fighters));
        true
    }

    /// Validated constructor behind the intake version flag: V0 keeps the
    /// legacy observer path, V1 arms the phased rows. Unknown versions
    /// reject the whole build as `None`, never partial.
    fn build_versioned(
        config: &ArenaConfig,
        tape: &FighterTape,
        record: bool,
        match_live: bool,
        version: u32,
    ) -> Option<Self> {
        let mut sim = Self::build(config, tape, record, match_live)?;
        if !sim.set_match_version(version) {
            return None;
        }
        Some(sim)
    }

    /// Pin the observer track behind the intake flag. Only V0/V1 land;
    /// anything else returns `false` and keeps the old path, never panics.
    /// The flag plus the phased cursors travel with the snapshot, so a
    /// resume continues on the path its prefix was built under.
    fn set_match_version(&mut self, version: u32) -> bool {
        if !self.match_track.set_version(version) {
            return false;
        }
        debug_assert_eq!(self.match_track.version(), version);
        true
    }

    /// One fixed tick: tape inputs cover their slots, brains fill the rest;
    /// combat resolves strictly, then each live slot's wish drives its
    /// locomotion machine (combat hit events feed interrupts on the same
    /// tick) and the requested root delta resolves through collision exactly
    /// once before the checkpoint hash folds it all (including locomotion
    /// state, the `sim` tag byte and, under `sim`, the controller hashes).
    fn step(&mut self, tape: &FighterTape) -> Option<()> {
        self.step_with(tape, None)
    }

    /// [`step`](Self::step) with an optional [`SlotController`]: it decides the slots it claims
    /// after the tape and the brains filled the row (a tape input still wins).
    fn step_with(
        &mut self,
        tape: &FighterTape,
        controller: Option<&mut dyn SlotController>,
    ) -> Option<()> {
        // Tape is (tick, fighter)-sorted, so a cursor consumes each tick's slice.
        let mut slots: [Option<FighterInput>; MAX_SLOTS] = [None; MAX_SLOTS];
        let mut from_tape = [false; MAX_SLOTS];
        while self.cursor < tape.0.len() && tape.0[self.cursor].tick == self.tick {
            let input = tape.0[self.cursor];
            let slot = usize::from(input.fighter.0);
            if slot < self.fighters {
                slots[slot] = Some(input);
                from_tape[slot] = true;
            }
            self.cursor += 1;
        }
        // AI fills the slots without tape inputs, one fresh percept each.
        // Rules v2 plans the tick's fights first: attack tokens and views.
        let plan = (self.rules.version == FIGHT_RULES_V2).then(|| {
            #[cfg(test)]
            if self.pairwise_plan {
                return engage::Plan::build_pairwise(
                    &self.states,
                    &self.fight,
                    &self.rules.teams,
                    self.fighters,
                );
            }
            engage::Plan::build(&self.states, &self.fight, &self.rules.teams, self.fighters)
        });
        for (slot, cell) in slots.iter_mut().enumerate().take(self.fighters) {
            if cell.is_none() {
                let mut percept = self.percept(slot);
                if let Some(plan) = &plan {
                    percept = plan.aim(percept, slot, &self.states, &self.fight);
                }
                let decided = self.brains[slot].decide(&percept);
                // Rules v2: the brain's intent wish is facing-relative
                // (forward, side); v1 keeps the frozen world-axis reading.
                *cell = Some(if self.rules.version == FIGHT_RULES_V2 {
                    brain_wish_to_world(decided, self.brains[slot].intent(), &percept)
                } else {
                    decided
                });
            }
        }
        // Fixed-size input row on the stack: the old
        // `Vec::with_capacity(self.fighters)` heap-allocated once per tick.
        let mut row = [FighterInput {
            tick: self.tick,
            fighter: FighterId(0),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: engine_input::Buttons::NONE,
        }; MAX_SLOTS];
        for (slot, cell) in slots.iter().enumerate().take(self.fighters) {
            row[slot] = (*cell)?;
        }
        if let Some(decider) = controller {
            let mut views = [controller::SlotView::default(); MAX_SLOTS];
            for (slot, view) in views.iter_mut().enumerate().take(self.fighters) {
                let facing = self.last_inputs[slot].map_or(0, |input| input.facing);
                *view = controller::slot_view(
                    self.states[slot].pos,
                    &self.fight.fighters[slot],
                    facing,
                );
            }
            let view = ControlView {
                tick: self.tick,
                seed: self.config.seed,
                slots: &views[..self.fighters],
            };
            controller::drive(decider, &view, &mut row[..self.fighters], &from_tape);
            // The motion loop below reads `slots`, so the decided inputs go back into it.
            for (cell, input) in slots.iter_mut().zip(&row[..self.fighters]) {
                *cell = Some(*input);
            }
            // Script effects land on fight state before combat resolves; a
            // controller with none drains an empty vec, the legacy path exactly.
            let effects = decider.take_effects();
            controller::apply_script_effects(&mut self.fight, &effects);
        }
        let inputs = &row[..self.fighters];
        // Latch the merged inputs for the DT1 recorder (tape plus brain
        // decisions); authority reads `inputs` below, never this copy.
        for (slot, input) in inputs.iter().enumerate() {
            self.last_inputs[slot] = Some(*input);
        }
        // Strict combat: entry validation makes failure unreachable, but a
        // construction bug rejects the run instead of skipping inputs.
        // Step into the reserved scratch (no growth allocs), then clone
        // exact-size into the owned tick log and clear; the step clears the
        // scratch first each tick, so the clone keeps every byte and the
        // scratch keeps its reservation. Quiet ticks move an empty Vec.
        if self.rules.version == FIGHT_RULES_V2 {
            // Rules v2: pre-move poses in integer mm, fixed-point facing
            // from the absolute IN1 facing, one blade per slot along it.
            let mut poses = [strike_pose(0, &self.states[0], &row[0], 0); MAX_SLOTS];
            for (slot, input) in inputs.iter().enumerate() {
                let id = u8::try_from(slot).ok()?;
                poses[slot] = strike_pose(id, &self.states[slot], input, self.rules.teams[slot]);
            }
            // Caller: `Sim::step_with` reads the resolved tables on the
            // strict combat leg; the live duel passes table version 2
            // (`ArenaConfig::rules`). Default v1 keeps every checkpoint
            // byte frozen.
            let tables = if self.config.table_version == TABLES_VERSION_V2 {
                &TABLES_V2
            } else {
                &TABLES_V1
            };
            // Caller: the cues entry shares the v2 core bit for bit (the
            // combat parity test pins the event stream); the game maps each
            // cue into an audio `FightCue` next slice (EG19c reads them for
            // the swing start). V1 rows cue nothing, so old tapes replay.
            try_step_v2_with_cues_into(
                inputs,
                &poses[..self.fighters],
                &mut self.fight,
                tables,
                &mut self.combat_scratch,
                &mut self.last_cues,
            )
            .ok()?;
        } else {
            try_step_into(inputs, &mut self.fight, &mut self.combat_scratch).ok()?;
        }
        let tick_events = if self.combat_scratch.is_empty() {
            Vec::new()
        } else {
            let logged = self.combat_scratch.clone();
            self.combat_scratch.clear();
            logged
        };
        if self.rules.balance {
            self.feed_balance(&tick_events)?;
        }
        // Combat-depth poise beside the fight step: route this tick's fight
        // events into victim poise via `DepthSet::observe`, then advance the
        // meters via `DepthSet::tick`. Presentation only, never hashed, so V1
        // bouts replay byte-identical. V2 rows observe hits and chips; V1
        // keeps poise damage 0 by skipping observe and never staggers.
        // Caller: `Sim::step_with` (the fight-tick owner).
        if self.rules.balance {
            self.depth.observe(&tick_events);
        }
        self.depth.tick();
        // Production call-site over the landed resolvers: the match track
        // resolves beside the fight path only while `match_live` holds.
        // The track honors its version flag (V0 legacy intakes, V1 phased
        // gates); authority stays on `try_step_into` until the golden migrates, so
        // these bytes join the owned tick log only and never the checkpoint
        // hash. All scratch lives in fixed stack/owned buffers; the owned
        // tick rows clone exact-size on event ticks only, so quiet ticks
        // move nothing and cold (`match_live == false`) ticks skip the
        // whole call-site: zero heap, zero match clocks.
        if self.match_live {
            self.resolve_match_tick(&row);
        }

        // One movement authority per slot: IN1 wish (quantized, fixed-tick,
        // never camera-relative) into LOC1, whose requested root delta
        // resolves through PH2 collision exactly once. Knocked-out fighters
        // hold pose (wishes still latch for the hash; locomotion frozen).
        #[cfg(test)]
        let mut tick_trace = Vec::with_capacity(self.fighters);
        for (slot, cell) in slots.iter().enumerate().take(self.fighters) {
            let input = (*cell)?;
            self.wishes[slot] = [input.mx, input.mz];
            // The spear bit only exists in a spear match; elsewhere it is
            // masked out so a stray press cannot reach the hash.
            self.buttons[slot] = if self.spear.is_some() {
                input.buttons.0
            } else {
                input.buttons.0 & !THROW_SPEAR
            };
            if self.fight.fighters[slot].hp_permille == 0 {
                #[cfg(test)]
                tick_trace.push(TickMotion {
                    requested: [0.0, 0.0],
                    resolved: [0.0, 0.0],
                    blocked: false,
                });
                if let Some(history) = self.loco_history.as_mut() {
                    if let Some(last) = history[slot].last().cloned() {
                        history[slot].push(last);
                    }
                }
                continue;
            }
            // Airborne follows the controller's grounded flag from the last
            // tick (`sim` only); the default leg stays grounded on the flat
            // floor. Facing stays the absolute IN1 intent so AI `face_toward`
            // keeps working; locomotion turns its own gait facing toward the
            // wish and reports the yaw it applied.
            #[cfg(feature = "sim")]
            self.locos[slot].set_airborne(!self.controllers[slot].grounded());
            let (speed, dir) = loco_wish(&input);
            // EG34: a fallen or catching fighter is down: its wish stops and
            // its root holds (the locomotion machine still eases to idle).
            let down = self.rules.balance
                && self.balance.get(slot).is_some_and(|state| {
                    matches!(state.gait(), GaitPhase::Fallen | GaitPhase::Catching)
                });
            let speed = if down { 0.0 } else { speed };
            let interrupt = interrupt_for_slot(&tick_events, slot as u8);
            // Authority ticks that record nothing take the root-only
            // advance (0 heap allocs); presentation runs keep `update` so
            // pose/contacts still land in history.
            let requested = if self.loco_history.is_none() {
                let root = self.locos[slot]
                    .update_root(speed, dir, self.tick, interrupt)
                    .ok()?;
                root.dp
            } else {
                let mut out = self.locos[slot]
                    .update(speed, dir, self.tick, interrupt)
                    .ok()?;
                // Ground feel rides the recorded pose only: the root delta
                // below is authoritative and passes through bit-identical.
                let dp = out.root.dp;
                let capsule = (dp[0] * dp[0] + dp[1] * dp[1]).sqrt() / TICK_DT;
                self.warps[slot]
                    .warp_output(&mut out, capsule, dir, &LOCO_LEG_JOINTS)
                    .ok()?;
                if let Some(history) = self.loco_history.as_mut() {
                    history[slot].push(out.pose.clone());
                }
                out.root.dp
            };
            let requested = if down { [0.0, 0.0] } else { requested };
            self.states[slot].yaw = yaw_from_centideg(input.facing);
            // Wall-inset bound shared by both legs: the default clamp below
            // and the controller walls (faces one skin inside this limit).
            let limit = wall_limit(self.config.half_extent_m);
            #[cfg(feature = "sim")]
            {
                // Single resolution: the blocked remainder dies here and is
                // never reapplied on a later tick.
                let resolved = self.controllers[slot].move_and_slide(requested, FIXED_DT);
                let (dx, dz) = (resolved.displacement[0], resolved.displacement[1]);
                let state = &mut self.states[slot];
                // Backstop clamp: controller walls sit one skin inside this
                // limit, so it never fires; it keeps the shared wall inset
                // live on both legs.
                state.pos[0] = canon_zero(state.pos[0] + dx).clamp(-limit, limit);
                state.pos[2] = canon_zero(state.pos[2] + dz).clamp(-limit, limit);
                #[cfg(test)]
                tick_trace.push(TickMotion {
                    requested,
                    resolved: [dx, dz],
                    blocked: resolved.wall_normal.is_some(),
                });
            }
            #[cfg(not(feature = "sim"))]
            {
                // Single resolution without physics: one clamp, then the
                // static world push-out when a kit world is supplied (the
                // spine step's rule). The blocked remainder is never
                // reapplied on a later tick.
                let state = &mut self.states[slot];
                let from = [state.pos[0], state.pos[2]];
                let raw = [state.pos[0] + requested[0], state.pos[2] + requested[1]];
                let to = [raw[0].clamp(-limit, limit), raw[1].clamp(-limit, limit)];
                let placed = if self.world.is_empty() {
                    to
                } else {
                    self.world.resolve(from, to, FIGHTER_RADIUS_M, limit)
                };
                #[cfg(test)]
                let clamped = placed != to || to != raw;
                #[cfg(test)]
                let (before_x, before_z) = (state.pos[0], state.pos[2]);
                state.pos[0] = canon_zero(placed[0]);
                state.pos[2] = canon_zero(placed[1]);
                #[cfg(test)]
                tick_trace.push(TickMotion {
                    requested,
                    resolved: [state.pos[0] - before_x, state.pos[2] - before_z],
                    blocked: clamped,
                });
            }
        }
        // Spear grapple: throws, flights, tether physics and the bodies it
        // drags, between the walking and the collision pass. Frozen with
        // everything else while hit-stop holds.
        if self.fight.hitstop == 0 {
            if let Some(mut spear) = self.spear.take() {
                let tables = if self.config.table_version == TABLES_VERSION_V2 {
                    &TABLES_V2
                } else {
                    &TABLES_V1
                };
                spear.step(&mut spear::Env {
                    inputs,
                    states: &mut self.states,
                    fight: &mut self.fight,
                    rules: &self.rules,
                    world: &self.world,
                    tables,
                    half_extent_m: self.config.half_extent_m,
                });
                self.spear = Some(spear);
            }
        }
        // EG20: one collision pass after every slot moved (default leg).
        if self.rules.collides() {
            self.collide();
        }
        // EG34: observe each body's post-move centre, then advance balance.
        // EG46: stagger memory steps beside balance under the same gate.
        // EG46b: the limb pool steps after stagger under the same gate.
        if self.rules.balance {
            for (state, balance) in self.states.iter().zip(self.balance.iter_mut()) {
                balance
                    .observe(&[state.pos], &[BALANCE_BODY_MASS_KG])
                    .ok()?;
                balance.step();
            }
            for memory in self.stagger.iter_mut() {
                memory.step();
            }
            self.limbs.step();
        }
        #[cfg(test)]
        if let Some(trace) = self.trace.as_mut() {
            trace.push(tick_trace);
        }

        // DX1: every sword hit strikes the chunk nearest the victim, one
        // sword unit above the fracture gate; the prop steps each tick and
        // the drained breaks join the tick log plus the hash below.
        for event in &tick_events {
            if let FightEvent::Hit { victim, .. } = *event {
                let slot = usize::from(victim);
                if slot < self.states.len() {
                    let pos = self.states[slot].pos;
                    if let Some(chunk) = nearest_chunk(&self.destruction, pos) {
                        if let Some(mass) = self.destruction.chunk_mass(chunk) {
                            let impulse = mass * self.destruction.break_threshold() + 1.0;
                            let _ = self.destruction.apply_impact(chunk, impulse, self.tick);
                        }
                    }
                }
            }
        }
        self.destruction.step(1.0 / TICK_HZ as f32);
        let tick_breaks = self.destruction.drain_events();
        let destruction_hash = self.destruction.state_hash();

        #[cfg(feature = "sim")]
        let physics_hash = {
            const PRIME: u64 = 0x1000_0000_01b3;
            let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
            for controller in &self.controllers {
                hash ^= controller.state_hash();
                hash = hash.wrapping_mul(PRIME);
            }
            hash
        };
        #[cfg(not(feature = "sim"))]
        let physics_hash = 0u64;

        let hash = hash_state(
            self.tick,
            &self.states[..self.fighters],
            &self.buttons[..self.fighters],
            &self.fight,
            &tick_events,
            physics_hash,
            &self.locos,
            destruction_hash,
            &tick_breaks,
        );
        let hash = self.rules.fold(
            hash,
            self.fighters,
            &self.balance,
            &self.stagger,
            &self.limbs,
        );
        // The spear state folds after the rules and before the world; a match
        // without one leaves the hash untouched.
        let hash = self.spear.as_ref().map_or(hash, |spear| spear.fold(hash));
        // A supplied world folds after the rules: the empty world digests to
        // zero and leaves the golden bytes untouched.
        let hash = fold_world(self.world.digest(), hash);
        self.checkpoints.push(Checkpoint {
            tick: self.tick,
            hash,
        });
        self.events.push(TickEvents {
            tick: self.tick,
            events: tick_events,
        });
        self.breaks.push(TickBreaks {
            tick: self.tick,
            events: tick_breaks,
        });
        if let Some(history) = self.history.as_mut() {
            history.push(self.states.clone());
        }
        if self.match_live {
            self.match_track.advance();
        }
        self.tick += 1;
        Some(())
    }

    /// Match observer call-site for one tick: the track honors its version
    /// flag (V0 legacy intakes, V1 phased gates); authority stays on
    /// `try_step_into`, so these bytes join the owned tick log only and
    /// never the checkpoint hash. All scratch lives in fixed stack/owned
    /// buffers; the owned tick rows clone exact-size on event ticks only,
    /// so quiet ticks move nothing.
    fn resolve_match_tick(&mut self, row: &[FighterInput; MAX_SLOTS]) {
        let fighters = self.fighters;
        let mut intents = [ExchangeIntent::Idle; MAX_SLOTS];
        for (slot, input) in row.iter().enumerate().take(fighters) {
            intents[slot] = match_wire::exchange_intent(input);
        }
        let mut poses = [match_wire::sweep_fighter(0, &self.states[0], &row[0]); MAX_SLOTS];
        for (slot, input) in row.iter().enumerate().take(fighters) {
            poses[slot] = match_wire::sweep_fighter(slot as u8, &self.states[slot], input);
        }
        let mut sweeps =
            [match_wire::sweep_for_attacker(0, [0, 0, 0], (0, 1)); MAX_SWEEPS_PER_TICK];
        let mut sweep_n: usize = 0;
        for (slot, input) in row.iter().enumerate().take(fighters) {
            if input.buttons.0 & ATTACK != 0 && sweep_n < MAX_SWEEPS_PER_TICK {
                let facing = match_wire::facing_vector(input.mx, input.mz, input.facing);
                let origin = match_wire::pos_mm(self.states[slot].pos);
                sweeps[sweep_n] = match_wire::sweep_for_attacker(slot as u8, origin, facing);
                sweep_n += 1;
            }
        }
        self.match_track
            .resolve(&intents, &poses[..fighters], &sweeps[..sweep_n]);
        // Locked riposte: a dodge-bit press while this slot holds an open
        // parry window attempts the riposte against the lowest-ID foe
        // through the match state; denials are silently skipped. The call
        // is pure resolver work (no heap); only a landed riposte grows the
        // tick log below, never a quiet tick.
        let mut landed = [ExchangeEvent::Parried {
            attacker: 0,
            victim: 0,
        }; MAX_SLOTS];
        let mut landed_n: usize = 0;
        for (slot, input) in row.iter().enumerate().take(fighters) {
            if input.buttons.0 & DODGE != 0 {
                let victim = i32::from(slot == 0) as u8;
                if let Ok(event) = self.match_track.riposte(slot as u8, victim) {
                    landed[landed_n] = event;
                    landed_n += 1;
                }
            }
        }
        // Single-alloc staging for the owned tick row: the track
        // slices plus landed ripostes size both rows exactly, so an
        // event tick allocates once per row (no regrow past the old
        // exact-size `to_vec` buffer) and quiet ticks move nothing.
        // The track keeps its buffers for the next tick.
        let track_exchange = self.match_track.exchange_events();
        let track_sweep = self.match_track.sweep_events();
        let mut exchange = Vec::with_capacity(track_exchange.len() + landed_n);
        exchange.extend_from_slice(track_exchange);
        exchange.extend_from_slice(&landed[..landed_n]);
        let mut sweep = Vec::with_capacity(track_sweep.len());
        sweep.extend_from_slice(track_sweep);
        self.match_log.push(TickMatchEvents {
            tick: self.tick,
            exchange,
            sweep,
        });
    }

    /// EG34: feed one tick's combat outcomes into the integer balance, in
    /// event order: a hit stumbles the victim (a fall when it lands inside
    /// an open stagger window, so a combo floors), a block chips the stance,
    /// a parry staggers the attacker and a knockdown falls the victim.
    /// EG46: each landed blow also feeds that slot's [`StaggerMemory`] on
    /// bone 0 through [`BalanceState::apply_memory_hit`]. `None` only on an
    /// out-of-range impulse (unreachable: the table stays inside the
    /// ceiling).
    fn feed_balance(&mut self, events: &[FightEvent]) -> Option<()> {
        let Self {
            balance, stagger, ..
        } = self;
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

    /// EG20 collision pass over this sim's fighters; see [`collide_states`].
    /// The test-only oracle switch routes the same rows through the
    /// pre-shortcut fixed pass count; the shipped path never sets it.
    fn collide(&mut self) {
        let (states, rules, half, fighters) = (
            &mut self.states,
            &self.rules,
            self.config.half_extent_m,
            self.fighters,
        );
        #[cfg(test)]
        if self.full_collide_passes {
            collide::collide_states_fixed_passes(states, rules, half, fighters);
            return;
        }
        collide::collide_states(states, rules, half, fighters);
    }

    /// Merged per-slot inputs from the last stepped tick. `None` slots never
    /// occur after a successful [`step`](Sim::step): every slot holds tape or
    /// brain input by then. The DT1 recorder reads this; authority never
    /// does.
    fn last_inputs(&self) -> [Option<FighterInput>; MAX_SLOTS] {
        self.last_inputs
    }

    /// Read-only integer balance verdict for `slot`: the slot entry or `None`
    /// for a bad slot. No copy, no lock, no new field. Presentation only; the
    /// digest path never reads this, so the hash cannot move.
    ///
    /// Consumer: the window fighter look (`engine-game` `fighter_look`).
    /// Data owner: this crate owns the per-slot states. Units: slot index.
    /// Determinism scope: same-machine read of hashed state, never hashed here.
    #[must_use]
    pub fn balance(&self, slot: u8) -> Option<&BalanceState> {
        self.balance.get(usize::from(slot))
    }

    /// Read-only poise meter for `slot`: the [`PoiseMeter`] entry or `None`
    /// for a bad slot. No copy, no lock. Presentation only; the digest path
    /// never reads this, so the hash cannot move. V1 meters stay full (never
    /// stagger); V2 meters drain on hits and chips via `DepthSet::observe`.
    ///
    /// Consumer: the window presentation reads the victim `staggered` trip
    /// from the meter. Data owner: this crate owns the
    /// per-slot [`DepthSet`]. Units: slot index. Determinism scope:
    /// same-machine read of tick output, never hashed here.
    #[must_use]
    pub fn poise(&self, slot: u8) -> Option<&PoiseMeter> {
        self.depth.poise(slot)
    }

    /// Read-only press-tick attack cues from the last stepped tick (EG19b):
    /// one `AttackStarted` per fresh attack start in fighter-ID order under
    /// tables v2; empty under v1 rows, on quiet ticks and past the press
    /// tick. No copy, no lock. Presentation only; the digest path never
    /// reads this, so the hash cannot move.
    ///
    /// Consumer: the window game leg maps each cue into an audio `FightCue`
    /// (EG19c starts the swing clip from them). Data owner: this crate owns
    /// the per-tick cue buffer. Units: fighter slots in press order.
    /// Determinism scope: same-machine read of tick output, never hashed.
    #[must_use]
    pub fn cues(&self) -> &[AttackStarted] {
        &self.last_cues
    }

    /// Map one fighter snapshot into its [`Percept`]: own pose, the nearest
    /// foe's pose plus attack flag, planar separation in meters, tick.
    /// Nearest wins with strict `<`, so ties keep the lowest slot.
    fn percept(&self, slot: usize) -> Percept {
        let me = self.states[slot];
        let mut foe = (slot + 1) % self.fighters;
        let mut best = f32::INFINITY;
        for (index, other) in self.states.iter().enumerate().take(self.fighters) {
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
        let foe_state = self.states[foe];
        let dist_m = if best.is_finite() { best.sqrt() } else { 0.0 };
        Percept::new(
            me.pos,
            me.yaw,
            foe_state.pos,
            self.fight.fighters[foe].is_attacking,
            dist_m,
            self.tick,
        )
    }

    /// Per-fighter root tables from the recorded trajectory. Empty when the
    /// sim did not record, or when extraction rejects (unreachable: poses
    /// are finite by construction, so this maps to empty like any invalid).
    fn root_tables(&self) -> Vec<RootTable> {
        let Some(history) = &self.history else {
            return Vec::new();
        };
        let mut tables = Vec::with_capacity(self.fighters);
        for slot in 0..self.fighters {
            let pos: Vec<[f32; 2]> = history
                .iter()
                .map(|states| [states[slot].pos[0], states[slot].pos[2]])
                .collect();
            let yaw: Vec<f32> = history.iter().map(|states| states[slot].yaw).collect();
            match extract_root(&pos, &yaw, 0) {
                Ok(table) => tables.push(table),
                Err(_) => return Vec::new(),
            }
        }
        tables
    }
}

/// Drive a validated config plus tape to completion; `None` on any failure.
/// `match_live` arms the per-tick resolver call-site (match observers only);
/// the hashed path passes `false`.
fn run_sim(
    config: &ArenaConfig,
    tape: &FighterTape,
    record: bool,
    match_live: bool,
) -> Option<Sim> {
    let mut sim = Sim::build(config, tape, record, match_live)?;
    while sim.tick < config.ticks {
        sim.step(tape)?;
    }
    Some(sim)
}

/// Drive a validated config plus tape to completion behind the intake
/// version flag; `None` on any failure. The phased observers pass V1 here.
fn run_sim_versioned(
    config: &ArenaConfig,
    tape: &FighterTape,
    record: bool,
    match_live: bool,
    version: u32,
) -> Option<Sim> {
    let mut sim = Sim::build_versioned(config, tape, record, match_live, version)?;
    while sim.tick < config.ticks {
        sim.step(tape)?;
    }
    Some(sim)
}

impl Sim {
    /// Rebuild the continuation sim from a snapshot without stepping yet.
    /// Same validation and member order as [`resume_inner`]: config match,
    /// tape bounds, snapshot lengths and version, controller restore under
    /// `sim`. The sync harness steps the returned sim tick by tick to
    /// collect per-system hashes; [`resume_inner`] steps it to completion.
    pub(crate) fn from_snapshot_parts(
        snapshot: MatchSnapshot,
        config: &ArenaConfig,
        tape: &FighterTape,
    ) -> Option<Sim> {
        if config.validate().is_err() {
            return None;
        }
        if tape.validate().is_err() {
            return None;
        }
        let fighters = usize::from(config.fighters);
        for input in &tape.0 {
            if input.tick >= config.ticks || usize::from(input.fighter.0) >= fighters {
                return None;
            }
        }
        if snapshot.tick > config.ticks
            || snapshot.config != *config
            || snapshot.states.len() != fighters
            || snapshot.brains.len() != fighters
            || snapshot.locos.len() != fighters
            || (snapshot.match_version != MATCH_WIRE_VERSION_V0
                && snapshot.match_version != MATCH_WIRE_VERSION_V1)
        {
            return None;
        }
        #[cfg(feature = "sim")]
        if snapshot.controllers.len() != fighters || snapshot.controller_snaps.len() != fighters {
            return None;
        }
        #[cfg(feature = "sim")]
        let controllers = {
            let mut live = snapshot.controllers;
            for (controller, snap) in live.iter_mut().zip(snapshot.controller_snaps.iter()) {
                controller.restore(snap.clone()).ok()?;
            }
            live
        };
        let cursor = tape
            .0
            .iter()
            .position(|input| input.tick >= snapshot.tick)
            .unwrap_or(tape.0.len());
        let ticks = config.ticks as usize;
        let depth_tables = if config.table_version == TABLES_VERSION_V2 {
            TABLES_V2
        } else {
            TABLES_V1
        };
        let depth = DepthSet::new(depth_tables, config.fighters);
        Some(Sim {
            config: *config,
            fighters,
            tick: snapshot.tick,
            states: snapshot.states,
            wishes: snapshot.wishes,
            buttons: snapshot.buttons,
            fight: snapshot.fight,
            match_track: match_wire::MatchTrack::from_state(
                snapshot.match_state,
                snapshot.match_version,
                snapshot.phased,
            ),
            brains: snapshot.brains,
            locos: snapshot.locos,
            warps: Vec::new(),
            destruction: snapshot.destruction,
            // Snapshots come from v1 prefix runners: the empty world is the match.
            world: MatchWorld::empty(),
            #[cfg(feature = "sim")]
            controllers,
            cursor,
            checkpoints: Vec::with_capacity(ticks),
            events: Vec::with_capacity(ticks),
            combat_scratch: Vec::with_capacity(MAX_EVENTS_PER_TICK),
            last_cues: Vec::with_capacity(MAX_ATTACK_STARTED_PER_TICK),
            breaks: Vec::with_capacity(ticks),
            match_log: Vec::with_capacity(ticks),
            // Resumes continue the observed match track so suffix match rows
            // match the uninterrupted observer run tick for tick.
            match_live: true,
            history: None,
            loco_history: None,
            last_inputs: [None; MAX_SLOTS],
            // Snapshots come from the v1 prefix runners only.
            rules: ArenaRules::V1,
            balance: Vec::new(),
            stagger: Vec::new(),
            depth,
            limbs: LimbPool::new(MAX_LIMBS).ok()?,
            spear: None,
            #[cfg(test)]
            trace: None,
            #[cfg(test)]
            pairwise_plan: false,
            #[cfg(test)]
            full_collide_passes: false,
        })
    }
}

/// Continue a snapshot to `config.ticks`. Under `sim` the world moves with
/// the snapshot, so the stored physics snapshot restores into the same
/// world that recorded it (same-world exact) and the continuation is
/// bit-identical to the uninterrupted run. The tape may differ from the
/// prefix tape (corrected inputs); it revalidates fully before one step.
fn resume_inner(snapshot: MatchSnapshot, config: &ArenaConfig, tape: &FighterTape) -> Option<Sim> {
    let mut sim = Sim::from_snapshot_parts(snapshot, config, tape)?;
    while sim.tick < config.ticks {
        sim.step(tape)?;
    }
    Some(sim)
}

/// One settled controller per fighter (`sim` only): floor plus four walls
/// over the arena extent, capsule spawned at its planar spawn. A single
/// zero-wish settle lands every capsule before tick 0 so the first tick
/// starts grounded. Only public [`Controller`] calls; any failure (`None`)
/// rejects the run before the first tick.
#[cfg(feature = "sim")]
fn build_controllers(config: &ArenaConfig, states: &[FighterState]) -> Option<Vec<Controller>> {
    let mut controllers = Vec::with_capacity(states.len());
    for state in states {
        let mut controller = Controller::spawn(
            [state.pos[0], CAPSULE_REST_Y_M, state.pos[2]],
            FIGHTER_RADIUS_M,
            CAPSULE_CYL_H_M,
        )
        .ok()?;
        let half = config.half_extent_m;
        controller
            .add_static_box([half, 0.5, half], [0.0, -0.5, 0.0])
            .ok()?;
        let wall_half_h = 3.0;
        let thick = 0.5;
        let edge = half + thick;
        let extent = half + 2.0 * thick;
        controller
            .add_static_box([thick, wall_half_h, extent], [edge, wall_half_h - 0.5, 0.0])
            .ok()?;
        controller
            .add_static_box(
                [thick, wall_half_h, extent],
                [-edge, wall_half_h - 0.5, 0.0],
            )
            .ok()?;
        controller
            .add_static_box([extent, wall_half_h, thick], [0.0, wall_half_h - 0.5, edge])
            .ok()?;
        controller
            .add_static_box(
                [extent, wall_half_h, thick],
                [0.0, wall_half_h - 0.5, -edge],
            )
            .ok()?;
        controller.move_and_slide([0.0, 0.0], FIXED_DT);
        controllers.push(controller);
    }
    Some(controllers)
}

/// Map one IN1 [`FighterInput`] wish (quantized `-100..=100` percents of top
/// speed, fixed-tick, world-planar AR1 semantics) to a LOC1 wish: planar
/// speed in m/s plus a unit `[x, z]` direction (`+ - * /` and one `sqrt`
/// only). Idle wishes yield zero speed with a safe default direction.
fn loco_wish(input: &FighterInput) -> (f32, [f32; 2]) {
    let (mx, mz) = (input.mx as f32, input.mz as f32);
    let len2 = mx * mx + mz * mz;
    if len2 <= 0.0 {
        return (0.0, [0.0, 1.0]);
    }
    let len = len2.sqrt();
    (len / 100.0 * TOP_SPEED_MPS, [mx / len, mz / len])
}

/// Map CB1 combat events to a LOC1 interrupt for one slot on the same tick:
/// any hit or parry freezes every slot (global hit-stop over the combat
/// freeze), a knockdown freezes every slot harder, and a blocked chip
/// freezes its victim only. Dodges and stamina-outs cause no reaction.
fn interrupt_for_slot(events: &[FightEvent], slot: u8) -> Option<CombatInterrupt> {
    let mut heavy = false;
    let mut light = false;
    for event in events {
        match *event {
            FightEvent::Hit { .. } => light = true,
            FightEvent::Parried { .. } => heavy = true,
            FightEvent::Knockdown { .. } => heavy = true,
            FightEvent::Blocked { victim, .. } if victim == slot => light = true,
            FightEvent::Blocked { .. }
            | FightEvent::Dodged { .. }
            | FightEvent::StaminaOut { .. } => {}
        }
    }
    if heavy {
        Some(CombatInterrupt::Heavy)
    } else if light {
        Some(CombatInterrupt::Light)
    } else {
        None
    }
}

/// Usable half-extent: walls sit at `half_extent_m`, capsule centers clamp
/// one radius inside so no capsule crosses a wall.
fn wall_limit(half_extent_m: f32) -> f32 {
    (half_extent_m - FIGHTER_RADIUS_M).max(0.0)
}

/// Bonded crate prop placement: a half-meter cube sitting on the floor at
/// the arena center. Fixed geometry keeps every match's prop identical;
/// only the bake seed varies with the match seed.
const CRATE_CENTER_M: [f32; 3] = [0.0, 0.5, 0.0];
/// Crate half-extent in meters.
const CRATE_HALF_M: f32 = 0.5;
/// Crate chunk count.
const CRATE_PIECES: usize = 8;

/// The bonded crate prop as it stands before its first step: what a picture of the break
/// needs to draw the crate whole and then as chunks. Read-only; the sim never reads it back.
///
/// Caller: `crates/game/src/break_window.rs` builds the window's break picture from it.
/// Units: meters, Y up. Determinism: the same config gives the same chunks, bonds and anchors
/// as the crate every live match of that config wires (same bake, same seed). Budget: one
/// bake at window start, never per tick.
#[derive(Clone, Debug)]
pub struct CrateRest {
    /// Center of the whole crate, meters.
    pub center_m: [f32; 3],
    /// Half-extent of the whole crate, meters.
    pub half_m: f32,
    /// Chunk boxes in the collection's index order (the `chunk` of a break event indexes it).
    pub chunks: Vec<engine_destruction::bonded::Chunk3>,
    /// Bonds in the collection's order (the `bond` of a break event indexes it).
    pub bonds: Vec<engine_destruction::bonded::Bond>,
    /// Chunks that never fall.
    pub anchors: Vec<u32>,
}

/// The crate prop a match of `config` wires, at rest; `None` when the bake is refused (the
/// same case that rejects the match). Additive and read-only: no sim state changes.
#[must_use]
pub fn crate_rest(config: &ArenaConfig) -> Option<CrateRest> {
    let collection = BondedCollection::crate_prop(
        CRATE_CENTER_M[0],
        CRATE_CENTER_M[1],
        CRATE_CENTER_M[2],
        CRATE_HALF_M,
        CRATE_PIECES,
        crate_seed(config),
    )
    .ok()?;
    Some(CrateRest {
        center_m: CRATE_CENTER_M,
        half_m: CRATE_HALF_M,
        chunks: collection.chunks().to_vec(),
        bonds: collection.bonds().to_vec(),
        anchors: collection.anchors(),
    })
}

/// Crate bake seed: the stream word right after the fighter spawn words, so
/// the same config wires the same crate and neighbouring seeds differ.
fn crate_seed(config: &ArenaConfig) -> u64 {
    seed::expand(config.seed, usize::from(config.fighters) * 2 + 1)
        .last()
        .copied()
        .unwrap_or(config.seed)
}

/// Chunk nearest a world position, squared distance with strict `<` so ties
/// keep the lowest chunk index. `None` only when the prop holds no chunks.
fn nearest_chunk(collection: &BondedCollection, pos: [f32; 3]) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut best_d2 = f32::MAX;
    for (i, chunk) in collection.chunks().iter().enumerate() {
        let dx = chunk.x - pos[0];
        let dy = chunk.y - pos[1];
        let dz = chunk.z - pos[2];
        let d2 = dx * dx + dy * dy + dz * dz;
        if d2 < best_d2 {
            best_d2 = d2;
            best = Some(i);
        }
    }
    best
}

/// Seed-expanded start poses: two stream words per fighter mapped uniformly
/// into `[-limit, limit] x [0, -limit, limit]`; yaw starts at zero.
fn spawn_states(seed: u64, fighters: u8, half_extent_m: f32) -> Vec<FighterState> {
    let limit = wall_limit(half_extent_m);
    let words = seed::expand(seed, usize::from(fighters) * 2);
    (0..fighters)
        .map(|i| {
            let slot = usize::from(i);
            let x = unit_f32(words[slot * 2]) * 2.0 - 1.0;
            let z = unit_f32(words[slot * 2 + 1]) * 2.0 - 1.0;
            FighterState {
                id: FighterId(i),
                pos: [canon_zero(x * limit), 0.0, canon_zero(z * limit)],
                yaw: 0.0,
            }
        })
        .collect()
}

/// Map one stream word uniformly into `[0, 1]` as `f32`.
fn unit_f32(word: u64) -> f32 {
    (word as f64 / u64::MAX as f64) as f32
}

/// Absolute facing in centidegrees (`-18000..=18000`) to yaw radians.
fn yaw_from_centideg(facing: i16) -> f32 {
    canon_zero(facing as f32 * PI / 18_000.0)
}

/// Normalize signed zero so `0.0` and `-0.0` hash identically.
pub(crate) fn canon_zero(value: f32) -> f32 {
    if value == 0.0 {
        0.0
    } else {
        value
    }
}

/// Canonical event bytes: tag plus attacker/victim plus damage/chip payload
/// (zero for payload-free events), in the combat emission order.
pub(crate) fn event_bytes(event: &FightEvent) -> [u8; 5] {
    match *event {
        FightEvent::Hit {
            attacker,
            victim,
            damage,
        } => {
            let payload = damage.to_le_bytes();
            [0, attacker, victim, payload[0], payload[1]]
        }
        FightEvent::Blocked {
            attacker,
            victim,
            chip,
        } => {
            let payload = chip.to_le_bytes();
            [1, attacker, victim, payload[0], payload[1]]
        }
        FightEvent::Parried { attacker, victim } => [2, attacker, victim, 0, 0],
        FightEvent::Dodged { attacker, victim } => [3, attacker, victim, 0, 0],
        FightEvent::StaminaOut { fighter } => [4, fighter, 0, 0, 0],
        FightEvent::Knockdown { attacker, victim } => [5, attacker, victim, 0, 0],
    }
}

/// Fold a [`MatchWorld::digest`] into a checkpoint hash. A zero digest (the
/// empty world) returns `hash` untouched, so the legacy golden stays
/// byte-frozen; a non-empty world mixes its bytes with the same prime the
/// rest of the hash uses (mirrors the spine step, so both steps agree).
fn fold_world(digest: u64, hash: u64) -> u64 {
    if digest == 0 {
        return hash;
    }
    const PRIME: u64 = 0x1000_0000_01b3;
    let mut hash = hash;
    for byte in digest.to_le_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// FNV-1a over canonical bytes: the AR1 set (tick plus per-fighter id,
/// position bits, yaw bits and button byte) extended with per-fighter
/// hp/stamina bits, the tick's combat event bytes, per-fighter locomotion
/// bytes (state, phase, facing, airborne, interrupt), a `sim` tag byte,
/// the destruction prop hash plus the tick's break bytes, and (under `sim`)
/// the controller hashes. The tag byte keeps default and
/// `sim` hashes from ever comparing equal. Floats enter only as `to_bits`
/// (little-endian), never as text; iteration order is fighter-slot order,
/// events stay in combat emission order, breaks in collection order.
#[expect(
    clippy::too_many_arguments,
    reason = "wide signature kept until its inputs move into a params struct"
)]
fn hash_state(
    tick: u32,
    states: &[FighterState],
    buttons: &[u8],
    fight: &FightState,
    events: &[FightEvent],
    physics_hash: u64,
    locos: &[Locomotion],
    destruction: u64,
    breaks: &[BreakEvent],
) -> u64 {
    hash_state_with_sim(
        tick,
        states,
        buttons,
        fight,
        events,
        physics_hash,
        locos,
        destruction,
        breaks,
        cfg!(feature = "sim"),
    )
}

/// [`hash_state`] with an explicit `sim` tag byte; the default path passes
/// `false` and the `sim` path passes `true` (via `cfg!`), so the two
/// configurations never compare equal. Ten parallel slices/flags: the
/// caller always passes the full match row, so bundling would only hide the
/// hash inputs.
#[expect(
    clippy::too_many_arguments,
    reason = "wide signature kept until its inputs move into a params struct"
)]
fn hash_state_with_sim(
    tick: u32,
    states: &[FighterState],
    buttons: &[u8],
    fight: &FightState,
    events: &[FightEvent],
    physics_hash: u64,
    locos: &[Locomotion],
    destruction: u64,
    breaks: &[BreakEvent],
    sim: bool,
) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x1000_0000_01b3;
    let mut hash = OFFSET;
    for byte in tick.to_le_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash ^= u64::from(sim as u8);
    hash = hash.wrapping_mul(PRIME);
    for (state, button) in states.iter().zip(buttons.iter()) {
        hash ^= u64::from(state.id.0);
        hash = hash.wrapping_mul(PRIME);
        for axis in state.pos {
            for byte in canon_zero(axis).to_bits().to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(PRIME);
            }
        }
        for byte in canon_zero(state.yaw).to_bits().to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
        hash ^= u64::from(*button);
        hash = hash.wrapping_mul(PRIME);
    }
    for loco in locos {
        let disc = u8::try_from(state_code(loco.state)).unwrap_or(u8::MAX);
        hash ^= u64::from(disc);
        hash = hash.wrapping_mul(PRIME);
        for axis in [loco.phase(), loco.facing()[0], loco.facing()[1]] {
            for byte in canon_zero(axis).to_bits().to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(PRIME);
            }
        }
        hash ^= u64::from(loco.is_airborne() as u8);
        hash = hash.wrapping_mul(PRIME);
        hash ^= u64::from(loco.interrupt_active() as u8);
        hash = hash.wrapping_mul(PRIME);
    }
    for slot in 0..states.len() {
        if let Some(combat) = fight.fighter(u8::try_from(slot).unwrap_or(u8::MAX)) {
            for byte in combat
                .hp_permille
                .to_le_bytes()
                .into_iter()
                .chain(combat.stamina.to_le_bytes())
            {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(PRIME);
            }
        }
    }
    for byte in (events.len() as u64).to_le_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    for event in events {
        for byte in event_bytes(event) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    for byte in destruction.to_le_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    for byte in (breaks.len() as u64).to_le_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    for brk in breaks {
        for byte in break_bytes(brk) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    for byte in physics_hash.to_le_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Final poses after a valid run; empty on invalid input. Test hook for the
/// wall-clamp invariant, which checkpoints alone cannot observe.
#[cfg(test)]
fn final_states(config: &ArenaConfig, tape: &FighterTape) -> Vec<FighterState> {
    run_sim(config, tape, false, false)
        .map(|sim| sim.states)
        .unwrap_or_default()
}

/// Per-tick collision record for one slot: the locomotion-requested planar
/// delta against the displacement collision actually applied. Test hook for
/// the single-resolution invariant (blocked remainder never reapplied).
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Debug)]
struct TickMotion {
    /// Requested horizontal delta for this tick (x, z), meters.
    requested: [f32; 2],
    /// Resolved horizontal displacement actually applied (x, z), meters.
    resolved: [f32; 2],
    /// True when collision shortened the request (wall face, slope gate or
    /// the default wall clamp).
    blocked: bool,
}

/// Per-tick [`TickMotion`] rows (one inner vector per tick, slots in order)
/// for a valid run; `None` on invalid input. Test hook: proves the requested
/// root delta resolves once and blocked delta never returns.
#[cfg(test)]
fn run_trace(config: &ArenaConfig, tape: &FighterTape) -> Option<Vec<Vec<TickMotion>>> {
    let mut sim = Sim::build(config, tape, false, false)?;
    sim.trace = Some(Vec::with_capacity(config.ticks as usize));
    while sim.tick < config.ticks {
        sim.step(tape)?;
    }
    sim.trace
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_anim::contract::validate_pose;
    #[cfg(feature = "sim")]
    use engine_anim::locomotion::LocoState;
    use engine_anim::locomotion::{loco_skeleton, Locomotion};
    use engine_combat::fight::ATTACK_DAMAGE;
    use engine_combat::FightEvent;
    use engine_input::fighter::Buttons;
    use engine_input::fighter::{ATTACK, DODGE};
    use engine_input::{Bindings, BoundAction, PhysicalButton};

    fn config() -> ArenaConfig {
        ArenaConfig {
            seed: 2040,
            ticks: 600,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: TABLES_VERSION_V1,
        }
    }

    #[test]
    fn valid_config_passes() {
        config().validate().unwrap();
    }

    #[test]
    fn body_aliases_resolve_to_input_types() {
        let id: BodyId = FighterId::new(0).expect("slot 0 inside cap");
        let back: FighterId = id;
        assert_eq!(back, FighterId::new(0).expect("slot 0 inside cap"));
        let input: BodyInput = FighterInput {
            tick: 0,
            fighter: id,
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        };
        let back_input: FighterInput = input;
        assert_eq!(back_input.fighter, back);
        let tape: BodyTape = FighterTape(vec![input]);
        let back_tape: FighterTape = tape;
        assert_eq!(back_tape.0.len(), 1);
        assert_eq!(back_tape.0[0], back_input);
    }

    #[test]
    fn sim_clippy_config_matches_canonical() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let own = root.join("clippy.toml");
        let canonical = root
            .parent()
            .expect("crates dir")
            .parent()
            .expect("workspace root")
            .join("tools/sim-clippy.toml");
        let own_text = std::fs::read_to_string(&own).expect("crates/arena/clippy.toml must exist");
        let canonical_text =
            std::fs::read_to_string(&canonical).expect("tools/sim-clippy.toml must exist");
        assert_eq!(
            own_text, canonical_text,
            "crates/arena/clippy.toml must be byte-identical to tools/sim-clippy.toml"
        );
    }

    #[test]
    fn config_ranges_rejected() {
        assert_eq!(
            ArenaConfig {
                ticks: 0,
                ..config()
            }
            .validate(),
            Err(ArenaError::BadTicks(0))
        );
        assert_eq!(
            ArenaConfig {
                fighters: 1,
                ..config()
            }
            .validate(),
            Err(ArenaError::BadFighters(1))
        );
        assert_eq!(
            ArenaConfig {
                fighters: 5,
                ..config()
            }
            .validate(),
            Ok(()),
            "GE3: 5 fighters fits the 16-slot cap"
        );
        assert_eq!(
            ArenaConfig {
                fighters: 17,
                ..config()
            }
            .validate(),
            Err(ArenaError::BadFighters(17))
        );
        assert_eq!(
            ArenaConfig {
                half_extent_m: f32::NAN,
                ..config()
            }
            .validate(),
            Err(ArenaError::BadExtent)
        );
    }

    #[test]
    fn try_new_validates_each_field() {
        assert_eq!(ArenaConfig::try_new(2040, 600, 2, 20.0), Ok(config()));
        assert_eq!(
            ArenaConfig::try_new(2040, 0, 2, 20.0),
            Err(ArenaError::BadTicks(0))
        );
        assert_eq!(
            ArenaConfig::try_new(2040, 600, 1, 20.0),
            Err(ArenaError::BadFighters(1))
        );
        assert_eq!(
            ArenaConfig::try_new(2040, 600, 2, f32::NAN),
            Err(ArenaError::BadExtent)
        );
        assert_eq!(
            ArenaConfig::try_new(2040, 600, 2, 4.0),
            Err(ArenaError::BadExtent)
        );
    }

    #[test]
    fn checkpoint_is_copy_and_comparable() {
        let (a, b) = (
            Checkpoint { tick: 3, hash: 9 },
            Checkpoint { tick: 3, hash: 9 },
        );
        assert_eq!(a, b);
        assert_ne!(a, Checkpoint { tick: 4, hash: 9 });
    }

    fn tape_input(tick: u32, fighter: u8, mx: i8, mz: i8) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId::new(fighter).unwrap(),
            mx,
            mz,
            facing: 0,
            buttons: Buttons::NONE,
        }
    }

    fn sample_tape() -> FighterTape {
        FighterTape(vec![
            tape_input(0, 0, 100, 0),
            tape_input(0, 1, -100, 50),
            tape_input(5, 0, 0, 100),
            tape_input(9, 1, 20, -30),
        ])
    }

    /// Every slot idle every tick: pins all fighters to human control so no
    /// brain ever decides. Sorted by `(tick, fighter)` by construction.
    fn idle_tape(config: &ArenaConfig) -> FighterTape {
        let mut inputs = Vec::new();
        for tick in 0..config.ticks {
            for fighter in 0..config.fighters {
                inputs.push(FighterInput {
                    tick,
                    fighter: FighterId::new(fighter).unwrap(),
                    mx: 0,
                    mz: 0,
                    facing: 0,
                    buttons: Buttons::NONE,
                });
            }
        }
        FighterTape(inputs)
    }

    /// Fighter 0 attacks once on tick 0, fighter 1 idles: the attack impacts
    /// on tick 4 per the combat table (startup 4), unblocked, undodged.
    fn attack_tape(config: &ArenaConfig) -> FighterTape {
        let mut inputs = vec![FighterInput {
            tick: 0,
            fighter: FighterId::new(0).unwrap(),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons(ATTACK),
        }];
        for tick in 0..config.ticks {
            inputs.push(FighterInput {
                tick,
                fighter: FighterId::new(1).unwrap(),
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: Buttons::NONE,
            });
        }
        inputs.sort_by_key(|input| (input.tick, input.fighter.0));
        FighterTape(inputs)
    }

    fn short_config() -> ArenaConfig {
        ArenaConfig {
            seed: 2040,
            ticks: 32,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: TABLES_VERSION_V1,
        }
    }

    #[test]
    fn same_tape_twice_gives_identical_checkpoints() {
        let config = short_config();
        let tape = sample_tape();
        assert_eq!(run(&config, &tape), run(&config, &tape));
    }

    #[test]
    fn changed_wish_changes_a_checkpoint() {
        let config = short_config();
        let mut altered = sample_tape();
        altered.0[1].mx = 99;
        let (a, b) = (run(&config, &sample_tape()), run(&config, &altered));
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(b.iter()).any(|(x, y)| x != y));
    }

    #[test]
    fn changed_button_changes_a_checkpoint() {
        let config = short_config();
        let mut altered = sample_tape();
        altered.0[0].buttons = Buttons(ATTACK);
        let (a, b) = (run(&config, &sample_tape()), run(&config, &altered));
        assert!(a.iter().zip(b.iter()).any(|(x, y)| x != y));
    }

    #[test]
    fn neighbouring_seeds_differ() {
        let (mut a, mut b) = (short_config(), short_config());
        a.seed = 2000;
        b.seed = 2001;
        let tape = sample_tape();
        assert_ne!(run(&a, &tape), run(&b, &tape));
    }

    #[test]
    fn tape_for_wrong_fighter_count_fails_empty() {
        let config = short_config();
        let tape = FighterTape(vec![tape_input(0, 2, 10, 10)]);
        assert!(run(&config, &tape).is_empty());
    }

    #[test]
    fn tape_tick_past_end_fails_empty() {
        let config = short_config();
        let tape = FighterTape(vec![tape_input(config.ticks, 0, 10, 10)]);
        assert!(run(&config, &tape).is_empty());
    }

    #[test]
    fn invalid_config_or_tape_returns_empty() {
        let tape = sample_tape();
        let bad = ArenaConfig {
            ticks: 0,
            ..short_config()
        };
        assert!(run(&bad, &tape).is_empty());
        let bad_tape = FighterTape(vec![FighterInput {
            mx: 101,
            ..tape_input(0, 0, 0, 0)
        }]);
        assert!(run(&short_config(), &bad_tape).is_empty());
    }

    #[test]
    fn wall_clamp_keeps_fighters_in_bounds() {
        let config = ArenaConfig {
            seed: 7,
            ticks: 600,
            fighters: 2,
            half_extent_m: 5.0,
            table_version: TABLES_VERSION_V1,
        };
        let mut inputs = Vec::new();
        for tick in 0..config.ticks {
            inputs.push(tape_input(tick, 0, 100, 100));
            inputs.push(tape_input(tick, 1, -100, -100));
        }
        let tape = FighterTape(inputs);
        let limit = config.half_extent_m - FIGHTER_RADIUS_M;
        for state in final_states(&config, &tape) {
            assert!(state.pos[0] >= -limit && state.pos[0] <= limit);
            assert!(state.pos[2] >= -limit && state.pos[2] <= limit);
            assert!(state.pos[0].is_finite() && state.pos[2].is_finite());
        }
        assert_eq!(run(&config, &tape).len(), config.ticks as usize);
    }

    #[test]
    fn spawn_starts_inside_walls() {
        let config = short_config();
        let limit = config.half_extent_m - FIGHTER_RADIUS_M;
        for state in final_states(&config, &FighterTape::default()) {
            assert!(state.pos[0] >= -limit && state.pos[0] <= limit);
            assert!(state.pos[2] >= -limit && state.pos[2] <= limit);
        }
    }

    #[test]
    fn ai_vs_ai_600_ticks_identical_twice() {
        let config = config();
        let tape = FighterTape::default();
        let (first, second) = (run(&config, &tape), run(&config, &tape));
        assert_eq!(first.len(), config.ticks as usize);
        assert_eq!(first, second);
        let (events_first, events_second) =
            (run_events(&config, &tape), run_events(&config, &tape));
        assert_eq!(events_first.len(), config.ticks as usize);
        assert_eq!(events_first, events_second);
        // Brains fill the empty slots: the all-AI run diverges from the
        // all-idle run (every intent moves, presses buttons or drains
        // stamina, all of which the hash covers).
        assert_ne!(first, run(&config, &idle_tape(&config)));
    }

    #[test]
    fn run_dataset_covers_every_tick_and_fighter() {
        let config = short_config();
        let tape = sample_tape();
        let dataset = run_dataset(&config, &tape).expect("valid tape records");
        assert_eq!(dataset.version, recorder::DATASET_VERSION);
        assert_eq!(
            dataset.rows.len(),
            config.ticks as usize * usize::from(config.fighters)
        );
        // Tick-major, pre-step ticks shared across the tick's rows.
        for (index, row) in dataset.rows.iter().enumerate() {
            let tick = (index / usize::from(config.fighters)) as u32;
            assert_eq!(row.observation.tick, tick, "row {index}");
            assert_eq!(row.action.input.tick, tick, "row {index}");
            assert_eq!(row.observation.fighters.len(), usize::from(config.fighters));
        }
        // Same tape twice records the same rows; invalid tapes record
        // nothing, never partial.
        assert_eq!(
            run_dataset(&config, &tape).expect("valid tape records"),
            dataset
        );
        let bad_tape = FighterTape(vec![FighterInput {
            mx: 101,
            ..tape_input(0, 0, 0, 0)
        }]);
        assert!(run_dataset(&config, &bad_tape).is_none());
        assert!(run_dataset(&config, &sample_tape()).is_some());
    }

    #[test]
    fn scripted_attack_tape_produces_hit_events() {
        let config = short_config();
        let tape = attack_tape(&config);
        let events = run_events(&config, &tape);
        assert_eq!(events.len(), config.ticks as usize);
        let hits: Vec<(u32, &FightEvent)> = events
            .iter()
            .flat_map(|tick| tick.events.iter().map(|event| (tick.tick, event)))
            .filter(|(_, event)| matches!(event, FightEvent::Hit { .. }))
            .collect();
        assert!(!hits.is_empty(), "attack press must land: {events:?}");
        // Impact lands on tick 4 per the combat table (startup 4), idle foe.
        assert_eq!(hits[0].0, 4);
        assert!(matches!(
            hits[0].1,
            FightEvent::Hit {
                attacker: 0,
                victim: 1,
                damage: ATTACK_DAMAGE,
            }
        ));
        // Event bytes feed the checkpoint hash: the attack run diverges from
        // the all-idle run.
        assert_ne!(run(&config, &tape), run(&config, &idle_tape(&config)));
    }

    /// Close-range V2 sim helper: parks the duel 1 m apart on +z so the
    /// blade reaches, then pins V2 rules before tick 0.
    #[cfg(test)]
    fn close_v2_sim(config: &ArenaConfig, tape: &FighterTape) -> Sim {
        let mut sim = Sim::build(config, tape, false, false).expect("valid sim");
        sim.states[0].pos = [0.0, 0.0, 0.0];
        sim.states[1].pos = [0.0, 0.0, 1.0];
        assert!(sim.set_rules(ArenaRules::v2_free_for_all()));
        sim
    }

    /// Every-tick human tape: idle wishes, neutral facing, one attack press
    /// for slot 0 on tick 0. No brain ever decides.
    #[cfg(test)]
    fn close_attack_tape(config: &ArenaConfig) -> FighterTape {
        let mut inputs = Vec::new();
        for tick in 0..config.ticks {
            for fighter in 0..config.fighters {
                let buttons = if tick == 0 && fighter == 0 {
                    Buttons(ATTACK)
                } else {
                    Buttons::NONE
                };
                inputs.push(FighterInput {
                    tick,
                    fighter: FighterId::new(fighter).expect("slots 0..2 are legal"),
                    mx: 0,
                    mz: 0,
                    facing: 0,
                    buttons,
                });
            }
        }
        inputs.sort_by_key(|input| (input.tick, input.fighter.0));
        FighterTape(inputs)
    }

    /// Hit ticks of a close-range V2-rules run, in tick order.
    #[cfg(test)]
    fn v2_hit_ticks(config: &ArenaConfig, tape: &FighterTape) -> Vec<u32> {
        let mut sim = close_v2_sim(config, tape);
        let mut hits = Vec::new();
        while sim.tick < config.ticks {
            sim.step(tape).expect("valid step");
            if let Some(row) = sim.events.last() {
                for event in &row.events {
                    if matches!(event, FightEvent::Hit { .. }) {
                        hits.push(row.tick);
                        break;
                    }
                }
            }
        }
        hits
    }

    #[test]
    fn table_version_v1_steps_startup_4() {
        let config = short_config();
        assert_eq!(config.table_version, TABLES_VERSION_V1);
        assert_eq!(
            ArenaConfig::try_new(2040, 32, 2, 20.0)
                .expect("valid")
                .table_version,
            TABLES_VERSION_V1
        );
        assert_eq!(
            ArenaConfig {
                table_version: 7,
                ..config
            }
            .validate(),
            Err(ArenaError::BadTableVersion(7))
        );
        assert!(
            run(
                &ArenaConfig {
                    table_version: 7,
                    ..config
                },
                &FighterTape::default()
            )
            .is_empty(),
            "unknown table version refuses the whole run"
        );
        let tape = close_attack_tape(&config);
        assert_eq!(v2_hit_ticks(&config, &tape), vec![4]);
        assert_eq!(
            v2_hit_ticks(&config, &tape),
            vec![4],
            "same config plus tape twice gives identical hit ticks"
        );
    }

    #[test]
    fn table_version_v2_steps_startup_15() {
        let base = short_config();
        let config = ArenaConfig {
            table_version: TABLES_VERSION_V2,
            ..base
        };
        assert_eq!(config.validate(), Ok(()));
        assert!((12..=18).contains(&TABLES_V2.attack_startup_ticks));
        assert_eq!(TABLES_V2.attack_startup_ticks, 15);
        assert!((6..=8).contains(&TABLES_V2.parry_window_ticks));
        assert_eq!(TABLES_V2.parry_window_ticks, 7);
        let tape = close_attack_tape(&config);
        assert_eq!(v2_hit_ticks(&config, &tape), vec![15]);
        assert_eq!(
            v2_hit_ticks(&config, &tape),
            vec![15],
            "same config plus tape twice gives identical hit ticks"
        );
    }

    #[test]
    fn table_version_config_selects_the_rules_it_builds() {
        let v1 = short_config();
        let v2 = ArenaConfig {
            table_version: TABLES_VERSION_V2,
            ..v1
        };
        assert_eq!(v1.rules(), ArenaRules::V1);
        assert_eq!(v2.rules(), ArenaRules::v2_free_for_all());
        let tape = close_attack_tape(&v2);
        let built = |config: &ArenaConfig| Sim::build(config, &tape, false, false).expect("valid");
        assert_eq!(built(&v1).rules, ArenaRules::V1);
        // No explicit `set_rules`: the config alone puts the sim on v2, so a
        // LAN authority replaying the config agrees with the live match.
        let mut sim = built(&v2);
        assert_eq!(sim.rules, ArenaRules::v2_free_for_all());
        sim.states[0].pos = [0.0, 0.0, 0.0];
        sim.states[1].pos = [0.0, 0.0, 1.0];
        let mut first_hit = None;
        while sim.tick < v2.ticks && first_hit.is_none() {
            sim.step(&tape).expect("valid step");
            let row = sim.events.last().expect("a row per tick");
            if row
                .events
                .iter()
                .any(|e| matches!(e, FightEvent::Hit { .. }))
            {
                first_hit = Some(row.tick);
            }
        }
        assert_eq!(first_hit, Some(15), "tables v2 startup, not v1's 4");
        assert_ne!(run(&v1, &tape), run(&v2, &tape), "v1 and v2 hashes differ");
    }

    #[test]
    fn v2_press_tick_surfaces_cues_in_fighter_id_order() {
        use engine_combat::fight::AttackStarted;

        let base = short_config();
        let config = ArenaConfig {
            table_version: TABLES_VERSION_V2,
            ..base
        };
        let tape = scripted_match_tape(&config, &[(0, 0, ATTACK), (0, 1, ATTACK)]);
        let mut sim = Sim::build(&config, &tape, false, false).expect("valid sim");
        assert_eq!(sim.rules, ArenaRules::v2_free_for_all());
        sim.step(&tape).expect("valid step");
        assert_eq!(
            sim.cues(),
            [AttackStarted { attacker: 0 }, AttackStarted { attacker: 1 }].as_slice()
        );
        // Cues ride beside the event stream, never inside it: under v2
        // startup the press tick carries no outcome yet.
        let row = sim.events.last().expect("a row per tick");
        assert!(row
            .events
            .iter()
            .all(|e| !matches!(e, FightEvent::Hit { .. })));
        // Same config plus tape twice gives identical cues.
        let mut rerun = Sim::build(&config, &tape, false, false).expect("valid sim");
        rerun.step(&tape).expect("valid step");
        assert_eq!(rerun.cues(), sim.cues());
    }

    #[test]
    fn v1_rows_and_quiet_ticks_leave_cues_empty() {
        let config = short_config();
        let press = scripted_match_tape(&config, &[(0, 0, ATTACK)]);
        let mut sim = Sim::build(&config, &press, false, false).expect("valid sim");
        sim.step(&press).expect("valid step");
        assert!(sim.cues().is_empty(), "v1 stream stays frozen");
        let base = short_config();
        let v2 = ArenaConfig {
            table_version: TABLES_VERSION_V2,
            ..base
        };
        let quiet = scripted_match_tape(&v2, &[]);
        let mut sim = Sim::build(&v2, &quiet, false, false).expect("valid sim");
        sim.step(&quiet).expect("valid step");
        assert!(sim.cues().is_empty(), "no press means no cue");
    }

    #[test]
    fn cues_clear_after_the_press_tick() {
        use engine_combat::fight::AttackStarted;

        let base = short_config();
        let config = ArenaConfig {
            table_version: TABLES_VERSION_V2,
            ..base
        };
        let tape = scripted_match_tape(&config, &[(0, 0, ATTACK)]);
        let mut sim = Sim::build(&config, &tape, false, false).expect("valid sim");
        sim.step(&tape).expect("valid step");
        assert_eq!(sim.cues(), [AttackStarted { attacker: 0 }].as_slice());
        // The swing is still in flight (v2 startup is 15 ticks) but the cue
        // belongs to the press tick only, so the next tick reads empty.
        sim.step(&tape).expect("valid step");
        assert!(sim.cues().is_empty(), "cues never linger a tick");
    }

    /// Fully human-driven tape: every slot every tick carries an explicit
    /// input, so no brain ever decides and the exchange script is exact.
    /// `presses` holds `(tick, fighter, buttons)` overrides over idle.
    #[cfg(test)]
    fn scripted_match_tape(config: &ArenaConfig, presses: &[(u32, u8, u8)]) -> FighterTape {
        let mut inputs = Vec::new();
        for tick in 0..config.ticks {
            for fighter in 0..config.fighters {
                let mut buttons = Buttons::NONE;
                for (t, f, b) in presses {
                    if *t == tick && *f == fighter {
                        buttons = Buttons(*b);
                    }
                }
                inputs.push(FighterInput {
                    tick,
                    fighter: FighterId::new(fighter).unwrap(),
                    mx: 0,
                    mz: 0,
                    facing: 0,
                    buttons,
                });
            }
        }
        FighterTape(inputs)
    }

    #[test]
    fn match_track_parry_and_clash_resolve_x2() {
        use engine_combat::exchange::ExchangeEvent;
        use engine_input::fighter::BLOCK;

        // Tick 0: 0 attacks while 1 holds block in-window, so the match
        // step answers a parry; tick 1: 1 presses dodge inside the open
        // window, so the locked riposte lands; tick 2: both attack, and
        // the clash names slot 1 on the carried poise stakes.
        let config = short_config();
        let tape = scripted_match_tape(
            &config,
            &[
                (0, 0, ATTACK),
                (0, 1, BLOCK),
                (1, 1, DODGE),
                (2, 0, ATTACK),
                (2, 1, ATTACK),
            ],
        );
        let first = run_match_events(&config, &tape);
        let second = run_match_events(&config, &tape);
        assert_eq!(first.len(), config.ticks as usize);
        assert_eq!(first, second);
        for (tick, row) in first.iter().enumerate() {
            assert_eq!(row.tick, tick as u32, "one match row per tick");
        }
        assert_eq!(
            first[0].exchange,
            vec![ExchangeEvent::Parried {
                attacker: 0,
                victim: 1
            }]
        );
        assert!(
            matches!(
                first[1].exchange[..],
                [ExchangeEvent::Riposte {
                    attacker: 1,
                    victim: 0,
                    ..
                }]
            ),
            "dodge inside the window must riposte: {:?}",
            first[1].exchange
        );
        // Winner 1: the tick-0 parry cost attacker 0 its poise stake
        // (750 vs 1000), so poise carries across ticks through the wiring.
        assert_eq!(
            first[2].exchange,
            vec![ExchangeEvent::Clash {
                a: 0,
                b: 1,
                winner: 1
            }]
        );
        // Canonical bytes ride the same path: identical twice, with the
        // parry, riposte and clash tags all present on their ticks.
        let bytes_first = run_match_bytes(&config, &tape);
        let bytes_second = run_match_bytes(&config, &tape);
        assert_eq!(bytes_first.len(), config.ticks as usize);
        assert_eq!(bytes_first, bytes_second);
        assert!(bytes_first[0].contains(&10), "tick 0 carries parry");
        assert!(bytes_first[1].contains(&40), "tick 1 carries riposte");
        assert!(bytes_first[2].contains(&12), "tick 2 carries clash");
        // The match log is observable but never hashed: checkpoints repeat,
        // and invalid tapes log nothing, never partial.
        assert_eq!(run(&config, &tape), run(&config, &tape));
        let bad = FighterTape(vec![FighterInput {
            mx: 101,
            ..tape_input(0, 0, 0, 0)
        }]);
        assert!(run_match_events(&config, &bad).is_empty());
        assert!(run_match_bytes(&config, &bad).is_empty());
    }

    #[test]
    fn match_version_v0_stable_v1_gated_x2() {
        use engine_input::fighter::BLOCK;

        // Slot 0 holds attack, slot 1 holds block: legacy answers every
        // tick, phased only on live ticks. Same tape, both flags, twice.
        let config = short_config();
        let mut inputs = Vec::new();
        for tick in 0..config.ticks {
            for (fighter, buttons) in [(0u8, ATTACK), (1u8, BLOCK)] {
                inputs.push(FighterInput {
                    tick,
                    fighter: FighterId::new(fighter).unwrap(),
                    mx: 0,
                    mz: 0,
                    facing: 0,
                    buttons: Buttons(buttons),
                });
            }
        }
        inputs.sort_by_key(|input| (input.tick, input.fighter.0));
        let tape = FighterTape(inputs);
        let v0_first = run_match_bytes(&config, &tape);
        let v0_second = run_match_bytes(&config, &tape);
        assert_eq!(v0_first, v0_second);
        let v1_first = run_match_bytes_v1(&config, &tape);
        let v1_second = run_match_bytes_v1(&config, &tape);
        assert_eq!(v1_first, v1_second);
        assert_ne!(v0_first, v1_first, "flag must move the observer bytes");
        // Legacy answers on tick 0 with a parry. (Sweeps stay quiet here:
        // the spawn spread parks the duel out of blade reach, so the byte
        // gate below watches exchange tags only; steel gating is pinned at
        // the call-site level in `match_wire`.)
        assert!(v0_first[0].contains(&10), "v0 tick 0 carries parry");
        // Phased gates startup: ticks 0..4 answer nothing.
        for (tick, row) in v1_first.iter().enumerate().take(4) {
            assert!(
                !row.contains(&10) && !row.contains(&12),
                "v1 tick {tick} must stay quiet through startup"
            );
        }
        // First live tick answers, then the cross window answers again.
        assert!(v1_first[4].contains(&10), "v1 tick 4 carries parry");
        for (tick, row) in v1_first.iter().enumerate().take(13).skip(10) {
            assert!(row.contains(&10), "v1 tick {tick} carries cross parry");
        }
        // The flag is observer-only: authority checkpoints repeat untouched.
        assert_eq!(run(&config, &tape), run(&config, &tape));
    }

    #[test]
    fn versioned_spellings_match_both_legs() {
        // The versioned entry plus the pinned spellings resolve one track:
        // V0 through both spellings answers, V1 through both spellings
        // gates, and an unknown version logs nothing.
        let config = short_config();
        let tape = scripted_match_tape(
            &config,
            &[
                (0, 0, ATTACK),
                (0, 1, engine_input::fighter::BLOCK),
                (2, 0, ATTACK),
                (2, 1, ATTACK),
            ],
        );
        assert_eq!(
            run_match_events(&config, &tape),
            run_match_events_versioned(&config, &tape, MATCH_WIRE_VERSION_V0)
        );
        assert_eq!(
            run_match_events_v1(&config, &tape),
            run_match_events_versioned(&config, &tape, MATCH_WIRE_VERSION_V1)
        );
        assert_eq!(
            run_match_bytes(&config, &tape),
            run_match_bytes_versioned(&config, &tape, MATCH_WIRE_VERSION_V0)
        );
        assert_eq!(
            run_match_bytes_v1(&config, &tape),
            run_match_bytes_versioned(&config, &tape, MATCH_WIRE_VERSION_V1)
        );
        assert_ne!(
            run_match_bytes(&config, &tape),
            run_match_bytes_v1(&config, &tape),
            "legs must differ on this tape"
        );
        assert!(run_match_events_versioned(&config, &tape, 7).is_empty());
        assert!(run_match_bytes_versioned(&config, &tape, 7).is_empty());
    }

    #[test]
    fn match_state_crosses_snapshot_restore() {
        use engine_combat::exchange::ExchangeEvent;

        // Tick 0 clashes both fighters (binds for 6 ticks); tick 2 presses
        // an attack into the bind, which the bound track must swallow. A
        // restore that dropped the match clocks would parry on tick 2.
        let config = short_config();
        let tape = scripted_match_tape(
            &config,
            &[
                (0, 0, ATTACK),
                (0, 1, ATTACK),
                (2, 0, ATTACK),
                (2, 1, engine_input::fighter::BLOCK),
            ],
        );
        let full = run_match_events(&config, &tape);
        assert_eq!(full.len(), config.ticks as usize);
        assert_eq!(
            full[0].exchange,
            vec![ExchangeEvent::Clash {
                a: 0,
                b: 1,
                winner: 0
            }]
        );
        assert!(
            full[2].exchange.is_empty(),
            "bound slots swallow tick 2: {:?}",
            full[2].exchange
        );
        // Prefix one tick, resume, and compare the suffix match rows: the
        // restored bind clock must swallow tick 2 exactly like the full run.
        let (snapshot, _) = run_prefix(&config, &tape, 1).expect("valid prefix");
        let resumed = resume_inner(snapshot, &config, &tape).expect("valid resume");
        assert_eq!(resumed.match_log, full[1..].to_vec());
    }

    #[test]
    fn v1_prefix_resume_matches_uninterrupted_v1() {
        use engine_input::fighter::BLOCK;

        // Held attack into held block: V1 gates startup, so a restore that
        // dropped the phased cursors would answer early on the suffix.
        let config = short_config();
        let mut inputs = Vec::new();
        for tick in 0..config.ticks {
            for (fighter, buttons) in [(0u8, ATTACK), (1u8, BLOCK)] {
                inputs.push(FighterInput {
                    tick,
                    fighter: FighterId::new(fighter).unwrap(),
                    mx: 0,
                    mz: 0,
                    facing: 0,
                    buttons: Buttons(buttons),
                });
            }
        }
        inputs.sort_by_key(|input| (input.tick, input.fighter.0));
        let tape = FighterTape(inputs);
        let full_v1 = run_match_events_versioned(&config, &tape, MATCH_WIRE_VERSION_V1);
        assert_eq!(full_v1.len(), config.ticks as usize);
        // Split inside startup: the suffix must stay quiet until the live tick.
        let (snapshot, prefix) = run_prefix_versioned(&config, &tape, 2, MATCH_WIRE_VERSION_V1)
            .expect("valid V1 prefix");
        assert_eq!(snapshot.match_version(), MATCH_WIRE_VERSION_V1);
        assert_eq!(prefix.len(), 2);
        let resumed = resume_inner(snapshot, &config, &tape).expect("valid resume");
        assert_eq!(resumed.match_log, full_v1[2..].to_vec());
        // V0 unchanged: the legacy spelling matches the uninterrupted V0 run.
        let full_v0 = run_match_events(&config, &tape);
        let (legacy, legacy_prefix) = run_prefix(&config, &tape, 2).expect("valid prefix");
        assert_eq!(legacy.match_version(), MATCH_WIRE_VERSION_V0);
        assert_eq!(
            legacy_prefix, prefix,
            "authority checkpoints ignore the flag"
        );
        let resumed_v0 = resume_inner(legacy, &config, &tape).expect("valid resume");
        assert_eq!(resumed_v0.match_log, full_v0[2..].to_vec());
        // A V1 fork clones the same carried cursors and stays deterministic.
        let (fork_snap, _) = run_prefix_versioned(&config, &tape, 2, MATCH_WIRE_VERSION_V1)
            .expect("valid V1 prefix");
        let fork = fork::Fork::from_snapshot(fork_snap, 2).expect("V1 snapshot forks");
        assert_eq!(fork.run_n(4), fork.run_n(4));
    }

    #[test]
    fn prefix_versioned_rejects_unknown_version() {
        let config = short_config();
        let tape = scripted_match_tape(&config, &[(0, 0, ATTACK), (0, 1, ATTACK)]);
        assert!(run_prefix_versioned(&config, &tape, 2, 7).is_none());
        assert!(run_match_events_versioned(&config, &tape, 7).is_empty());
        assert!(run_match_bytes_versioned(&config, &tape, 7).is_empty());
    }

    #[test]
    fn cold_path_skips_match_log_but_keeps_authority() {
        use engine_combat::exchange::ExchangeEvent;
        use engine_input::fighter::BLOCK;

        // Same parry/riposte/clash script as the observer test above, so an
        // empty cold log proves gating, never a quiet tape.
        let config = short_config();
        let tape = scripted_match_tape(
            &config,
            &[
                (0, 0, ATTACK),
                (0, 1, BLOCK),
                (1, 1, DODGE),
                (2, 0, ATTACK),
                (2, 1, ATTACK),
            ],
        );
        let mut cold = Sim::build(&config, &tape, false, false).expect("valid cold sim");
        while cold.tick < config.ticks {
            cold.step(&tape).expect("valid cold step");
        }
        assert!(cold.match_log.is_empty(), "cold path logs no match rows");
        assert_eq!(cold.checkpoints.len(), config.ticks as usize);
        assert_eq!(cold.events.len(), config.ticks as usize);
        assert_eq!(
            cold.checkpoints,
            run(&config, &tape),
            "cold sim matches the authority entry point"
        );
        let live = run_match_events(&config, &tape);
        assert_eq!(
            live[0].exchange,
            vec![ExchangeEvent::Parried {
                attacker: 0,
                victim: 1
            }],
            "control: the tape is eventful through the live observer"
        );
        // Failure cases: bad config or bad tape builds nothing on either path.
        let bad_config = ArenaConfig { ticks: 0, ..config };
        assert!(Sim::build(&bad_config, &tape, false, false).is_none());
        assert!(Sim::build(&bad_config, &tape, false, true).is_none());
        let bad_tape = FighterTape(vec![FighterInput {
            mx: 101,
            ..tape_input(0, 0, 0, 0)
        }]);
        assert!(Sim::build(&config, &bad_tape, false, false).is_none());
        assert!(Sim::build(&config, &bad_tape, false, true).is_none());
    }

    #[test]
    fn cold_path_saves_heap_and_keeps_hash() {
        use engine_input::fighter::BLOCK;

        fn run_counted(
            config: &ArenaConfig,
            tape: &FighterTape,
            match_live: bool,
        ) -> (u64, Vec<Checkpoint>) {
            let mut checkpoints = Vec::new();
            let ((), measured) = engine_core::alloc_probe::measure(|| {
                let mut sim = Sim::build(config, tape, false, match_live).expect("valid sim");
                while sim.tick < config.ticks {
                    sim.step(tape).expect("valid step");
                }
                checkpoints = sim.checkpoints.clone();
            });
            (measured.calls, checkpoints)
        }

        let config = short_config();
        let tape = scripted_match_tape(
            &config,
            &[
                (0, 0, ATTACK),
                (0, 1, BLOCK),
                (1, 1, DODGE),
                (2, 0, ATTACK),
                (2, 1, ATTACK),
            ],
        );
        let (cold_count, cold_hash) = run_counted(&config, &tape, false);
        let (live_count, live_hash) = run_counted(&config, &tape, true);
        assert_eq!(cold_hash, live_hash, "gating never touches the hash");
        assert!(
            cold_count < live_count,
            "cold path must save heap: cold {cold_count} vs live {live_count}"
        );
    }

    #[test]
    fn sword_hit_breaks_crate_identical_twice() {
        let config = short_config();
        let tape = attack_tape(&config);
        let (first, second) = (run_breaks(&config, &tape), run_breaks(&config, &tape));
        assert_eq!(first.len(), config.ticks as usize);
        assert_eq!(first, second);
        for (tick, row) in first.iter().enumerate() {
            assert_eq!(row.tick, tick as u32, "one break row per tick");
        }
        // The tick-4 sword hit fractures the prop on the same tick.
        assert!(
            !first[4].events.is_empty(),
            "sword hit must break the crate: {first:?}"
        );
        assert!(
            first[4]
                .events
                .iter()
                .any(|event| matches!(event, BreakEvent::BondBroken { .. })),
            "impact snaps bonds: {:?}",
            first[4].events
        );
        assert!(
            first
                .iter()
                .flat_map(|row| row.events.iter())
                .any(|event| matches!(event, BreakEvent::ChunkFreed { .. })),
            "detached islands must free: {first:?}"
        );
        // Idle fighters never swing: the prop stays whole for the whole run.
        let idle = run_breaks(&config, &idle_tape(&config));
        assert_eq!(idle.len(), config.ticks as usize);
        assert!(idle.iter().all(|row| row.events.is_empty()));
        // Breaks ride the checkpoints: the attack run diverges from idle.
        assert_ne!(run(&config, &tape), run(&config, &idle_tape(&config)));
    }

    #[test]
    fn crate_rest_names_the_crate_the_match_breaks() {
        let config = short_config();
        let rest = crate_rest(&config).expect("the crate bakes");
        assert_eq!(rest.center_m, CRATE_CENTER_M);
        assert!(!rest.chunks.is_empty() && !rest.anchors.is_empty());
        let again = crate_rest(&config).expect("the crate bakes twice");
        assert_eq!(rest.chunks.len(), again.chunks.len());
        assert_eq!(rest.bonds.len(), again.bonds.len());
        assert_eq!(rest.anchors, again.anchors);
        // Every index a live break names is a chunk or a bond of this rest crate.
        let breaks = run_breaks(&config, &attack_tape(&config));
        let mut named = 0usize;
        for event in breaks.iter().flat_map(|row| row.events.iter()) {
            match *event {
                BreakEvent::BondBroken { bond, .. } => {
                    assert!((bond as usize) < rest.bonds.len(), "bond {bond}");
                }
                BreakEvent::ChunkFreed { chunk, .. } => {
                    assert!((chunk as usize) < rest.chunks.len(), "chunk {chunk}");
                }
            }
            named += 1;
        }
        assert!(named > 0, "the attack tape breaks the crate");
        // A bad config bakes nothing it should not: the seed alone moves the pattern.
        let other = ArenaConfig { seed: 7, ..config };
        let moved = crate_rest(&other).expect("another seed bakes");
        assert_eq!(moved.chunks.len(), rest.chunks.len());
    }

    #[test]
    fn breaks_feed_the_checkpoint_hash() {
        // Same poses/buttons/fight/physics, different break slices: the hash
        // must differ, so silent ticks can never alias a breaking tick.
        let config = short_config();
        let states = spawn_states(config.seed, config.fighters, config.half_extent_m);
        let buttons = [0; MAX_SLOTS];
        let fight = FightState::new(config.fighters).unwrap();
        let locos = [Locomotion::new(), Locomotion::new()];
        let brk = BreakEvent::ChunkFreed { tick: 4, chunk: 3 };
        let quiet = hash_state(
            4,
            &states,
            &buttons[..2],
            &fight,
            &[],
            0x1234,
            &locos,
            0x5678,
            &[],
        );
        let loud = hash_state(
            4,
            &states,
            &buttons[..2],
            &fight,
            &[],
            0x1234,
            &locos,
            0x5678,
            &[brk],
        );
        assert_ne!(quiet, loud);
        assert_eq!(
            loud,
            hash_state(
                4,
                &states,
                &buttons[..2],
                &fight,
                &[],
                0x1234,
                &locos,
                0x5678,
                &[brk],
            )
        );
        // The destruction prop hash feeds the checkpoints too.
        assert_ne!(
            quiet,
            hash_state(
                4,
                &states,
                &buttons[..2],
                &fight,
                &[],
                0x1234,
                &locos,
                0x9abc,
                &[],
            )
        );
    }

    #[test]
    fn invalid_tape_gives_empty_breaks() {
        let config = short_config();
        let bad_wish = FighterTape(vec![FighterInput {
            mx: 101,
            ..tape_input(0, 0, 0, 0)
        }]);
        assert!(run_breaks(&config, &bad_wish).is_empty());
        let bad = ArenaConfig {
            ticks: 0,
            ..short_config()
        };
        assert!(run_breaks(&bad, &sample_tape()).is_empty());
        // Control: the valid tape logs one row per tick.
        assert_eq!(
            run_breaks(&config, &sample_tape()).len(),
            config.ticks as usize
        );
    }

    #[test]
    fn combat_events_feed_the_checkpoint_hash() {
        // Same poses/buttons/fight/physics, different event slices: the hash
        // must differ, so event counts can never alias an idle tick.
        let config = short_config();
        let states = spawn_states(config.seed, config.fighters, config.half_extent_m);
        let buttons = [0; MAX_SLOTS];
        let fight = FightState::new(config.fighters).unwrap();
        let locos = [Locomotion::new(), Locomotion::new()];
        let quiet = hash_state(
            4,
            &states,
            &buttons[..2],
            &fight,
            &[],
            0x1234,
            &locos,
            0,
            &[],
        );
        let hit = FightEvent::Hit {
            attacker: 0,
            victim: 1,
            damage: ATTACK_DAMAGE,
        };
        let loud = hash_state(
            4,
            &states,
            &buttons[..2],
            &fight,
            &[hit],
            0x1234,
            &locos,
            0,
            &[],
        );
        assert_ne!(quiet, loud);
        // Same inputs twice hash identically (no ordering or padding leak).
        assert_eq!(
            loud,
            hash_state(
                4,
                &states,
                &buttons[..2],
                &fight,
                &[hit],
                0x1234,
                &locos,
                0,
                &[]
            )
        );
    }

    #[test]
    fn changed_tape_diverges() {
        // Removing the attack press changes combat outcomes and hp, so the
        // checkpoints must diverge from the attack run.
        let config = short_config();
        let attack = attack_tape(&config);
        let mut disarmed = attack.clone();
        disarmed.0[0].buttons = Buttons::NONE;
        let (a, b) = (run(&config, &attack), run(&config, &disarmed));
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(b.iter()).any(|(x, y)| x != y));
    }

    #[test]
    fn snapshot_restore_mid_match_resimulates_identical() {
        let config = config();
        // Human wishes plus an attack press on fighter 0; fighter 1 is
        // AI-filled, so brains, combat and physics all cross the snapshot.
        let mut inputs = vec![
            tape_input(0, 0, 100, 0),
            FighterInput {
                tick: 10,
                fighter: FighterId::new(0).unwrap(),
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: Buttons(ATTACK),
            },
            tape_input(40, 0, 0, -100),
        ];
        inputs.sort_by_key(|input| (input.tick, input.fighter.0));
        let tape = FighterTape(inputs);
        let full = run(&config, &tape);
        assert_eq!(full.len(), config.ticks as usize);
        let (snapshot, prefix) = run_prefix(&config, &tape, 200).expect("valid prefix");
        assert_eq!(prefix.len(), 200);
        let suffix = resume(snapshot, &config, &tape);
        assert_eq!(suffix.len(), config.ticks as usize - 200);
        let rejoined: Vec<Checkpoint> = prefix.into_iter().chain(suffix).collect();
        assert_eq!(rejoined, full);
        // A config mismatch resumes nothing, never partially (its snapshot
        // is consumed by the rejected resume).
        let mut other = config;
        other.seed = 999;
        let (stale, _) = run_prefix(&config, &tape, 200).expect("valid prefix");
        assert!(resume(stale, &other, &tape).is_empty());
        // A prefix past the end is rejected, not clamped.
        assert!(run_prefix(&config, &tape, config.ticks + 1).is_none());
    }

    #[test]
    fn invalid_tape_input_rejected_loudly() {
        let config = short_config();
        // Unknown button bits: the whole run is empty, not one skipped input.
        let bad_buttons = FighterTape(vec![FighterInput {
            tick: 0,
            fighter: FighterId::new(0).unwrap(),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons(0x08),
        }]);
        assert!(run(&config, &bad_buttons).is_empty());
        assert!(run_events(&config, &bad_buttons).is_empty());
        assert!(run_presentation(&config, &bad_buttons).is_empty());
        assert!(run_prefix(&config, &bad_buttons, 4).is_none());
        // Out-of-range wish: same loud rejection across every entrypoint.
        let bad_wish = FighterTape(vec![FighterInput {
            mx: 101,
            ..tape_input(0, 0, 0, 0)
        }]);
        assert!(run(&config, &bad_wish).is_empty());
        assert!(run_events(&config, &bad_wish).is_empty());
        // A resume under an invalid tape writes nothing either.
        let (snapshot, _) = run_prefix(&config, &sample_tape(), 8).expect("valid prefix");
        assert!(resume(snapshot, &config, &bad_wish).is_empty());
        // Control: the valid tape is non-empty through every entrypoint.
        assert!(!run(&config, &sample_tape()).is_empty());
        assert!(!run_events(&config, &sample_tape()).is_empty());
        assert!(!run_presentation(&config, &sample_tape()).is_empty());
        assert!(!run_loco_poses(&config, &sample_tape()).is_empty());
    }

    #[test]
    fn presentation_roots_validate_and_replay() {
        use engine_anim::contract::validate_root;

        let config = short_config();
        let tape = sample_tape();
        let tables = run_presentation(&config, &tape);
        assert_eq!(tables.len(), usize::from(config.fighters));
        for table in &tables {
            validate_root(table).expect("arena tables always validate");
            assert_eq!(table.deltas.len(), config.ticks as usize);
            assert_eq!(table.deltas[0].tick, 0);
            assert!(table
                .deltas
                .windows(2)
                .all(|pair| pair[1].tick == pair[0].tick + 1));
        }
        // Same trajectory twice gives the same tables.
        assert_eq!(tables, run_presentation(&config, &tape));
    }

    #[test]
    fn presentation_pose_samples_without_touching_authority() {
        use engine_anim::contract::Joint;
        use engine_anim::{Clip, Track};

        let skeleton = Skeleton {
            joints: vec![
                Joint {
                    name: "root".into(),
                    parent: None,
                },
                Joint {
                    name: "spine".into(),
                    parent: Some(0),
                },
            ],
        };
        let clip = Clip {
            tracks: vec![
                Track {
                    times: vec![0.0, 1.0],
                    values: vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
                },
                Track {
                    times: vec![0.0, 1.0],
                    values: vec![[0.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
                },
            ],
        };
        // Tick 30 is 0.5 s: the root sits at the clip midpoint.
        let pose = presentation_pose(&skeleton, &clip, 30).expect("valid sample");
        assert_eq!(pose.positions[0], [1.0, 0.0, 0.0]);
        // Sampling wraps with the clip phase and never feeds the match hash.
        assert_eq!(
            presentation_pose(&skeleton, &clip, 90).expect("wrapped sample"),
            pose
        );
        let config = short_config();
        let tape = sample_tape();
        let before = run(&config, &tape);
        let _ = presentation_pose(&skeleton, &clip, 7);
        assert_eq!(run(&config, &tape), before);
    }

    #[test]
    fn sim_tag_byte_separates_configurations() {
        // The tag byte keeps default and `sim` hashes from ever comparing
        // equal, even over identical state, combat and physics bytes.
        let config = short_config();
        let states = spawn_states(config.seed, config.fighters, config.half_extent_m);
        let buttons = [0; MAX_SLOTS];
        let fight = FightState::new(config.fighters).unwrap();
        let locos = [Locomotion::new(), Locomotion::new()];
        let hit = FightEvent::Hit {
            attacker: 0,
            victim: 1,
            damage: ATTACK_DAMAGE,
        };
        assert_ne!(
            hash_state_with_sim(
                4,
                &states,
                &buttons[..2],
                &fight,
                &[hit],
                0x1234,
                &locos,
                0,
                &[],
                false
            ),
            hash_state_with_sim(
                4,
                &states,
                &buttons[..2],
                &fight,
                &[hit],
                0x1234,
                &locos,
                0,
                &[],
                true
            )
        );
        // Each tag is self-consistent (no ordering or padding leak).
        assert_eq!(
            hash_state_with_sim(
                4,
                &states,
                &buttons[..2],
                &fight,
                &[hit],
                0x1234,
                &locos,
                0,
                &[],
                true
            ),
            hash_state_with_sim(
                4,
                &states,
                &buttons[..2],
                &fight,
                &[hit],
                0x1234,
                &locos,
                0,
                &[],
                true
            )
        );
    }

    /// Settled controllers (`sim` only): capsules spawned at rest height
    /// hold rest over 600 fixed steps, the hash is stable at rest, and the
    /// public snapshot/restore pair resimulates bit-identical.
    #[cfg(feature = "sim")]
    #[test]
    fn sim_controllers_settle_and_rest() {
        let config = short_config();
        let states = spawn_states(config.seed, config.fighters, config.half_extent_m);
        let mut controllers = build_controllers(&config, &states).expect("controllers build");
        assert_eq!(controllers.len(), states.len());
        for _ in 0..600 {
            for controller in controllers.iter_mut() {
                controller.move_and_slide([0.0, 0.0], FIXED_DT);
            }
        }
        for (id, controller) in controllers.iter().enumerate() {
            let pos = controller.position();
            assert!(
                (pos[0] - states[id].pos[0]).abs() < 1e-6,
                "no planar drift {id}: {pos:?}"
            );
            assert!(
                (pos[1] - CAPSULE_REST_Y_M).abs() < 0.1,
                "capsule {id} y={} want {CAPSULE_REST_Y_M}",
                pos[1]
            );
            assert!(pos.iter().all(|c| c.is_finite()), "{pos:?}");
            assert!(controller.grounded(), "capsule {id} rests grounded");
        }
        let hash = controllers
            .iter()
            .map(|c| c.state_hash())
            .reduce(|a, b| a ^ b)
            .unwrap_or(0);
        assert_eq!(
            hash,
            controllers
                .iter()
                .map(|c| c.state_hash())
                .reduce(|a, b| a ^ b)
                .unwrap_or(0)
        );
        let snaps: Vec<_> = controllers.iter().map(|c| c.snapshot()).collect();
        for _ in 0..30 {
            for controller in controllers.iter_mut() {
                controller.move_and_slide([0.05, 0.0], FIXED_DT);
            }
        }
        let after: Vec<_> = controllers
            .iter()
            .map(|c| (c.state_hash(), c.position()))
            .collect();
        for (controller, snap) in controllers.iter_mut().zip(snaps) {
            controller.restore(snap).expect("restore");
        }
        for _ in 0..30 {
            for controller in controllers.iter_mut() {
                controller.move_and_slide([0.05, 0.0], FIXED_DT);
            }
        }
        let replay: Vec<_> = controllers
            .iter()
            .map(|c| (c.state_hash(), c.position()))
            .collect();
        assert_eq!(replay, after);
    }

    // ---- LOC2 integration ---------------------------------------------------

    /// One human tick through the IN1 public API: button bits from `pressed`,
    /// caller-quantized wish, absolute facing. Out-of-range wishes fail
    /// loudly instead of clamping, so bad callers reject the whole run.
    fn human_input(
        bindings: &Bindings,
        tick: u32,
        fighter: u8,
        pressed: &[PhysicalButton],
        wish: (i8, i8),
        facing: i16,
    ) -> FighterInput {
        bindings
            .to_fighter_input(
                tick,
                FighterId::new(fighter).expect("course uses slots 0..2"),
                pressed,
                wish,
                facing,
            )
            .expect("course wishes stay in range")
    }

    /// LOC2 recorded course: move/accelerate, stop, 180-degree turn, strafe,
    /// then one attack whose hit-stop freezes both driving fighters. Fully
    /// human-driven (every tick every slot) through IN1 bindings output, so
    /// no brain ever decides on this course.
    fn loc2_course_tape() -> FighterTape {
        let bindings = Bindings::new();
        let attack = bindings.button(BoundAction::Attack);
        let mut inputs = Vec::new();
        for tick in 0..120u32 {
            // Fighter 0: full +X, stop, reverse with an about-face, strafe,
            // then +X again with one attack press on tick 82 (impact on 86
            // per the combat table) before going idle.
            let (wish0, facing0, attack0) = if tick < 30 {
                ((100, 0), 0, false)
            } else if tick < 42 {
                ((0, 0), 0, false)
            } else if tick < 62 {
                ((-100, 0), 9_000, false)
            } else if tick < 82 {
                ((0, 100), 0, false)
            } else if tick < 100 {
                ((100, 0), 0, tick == 82)
            } else {
                ((0, 0), 0, false)
            };
            let pressed0: &[PhysicalButton] = if attack0 { &[attack] } else { &[] };
            inputs.push(human_input(&bindings, tick, 0, pressed0, wish0, facing0));
            // Fighter 1: idle, then full +Z through the impact tick so the
            // hit-stop freeze is observable on a driving victim, then idle.
            let wish1 = if (78..110).contains(&tick) {
                (0, 100)
            } else {
                (0, 0)
            };
            inputs.push(human_input(&bindings, tick, 1, &[], wish1, 0));
        }
        FighterTape(inputs)
    }

    fn loc2_course_config() -> ArenaConfig {
        ArenaConfig {
            seed: 2040,
            ticks: 120,
            fighters: 2,
            half_extent_m: 10.0,
            table_version: TABLES_VERSION_V1,
        }
    }

    fn hit_ticks(events: &[TickEvents]) -> Vec<u32> {
        let mut ticks = Vec::new();
        for tick in events {
            for event in &tick.events {
                if matches!(event, FightEvent::Hit { .. }) {
                    ticks.push(tick.tick);
                }
            }
        }
        ticks
    }

    #[test]
    fn loc2_course_recorded_x2_identical() {
        let config = loc2_course_config();
        let tape = loc2_course_tape();
        let (first, second) = (run(&config, &tape), run(&config, &tape));
        assert_eq!(first.len(), config.ticks as usize);
        assert_eq!(first, second);
        let (events_first, events_second) =
            (run_events(&config, &tape), run_events(&config, &tape));
        assert_eq!(events_first.len(), config.ticks as usize);
        assert_eq!(events_first, events_second);
        // The attack press lands once, on tick 86 (startup 4 from tick 82).
        assert_eq!(hit_ticks(&events_first), vec![86]);
        // Locomotion presentation poses ride along: per fighter, per tick,
        // always valid on the LOC1 skeleton, identical twice.
        let poses = run_loco_poses(&config, &tape);
        assert_eq!(poses.len(), usize::from(config.fighters));
        let skeleton = loco_skeleton();
        for fighter in &poses {
            assert_eq!(fighter.len(), config.ticks as usize);
            for pose in fighter {
                validate_pose(&skeleton, pose).expect("loco poses always validate");
            }
        }
        assert_eq!(poses, run_loco_poses(&config, &tape));
        // Changed input diverges: the course covers wishes, facing and
        // buttons across its phases. (The wish change below crosses the
        // idle threshold: a one-point change inside a capped gait would be
        // absorbed by the speed cap by design.)
        let mut altered = tape.clone();
        altered.0[0].mx = 0;
        altered.0[0].mz = 0;
        let diverted = run(&config, &altered);
        assert!(run(&config, &tape)
            .iter()
            .zip(diverted.iter())
            .any(|(a, b)| a != b));
        let mut refaced = tape.clone();
        refaced.0[50].facing = 100;
        let refaced_run = run(&config, &refaced);
        assert!(run(&config, &tape)
            .iter()
            .zip(refaced_run.iter())
            .any(|(a, b)| a != b));
    }

    #[test]
    fn loc2_hitstop_freezes_all_motion() {
        let config = short_config();
        let bindings = Bindings::new();
        let attack = bindings.button(BoundAction::Attack);
        let mut inputs = Vec::new();
        for tick in 0..config.ticks {
            let pressed0: &[PhysicalButton] = if tick == 0 { &[attack] } else { &[] };
            inputs.push(human_input(&bindings, tick, 0, pressed0, (100, 0), 0));
            inputs.push(human_input(&bindings, tick, 1, &[], (0, 100), 0));
        }
        let tape = FighterTape(inputs);
        assert_eq!(hit_ticks(&run_events(&config, &tape)), vec![4]);
        let trace = run_trace(&config, &tape).expect("valid trace");
        assert_eq!(trace.len(), config.ticks as usize);
        // Both fighters drive into the impact tick; the interrupt zeroes
        // their requested intent on that tick and hit-stop holds the
        // resolved deltas at zero even though tick 3 asked to move.
        for (slot, (before, impact)) in trace[3].iter().zip(trace[4].iter()).enumerate() {
            assert_ne!(
                before.requested,
                [0.0, 0.0],
                "slot {slot} drives into impact"
            );
            assert_eq!(
                impact.requested,
                [0.0, 0.0],
                "interrupt zeroes slot {slot} intent on impact"
            );
            assert_eq!(impact.resolved, [0.0, 0.0], "hit-stop freezes slot {slot}");
        }
        // The combat hit-stop window stays frozen for the driving victim.
        for (tick, row) in trace.iter().enumerate().skip(5).take(3) {
            assert_eq!(row[1].resolved, [0.0, 0.0], "hit-stop holds tick {tick}");
        }
        // Pre-impact motion was real: the Start ramp already moves.
        assert!(
            trace[2][1].resolved[1] > 0.0,
            "victim advances before impact: {:?}",
            trace[2][1]
        );
        // Disarming the attack diverges and lets the victim through.
        let mut disarmed = tape.clone();
        disarmed.0[0].buttons = Buttons::NONE;
        let disarmed_run = run(&config, &disarmed);
        assert!(run(&config, &tape)
            .iter()
            .zip(disarmed_run.iter())
            .any(|(a, b)| a != b));
        let free = run_trace(&config, &disarmed).expect("valid trace");
        assert!(
            free[4][1].resolved[1] > 0.0,
            "no interrupt without the hit: {:?}",
            free[4][1]
        );
    }

    #[test]
    fn loc2_blocked_delta_never_reapplied() {
        let config = ArenaConfig {
            seed: 7,
            ticks: 400,
            fighters: 2,
            half_extent_m: 5.0,
            table_version: TABLES_VERSION_V1,
        };
        let mut inputs = Vec::new();
        for tick in 0..config.ticks {
            inputs.push(tape_input(tick, 0, 100, 100));
            inputs.push(tape_input(tick, 1, -100, -100));
        }
        let tape = FighterTape(inputs);
        let trace = run_trace(&config, &tape).expect("valid trace");
        assert_eq!(trace.len(), config.ticks as usize);
        let mut blocked_ticks = 0u32;
        for (tick, row) in trace.iter().enumerate() {
            for (slot, motion) in row.iter().enumerate() {
                assert!(motion.requested.iter().all(|v| v.is_finite()));
                assert!(motion.resolved.iter().all(|v| v.is_finite()));
                let req = motion.requested[0] * motion.requested[0]
                    + motion.requested[1] * motion.requested[1];
                let res = motion.resolved[0] * motion.resolved[0]
                    + motion.resolved[1] * motion.resolved[1];
                assert!(
                    res <= req + 1e-6,
                    "tick {tick} slot {slot}: resolved {res} exceeds requested {req}"
                );
                if motion.blocked {
                    blocked_ticks += 1;
                    assert!(
                        res < req - 1e-9,
                        "blocked tick {tick} slot {slot} must shorten the delta"
                    );
                }
            }
        }
        assert!(blocked_ticks > 0, "the wall drive must block");
        assert_eq!(run(&config, &tape), run(&config, &tape));
        let limit = config.half_extent_m - FIGHTER_RADIUS_M;
        for state in final_states(&config, &tape) {
            assert!(state.pos[0] >= -limit && state.pos[0] <= limit);
            assert!(state.pos[2] >= -limit && state.pos[2] <= limit);
        }
    }

    #[test]
    fn loc2_snapshot_restore_resimulates_identical() {
        let config = loc2_course_config();
        let tape = loc2_course_tape();
        let full = run(&config, &tape);
        assert_eq!(full.len(), config.ticks as usize);
        // Prefix crosses move/stop/turn/strafe plus locomotion transients;
        // the suffix crosses the hit-stop impact.
        let (snapshot, prefix) = run_prefix(&config, &tape, 60).expect("valid prefix");
        assert_eq!(prefix.len(), 60);
        let suffix = resume(snapshot, &config, &tape);
        assert_eq!(suffix.len(), config.ticks as usize - 60);
        let rejoined: Vec<Checkpoint> = prefix.into_iter().chain(suffix).collect();
        assert_eq!(rejoined, full);
    }

    #[test]
    fn loc2_bindings_wishes_drive_the_match() {
        let config = short_config();
        let bindings = Bindings::new();
        let fwd = bindings.button(BoundAction::MoveForward);
        let back = bindings.button(BoundAction::MoveBack);
        // Same pressed buttons twice: identical checkpoints (fixed-tick
        // wishes only; no camera or frame-time input anywhere).
        let tape_for = |pressed: &[PhysicalButton]| {
            let mut inputs = Vec::new();
            for tick in 0..config.ticks {
                let (mx, mz) = bindings.wish_from_pressed(pressed);
                inputs.push(human_input(&bindings, tick, 0, pressed, (mx, mz), 0));
                inputs.push(human_input(&bindings, tick, 1, &[], (0, 0), 0));
            }
            FighterTape(inputs)
        };
        let fwd_tape = tape_for(&[fwd]);
        assert_eq!(run(&config, &fwd_tape), run(&config, &fwd_tape));
        // Opposed wish diverges; opposed pairs cancel to idle by contract.
        let back_tape = tape_for(&[back]);
        let back_run = run(&config, &back_tape);
        assert!(run(&config, &fwd_tape)
            .iter()
            .zip(back_run.iter())
            .any(|(a, b)| a != b));
        assert_eq!(bindings.wish_from_pressed(&[fwd]), (100, 0));
        assert_eq!(bindings.wish_from_pressed(&[fwd, back]), (0, 0));
    }

    /// LOC1+PH2 wiring (`sim` only): locomotion wishes from IN1 pressed
    /// buttons drive one controller over a ramp/step course with exactly one
    /// `move_and_slide` per tick, plus a jump for the air leg. The same
    /// script twice is identical, and snapshot/restore resimulates
    /// bit-identical.
    #[cfg(feature = "sim")]
    #[test]
    fn loc2_wired_slope_step_air_course_x2() {
        use engine_physics::controller::Controller;

        fn course() -> Controller {
            // Floor, shallow ramp, step ledge and deck sized for the arena
            // capsule (radius 0.4): the step face stays within the 0.5 m
            // gate once the ramp has lifted the feet (overlap starts at the
            // ramp point where the face first fits the gate).
            let mut controller = Controller::spawn(
                [-2.0, CAPSULE_REST_Y_M, 0.0],
                FIGHTER_RADIUS_M,
                CAPSULE_CYL_H_M,
            )
            .expect("spawn");
            controller
                .add_static_box([10.0, 0.5, 3.0], [0.0, -0.5, 0.0])
                .expect("floor");
            controller
                .add_ramp(2.0, 4.0, 0.0, 0.8, 3.0, 0.0)
                .expect("shallow ramp");
            controller
                .add_static_box([0.5, 0.55, 3.0], [4.5, 0.55, 0.0])
                .expect("step");
            controller
                .add_static_box([2.0, 0.55, 3.0], [7.0, 0.55, 0.0])
                .expect("deck");
            controller.move_and_slide([0.0, 0.0], FIXED_DT);
            controller
        }

        let bindings = Bindings::new();
        let fwd = bindings.button(BoundAction::MoveForward);
        let skeleton = loco_skeleton();
        // One wired tick: IN1 wish into LOC1, requested root delta through
        // PH2 exactly once, airborne mirroring the controller.
        let step_wired = |loco: &mut Locomotion, ctrl: &mut Controller, tick: u32| {
            let (mx, mz) = bindings.wish_from_pressed(&[fwd]);
            let input = human_input(&bindings, tick, 0, &[fwd], (mx, mz), 0);
            let (speed, dir) = loco_wish(&input);
            loco.set_airborne(!ctrl.grounded());
            let out = loco
                .update(speed, dir, tick, None)
                .expect("course wish valid");
            validate_pose(&skeleton, &out.pose).expect("pose valid on the course");
            ctrl.move_and_slide(out.root.dp, FIXED_DT)
        };

        // Slope/step leg: 240 driving ticks cross ramp plus step and stop
        // mid-deck (the lateral wish holds strafe, capped at 2 m/s; longer
        // drives walk off the deck end).
        let drive_course = |ctrl: &mut Controller| {
            let mut loco = Locomotion::new();
            let mut stepped = false;
            for tick in 0..240u32 {
                let resolved = step_wired(&mut loco, ctrl, tick);
                stepped = stepped || resolved.stepped;
            }
            (ctrl.state_hash(), ctrl.position(), stepped)
        };
        let mut first = course();
        let run_a = drive_course(&mut first);
        let mut second = course();
        let run_b = drive_course(&mut second);
        assert_eq!(run_a, run_b);
        assert!(run_a.2, "course must autostep at least once");
        assert!(
            run_a.1[0] > 5.0 && run_a.1[0] < 9.0,
            "deck span: {:?}",
            run_a.1
        );
        assert!(
            (run_a.1[1] - (1.1 + CAPSULE_REST_Y_M)).abs() < 0.05,
            "deck height: {:?}",
            run_a.1
        );

        // Air leg: jump from the deck, limited air control, land grounded.
        let mut loco = Locomotion::new();
        first.jump(6.0);
        let mut ever_air = false;
        let mut saw_loco_air = false;
        for tick in 240..360u32 {
            let resolved = step_wired(&mut loco, &mut first, tick);
            if !resolved.grounded {
                ever_air = true;
            }
            saw_loco_air = saw_loco_air || loco.state == LocoState::Air;
            assert!(resolved.requested.iter().all(|v| v.is_finite()));
            let req = resolved.requested[0] * resolved.requested[0]
                + resolved.requested[1] * resolved.requested[1];
            let res = resolved.displacement[0] * resolved.displacement[0]
                + resolved.displacement[1] * resolved.displacement[1];
            assert!(res <= req + 1e-6, "tick {tick}: air never adds motion");
        }
        assert!(ever_air, "jump must leave the ground");
        assert!(saw_loco_air, "locomotion must mirror the airborne leg");
        assert!(first.grounded(), "must land again after the jump");

        // Snapshot leg: restore at tick 20, resimulate 20..40 bit-identical
        // across the LOC1+PH2 seam (controller plus locomotion machine).
        let mut ctrl = course();
        let mut loco = Locomotion::new();
        for tick in 0..20u32 {
            step_wired(&mut loco, &mut ctrl, tick);
        }
        let (loco_snap, ctrl_snap) = (loco.clone(), ctrl.snapshot());
        for tick in 20..40u32 {
            step_wired(&mut loco, &mut ctrl, tick);
        }
        let (hash_a, pos_a, loco_a) = (ctrl.state_hash(), ctrl.position(), loco.clone());
        ctrl.restore(ctrl_snap).expect("restore");
        loco = loco_snap;
        for tick in 20..40u32 {
            step_wired(&mut loco, &mut ctrl, tick);
        }
        assert_eq!(
            (ctrl.state_hash(), ctrl.position(), loco),
            (hash_a, pos_a, loco_a)
        );
    }

    // ---- Steady-state allocation gate -----------------------------------

    /// Ticks of warmup before the counting window opens.
    const STEADY_WARMUP_TICKS: u32 = 10;
    /// Measured ticks inside the counting window.
    const STEADY_MEASURE_TICKS: usize = 600;

    /// Idle config for one steady workload: every slot human-idle every
    /// tick, so no brain ever decides, no combat event fires and no sword
    /// hit ever reaches the prop. The window then observes the pure
    /// per-tick floor: locomotion presentation poses plus contacts and the
    /// destruction support refresh. Any arena-scaffold allocation per tick
    /// (the old input `Vec` cost exactly one) moves this floor.
    fn steady_config(fighters: u8) -> ArenaConfig {
        ArenaConfig {
            seed: 1,
            ticks: STEADY_WARMUP_TICKS + STEADY_MEASURE_TICKS as u32,
            fighters,
            half_extent_m: 20.0,
            table_version: TABLES_VERSION_V1,
        }
    }

    /// Heap allocations over `STEADY_MEASURE_TICKS` steady ticks after
    /// warmup, plus the final checkpoint hash. Builds and warms outside
    /// the window; only the steady slice counts. `live` arms the match
    /// observer call-site (`match_live`); the idle tape keeps its rows
    /// empty so the reading covers the quiet path on both legs.
    fn steady_allocs_and_hash(fighters: u8, live: bool) -> (usize, u64) {
        let config = steady_config(fighters);
        let tape = idle_tape(&config);
        let mut sim = Sim::build(&config, &tape, false, live).expect("valid steady sim");
        for _ in 0..STEADY_WARMUP_TICKS {
            sim.step(&tape).expect("valid warmup step");
        }
        let mut hash = 0u64;
        let ((), measured) = engine_core::alloc_probe::measure(|| {
            for _ in 0..STEADY_MEASURE_TICKS {
                sim.step(&tape).expect("valid steady step");
            }
            hash = sim
                .checkpoints
                .last()
                .expect("steady slice checkpoints")
                .hash;
            std::hint::black_box(hash);
        });
        let allocs = usize::try_from(measured.calls).expect("steady alloc count fits");
        (allocs, hash)
    }

    #[test]
    fn alloc_probe_sees_a_real_allocation() {
        // Calibration: a blind counter must not fake the readings below.
        let ((), seen) = engine_core::alloc_probe::measure(|| {
            std::hint::black_box(vec![0u8; 64]);
        });
        assert!(seen.calls >= 1, "alloc probe is blind");
    }

    #[test]
    fn steady_tick_alloc_floor_4_fighter() {
        // Floor decomposition per tick after the buffer-reuse landings,
        // measured with the probe above: locomotion root advance 0
        // (`update_root`), destruction refresh 0 (scratch reuse), combat
        // quiet ticks 0 (reserved scratch), arena scaffold 0 — leaving
        // only the test-only motion-trace scratch (production builds
        // skip it, so the production floor is 0/tick). Fails on any
        // per-tick `Vec` regression; recount toward zero, never away.
        const PER_TICK_FLOOR: usize = 1;
        let (first, hash_first) = steady_allocs_and_hash(4, false);
        let (second, hash_second) = steady_allocs_and_hash(4, false);
        assert_eq!(hash_first, hash_second, "steady slices hash identically");
        assert_eq!(first, second, "steady slices allocate identically");
        assert_eq!(
            first,
            PER_TICK_FLOOR * STEADY_MEASURE_TICKS,
            "4-fighter steady floor moved: recount (arena scaffold must stay 0/tick)"
        );
    }

    #[test]
    fn steady_tick_alloc_scales_8_fighter() {
        // Scaled workload beside the 4-fighter one: four more fighters add
        // nothing on the authority path (root advance is alloc-free at any
        // scale), proving per-fighter cost is flat zero. Test floor is the
        // trace scratch only (1/tick); production floor is 0/tick.
        const PER_TICK_FLOOR: usize = 1;
        let (four, _) = steady_allocs_and_hash(4, false);
        let (eight, hash_eight) = steady_allocs_and_hash(8, false);
        let (eight_again, hash_again) = steady_allocs_and_hash(8, false);
        assert_eq!(hash_eight, hash_again, "8-fighter runs hash identically");
        assert_eq!(eight, eight_again, "8-fighter runs allocate identically");
        assert_eq!(
            eight,
            PER_TICK_FLOOR * STEADY_MEASURE_TICKS,
            "8-fighter steady floor moved: recount (fixed overhead must stay flat)"
        );
        assert_eq!(
            eight - four,
            0,
            "four more fighters must add zero allocs on the authority path"
        );
    }

    #[test]
    fn live_observer_adds_zero_heap_on_quiet_ticks() {
        // The owned match rows must cost zero heap on quiet ticks: the
        // observer call-site runs every live tick, but empty rows move
        // nothing, so the live floor equals the cold floor (the test-only
        // trace scratch; production 0/tick). Event ticks keep their
        // bounded owned rows by design (one exact-size `Vec` per row), so
        // this pins the quiet path and any per-tick staging regression
        // fails here. Control: the idle tape leaves every observed row
        // empty, proving the reading covers the quiet path.
        const PER_TICK_FLOOR: usize = 1;
        let config = steady_config(4);
        let tape = idle_tape(&config);
        let rows = run_match_events(&config, &tape);
        assert_eq!(rows.len(), config.ticks as usize, "one row per tick");
        assert!(
            rows.iter()
                .all(|row| row.exchange.is_empty() && row.sweep.is_empty()),
            "idle tape must stay quiet through the live observer"
        );
        let (cold, cold_hash) = steady_allocs_and_hash(4, false);
        let (live, live_hash) = steady_allocs_and_hash(4, true);
        let (live_again, live_hash_again) = steady_allocs_and_hash(4, true);
        assert_eq!(cold_hash, live_hash, "gating never touches the hash");
        assert_eq!(live_hash, live_hash_again, "live slices hash identically");
        assert_eq!(live, live_again, "live slices allocate identically");
        assert_eq!(
            live,
            PER_TICK_FLOOR * STEADY_MEASURE_TICKS,
            "live quiet floor moved: the observer must add 0 heap per quiet tick"
        );
        assert_eq!(
            live, cold,
            "live observer must cost zero heap over cold on quiet ticks"
        );
    }

    fn vec_env(seed: u64, ticks: u32) -> BatchEnv {
        let config = ArenaConfig {
            seed,
            ticks,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: TABLES_VERSION_V1,
        };
        let mut inputs = Vec::new();
        for tick in 0..ticks {
            for fighter in 0..2 {
                inputs.push(FighterInput {
                    tick,
                    fighter: FighterId::new(fighter).expect("slots 0..2 are legal"),
                    mx: if fighter == 0 { 60 } else { -60 },
                    mz: 0,
                    facing: 0,
                    buttons: Buttons::NONE,
                });
            }
        }
        inputs.sort_by_key(|input| (input.tick, input.fighter.0));
        BatchEnv {
            config,
            tape: FighterTape(inputs),
        }
    }

    #[test]
    fn arena_config_stays_copy_send_sync() {
        const fn assert_copy_send_sync<T: Copy + Send + Sync>() {}
        assert_copy_send_sync::<ArenaConfig>();
        let config = config();
        let (first, second) = (config, config);
        assert_eq!(first, second);
    }

    #[test]
    fn vec_parity_batch_matches_single_steps() {
        // Eight envs: batch observations equal eight single-step
        // observations, and per-env hashes replay identically.
        let envs: Vec<BatchEnv> = (0..8).map(|index| vec_env(2040 + index, 64)).collect();
        let batched = vec_step(&envs);
        assert_eq!(batched.len(), 8);
        let mut single = Vec::with_capacity(8);
        for env in &envs {
            single.push(observe_state(&env.config, &env.tape));
        }
        assert_eq!(batched, single, "batch obs must equal 8x single-step obs");
        for snap in &batched {
            assert!(snap.is_some(), "every fixture env must simulate");
        }
        let again = vec_step(&envs);
        assert_eq!(again, batched, "same seeds plus tapes replay identically");
        let first_hashes: Vec<u64> = batched
            .iter()
            .map(|snap| snap.as_ref().map_or(0, |shot| shot.digest))
            .collect();
        let second_hashes: Vec<u64> = again
            .iter()
            .map(|snap| snap.as_ref().map_or(0, |shot| shot.digest))
            .collect();
        assert_eq!(first_hashes, second_hashes);
        assert!(
            first_hashes.iter().all(|hash| *hash != 0),
            "64 ticks must move every digest"
        );
    }

    #[test]
    fn vec_step_keeps_input_order_and_isolates_bad_envs() {
        // A bad env yields None at its index; neighbors still simulate,
        // and input order (never completion order) is preserved.
        let mut envs: Vec<BatchEnv> = (0..4).map(|index| vec_env(2040 + index, 32)).collect();
        envs.insert(
            1,
            BatchEnv {
                config: ArenaConfig {
                    seed: 999,
                    ticks: 0,
                    fighters: 2,
                    half_extent_m: 20.0,
                    table_version: TABLES_VERSION_V1,
                },
                tape: FighterTape::default(),
            },
        );
        let batched = vec_step(&envs);
        assert_eq!(batched.len(), 5);
        assert!(batched[1].is_none(), "ticks=0 must refuse at its index");
        for (index, snap) in batched.iter().enumerate() {
            if index != 1 {
                assert!(snap.is_some(), "env {index} must still simulate");
            }
        }
        assert!(vec_step(&[]).is_empty(), "empty batch stays empty");
    }

    #[test]
    fn vec_step_uneven_batches_match_single_steps_slot_for_slot() {
        // Batch sizes that do not divide by the worker count put the chunk
        // boundaries in different places; every slot must still hold its
        // own env's observation, including a refusal in the last chunk.
        for count in [1usize, 3, 7, 13] {
            let mut envs: Vec<BatchEnv> = (0..count as u64)
                .map(|index| vec_env(77 + index, 8))
                .collect();
            envs[count - 1].config.ticks = 0;
            let batched = vec_step(&envs);
            let single: Vec<_> = envs
                .iter()
                .map(|env| observe_state(&env.config, &env.tape))
                .collect();
            assert_eq!(batched, single, "batch of {count} must equal single steps");
            assert!(
                batched[count - 1].is_none(),
                "the refusal stays at its index"
            );
        }
    }

    /// EG46 helper: feed `hits` landed blows to slot 1 under v2 rules and
    /// fold the v2 digest. Hits land back to back inside the memory window,
    /// so pain has no tick to shed between blows.
    fn stagger_hash_digest(hits: usize) -> (GaitPhase, u64) {
        let config = short_config();
        let tape = idle_tape(&config);
        let mut sim = Sim::build(&config, &tape, false, false).expect("valid sim");
        assert!(
            sim.set_rules(ArenaRules::v2_free_for_all()),
            "v2 pins before tick 0"
        );
        let events = vec![
            FightEvent::Hit {
                attacker: 0,
                victim: 1,
                damage: ATTACK_DAMAGE,
            };
            hits
        ];
        sim.feed_balance(&events).expect("bounded impulses");
        let gait = sim.balance(1).expect("victim slot").gait();
        let digest = sim.rules.fold(
            0x9e37_79b9_7f4a_7c15,
            sim.fighters,
            &sim.balance,
            &sim.stagger,
            &sim.limbs,
        );
        (gait, digest)
    }

    #[test]
    fn stagger_hash_three_hits_floor_and_repeat_x2() {
        let (gait, first) = stagger_hash_digest(3);
        assert_eq!(
            gait,
            GaitPhase::Fallen,
            "three blows inside the window floor the victim"
        );
        let (repeat_gait, second) = stagger_hash_digest(3);
        assert_eq!(
            repeat_gait,
            GaitPhase::Fallen,
            "the repeat run floors the victim too"
        );
        assert_eq!(first, second, "same three hits hash twice");
        assert_ne!(first, 0x9e37_79b9_7f4a_7c15, "v2 fold must move the base");
    }

    #[test]
    fn stagger_hash_differs_from_two_hit_run() {
        let (_, two) = stagger_hash_digest(2);
        let (gait_three, three) = stagger_hash_digest(3);
        assert_eq!(
            gait_three,
            GaitPhase::Fallen,
            "three blows floor the victim"
        );
        assert_ne!(
            two, three,
            "extra blow inside the window must move the digest"
        );
    }

    #[test]
    fn stagger_hash_v1_frozen_and_bad_slot_none() {
        let config = short_config();
        let tape = idle_tape(&config);
        let mut sim = Sim::build(&config, &tape, false, false).expect("valid sim");
        let base = 0x9e37_79b9_7f4a_7c15;
        let frozen = sim
            .rules
            .fold(base, sim.fighters, &sim.balance, &sim.stagger, &sim.limbs);
        assert_eq!(frozen, base, "v1 leaves the hash untouched");
        assert!(sim.balance(99).is_none(), "bad slot reads None");
        let bad = ArenaRules {
            version: 999,
            ..ArenaRules::V1
        };
        assert!(!sim.set_rules(bad), "unknown version refuses");
        assert_eq!(sim.rules.version, FIGHT_RULES_V1, "refusal keeps v1");
    }

    /// EG46b helper: fold the v2 digest with an optional scripted cut.
    /// The cut uses swing 100 (the tick-100 sever) on joint 3, so the pool
    /// holds one limb in the cut run and none in the uncut run.
    fn limb_hash_digest(with_cut: bool) -> u64 {
        use engine_combat::Cut;
        let config = short_config();
        let tape = idle_tape(&config);
        let mut sim = Sim::build(&config, &tape, false, false).expect("valid sim");
        assert!(
            sim.set_rules(ArenaRules::v2_free_for_all()),
            "v2 pins before tick 0"
        );
        if with_cut {
            let cut = Cut {
                swing: 100,
                attacker: 2,
                victim: 1,
                joint: 3,
                flags: 0,
                energy_q16: 600_000,
                protection_q16: 589_824,
            };
            crate::limbs::spawn_limbs(&mut sim.limbs, &[cut], 0).expect("valid cut");
            sim.limbs.step();
        }
        sim.rules.fold(
            0x9e37_79b9_7f4a_7c15,
            sim.fighters,
            &sim.balance,
            &sim.stagger,
            &sim.limbs,
        )
    }

    #[test]
    fn limb_hash_cut_repeats_x2() {
        let first = limb_hash_digest(true);
        let second = limb_hash_digest(true);
        assert_eq!(first, second, "same cut hashes twice");
        assert_ne!(
            first, 0x9e37_79b9_7f4a_7c15,
            "v2 limb fold must move the base"
        );
    }

    #[test]
    fn limb_hash_differs_from_uncut_run() {
        let uncut = limb_hash_digest(false);
        let cut = limb_hash_digest(true);
        assert_ne!(cut, uncut, "a severed limb must move the digest");
    }

    /// v2 bout fixture for the timing receipts: 8 fighters, 600 ticks, v2
    /// tables, so [`ArenaConfig::rules`] pins rules v2 at build.
    fn v2_bout_config() -> ArenaConfig {
        ArenaConfig {
            seed: 2040,
            ticks: 600,
            fighters: 8,
            half_extent_m: 20.0,
            table_version: TABLES_VERSION_V2,
        }
    }

    /// Microseconds of one `span` that covered `reps` calls.
    fn micros_per(span: std::time::Duration, reps: usize) -> f64 {
        span.as_secs_f64() * 1.0e6 / f64::from(u32::try_from(reps).unwrap_or(1))
    }

    /// Decomposition receipt: the p50 microseconds of one rules-v2 8-fighter
    /// tick's whole step, then of the pieces it spends its time in
    /// ([`engage::Plan::build`], the destruction digest, [`ArenaRules::fold`],
    /// [`collide::collide_states`], the eight [`strike_pose`] builds, and the
    /// fold's own sub-hashes). Every piece runs on the poses one warmed real
    /// bout holds, interleaved so one machine load sits under each. Run:
    /// cargo test -p engine-arena --release --lib v2_tick_decomposition_receipt
    /// -- --ignored --nocapture
    #[test]
    #[ignore = "timing: run with --ignored (release, one thread) for the receipt"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn v2_tick_decomposition_receipt() {
        const SAMPLES: usize = 101;
        const BATCH: usize = 64;
        const TICK_REPS: usize = 11;
        let config = v2_bout_config();
        let tape = FighterTape::default();
        let mut warm = Sim::build(&config, &tape, false, false).expect("warm sim");
        for _ in 0..120 {
            warm.step(&tape).expect("warm step");
        }
        let states = warm.states.clone();
        let fight = warm.fight;
        let teams = warm.rules.teams;
        let count = warm.fighters;
        let rows: [FighterInput; MAX_SLOTS] = std::array::from_fn(|slot| FighterInput {
            tick: 0,
            fighter: FighterId(u8::try_from(slot).unwrap_or(0)),
            mx: 60,
            mz: 0,
            facing: 0,
            buttons: Buttons(ATTACK),
        });
        let mut scratch = states.clone();
        let base = 0x9e37_79b9_7f4a_7c15;
        let mut columns: [Vec<f64>; 8] = std::array::from_fn(|_| Vec::with_capacity(SAMPLES));
        for _ in 0..SAMPLES {
            let begin = std::time::Instant::now();
            for _ in 0..BATCH {
                std::hint::black_box(engage::Plan::build(&states, &fight, &teams, count));
            }
            columns[0].push(micros_per(begin.elapsed(), BATCH));
            let begin = std::time::Instant::now();
            for _ in 0..BATCH {
                std::hint::black_box(warm.destruction.state_hash());
            }
            columns[1].push(micros_per(begin.elapsed(), BATCH));
            let begin = std::time::Instant::now();
            for _ in 0..BATCH {
                std::hint::black_box(warm.rules.fold(
                    base,
                    count,
                    &warm.balance,
                    &warm.stagger,
                    &warm.limbs,
                ));
            }
            columns[2].push(micros_per(begin.elapsed(), BATCH));
            let begin = std::time::Instant::now();
            for _ in 0..BATCH {
                scratch.copy_from_slice(&states);
                collide::collide_states(&mut scratch, &warm.rules, config.half_extent_m, count);
                std::hint::black_box(&scratch);
            }
            columns[3].push(micros_per(begin.elapsed(), BATCH));
            let begin = std::time::Instant::now();
            for _ in 0..BATCH {
                for slot in 0..count {
                    std::hint::black_box(strike_pose(
                        u8::try_from(slot).unwrap_or(0),
                        &states[slot],
                        &rows[slot],
                        teams[slot],
                    ));
                }
            }
            columns[4].push(micros_per(begin.elapsed(), BATCH));
            let begin = std::time::Instant::now();
            for _ in 0..BATCH {
                let mut sub = 0u64;
                for state in &warm.balance {
                    sub ^= state.state_hash();
                }
                std::hint::black_box(sub);
            }
            columns[5].push(micros_per(begin.elapsed(), BATCH));
            let begin = std::time::Instant::now();
            for _ in 0..BATCH {
                let mut sub = 0u64;
                for memory in &warm.stagger {
                    sub ^= memory.state_hash();
                }
                std::hint::black_box(sub);
            }
            columns[6].push(micros_per(begin.elapsed(), BATCH));
            let begin = std::time::Instant::now();
            for _ in 0..BATCH {
                std::hint::black_box(warm.limbs.state_hash());
            }
            columns[7].push(micros_per(begin.elapsed(), BATCH));
        }
        let names = [
            "plan_build",
            "destruction_hash",
            "rules_fold",
            "collide_states",
            "strike_pose_x8",
            "fold_balance_x8",
            "fold_stagger_x8",
            "fold_limbs",
        ];
        for (name, column) in names.iter().zip(columns.iter_mut()) {
            column.sort_by(|a, b| a.partial_cmp(b).expect("finite samples"));
            eprintln!("{name}_p50_us={:.4}", column[SAMPLES / 2]);
        }
        let mut ticks = Vec::with_capacity(TICK_REPS);
        for _ in 0..TICK_REPS {
            let begin = std::time::Instant::now();
            let mut sim = Sim::build(&config, &tape, false, false).expect("timed sim");
            while sim.tick < config.ticks {
                sim.step(&tape).expect("timed step");
            }
            ticks.push(begin.elapsed().as_secs_f64() * 1.0e6 / f64::from(config.ticks));
        }
        ticks.sort_by(|a, b| a.partial_cmp(b).expect("finite samples"));
        eprintln!(
            "whole_tick_p50_us={:.4} fighters={count} ticks={} limbs={}",
            ticks[TICK_REPS / 2],
            config.ticks,
            warm.limbs.limbs().len()
        );
    }
}
