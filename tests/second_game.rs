//! Second-game fixture (GP5): tag over the public ECS/physics lanes.
//!
//! Purpose: prove the engine hosts a materially different small game with
//! zero engine edits. Owner: the GP5 fixture (tests + data only). Units:
//! meters, seconds, fixed 60 Hz ticks. Determinism: same config plus inputs
//! gives identical checkpoints twice; sim math is `+ - * /` and one `sqrt`
//! in the presentation-only camera, index order, fixed dt, no wall clock.
//! Errors: typed [`ConfigError`], [`InputError`] and [`ReplayError`]; bad
//! fixture data fails loudly, never panics on user bytes.

use engine_core::{hash_bytes, TickState, DT};
use engine_ecs::{Position, Velocity, World};
use engine_physics::{Body, Space};
use std::sync::OnceLock;

/// Fixture rules version. v1 stays frozen for the legacy proof fixture.
const FIXTURE_VERSION: u16 = 2;
/// Bound replay work per match.
const MAX_TICKS: u32 = 10_000;
/// Runners per match, `2..=MAX_RUNNERS`.
const MAX_RUNNERS: u8 = 8;
/// Bound recorded moves per match.
const MAX_INPUTS: usize = 512;
/// Fastest legal velocity assignment, m/s.
const MAX_SPEED_MPS: f32 = 20.0;
/// Goal disc radius bounds, meters.
const GOAL_RADIUS_MIN_M: f32 = 0.5;
/// Goal disc radius bounds, meters.
const GOAL_RADIUS_MAX_M: f32 = 5.0;
/// Playfield half-extent bounds, meters.
const EXTENT_MIN_M: f32 = 5.0;
/// Playfield half-extent bounds, meters.
const EXTENT_MAX_M: f32 = 50.0;
/// FNV-1a prime for combining lane hashes.
const PRIME: u64 = 0x1000_0000_01b3;
/// SplitMix64 increment (integer seed expansion, no low-bit aliasing).
const SPLITMIX_INC: u64 = 0x9E37_79B9_7F4A_7C15;

/// Tag match configuration: every field is validated before a run.
#[derive(Clone, Copy, PartialEq, Debug)]
struct GameConfig {
    /// Initialization seed; expanded with SplitMix64 (seeds never alias).
    seed: u64,
    /// Fixed steps, `1..=MAX_TICKS`.
    ticks: u32,
    /// Runners, `2..=MAX_RUNNERS`.
    runners: u8,
    /// Goal disc center `[x, y]` in meters.
    goal: [f32; 2],
    /// Goal disc radius in meters.
    goal_radius_m: f32,
    /// Playfield half-extent in meters.
    half_extent_m: f32,
}

/// One velocity assignment applied before the named zero-based tick.
#[derive(Clone, Copy, PartialEq, Debug)]
struct MoveInput {
    /// Application tick, strictly below `GameConfig::ticks`.
    tick: u32,
    /// Runner slot below `GameConfig::runners`.
    body: u8,
    /// Horizontal velocity in m/s.
    vx: f32,
    /// Vertical velocity in m/s.
    vy: f32,
}

/// Checkpoint after one fixed step.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Checkpoint {
    /// Zero-based fixed tick.
    tick: u32,
    /// State hash after the tick.
    hash: u64,
}

/// Recorded match: config, ordered moves, one checkpoint per tick, tag win.
#[derive(Clone, PartialEq, Debug)]
struct Tape {
    /// Rules the match ran under.
    config: GameConfig,
    /// Ordered, validated moves.
    inputs: Vec<MoveInput>,
    /// One post-step checkpoint per tick.
    checkpoints: Vec<Checkpoint>,
    /// First tick a runner entered the goal disc, if any.
    win_tick: Option<u32>,
    /// Lowest-index runner inside the disc on `win_tick`, if any.
    winner: Option<u8>,
}

/// Follow-camera parameters as data (presentation only, never hashed).
#[derive(Clone, Copy, PartialEq, Debug)]
struct CameraParams {
    /// Camera height over the plane in meters.
    height_m: f32,
    /// Lead toward the current leader in meters.
    lead_m: f32,
}

