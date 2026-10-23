//! Bounded input-tape replay of the archetype ECS/physics fixture.
//! Legacy and full-state checkpoints have separate versioned scopes; both
//! scopes stay byte-stable with the legacy encoding (see
//! [`engine_ecs::WORLD_FORK_DECISION`]), while ticks run through
//! [`Schedule::run_arena_tick`](engine_ecs::schedule::Schedule::run_arena_tick).

use engine_core::hash::FNV64_PRIME;
use engine_core::{hash_bytes, CounterRng, Fnv64, Rng, TickState, DT};
use engine_ecs::archetype::{Entity, Position, Velocity, World};
use engine_ecs::schedule::{Phase, Schedule};
use engine_ecs::snapshot::SNAPSHOT_VERSION;
use engine_ecs::PhaseArena;
use engine_physics::{Body, Space};
use std::fmt;
use std::path::{Component, Path};

pub mod a9;
#[cfg(feature = "arena")]
pub mod arena_matrix;
pub mod break_corpus;
pub mod bug;
pub mod chaos;
/// 20,000-agent crowd determinism harness: one hash per machine.
pub mod crowd;
pub mod dataset;
/// Differential receipt comparison: two builds, one curated scenario list.
/// Native only: it reads and writes receipt files and runs the builds.
#[cfg(not(target_arch = "wasm32"))]
pub mod differential;
/// Pillars receipt: the sample's distilled net, AI write chain and museum.
pub mod fb_pillars;
pub mod fb_ship;
pub mod fb_versions;
#[cfg(feature = "fuzz")]
pub mod fuzz;
pub mod gpu_live;
/// Crash-safe append-only tick journal beside the whole-blob tape codec.
pub mod journal;
/// Headless pixel-truth gate over the ticked render rows (sky/decals PROBE).
pub use gpu_live::live_gate_report;
/// MV1 workflow receipt shape plus its hand-rolled text encoding (std-only).
/// Behind `m1` like the harness dev-deps, so the headless default graph
/// stays std-only; the harness test requires the same feature.
#[cfg(feature = "m1")]
pub mod m1;
/// Cross-architecture golden matrix: one digest row per machine (HS2).
pub mod matrix;
pub mod museum;
/// Anti-vacuity control on the digest gates: refuse digests that never read the sim.
pub mod not_vacuous;
pub mod online_soak;
/// Rapier ragdoll fall tape for the golden matrix: per-tick body digests (WEB3).
#[cfg(feature = "ragdoll")]
pub mod ragdoll_matrix;
pub mod soak;
/// Spine parity column of the proof scorecard (ARC10).
#[cfg(feature = "spine")]
pub mod spine_parity;
pub mod tape;
/// Tape-decode one-harness fuzz: replay-hash oracle plus shrink (HS23).
#[cfg(feature = "fuzz")]
pub mod tape_fuzz;
pub mod trace;

#[cfg(kani)]
#[path = "../kani/mod.rs"]
mod kani_harness;

#[cfg(test)]
mod kani_kernels;

/// Frozen no-input, seed-2040, 600-tick regression checksum.
pub const GOLDEN_600: u64 = 0xd57b5c8ae3096614;
/// Change when the fixture's initialization, rules or full-state encoding changes.
pub const FIXTURE_VERSION: u16 = 1;
/// A new in-memory fixture version whose seed expansion has no low-bit alias.
/// It is not a tape or a recordable format; the legacy v1 golden stays frozen.
pub const FIXTURE_VERSION_V2: u16 = 2;
/// Paired ECS entities and physics bodies created by this fixture.
pub const BODY_COUNT: u32 = 100;
/// Bound replay CPU work and checkpoint allocation.
pub const MAX_TICKS: u32 = 100_000;
/// Bound the recorded external input queue.
pub const MAX_INPUTS: usize = 4096;

/// Complete configuration for this procedural, asset-free fixture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Initialization seed; all u64 values are supported.
    pub seed: u64,
    /// Number of fixed steps, in 1..=MAX_TICKS.
    pub ticks: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            seed: 2040,
            ticks: 600,
        }
    }
}

/// Accepted velocity assignment applied before the named zero-based tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Input {
    /// Application tick, strictly before Config::ticks.
    pub tick: u32,
    /// Stable paired slot in this fixed-population fixture.
    pub body: u32,
    /// Horizontal velocity in meters/second.
    pub vx: f32,
    /// Vertical velocity in meters/second.
    pub vy: f32,
}

impl Input {
    pub(crate) fn append_bytes(&self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&self.tick.to_le_bytes());
        bytes.extend_from_slice(&self.body.to_le_bytes());
        bytes.extend_from_slice(&self.vx.to_le_bytes());
        bytes.extend_from_slice(&self.vy.to_le_bytes());
    }
}

/// Checkpoint after a fixed step; neither checksum is cryptographic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    /// Historical running hash, preserving the original golden's coverage.
    pub legacy: u64,
    /// All future-affecting state of this fixture, including its input context.
    pub state: u64,
}

/// Replay from the fixture's initial state, not an arbitrary game save/resume.
#[derive(Clone, Debug, PartialEq)]
pub struct Tape {
    /// Procedural initial-state configuration.
    pub config: Config,
    /// Ordered, validated assignments; no provider is consulted during replay.
    pub inputs: Vec<Input>,
    /// One post-step checkpoint per tick.
    pub checkpoints: Vec<Checkpoint>,
}

