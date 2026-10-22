//! fork: counterfactual match forks (FK1, AI-NATIVE pillar 3).
//!
//! Consumer: playtest/balance agents and (later) MCP `sim.fork`: snapshot a
//! match, apply a proposal to the fork, run N ticks, read metrics. Data
//! owner: [`Fork`] owns a snapshot-clone of the match (states, combat,
//! match clocks, brains, locomotion, the bonded destruction prop, plus controller
//! snapshots under `sim`); the live
//! match shares nothing with it. Units: fixed ticks, meters, permille.
//! Determinism scope: same-machine reproducibility — same snapshot plus same
//! proposals plus same `run_n` length gives identical [`ForkMetrics`]; the
//! fork worker reuses the exact arena step path, and under
//! `sim` it rebuilds controllers from the stored snapshots and restores them
//! before the first tick, so two identical forks stay bit-identical (a fork
//! is not guaranteed identical to the uninterrupted run under `sim`, where
//! Rapier pipeline-internal contact state is not part of the public
//! snapshot). Error policy: invalid proposals fail as [`ForkError`], a tick
//! mismatch in [`Fork::from_snapshot`] is `None`; the worker never panics.
//!
//! Threading contract: [`Fork::run_n`] clones the fork state into one job on
//! the shared job pool (`engine_ecs::pool::shared()`, E21) and waits for it
//! before returning. The live match is
//! never touched (snapshot-clone isolation: no shared locks, no shared
//! mutable state), so the fixed tick never waits on a fork; only the calling
//! thread blocks on its own wait. Concurrent forks are independent. A 0-tick
//! fork returns immediately with zeroed metrics.
//!
//! Ship configuration: NET1/LOC2-style consumers enable `engine-arena/sim`
//! (physics settling plus wall collision inside the fork worker). Fork tests
//! run in both legs, but `sim` is the shipped configuration.

#[cfg(feature = "sim")]
use super::build_controllers;
use super::recorder::{on_fork_step, Dataset, Row, StepRecord, DATASET_VERSION};
use super::Sim;
use super::{
    ArenaConfig, FighterState, MatchSnapshot, MATCH_WIRE_VERSION_V0, MATCH_WIRE_VERSION_V1,
    MAX_SLOTS, TICK_HZ,
};
use engine_combat::depth::DepthSet;
use engine_combat::exchange::PhasedFighter;
use engine_combat::fight::MAX_ATTACK_STARTED_PER_TICK;
use engine_combat::FightEvent;
use engine_combat::MAX_EVENTS_PER_TICK;
use engine_core::Fnv64;
use engine_destruction::bonded::BondedCollection;
use engine_input::{FighterError, FighterId, FighterInput, FighterTape};
#[cfg(feature = "sim")]
use engine_physics::controller::ControllerSnapshot;
use engine_physics::limb::{LimbPool, MAX_LIMBS};

/// The shipped duel tiers, re-exported from `engine_ai::tier` (single owner
/// of the numbers) so a balance consumer outside the AI crate installs the
/// one shipped table instead of copying its rows.
pub use engine_ai::tier::{TierRow, SHIPPED_TIERS, TIER_DUELS, WIN_BAR_PERMILLE};

/// Planar speed below which a fighter counts as stuck, meters per second.
pub const STUCK_SPEED_MPS: f32 = 0.2;

/// Fighter count as a slot byte for zeroed metrics.
/// Valid counts are `2..=MAX_PARTICIPANTS` (32), so every live value fits
/// `u8` exactly; out-of-range counts saturate at `u8::MAX` instead of
/// truncating low bits, keeping zeroed-metric length bounded.
fn fighter_slot_count(fighters: usize) -> u8 {
    u8::try_from(fighters).unwrap_or(u8::MAX)
}

/// Counterfactual proposal applied to a fork before it runs. Minimal set:
/// morale intent, a duel tier, script replacement and a freeze. There is
/// deliberately no `RemoveFighter`: the slot count is fixed by the snapshot
/// config.
#[derive(Clone, PartialEq, Debug)]
pub enum ForkProposal {
    /// Set one brain's morale (`0..=1000`, 500 neutral): high morale attacks
    /// and advances, low morale blocks, dodges and retreats. The damage-intent
    /// analogue for balance probes.
    SetAggression {
        /// Fighter slot, `0..config.fighters`.
        fighter: u8,
        /// Morale in permille, `0..=1000`.
        permille: u16,
    },
    /// Install one shipped duel tier on a brain: its role pins the policy and
    /// its reaction plus aggression shape the temperament, so a balance
    /// difficulty becomes a skill rung, never a stat wall or hidden info.
    SetTier {
        /// Fighter slot, `0..config.fighters`.
        fighter: u8,
        /// The tier row to install; single owner is `engine_ai::tier`.
        tier: TierRow,
    },
    /// Replace the fork tape slice for one fighter. Entries run on absolute
    /// match ticks; gaps are AI-filled like the real tick.
    SwapTape {
        /// Fighter slot, `0..config.fighters`.
        fighter: u8,
        /// Replacement inputs for that fighter, tick-sorted, unique ticks.
        inputs: Vec<FighterInput>,
    },
    /// Freeze one fighter: every fork tick forces an idle wish with no
    /// buttons, overriding tape and brain. The frozen-foe analogue.
    FreezeFighter {
        /// Fighter slot, `0..config.fighters`.
        fighter: u8,
    },
}

/// Typed fork failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ForkError {
    /// Fighter slot outside `0..config.fighters`, culprit carried.
    UnknownFighter(u8),
    /// Morale outside `0..=1000`, culprit carried.
    BadPermille(u16),
    /// One replacement input failed validation, source carried.
    BadTape(FighterError),
    /// Replacement input names a different fighter than the proposal slot.
    MisfiledTape {
        /// Proposal slot.
        want: u8,
        /// Input's actual slot.
        got: u8,
    },
    /// Replacement tick outside the fork window
    /// (`base_tick..config.ticks`), culprit carried.
    TickOutOfRange(u32),
    /// Replacement slice not sorted by tick, index carried.
    UnsortedTape(usize),
    /// Same tick twice in the replacement slice, index carried.
    DuplicateInput(usize),
}

/// A fork whose worker thread panicked. No metrics come back with it: the
/// panic is a crash, never a zeroed run that reads as a clean one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ForkCrashed;

impl std::fmt::Display for ForkCrashed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("fork worker panicked; no metrics")
    }
}

impl std::error::Error for ForkCrashed {}

/// Fork outcome metrics, in fighter-slot order.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ForkMetrics {
    /// Total damage dealt by each fighter in permille (hits plus block chip,
    /// saturating), one `(slot, permille)` entry per fighter.
    pub damage_dealt: Vec<(u8, u16)>,
    /// Fork-relative ticks elapsed until the first kill (1-based count:
    /// a kill on the first fork tick reports `Some(1)`), `None` when nobody
    /// dropped to zero hp inside the window.
    pub ticks_to_kill: Option<u32>,
    /// Ticks each fighter spent stuck (planar speed below
    /// [`STUCK_SPEED_MPS`]), one `(slot, ticks)` entry per fighter.
    /// Knocked-out fighters hold pose, so they read as stuck.
    pub stuck_ticks: Vec<(u8, u32)>,
}

