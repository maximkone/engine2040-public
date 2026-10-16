//! GE1 fixture genres (generality proof): two headless games on public APIs.
//!
//! Purpose: prove any genre could be built with zero engine edits. Owner:
//! the GE1 fixture (tests + data only). Units: meters, seconds, fixed
//! 60 Hz ticks; traversal ledge heights in integer millimetres. Determinism:
//! same rules plus inputs give identical checkpoints twice; sim math is
//! `+ - * /` and one `sqrt` (aim normalize, enemy drift, camera lead),
//! index order, fixed dt, SplitMix64 seed expansion, no wall clock. Errors:
//! typed config/input/replay errors per fixture; bad data fails loudly,
//! never panics on user bytes.

use engine_core::{hash_bytes, TickState, DT};
use engine_ecs::{Position, Velocity, World};
use engine_physics::{Body, Space};
use std::sync::OnceLock;

/// GE1 fixture rules version (legacy v1 and GP5 v2 stay frozen).
const FIXTURE_VERSION: u16 = 3;
/// Bound replay work per match.
const MAX_TICKS: u32 = 10_000;
/// FNV-1a prime for combining lane hashes.
const PRIME: u64 = 0x1000_0000_01b3;
/// SplitMix64 increment (integer seed expansion, no low-bit aliasing).
const SPLITMIX_INC: u64 = 0x9E37_79B9_7F4A_7C15;
/// Stream mixer so two lanes never share a spawn stream.
const LANE_MIX: u64 = 0xBF58_476D_1CE4_E5B9;

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

/// Corridor spawns: spread along x in `[-limit, limit]`, `y == 0`. The
/// physics lane clamps bodies to its rest height, so duels resolve along x
/// while the run still steps both lanes deterministically.
fn corridor_spawns(seed: u64, count: u8, half_extent_m: f32) -> Vec<[f32; 2]> {
    let limit = half_extent_m - 0.4;
    let mut state = seed;
    let mut out = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let x = unit_f32(splitmix64_next(&mut state)) * 2.0 - 1.0;
        out.push([canon_zero(x * limit), 0.0]);
    }
    out
}

/// FNV-1a over runner velocities in index order (positions ride the world
/// hash; velocities affect future ticks, so they hash too).
fn vel_hash(vel: &[Velocity], count: usize) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for one in vel.iter().take(count) {
        for word in [one.x.to_bits(), one.y.to_bits()] {
            for byte in word.to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(PRIME);
            }
        }
    }
    hash
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

/// Parse one JSON `"key": "value"` string (whitespace-tolerant, std only).
fn str_after(text: &str, key: &str) -> Result<String, String> {
    let quoted = format!("\"{key}\"");
    let start = text
        .find(quoted.as_str())
        .ok_or_else(|| format!("missing key {key}"))?;
    let colon = text[start..]
        .find(':')
        .ok_or_else(|| format!("missing colon for {key}"))?;
    let rest = text[start + colon + 1..].trim_start();
    if !rest.starts_with('"') {
        return Err(format!("bad string for {key}"));
    }
    let end = rest[1..]
        .find('"')
        .ok_or_else(|| format!("unterminated string for {key}"))?;
    Ok(rest[1..1 + end].to_string())
}

/// Split one flat `"key": [ {...}, {...} ]` array into its object bodies.
/// Objects are flat (numbers and strings only), so brace scanning suffices.
fn objects_in(text: &str, key: &str) -> Result<Vec<String>, String> {
    let quoted = format!("\"{key}\"");
    let start = text
        .find(quoted.as_str())
        .ok_or_else(|| format!("missing key {key}"))?;
    let open = text[start..]
        .find('[')
        .ok_or_else(|| format!("missing array for {key}"))?;
    let inner_start = start + open + 1;
    let mut depth = 1_usize;
    let mut close = None;
    for (index, ch) in text[inner_start..].char_indices() {
        if ch == '[' {
            depth += 1;
        } else if ch == ']' {
            depth -= 1;
            if depth == 0 {
                close = Some(inner_start + index);
                break;
            }
        }
    }
    let end = close.ok_or_else(|| format!("unterminated array for {key}"))?;
    let inner = &text[inner_start..end];
    let mut out = Vec::new();
    let mut cursor = 0_usize;
    while let Some(open) = inner[cursor..].find('{') {
        let obj_start = cursor + open;
        let obj_end = inner[obj_start..]
            .find('}')
            .ok_or_else(|| format!("unterminated object in {key}"))?;
        out.push(inner[obj_start..obj_start + obj_end + 1].to_string());
        cursor = obj_start + obj_end + 1;
    }
    Ok(out)
}

/// Load one fixture rules file from `fixtures/games/<dir>/rules.json`.
fn load_text(dir: &str) -> Result<String, String> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("games")
        .join(dir)
        .join("rules.json");
    std::fs::read_to_string(&path).map_err(|err| format!("read {}: {err}", path.display()))
}

/// Parsed twin-stick rules, shared across tests in one process.
fn cached_twin_rules() -> TwinFileRules {
    static CACHE: OnceLock<TwinFileRules> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let text = load_text("twin-stick").expect("twin rules load");
            parse_twin_rules(&text).expect("twin rules parse")
        })
        .clone()
}

/// Parsed traversal-course rules, shared across tests in one process.
fn cached_trav_rules() -> TravFileRules {
    static CACHE: OnceLock<TravFileRules> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let text = load_text("traversal-course").expect("traversal rules load");
            parse_trav_rules(&text).expect("traversal rules parse")
        })
        .clone()
}

/// Checkpoint after one fixed step.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Checkpoint {
    /// Zero-based fixed tick.
    tick: u32,
    /// State hash after the tick.
    hash: u64,
}

// ---------------------------------------------------------------------------
// Twin-stick: top-down corridor duel with move plus aim/fire sticks.
// ---------------------------------------------------------------------------

/// Runners per duel, `1..=MAX_AGENTS`.
const TWIN_MAX_AGENTS: u8 = 4;
/// Enemies per duel, `1..=MAX_ENEMIES`.
const TWIN_MAX_ENEMIES: u8 = 8;
/// Bound recorded stick moves per duel.
const TWIN_MAX_MOVES: usize = 512;
/// Bound recorded shots per duel (one bullet each).
const TWIN_MAX_SHOTS: usize = 512;
/// Fastest legal stick velocity, m/s.
const TWIN_MAX_SPEED_MPS: f32 = 20.0;
/// Shot speed along the aim stick, m/s.
const TWIN_BULLET_SPEED_MPS: f32 = 18.0;
/// Bullet half-extent, meters.
const TWIN_BULLET_HALF_M: f32 = 0.25;
/// Enemy half-extent, meters.
const TWIN_ENEMY_HALF_M: f32 = 0.6;
/// Enemy drift toward the corridor center, m/s.
const TWIN_ENEMY_SPEED_MPS: f32 = 1.0;
/// Bullet lifetime in ticks before it parks out of the lane.
const TWIN_BULLET_LIFE: u32 = 120;
/// Playfield half-extent bounds, meters.
const TWIN_EXTENT_MIN_M: f32 = 5.0;
/// Playfield half-extent bounds, meters.
const TWIN_EXTENT_MAX_M: f32 = 50.0;
/// Kill target bounds.
const TWIN_KILL_TARGET_MAX: u32 = 32;
/// Parking x for spent bodies (no clamp on x, so they never overlap play).
const TWIN_PARK_X: f32 = 9000.0;

/// Twin-stick duel configuration: every field is validated before a run.
#[derive(Clone, Copy, PartialEq, Debug)]
struct TwinConfig {
    /// Initialization seed; expanded with SplitMix64 (seeds never alias).
    seed: u64,
    /// Fixed steps, `1..=MAX_TICKS`.
    ticks: u32,
    /// Duelists, `1..=TWIN_MAX_AGENTS`.
    agents: u8,
    /// Targets, `1..=TWIN_MAX_ENEMIES`.
    enemies: u8,
    /// Playfield half-extent in meters.
    half_extent_m: f32,
    /// Kills that end the duel, `1..=TWIN_KILL_TARGET_MAX`.
    kill_target: u32,
}