/// Invalid fixture data or the first reproducibility failure.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// Invalid data, with a bounded diagnostic.
    Invalid(&'static str),
    /// Unsafe tape file path refused before any file IO.
    UnsafePath(&'static str),
    /// Unsupported tape or fixture version.
    Version { format: u16, fixture: u16 },
    /// First mismatching zero-based tick, the first diverging checkpoint
    /// scope (`"legacy"` trajectory or `"state"` full encoding), and both
    /// checkpoint scopes.
    Divergence {
        tick: u32,
        system: &'static str,
        expected: Checkpoint,
        actual: Checkpoint,
    },
    /// Intentional simulation changes need an explicit golden-bump receipt.
    Golden { actual: u64 },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(f, "invalid proof: {reason}"),
            Self::UnsafePath(reason) => write!(f, "unsafe tape path: {reason}"),
            Self::Version { format, fixture } => {
                write!(
                    f,
                    "unsupported tape version {format}, fixture version {fixture}"
                )
            }
            Self::Divergence {
                tick,
                system,
                expected,
                actual,
            } => write!(
                f,
                "replay diverged at tick {tick} (system {system}): expected {:016x}/{:016x}, got {:016x}/{:016x}",
                expected.legacy, expected.state, actual.legacy, actual.state
            ),
            Self::Golden { actual } => {
                write!(
                    f,
                    "golden mismatch: got {actual:016x}, expected {GOLDEN_600:016x}"
                )
            }
        }
    }
}

impl std::error::Error for Error {}

pub(crate) fn validate(config: Config, inputs: &[Input]) -> Result<(), Error> {
    if config.ticks == 0 || config.ticks > MAX_TICKS {
        return Err(Error::Invalid("ticks must be in 1..=100000"));
    }
    if inputs.len() > MAX_INPUTS {
        return Err(Error::Invalid("too many inputs"));
    }
    for input in inputs {
        if input.tick >= config.ticks || input.body >= BODY_COUNT {
            return Err(Error::Invalid("input tick or body is out of range"));
        }
        if [input.vx, input.vy]
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 10_000.0)
        {
            return Err(Error::Invalid(
                "velocity must be finite and within +/-10000",
            ));
        }
    }
    if inputs
        .windows(2)
        .any(|pair| (pair[0].tick, pair[0].body) >= (pair[1].tick, pair[1].body))
    {
        return Err(Error::Invalid(
            "inputs must be unique and ordered by (tick, body)",
        ));
    }
    Ok(())
}

/// Refuse lane-escape tape paths before any file IO.
/// Owner: engine-proof; consumer: proof CLI `read_tape`/`write_tape`.
/// Units: file-system path components; determinism: none (IO guard only).
/// Errors: [`Error::UnsafePath`] for empty paths, absolute paths, `..`
/// traversal, or any `.lanes` component matched case-insensitively (so
/// case-insensitive filesystems cannot bypass the guard); trailing dots and
/// spaces are stripped before the comparison because Windows strips them
/// when resolving paths (so `.lanes.`/`.lanes ` resolve into `.lanes/`);
/// relative child paths pass through.
pub fn check_tape_path(path: &Path) -> Result<(), Error> {
    if path.as_os_str().is_empty() {
        return Err(Error::UnsafePath("tape path is empty"));
    }
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                return Err(Error::UnsafePath("absolute tape paths are refused"));
            }
            Component::ParentDir => {
                return Err(Error::UnsafePath(
                    "parent traversal in tape path is refused",
                ));
            }
            Component::Normal(part) => {
                let mut bytes = part.as_encoded_bytes();
                while bytes.last().is_some_and(|b| *b == b'.' || *b == b' ') {
                    bytes = &bytes[..bytes.len() - 1];
                }
                if bytes.eq_ignore_ascii_case(b".lanes") {
                    return Err(Error::UnsafePath("lane directory in tape path is refused"));
                }
            }
            // A `.` component names no directory, so it cannot be the lane dir.
            Component::CurDir => {}
        }
    }
    if path.is_absolute() {
        return Err(Error::UnsafePath("absolute tape paths are refused"));
    }
    Ok(())
}

/// Scratch sizes for the fixed-population fixture tick: every slot stays
/// live, so the census plus checkpoint always fit these reserves.
const ARENA_HEADROOM_ENTITIES: usize = 8;
const ARENA_BYTE_LEN: usize = 4096;

struct Simulation {
    world: World,
    /// Slot order for the fixed population: `entities[i]` is body `i`.
    entities: Vec<Entity>,
    space: Space,
    clock: TickState,
    context_hash: u64,
    schedule: Schedule<World>,
    arena: PhaseArena,
}

impl Simulation {
    fn new(config: Config, inputs: &[Input]) -> Self {
        Self::build(config, inputs, FIXTURE_VERSION, None)
    }