/// Which fold of a forked tick moved first. Checked in this order at one tick,
/// so the earliest cause wins: a pose change is reported over the combat
/// change it causes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ForkComponent {
    /// Fighter ids, feet positions and yaw plus the tick's buttons.
    Poses,
    /// Combat state (hp, stamina, action clocks) plus the tick's events.
    Combat,
    /// The bonded destruction prop.
    Destruction,
    /// The whole-row checkpoint hash, when every fold above agrees.
    Row,
}

/// One traced fork tick: the sim's checkpoint row hash plus the folds behind
/// it, all in one `Copy` handle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TraceTick {
    /// Absolute match tick (the checkpoint's own).
    pub tick: u32,
    /// [`ForkComponent::Poses`] fold.
    pub poses: u64,
    /// [`ForkComponent::Combat`] fold.
    pub combat: u64,
    /// [`ForkComponent::Destruction`] fold.
    pub destruction: u64,
    /// The row hash the sim's checkpoints carry for this tick.
    pub row: u64,
}

/// Where two forked legs part.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ForkDivergence {
    /// Absolute tick of the first differing shared tick.
    pub tick: u32,
    /// The component that moved first at that tick.
    pub component: ForkComponent,
}

/// A forked leg's spine trace: one [`TraceTick`] per fork tick, in fork-tick
/// order (the first entry is `base_tick + 1`). Two legs join when they agree
/// on every shared tick, and [`ForkTrace::first_divergence`] names the first
/// tick they part at plus the component that moved.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ForkTrace {
    ticks: Vec<TraceTick>,
}

impl ForkTrace {
    /// The traced ticks, oldest first.
    pub fn ticks(&self) -> &[TraceTick] {
        &self.ticks
    }

    /// Number of traced ticks.
    pub fn len(&self) -> usize {
        self.ticks.len()
    }

    /// True when the fork produced no traced tick (0-tick window or a step
    /// failure that ended the window early).
    pub fn is_empty(&self) -> bool {
        self.ticks.is_empty()
    }

    /// First shared tick whose folds differ, naming the tick and the earliest
    /// component that moved. `None` when every shared tick agrees. A shorter
    /// trace joins on its shared prefix: no tick is invented for a leg that
    /// ended early.
    #[must_use]
    pub fn first_divergence(&self, other: &ForkTrace) -> Option<ForkDivergence> {
        for (left, right) in self.ticks.iter().zip(other.ticks.iter()) {
            let component = if left.poses != right.poses {
                Some(ForkComponent::Poses)
            } else if left.combat != right.combat {
                Some(ForkComponent::Combat)
            } else if left.destruction != right.destruction {
                Some(ForkComponent::Destruction)
            } else if left.row != right.row {
                Some(ForkComponent::Row)
            } else {
                None
            };
            if let Some(component) = component {
                return Some(ForkDivergence {
                    tick: left.tick,
                    component,
                });
            }
        }
        None
    }
}

/// A snapshot-clone of a match plus queued proposals, ready to run N ticks
/// on a worker thread. Reusable: [`run_n`](Fork::run_n) borrows, so the same
/// fork runs twice with identical metrics.
#[derive(Clone, Debug)]
pub struct Fork {
    config: ArenaConfig,
    base_tick: u32,
    fighters: usize,
    states: Vec<FighterState>,
    wishes: [[i8; 2]; MAX_SLOTS],
    buttons: [u8; MAX_SLOTS],
    fight: engine_combat::FightState,
    match_state: engine_combat::arena::MatchState,
    match_version: u32,
    phased: [PhasedFighter; MAX_SLOTS],
    brains: Vec<engine_ai::DepthBrain>,
    locos: Vec<engine_anim::locomotion::Locomotion>,
    destruction: BondedCollection,
    #[cfg(feature = "sim")]
    controller_snaps: Vec<ControllerSnapshot>,
    tape_inputs: Vec<FighterInput>,
    frozen: [bool; MAX_SLOTS],
}

impl Fork {
    /// Clone a [`MatchSnapshot`] into a fork. Consumes the snapshot
    /// (single-use, like [`resume`](super::resume)): take one prefix per
    /// fork, or one for the fork and one for the control resume. The `tick`
    /// must equal the snapshot's own tick, otherwise `None` — a stale or
    /// misfiled snapshot never forks silently. Under `sim` the live
    /// controllers stay with the consumed snapshot; the worker rebuilds its
    /// own from the stored snapshots, so the fork stays `Clone` + `Send`.
    pub fn from_snapshot(snapshot: MatchSnapshot, tick: u32) -> Option<Self> {
        if tick != snapshot.tick {
            return None;
        }
        let MatchSnapshot {
            config,
            tick: base_tick,
            states,
            wishes,
            buttons,
            fight,
            match_state,
            match_version,
            phased,
            brains,
            locos,
            destruction,
            rise: _,
            height,
            #[cfg(feature = "sim")]
            controller_snaps,
            #[cfg(feature = "sim")]
                controllers: _,
        } = snapshot;
        // Forks run the v1 prefix rows, which are floor-plane: a height match
        // is refused rather than forked into a sim that would drop its fall.
        if height {
            return None;
        }
        let fighters = usize::from(config.fighters);
        if states.len() != fighters || brains.len() != fighters || locos.len() != fighters {
            return None;
        }
        if match_version != MATCH_WIRE_VERSION_V0 && match_version != MATCH_WIRE_VERSION_V1 {
            return None;
        }
        #[cfg(feature = "sim")]
        if controller_snaps.len() != fighters {
            return None;
        }
        Some(Self {
            config,
            base_tick,
            fighters,
            states,
            wishes,
            buttons,
            fight,
            match_state,
            match_version,
            phased,
            brains,
            locos,
            destruction,
            #[cfg(feature = "sim")]
            controller_snaps,
            tape_inputs: Vec::new(),
            frozen: [false; MAX_SLOTS],
        })
    }

    /// Queue a proposal on the fork. `SetAggression` applies to the stored
    /// brain immediately; `SetTier` installs the tier on the stored brain;
    /// `SwapTape` merges into the fork tape (replacing that fighter's earlier
    /// slice); `FreezeFighter` latches the slot and wins over tape at run
    /// time. Invalid proposals change nothing.
    pub fn apply_proposal(&mut self, proposal: ForkProposal) -> Result<(), ForkError> {
        match proposal {
            ForkProposal::SetAggression { fighter, permille } => {
                let slot = self.slot(fighter)?;
                if permille > 1000 {
                    return Err(ForkError::BadPermille(permille));
                }
                self.brains[slot].set_morale(i32::from(permille));
                Ok(())
            }
            ForkProposal::SetTier { fighter, tier } => {
                let slot = self.slot(fighter)?;
                self.brains[slot].set_tier(tier);
                Ok(())
            }
            ForkProposal::SwapTape { fighter, inputs } => {
                self.slot(fighter)?;
                self.validate_slice(fighter, &inputs)?;
                self.tape_inputs.retain(|input| input.fighter.0 != fighter);
                self.tape_inputs.extend(inputs);
                self.tape_inputs
                    .sort_by_key(|input| (input.tick, input.fighter.0));
                Ok(())
            }
            ForkProposal::FreezeFighter { fighter } => {
                let slot = self.slot(fighter)?;
                self.frozen[slot] = true;
                Ok(())
            }
        }
    }

