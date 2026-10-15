//! AI playtest agent: drive headless arena matches, watch metrics
//! each tick, file replayable bug reports.
//!
//! Consumer: the A7 gate (the agent finds at least one seeded bug with a
//! replaying tape) and the causal-replay and red-team hunts, which read
//! [`BugFile`]. Data owner: [`Agent`] owns its [`AgentConfig`] plus the
//! [`Driver`] under test; the caller owns nothing after [`Agent::drive`]
//! returns. Units: fixed ticks at [`engine_arena::TICK_HZ`], meters, m/s for
//! stuck speeds. Determinism scope: same-machine reproducibility — the same
//! seed twice gives the identical [`AgentReport`] (arena forks plus sorted
//! tapes and slot-ordered scans; no wall clock, no threads of our own, no
//! hash-map order). Error policy: invalid configs expire their job with a
//! bounded log line and file nothing; malformed [`BugFile`] bytes replay to
//! an empty vec, never a panic.
//!
//! Layering: the [`PlayRunner`] is a local stub of the endpoint `play.run` job
//! shape — [`JobId`] handles with submit/poll/cancel/expiry and bounded logs
//! as plain data. It never imports the endpoint crate. Trajectories come
//! from fork-style snapshots ([`engine_arena::run_prefix`] plus
//! [`Fork::run_recorded`](engine_arena::fork::Fork::run_recorded)), and the
//! stuck threshold reuses
//! [`STUCK_SPEED_MPS`](engine_arena::fork::STUCK_SPEED_MPS) so the agent and
//! the fork metrics agree on what "stuck" means. [`crate::verify`] and
//! the [`crate::trigger::Trigger`] stay separate entrypoints: they check
//! authored content and editor revisions, while this agent watches live match
//! trajectories for stuck fighters, desync and impossible states.
//!
//! Backends: the agent never imports a provider. [`Driver`] is a plain
//! `{ policy, caps }` value — mock, local and remote backends swap the driver
//! only; agent logic never branches on it.
//!
//! Tape construction without an input dependency: this crate depends only on
//! `engine-arena` (which re-exports [`FighterInput`](engine_arena::FighterInput)
//! but not its button type), so seeded tapes derive from one template input
//! recorded on the same config through [`engine_arena::run_dataset`] —
//! deterministic per seed — with struct-update syntax filling the fields this
//! crate never names.

use engine_arena::fork::{Fork, ForkProposal};
use engine_arena::recorder::DATASET_VERSION;
use engine_arena::{ArenaConfig, Checkpoint, FighterId, FighterInput, FighterTape};
use engine_arena::{MAX_PARTICIPANTS, TICK_HZ};

/// Neutral alias for the input tape contract type: engine code outside the
/// GE2 genre ledger imports the tape from here instead of naming the genre
/// type (mirrors net `session::BodyTape`; this crate has no net dependency).
pub(crate) use engine_arena::FighterTape as BodyTape;

/// Stuck window: 3 s of sub-threshold speed at [`TICK_HZ`].
pub const STUCK_WINDOW_TICKS: u32 = 3 * TICK_HZ;

/// Samples per stuck verdict: [`STUCK_WINDOW_TICKS`] consecutive speeds.
const STUCK_SAMPLES: usize = STUCK_WINDOW_TICKS as usize;

/// Bounded lines per job log; oldest lines yield to newer ones.
pub const MAX_LOG_LINES: usize = 16;

/// Tape codec magic: ASCII `PTA1` (playtest agent tape v1).
const TAPE_MAGIC: [u8; 4] = [0x50, 0x54, 0x41, 0x31];
/// One canonical input is 10 bytes (see [`FighterInput::append_bytes`]).
const INPUT_BYTES: usize = 10;

/// Which backend the [`Driver`] stands in for. Labels only: the agent runs
/// the same pipeline for every backend; swapping backends swaps the driver.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backend {
    /// In-process deterministic stand-in.
    Mock,
    /// Local model sidecar.
    Local,
    /// Remote model service.
    Remote,
}

/// Plain driver limits plus a backend label. No provider import: caps travel
/// as data so mock/local/remote differ only in the [`Driver`] value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Caps {
    /// Which backend this driver stands in for.
    pub backend: Backend,
    /// Largest match this driver runs; larger requests expire with a log.
    pub max_ticks: u32,
    /// Largest tournament this driver runs; [`Agent::drive`] truncates past it.
    pub max_matches: u32,
}

impl Caps {
    /// Headless-friendly defaults: short matches, small tournaments.
    pub fn mock() -> Self {
        Self {
            backend: Backend::Mock,
            max_ticks: 3600,
            max_matches: 8,
        }
    }
}

/// How the [`Driver`] builds tapes. Seeded-bug policies are genuine arena
/// inputs, never observation-layer fakes: [`Policy::Stall`] pins a fighter
/// with real idle wishes, [`Policy::StallAll`] pins every slot (a lone
/// pinned fighter would still be hit by the brain-driven slots, so a
/// stillness window needs the whole match idle), and [`Policy::Divert`]
/// changes one real probe input (hashes genuinely diverge downstream).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Policy {
    /// Empty tapes: every slot brain-driven (the clean reference).
    Brains,
    /// Pin one fighter with idle wishes every tick; all other slots
    /// brain-driven. Models a partially stuck build; the pinned slot can
    /// still be moved by brain-driven hits, so stillness is not guaranteed.
    Stall {
        /// Fighter slot to pin, clamped into `0..fighters`.
        fighter: u8,
    },
    /// Pin every slot with idle wishes every tick. The sim truly holds
    /// still, so stillness windows trip deterministically.
    StallAll,
    /// Control runs brains; the probe overrides one real input. Models a
    /// desyncing build under test.
    Divert {
        /// Probe tick to override, clamped into `0..ticks`.
        tick: u32,
        /// Probe slot to override, clamped into `0..fighters`.
        fighter: u8,
    },
}