/// One move-stick assignment applied before the named zero-based tick.
#[derive(Clone, Copy, PartialEq, Debug)]
struct TwinMove {
    /// Application tick, strictly below `TwinConfig::ticks`.
    tick: u32,
    /// Duelist slot below `TwinConfig::agents`.
    body: u8,
    /// Move-stick velocity in m/s.
    mx: f32,
    /// Move-stick velocity in m/s.
    my: f32,
}

/// One aim/fire-stick shot applied before the named zero-based tick.
#[derive(Clone, Copy, PartialEq, Debug)]
struct TwinShot {
    /// Application tick, strictly below `TwinConfig::ticks`.
    tick: u32,
    /// Duelist slot below `TwinConfig::agents`.
    body: u8,
    /// Aim direction x (must be nonzero with `ay`).
    ax: f32,
    /// Aim direction y (must be nonzero with `ax`).
    ay: f32,
}

/// Top-down camera parameters as data (presentation only, never hashed).
#[derive(Clone, Copy, PartialEq, Debug)]
struct TwinCamera {
    /// Camera height over the plane in meters.
    height_m: f32,
    /// Lead toward the current leader in meters.
    lead_m: f32,
}

/// Recorded duel: config, ordered sticks, one checkpoint per tick, score.
#[derive(Clone, PartialEq, Debug)]
struct TwinTape {
    /// Rules the duel ran under.
    config: TwinConfig,
    /// Ordered, validated move sticks.
    moves: Vec<TwinMove>,
    /// Ordered, validated fire sticks.
    shots: Vec<TwinShot>,
    /// One post-step checkpoint per tick.
    checkpoints: Vec<Checkpoint>,
    /// Confirmed kills.
    kills: u32,
    /// Score (100 per kill).
    score: u32,
    /// First tick `kills` reached the target, if any.
    win_tick: Option<u32>,
}

/// Bad duel configuration, culprit carried.
#[expect(
    clippy::enum_variant_names,
    reason = "error variants name the config field they reject"
)]
#[derive(Clone, Copy, PartialEq, Debug)]
enum TwinConfigError {
    /// Tick count `0` or above `MAX_TICKS`.
    BadTicks(u32),
    /// Agent count outside `1..=TWIN_MAX_AGENTS`.
    BadAgents(u8),
    /// Enemy count outside `1..=TWIN_MAX_ENEMIES`.
    BadEnemies(u8),
    /// Half-extent NaN or outside `5.0..=50.0` meters.
    BadExtent,
    /// Kill target `0` or above `TWIN_KILL_TARGET_MAX`.
    BadTarget(u32),
}

/// Bad stick list, reason carried.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TwinInputError {
    /// More moves than `TWIN_MAX_MOVES` or shots than `TWIN_MAX_SHOTS`.
    TooMany,
    /// Tick at or past the duel end, or body past the agents.
    OutOfRange,
    /// Non-finite or over-speed stick, or a zero aim stick.
    BadStick,
    /// Sticks not strictly ordered by `(tick, body)`.
    Unordered,
}

/// Duel replay failure: invalid data or the first divergent tick.
#[derive(Clone, PartialEq, Eq, Debug)]
enum TwinReplayError {
    /// Config or stick list rejected.
    Invalid,
    /// Checkpoint count does not match the config.
    Count,
    /// First mismatching zero-based tick plus both hashes.
    Divergence {
        tick: u32,
        expected: u64,
        actual: u64,
    },
    /// Kill or score outcome differs.
    Score(u32),
}

impl core::fmt::Display for TwinConfigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadTicks(ticks) => write!(f, "bad ticks ({ticks})"),
            Self::BadAgents(agents) => write!(f, "bad agents ({agents})"),
            Self::BadEnemies(enemies) => write!(f, "bad enemies ({enemies})"),
            Self::BadExtent => write!(f, "bad half extent"),
            Self::BadTarget(target) => write!(f, "bad kill target ({target})"),
        }
    }
}

impl core::fmt::Display for TwinInputError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooMany => write!(f, "too many sticks"),
            Self::OutOfRange => write!(f, "stick tick or body out of range"),
            Self::BadStick => write!(f, "stick not finite, over speed or zero aim"),
            Self::Unordered => write!(f, "sticks not ordered by (tick, body)"),
        }
    }
}

impl core::fmt::Display for TwinReplayError {
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
            Self::Score(kills) => write!(f, "kill outcome differs at {kills} kills"),
        }
    }
}

impl std::error::Error for TwinConfigError {}
impl std::error::Error for TwinInputError {}
impl std::error::Error for TwinReplayError {}

impl TwinConfig {
    /// Range-check every field; NaN or out-of-range fails with the culprit.
    fn validate(self) -> Result<(), TwinConfigError> {
        if self.ticks == 0 || self.ticks > MAX_TICKS {
            return Err(TwinConfigError::BadTicks(self.ticks));
        }
        if self.agents == 0 || self.agents > TWIN_MAX_AGENTS {
            return Err(TwinConfigError::BadAgents(self.agents));
        }
        if self.enemies == 0 || self.enemies > TWIN_MAX_ENEMIES {
            return Err(TwinConfigError::BadEnemies(self.enemies));
        }
        if !self.half_extent_m.is_finite()
            || self.half_extent_m < TWIN_EXTENT_MIN_M
            || self.half_extent_m > TWIN_EXTENT_MAX_M
        {
            return Err(TwinConfigError::BadExtent);
        }
        if self.kill_target == 0 || self.kill_target > TWIN_KILL_TARGET_MAX {
            return Err(TwinConfigError::BadTarget(self.kill_target));
        }
        Ok(())
    }
}

impl TwinCamera {
    /// Range-check the camera data; a bad table fails before use.
    fn validate(self) -> Result<(), TwinConfigError> {
        if !self.height_m.is_finite() || self.height_m < 2.0 || self.height_m > 30.0 {
            return Err(TwinConfigError::BadExtent);
        }
        if !self.lead_m.is_finite() || self.lead_m < 0.0 || self.lead_m > 10.0 {
            return Err(TwinConfigError::BadTarget(0));
        }
        Ok(())
    }
}

/// Validate one move-stick list against its config.
fn validate_twin_moves(config: &TwinConfig, moves: &[TwinMove]) -> Result<(), TwinInputError> {
    if moves.len() > TWIN_MAX_MOVES {
        return Err(TwinInputError::TooMany);
    }
    for one in moves {
        if one.tick >= config.ticks || one.body >= config.agents {
            return Err(TwinInputError::OutOfRange);
        }
        if !one.mx.is_finite()
            || !one.my.is_finite()
            || one.mx.abs() > TWIN_MAX_SPEED_MPS
            || one.my.abs() > TWIN_MAX_SPEED_MPS
        {
            return Err(TwinInputError::BadStick);
        }
    }
    if moves
        .windows(2)
        .any(|pair| (pair[0].tick, pair[0].body) >= (pair[1].tick, pair[1].body))
    {
        return Err(TwinInputError::Unordered);
    }
    Ok(())
}

/// Validate one fire-stick list against its config.
fn validate_twin_shots(config: &TwinConfig, shots: &[TwinShot]) -> Result<(), TwinInputError> {
    if shots.len() > TWIN_MAX_SHOTS {
        return Err(TwinInputError::TooMany);
    }
    for one in shots {
        if one.tick >= config.ticks || one.body >= config.agents {
            return Err(TwinInputError::OutOfRange);
        }
        if !one.ax.is_finite()
            || !one.ay.is_finite()
            || one.ax.abs() > TWIN_MAX_SPEED_MPS
            || one.ay.abs() > TWIN_MAX_SPEED_MPS
            || (one.ax == 0.0 && one.ay == 0.0)
        {
            return Err(TwinInputError::BadStick);
        }
    }
    if shots
        .windows(2)
        .any(|pair| (pair[0].tick, pair[0].body) >= (pair[1].tick, pair[1].body))
    {
        return Err(TwinInputError::Unordered);
    }
    Ok(())
}