/// Bad match configuration, culprit carried.
#[expect(
    clippy::enum_variant_names,
    reason = "error variants name the config field they reject"
)]
#[derive(Clone, Copy, PartialEq, Debug)]
enum ConfigError {
    /// Tick count `0` or above `MAX_TICKS`.
    BadTicks(u32),
    /// Runner count outside `2..=MAX_RUNNERS`.
    BadRunners(u8),
    /// Goal disc outside the playfield or with a bad radius.
    BadGoal,
    /// Half-extent NaN or outside `5.0..=50.0` meters.
    BadExtent,
}

/// Bad move list, reason carried.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum InputError {
    /// More than `MAX_INPUTS` moves.
    TooMany,
    /// Tick at or past the match end, or body past the runners.
    OutOfRange,
    /// Non-finite or over-speed velocity.
    BadVelocity,
    /// Moves not strictly ordered by `(tick, body)`.
    Unordered,
}

/// Replay failure: invalid data or the first divergent tick.
#[derive(Clone, PartialEq, Eq, Debug)]
enum ReplayError {
    /// Config or move list rejected.
    Invalid,
    /// Checkpoint count does not match the config.
    Count,
    /// First mismatching zero-based tick plus both hashes.
    Divergence {
        tick: u32,
        expected: u64,
        actual: u64,
    },
    /// Win outcome differs at the named tick.
    Win(u32),
}

impl core::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadTicks(ticks) => write!(f, "bad ticks ({ticks})"),
            Self::BadRunners(runners) => write!(f, "bad runners ({runners})"),
            Self::BadGoal => write!(f, "bad goal disc"),
            Self::BadExtent => write!(f, "bad half extent"),
        }
    }
}

impl core::fmt::Display for InputError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooMany => write!(f, "too many inputs"),
            Self::OutOfRange => write!(f, "input tick or body out of range"),
            Self::BadVelocity => write!(f, "velocity not finite or over speed"),
            Self::Unordered => write!(f, "inputs not ordered by (tick, body)"),
        }
    }
}

impl core::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Invalid => write!(f, "invalid tape"),
            Self::Count => write!(f, "checkpoint count does not match ticks"),
            Self::Divergence {
                tick,
                expected,
                actual,
            } => write!(
                f,
                "replay diverged at tick {tick}: expected {expected:016x}, got {actual:016x}"
            ),
            Self::Win(tick) => write!(f, "win outcome differs at tick {tick}"),
        }
    }
}

impl std::error::Error for ConfigError {}
impl std::error::Error for InputError {}
impl std::error::Error for ReplayError {}

impl GameConfig {
    /// Range-check every field; NaN or out-of-range fails with the culprit.
    fn validate(self) -> Result<(), ConfigError> {
        if self.ticks == 0 || self.ticks > MAX_TICKS {
            return Err(ConfigError::BadTicks(self.ticks));
        }
        if self.runners < 2 || self.runners > MAX_RUNNERS {
            return Err(ConfigError::BadRunners(self.runners));
        }
        if !self.half_extent_m.is_finite()
            || self.half_extent_m < EXTENT_MIN_M
            || self.half_extent_m > EXTENT_MAX_M
        {
            return Err(ConfigError::BadExtent);
        }
        if !self.goal_radius_m.is_finite()
            || self.goal_radius_m < GOAL_RADIUS_MIN_M
            || self.goal_radius_m > GOAL_RADIUS_MAX_M
            || !self.goal[0].is_finite()
            || !self.goal[1].is_finite()
            || self.goal[0].abs() > self.half_extent_m
            || self.goal[1].abs() > self.half_extent_m
        {
            return Err(ConfigError::BadGoal);
        }
        Ok(())
    }
}

impl CameraParams {
    /// Range-check the camera data; a bad table fails before use.
    fn validate(self) -> Result<(), ConfigError> {
        if !self.height_m.is_finite() || self.height_m < 2.0 || self.height_m > 30.0 {
            return Err(ConfigError::BadExtent);
        }
        if !self.lead_m.is_finite() || self.lead_m < 0.0 || self.lead_m > 10.0 {
            return Err(ConfigError::BadGoal);
        }
        Ok(())
    }
}