    /// Run `ticks` fork ticks on a worker thread and return the metrics.
    /// Clamped to the remaining match (`config.ticks - base_tick`); a 0-tick
    /// fork joins immediately with zeroed metrics. Deterministic: the worker
    /// steps the same fixed-tick path as the real match, so the same fork
    /// twice gives identical metrics. Vectorized stepping
    /// ([`vec_step`](super::vec_step)) reuses this clone-per-env isolation
    /// across N worlds: one snapshot plus tape per env, no shared mutation.
    /// A crashed worker reads as zeros here; callers that must see the crash
    /// use [`try_run_n`](Fork::try_run_n).
    pub fn run_n(&self, ticks: u32) -> ForkMetrics {
        let fighters = fighter_slot_count(self.fighters);
        self.try_run_n(ticks).unwrap_or_else(|_| ForkMetrics {
            damage_dealt: (0..fighters).map(|slot| (slot, 0)).collect(),
            ticks_to_kill: None,
            stuck_ticks: (0..fighters).map(|slot| (slot, 0)).collect(),
        })
    }

    /// [`run_n`](Fork::run_n) with the crash kept: a panicking worker returns
    /// [`ForkCrashed`] instead of zeroed metrics, which would read as a clean run.
    pub fn try_run_n(&self, ticks: u32) -> Result<ForkMetrics, ForkCrashed> {
        self.run_worker(ticks, |job| job.run())
    }

    /// The one worker-thread path: queue the job on the shared job pool, wait
    /// for it, and turn a panic into [`ForkCrashed`]. The job is built on the
    /// caller's thread first.
    ///
    /// E21: the fork rides the shared pool (one worker set, started once)
    /// instead of spawning a thread per fork. The pool re-raises a job panic
    /// on the caller, so the crash still reads as [`ForkCrashed`] and never
    /// as zeroed metrics.
    fn run_worker<T, F>(&self, ticks: u32, work: F) -> Result<T, ForkCrashed>
    where
        T: Send + 'static,
        F: FnOnce(ForkJob) -> T + Send + 'static,
    {
        let job = self.job(ticks);
        let (sender, receiver) = std::sync::mpsc::channel::<T>();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            engine_ecs::pool::shared().scope(|scope| {
                scope.spawn(move || {
                    let _ = sender.send(work(job));
                });
            });
        }));
        if outcome.is_err() {
            return Err(ForkCrashed);
        }
        receiver.recv().map_err(|_| ForkCrashed)
    }

    /// Recorded fork run: same worker isolation as [`run_n`](Fork::run_n),
    /// but each tick records one row per fighter through
    /// [`on_fork_step`] (pre-step
    /// observation, decided input, damage outcome). Deterministic: the same
    /// fork twice records identical rows, and the metrics equal [`run_n`](Fork::run_n).
    ///
    /// Consumer: DT2 student trainer; the sibling live-match entrypoint is
    /// [`run_dataset`](super::run_dataset).
    pub fn run_recorded(&self, ticks: u32) -> (ForkMetrics, Dataset) {
        let fighters = fighter_slot_count(self.fighters);
        match self.run_worker(ticks, ForkJob::run_recorded_inner) {
            Ok(pair) => pair,
            Err(_) => (
                ForkMetrics {
                    damage_dealt: (0..fighters).map(|slot| (slot, 0)).collect(),
                    ticks_to_kill: None,
                    stuck_ticks: (0..fighters).map(|slot| (slot, 0)).collect(),
                },
                Dataset::empty(),
            ),
        }
    }

    /// Runs the fork window on its own worker and keeps the per-tick spine
    /// trace instead of metrics alone. Same isolation and step path as
    /// [`run_n`](Fork::run_n): the same fork twice traces identical, and two
    /// forks off identical prefixes agree tick for tick. A crashed worker
    /// reads as an empty trace; callers that must see the crash use the
    /// metrics path ([`try_run_n`](Fork::try_run_n)).
    ///
    /// Consumer: ARC3/ARC5 rejoin, which compares two legs with
    /// [`ForkTrace::first_divergence`].
    pub fn run_trace(&self, ticks: u32) -> ForkTrace {
        self.run_worker(ticks, ForkJob::run_trace_inner)
            .unwrap_or_default()
    }

    /// Slot index or [`ForkError::UnknownFighter`].
    fn slot(&self, fighter: u8) -> Result<usize, ForkError> {
        let slot = usize::from(fighter);
        if slot < self.fighters {
            Ok(slot)
        } else {
            Err(ForkError::UnknownFighter(fighter))
        }
    }

    /// Validate one replacement slice: per-input ranges, correct fighter,
    /// ticks inside `base_tick..config.ticks`, sorted with unique ticks.
    fn validate_slice(&self, fighter: u8, inputs: &[FighterInput]) -> Result<(), ForkError> {
        let mut prev: Option<u32> = None;
        for (index, input) in inputs.iter().enumerate() {
            input.validate().map_err(ForkError::BadTape)?;
            if input.fighter.0 != fighter {
                return Err(ForkError::MisfiledTape {
                    want: fighter,
                    got: input.fighter.0,
                });
            }
            if input.tick < self.base_tick || input.tick >= self.config.ticks {
                return Err(ForkError::TickOutOfRange(input.tick));
            }
            if let Some(last) = prev {
                if input.tick < last {
                    return Err(ForkError::UnsortedTape(index));
                }
                if input.tick == last {
                    return Err(ForkError::DuplicateInput(index));
                }
            }
            prev = Some(input.tick);
        }
        Ok(())
    }

    /// Clone the fork state into a worker job, clamped to the remaining
    /// match ticks. The observer intake flag plus phased cursors travel
    /// with the job, so the fork suffix resolves on its prefix's path.
    fn job(&self, ticks: u32) -> ForkJob {
        let remaining = self.config.ticks.saturating_sub(self.base_tick);
        ForkJob {
            config: self.config,
            base_tick: self.base_tick,
            ticks: ticks.min(remaining),
            fighters: self.fighters,
            states: self.states.clone(),
            wishes: self.wishes,
            buttons: self.buttons,
            fight: self.fight,
            match_state: self.match_state,
            match_version: self.match_version,
            phased: self.phased,
            brains: self.brains.clone(),
            locos: self.locos.clone(),
            destruction: self.destruction.clone(),
            #[cfg(feature = "sim")]
            controller_snaps: self.controller_snaps.clone(),
            tape_inputs: self.tape_inputs.clone(),
            frozen: self.frozen,
        }
    }

    /// Suffix match rows for `ticks` fork ticks on the calling thread: a
    /// test probe over the same worker restore as [`run_n`](Fork::run_n),
    /// proving the fork suffix resolves on its prefix's intake path. Empty
    /// on an empty window or a step failure, never panics.
    #[cfg(test)]
    fn suffix_match_rows(&self, ticks: u32) -> Vec<super::TickMatchEvents> {
        let job = self.job(ticks);
        let end = job.base_tick.saturating_add(job.ticks);
        let steps = job.ticks as usize;
        let Some((mut sim, tape)) = job.build_sim(steps) else {
            return Vec::new();
        };
        while sim.tick < end {
            if sim.step(&tape).is_none() {
                return Vec::new();
            }
        }
        sim.match_log
    }
}