/// Provider-neutral match driver: a plain `{ policy, caps }` value. The
/// agent asks it for control/probe tapes only; it never sees provider
/// request types, SDKs or network handles.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Driver {
    /// Tape-building policy (seeded bugs included).
    pub policy: Policy,
    /// Backend label plus limits.
    pub caps: Caps,
}

impl Driver {
    /// Mock driver with brain-driven tapes.
    pub fn mock() -> Self {
        Self {
            policy: Policy::Brains,
            caps: Caps::mock(),
        }
    }

    /// Control tape for `config`: brains, or a pinned fighter under
    /// [`Policy::Stall`]. Always valid by construction; falls back to brains
    /// when no template input records (defensive only: valid configs always
    /// record at least one row).
    pub fn control_tape(&self, config: &ArenaConfig) -> FighterTape {
        match self.policy {
            Policy::Brains | Policy::Divert { .. } => FighterTape::default(),
            Policy::Stall { fighter } => {
                let slot = clamp_slot(fighter, config.fighters);
                Self::idle_tape(config, &[slot])
            }
            Policy::StallAll => {
                let slots: Vec<u8> = (0..config.fighters).collect();
                Self::idle_tape(config, &slots)
            }
        }
    }

    /// Idle wishes for `slots` every tick; empty tape when no template
    /// input records (defensive only: valid configs always record one).
    fn idle_tape(config: &ArenaConfig, slots: &[u8]) -> FighterTape {
        let Some(template) = template_input(config) else {
            return FighterTape::default();
        };
        let mut inputs = Vec::new();
        for tick in 0..config.ticks {
            for slot in slots {
                let Some(id) = FighterId::new(*slot) else {
                    continue;
                };
                inputs.push(FighterInput {
                    tick,
                    fighter: id,
                    mx: 0,
                    mz: 0,
                    ..template
                });
            }
        }
        FighterTape(inputs)
    }

    /// Probe tape for `config`: a clone of `control`, except under
    /// [`Policy::Divert`], which overrides one real input with a hard
    /// sideways wish so downstream hashes diverge.
    pub fn probe_tape(&self, config: &ArenaConfig, control: &FighterTape) -> FighterTape {
        match self.policy {
            Policy::Brains | Policy::Stall { .. } | Policy::StallAll => control.clone(),
            Policy::Divert { tick, fighter } => {
                let Some(template) = template_input(config) else {
                    return control.clone();
                };
                let slot = clamp_slot(fighter, config.fighters);
                let Some(id) = FighterId::new(slot) else {
                    return control.clone();
                };
                let at = tick.min(config.ticks.saturating_sub(1));
                let mut inputs = control.0.clone();
                inputs.retain(|input| !(input.tick == at && input.fighter.0 == slot));
                inputs.push(FighterInput {
                    tick: at,
                    fighter: id,
                    mx: 100,
                    mz: 0,
                    ..template
                });
                inputs.sort_by_key(|input| (input.tick, input.fighter.0));
                FighterTape(inputs)
            }
        }
    }
}

/// Clamp a policy slot into `0..fighters`.
fn clamp_slot(fighter: u8, fighters: u8) -> u8 {
    if fighters == 0 {
        0
    } else {
        fighter.min(fighters.saturating_sub(1))
    }
}

/// One validated template input recorded on `config` with an empty tape:
/// the tick-0 brain decision both seeded tapes derive from. `None` on
/// invalid configs only (valid configs always record at least one row).
pub(crate) fn template_input(config: &ArenaConfig) -> Option<FighterInput> {
    engine_arena::run_dataset(config, &FighterTape::default())?
        .rows
        .into_iter()
        .next()
        .map(|row| row.action.input)
}

/// Agent match configuration: one headless arena match per seed.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AgentConfig {
    /// First seed; match `i` uses `base_seed.wrapping_add(i)`.
    pub base_seed: u64,
    /// Fixed ticks per match (`1..=engine_arena::MAX_TICKS`).
    pub ticks: u32,
    /// Fighters per match (`2..=MAX_PARTICIPANTS`).
    pub fighters: u8,
    /// Arena half-extent in meters (`5.0..=100.0`).
    pub half_extent_m: f32,
}

/// Agent configuration failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AgentError {
    /// Tick count `0` or above `MAX_TICKS`, culprit carried.
    BadTicks(u32),
    /// Fighter count outside `2..=MAX_PARTICIPANTS`, culprit carried.
    BadFighters(u8),
    /// Half-extent NaN or outside `5.0..=100.0` meters.
    BadExtent,
}

impl AgentConfig {
    /// Range-check every field; NaN or out-of-range fails.
    pub fn validate(self) -> Result<(), AgentError> {
        if self.ticks == 0 || self.ticks > engine_arena::MAX_TICKS {
            return Err(AgentError::BadTicks(self.ticks));
        }
        if self.fighters < 2 || self.fighters > MAX_PARTICIPANTS {
            return Err(AgentError::BadFighters(self.fighters));
        }
        if !self.half_extent_m.is_finite() || self.half_extent_m < 5.0 || self.half_extent_m > 100.0
        {
            return Err(AgentError::BadExtent);
        }
        Ok(())
    }