/// Validate one move list against its config.
fn validate_inputs(config: &GameConfig, inputs: &[MoveInput]) -> Result<(), InputError> {
    if inputs.len() > MAX_INPUTS {
        return Err(InputError::TooMany);
    }
    for input in inputs {
        if input.tick >= config.ticks || input.body >= config.runners {
            return Err(InputError::OutOfRange);
        }
        if !input.vx.is_finite()
            || !input.vy.is_finite()
            || input.vx.abs() > MAX_SPEED_MPS
            || input.vy.abs() > MAX_SPEED_MPS
        {
            return Err(InputError::BadVelocity);
        }
    }
    if inputs
        .windows(2)
        .any(|pair| (pair[0].tick, pair[0].body) >= (pair[1].tick, pair[1].body))
    {
        return Err(InputError::Unordered);
    }
    Ok(())
}

/// One SplitMix64 step: integer math only, every seed draws its own stream.
fn splitmix64_next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(SPLITMIX_INC);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Map one stream word uniformly into `[0, 1]` as `f32`.
fn unit_f32(word: u64) -> f32 {
    (word as f64 / u64::MAX as f64) as f32
}

/// Normalize signed zero so `0.0` and `-0.0` hash identically.
fn canon_zero(value: f32) -> f32 {
    if value == 0.0 {
        0.0
    } else {
        value
    }
}

/// Seed-expanded start positions: two stream words per runner mapped into
/// `[-limit, limit]`; yaw-free planar spawns, index order.
fn spawn_positions(seed: u64, runners: u8, half_extent_m: f32) -> Vec<[f32; 2]> {
    let limit = half_extent_m - 0.4;
    let mut state = seed;
    let mut out = Vec::with_capacity(usize::from(runners));
    for _ in 0..runners {
        let x = unit_f32(splitmix64_next(&mut state)) * 2.0 - 1.0;
        let y = unit_f32(splitmix64_next(&mut state)) * 2.0 - 1.0;
        out.push([canon_zero(x * limit), canon_zero(y * limit)]);
    }
    out
}

/// Live match: an ECS plane for the tag plus a physics mirror stepped in
/// lockstep (the proof composition pattern, different population, spawn,
/// rules and win). The world owns tag positions; the space mirror proves a
/// second lane steps deterministically under the same inputs.
struct Sim {
    /// Match rules.
    config: GameConfig,
    /// Moves in `(tick, body)` order.
    inputs: Vec<MoveInput>,
    /// Cursor into `inputs`.
    cursor: usize,
    /// Zero-based fixed tick.
    tick: u32,
    /// Planar runner state.
    world: World,
    /// Physics mirror bodies.
    space: Space,
    /// Running clock over per-tick step hashes.
    clock: TickState,
    /// Rules-plus-moves context folded into every hash.
    context: u64,
    /// Checkpoints so far.
    checkpoints: Vec<Checkpoint>,
    /// First tag tick, once any runner enters the disc.
    win_tick: Option<u32>,
    /// Tag winner, lowest index on ties.
    winner: Option<u8>,
}