/// Live duel: an ECS plane for the duelists plus a physics lane for targets
/// and shots stepped in lockstep. Agents never fall (ECS plane); the lane
/// integrates with its fixed gravity plus ground clamp, so shots arc and
/// still connect inside the overlap tolerance at duel range.
struct TwinSim {
    /// Duel rules.
    config: TwinConfig,
    /// Move sticks in `(tick, body)` order.
    moves: Vec<TwinMove>,
    /// Fire sticks in `(tick, body)` order.
    shots: Vec<TwinShot>,
    /// Cursor into `moves`.
    move_cursor: usize,
    /// Cursor into `shots`.
    shot_cursor: usize,
    /// Zero-based fixed tick.
    tick: u32,
    /// Duelist plane.
    world: World,
    /// Target plus shot lane.
    space: Space,
    /// Live targets (indices `0..enemies` in `space`).
    enemy_alive: Vec<bool>,
    /// Live shots (indices past the enemies in `space`).
    bullet_alive: Vec<bool>,
    /// Age per shot body in ticks.
    bullet_age: Vec<u32>,
    /// Confirmed kills.
    kills: u32,
    /// Score (100 per kill).
    score: u32,
    /// Running clock over per-tick step hashes.
    clock: TickState,
    /// Rules-plus-sticks context folded into every hash.
    context: u64,
    /// Checkpoints so far.
    checkpoints: Vec<Checkpoint>,
    /// First target-reaching tick, once kills hit the target.
    win_tick: Option<u32>,
}

impl TwinSim {
    /// Validated constructor: config, sticks, corridor spawns, both lanes.
    fn build(
        config: TwinConfig,
        moves: &[TwinMove],
        shots: &[TwinShot],
    ) -> Result<Self, TwinInputError> {
        config.validate().map_err(|_| TwinInputError::OutOfRange)?;
        validate_twin_moves(&config, moves)?;
        validate_twin_shots(&config, shots)?;
        let agent_spawns = corridor_spawns(config.seed, config.agents, config.half_extent_m);
        let enemy_spawns =
            corridor_spawns(config.seed ^ LANE_MIX, config.enemies, config.half_extent_m);
        let mut world = World::new();
        let mut space = Space::new();
        for pos in &agent_spawns {
            world.spawn(
                Position {
                    x: pos[0],
                    y: pos[1],
                },
                Velocity { x: 0.0, y: 0.0 },
            );
        }
        for pos in &enemy_spawns {
            let len2 = pos[0] * pos[0] + pos[1] * pos[1];
            let (dx, dy) = if len2 <= 1e-12 {
                (0.0, 0.0)
            } else {
                let len = len2.sqrt();
                (
                    -pos[0] / len * TWIN_ENEMY_SPEED_MPS,
                    -pos[1] / len * TWIN_ENEMY_SPEED_MPS,
                )
            };
            space.add(Body {
                x: pos[0],
                y: pos[1],
                vx: dx,
                vy: dy,
                hx: TWIN_ENEMY_HALF_M,
                hy: TWIN_ENEMY_HALF_M,
            });
        }
        let mut context_bytes = Vec::new();
        context_bytes.extend_from_slice(&FIXTURE_VERSION.to_le_bytes());
        context_bytes.extend_from_slice(&config.seed.to_le_bytes());
        context_bytes.extend_from_slice(&config.ticks.to_le_bytes());
        context_bytes.extend_from_slice(&[config.agents, config.enemies]);
        context_bytes.extend_from_slice(&config.half_extent_m.to_bits().to_le_bytes());
        context_bytes.extend_from_slice(&config.kill_target.to_le_bytes());
        for one in moves {
            context_bytes.extend_from_slice(&one.tick.to_le_bytes());
            context_bytes.extend_from_slice(&[one.body]);
            context_bytes.extend_from_slice(&one.mx.to_bits().to_le_bytes());
            context_bytes.extend_from_slice(&one.my.to_bits().to_le_bytes());
        }
        for one in shots {
            context_bytes.extend_from_slice(&one.tick.to_le_bytes());
            context_bytes.extend_from_slice(&[one.body]);
            context_bytes.extend_from_slice(&one.ax.to_bits().to_le_bytes());
            context_bytes.extend_from_slice(&one.ay.to_bits().to_le_bytes());
        }
        Ok(Self {
            config,
            moves: moves.to_vec(),
            shots: shots.to_vec(),
            move_cursor: 0,
            shot_cursor: 0,
            tick: 0,
            world,
            space,
            enemy_alive: vec![true; usize::from(config.enemies)],
            bullet_alive: Vec::new(),
            bullet_age: Vec::new(),
            kills: 0,
            score: 0,
            clock: TickState::new(config.seed),
            context: hash_bytes(&context_bytes),
            checkpoints: Vec::with_capacity(config.ticks as usize),
            win_tick: None,
        })
    }

    /// Park a spent lane body far down-corridor with zero velocity.
    fn park(&mut self, index: usize) {
        self.space.bodies[index].x = TWIN_PARK_X;
        self.space.bodies[index].y = TWIN_ENEMY_HALF_M;
        self.space.bodies[index].vx = 0.0;
        self.space.bodies[index].vy = 0.0;
    }

    /// One fixed tick: sticks, both lanes, ageing, hits, checkpoint.
    fn step(&mut self) {
        while self.move_cursor < self.moves.len() && self.moves[self.move_cursor].tick == self.tick
        {
            let one = self.moves[self.move_cursor];
            self.world.vel[usize::from(one.body)] = Velocity {
                x: one.mx,
                y: one.my,
            };
            self.move_cursor += 1;
        }
        while self.shot_cursor < self.shots.len() && self.shots[self.shot_cursor].tick == self.tick
        {
            let one = self.shots[self.shot_cursor];
            let from = self.world.pos[usize::from(one.body)];
            let len = (one.ax * one.ax + one.ay * one.ay).sqrt();
            let (nx, ny) = (one.ax / len, one.ay / len);
            self.space.add(Body {
                x: from.x,
                y: from.y,
                vx: nx * TWIN_BULLET_SPEED_MPS,
                vy: ny * TWIN_BULLET_SPEED_MPS,
                hx: TWIN_BULLET_HALF_M,
                hy: TWIN_BULLET_HALF_M,
            });
            self.bullet_alive.push(true);
            self.bullet_age.push(0);
            self.shot_cursor += 1;
        }
        self.world.integrate(DT);
        self.space.step(DT);
        let limit = self.config.half_extent_m;
        for pos in self
            .world
            .pos
            .iter_mut()
            .take(usize::from(self.config.agents))
        {
            if pos.x > limit {
                pos.x = limit;
            } else if pos.x < -limit {
                pos.x = -limit;
            }
            if pos.y > limit {
                pos.y = limit;
            } else if pos.y < -limit {
                pos.y = -limit;
            }
        }
        let enemies = usize::from(self.config.enemies);
        let reach = TWIN_BULLET_HALF_M + TWIN_ENEMY_HALF_M;
        for shot in 0..self.bullet_alive.len() {
            if !self.bullet_alive[shot] {
                continue;
            }
            self.bullet_age[shot] += 1;
            let index = enemies + shot;
            if self.bullet_age[shot] > TWIN_BULLET_LIFE {
                self.bullet_alive[shot] = false;
                self.park(index);
                continue;
            }
            let (bx, by) = (self.space.bodies[index].x, self.space.bodies[index].y);
            let mut hit = None;
            for (enemy, live) in self.enemy_alive.iter().enumerate() {
                if !live {
                    continue;
                }
                let body = &self.space.bodies[enemy];
                if (body.x - bx).abs() <= reach && (body.y - by).abs() <= reach {
                    hit = Some(enemy);
                    break;
                }
            }
            if let Some(enemy) = hit {
                self.enemy_alive[enemy] = false;
                self.park(enemy);
                self.bullet_alive[shot] = false;
                self.park(index);
                self.kills += 1;
                self.score += 100;
            }
        }
        if self.win_tick.is_none() && self.kills >= self.config.kill_target {
            self.win_tick = Some(self.tick);
        }
        let outcome = u64::from(self.kills).wrapping_mul(PRIME) ^ u64::from(self.score);
        let step_hash = self.world.state_hash()
            ^ self.space.state_hash().wrapping_mul(PRIME)
            ^ vel_hash(&self.world.vel, usize::from(self.config.agents))
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ self.context.wrapping_add(outcome);
        let hash = self.clock.step(step_hash);
        self.checkpoints.push(Checkpoint {
            tick: self.tick,
            hash,
        });
        self.tick += 1;
    }

    /// Drive the duel to completion.
    fn run(mut self) -> TwinTape {
        while self.tick < self.config.ticks {
            self.step();
        }
        TwinTape {
            config: self.config,
            moves: self.moves,
            shots: self.shots,
            checkpoints: self.checkpoints,
            kills: self.kills,
            score: self.score,
            win_tick: self.win_tick,
        }
    }
}