/// Owned worker input: everything one fork run reads. All fields are
/// `Send` (plain data plus snapshot clones); the worker rebuilds live
/// controllers from the snapshots, so no live physics world crosses threads.
struct ForkJob {
    config: ArenaConfig,
    base_tick: u32,
    ticks: u32,
    fighters: usize,
    states: Vec<FighterState>,
    wishes: [[i8; 2]; MAX_SLOTS],
    buttons: [u8; MAX_SLOTS],
    fight: engine_combat::FightState,
    match_state: engine_combat::arena::MatchState,
    match_version: u32,
    phased: [PhasedFighter; MAX_SLOTS],
    brains: Vec<engine_ai::DepthBrain>,
    locos: Vec<engine_anim::locomotion::Locomotion>,
    destruction: BondedCollection,
    #[cfg(feature = "sim")]
    controller_snaps: Vec<ControllerSnapshot>,
    tape_inputs: Vec<FighterInput>,
    frozen: [bool; MAX_SLOTS],
}

impl ForkJob {
    /// Plain worker body: metrics only, no row allocation.
    fn run(self) -> ForkMetrics {
        self.run_inner(false).0
    }

    /// Recorded worker body: metrics plus one row per fighter per tick
    /// through the decision recorder.
    fn run_recorded_inner(self) -> (ForkMetrics, Dataset) {
        let (metrics, rows) = self.run_inner(true);
        (
            metrics,
            Dataset {
                version: DATASET_VERSION,
                rows: rows.unwrap_or_default(),
            },
        )
    }

    /// Trace worker body: the same step path and the same tape fill as
    /// [`run_inner`](ForkJob::run_inner), recording one [`TraceTick`] per fork
    /// tick (the sim's checkpoint row hash plus the three component folds). A
    /// step failure ends the window with the ticks already traced, never a
    /// fabricated one.
    fn run_trace_inner(self) -> ForkTrace {
        let steps = step_count(self.ticks);
        let fighters = self.fighters;
        let Some((mut sim, tape)) = self.build_sim(steps) else {
            return ForkTrace::default();
        };
        let mut ticks = Vec::with_capacity(steps);
        for _ in 0..steps {
            if sim.step(&tape).is_none() {
                break;
            }
            let Some(checkpoint) = sim.checkpoints.last() else {
                break;
            };
            let events = sim
                .events
                .last()
                .map_or(&[][..], |row| row.events.as_slice());
            let live = sim.states.len().min(fighters);
            ticks.push(TraceTick {
                tick: checkpoint.tick,
                poses: fold_poses(&sim.states[..live], &sim.buttons),
                combat: fold_combat(&sim.fight, events),
                destruction: sim.destruction.state_hash(),
                row: checkpoint.hash,
            });
        }
        ForkTrace { ticks }
    }

    /// Build the worker sim plus its window tape: one owner for the fork
    /// restore (poses, combat, the intake flag plus phased cursors, brains,
    /// locomotion, the destruction prop, rebuilt controllers under `sim`).
    /// `None` on a 0-tick window, an unknown intake version, or an
    /// unbuildable controller world; the caller ends the window with partial
    /// output rather than trapping the worker.
    fn build_sim(self, steps: usize) -> Option<(Sim, FighterTape)> {
        if steps == 0
            || (self.match_version != MATCH_WIRE_VERSION_V0
                && self.match_version != MATCH_WIRE_VERSION_V1)
        {
            return None;
        }
        let tape = self.tape(steps);
        let cursor = tape
            .0
            .iter()
            .position(|input| input.tick >= self.base_tick)
            .unwrap_or(tape.0.len());
        #[cfg(feature = "sim")]
        let controllers = self.rebuilt_controllers()?;
        // Forks come from v1 prefix snapshots only: V1 tables keep poise
        // damage 0, so the fork depth never staggers and never hashes.
        let depth = DepthSet::new(engine_combat::TABLES_V1, self.config.fighters);
        Some((
            Sim {
                config: self.config,
                fighters: self.fighters,
                tick: self.base_tick,
                states: self.states,
                wishes: self.wishes,
                buttons: self.buttons,
                fight: self.fight,
                match_track: super::match_wire::MatchTrack::from_state(
                    &self.match_state,
                    self.match_version,
                    self.phased,
                ),
                brains: self.brains,
                locos: self.locos,
                warps: Vec::new(),
                destruction: self.destruction,
                // Forks come from v1 prefix snapshots, which carry no
                // obstacle world: the empty world is the legacy match.
                world: super::world::MatchWorld::empty(),
                #[cfg(feature = "sim")]
                controllers,
                cursor,
                checkpoints: Vec::with_capacity(steps),
                events: Vec::with_capacity(steps),
                combat_scratch: Vec::with_capacity(MAX_EVENTS_PER_TICK),
                // Forks come from v1 prefix snapshots only, so the cue buffer
                // stays empty; it still needs its reservation to build.
                last_cues: Vec::with_capacity(MAX_ATTACK_STARTED_PER_TICK),
                last_deflects: Vec::new(),
                breaks: Vec::with_capacity(steps),
                // Cut rows fill as the fork replays, one row per tick like
                // `breaks`; a v1 prefix snapshot carries none.
                cuts: Vec::with_capacity(steps),
                match_log: Vec::with_capacity(steps),
                // Forks keep the observed match track live, matching the
                // observer entry points (`run_match_events`).
                match_live: true,
                history: None,
                loco_history: None,
                last_inputs: [None; MAX_SLOTS],
                // Forks come from v1 prefix snapshots only.
                rules: super::ArenaRules::V1,
                balance: Vec::new(),
                stagger: Vec::new(),
                knockback: Vec::new(),
                depth,
                limbs: LimbPool::new(MAX_LIMBS).ok()?,
                spear: None,
                // Forks come from v1 prefix snapshots only, which carry no
                // pilum volley: the channel stays off.
                pilum: None,
                // The same holds for the manuballista: no bolt flies in a
                // fork of a v1 prefix.
                bolt: None,
                rise: Vec::new(),
                height: false,
                #[cfg(test)]
                trace: None,
                #[cfg(test)]
                pairwise_plan: false,
                #[cfg(test)]
                full_collide_passes: false,
            },
            tape,
        ))
    }