    /// V2 fixture for tests only: the same scene, with a keyed counter stream
    /// so seeds `2k` and `2k + 1` take different trajectories. The context
    /// folds `FIXTURE_VERSION_V2`, so it cannot share a digest with v1.
    #[cfg(test)]
    fn new_v2(config: Config, inputs: &[Input]) -> Self {
        Self::build(
            config,
            inputs,
            FIXTURE_VERSION_V2,
            Some(CounterRng::new(config.seed)),
        )
    }

    fn build(config: Config, inputs: &[Input], version: u16, counter: Option<CounterRng>) -> Self {
        let mut world = World::new();
        let mut space = Space::new();
        let mut rng = Rng::new(config.seed);
        let mut word = 0u64;
        let mut entities = Vec::new();
        for _ in 0..BODY_COUNT {
            let (x, y) = if let Some(stream) = counter.as_ref() {
                let x = f32::from(u8::try_from(stream.sample(word) % 100).unwrap_or(0));
                word += 1;
                let y = f32::from(u8::try_from(stream.sample(word) % 100).unwrap_or(0));
                word += 1;
                (x, y)
            } else {
                let x = (rng.next_u64() % 100) as f32;
                let y = (rng.next_u64() % 100) as f32;
                (x, y)
            };
            entities.push(world.spawn(Some(Position { x, y }), Some(Velocity { x: 1.0, y: 0.5 })));
            space.add(Body {
                x,
                y: y + 10.0,
                vx: 0.5,
                vy: 0.0,
                hx: 0.5,
                hy: 0.5,
            });
        }
        // Setup-time sizing only: the tick path below never grows.
        let slots = entities.len() + ARENA_HEADROOM_ENTITIES;
        world.reserve(slots);
        let mut schedule: Schedule<World> = Schedule::new();
        schedule.add_system(Phase::Update, "integrate", |world, ctx| {
            world.integrate(ctx.dt);
        });
        let mut arena = PhaseArena::with_capacity(slots, ARENA_BYTE_LEN);
        arena.reserve(slots, ARENA_BYTE_LEN);
        let mut context = Vec::new();
        context.extend_from_slice(&version.to_le_bytes());
        context.extend_from_slice(&config.seed.to_le_bytes());
        context.extend_from_slice(&config.ticks.to_le_bytes());
        for input in inputs {
            input.append_bytes(&mut context);
        }
        Self {
            world,
            entities,
            space,
            clock: TickState::new(config.seed),
            context_hash: hash_bytes(&context),
            schedule,
            arena,
        }
    }

    /// Legacy-scope world hash over positions only, in slot order: the same
    /// FNV-1a64 field sequence the legacy `World::state_hash` mixed, so the
    /// golden scope never moves under the arena tick path.
    fn legacy_world_hash(&self) -> u64 {
        let mut hash = Fnv64::new();
        for entity in &self.entities {
            if let Some(pos) = self.world.get_pos(*entity) {
                hash.write_u32(pos.x.to_bits());
                hash.write_u32(pos.y.to_bits());
            }
        }
        hash.finish()
    }