impl Sim {
    /// Validated constructor: config, moves, spawns, both lanes, context.
    fn build(config: GameConfig, inputs: &[MoveInput]) -> Result<Self, InputError> {
        config.validate().map_err(|_| InputError::OutOfRange)?;
        validate_inputs(&config, inputs)?;
        let spawns = spawn_positions(config.seed, config.runners, config.half_extent_m);
        let mut world = World::new();
        let mut space = Space::new();
        for pos in &spawns {
            world.spawn(
                Position {
                    x: pos[0],
                    y: pos[1],
                },
                Velocity { x: 0.0, y: 0.0 },
            );
            space.add(Body {
                x: pos[0],
                y: pos[1] + 10.0,
                vx: 0.5,
                vy: 0.0,
                hx: 0.5,
                hy: 0.5,
            });
        }
        let mut context_bytes = Vec::new();
        context_bytes.extend_from_slice(&FIXTURE_VERSION.to_le_bytes());
        context_bytes.extend_from_slice(&config.seed.to_le_bytes());
        context_bytes.extend_from_slice(&config.ticks.to_le_bytes());
        context_bytes.extend_from_slice(&[config.runners]);
        for axis in config.goal {
            context_bytes.extend_from_slice(&axis.to_bits().to_le_bytes());
        }
        context_bytes.extend_from_slice(&config.goal_radius_m.to_bits().to_le_bytes());
        context_bytes.extend_from_slice(&config.half_extent_m.to_bits().to_le_bytes());
        for input in inputs {
            context_bytes.extend_from_slice(&input.tick.to_le_bytes());
            context_bytes.extend_from_slice(&[input.body]);
            context_bytes.extend_from_slice(&input.vx.to_bits().to_le_bytes());
            context_bytes.extend_from_slice(&input.vy.to_bits().to_le_bytes());
        }
        Ok(Self {
            config,
            inputs: inputs.to_vec(),
            cursor: 0,
            tick: 0,
            world,
            space,
            clock: TickState::new(config.seed),
            context: hash_bytes(&context_bytes),
            checkpoints: Vec::with_capacity(config.ticks as usize),
            win_tick: None,
            winner: None,
        })
    }

    /// FNV-1a over runner velocities in index order (positions ride the
    /// world hash; velocities affect future ticks, so they hash too).
    fn vel_hash(&self) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for vel in self.world.vel.iter().take(usize::from(self.config.runners)) {
            for word in [vel.x.to_bits(), vel.y.to_bits()] {
                for byte in word.to_le_bytes() {
                    hash ^= u64::from(byte);
                    hash = hash.wrapping_mul(PRIME);
                }
            }
        }
        hash
    }

    /// First runner inside the goal disc after this tick, lowest index wins
    /// ties by strict `<` over squared distance (no square root in sim).
    fn tag_winner(&self) -> Option<u8> {
        let radius2 = self.config.goal_radius_m * self.config.goal_radius_m;
        let mut best: Option<u8> = None;
        let mut best_d2 = f32::INFINITY;
        for (index, pos) in self
            .world
            .pos
            .iter()
            .enumerate()
            .take(usize::from(self.config.runners))
        {
            let dx = pos.x - self.config.goal[0];
            let dy = pos.y - self.config.goal[1];
            let d2 = dx * dx + dy * dy;
            if d2 <= radius2 && d2 < best_d2 {
                best_d2 = d2;
                best = Some(index as u8);
            }
        }
        best
    }

    /// One fixed tick: apply this tick's moves to both lanes, step both,
    /// record the tag, fold every future-affecting byte into the checkpoint.
    fn step(&mut self) {
        while self.cursor < self.inputs.len() && self.inputs[self.cursor].tick == self.tick {
            let input = self.inputs[self.cursor];
            let slot = usize::from(input.body);
            self.world.vel[slot] = Velocity {
                x: input.vx,
                y: input.vy,
            };
            self.space.bodies[slot].vx = input.vx;
            self.space.bodies[slot].vy = input.vy;
            self.cursor += 1;
        }
        self.world.integrate(DT);
        self.space.step(DT);
        if self.win_tick.is_none() {
            if let Some(winner) = self.tag_winner() {
                self.win_tick = Some(self.tick);
                self.winner = Some(winner);
            }
        }
        let win_bits = match (self.win_tick, self.winner) {
            (Some(tick), Some(winner)) => {
                u64::from(tick).wrapping_mul(PRIME) ^ u64::from(winner).wrapping_add(1)
            }
            _ => 0,
        };
        let step_hash = self.world.state_hash()
            ^ self.space.state_hash().wrapping_mul(PRIME)
            ^ self.vel_hash().wrapping_mul(0x1000_0000_01b3)
            ^ self.context.wrapping_add(win_bits);
        let hash = self.clock.step(step_hash);
        self.checkpoints.push(Checkpoint {
            tick: self.tick,
            hash,
        });
        self.tick += 1;
    }

    /// Drive the match to completion.
    fn run(mut self) -> Tape {
        while self.tick < self.config.ticks {
            self.step();
        }
        Tape {
            config: self.config,
            inputs: self.inputs,
            checkpoints: self.checkpoints,
            win_tick: self.win_tick,
            winner: self.winner,
        }
    }
}