    /// Step the fork window and fold events plus motion into metrics. Never
    /// panics on stored data: validated at apply time, clamped here; an
    /// internal step failure ends the window with partial metrics rather
    /// than trapping the worker. Recorded runs additionally snapshot the
    /// pre-step observation sources each tick (plain forks pay nothing).
    fn run_inner(self, record: bool) -> (ForkMetrics, Option<Vec<Row>>) {
        let mut zero = self.zeros();
        let steps = step_count(self.ticks);
        let fighters = self.fighters;
        let base_tick = self.base_tick;
        let Some((mut sim, tape)) = self.build_sim(steps) else {
            return (zero, record.then(Vec::new));
        };
        // Stuck threshold as a squared per-tick distance: 0.2 m/s at 60 Hz
        // steps without a square root in the loop.
        let step_m = STUCK_SPEED_MPS / TICK_HZ as f32;
        let stuck_sq = step_m * step_m;
        // Damage counters in fixed storage: slots beyond the fork read zero
        // and only `..fighters` folds into the metrics.
        let mut damage = [0u16; MAX_SLOTS];
        let mut rows: Option<Vec<Row>> = if record {
            Some(Vec::with_capacity(steps * fighters))
        } else {
            None
        };
        for index in 0..steps {
            // Pre-step snapshot on the stack: poses feed the stuck fold, and
            // the recorded path borrows the same prefix plus the combat and
            // damage copies below. `sim.step` advances in place, so snapshot
            // first; fixed arrays keep every fork tick off the heap. The tail
            // mirrors slot 0 and is never read: only `..fighters` is live.
            // Slot count is fixed by the snapshot config (`from_snapshot`
            // rejects length mismatch), so both the index and the prefix
            // below stay in range.
            debug_assert_eq!(sim.states.len(), fighters);
            let mut pre_states = [sim.states[0]; MAX_SLOTS];
            pre_states[..fighters].copy_from_slice(&sim.states);
            // Recorder snapshots: pre-step combat state and damage counters.
            // Plain `Copy` values evaluated only on recorded runs, so plain
            // forks still pay no heap and no copies here.
            let record_before = if rows.is_some() {
                Some((sim.fight, damage))
            } else {
                None
            };
            if sim.step(&tape).is_none() {
                break;
            }
            let Some(row) = sim.events.last() else {
                break;
            };
            let mut killed = false;
            for event in &row.events {
                match *event {
                    FightEvent::Hit {
                        attacker,
                        damage: hit,
                        ..
                    } => {
                        let slot = usize::from(attacker);
                        if slot < fighters {
                            damage[slot] = damage[slot].saturating_add(hit);
                        }
                    }
                    FightEvent::Blocked { attacker, chip, .. } => {
                        let slot = usize::from(attacker);
                        if slot < fighters {
                            damage[slot] = damage[slot].saturating_add(chip);
                        }
                    }
                    FightEvent::Knockdown { .. } => {
                        killed = true;
                        if zero.ticks_to_kill.is_none() {
                            zero.ticks_to_kill = Some(index as u32 + 1);
                        }
                    }
                    FightEvent::Parried { .. }
                    | FightEvent::Dodged { .. }
                    | FightEvent::StaminaOut { .. } => {}
                }
            }
            for (slot, pre) in pre_states.iter().enumerate().take(fighters) {
                let was = pre.pos;
                let now = sim.states[slot].pos;
                let (dx, dz) = (now[0] - was[0], now[2] - was[2]);
                if dx * dx + dz * dz < stuck_sq {
                    zero.stuck_ticks[slot].1 += 1;
                }
            }
            // One row per fighter: the tick's shared pre-step observation,
            // that slot's decided input, and its damage before/after.
            //
            // E21/RC36: the rows are built per fighter on the shared job
            // pool and merged in fighter order, so the bytes are the ones
            // the serial loop wrote, whatever the worker count.
            if let (Some(out), Some((pre_fight, before_damage))) = (rows.as_mut(), record_before) {
                let inputs = sim.last_inputs();
                let tick = base_tick + index as u32;
                let observed = &pre_states[..fighters];
                let row_of = |slot: usize| -> Option<Row> {
                    let input = inputs[slot]?;
                    let step = StepRecord {
                        tick,
                        states: observed,
                        fight: &pre_fight,
                        input,
                        damage_before: before_damage[slot],
                        damage_after: damage[slot],
                        killed,
                    };
                    on_fork_step(&step)
                };
                // The rows are built on the shared job pool when the `spine` feature brings
                // `engine-ecs` in; the default graph keeps the serial loop, which writes the
                // same rows in fighter order.
                #[cfg(feature = "spine")]
                let partials = engine_ecs::parallel::fork_join_on(
                    engine_ecs::pool::shared(),
                    0..fighters,
                    engine_ecs::jobs::ChunkSize::new(1),
                    |range| range.filter_map(row_of).collect::<Vec<Row>>(),
                    Vec::new(),
                    |mut acc, part| {
                        acc.extend(part);
                        acc
                    },
                )
                .unwrap_or_else(|_| {
                    // The range is non-empty, so the fork cannot refuse; the
                    // serial loop gives the same rows if it ever does.
                    (0..fighters).filter_map(row_of).collect::<Vec<Row>>()
                });
                #[cfg(not(feature = "spine"))]
                let partials = (0..fighters).filter_map(row_of).collect::<Vec<Row>>();
                out.extend(partials);
            }
        }
        for (slot, total) in damage.iter().enumerate().take(fighters) {
            zero.damage_dealt[slot].1 = *total;
        }
        (zero, rows)
    }

    /// Zeroed metrics with one entry per fighter.
    fn zeros(&self) -> ForkMetrics {
        let fighters = fighter_slot_count(self.fighters);
        ForkMetrics {
            damage_dealt: (0..fighters).map(|slot| (slot, 0)).collect(),
            ticks_to_kill: None,
            stuck_ticks: (0..fighters).map(|slot| (slot, 0)).collect(),
        }
    }

    /// Fork tape for this window: stored overrides plus one idle input per
    /// frozen slot per tick (freeze wins over swap on collisions).
    fn tape(&self, steps: usize) -> FighterTape {
        let mut inputs = self.tape_inputs.clone();
        for slot in 0..self.fighters {
            if !self.frozen[slot] {
                continue;
            }
            let Some(id) = FighterId::new(slot as u8) else {
                continue;
            };
            inputs.retain(|input| {
                !(input.fighter.0 == slot as u8
                    && input.tick >= self.base_tick
                    && input.tick < self.base_tick + self.ticks)
            });
            inputs.extend((0..steps).map(|index| FighterInput {
                tick: self.base_tick + index as u32,
                fighter: id,
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: engine_input::fighter::Buttons::NONE,
            }));
        }
        inputs.sort_by_key(|input| (input.tick, input.fighter.0));
        FighterTape(inputs)
    }

    /// Rebuild one settled controller per fighter and restore the stored
    /// snapshots, so the worker steps the same collision world as the match.
    #[cfg(feature = "sim")]
    fn rebuilt_controllers(&self) -> Option<Vec<engine_physics::controller::Controller>> {
        let mut controllers = build_controllers(&self.config, &self.states)?;
        if controllers.len() != self.fighters || self.controller_snaps.len() != self.fighters {
            return None;
        }
        for (controller, snap) in controllers.iter_mut().zip(self.controller_snaps.iter()) {
            controller.restore(snap.clone()).ok()?;
        }
        Some(controllers)
    }
}

/// Fork window length in steps. `u32` ticks fit `usize` on every target this
/// engine builds for; the saturating fallback keeps a narrower target from
/// wrapping instead of casting.
fn step_count(ticks: u32) -> usize {
    usize::try_from(ticks).unwrap_or(usize::MAX)
}