    /// Canonical world bytes in the legacy snapshot field order (version,
    /// slot count, then one live flag plus position plus velocity per slot
    /// in slot order): identical bytes to the legacy encoding, since every
    /// fixture slot stays live.
    fn append_world_canonical(&self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&SNAPSHOT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&BODY_COUNT.to_le_bytes());
        for entity in &self.entities {
            if let (Some(pos), Some(vel)) =
                (self.world.get_pos(*entity), self.world.get_vel(*entity))
            {
                bytes.push(1);
                for value in [pos.x, pos.y, vel.x, vel.y] {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
    }

    fn state_hash(&self) -> u64 {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&self.context_hash.to_le_bytes());
        bytes.extend_from_slice(&self.clock.tick.to_le_bytes());
        bytes.extend_from_slice(&self.clock.acc.to_le_bytes());
        self.append_world_canonical(&mut bytes);
        bytes.extend_from_slice(&(self.space.bodies.len() as u32).to_le_bytes());
        for body in &self.space.bodies {
            for value in [body.x, body.y, body.vx, body.vy, body.hx, body.hy] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        hash_bytes(&bytes)
    }

    /// Canonical space bytes: body count plus six LE floats per body in slot
    /// order (the same bytes [`state_hash`](Self::state_hash) folds).
    fn append_space_canonical(&self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&(self.space.bodies.len() as u32).to_le_bytes());
        for body in &self.space.bodies {
            for value in [body.x, body.y, body.vx, body.vy, body.hx, body.hy] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
    }

    /// Canonical clock bytes: tick plus running accumulator, LE.
    fn append_clock_canonical(&self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&self.clock.tick.to_le_bytes());
        bytes.extend_from_slice(&self.clock.acc.to_le_bytes());
    }

    /// Per-system diagnostic split over the current state. The authority
    /// checkpoints stay `legacy`/`state`; this split only attributes a
    /// divergence to its field family for the desync report.
    fn system_split(&self) -> SystemSplit {
        let mut world_bytes = Vec::new();
        self.append_world_canonical(&mut world_bytes);
        let mut space_bytes = Vec::new();
        self.append_space_canonical(&mut space_bytes);
        let mut clock_bytes = Vec::new();
        self.append_clock_canonical(&mut clock_bytes);
        SystemSplit {
            world: hash_bytes(&world_bytes),
            space: hash_bytes(&space_bytes),
            clock: hash_bytes(&clock_bytes),
        }
    }

    /// Step one fixed tick through the arena schedule: validated inputs
    /// retarget velocities, the schedule integrates through per-phase
    /// scratch, then physics steps. A tick that outgrows its presized
    /// scratch refuses with [`Error::Invalid`] instead of allocating.
    fn step(&mut self, inputs: &[Input]) -> Result<Checkpoint, Error> {
        for input in inputs {
            let slot = usize::try_from(input.body)
                .ok()
                .filter(|slot| *slot < self.entities.len())
                .ok_or(Error::Invalid("input body is out of range"))?;
            let body = &mut self.space.bodies[slot];
            body.vx = input.vx;
            body.vy = input.vy;
            if !self.world.set_vel(
                self.entities[slot],
                Velocity {
                    x: input.vx,
                    y: input.vy,
                },
            ) {
                return Err(Error::Invalid("fixture slot went stale mid-tape"));
            }
        }
        self.schedule
            .run_arena_tick(&mut self.world, &mut self.arena, DT)
            .map_err(|_| Error::Invalid("arena scratch overflow on the fixture tick"))?;
        self.space.step(DT);
        let legacy = self
            .clock
            .step(self.legacy_world_hash() ^ self.space.state_hash().wrapping_mul(FNV64_PRIME));
        Ok(Checkpoint {
            legacy,
            state: self.state_hash(),
        })
    }
}

fn execute(
    config: Config,
    inputs: &[Input],
    expected: Option<&[Checkpoint]>,
) -> Result<Vec<Checkpoint>, Error> {
    execute_with(config, inputs, expected, |_, _| Ok(()))
}

/// [`execute`] with `on_tick` called after every step with that tick's inputs
/// and checkpoint, so a recorder can stream ticks (the journal writer) while
/// the run is still going. An observer error stops the run at that tick.
pub(crate) fn execute_with<E: From<Error>>(
    config: Config,
    inputs: &[Input],
    expected: Option<&[Checkpoint]>,
    mut on_tick: impl FnMut(&[Input], Checkpoint) -> Result<(), E>,
) -> Result<Vec<Checkpoint>, E> {
    validate(config, inputs)?;
    if expected.is_some_and(|values| values.len() != config.ticks as usize) {
        return Err(Error::Invalid("checkpoint count does not match ticks").into());
    }
    let mut sim = Simulation::new(config, inputs);
    let mut checkpoints = Vec::with_capacity(config.ticks as usize);
    let mut state_only = None;
    let mut cursor = 0;
    for tick in 0..config.ticks {
        let start = cursor;
        while cursor < inputs.len() && inputs[cursor].tick == tick {
            cursor += 1;
        }
        let actual = sim.step(&inputs[start..cursor])?;
        on_tick(&inputs[start..cursor], actual)?;
        if let Some(values) = expected {
            let wanted = values[tick as usize];
            if actual != wanted {
                // Scope attribution: the legacy scope folds the simulated
                // trajectory (world plus space via the tick clock), so a
                // moved trajectory names it at its own tick, even though the
                // input context already differs in `state` from tick 0. A
                // pure encoding/context mismatch (such as a flipped
                // checkpoint byte) names state, so the first state-only
                // mismatch is kept while the replay looks for a legacy one.
                if actual.legacy != wanted.legacy {
                    return Err(Error::Divergence {
                        tick,
                        system: "legacy",
                        expected: wanted,
                        actual,
                    }
                    .into());
                }
                state_only.get_or_insert(Error::Divergence {
                    tick,
                    system: "state",
                    expected: wanted,
                    actual,
                });
            }
        }
        checkpoints.push(actual);
    }
    if let Some(error) = state_only {
        return Err(error.into());
    }
    if config == Config::default() && inputs.is_empty() && sim.clock.hash() != GOLDEN_600 {
        return Err(Error::Golden {
            actual: sim.clock.hash(),
        }
        .into());
    }
    Ok(checkpoints)
}

/// Per-system diagnostic hashes for one post-step state, in check order:
/// ECS world, physics space, tick clock. The authority checkpoints stay
/// `legacy`/`state`; this split only attributes a divergence to its field
/// family (the arena crate carries the wider
/// world/space/combat/rng/ai split for the full game).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemSplit {
    /// FNV-1a64 over the world canonical bytes (slot-ordered live flag,
    /// positions and velocities).
    pub world: u64,
    /// FNV-1a64 over the space canonical bytes (body extents and velocities).
    pub space: u64,
    /// FNV-1a64 over the clock bytes (tick plus running accumulator).
    pub clock: u64,
}

/// System names in [`SystemSplit`] field order.
pub const SYSTEM_NAMES: [&str; 3] = ["world", "space", "clock"];

/// First system whose hashes differ, in [`SYSTEM_NAMES`] order.
pub fn first_diverging_system(a: &SystemSplit, b: &SystemSplit) -> Option<&'static str> {
    if a.world != b.world {
        return Some("world");
    }
    if a.space != b.space {
        return Some("space");
    }
    if a.clock != b.clock {
        return Some("clock");
    }
    None
}