    /// Match `index` as an [`ArenaConfig`].
    fn match_config(self, index: u32) -> ArenaConfig {
        ArenaConfig {
            seed: self.base_seed.wrapping_add(u64::from(index)),
            ticks: self.ticks,
            fighters: self.fighters,
            half_extent_m: self.half_extent_m,
            table_version: 1,
        }
    }
}

/// One watched metric that filed a bug.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Metric {
    /// A fighter held below [`STUCK_SPEED_MPS`](engine_arena::fork::STUCK_SPEED_MPS)
    /// for [`STUCK_WINDOW_TICKS`] consecutive ticks.
    Stuck {
        /// Stuck fighter slot.
        fighter: u8,
    },
    /// Probe checkpoints diverged from control; expected/observed hashes at
    /// the first divergent tick carried.
    Desync {
        /// Control hash at the first divergent tick.
        expected: u64,
        /// Probe hash at the first divergent tick.
        observed: u64,
    },
    /// An impossible fighter pose entered the trajectory.
    Impossible {
        /// Offending fighter slot.
        fighter: u8,
        /// What made the pose impossible.
        kind: ImpossibleKind,
    },
}

/// Impossible-pose kinds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImpossibleKind {
    /// Non-finite position component.
    NonFinite,
    /// Planar position outside the arena half-extent.
    OutOfBounds,
}

/// One filed bug: the exact bytes to replay it, the first bad tick, and the
/// watched metric that fired. `tape_bytes` is the [`TAPE_MAGIC`] encoding of
/// the match config plus control/probe tapes, so [`BugFile::replay`] reruns
/// the identical match and finds the identical bug.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BugFile {
    /// Canonical match bytes (config plus both tapes).
    pub tape_bytes: Vec<u8>,
    /// First tick the metric fired.
    pub first_bad_tick: u32,
    /// Which watcher filed this bug.
    pub metric: Metric,
}

impl BugFile {
    /// Replay the filed match and return every bug the watchers file.
    /// Deterministic: replaying a filed bug always contains the filed bug
    /// itself. Empty when the bytes are malformed, never a panic.
    pub fn replay(&self) -> Vec<BugFile> {
        let Some((config, control, probe)) = decode_tape(&self.tape_bytes) else {
            return Vec::new();
        };
        let watched = Watched {
            control: engine_arena::run(&config, &control),
            probe: engine_arena::run(&config, &probe),
            trajectory: record_trajectory(&config, &probe),
        };
        watch_match(&config, &control, &probe, &watched)
    }
}

/// Agent outcome over one [`Agent::drive`] call. Bugs arrive in match order;
/// within a match, desync files first, then stuck per slot, then impossible
/// per slot — so the same seed twice gives the identical report.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AgentReport {
    /// First seed driven.
    pub base_seed: u64,
    /// Matches requested.
    pub requested: u32,
    /// Matches actually run (truncated at [`Caps::max_matches`]).
    pub ran: u32,
    /// Every filed bug, in match then watcher order.
    pub bugs: Vec<BugFile>,
}

/// Opaque play.run-style job handle, handed out in increasing order.
pub type JobId = u64;

/// Lifecycle of one playtest job. Mirrors the trigger job shape plus an
/// expiry terminal: oversize or invalid submissions finish as `Expired`
/// without simulating.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JobStatus {
    /// Waiting for the running slot.
    Queued,
    /// Owning the running slot, advancing one stage per poll.
    Running,
    /// All stages finished; the completion carries the watched data.
    Done,
    /// Cancelled before completion; never yields a completion.
    Cancelled,
    /// Rejected without simulating (invalid config or over driver caps).
    Expired,
}

impl JobStatus {
    /// True while the job still holds a queue or running slot.
    pub(crate) const fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }
}

/// Bounded pipeline stages: one [`PlayRunner::poll`] call advances the
/// running job by exactly one stage, so the caller never waits on a full
/// match.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    /// Full control run (`run` over the control tape).
    Control,
    /// Full probe run (`run` over the probe tape).
    Probe,
    /// Fork-style trajectory (prefix snapshot plus recorded fork).
    Trajectory,
}

/// One finished job: the watched data the agent scans for bugs.
#[derive(Clone, Debug)]
pub struct JobDone {
    /// Finished job handle.
    id: JobId,
    /// Match configuration (seed carried).
    config: ArenaConfig,
    /// Control tape (canonical bytes source).
    control_tape: FighterTape,
    /// Probe tape (canonical bytes source).
    probe_tape: FighterTape,
    /// Control checkpoints, one per tick.
    control: Vec<Checkpoint>,
    /// Probe checkpoints, one per tick.
    probe: Vec<Checkpoint>,
    /// Probe trajectory, one pose row per tick per fighter.
    trajectory: Trajectory,
}

/// Internal per-job state.
struct InnerJob {
    id: JobId,
    config: ArenaConfig,
    control_tape: FighterTape,
    probe_tape: FighterTape,
    status: JobStatus,
    stage: Stage,
    control: Vec<Checkpoint>,
    probe: Vec<Checkpoint>,
    trajectory: Trajectory,
    log: Vec<String>,
}

/// Local stub of the endpoint `play.run` job shape: submit/poll/cancel/expiry
/// with bounded logs as plain data. Single-threaded and deterministic: jobs
/// advance in [`JobId`] order, one stage per poll.
pub struct PlayRunner {
    next_id: JobId,
    caps: Caps,
    jobs: Vec<InnerJob>,
}

impl PlayRunner {
    /// Empty runner enforcing `caps`.
    pub fn new(caps: Caps) -> Self {
        Self {
            next_id: 0,
            caps,
            jobs: Vec::new(),
        }
    }