/// Poses fold: fighter ids, feet positions and yaw plus the tick's buttons.
/// Positions go through the checkpoint path's canonical zeroing, so `-0.0`
/// and `0.0` never trace as two different worlds.
fn fold_poses(states: &[FighterState], buttons: &[u8]) -> u64 {
    let mut fold = Fnv64::new();
    for state in states {
        fold.write_u32(u32::from(state.id.0));
        for axis in state.pos {
            fold.write_f32(super::canon_zero(axis));
        }
        fold.write_f32(super::canon_zero(state.yaw));
    }
    for button in buttons {
        fold.write_u8(*button);
    }
    fold.finish()
}

/// Combat fold: the active fighters' hp, stamina and action clocks plus this
/// tick's events. One numeric code per event kind, so the fold needs no
/// `Hash` on the event.
fn fold_combat(fight: &engine_combat::FightState, events: &[FightEvent]) -> u64 {
    let mut fold = Fnv64::new();
    fold.write_u8(fight.fighter_count);
    fold.write_u32(fight.hitstop);
    for state in fight.fighters.iter().take(usize::from(fight.fighter_count)) {
        fold.write_u16(state.hp_permille);
        fold.write_u16(state.stamina);
        fold.write_u32(state.attack_age);
        fold.write_bool(state.is_attacking);
        fold.write_u32(state.block_age);
        fold.write_bool(state.is_blocking);
        fold.write_u32(state.dodge_age);
        fold.write_bool(state.is_dodging);
        fold.write_u32(state.react);
        fold.write_u32(state.knockdown);
    }
    fold.write_len(events.len());
    for event in events {
        fold.write_u32(event_code(*event));
    }
    fold.finish()
}