/// Step `steps` ticks from the initial state and return the per-system
/// split there. `None` on invalid config/inputs or `steps > config.ticks`.
///
/// Consumer: desync reports and `tools/desync-diff.mjs`, which dumps the
/// matching [`system_canonical_bytes`]. Units: fixed ticks; determinism:
/// same config plus inputs plus steps gives identical splits.
pub fn system_split_at(config: Config, inputs: &[Input], steps: u32) -> Option<SystemSplit> {
    validate(config, inputs).ok()?;
    if steps > config.ticks {
        return None;
    }
    let mut sim = Simulation::new(config, inputs);
    let mut cursor = 0;
    for tick in 0..steps {
        let start = cursor;
        while cursor < inputs.len() && inputs[cursor].tick == tick {
            cursor += 1;
        }
        sim.step(&inputs[start..cursor]).ok()?;
    }
    Some(sim.system_split())
}

/// Canonical bytes for one system (`"world"`, `"space"` or `"clock"`) at
/// the post-`steps` state, rebuilt from scratch. `None` on invalid
/// config/inputs, unknown system, or `steps > config.ticks`.
///
/// Consumer: `tools/desync-diff.mjs` dumps these bytes for the reported
/// system into the Factorio-style report dir. The [`SystemSplit`] hash for
/// the same system equals the FNV-1a64 of these bytes.
pub fn system_canonical_bytes(
    config: Config,
    inputs: &[Input],
    steps: u32,
    system: &str,
) -> Option<Vec<u8>> {
    validate(config, inputs).ok()?;
    if steps > config.ticks {
        return None;
    }
    let mut sim = Simulation::new(config, inputs);
    let mut cursor = 0;
    for tick in 0..steps {
        let start = cursor;
        while cursor < inputs.len() && inputs[cursor].tick == tick {
            cursor += 1;
        }
        sim.step(&inputs[start..cursor]).ok()?;
    }
    let mut out = Vec::new();
    match system {
        "world" => sim.append_world_canonical(&mut out),
        "space" => sim.append_space_canonical(&mut out),
        "clock" => sim.append_clock_canonical(&mut out),
        _ => return None,
    }
    Some(out)
}

/// Run real ECS/physics steps and record both checkpoint scopes.
pub fn record(config: Config, inputs: &[Input]) -> Result<Tape, Error> {
    let checkpoints = execute(config, inputs, None)?;
    Ok(Tape {
        config,
        inputs: inputs.to_vec(),
        checkpoints,
    })
}

/// Reconstruct initial state, apply recorded inputs, and stop at first divergence.
pub fn replay(tape: &Tape) -> Result<(), Error> {
    execute(tape.config, &tape.inputs, Some(&tape.checkpoints))?;
    Ok(())
}