    /// Submit one match. Invalid configs and over-cap tick counts expire
    /// immediately with a bounded log line and never simulate.
    pub fn submit(
        &mut self,
        config: ArenaConfig,
        control_tape: FighterTape,
        probe_tape: FighterTape,
    ) -> JobId {
        let id = self.next_id;
        self.next_id += 1;
        let mut log = Vec::new();
        let status = if config.validate().is_err() {
            push_log(&mut log, "expired: invalid match config");
            JobStatus::Expired
        } else if config.ticks > self.caps.max_ticks {
            push_log(
                &mut log,
                &format!(
                    "expired: {} ticks over cap {}",
                    config.ticks, self.caps.max_ticks
                ),
            );
            JobStatus::Expired
        } else {
            push_log(&mut log, "queued");
            JobStatus::Queued
        };
        self.jobs.push(InnerJob {
            id,
            config,
            control_tape,
            probe_tape,
            status,
            stage: Stage::Control,
            control: Vec::new(),
            probe: Vec::new(),
            trajectory: Trajectory::empty(),
            log,
        });
        id
    }

    /// Cancel a queued or running job. Terminal and unknown ids return
    /// `false` and change nothing.
    pub fn cancel(&mut self, id: JobId) -> bool {
        match self.jobs.iter_mut().find(|job| job.id == id) {
            Some(job) if job.status.is_active() => {
                job.status = JobStatus::Cancelled;
                push_log(&mut job.log, "cancelled");
                true
            }
            _ => false,
        }
    }

    /// Observable status snapshot, or `None` for unknown ids.
    pub fn status(&self, id: JobId) -> Option<JobStatus> {
        self.jobs
            .iter()
            .find(|job| job.id == id)
            .map(|job| job.status)
    }

    /// Bounded stage log for one job, oldest first, or `None` for unknown ids.
    pub fn log(&self, id: JobId) -> Option<&[String]> {
        self.jobs
            .iter()
            .find(|job| job.id == id)
            .map(|job| job.log.as_slice())
    }

    /// Advance the oldest active job by one stage and return completions
    /// (zero or one). Expired jobs surface no completion.
    pub fn poll(&mut self) -> Vec<JobDone> {
        let running = self
            .jobs
            .iter()
            .filter(|job| job.status == JobStatus::Running)
            .min_by_key(|job| job.id)
            .map(|job| job.id);
        let target = match running {
            Some(id) => Some(id),
            None => {
                let next = self
                    .jobs
                    .iter_mut()
                    .filter(|job| job.status == JobStatus::Queued)
                    .min_by_key(|job| job.id)
                    .map(|job| job.id);
                if let Some(id) = next {
                    if let Some(job) = self.jobs.iter_mut().find(|job| job.id == id) {
                        job.status = JobStatus::Running;
                        push_log(&mut job.log, "running");
                    }
                }
                next
            }
        };
        let Some(id) = target else {
            return Vec::new();
        };
        // One bounded stage of real match work.
        let finished = {
            let job = self
                .jobs
                .iter_mut()
                .find(|job| job.id == id)
                .expect("poll target exists");
            match job.stage {
                Stage::Control => {
                    job.control = engine_arena::run(&job.config, &job.control_tape);
                    push_log(
                        &mut job.log,
                        &format!("control: {} checkpoints", job.control.len()),
                    );
                    job.stage = Stage::Probe;
                    false
                }
                Stage::Probe => {
                    job.probe = engine_arena::run(&job.config, &job.probe_tape);
                    push_log(
                        &mut job.log,
                        &format!("probe: {} checkpoints", job.probe.len()),
                    );
                    job.stage = Stage::Trajectory;
                    false
                }
                Stage::Trajectory => {
                    job.trajectory = record_trajectory(&job.config, &job.probe_tape);
                    push_log(
                        &mut job.log,
                        &format!("trajectory: {} ticks", job.trajectory.ticks),
                    );
                    true
                }
            }
        };
        if !finished {
            return Vec::new();
        }
        let job = self
            .jobs
            .iter_mut()
            .find(|job| job.id == id)
            .expect("poll target exists");
        if job.status == JobStatus::Cancelled {
            return Vec::new();
        }
        job.status = JobStatus::Done;
        push_log(&mut job.log, "done");
        vec![JobDone {
            id: job.id,
            config: job.config,
            control_tape: core::mem::take(&mut job.control_tape),
            probe_tape: core::mem::take(&mut job.probe_tape),
            control: core::mem::take(&mut job.control),
            probe: core::mem::take(&mut job.probe),
            trajectory: core::mem::take(&mut job.trajectory),
        }]
    }

    /// Active (`Queued` + `Running`) job count.
    pub fn active_count(&self) -> usize {
        self.jobs
            .iter()
            .filter(|job| job.status.is_active())
            .count()
    }
}

/// Push one log line, keeping the log within [`MAX_LOG_LINES`] by dropping
/// the oldest line first.
fn push_log(log: &mut Vec<String>, line: &str) {
    if log.len() >= MAX_LOG_LINES {
        log.remove(0);
    }
    log.push(line.to_string());
}

/// Per-tick absolute poses for one match: `pos[tick][slot]`.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Trajectory {
    /// Ticks with poses (rows observed).
    pub ticks: u32,
    /// Fighter slots per tick.
    pub fighters: u8,
    /// Arena half-extent in meters (the out-of-bounds bound).
    pub half_extent_m: f32,
    /// Absolute feet positions in meters, tick-major.
    pub pos: Vec<Vec<[f32; 3]>>,
}

impl Trajectory {
    /// Empty trajectory with no ticks.
    pub fn empty() -> Self {
        Self::default()
    }
}