/// Boot plus headless play: validate, simulate every fixed tick, record.
fn record(config: GameConfig, inputs: &[MoveInput]) -> Result<Tape, InputError> {
    Ok(Sim::build(config, inputs)?.run())
}

/// Replay a tape: re-simulate and stop at the first divergent tick or win.
fn replay(tape: &Tape) -> Result<(), ReplayError> {
    if tape.checkpoints.len() != tape.config.ticks as usize {
        return Err(ReplayError::Count);
    }
    let fresh = record(tape.config, &tape.inputs).map_err(|_| ReplayError::Invalid)?;
    for (want, got) in tape.checkpoints.iter().zip(fresh.checkpoints.iter()) {
        if want != got {
            return Err(ReplayError::Divergence {
                tick: want.tick,
                expected: want.hash,
                actual: got.hash,
            });
        }
    }
    if tape.win_tick != fresh.win_tick || tape.winner != fresh.winner {
        return Err(ReplayError::Win(tape.win_tick.unwrap_or(tape.config.ticks)));
    }
    Ok(())
}

/// Follow-camera center over planar positions: mean plus a lead toward the
/// goal-nearest runner. Pure presentation; never hashed, never sim input.
fn follow_center(positions: &[[f32; 2]], goal: [f32; 2], params: &CameraParams) -> [f32; 3] {
    let count = positions.len().max(1) as f32;
    let (mut sx, mut sy) = (0.0_f32, 0.0_f32);
    for pos in positions {
        sx += pos[0];
        sy += pos[1];
    }
    let avg = [sx / count, sy / count];
    let mut leader = avg;
    let mut best = f32::INFINITY;
    for pos in positions {
        let dx = pos[0] - goal[0];
        let dy = pos[1] - goal[1];
        let d2 = dx * dx + dy * dy;
        if d2 < best {
            best = d2;
            leader = *pos;
        }
    }
    let (ox, oy) = (leader[0] - avg[0], leader[1] - avg[1]);
    let len2 = ox * ox + oy * oy;
    if len2 <= 1e-12 || params.lead_m == 0.0 {
        [avg[0], avg[1], params.height_m]
    } else {
        let len = len2.sqrt();
        [
            avg[0] + params.lead_m * ox / len,
            avg[1] + params.lead_m * oy / len,
            params.height_m,
        ]
    }
}

/// Rules file record: the authority the table test boots from.
#[derive(Clone, PartialEq, Debug)]
struct FileRules {
    /// Rules version; must equal `FIXTURE_VERSION`.
    version: u16,
    /// Match rules.
    config: GameConfig,
    /// Camera table.
    camera: CameraParams,
    /// Ordered moves.
    inputs: Vec<MoveInput>,
}

/// Parse one JSON number after `"key":` (whitespace-tolerant, std only).
fn num_after(text: &str, key: &str) -> Result<f64, String> {
    let quoted = format!("\"{key}\"");
    let start = text
        .find(quoted.as_str())
        .ok_or_else(|| format!("missing key {key}"))?;
    let colon = text[start..]
        .find(':')
        .ok_or_else(|| format!("missing colon for {key}"))?;
    let rest = text[start + colon + 1..].trim_start();
    let mut end = 0_usize;
    for (index, ch) in rest.char_indices() {
        if ch.is_ascii_digit() || matches!(ch, '-' | '+' | '.' | 'e' | 'E') {
            end = index + ch.len_utf8();
        } else {
            break;
        }
    }
    rest[..end]
        .parse::<f64>()
        .map_err(|_| format!("bad number for {key}"))
}