/// One combat event as a number, damage carried, so two ticks with the same
/// events in the same order fold equal and a different event never collides
/// with a different count.
fn event_code(event: FightEvent) -> u32 {
    match event {
        FightEvent::Hit { damage, .. } => 1_000_000 + u32::from(damage),
        FightEvent::Blocked { chip, .. } => 2_000_000 + u32::from(chip),
        FightEvent::Knockdown { .. } => 3_000_000,
        FightEvent::Parried { .. } => 4_000_000,
        FightEvent::Dodged { .. } => 5_000_000,
        FightEvent::StaminaOut { .. } => 6_000_000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_input::fighter::{Buttons, ATTACK, BLOCK};

    fn config() -> ArenaConfig {
        ArenaConfig {
            seed: 2040,
            ticks: 600,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: engine_combat::tables::TABLES_VERSION_V1,
        }
    }

    fn fork_at(ticks: u32) -> Fork {
        let (snapshot, _) = super::super::run_prefix(&config(), &FighterTape::default(), ticks)
            .expect("valid prefix");
        Fork::from_snapshot(snapshot, ticks).expect("tick matches snapshot")
    }

    /// Relentless attack script for one fighter over the fork window:
    /// attack pressed every tick, idle wish, neutral facing.
    fn attack_spam(fighter: u8, base: u32, steps: u32) -> Vec<FighterInput> {
        let id = FighterId::new(fighter).expect("course uses slots 0..2");
        (0..steps)
            .map(|index| FighterInput {
                tick: base + index,
                fighter: id,
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: Buttons(ATTACK),
            })
            .collect()
    }

    #[test]
    fn tick_mismatch_forks_nothing() {
        let (snapshot, _) =
            super::super::run_prefix(&config(), &FighterTape::default(), 60).expect("valid prefix");
        assert!(Fork::from_snapshot(snapshot, 61).is_none());
    }

    #[test]
    fn fighter_slot_count_clamps_out_of_range_instead_of_truncating() {
        assert_eq!(super::fighter_slot_count(300), u8::MAX);
        assert_eq!(super::fighter_slot_count(2), 2);
    }

    #[test]
    fn same_fork_twice_gives_identical_metrics() {
        let fork = fork_at(60);
        let (first, second) = (fork.run_n(120), fork.run_n(120));
        assert_eq!(first, second);
        // Two forks from two identical prefixes agree as well.
        let replay = fork_at(60);
        assert_eq!(replay.run_n(120), first);
        assert_eq!(first.damage_dealt.len(), 2);
        assert_eq!(first.stuck_ticks.len(), 2);
    }

    #[test]
    fn real_match_untouched_by_forks() {
        let tape = FighterTape::default();
        let control = super::super::run(&config(), &tape);
        assert_eq!(control.len(), config().ticks as usize);
        let mut fork = fork_at(200);
        fork.apply_proposal(ForkProposal::FreezeFighter { fighter: 1 })
            .expect("slot 1 freezes");
        let _ = fork.run_n(120);
        let after = super::super::run(&config(), &tape);
        assert_eq!(control, after);
    }

    #[test]
    fn forked_legs_resimulate_identical_twice() {
        let fork = fork_at(60);
        let first = fork.run_trace(120);
        assert_eq!(first.len(), 120, "one traced tick per fork tick");
        assert_eq!(
            first.ticks()[0].tick,
            60,
            "the checkpoint tick is the base tick of the first step"
        );
        assert_eq!(first.ticks()[119].tick, 179, "the last tick of the window");
        assert_eq!(first, fork.run_trace(120), "the same fork twice agrees");
        assert_eq!(
            fork_at(60).run_trace(120),
            first,
            "two forks off identical prefixes agree tick for tick"
        );
        assert!(
            first.first_divergence(&first).is_none(),
            "a leg joins itself"
        );
        // The trace rides the same window as the metrics path.
        let metrics = fork.run_n(120);
        assert_eq!(metrics.damage_dealt.len(), 2);
    }

    #[test]
    fn a_seeded_input_names_its_tick_and_component() {
        // The control leg and a leg whose fighter 0 attacks from tick 100.
        let control = fork_at(60).run_trace(120);
        let mut seeded = fork_at(60);
        seeded
            .apply_proposal(ForkProposal::SwapTape {
                fighter: 0,
                inputs: attack_spam(0, 100, 20),
            })
            .expect("tape swap validates");
        let divergent = seeded.run_trace(120);
        // Every tick before the seeded input agrees...
        for (left, right) in control.ticks().iter().zip(divergent.ticks()).take(40) {
            assert_eq!(left, right, "tick {} is before the seed", left.tick);
        }
        // ...and the first tick it lands on is the named one.
        let found = control
            .first_divergence(&divergent)
            .expect("the seeded tick diverges");
        assert_eq!(found.tick, 100, "the tick the seeded input lands on");
        assert!(
            matches!(
                found.component,
                ForkComponent::Poses | ForkComponent::Combat
            ),
            "an attack moves pose and combat, not the prop: {:?}",
            found.component
        );
    }

    #[test]
    fn divergence_names_the_first_moving_component() {
        let tick = |tick: u32, poses: u64, combat: u64, destruction: u64, row: u64| TraceTick {
            tick,
            poses,
            combat,
            destruction,
            row,
        };
        let left = ForkTrace {
            ticks: vec![
                tick(1, 10, 20, 30, 40),
                tick(2, 11, 21, 31, 41),
                tick(3, 12, 22, 32, 42),
            ],
        };
        let combat = ForkTrace {
            ticks: vec![
                tick(1, 10, 20, 30, 40),
                tick(2, 11, 99, 31, 41),
                tick(3, 12, 22, 32, 42),
            ],
        };
        assert_eq!(
            left.first_divergence(&combat),
            Some(ForkDivergence {
                tick: 2,
                component: ForkComponent::Combat,
            })
        );
        // A pose move at the same tick is named before the combat change.
        let poses = ForkTrace {
            ticks: vec![tick(1, 10, 20, 30, 40), tick(2, 77, 99, 31, 41)],
        };
        assert_eq!(
            left.first_divergence(&poses),
            Some(ForkDivergence {
                tick: 2,
                component: ForkComponent::Poses,
            })
        );
        // The prop alone, then the row hash alone.
        let prop = ForkTrace {
            ticks: vec![tick(1, 10, 20, 30, 40), tick(2, 11, 21, 88, 41)],
        };
        assert_eq!(
            left.first_divergence(&prop).map(|d| d.component),
            Some(ForkComponent::Destruction)
        );
        let row = ForkTrace {
            ticks: vec![tick(1, 10, 20, 30, 40), tick(2, 11, 21, 31, 99)],
        };
        assert_eq!(
            left.first_divergence(&row),
            Some(ForkDivergence {
                tick: 2,
                component: ForkComponent::Row,
            })
        );
        // A leg that ended early joins on its shared prefix.
        let short = ForkTrace {
            ticks: vec![tick(1, 10, 20, 30, 40)],
        };
        assert_eq!(left.first_divergence(&short), None);
        assert_eq!(short.first_divergence(&left), None);
    }

    #[test]
    fn frozen_foes_deal_nothing_and_hold_still() {
        let mut fork = fork_at(0);
        for fighter in 0..2 {
            fork.apply_proposal(ForkProposal::FreezeFighter { fighter })
                .expect("slots freeze");
        }
        let metrics = fork.run_n(120);
        assert_eq!(metrics.damage_dealt, vec![(0, 0), (1, 0)]);
        assert_eq!(metrics.ticks_to_kill, None);
        // Idle wishes never move: every tick counts as stuck.
        assert_eq!(metrics.stuck_ticks, vec![(0, 120), (1, 120)]);
        assert_eq!(metrics, fork.run_n(120));
    }

    #[test]
    fn balance_breaking_proposal_shifts_metrics_vs_control() {
        // Control: both foes frozen, so the window is exactly zero damage
        // with no kill. Treatment: fighter 0 spams attacks into a frozen
        // foe, so every impact lands full damage until the kill.
        let mut control = fork_at(0);
        for fighter in 0..2 {
            control
                .apply_proposal(ForkProposal::FreezeFighter { fighter })
                .expect("slots freeze");
        }
        let calm = control.run_n(600);
        let mut wild = fork_at(0);
        wild.apply_proposal(ForkProposal::SwapTape {
            fighter: 0,
            inputs: attack_spam(0, 0, 600),
        })
        .expect("spam validates");
        wild.apply_proposal(ForkProposal::FreezeFighter { fighter: 1 })
            .expect("foe freezes");
        let shifted = wild.run_n(600);
        assert!(shifted.damage_dealt[0].0 == 0 && shifted.damage_dealt[1].0 == 1);
        assert!(shifted.damage_dealt[0].1 > 0, "spam must land: {shifted:?}");
        assert_eq!(shifted.damage_dealt[1].1, 0);
        assert!(
            shifted.ticks_to_kill.is_some(),
            "relentless spam kills: {shifted:?}"
        );
        assert_ne!(shifted, calm);
        assert_eq!(shifted, wild.run_n(600));
    }

    #[test]
    fn worker_completes_zero_and_full_length_forks() {
        let fork = fork_at(0);
        let empty = fork.run_n(0);
        assert_eq!(empty.damage_dealt, vec![(0, 0), (1, 0)]);
        assert_eq!(empty.ticks_to_kill, None);
        assert_eq!(empty.stuck_ticks, vec![(0, 0), (1, 0)]);
        // Full-match fork joins without deadlock and stays deterministic.
        let full = fork.run_n(600);
        assert_eq!(full.damage_dealt.len(), 2);
        assert_eq!(full.stuck_ticks.len(), 2);
        assert_eq!(full, fork.run_n(600));
    }

    #[test]
    fn forced_worker_panic_is_a_crash_not_zero_metrics() {
        // A panic inside one fork's worker comes back as ForkCrashed; the
        // zeroed metrics the plain path used to return would read as a run.
        let fork = fork_at(0);
        let crashed = fork.run_worker(600, |_: ForkJob| -> ForkMetrics {
            panic!("forced fork panic")
        });
        assert_eq!(crashed, Err(ForkCrashed));
        // The healthy path is unchanged: the typed run equals the plain run.
        assert_eq!(fork.try_run_n(120), Ok(fork.run_n(120)));
    }

    #[test]
    fn aggression_extremes_stay_deterministic() {
        let mut brave = fork_at(60);
        brave
            .apply_proposal(ForkProposal::SetAggression {
                fighter: 0,
                permille: 1000,
            })
            .expect("max morale applies");
        let mut calm = fork_at(60);
        calm.apply_proposal(ForkProposal::SetAggression {
            fighter: 0,
            permille: 0,
        })
        .expect("min morale applies");
        assert_eq!(brave.run_n(120), brave.run_n(120));
        assert_eq!(calm.run_n(120), calm.run_n(120));
    }

    #[test]
    fn invalid_proposals_change_nothing() {
        let mut fork = fork_at(60);
        assert_eq!(
            fork.apply_proposal(ForkProposal::SetAggression {
                fighter: 9,
                permille: 500,
            }),
            Err(ForkError::UnknownFighter(9))
        );
        assert_eq!(
            fork.apply_proposal(ForkProposal::SetAggression {
                fighter: 0,
                permille: 1001,
            }),
            Err(ForkError::BadPermille(1001))
        );
        // `SetTier` mirrors the morale row: a slot outside the fighter count
        // refuses with the culprit carried, never a panic.
        assert_eq!(
            fork.apply_proposal(ForkProposal::SetTier {
                fighter: 2,
                tier: SHIPPED_TIERS[0],
            }),
            Err(ForkError::UnknownFighter(2))
        );
        assert_eq!(
            fork.apply_proposal(ForkProposal::FreezeFighter { fighter: 7 }),
            Err(ForkError::UnknownFighter(7))
        );
        // Misfiled input: slice for fighter 0 names fighter 1.
        let misfiled = FighterInput {
            tick: 60,
            fighter: FighterId::new(1).expect("slot 1 legal"),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        };
        assert_eq!(
            fork.apply_proposal(ForkProposal::SwapTape {
                fighter: 0,
                inputs: vec![misfiled],
            }),
            Err(ForkError::MisfiledTape { want: 0, got: 1 })
        );
        // Past tick (before the fork base) is out of range.
        let stale = FighterInput {
            tick: 59,
            fighter: FighterId::new(0).expect("slot 0 legal"),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        };
        assert_eq!(
            fork.apply_proposal(ForkProposal::SwapTape {
                fighter: 0,
                inputs: vec![stale],
            }),
            Err(ForkError::TickOutOfRange(59))
        );
        // Unknown button bits fail through the fighter contract.
        let bad = FighterInput {
            tick: 60,
            fighter: FighterId::new(0).expect("slot 0 legal"),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons(0x08),
        };
        assert!(matches!(
            fork.apply_proposal(ForkProposal::SwapTape {
                fighter: 0,
                inputs: vec![bad],
            }),
            Err(ForkError::BadTape(_))
        ));
        // The rejected proposals left no trace: the fork still runs clean.
        let metrics = fork.run_n(60);
        assert_eq!(metrics, fork.run_n(60));
    }

    #[test]
    fn recorded_fork_matches_plain_run_and_replays() {
        let fork = fork_at(60);
        let plain = fork.run_n(120);
        let (metrics, dataset) = fork.run_recorded(120);
        assert_eq!(metrics, plain);
        assert_eq!(dataset.version, DATASET_VERSION);
        assert_eq!(dataset.rows.len(), 120 * 2);
        // Row shape: tick-major, pre-step ticks, one row per fighter.
        for (index, row) in dataset.rows.iter().enumerate() {
            let tick = 60 + (index / 2) as u32;
            assert_eq!(row.observation.tick, tick, "row {index}");
            assert_eq!(row.action.input.tick, tick, "row {index}");
            assert_eq!(row.observation.fighters.len(), 2);
            assert_eq!(row.action.fighter, (index % 2) as u8);
        }
        // Counters start at zero damage on the first tick.
        assert_eq!(dataset.rows[0].outcome.damage_before, 0);
        assert_eq!(dataset.rows[1].outcome.damage_before, 0);
        // Replay: identical rows and metrics.
        let (again_metrics, again) = fork.run_recorded(120);
        assert_eq!(again_metrics, metrics);
        assert_eq!(again, dataset);
        // A 0-tick recording is an empty v1 dataset.
        let (empty_metrics, empty) = fork.run_recorded(0);
        assert!(empty.rows.is_empty());
        assert_eq!(empty.version, DATASET_VERSION);
        assert_eq!(empty_metrics, fork.run_n(0));
    }

    #[test]
    fn recorded_totals_match_fork_metrics() {
        // Attack spam into a frozen foe lands damage and kills: the row
        // outcomes must agree with the metrics fold over the same run, and
        // per-slot counters must chain exactly (before[k+1] == after[k]).
        let mut fork = fork_at(0);
        fork.apply_proposal(ForkProposal::SwapTape {
            fighter: 0,
            inputs: attack_spam(0, 0, 600),
        })
        .expect("spam validates");
        fork.apply_proposal(ForkProposal::FreezeFighter { fighter: 1 })
            .expect("foe freezes");
        let (metrics, dataset) = fork.run_recorded(600);
        assert!(metrics.damage_dealt[0].1 > 0, "{metrics:?}");
        assert_eq!(metrics.damage_dealt[1].1, 0);
        for (slot, (_, total)) in metrics.damage_dealt.iter().enumerate() {
            let last = dataset
                .rows
                .iter()
                .rev()
                .find(|row| usize::from(row.action.fighter) == slot)
                .expect("every slot acts every tick");
            assert_eq!(last.outcome.damage_after, *total, "slot {slot}");
            let mut chained = 0u16;
            for row in dataset
                .rows
                .iter()
                .filter(|row| usize::from(row.action.fighter) == slot)
            {
                assert_eq!(row.outcome.damage_before, chained, "slot {slot}");
                assert!(row.outcome.damage_after >= row.outcome.damage_before);
                chained = row.outcome.damage_after;
            }
        }
        assert_eq!(
            dataset.rows.iter().any(|row| row.outcome.killed),
            metrics.ticks_to_kill.is_some()
        );
    }

    /// Held attack into held block over the whole match, one input per slot
    /// per tick, so no brain ever decides and the exchange script is exact.
    fn held_duel() -> FighterTape {
        let mut inputs = Vec::new();
        for tick in 0..config().ticks {
            for (fighter, buttons) in [(0u8, ATTACK), (1u8, BLOCK)] {
                inputs.push(FighterInput {
                    tick,
                    fighter: FighterId::new(fighter).expect("course uses slots 0..2"),
                    mx: 0,
                    mz: 0,
                    facing: 0,
                    buttons: Buttons(buttons),
                });
            }
        }
        inputs.sort_by_key(|input| (input.tick, input.fighter.0));
        FighterTape(inputs)
    }

    #[test]
    fn v1_fork_suffix_matches_uninterrupted_v1() {
        // V1 gates startup, so a fork worker that dropped the phased cursors
        // (or silently fell back to legacy) would answer early on the suffix.
        // The fork replays the window tape exactly, so its suffix match rows
        // must equal the uninterrupted V1 rows tick for tick.
        let tape = held_duel();
        let full_v1 =
            super::super::run_match_events_versioned(&config(), &tape, MATCH_WIRE_VERSION_V1);
        assert_eq!(full_v1.len(), config().ticks as usize);
        let (snapshot, _) =
            super::super::run_prefix_versioned(&config(), &tape, 2, MATCH_WIRE_VERSION_V1)
                .expect("valid V1 prefix");
        let mut fork = Fork::from_snapshot(snapshot, 2).expect("V1 snapshot forks");
        for fighter in 0..2 {
            let window: Vec<FighterInput> = tape
                .0
                .iter()
                .filter(|input| input.fighter.0 == fighter && input.tick >= 2)
                .copied()
                .collect();
            fork.apply_proposal(ForkProposal::SwapTape {
                fighter,
                inputs: window,
            })
            .expect("window validates");
        }
        let remaining = config().ticks - 2;
        assert_eq!(
            fork.suffix_match_rows(remaining),
            full_v1[2..].to_vec(),
            "V1 fork suffix must match the uninterrupted V1 rows"
        );
        assert_eq!(
            fork.suffix_match_rows(remaining),
            fork.suffix_match_rows(remaining),
            "fork suffix replays deterministically"
        );
        // V0 control: the legacy fork suffix matches the legacy run, so the
        // flag carriage changes nothing on the old path.
        let full_v0 = super::super::run_match_events(&config(), &tape);
        let (legacy, _) = super::super::run_prefix(&config(), &tape, 2).expect("valid prefix");
        let mut legacy_fork = Fork::from_snapshot(legacy, 2).expect("V0 snapshot forks");
        for fighter in 0..2 {
            let window: Vec<FighterInput> = tape
                .0
                .iter()
                .filter(|input| input.fighter.0 == fighter && input.tick >= 2)
                .copied()
                .collect();
            legacy_fork
                .apply_proposal(ForkProposal::SwapTape {
                    fighter,
                    inputs: window,
                })
                .expect("window validates");
        }
        assert_eq!(
            legacy_fork.suffix_match_rows(remaining),
            full_v0[2..].to_vec(),
            "V0 fork suffix must match the uninterrupted V0 rows"
        );
        // Unknown version still refuses at the prefix gate: no silent legacy.
        assert!(super::super::run_prefix_versioned(&config(), &tape, 2, 7).is_none());
    }
}