/// Record one probe trajectory through a fork-style snapshot: prefix at tick
/// 0, then a recorded fork over the whole match. Each dataset row carries
/// the tick's pre-step poses for every fighter, so the rows are the watched
/// trajectory. Misshapen data (bad version, ragged rows) maps to an empty
/// trajectory, never a panic.
fn record_trajectory(config: &ArenaConfig, probe: &FighterTape) -> Trajectory {
    let Some((snapshot, _)) = engine_arena::run_prefix(config, probe, 0) else {
        return Trajectory::empty();
    };
    let Some(mut fork) = Fork::from_snapshot(snapshot, 0) else {
        return Trajectory::empty();
    };
    // Fresh forks start with an empty tape (brains would fill every slot),
    // so swap each fighter's probe slice in; slots without inputs stay
    // brain-driven, exactly like the live match.
    for slot in 0..config.fighters {
        let slice: Vec<FighterInput> = probe
            .0
            .iter()
            .filter(|input| input.fighter.0 == slot)
            .copied()
            .collect();
        if slice.is_empty() {
            continue;
        }
        if fork
            .apply_proposal(ForkProposal::SwapTape {
                fighter: slot,
                inputs: slice,
            })
            .is_err()
        {
            return Trajectory::empty();
        }
    }
    let (_, dataset) = fork.run_recorded(config.ticks);
    if dataset.version != DATASET_VERSION {
        return Trajectory::empty();
    }
    let fighters = usize::from(config.fighters);
    if dataset.rows.len() != config.ticks as usize * fighters {
        return Trajectory::empty();
    }
    let mut pos = Vec::with_capacity(config.ticks as usize);
    for (tick, chunk) in dataset.rows.chunks(fighters).enumerate() {
        if chunk.iter().any(|row| row.observation.tick != tick as u32) {
            return Trajectory::empty();
        }
        if chunk
            .iter()
            .any(|row| row.observation.fighters.len() != fighters)
        {
            return Trajectory::empty();
        }
        pos.push(
            chunk[0]
                .observation
                .fighters
                .iter()
                .map(|entry| entry.pos)
                .collect(),
        );
    }
    Trajectory {
        ticks: config.ticks,
        fighters: config.fighters,
        half_extent_m: config.half_extent_m,
        pos,
    }
}

/// First tick where `control` and `probe` checkpoints differ. Length
/// mismatch counts as divergence at the shared prefix end.
pub fn scan_desync(control: &[Checkpoint], probe: &[Checkpoint]) -> Option<u32> {
    for (index, (left, right)) in control.iter().zip(probe.iter()).enumerate() {
        if left != right {
            return Some(index as u32);
        }
    }
    if control.len() != probe.len() {
        return Some(control.len().min(probe.len()) as u32);
    }
    None
}

/// Stuck fighters: planar speed below
/// [`STUCK_SPEED_MPS`](engine_arena::fork::STUCK_SPEED_MPS) for
/// [`STUCK_WINDOW_TICKS`] consecutive ticks. Speed sample `t` spans poses
/// `t` and `t + 1`; the reported tick is the window's last sample (the tick
/// the condition trips). One bug per fighter (the first window), in slot
/// order. Ragged rows reset the window; they never panic.
pub fn scan_stuck(trajectory: &Trajectory) -> Vec<(u8, u32)> {
    let step_m = engine_arena::fork::STUCK_SPEED_MPS / TICK_HZ as f32;
    let stuck_sq = step_m * step_m;
    let mut out = Vec::new();
    for slot in 0..usize::from(trajectory.fighters) {
        let mut run = 0usize;
        let mut trip: Option<u32> = None;
        for (sample, window) in trajectory.pos.windows(2).enumerate() {
            let (Some(first), Some(second)) = (window[0].get(slot), window[1].get(slot)) else {
                run = 0;
                continue;
            };
            let (dx, dz) = (second[0] - first[0], second[2] - first[2]);
            if dx * dx + dz * dz < stuck_sq {
                run += 1;
                if run == STUCK_SAMPLES {
                    trip = Some(sample as u32);
                    break;
                }
            } else {
                run = 0;
            }
        }
        if let Some(tick) = trip {
            out.push((slot as u8, tick));
        }
    }
    out
}

/// Impossible poses: non-finite components or planar positions outside the
/// arena half-extent. First bad tick per fighter, in slot order.
pub fn scan_impossible(trajectory: &Trajectory) -> Vec<(u32, u8, ImpossibleKind)> {
    let mut out = Vec::new();
    for slot in 0..usize::from(trajectory.fighters) {
        for (tick, poses) in trajectory.pos.iter().enumerate() {
            let Some(pose) = poses.get(slot) else {
                continue;
            };
            if pose.iter().any(|axis| !axis.is_finite()) {
                out.push((tick as u32, slot as u8, ImpossibleKind::NonFinite));
                break;
            }
            if pose[0].abs() > trajectory.half_extent_m || pose[2].abs() > trajectory.half_extent_m
            {
                out.push((tick as u32, slot as u8, ImpossibleKind::OutOfBounds));
                break;
            }
        }
    }
    out
}

/// Watched data for one match (checkpoints plus trajectory).
struct Watched {
    control: Vec<Checkpoint>,
    probe: Vec<Checkpoint>,
    trajectory: Trajectory,
}