/// Parse one JSON `[a, b]` pair after `"key":`.
fn pair_after(text: &str, key: &str) -> Result<[f64; 2], String> {
    let quoted = format!("\"{key}\"");
    let start = text
        .find(quoted.as_str())
        .ok_or_else(|| format!("missing key {key}"))?;
    let open = text[start..]
        .find('[')
        .ok_or_else(|| format!("missing array for {key}"))?;
    let rest = &text[start + open + 1..];
    let close = rest
        .find(']')
        .ok_or_else(|| format!("unterminated array for {key}"))?;
    let mut values = Vec::new();
    for part in rest[..close].split(',') {
        values.push(
            part.trim()
                .parse::<f64>()
                .map_err(|_| format!("bad array entry for {key}"))?,
        );
    }
    if values.len() != 2 {
        return Err(format!("array for {key} needs 2 entries"));
    }
    Ok([values[0], values[1]])
}

/// Parse the fixture rules file into validated tables (std only, no serde).
fn parse_rules(text: &str) -> Result<FileRules, String> {
    let version = num_after(text, "version")? as u16;
    if version != FIXTURE_VERSION {
        return Err(format!("rules version {version} != {FIXTURE_VERSION}"));
    }
    let mode_start = text.find("\"mode\"").ok_or("missing key mode")?;
    let mode_colon = text[mode_start..].find(':').ok_or("missing colon")?;
    let mode_rest = text[mode_start + mode_colon + 1..].trim_start();
    if !mode_rest.starts_with("\"tag\"") {
        return Err("rules mode must be \"tag\"".to_string());
    }
    let config = GameConfig {
        seed: num_after(text, "seed")? as u64,
        ticks: num_after(text, "ticks")? as u32,
        runners: num_after(text, "runners")? as u8,
        goal: {
            let pair = pair_after(text, "goal_m")?;
            [pair[0] as f32, pair[1] as f32]
        },
        goal_radius_m: num_after(text, "goal_radius_m")? as f32,
        half_extent_m: num_after(text, "half_extent_m")? as f32,
    };
    config
        .validate()
        .map_err(|err| format!("rules config: {err}"))?;
    let camera_block = text
        .find("\"camera\"")
        .and_then(|start| {
            text[start..]
                .find('{')
                .map(|open| &text[start + open + 1..])
        })
        .ok_or("missing camera table")?;
    let camera_end = camera_block.find('}').ok_or("unterminated camera")?;
    let camera_text = &camera_block[..camera_end];
    let camera = CameraParams {
        height_m: num_after(camera_text, "height_m")? as f32,
        lead_m: num_after(camera_text, "lead_m")? as f32,
    };
    camera
        .validate()
        .map_err(|err| format!("rules camera: {err}"))?;
    let inputs_start = text.find("\"inputs\"").ok_or("missing key inputs")?;
    let inputs_open = text[inputs_start..]
        .find('[')
        .ok_or("missing inputs array")?;
    let inputs_text = &text[inputs_start + inputs_open + 1..];
    let mut inputs = Vec::new();
    let mut cursor = 0_usize;
    while let Some(open) = inputs_text[cursor..].find('{') {
        let obj_start = cursor + open;
        let obj_end = inputs_text[obj_start..]
            .find('}')
            .ok_or("unterminated input")?;
        let obj = &inputs_text[obj_start..obj_start + obj_end + 1];
        inputs.push(MoveInput {
            tick: num_after(obj, "tick")? as u32,
            body: num_after(obj, "body")? as u8,
            vx: num_after(obj, "vx")? as f32,
            vy: num_after(obj, "vy")? as f32,
        });
        cursor = obj_start + obj_end + 1;
        if inputs.len() > MAX_INPUTS {
            return Err("too many inputs".to_string());
        }
    }
    if inputs.is_empty() {
        return Err("inputs array is empty".to_string());
    }
    validate_inputs(&config, &inputs).map_err(|err| format!("rules inputs: {err}"))?;
    Ok(FileRules {
        version,
        config,
        camera,
        inputs,
    })
}

/// Load the fixture rules file beside the repository root.
fn load_rules() -> Result<FileRules, String> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("games")
        .join("second-game")
        .join("rules.json");
    let text =
        std::fs::read_to_string(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
    parse_rules(&text)
}

/// Parsed fixture rules, shared across tests in one process.
fn cached_rules() -> FileRules {
    static CACHE: OnceLock<FileRules> = OnceLock::new();
    CACHE
        .get_or_init(|| load_rules().expect("fixture rules load"))
        .clone()
}