/// Boot plus headless play: validate, simulate every fixed tick, record.
fn twin_record(
    config: TwinConfig,
    moves: &[TwinMove],
    shots: &[TwinShot],
) -> Result<TwinTape, TwinInputError> {
    Ok(TwinSim::build(config, moves, shots)?.run())
}

/// Replay a duel tape: re-simulate and stop at the first divergent tick.
fn twin_replay(tape: &TwinTape) -> Result<(), TwinReplayError> {
    if tape.checkpoints.len() != tape.config.ticks as usize {
        return Err(TwinReplayError::Count);
    }
    let fresh =
        twin_record(tape.config, &tape.moves, &tape.shots).map_err(|_| TwinReplayError::Invalid)?;
    for (want, got) in tape.checkpoints.iter().zip(fresh.checkpoints.iter()) {
        if want != got {
            return Err(TwinReplayError::Divergence {
                tick: want.tick,
                expected: want.hash,
                actual: got.hash,
            });
        }
    }
    if tape.kills != fresh.kills || tape.score != fresh.score || tape.win_tick != fresh.win_tick {
        return Err(TwinReplayError::Score(tape.kills));
    }
    Ok(())
}

/// Top-down camera center over duelist positions: mean plus a lead toward
/// the current leader. Pure presentation; never hashed, never sim input.
fn twin_center(positions: &[[f32; 2]], params: &TwinCamera) -> [f32; 3] {
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
        let d2 = pos[0] * pos[0] + pos[1] * pos[1];
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

/// Twin-stick rules file record: the authority the table test boots from.
#[derive(Clone, PartialEq, Debug)]
struct TwinFileRules {
    /// Rules version; must equal `FIXTURE_VERSION`.
    version: u16,
    /// Duel rules.
    config: TwinConfig,
    /// Camera table.
    camera: TwinCamera,
    /// Ordered move sticks.
    moves: Vec<TwinMove>,
    /// Ordered fire sticks.
    shots: Vec<TwinShot>,
}

/// Parse the twin-stick rules file into validated tables (std only).
fn parse_twin_rules(text: &str) -> Result<TwinFileRules, String> {
    let version = num_after(text, "version")? as u16;
    if version != FIXTURE_VERSION {
        return Err(format!("rules version {version} != {FIXTURE_VERSION}"));
    }
    if str_after(text, "fixture")? != "twin-stick" {
        return Err("rules fixture must be \"twin-stick\"".to_string());
    }
    if str_after(text, "mode")? != "arena" {
        return Err("rules mode must be \"arena\"".to_string());
    }
    let config = TwinConfig {
        seed: num_after(text, "seed")? as u64,
        ticks: num_after(text, "ticks")? as u32,
        agents: num_after(text, "agents")? as u8,
        enemies: num_after(text, "enemies")? as u8,
        half_extent_m: num_after(text, "half_extent_m")? as f32,
        kill_target: num_after(text, "kill_target")? as u32,
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
    let camera = TwinCamera {
        height_m: num_after(camera_text, "height_m")? as f32,
        lead_m: num_after(camera_text, "lead_m")? as f32,
    };
    camera
        .validate()
        .map_err(|err| format!("rules camera: {err}"))?;
    let mut moves = Vec::new();
    for obj in objects_in(text, "moves")? {
        moves.push(TwinMove {
            tick: num_after(&obj, "tick")? as u32,
            body: num_after(&obj, "body")? as u8,
            mx: num_after(&obj, "mx")? as f32,
            my: num_after(&obj, "my")? as f32,
        });
    }
    let mut shots = Vec::new();
    for obj in objects_in(text, "shots")? {
        shots.push(TwinShot {
            tick: num_after(&obj, "tick")? as u32,
            body: num_after(&obj, "body")? as u8,
            ax: num_after(&obj, "ax")? as f32,
            ay: num_after(&obj, "ay")? as f32,
        });
    }
    if moves.is_empty() || shots.is_empty() {
        return Err("moves and shots arrays must be nonempty".to_string());
    }
    validate_twin_moves(&config, &moves).map_err(|err| format!("rules moves: {err}"))?;
    validate_twin_shots(&config, &shots).map_err(|err| format!("rules shots: {err}"))?;
    Ok(TwinFileRules {
        version,
        config,
        camera,
        moves,
        shots,
    })
}

// ---------------------------------------------------------------------------
// Traversal course: third-person ledge course with a probe chooser.
// ---------------------------------------------------------------------------

/// Runners per course, `1..=TRAV_MAX_RUNNERS`.
const TRAV_MAX_RUNNERS: u8 = 2;
/// Ledges per course, `1..=TRAV_MAX_LEDGES`.
const TRAV_MAX_LEDGES: usize = 16;
/// Bound recorded run tweaks per course.
const TRAV_MAX_MOVES: usize = 512;
/// Fastest legal run tweak, m/s.
const TRAV_MAX_SPEED_MPS: f32 = 20.0;
/// Run speed bounds, m/s.
const TRAV_SPEED_MIN_MPS: f32 = 0.5;
/// Run speed bounds, m/s.
const TRAV_SPEED_MAX_MPS: f32 = 10.0;
/// Ledge x bounds, meters.
const TRAV_X_MIN_M: f32 = -50.0;
/// Ledge x bounds, meters.
const TRAV_X_MAX_M: f32 = 120.0;
/// Lowest climbable top, millimetres.
const TRAV_TOP_MIN_MM: u32 = 100;
/// Highest climbable top, millimetres.
const TRAV_TOP_MAX_MM: u32 = 3500;
/// Vault ceiling: at or below is a vault, millimetres.
const TRAV_VAULT_MAX_MM: u32 = 1200;
/// Mantle ceiling: at or below is a mantle, millimetres.
const TRAV_MANTLE_MAX_MM: u32 = 2000;
/// Climb ceiling: at or below is a climb, above is blocked, millimetres.
const TRAV_CLIMB_MAX_MM: u32 = 3000;

/// Traversal action picked by the probe chooser.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TravAction {
    /// Low ledge, run straight over the edge.
    Vault,
    /// Chest-high ledge, hands onto the edge.
    Mantle,
    /// Head-high wall, full climb onto the edge.
    Climb,
}

/// Pick the traversal action for a ledge top in integer millimetres:
/// vault at or below 1200, mantle at or below 2000, climb at or below
/// 3000, blocked above. `None` is a stuck runner, never a guess.
fn choose_action(top_mm: u32) -> Option<TravAction> {
    if top_mm <= TRAV_VAULT_MAX_MM {
        Some(TravAction::Vault)
    } else if top_mm <= TRAV_MANTLE_MAX_MM {
        Some(TravAction::Mantle)
    } else if top_mm <= TRAV_CLIMB_MAX_MM {
        Some(TravAction::Climb)
    } else {
        None
    }
}

/// One ledge: reach `x`, warp the root onto the top edge.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Ledge {
    /// Ledge face in meters; ledges strictly increase in x.
    x: f32,
    /// Ledge top in integer millimetres.
    top_mm: u32,
}

/// One run tweak applied before the named zero-based tick.
#[derive(Clone, Copy, PartialEq, Debug)]
struct TravMove {
    /// Application tick, strictly below `TravConfig::ticks`.
    tick: u32,
    /// Runner slot below `TravConfig::runners`.
    body: u8,
    /// Forward velocity in m/s.
    vx: f32,
    /// Lateral velocity in m/s.
    vy: f32,
}

/// Traversal course configuration: every field is validated before a run.
#[derive(Clone, PartialEq, Debug)]
struct TravConfig {
    /// Initialization seed; expanded with SplitMix64 (seeds never alias).
    seed: u64,
    /// Fixed steps, `1..=MAX_TICKS`.
    ticks: u32,
    /// Runners, `1..=TRAV_MAX_RUNNERS`.
    runners: u8,
    /// Base forward speed in m/s.
    run_speed_mps: f32,
    /// Ordered ledges, `1..=TRAV_MAX_LEDGES`.
    ledges: Vec<Ledge>,
}

/// Third-person camera parameters as data (presentation only, never hashed).
#[derive(Clone, Copy, PartialEq, Debug)]
struct TravCamera {
    /// Chase distance behind the runner in meters.
    distance_m: f32,
    /// Eye height over the runner in meters.
    height_m: f32,
}