/// Watch one finished match and file every bug: desync first, then stuck per
/// slot, then impossible per slot.
fn watch_match(
    config: &ArenaConfig,
    control_tape: &FighterTape,
    probe_tape: &FighterTape,
    done: &Watched,
) -> Vec<BugFile> {
    let tape_bytes = encode_tape(config, control_tape, probe_tape);
    let mut bugs = Vec::new();
    if let Some(tick) = scan_desync(&done.control, &done.probe) {
        if let (Some(expected), Some(observed)) = (
            done.control.get(tick as usize).map(|point| point.hash),
            done.probe.get(tick as usize).map(|point| point.hash),
        ) {
            bugs.push(BugFile {
                tape_bytes: tape_bytes.clone(),
                first_bad_tick: tick,
                metric: Metric::Desync { expected, observed },
            });
        }
    }
    for (fighter, tick) in scan_stuck(&done.trajectory) {
        bugs.push(BugFile {
            tape_bytes: tape_bytes.clone(),
            first_bad_tick: tick,
            metric: Metric::Stuck { fighter },
        });
    }
    for (tick, fighter, kind) in scan_impossible(&done.trajectory) {
        bugs.push(BugFile {
            tape_bytes: tape_bytes.clone(),
            first_bad_tick: tick,
            metric: Metric::Impossible { fighter, kind },
        });
    }
    bugs
}

/// The AI playtest agent: drives headless arena matches through
/// [`PlayRunner`] jobs and files a [`BugFile`] per watched metric.
pub struct Agent {
    /// Match configuration (seeds, ticks, fighters, extent).
    pub config: AgentConfig,
    /// Backend driver under test.
    pub driver: Driver,
}

impl Agent {
    /// Drive `matches` headless matches and return the report. Truncated at
    /// [`Caps::max_matches`]; invalid configs expire their jobs and file
    /// nothing. Deterministic: the same seed twice gives the identical
    /// report.
    pub fn drive(&self, matches: u32) -> AgentReport {
        let ran = matches.min(self.driver.caps.max_matches);
        let mut runner = PlayRunner::new(self.driver.caps);
        for index in 0..ran {
            let config = self.config.match_config(index);
            let control = self.driver.control_tape(&config);
            let probe = self.driver.probe_tape(&config, &control);
            runner.submit(config, control, probe);
        }
        let mut done: Vec<JobDone> = Vec::new();
        while runner.active_count() > 0 {
            done.extend(runner.poll());
        }
        // Completions already arrive in `JobId` (submit) order from the
        // single running slot; re-sort by handle so the order is explicit.
        done.sort_by_key(|completion| completion.id);
        let mut bugs = Vec::new();
        for completion in &done {
            let watched = Watched {
                control: completion.control.clone(),
                probe: completion.probe.clone(),
                trajectory: completion.trajectory.clone(),
            };
            bugs.extend(watch_match(
                &completion.config,
                &completion.control_tape,
                &completion.probe_tape,
                &watched,
            ));
        }
        AgentReport {
            base_seed: self.config.base_seed,
            requested: matches,
            ran,
            bugs,
        }
    }
}

/// Encode one match canonically: magic, config, then both tapes with counts
/// and 10-byte canonical inputs.
fn encode_tape(config: &ArenaConfig, control: &FighterTape, probe: &FighterTape) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&TAPE_MAGIC);
    bytes.extend_from_slice(&config.seed.to_le_bytes());
    bytes.extend_from_slice(&config.ticks.to_le_bytes());
    bytes.push(config.fighters);
    bytes.extend_from_slice(&config.half_extent_m.to_bits().to_le_bytes());
    for tape in [control, probe] {
        bytes.extend_from_slice(&(tape.0.len() as u32).to_le_bytes());
        for input in &tape.0 {
            input.append_bytes(&mut bytes);
        }
    }
    bytes
}