/// Base tape for the shipped rules, simulated once per process.
fn cached_tape() -> Tape {
    static CACHE: OnceLock<Tape> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let rules = cached_rules();
            record(rules.config, &rules.inputs).expect("fixture record")
        })
        .clone()
}

/// Scripted moves: chase the goal for `ticks` with one kick per runner.
fn chase_script(config: &GameConfig) -> Vec<MoveInput> {
    let spawns = spawn_positions(config.seed, config.runners, config.half_extent_m);
    let mut inputs = Vec::new();
    for (body, spawn) in spawns.iter().enumerate() {
        let dx = config.goal[0] - spawn[0];
        let dy = config.goal[1] - spawn[1];
        let len2 = dx * dx + dy * dy;
        if len2 <= 1e-12 {
            continue;
        }
        let len = len2.sqrt();
        let speed = 6.0_f32;
        inputs.push(MoveInput {
            tick: (body as u32 * 7) % config.ticks,
            body: body as u8,
            vx: speed * dx / len,
            vy: speed * dy / len,
        });
    }
    inputs.sort_by_key(|input| (input.tick, input.body));
    inputs
}

#[cfg(test)]
mod fixture_tests {
    use super::*;

    /// Rules-file authority: the shipped tables parse, validate and match
    /// the documented second-game fixture values.
    #[test]
    fn rules_file_is_the_authority() {
        let rules = cached_rules();
        assert_eq!(rules.version, FIXTURE_VERSION);
        assert_eq!(rules.config.seed, 77);
        assert_eq!(rules.config.ticks, 120);
        assert_eq!(rules.config.runners, 4);
        assert_eq!(rules.config.goal, [8.0, 6.0]);
        assert_eq!(rules.config.goal_radius_m, 1.5);
        assert_eq!(rules.config.half_extent_m, 20.0);
        assert_eq!(
            rules.camera,
            CameraParams {
                height_m: 12.0,
                lead_m: 2.0,
            }
        );
        assert_eq!(rules.inputs.len(), 3);
    }

    /// Boot plus headless play: the rules file boots, plays without any
    /// device, and records one checkpoint per tick, identically twice.
    #[test]
    fn boots_plays_headless_and_records_twice_identical() {
        let rules = cached_rules();
        let first = cached_tape();
        let second = cached_tape();
        assert_eq!(first, second);
        assert_eq!(first.checkpoints.len(), rules.config.ticks as usize);
        assert_eq!(first.inputs.len(), 3);
        let mut hashes: Vec<u64> = first.checkpoints.iter().map(|point| point.hash).collect();
        hashes.sort_unstable();
        hashes.dedup();
        assert!(hashes.len() > 1, "match never moved");
        replay(&first).expect("fixture replay");
    }

    /// Scripted chase tags the goal: a goal placed on runner 0's spawn wins
    /// on tick 0 with the lowest index, and the same script replays.
    #[test]
    fn chase_script_tags_and_replays() {
        let config = GameConfig {
            seed: 77,
            ticks: 120,
            runners: 4,
            goal: spawn_positions(77, 4, 20.0)[0],
            goal_radius_m: 1.5,
            half_extent_m: 20.0,
        };
        let tape = record(config, &chase_script(&config)).expect("chase record");
        assert_eq!(tape.win_tick, Some(0));
        assert_eq!(tape.winner, Some(0));
        replay(&tape).expect("chase replay");
        let idle = record(config, &[]).expect("idle record");
        assert_eq!(idle.win_tick, Some(0));
        assert_ne!(tape.checkpoints, idle.checkpoints);
    }

    /// Replay names the first divergent tick: corrupting one checkpoint hash
    /// fails at exactly that tick, and a short tape fails the count gate.
    #[test]
    fn replay_names_first_divergence() {
        let mut tape = cached_tape();
        tape.checkpoints[40].hash ^= 1;
        assert_eq!(
            replay(&tape),
            Err(ReplayError::Divergence {
                tick: 40,
                expected: tape.checkpoints[40].hash,
                actual: cached_tape().checkpoints[40].hash,
            })
        );
        tape.checkpoints.pop();
        assert_eq!(replay(&tape), Err(ReplayError::Count));
    }