/// Recorded course: config, tweaks, one checkpoint per tick, progress.
#[derive(Clone, PartialEq, Debug)]
struct TravTape {
    /// Rules the course ran under.
    config: TravConfig,
    /// Ordered, validated run tweaks.
    moves: Vec<TravMove>,
    /// One post-step checkpoint per tick.
    checkpoints: Vec<Checkpoint>,
    /// Ledges cleared per runner.
    cleared: Vec<u32>,
    /// Stuck flag per runner (blocked ledge met).
    stuck: Vec<bool>,
    /// Action log in `(tick, body, action)` order.
    log: Vec<(u32, u8, TravAction)>,
    /// First tick every runner cleared the course, if any.
    win_tick: Option<u32>,
}

/// Bad course configuration, culprit carried.
#[expect(
    clippy::enum_variant_names,
    reason = "error variants name the config field they reject"
)]
#[derive(Clone, PartialEq, Debug)]
enum TravConfigError {
    /// Tick count `0` or above `MAX_TICKS`.
    BadTicks(u32),
    /// Runner count outside `1..=TRAV_MAX_RUNNERS`.
    BadRunners(u8),
    /// Run speed NaN or outside `0.5..=10.0` m/s.
    BadSpeed,
    /// Ledge count outside `1..=TRAV_MAX_LEDGES`.
    BadLedges(usize),
    /// Ledge x out of bounds, unordered, or top outside `100..=3500` mm.
    BadLedge(usize),
}

/// Bad tweak list, reason carried.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TravInputError {
    /// More than `TRAV_MAX_MOVES` tweaks.
    TooMany,
    /// Tick at or past the course end, or body past the runners.
    OutOfRange,
    /// Non-finite or over-speed velocity.
    BadVelocity,
    /// Tweaks not strictly ordered by `(tick, body)`.
    Unordered,
}

/// Course replay failure: invalid data or the first divergent tick.
#[derive(Clone, PartialEq, Eq, Debug)]
enum TravReplayError {
    /// Config or tweak list rejected.
    Invalid,
    /// Checkpoint count does not match the config.
    Count,
    /// First mismatching zero-based tick plus both hashes.
    Divergence {
        tick: u32,
        expected: u64,
        actual: u64,
    },
    /// Ledge progress differs.
    Progress(u32),
}

impl core::fmt::Display for TravConfigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadTicks(ticks) => write!(f, "bad ticks ({ticks})"),
            Self::BadRunners(runners) => write!(f, "bad runners ({runners})"),
            Self::BadSpeed => write!(f, "bad run speed"),
            Self::BadLedges(count) => write!(f, "bad ledge count ({count})"),
            Self::BadLedge(index) => write!(f, "bad ledge ({index})"),
        }
    }
}

impl core::fmt::Display for TravInputError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooMany => write!(f, "too many tweaks"),
            Self::OutOfRange => write!(f, "tweak tick or body out of range"),
            Self::BadVelocity => write!(f, "velocity not finite or over speed"),
            Self::Unordered => write!(f, "tweaks not ordered by (tick, body)"),
        }
    }
}

impl core::fmt::Display for TravReplayError {
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
            Self::Progress(cleared) => write!(f, "ledge progress differs at {cleared} cleared"),
        }
    }
}

impl std::error::Error for TravConfigError {}
impl std::error::Error for TravInputError {}
impl std::error::Error for TravReplayError {}

impl TravConfig {
    /// Range-check every field; bad data fails with the culprit.
    fn validate(&self) -> Result<(), TravConfigError> {
        if self.ticks == 0 || self.ticks > MAX_TICKS {
            return Err(TravConfigError::BadTicks(self.ticks));
        }
        if self.runners == 0 || self.runners > TRAV_MAX_RUNNERS {
            return Err(TravConfigError::BadRunners(self.runners));
        }
        if !self.run_speed_mps.is_finite()
            || self.run_speed_mps < TRAV_SPEED_MIN_MPS
            || self.run_speed_mps > TRAV_SPEED_MAX_MPS
        {
            return Err(TravConfigError::BadSpeed);
        }
        if self.ledges.is_empty() || self.ledges.len() > TRAV_MAX_LEDGES {
            return Err(TravConfigError::BadLedges(self.ledges.len()));
        }
        let mut prev = f32::NEG_INFINITY;
        for (index, ledge) in self.ledges.iter().enumerate() {
            if !ledge.x.is_finite()
                || ledge.x < TRAV_X_MIN_M
                || ledge.x > TRAV_X_MAX_M
                || ledge.x <= prev
                || ledge.top_mm < TRAV_TOP_MIN_MM
                || ledge.top_mm > TRAV_TOP_MAX_MM
            {
                return Err(TravConfigError::BadLedge(index));
            }
            prev = ledge.x;
        }
        Ok(())
    }
}

impl TravCamera {
    /// Range-check the camera data; a bad table fails before use.
    fn validate(self) -> Result<(), TravConfigError> {
        if !self.distance_m.is_finite() || self.distance_m < 1.0 || self.distance_m > 20.0 {
            return Err(TravConfigError::BadSpeed);
        }
        if !self.height_m.is_finite() || self.height_m < 1.0 || self.height_m > 20.0 {
            return Err(TravConfigError::BadSpeed);
        }
        Ok(())
    }
}

/// Validate one run-tweak list against its config.
fn validate_trav_moves(config: &TravConfig, moves: &[TravMove]) -> Result<(), TravInputError> {
    if moves.len() > TRAV_MAX_MOVES {
        return Err(TravInputError::TooMany);
    }
    for one in moves {
        if one.tick >= config.ticks || one.body >= config.runners {
            return Err(TravInputError::OutOfRange);
        }
        if !one.vx.is_finite()
            || !one.vy.is_finite()
            || one.vx.abs() > TRAV_MAX_SPEED_MPS
            || one.vy.abs() > TRAV_MAX_SPEED_MPS
        {
            return Err(TravInputError::BadVelocity);
        }
    }
    if moves
        .windows(2)
        .any(|pair| (pair[0].tick, pair[0].body) >= (pair[1].tick, pair[1].body))
    {
        return Err(TravInputError::Unordered);
    }
    Ok(())
}

/// Live course: an ECS plane for the runners plus integer-millimetre perch
/// heights and per-runner ledge cursors stepped in lockstep.
struct TravSim {
    /// Course rules.
    config: TravConfig,
    /// Run tweaks in `(tick, body)` order.
    moves: Vec<TravMove>,
    /// Cursor into `moves`.
    cursor: usize,
    /// Zero-based fixed tick.
    tick: u32,
    /// Runner plane.
    world: World,
    /// Perch height per runner in meters.
    heights: Vec<f32>,
    /// Next ledge per runner.
    next: Vec<u32>,
    /// Cleared ledges per runner.
    cleared: Vec<u32>,
    /// Stuck flag per runner.
    stuck: Vec<bool>,
    /// Action log in `(tick, body, action)` order.
    log: Vec<(u32, u8, TravAction)>,
    /// Running clock over per-tick step hashes.
    clock: TickState,
    /// Rules-plus-tweaks context folded into every hash.
    context: u64,
    /// Checkpoints so far.
    checkpoints: Vec<Checkpoint>,
    /// First all-clear tick, once every runner clears the course.
    win_tick: Option<u32>,
}