/// Decode [`encode_tape`] bytes: magic, exact length, valid config and
/// valid tapes within arena bounds. `None` on any malformed input, never a
/// panic. `pub(crate)` for the causal replay, which attributes the probe
/// tape against the control tape.
pub(crate) fn decode_tape(bytes: &[u8]) -> Option<(ArenaConfig, FighterTape, FighterTape)> {
    if bytes.len() < 4 + 8 + 4 + 1 + 4 {
        return None;
    }
    if bytes[0..4] != TAPE_MAGIC {
        return None;
    }
    let mut cursor = 4usize;
    let seed = u64::from_le_bytes(bytes[cursor..cursor + 8].try_into().ok()?);
    cursor += 8;
    let ticks = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().ok()?);
    cursor += 4;
    let fighters = bytes[cursor];
    cursor += 1;
    let half_extent_m = f32::from_bits(u32::from_le_bytes(
        bytes[cursor..cursor + 4].try_into().ok()?,
    ));
    cursor += 4;
    let config = ArenaConfig {
        seed,
        ticks,
        fighters,
        half_extent_m,
        table_version: 1,
    };
    if config.validate().is_err() {
        return None;
    }
    let mut tapes = Vec::with_capacity(2);
    for _ in 0..2 {
        if bytes.len() < cursor + 4 {
            return None;
        }
        let count = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().ok()?) as usize;
        cursor += 4;
        if count > engine_arena::MAX_TICKS as usize * usize::from(MAX_PARTICIPANTS) {
            return None;
        }
        if bytes.len() < cursor + count * INPUT_BYTES {
            return None;
        }
        let mut inputs = Vec::with_capacity(count);
        for _ in 0..count {
            let chunk = &bytes[cursor..cursor + INPUT_BYTES];
            cursor += INPUT_BYTES;
            let tick = u32::from_le_bytes(chunk[0..4].try_into().ok()?);
            let fighter = FighterId::new(chunk[4])?;
            let facing = i16::from_le_bytes(chunk[7..9].try_into().ok()?);
            // Buttons travel as the raw byte; validation below rejects
            // unknown bits. Rebuild without naming the button type: copy the
            // byte through a template input's buttons field.
            inputs.push((
                tick,
                fighter,
                chunk[5] as i8,
                chunk[6] as i8,
                facing,
                chunk[9],
            ));
        }
        // Materialize inputs through the template so the button byte never
        // needs a named type in this crate.
        let template = template_input(&config)?;
        let mut materialized = Vec::with_capacity(count);
        for (tick, fighter, mx, mz, facing, buttons) in inputs {
            let mut input = FighterInput {
                tick,
                fighter,
                mx,
                mz,
                facing,
                ..template
            };
            input.buttons.0 = buttons;
            materialized.push(input);
        }
        let tape = FighterTape(materialized);
        if tape.validate().is_err() {
            return None;
        }
        // Arena-level bounds mirror `run`: every tape tick below ticks and
        // every slot below fighters.
        if tape
            .0
            .iter()
            .any(|input| input.tick >= ticks || input.fighter.0 >= fighters)
        {
            return None;
        }
        tapes.push(tape);
    }
    if cursor != bytes.len() {
        return None;
    }
    Some((config, tapes.remove(0), tapes.remove(0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent_config() -> AgentConfig {
        AgentConfig {
            base_seed: 2040,
            ticks: 400,
            fighters: 2,
            half_extent_m: 20.0,
        }
    }

    fn arena_config() -> ArenaConfig {
        ArenaConfig {
            seed: 2040,
            ticks: 400,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: 1,
        }
    }

    /// Crafted trajectory: fighter 0 walks 1 m/s for 10 ticks then holds
    /// still; fighter 1 always walks. Speeds come from consecutive poses.
    fn crafted_stuck() -> Trajectory {
        let mut pos = Vec::new();
        for tick in 0..200u32 {
            let walker = tick as f32 / TICK_HZ as f32;
            let stalled = (tick.min(10)) as f32 / TICK_HZ as f32;
            pos.push(vec![[stalled, 0.0, 0.0], [walker, 0.0, 0.0]]);
        }
        Trajectory {
            ticks: 200,
            fighters: 2,
            half_extent_m: 20.0,
            pos,
        }
    }

    #[test]
    fn stuck_scan_trips_at_the_window_end() {
        // Fighter 0 moves for samples 0..10, then holds: the first 180-long
        // still window starts at sample 10 and trips at sample 189.
        assert_eq!(scan_stuck(&crafted_stuck()), vec![(0, 189)]);
    }

    #[test]
    fn stuck_scan_silent_without_a_full_window() {
        let mut trajectory = crafted_stuck();
        trajectory.pos.truncate(100);
        trajectory.ticks = 100;
        assert!(scan_stuck(&trajectory).is_empty());
    }

    #[test]
    fn impossible_scan_flags_nan_and_out_of_bounds() {
        let mut trajectory = crafted_stuck();
        trajectory.pos[5][1][0] = f32::NAN;
        assert_eq!(
            scan_impossible(&trajectory),
            vec![(5, 1, ImpossibleKind::NonFinite)]
        );
        let mut trajectory = crafted_stuck();
        trajectory.pos[7][0][0] = 25.0;
        assert_eq!(
            scan_impossible(&trajectory),
            vec![(7, 0, ImpossibleKind::OutOfBounds)]
        );
        assert!(scan_impossible(&crafted_stuck()).is_empty());
    }

    #[test]
    fn desync_scan_names_the_first_divergent_tick() {
        let config = arena_config();
        let tape = FighterTape::default();
        let control = engine_arena::run(&config, &tape);
        assert!(!control.is_empty());
        assert_eq!(scan_desync(&control, &control), None);
        let mut probe = control.clone();
        probe[3].hash ^= 1;
        assert_eq!(scan_desync(&control, &probe), Some(3));
        let short = &control[..control.len() - 1];
        assert_eq!(scan_desync(&control, short), Some(short.len() as u32));
    }

    #[test]
    fn tape_codec_roundtrips_and_rejects_garbage() {
        let config = arena_config();
        let driver = Driver {
            policy: Policy::Divert {
                tick: 50,
                fighter: 0,
            },
            caps: Caps::mock(),
        };
        let control = driver.control_tape(&config);
        let probe = driver.probe_tape(&config, &control);
        control.validate().expect("driver tapes validate");
        probe.validate().expect("driver tapes validate");
        let bytes = encode_tape(&config, &control, &probe);
        let (back_config, back_control, back_probe) =
            decode_tape(&bytes).expect("roundtrip decodes");
        assert_eq!(back_config, config);
        assert_eq!(back_control, control);
        assert_eq!(back_probe, probe);
        // Garbage: bad magic, truncation, trailing bytes, bad config.
        let mut bad = bytes.clone();
        bad[0] ^= 0xff;
        assert!(decode_tape(&bad).is_none());
        assert!(decode_tape(&bytes[..bytes.len() - 1]).is_none());
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(decode_tape(&trailing).is_none());
        assert!(decode_tape(&bytes[..8]).is_none());
        let mut bad_ticks = bytes.clone();
        bad_ticks[12..16].copy_from_slice(&0u32.to_le_bytes());
        assert!(decode_tape(&bad_ticks).is_none());
    }

    #[test]
    fn seeded_stuck_fighter_is_found_with_tick_and_replaying_tape() {
        let agent = Agent {
            config: agent_config(),
            driver: Driver {
                policy: Policy::StallAll,
                caps: Caps::mock(),
            },
        };
        let report = agent.drive(1);
        assert_eq!((report.requested, report.ran), (1, 1));
        // Every slot idles from tick 0 with no brain-driven slot to disturb
        // them: every speed sample is zero, so the first 180-still window
        // trips at sample 179 for each fighter.
        let stuck = report
            .bugs
            .iter()
            .find(|bug| bug.metric == Metric::Stuck { fighter: 1 })
            .expect("stalled fighter files stuck");
        assert_eq!(stuck.first_bad_tick, STUCK_WINDOW_TICKS - 1);
        // The filed tape replays exactly to the same bug.
        let replayed = stuck.replay();
        assert!(
            replayed.contains(stuck),
            "replay must reproduce the filed bug"
        );
    }

    #[test]
    fn desync_is_diagnosed_with_first_divergent_tick() {
        let agent = Agent {
            config: agent_config(),
            driver: Driver {
                policy: Policy::Divert {
                    tick: 50,
                    fighter: 0,
                },
                caps: Caps::mock(),
            },
        };
        let report = agent.drive(1);
        let desync = report
            .bugs
            .iter()
            .find(|bug| matches!(bug.metric, Metric::Desync { .. }))
            .expect("diverted probe files desync");
        assert!(
            desync.first_bad_tick >= 50,
            "divergence cannot predate the diverted tick: {}",
            desync.first_bad_tick
        );
        let replayed = desync.replay();
        assert!(
            replayed.contains(desync),
            "replay must reproduce the desync with its tick"
        );
    }

    #[test]
    fn clean_match_files_nothing() {
        let agent = Agent {
            config: AgentConfig {
                ticks: 300,
                ..agent_config()
            },
            driver: Driver::mock(),
        };
        let report = agent.drive(2);
        assert_eq!((report.requested, report.ran), (2, 2));
        assert!(
            report.bugs.is_empty(),
            "clean brains file nothing: {:?}",
            report.bugs
        );
    }

    #[test]
    fn same_seed_twice_gives_identical_reports() {
        for policy in [
            Policy::Brains,
            Policy::Stall { fighter: 0 },
            Policy::StallAll,
            Policy::Divert {
                tick: 50,
                fighter: 1,
            },
        ] {
            let agent = Agent {
                config: agent_config(),
                driver: Driver {
                    policy,
                    caps: Caps::mock(),
                },
            };
            let (first, second) = (agent.drive(2), agent.drive(2));
            assert_eq!(first, second, "policy {policy:?} must repeat exactly");
        }
    }

    #[test]
    fn runner_cancels_and_expires_with_bounded_logs() {
        let caps = Caps::mock();
        let mut runner = PlayRunner::new(caps);
        let config = arena_config();
        let tape = FighterTape::default();
        let first = runner.submit(config, tape.clone(), tape.clone());
        let second = runner.submit(config, tape.clone(), tape.clone());
        assert!(runner.cancel(second), "queued cancel must succeed");
        assert_eq!(runner.status(second), Some(JobStatus::Cancelled));
        assert!(!runner.cancel(second), "second cancel is a no-op");
        assert!(!runner.cancel(4242), "unknown id cancels nothing");
        let mut done = Vec::new();
        while runner.active_count() > 0 {
            done.extend(runner.poll());
        }
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].id, first);
        assert_eq!(runner.status(first), Some(JobStatus::Done));
        let log = runner.log(first).expect("finished job keeps its log");
        assert!(log.len() <= MAX_LOG_LINES, "logs stay bounded");
        assert!(log.iter().any(|line| line.as_str() == "done"));

        // Oversize and invalid submissions expire without simulating.
        let mut runner = PlayRunner::new(caps);
        let oversized = ArenaConfig {
            ticks: caps.max_ticks + 1,
            ..config
        };
        let expired = runner.submit(oversized, tape.clone(), tape.clone());
        assert_eq!(runner.status(expired), Some(JobStatus::Expired));
        let invalid = ArenaConfig {
            fighters: 17,
            ..config
        };
        let rejected = runner.submit(invalid, tape.clone(), tape.clone());
        assert_eq!(runner.status(rejected), Some(JobStatus::Expired));
        assert_eq!(runner.active_count(), 0);
        assert!(runner.poll().is_empty(), "expired jobs never complete");
    }

    #[test]
    fn invalid_agent_config_expires_and_files_nothing() {
        let agent = Agent {
            config: AgentConfig {
                ticks: 0,
                ..agent_config()
            },
            driver: Driver::mock(),
        };
        assert!(agent.config.validate().is_err());
        let report = agent.drive(1);
        assert_eq!((report.requested, report.ran), (1, 1));
        assert!(report.bugs.is_empty());
    }

    #[test]
    fn drive_truncates_past_max_matches() {
        let agent = Agent {
            config: agent_config(),
            driver: Driver {
                policy: Policy::Brains,
                caps: Caps {
                    max_matches: 1,
                    ..Caps::mock()
                },
            },
        };
        let report = agent.drive(4);
        assert_eq!((report.requested, report.ran), (4, 1));
    }

    #[test]
    fn backends_swap_without_changing_logic() {
        // Mock/local/remote differ only in the driver label: the same seed
        // and policy give the same bugs on every backend.
        let mut bugs_per_backend = Vec::new();
        for backend in [Backend::Mock, Backend::Local, Backend::Remote] {
            let agent = Agent {
                config: agent_config(),
                driver: Driver {
                    policy: Policy::StallAll,
                    caps: Caps {
                        backend,
                        ..Caps::mock()
                    },
                },
            };
            bugs_per_backend.push(agent.drive(1).bugs);
        }
        assert_eq!(bugs_per_backend[0], bugs_per_backend[1]);
        assert_eq!(bugs_per_backend[1], bugs_per_backend[2]);
    }
}