/// Passive fixed-step plan beside the tick loop.
/// Caller: `crate::matrix::collect_local_entry` steps one plan per row. Units: integer ms; no wall clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FramePlan(u64, u64);
/// Passive step decision; `alpha` is presentation blend in 0.0..1.0 and never hashed.
/// Caller: `crate::matrix::collect_local_entry` consumes the decision beside the row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameTick {
    pub steps: u32,
    pub dropped: u32,
    pub tick: u64,
    pub alpha: f32,
}
impl FramePlan {
    /// Fresh plan with empty accumulator at tick zero.
    #[must_use]
    pub fn new() -> Self {
        Self(0, 0)
    }
    /// Pure clamp-and-discard: add `dt_ms`, run at most `max_steps` of `step_ms`, drop exact excess.
    pub fn begin_frame(&mut self, dt_ms: u64, step_ms: u64, max_steps: u32) -> FrameTick {
        let step = step_ms.max(1);
        self.0 = self.0.saturating_add(dt_ms);
        let want = u32::try_from(self.0 / step).unwrap_or(u32::MAX);
        let steps = want.min(max_steps);
        let dropped = want.saturating_sub(steps);
        self.0 -= u64::from(steps.saturating_add(dropped)) * step;
        self.1 += u64::from(steps);
        FrameTick {
            steps,
            dropped,
            tick: self.1,
            alpha: (self.0 as f32) / (step as f32),
        }
    }
}
impl Default for FramePlan {
    /// Same empty accumulator as [`FramePlan::new`].
    fn default() -> Self {
        Self::new()
    }
}
/// Trace input file bytes: 16 LE bytes per input in tape order.
/// Caller: `crate::matrix::emit_local_digests` proves file-vs-memory equality.
#[must_use]
pub fn encode_trace_inputs(inputs: &[Input]) -> Vec<u8> {
    let mut out = Vec::with_capacity(inputs.len() * 16);
    for input in inputs {
        input.append_bytes(&mut out);
    }
    out
}
/// Decode trace bytes under `config`; same bytes replay to identical digests.
/// Caller: `crate::matrix::emit_local_digests` replays the file bytes.
/// Errors: [`Error::Invalid`] on bad length or failed [`validate`].
pub fn decode_trace_inputs(config: Config, bytes: &[u8]) -> Result<Vec<Input>, Error> {
    if !bytes.len().is_multiple_of(16) || bytes.len() / 16 > MAX_INPUTS {
        return Err(Error::Invalid("bad trace length"));
    }
    let mut inputs = Vec::with_capacity(bytes.len() / 16);
    for chunk in bytes.chunks_exact(16) {
        inputs.push(Input {
            tick: u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]),
            body: u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]),
            vx: f32::from_le_bytes([chunk[8], chunk[9], chunk[10], chunk[11]]),
            vy: f32::from_le_bytes([chunk[12], chunk[13], chunk[14], chunk[15]]),
        });
    }
    validate(config, &inputs)?;
    Ok(inputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sched_shuffle_keeps_the_golden_across_50_seeded_orders() {
        use std::{cell::RefCell, collections::BTreeSet, rc::Rc};
        let config = Config::default();
        let want = *record(config, &[]).unwrap().checkpoints.last().unwrap();
        assert_eq!(want.legacy, GOLDEN_600);
        let mut orders = BTreeSet::new();
        for seed in 0..50u64 {
            let log = Rc::new(RefCell::new(Vec::new()));
            let mut schedule: Schedule<World> = Schedule::new();
            // Position is column bit 1, velocity bit 2: integrate reads
            // velocity and writes position. The probes touch no column, so
            // all six orders of the three systems are free.
            let integrate_log = Rc::clone(&log);
            schedule.add_system_with_access(Phase::Update, "integrate", 2, 1, move |world, ctx| {
                integrate_log.borrow_mut().push("integrate");
                world.integrate(ctx.dt);
            });
            for name in ["probe_a", "probe_b"] {
                let log = Rc::clone(&log);
                schedule.add_system(Phase::Update, name, move |_, _| log.borrow_mut().push(name));
            }
            schedule.set_shuffle_seed(seed);
            assert!(schedule.validate().is_ok());
            let mut sim = Simulation::new(config, &[]);
            sim.schedule = schedule;
            let mut last = None;
            for _ in 0..config.ticks {
                last = Some(sim.step(&[]).unwrap());
            }
            assert_eq!(last, Some(want), "seed {seed} moved the golden");
            orders.insert(log.borrow()[..3].to_vec());
        }
        assert!(
            orders.len() >= 4,
            "50 seeds reached only {} orders",
            orders.len()
        );
    }

    #[test]
    fn sched_shuffle_refuses_a_missing_edge_naming_both_systems() {
        use engine_ecs::schedule::ScheduleError;
        let mut schedule: Schedule<World> = Schedule::new();
        schedule.add_system_with_access(Phase::Update, "integrate", 2, 1, |world, ctx| {
            world.integrate(ctx.dt);
        });
        // A reader of Position with no order edge to the writer.
        schedule.add_system_with_access(Phase::Update, "probe_a", 1, 0, |_, _| {});
        let err = schedule.validate().unwrap_err();
        assert_eq!(
            err,
            ScheduleError::Conflict("integrate", "probe_a", Phase::Update)
        );
        let text = err.to_string();
        assert!(
            text.contains("integrate") && text.contains("probe_a"),
            "{text}"
        );
        schedule.add_order("integrate", "probe_a");
        assert!(schedule.validate().is_ok());
    }

    #[test]
    fn frozen_default_and_distinct_seed_proofs() {
        let tape = record(Config::default(), &[]).unwrap();
        assert_eq!(tape.checkpoints.last().unwrap().legacy, GOLDEN_600);
        replay(&tape).unwrap();
        let other = record(
            Config {
                seed: 19,
                ticks: 600,
            },
            &[],
        )
        .unwrap();
        assert_ne!(tape.checkpoints, other.checkpoints);
    }

    #[test]
    fn trace_replays_to_identical_digests() {
        let config = Config::default();
        let inputs = [Input {
            tick: 5,
            body: 1,
            vx: 1.0,
            vy: 2.0,
        }];
        let mem = record(config, &inputs).unwrap();
        let bytes = encode_trace_inputs(&inputs);
        assert_eq!(decode_trace_inputs(config, &bytes).unwrap(), inputs);
        let file = record(config, &decode_trace_inputs(config, &bytes).unwrap()).unwrap();
        assert_eq!(file.checkpoints, mem.checkpoints);
        replay(&file).unwrap();
        replay(&file).unwrap();
        assert!(decode_trace_inputs(config, &[0u8; 15]).is_err());
        let mut plan = FramePlan::new();
        let decision = plan.begin_frame(100, 16, 5);
        assert_eq!((decision.steps, decision.dropped, decision.tick), (5, 1, 5));
        assert!((0.0..1.0).contains(&decision.alpha));
    }

    #[test]
    fn input_changes_simulation_at_the_recorded_tick() {
        let config = Config { seed: 7, ticks: 12 };
        let input = Input {
            tick: 5,
            body: 0,
            vx: -20.0,
            vy: 8.0,
        };
        let base = record(config, &[]).unwrap();
        let changed = record(config, &[input]).unwrap();
        for tick in 0..5 {
            assert_eq!(
                base.checkpoints[tick].legacy,
                changed.checkpoints[tick].legacy
            );
        }
        assert_ne!(base.checkpoints[5].legacy, changed.checkpoints[5].legacy);
        replay(&changed).unwrap();
        let mut altered = changed;
        altered.checkpoints[7].state ^= 1;
        assert!(matches!(
            replay(&altered),
            Err(Error::Divergence { tick: 7, .. })
        ));
    }

    #[test]
    fn full_state_covers_velocity_extents_and_pending_input_context() {
        let mut sim = Simulation::new(Config::default(), &[]);
        let legacy = sim.legacy_world_hash();
        let before = sim.state_hash();
        let first = sim.entities[0];
        let mut vel = sim.world.get_vel(first).expect("fixture slot live");
        vel.x += 1.0;
        assert!(sim.world.set_vel(first, vel));
        assert_eq!(sim.legacy_world_hash(), legacy);
        assert_ne!(sim.state_hash(), before);
        let before = sim.state_hash();
        sim.space.bodies[0].hx += 1.0;
        assert_ne!(sim.state_hash(), before);
        let with_input = Simulation::new(
            Config::default(),
            &[Input {
                tick: 100,
                body: 0,
                vx: 1.0,
                vy: 2.0,
            }],
        );
        let without = Simulation::new(Config::default(), &[]);
        assert_ne!(with_input.state_hash(), without.state_hash());
    }

    fn trajectory(seed: u64, ticks: u32) -> (u64, u64) {
        let mut sim = Simulation::new(Config { seed, ticks }, &[]);
        for _ in 0..ticks {
            sim.step(&[])
                .expect("presized fixture tick fits its scratch");
        }
        (sim.legacy_world_hash(), sim.space.state_hash())
    }

    fn trajectory_v2(seed: u64, ticks: u32) -> (u64, u64) {
        let mut sim = Simulation::new_v2(Config { seed, ticks }, &[]);
        for _ in 0..ticks {
            sim.step(&[])
                .expect("presized fixture tick fits its scratch");
        }
        (sim.legacy_world_hash(), sim.space.state_hash())
    }

    #[test]
    fn sim_ticks_through_run_arena_tick() {
        // The fixture tick advances the arena schedule (not a direct
        // integrate), stays deterministic x2, and a short scratch refuses.
        let config = Config { seed: 7, ticks: 4 };
        let mut first = Simulation::new(config, &[]);
        let mut second = Simulation::new(config, &[]);
        assert_eq!(first.schedule.tick(), 0);
        for _ in 0..4 {
            assert_eq!(
                first.step(&[]).expect("presized fixture tick fits"),
                second.step(&[]).expect("presized fixture tick fits")
            );
        }
        assert_eq!((first.schedule.tick(), second.schedule.tick()), (4, 4));
        assert_eq!(
            first
                .world
                .checkpoint_with(&mut first.arena)
                .expect("presized scratch checkpoints"),
            first.world.state_hash()
        );
        let mut tiny = PhaseArena::new();
        let mut refusing: Schedule<World> = Schedule::new();
        refusing.add_system(Phase::Update, "integrate", |world, ctx| {
            world.integrate(ctx.dt);
        });
        assert!(matches!(
            refusing.run_arena_tick(&mut first.world, &mut tiny, DT),
            Err(engine_ecs::ArenaError::Overflow { .. })
        ));
    }

    #[test]
    fn legacy_seeds_differing_only_in_low_bit_share_a_trajectory() {
        // Receipts still differ because the raw seed is folded into both hashes,
        // so a receipt comparison cannot show that two seeds simulated differently.
        assert_eq!(trajectory(2, 40), trajectory(3, 40));
        let even = record(Config { seed: 2, ticks: 40 }, &[]).unwrap();
        let odd = record(Config { seed: 3, ticks: 40 }, &[]).unwrap();
        assert_ne!(even.checkpoints, odd.checkpoints);
    }

    #[test]
    fn v2_seeds_differing_only_in_low_bit_take_different_trajectories() {
        // V2 is a new in-memory fixture, not a tape: it uses CounterRng and a
        // new version byte, so adjacent seeds stop alias while v1 stays frozen.
        assert_eq!(trajectory_v2(2, 40), trajectory_v2(2, 40));
        assert_ne!(trajectory_v2(2, 40), trajectory_v2(3, 40));
        assert_ne!(trajectory_v2(0, 40), trajectory_v2(1, 40));
        assert_ne!(
            trajectory_v2(2, 40),
            trajectory(2, 40),
            "the version byte and seed stream both change"
        );
    }

    #[test]
    fn proof_gate_seeds_have_distinct_trajectories() {
        // Seeds used by tools/proof.mjs; keep the two lists in sync.
        let traces: Vec<_> = [0, 2, 2040, 999_983]
            .map(|seed| trajectory(seed, 40))
            .to_vec();
        for (i, a) in traces.iter().enumerate() {
            assert!(traces[i + 1..].iter().all(|b| a != b), "{traces:?}");
        }
    }

    #[test]
    fn invalid_and_ambiguous_inputs_are_rejected() {
        let input = Input {
            tick: 0,
            body: 0,
            vx: 1.0,
            vy: 0.0,
        };
        assert!(record(Config { seed: 0, ticks: 0 }, &[]).is_err());
        assert!(record(
            Config {
                seed: 0,
                ticks: MAX_TICKS + 1
            },
            &[]
        )
        .is_err());
        for bad in [
            Input {
                vx: f32::NAN,
                ..input
            },
            Input {
                body: BODY_COUNT,
                ..input
            },
            Input { tick: 600, ..input },
        ] {
            assert!(record(Config::default(), &[bad]).is_err());
        }
        assert!(record(Config::default(), &[input, input]).is_err());
        assert!(record(Config::default(), &[Input { tick: 1, ..input }, input]).is_err());
    }

    #[test]
    fn tampered_tape_replay_reports_tick_and_system() {
        let config = Config {
            seed: 19,
            ticks: 120,
        };
        let inputs = [
            Input {
                tick: 3,
                body: 0,
                vx: -8.0,
                vy: 4.0,
            },
            Input {
                tick: 40,
                body: 1,
                vx: 2.0,
                vy: 6.0,
            },
        ];
        let tape = record(config, &inputs).unwrap();
        // Flipped checkpoint byte: the trajectory still replays, only the
        // stored full-encoding hash mismatches at the final tick.
        let mut tampered = tape.clone();
        tampered.checkpoints[119].state ^= 1;
        let error = replay(&tampered).expect_err("tampered checkpoint must diverge");
        assert!(
            matches!(
                error,
                Error::Divergence {
                    tick: 119,
                    system: "state",
                    ..
                }
            ),
            "got {error:?}"
        );
        assert!(format!("{error}").contains("tick 119"));
        assert!(format!("{error}").contains("system state"));
        // Altered input: the trajectory itself moves at the input tick, so
        // the legacy scope names it first.
        let mut moved = tape.clone();
        moved.inputs[0].vx = 19.0;
        let error = replay(&moved).expect_err("altered input must diverge");
        assert!(
            matches!(
                error,
                Error::Divergence {
                    tick: 3,
                    system: "legacy",
                    ..
                }
            ),
            "got {error:?}"
        );
        assert!(format!("{error}").contains("tick 3"));
        assert!(format!("{error}").contains("system legacy"));
    }

    #[test]
    fn system_split_is_stable_sensitive_and_matches_its_bytes() {
        let config = Config {
            seed: 19,
            ticks: 120,
        };
        let inputs = [Input {
            tick: 3,
            body: 0,
            vx: -8.0,
            vy: 4.0,
        }];
        let first = system_split_at(config, &inputs, 40).expect("valid split");
        let second = system_split_at(config, &inputs, 40).expect("valid split");
        assert_eq!(first, second);
        for name in SYSTEM_NAMES {
            let bytes =
                system_canonical_bytes(config, &inputs, 40, name).expect("system dumps bytes");
            assert_eq!(
                hash_bytes(&bytes),
                match name {
                    "world" => first.world,
                    "space" => first.space,
                    _ => first.clock,
                }
            );
            assert_eq!(
                bytes,
                system_canonical_bytes(config, &inputs, 40, name).expect("bytes replay")
            );
        }
        assert!(system_canonical_bytes(config, &inputs, 40, "combat").is_none());
        assert!(system_canonical_bytes(config, &inputs, 121, "world").is_none());
        assert!(system_split_at(config, &inputs, 121).is_none());
        // A moved velocity moves the world bytes (and the shared space
        // velocities) at the input tick, so the world system diverges first.
        let mut altered = inputs;
        altered[0].vx = 19.0;
        let moved = system_split_at(config, &altered, 40).expect("moved split");
        assert_eq!(first_diverging_system(&first, &moved), Some("world"));
        // Untouched prefix agrees tick for tick.
        let prefix = system_split_at(config, &inputs, 3).expect("prefix split");
        let same = system_split_at(config, &altered, 3).expect("same prefix");
        assert_eq!(prefix, same);
    }

    #[test]
    fn lane_traversal_and_absolute_tape_paths_are_refused_by_name() {
        for bad in [
            ".lanes/evil.e2040",
            "a/.lanes/evil.e2040",
            ".lanes",
            ".LANES/evil.e2040",
            ".Lanes/evil.e2040",
            "a/.LANES/evil.e2040",
            ".LANES",
            ".lanes./evil.e2040",
            ".LANES./evil.e2040",
            "a/.lanes./evil.e2040",
            ".lanes./redteam-probe-dot.e2040",
            ".lanes./",
            ".lanes.",
            ".lanes ",
            ".lanes.../evil.e2040",
            ".LANES. /evil.e2040",
            "../evil.e2040",
            "a/../../evil.e2040",
            "..",
            "/tmp/evil.e2040",
            "",
        ] {
            let error = check_tape_path(Path::new(bad)).unwrap_err();
            assert!(
                matches!(error, Error::UnsafePath(_)),
                "path {bad:?} gave {error:?}"
            );
            assert!(format!("{error}").contains("unsafe tape path"), "{error}");
        }
        // A similarly named component without the exact lane directory passes.
        for good in [
            "tape.e2040",
            "out/tape.e2040",
            "input tape.e2040",
            ".lanes-backup/tape.e2040",
            ".LANES-BACKUP/tape.e2040",
            "./tape.e2040",
        ] {
            assert!(check_tape_path(Path::new(good)).is_ok(), "{good:?}");
        }
        // Absolute-form check that holds on Windows and Unix alike.
        let absolute = std::env::temp_dir().join("engine2040-proof-absolute.e2040");
        assert!(absolute.is_absolute());
        assert!(matches!(
            check_tape_path(&absolute),
            Err(Error::UnsafePath(_))
        ));
    }
}