    /// Failure cases: every out-of-range config, move and rule text fails
    /// loudly instead of simulating.
    #[test]
    fn bad_configs_inputs_and_rules_fail() {
        let good = cached_rules().config;
        for bad in [
            GameConfig { ticks: 0, ..good },
            GameConfig {
                ticks: MAX_TICKS + 1,
                ..good
            },
            GameConfig { runners: 1, ..good },
            GameConfig { runners: 9, ..good },
            GameConfig {
                half_extent_m: f32::NAN,
                ..good
            },
            GameConfig {
                goal_radius_m: 0.1,
                ..good
            },
            GameConfig {
                goal: [99.0, 0.0],
                ..good
            },
        ] {
            assert!(record(bad, &[]).is_err(), "config accepted: {bad:?}");
        }
        let kick = MoveInput {
            tick: 0,
            body: 0,
            vx: 1.0,
            vy: 0.0,
        };
        for bad in [
            vec![MoveInput {
                tick: good.ticks,
                ..kick
            }],
            vec![MoveInput {
                body: good.runners,
                ..kick
            }],
            vec![MoveInput {
                vx: f32::INFINITY,
                ..kick
            }],
            vec![MoveInput {
                vx: MAX_SPEED_MPS + 1.0,
                ..kick
            }],
            vec![kick, kick],
            vec![MoveInput { tick: 2, ..kick }, MoveInput { tick: 1, ..kick }],
        ] {
            assert!(record(good, &bad).is_err(), "inputs accepted: {bad:?}");
        }
        assert!(parse_rules("{\"version\": 2}").is_err());
        assert!(parse_rules("not json").is_err());
    }

    /// Camera is data and presentation only: two tables over one tape share
    /// every checkpoint and hash but frame different centers.
    #[test]
    fn camera_tables_share_hash_but_frame_differently() {
        let rules = cached_rules();
        let tape = cached_tape();
        let near = CameraParams {
            height_m: 6.0,
            lead_m: 0.0,
        };
        let far = CameraParams {
            height_m: 20.0,
            lead_m: 4.0,
        };
        near.validate().expect("near camera");
        far.validate().expect("far camera");
        let sim = Sim::build(rules.config, &rules.inputs).expect("fixture build");
        let positions: Vec<[f32; 2]> = sim.world.pos.iter().map(|pos| [pos.x, pos.y]).collect();
        let (near_center, far_center) = (
            follow_center(&positions, rules.config.goal, &near),
            follow_center(&positions, rules.config.goal, &far),
        );
        assert_ne!(near_center, far_center);
        assert_eq!(near_center[2], 6.0);
        assert_eq!(far_center[2], 20.0);
        let _ = tape;
    }

    /// Different rules, different game: moving the goal disc changes the
    /// receipts while the same seed still simulates deterministically.
    #[test]
    fn different_goal_is_a_different_game() {
        let rules = cached_rules();
        let moved = GameConfig {
            goal: [-8.0, -6.0],
            ..rules.config
        };
        let base = cached_tape();
        let other = record(moved, &rules.inputs).expect("moved record");
        assert_ne!(base.checkpoints, other.checkpoints);
        assert_eq!(other, record(moved, &rules.inputs).expect("moved record"));
    }

    /// Neighboring seeds never alias: SplitMix64 gives every seed its own
    /// spawns and its own receipts.
    #[test]
    fn neighboring_seeds_never_alias() {
        let rules = cached_rules();
        let even = GameConfig {
            seed: 78,
            ..rules.config
        };
        let odd = GameConfig {
            seed: 79,
            ..rules.config
        };
        assert_ne!(
            spawn_positions(even.seed, even.runners, even.half_extent_m),
            spawn_positions(odd.seed, odd.runners, odd.half_extent_m)
        );
        assert_ne!(
            record(even, &rules.inputs)
                .expect("even record")
                .checkpoints,
            record(odd, &rules.inputs).expect("odd record").checkpoints
        );
    }
}