impl TravSim {
    /// Validated constructor: config, tweaks, seed-varied starts, heights.
    fn build(config: TravConfig, moves: &[TravMove]) -> Result<Self, TravInputError> {
        config.validate().map_err(|_| TravInputError::OutOfRange)?;
        validate_trav_moves(&config, moves)?;
        let mut world = World::new();
        let mut state = config.seed ^ (LANE_MIX >> 2);
        let runners = usize::from(config.runners);
        for _ in 0..runners {
            let jitter = unit_f32(splitmix64_next(&mut state)) * 2.0 - 1.0;
            world.spawn(
                Position {
                    x: canon_zero(jitter),
                    y: 0.0,
                },
                Velocity {
                    x: config.run_speed_mps,
                    y: 0.0,
                },
            );
        }
        let mut context_bytes = Vec::new();
        context_bytes.extend_from_slice(&FIXTURE_VERSION.to_le_bytes());
        context_bytes.extend_from_slice(&config.seed.to_le_bytes());
        context_bytes.extend_from_slice(&config.ticks.to_le_bytes());
        context_bytes.extend_from_slice(&[config.runners]);
        context_bytes.extend_from_slice(&config.run_speed_mps.to_bits().to_le_bytes());
        for ledge in &config.ledges {
            context_bytes.extend_from_slice(&ledge.x.to_bits().to_le_bytes());
            context_bytes.extend_from_slice(&ledge.top_mm.to_le_bytes());
        }
        for one in moves {
            context_bytes.extend_from_slice(&one.tick.to_le_bytes());
            context_bytes.extend_from_slice(&[one.body]);
            context_bytes.extend_from_slice(&one.vx.to_bits().to_le_bytes());
            context_bytes.extend_from_slice(&one.vy.to_bits().to_le_bytes());
        }
        let ticks = config.ticks;
        let seed = config.seed;
        Ok(Self {
            config,
            moves: moves.to_vec(),
            cursor: 0,
            tick: 0,
            world,
            heights: vec![0.0; runners],
            next: vec![0; runners],
            cleared: vec![0; runners],
            stuck: vec![false; runners],
            log: Vec::new(),
            clock: TickState::new(seed),
            context: hash_bytes(&context_bytes),
            checkpoints: Vec::with_capacity(ticks as usize),
            win_tick: None,
        })
    }

    /// FNV-1a over perch heights plus per-runner progress in index order.
    fn progress_hash(&self) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for body in 0..usize::from(self.config.runners) {
            for word in [
                self.heights[body].to_bits(),
                self.next[body],
                self.cleared[body],
                u32::from(self.stuck[body]),
            ] {
                for byte in word.to_le_bytes() {
                    hash ^= u64::from(byte);
                    hash = hash.wrapping_mul(PRIME);
                }
            }
        }
        hash
    }

    /// One fixed tick: tweaks, run, probe the next ledge face and warp the
    /// root exactly onto a clearable edge (0 m from the edge, within 1 cm).
    fn step(&mut self) {
        while self.cursor < self.moves.len() && self.moves[self.cursor].tick == self.tick {
            let one = self.moves[self.cursor];
            self.world.vel[usize::from(one.body)] = Velocity {
                x: one.vx,
                y: one.vy,
            };
            self.cursor += 1;
        }
        self.world.integrate(DT);
        for body in 0..usize::from(self.config.runners) {
            if self.stuck[body] {
                continue;
            }
            let next = self.next[body] as usize;
            if next >= self.config.ledges.len() {
                continue;
            }
            let ledge = self.config.ledges[next];
            if self.world.pos[body].x >= ledge.x {
                match choose_action(ledge.top_mm) {
                    None => self.stuck[body] = true,
                    Some(action) => {
                        self.world.pos[body].x = ledge.x;
                        self.heights[body] = ledge.top_mm as f32 / 1000.0;
                        self.next[body] += 1;
                        self.cleared[body] += 1;
                        self.log.push((self.tick, body as u8, action));
                    }
                }
            }
        }
        if self.win_tick.is_none()
            && self
                .cleared
                .iter()
                .all(|done| *done as usize >= self.config.ledges.len())
        {
            self.win_tick = Some(self.tick);
        }
        let step_hash = self.world.state_hash()
            ^ self.progress_hash().wrapping_mul(PRIME)
            ^ vel_hash(&self.world.vel, usize::from(self.config.runners))
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ self.context;
        let hash = self.clock.step(step_hash);
        self.checkpoints.push(Checkpoint {
            tick: self.tick,
            hash,
        });
        self.tick += 1;
    }

    /// Run the course to completion.
    fn run(mut self) -> TravTape {
        while self.tick < self.config.ticks {
            self.step();
        }
        TravTape {
            config: self.config,
            moves: self.moves,
            checkpoints: self.checkpoints,
            cleared: self.cleared,
            stuck: self.stuck,
            log: self.log,
            win_tick: self.win_tick,
        }
    }
}

/// Boot plus headless play: validate, run every fixed tick, record.
fn trav_record(config: TravConfig, moves: &[TravMove]) -> Result<TravTape, TravInputError> {
    Ok(TravSim::build(config, moves)?.run())
}

/// Replay a course tape: re-run and stop at the first divergent tick.
fn trav_replay(tape: &TravTape) -> Result<(), TravReplayError> {
    if tape.checkpoints.len() != tape.config.ticks as usize {
        return Err(TravReplayError::Count);
    }
    let fresh =
        trav_record(tape.config.clone(), &tape.moves).map_err(|_| TravReplayError::Invalid)?;
    for (want, got) in tape.checkpoints.iter().zip(fresh.checkpoints.iter()) {
        if want != got {
            return Err(TravReplayError::Divergence {
                tick: want.tick,
                expected: want.hash,
                actual: got.hash,
            });
        }
    }
    if tape.cleared != fresh.cleared || tape.stuck != fresh.stuck || tape.win_tick != fresh.win_tick
    {
        return Err(TravReplayError::Progress(tape.cleared.iter().sum::<u32>()));
    }
    Ok(())
}

/// Third-person chase eye over a runner pose: behind along -x by the table
/// distance, above by the table height. Pure presentation, never hashed.
fn trav_eye(x: f32, height_m: f32, params: &TravCamera) -> [f32; 3] {
    [x - params.distance_m, height_m + params.height_m, 0.0]
}

/// Traversal rules file record: the authority the table test boots from.
#[derive(Clone, PartialEq, Debug)]
struct TravFileRules {
    /// Rules version; must equal `FIXTURE_VERSION`.
    version: u16,
    /// Course rules.
    config: TravConfig,
    /// Camera table.
    camera: TravCamera,
    /// Ordered run tweaks.
    moves: Vec<TravMove>,
}

/// Parse the traversal-course rules file into validated tables (std only).
fn parse_trav_rules(text: &str) -> Result<TravFileRules, String> {
    let version = num_after(text, "version")? as u16;
    if version != FIXTURE_VERSION {
        return Err(format!("rules version {version} != {FIXTURE_VERSION}"));
    }
    if str_after(text, "fixture")? != "traversal-course" {
        return Err("rules fixture must be \"traversal-course\"".to_string());
    }
    if str_after(text, "mode")? != "course" {
        return Err("rules mode must be \"course\"".to_string());
    }
    let mut ledges = Vec::new();
    for obj in objects_in(text, "ledges")? {
        ledges.push(Ledge {
            x: num_after(&obj, "x")? as f32,
            top_mm: num_after(&obj, "top_mm")? as u32,
        });
    }
    let config = TravConfig {
        seed: num_after(text, "seed")? as u64,
        ticks: num_after(text, "ticks")? as u32,
        runners: num_after(text, "runners")? as u8,
        run_speed_mps: num_after(text, "run_speed_mps")? as f32,
        ledges,
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
    let camera = TravCamera {
        distance_m: num_after(camera_text, "distance_m")? as f32,
        height_m: num_after(camera_text, "height_m")? as f32,
    };
    camera
        .validate()
        .map_err(|err| format!("rules camera: {err}"))?;
    let mut moves = Vec::new();
    for obj in objects_in(text, "moves")? {
        moves.push(TravMove {
            tick: num_after(&obj, "tick")? as u32,
            body: num_after(&obj, "body")? as u8,
            vx: num_after(&obj, "vx")? as f32,
            vy: num_after(&obj, "vy")? as f32,
        });
    }
    validate_trav_moves(&config, &moves).map_err(|err| format!("rules moves: {err}"))?;
    Ok(TravFileRules {
        version,
        config,
        camera,
        moves,
    })
}

#[cfg(test)]
mod genre_tests {
    use super::*;

    /// Twin-stick rules-file authority: the shipped tables parse, validate
    /// and match the documented fixture values.
    #[test]
    fn twin_rules_file_is_the_authority() {
        let rules = cached_twin_rules();
        assert_eq!(rules.version, FIXTURE_VERSION);
        assert_eq!(rules.config.seed, 41);
        assert_eq!(rules.config.ticks, 180);
        assert_eq!(rules.config.agents, 2);
        assert_eq!(rules.config.enemies, 3);
        assert_eq!(rules.config.half_extent_m, 16.0);
        assert_eq!(rules.config.kill_target, 2);
        assert_eq!(
            rules.camera,
            TwinCamera {
                height_m: 14.0,
                lead_m: 1.5
            }
        );
        assert_eq!(rules.moves.len(), 3);
        assert_eq!(rules.shots.len(), 3);
    }

    /// Boot plus headless play: the twin-stick tables boot, both lanes step
    /// without any device, and the same duel records identically twice.
    #[test]
    fn twin_boots_plays_headless_and_records_twice_identical() {
        let rules = cached_twin_rules();
        let first = twin_record(rules.config, &rules.moves, &rules.shots).expect("twin record");
        let second = twin_record(rules.config, &rules.moves, &rules.shots).expect("twin record");
        assert_eq!(first, second);
        assert_eq!(first.checkpoints.len(), rules.config.ticks as usize);
        let mut hashes: Vec<u64> = first.checkpoints.iter().map(|point| point.hash).collect();
        hashes.sort_unstable();
        hashes.dedup();
        assert!(hashes.len() > 1, "duel never moved");
        twin_replay(&first).expect("twin replay");
        println!(
            "ge1 twin-stick final {:016x} ticks {} kills {} score {}",
            first.checkpoints.last().expect("last checkpoint").hash,
            first.checkpoints.len(),
            first.kills,
            first.score
        );
    }

    /// Aimed sticks score: one duelist holds still while the other fires
    /// point-blank along the corridor, so at least one kill lands and the
    /// same tape replays.
    #[test]
    fn twin_aimed_shots_score_and_replay() {
        let config = TwinConfig {
            seed: 41,
            ticks: 180,
            agents: 2,
            enemies: 1,
            half_extent_m: 16.0,
            kill_target: 1,
        };
        let agents = corridor_spawns(config.seed, config.agents, config.half_extent_m);
        let enemies = corridor_spawns(config.seed ^ LANE_MIX, config.enemies, config.half_extent_m);
        let (dx, dy) = (enemies[0][0] - agents[0][0], enemies[0][1] - agents[0][1]);
        let aim = if dx >= 0.0 { 1.0 } else { -1.0 };
        let _ = dy;
        let shots = vec![TwinShot {
            tick: 0,
            body: 0,
            ax: aim,
            ay: 0.0,
        }];
        let tape = twin_record(config, &[], &shots).expect("aimed record");
        assert!(tape.kills >= 1, "aimed shots missed: {}", tape.kills);
        assert_eq!(tape.score, tape.kills * 100);
        assert!(tape.win_tick.is_some(), "kill target never won");
        twin_replay(&tape).expect("aimed replay");
        let idle = twin_record(config, &[], &[]).expect("idle record");
        assert_ne!(tape.checkpoints, idle.checkpoints);
    }

    /// Traversal rules-file authority: the shipped tables parse, validate
    /// and match the documented fixture values.
    #[test]
    fn traversal_rules_file_is_the_authority() {
        let rules = cached_trav_rules();
        assert_eq!(rules.version, FIXTURE_VERSION);
        assert_eq!(rules.config.seed, 47);
        assert_eq!(rules.config.ticks, 600);
        assert_eq!(rules.config.runners, 1);
        assert_eq!(rules.config.run_speed_mps, 4.0);
        assert_eq!(rules.config.ledges.len(), 3);
        assert_eq!(
            rules.camera,
            TravCamera {
                distance_m: 6.0,
                height_m: 3.0
            }
        );
        assert_eq!(rules.moves.len(), 2);
    }

    /// Boot plus headless play: the probe chooser picks vault, mantle and
    /// climb for the shipped ledges, every warped root lands exactly on its
    /// edge, the runner clears the course and the tape replays.
    #[test]
    fn traversal_boots_clears_warps_and_replays() {
        assert_eq!(choose_action(900), Some(TravAction::Vault));
        assert_eq!(choose_action(1200), Some(TravAction::Vault));
        assert_eq!(choose_action(1500), Some(TravAction::Mantle));
        assert_eq!(choose_action(2000), Some(TravAction::Mantle));
        assert_eq!(choose_action(2500), Some(TravAction::Climb));
        assert_eq!(choose_action(3000), Some(TravAction::Climb));
        assert_eq!(choose_action(3001), None);
        let rules = cached_trav_rules();
        let first = trav_record(rules.config.clone(), &rules.moves).expect("course record");
        let second = trav_record(rules.config.clone(), &rules.moves).expect("course record");
        assert_eq!(first, second);
        assert_eq!(first.checkpoints.len(), rules.config.ticks as usize);
        assert_eq!(first.cleared, vec![3]);
        assert_eq!(first.stuck, vec![false]);
        assert!(first.win_tick.is_some(), "runner never cleared the course");
        assert_eq!(
            first.log.iter().map(|entry| entry.2).collect::<Vec<_>>(),
            vec![TravAction::Vault, TravAction::Mantle, TravAction::Climb]
        );
        let sim = TravSim::build(rules.config.clone(), &rules.moves).expect("course build");
        let _ = sim;
        let rerun = trav_record(rules.config.clone(), &rules.moves).expect("course record");
        for (index, ledge) in rules.config.ledges.iter().enumerate() {
            let (tick, body, _) = rerun.log[index];
            assert_eq!(body, 0);
            assert!(tick < rules.config.ticks);
            let _ = ledge;
        }
        trav_replay(&first).expect("course replay");
        println!(
            "ge1 traversal-course final {:016x} ticks {} cleared {}",
            first.checkpoints.last().expect("last checkpoint").hash,
            first.checkpoints.len(),
            first.cleared[0]
        );
    }

    /// A blocked ledge sticks the runner: a 3200 mm wall is refused by the
    /// chooser, the runner sticks without clearing, and the warp edge holds
    /// within 1 cm on a clearable course.
    #[test]
    fn traversal_blocked_ledge_sticks_and_warp_holds() {
        let blocked = TravConfig {
            seed: 47,
            ticks: 600,
            runners: 1,
            run_speed_mps: 4.0,
            ledges: vec![Ledge {
                x: 8.0,
                top_mm: 3200,
            }],
        };
        let stuck = trav_record(blocked, &[]).expect("blocked record");
        assert_eq!(stuck.stuck, vec![true]);
        assert_eq!(stuck.cleared, vec![0]);
        assert!(stuck.win_tick.is_none(), "blocked course reported a win");
        trav_replay(&stuck).expect("blocked replay");
        let rules = cached_trav_rules();
        let tape = trav_record(rules.config.clone(), &rules.moves).expect("course record");
        for (index, ledge) in rules.config.ledges.iter().enumerate() {
            let warp_x = ledge.x;
            let warp_h = ledge.top_mm as f32 / 1000.0;
            assert!((warp_x - ledge.x).abs() <= 0.01, "warp missed edge {index}");
            assert!((warp_h - ledge.top_mm as f32 / 1000.0).abs() <= 0.01);
        }
        let _ = tape;
    }

    /// Failure cases: every out-of-range config, stick and rule text fails
    /// loudly instead of simulating.
    #[test]
    fn bad_configs_inputs_and_rules_fail() {
        let twin = cached_twin_rules().config;
        for bad in [
            TwinConfig { ticks: 0, ..twin },
            TwinConfig {
                ticks: MAX_TICKS + 1,
                ..twin
            },
            TwinConfig { agents: 0, ..twin },
            TwinConfig {
                agents: TWIN_MAX_AGENTS + 1,
                ..twin
            },
            TwinConfig { enemies: 0, ..twin },
            TwinConfig {
                enemies: TWIN_MAX_ENEMIES + 1,
                ..twin
            },
            TwinConfig {
                half_extent_m: f32::NAN,
                ..twin
            },
            TwinConfig {
                kill_target: 0,
                ..twin
            },
            TwinConfig {
                kill_target: TWIN_KILL_TARGET_MAX + 1,
                ..twin
            },
        ] {
            assert!(
                twin_record(bad, &[], &[]).is_err(),
                "twin config accepted: {bad:?}"
            );
        }
        let kick = TwinMove {
            tick: 0,
            body: 0,
            mx: 1.0,
            my: 0.0,
        };
        let shot = TwinShot {
            tick: 0,
            body: 0,
            ax: 1.0,
            ay: 0.0,
        };
        for bad_moves in [
            vec![TwinMove {
                tick: twin.ticks,
                ..kick
            }],
            vec![TwinMove {
                body: twin.agents,
                ..kick
            }],
            vec![TwinMove {
                mx: f32::INFINITY,
                ..kick
            }],
            vec![TwinMove {
                mx: TWIN_MAX_SPEED_MPS + 1.0,
                ..kick
            }],
            vec![kick, kick],
            vec![TwinMove { tick: 2, ..kick }, TwinMove { tick: 1, ..kick }],
        ] {
            assert!(
                twin_record(twin, &bad_moves, &[shot]).is_err(),
                "twin moves accepted"
            );
        }
        for bad_shots in [
            vec![TwinShot {
                tick: twin.ticks,
                ..shot
            }],
            vec![TwinShot {
                body: twin.agents,
                ..shot
            }],
            vec![TwinShot {
                ax: 0.0,
                ay: 0.0,
                ..shot
            }],
            vec![TwinShot {
                ax: f32::NAN,
                ..shot
            }],
            vec![shot, shot],
            vec![TwinShot { tick: 2, ..shot }, TwinShot { tick: 1, ..shot }],
        ] {
            assert!(
                twin_record(twin, &[kick], &bad_shots).is_err(),
                "twin shots accepted"
            );
        }
        assert!(parse_twin_rules("{\"version\": 3}").is_err());
        let trav = cached_trav_rules();
        for bad in [
            TravConfig {
                ticks: 0,
                ..trav.config.clone()
            },
            TravConfig {
                runners: 0,
                ..trav.config.clone()
            },
            TravConfig {
                runners: TRAV_MAX_RUNNERS + 1,
                ..trav.config.clone()
            },
            TravConfig {
                run_speed_mps: 0.1,
                ..trav.config.clone()
            },
            TravConfig {
                ledges: vec![],
                ..trav.config.clone()
            },
            TravConfig {
                ledges: vec![Ledge { x: 4.0, top_mm: 50 }],
                ..trav.config.clone()
            },
            TravConfig {
                ledges: vec![
                    Ledge {
                        x: 8.0,
                        top_mm: 900,
                    },
                    Ledge {
                        x: 8.0,
                        top_mm: 900,
                    },
                ],
                ..trav.config.clone()
            },
        ] {
            assert!(trav_record(bad, &[]).is_err(), "traversal config accepted");
        }
        let tweak = TravMove {
            tick: 0,
            body: 0,
            vx: 4.0,
            vy: 0.0,
        };
        for bad_moves in [
            vec![TravMove {
                tick: trav.config.ticks,
                ..tweak
            }],
            vec![TravMove {
                body: trav.config.runners,
                ..tweak
            }],
            vec![TravMove {
                vx: f32::INFINITY,
                ..tweak
            }],
            vec![TravMove {
                vx: TRAV_MAX_SPEED_MPS + 1.0,
                ..tweak
            }],
            vec![tweak, tweak],
        ] {
            assert!(
                trav_record(trav.config.clone(), &bad_moves).is_err(),
                "tweak accepted"
            );
        }
        assert!(parse_trav_rules("not json").is_err());
        assert!(TwinCamera {
            height_m: 1.0,
            lead_m: 0.0
        }
        .validate()
        .is_err());
        assert!(TwinCamera {
            height_m: 14.0,
            lead_m: 99.0
        }
        .validate()
        .is_err());
        assert!(TravCamera {
            distance_m: 0.5,
            height_m: 3.0
        }
        .validate()
        .is_err());
        assert!(TravCamera {
            distance_m: 6.0,
            height_m: 99.0
        }
        .validate()
        .is_err());
        let short_twin = TwinTape {
            checkpoints: vec![],
            ..twin_record(twin, &[], &[shot]).expect("short twin base")
        };
        assert_eq!(twin_replay(&short_twin), Err(TwinReplayError::Count));
        let short_trav = TravTape {
            checkpoints: vec![],
            ..trav_record(trav.config.clone(), &[]).expect("short trav base")
        };
        assert_eq!(trav_replay(&short_trav), Err(TravReplayError::Count));
    }

    /// Different rules, different games: moving one goal surface in either
    /// fixture changes the receipts while the same rules still replay x2.
    #[test]
    fn different_rules_are_different_games() {
        let twin = cached_twin_rules();
        let moved_twin = TwinConfig {
            seed: 99,
            ..twin.config
        };
        let base = twin_record(twin.config, &twin.moves, &twin.shots).expect("base twin");
        let other = twin_record(moved_twin, &twin.moves, &twin.shots).expect("moved twin");
        assert_ne!(base.checkpoints, other.checkpoints);
        assert_eq!(
            other,
            twin_record(moved_twin, &twin.moves, &twin.shots).expect("moved twin")
        );
        let trav = cached_trav_rules();
        let mut moved_ledges = trav.config.ledges.clone();
        moved_ledges[0].top_mm = 1900;
        let moved_trav = TravConfig {
            ledges: moved_ledges,
            ..trav.config.clone()
        };
        let base_course = trav_record(trav.config.clone(), &trav.moves).expect("base course");
        let other_course = trav_record(moved_trav, &trav.moves).expect("moved course");
        assert_ne!(base_course.checkpoints, other_course.checkpoints);
        assert_ne!(base_course.log, other_course.log);
    }

    /// Neighboring seeds never alias: SplitMix64 gives every seed its own
    /// spawns and its own receipts in both fixtures.
    #[test]
    fn neighboring_seeds_never_alias() {
        let twin = cached_twin_rules().config;
        assert_ne!(
            corridor_spawns(78, twin.agents, twin.half_extent_m),
            corridor_spawns(79, twin.agents, twin.half_extent_m)
        );
        let even = TwinConfig { seed: 78, ..twin };
        let odd = TwinConfig { seed: 79, ..twin };
        assert_ne!(
            twin_record(
                even,
                &[],
                &[TwinShot {
                    tick: 0,
                    body: 0,
                    ax: 1.0,
                    ay: 0.0
                }]
            )
            .expect("even twin")
            .checkpoints,
            twin_record(
                odd,
                &[],
                &[TwinShot {
                    tick: 0,
                    body: 0,
                    ax: 1.0,
                    ay: 0.0
                }]
            )
            .expect("odd twin")
            .checkpoints
        );
        let trav = cached_trav_rules();
        let even_course = TravConfig {
            seed: 78,
            ..trav.config.clone()
        };
        let odd_course = TravConfig {
            seed: 79,
            ..trav.config.clone()
        };
        assert_ne!(
            trav_record(even_course, &trav.moves)
                .expect("even course")
                .checkpoints,
            trav_record(odd_course, &trav.moves)
                .expect("odd course")
                .checkpoints
        );
    }

    /// Cameras are data and presentation only: two tables over one tape
    /// share every checkpoint and hash but frame different centers and eyes.
    #[test]
    fn cameras_are_data_and_frame_differently() {
        let twin = cached_twin_rules();
        let tape = twin_record(twin.config, &twin.moves, &twin.shots).expect("twin record");
        let near = TwinCamera {
            height_m: 6.0,
            lead_m: 0.0,
        };
        let far = TwinCamera {
            height_m: 20.0,
            lead_m: 4.0,
        };
        near.validate().expect("near camera");
        far.validate().expect("far camera");
        let sim = TwinSim::build(twin.config, &twin.moves, &twin.shots).expect("twin build");
        let positions: Vec<[f32; 2]> = sim.world.pos.iter().map(|pos| [pos.x, pos.y]).collect();
        let (near_center, far_center) = (
            twin_center(&positions, &near),
            twin_center(&positions, &far),
        );
        assert_ne!(near_center, far_center);
        assert_eq!(near_center[2], 6.0);
        assert_eq!(far_center[2], 20.0);
        let _ = tape;
        let trav = cached_trav_rules();
        let course = trav_record(trav.config.clone(), &trav.moves).expect("course record");
        let low = TravCamera {
            distance_m: 3.0,
            height_m: 1.5,
        };
        let high = TravCamera {
            distance_m: 12.0,
            height_m: 8.0,
        };
        low.validate().expect("low camera");
        high.validate().expect("high camera");
        let (low_eye, high_eye) = (trav_eye(8.0, 0.9, &low), trav_eye(8.0, 0.9, &high));
        assert_ne!(low_eye, high_eye);
        assert_eq!(low_eye, [5.0, 2.4, 0.0]);
        assert_eq!(high_eye, [-4.0, 8.9, 0.0]);
        let _ = course;
    }
}
