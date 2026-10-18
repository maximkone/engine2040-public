use super::*;
use engine_core::splitmix64;

// ---------------------------------------------------------------------------
// Fixture rule sets: the mode framework boots fixture rule
// sets through this loader with no engine edits. Rules arrive as data text
// in two schemas: the tag schema (`fixtures/games/second-game/rules.json`:
// tag mode, seed, ticks, runners, goal disc, follow-camera table, ordered
// input script) and the traversal-course schema
// (`fixtures/games/traversal-course/rules.json`: course mode, run speed,
// ledges, chase-camera table, run tweaks). Both validate at the boundary
// with typed errors, run headless in [`FixtureSession`] and replay with
// first-divergence naming. The camera tables are presentation only: two
// tables over one tape share every sim hash but frame differently.
// ---------------------------------------------------------------------------

/// Fixture rule-set version this loader boots (the tag-schema authority).
pub const FIXTURE_RULES_VERSION: u16 = 2;
/// Traversal-course rule-set version this loader boots (the GE1 authority).
pub const COURSE_RULES_VERSION: u16 = 3;
/// Runners per course, `1..=COURSE_MAX_RUNNERS` (GE1 mirror).
pub const COURSE_MAX_RUNNERS: u8 = 2;
/// Ledges per course, `1..=COURSE_MAX_LEDGES` (GE1 mirror).
pub const COURSE_MAX_LEDGES: usize = 16;
/// Course cruise-speed bounds, m/s (GE1 mirror).
pub const COURSE_SPEED_MIN_MPS: f32 = 0.5;
/// Course cruise-speed bounds, m/s (GE1 mirror).
pub const COURSE_SPEED_MAX_MPS: f32 = 10.0;
/// Course ledge x bounds, meters (GE1 mirror).
pub const COURSE_X_MIN_M: f32 = -50.0;
/// Course ledge x bounds, meters (GE1 mirror).
pub const COURSE_X_MAX_M: f32 = 120.0;
/// Course ledge top bounds, millimetres (GE1 mirror).
pub const COURSE_TOP_MIN_MM: f32 = 100.0;
/// Course ledge top bounds, millimetres (GE1 mirror).
pub const COURSE_TOP_MAX_MM: f32 = 3500.0;
/// Course chase-camera distance/height bounds, meters (GE1 mirror).
pub const COURSE_CAM_MIN_M: f32 = 1.0;
/// Course chase-camera distance/height bounds, meters (GE1 mirror).
pub const COURSE_CAM_MAX_M: f32 = 20.0;
/// Course spawn box half-extent, meters: the course schema carries no
/// playfield (GE1 owns bounds), so course runners seed-expand into this
/// fixed box instead of an extent-derived one.
pub const COURSE_SPAWN_LIMIT_M: f32 = 10.0;
/// Runners per fixture match, `2..=FIXTURE_MAX_RUNNERS`.
pub const FIXTURE_MAX_RUNNERS: u8 = 8;
/// Bound fixture ticks per match.
pub const FIXTURE_MAX_TICKS: u32 = 10_000;
/// Bound scripted moves per fixture match.
pub const FIXTURE_MAX_INPUTS: usize = 512;
/// Fastest legal fixture velocity assignment, m/s.
pub const FIXTURE_MAX_SPEED_MPS: f32 = 20.0;
/// Fixture fixed step in seconds (60 Hz, the engine tick).
pub const FIXTURE_DT_S: f32 = 1.0 / 60.0;
/// Goal disc radius bounds, meters.
pub const FIXTURE_GOAL_RADIUS_MIN_M: f32 = 0.5;
/// Goal disc radius bounds, meters.
pub const FIXTURE_GOAL_RADIUS_MAX_M: f32 = 5.0;
/// Playfield half-extent bounds, meters.
pub const FIXTURE_EXTENT_MIN_M: f32 = 5.0;
/// Playfield half-extent bounds, meters.
pub const FIXTURE_EXTENT_MAX_M: f32 = 50.0;
/// Follow-camera height bounds, meters.
pub const FIXTURE_CAM_HEIGHT_MIN_M: f32 = 2.0;
/// Follow-camera height bounds, meters.
pub const FIXTURE_CAM_HEIGHT_MAX_M: f32 = 30.0;
/// Follow-camera lead bounds, meters.
pub const FIXTURE_CAM_LEAD_MAX_M: f32 = 10.0;

/// Fixture rule-set failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FixtureError {
    /// Rules version is not [`FIXTURE_RULES_VERSION`], version carried.
    BadVersion(u16),
    /// Mode string is not `"tag"` (the only rule set this loader boots).
    UnknownMode,
    /// Tick count `0` or above [`FIXTURE_MAX_TICKS`].
    BadTicks(u32),
    /// Runner count outside `2..=FIXTURE_MAX_RUNNERS`.
    BadRunners(u8),
    /// Goal disc outside the playfield or with a bad radius.
    BadGoal,
    /// Half-extent NaN or outside `5.0..=50.0` meters.
    BadExtent,
    /// Course cruise speed NaN or outside `0.5..=10.0` m/s.
    BadSpeed,
    /// Course ledge count outside `1..=COURSE_MAX_LEDGES`, count carried.
    BadLedges(usize),
    /// More than [`COURSE_MAX_LEDGES`] ledges while scanning.
    TooManyLedges,
    /// Course ledge out of bounds, unordered or with a bad top, index carried.
    BadLedge(usize),
    /// Follow-camera row outside its range, row name carried.
    BadCamera(&'static str),
    /// Director table refused while booting a mode, reason carried.
    BadCameraTable(&'static str),
    /// More than [`FIXTURE_MAX_INPUTS`] moves.
    TooManyInputs,
    /// Tick at or past the match end, or body past the runners.
    OutOfRange,
    /// Non-finite or over-speed velocity.
    BadVelocity,
    /// Moves not strictly ordered by `(tick, body)`.
    Unordered,
    /// Input script is empty (schema parity with the tag-schema authority).
    EmptyScript,
    /// Needed key absent from the rules text, key name carried.
    MissingKey(&'static str),
    /// Needed value would not parse, key name carried.
    BadValue(&'static str),
    /// Malformed rules structure, site carried (`camera`, `inputs`, `mode`).
    BadRules(&'static str),
    /// Replay checkpoint count does not match the config.
    Count,
    /// First mismatching zero-based tick plus both hashes.
    Divergence {
        /// Zero-based tick of the first mismatch.
        tick: u32,
        /// Hash the tape recorded.
        expected: u64,
        /// Hash the re-run produced.
        actual: u64,
    },
    /// Win outcome differs at the named tick.
    Win(u32),
}

impl core::fmt::Display for FixtureError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadVersion(version) => write!(f, "fixture rules version {version}"),
            Self::UnknownMode => write!(f, "fixture mode must be \"tag\""),
            Self::BadTicks(ticks) => write!(f, "bad fixture ticks ({ticks})"),
            Self::BadRunners(runners) => write!(f, "bad fixture runners ({runners})"),
            Self::BadGoal => write!(f, "bad fixture goal disc"),
            Self::BadExtent => write!(f, "bad fixture half extent"),
            Self::BadSpeed => write!(f, "bad course run speed"),
            Self::BadLedges(count) => write!(f, "bad course ledge count ({count})"),
            Self::TooManyLedges => write!(f, "too many course ledges"),
            Self::BadLedge(index) => write!(f, "bad course ledge ({index})"),
            Self::BadCamera(row) => write!(f, "bad fixture camera row ({row})"),
            Self::BadCameraTable(reason) => write!(f, "bad fixture camera table ({reason})"),
            Self::TooManyInputs => write!(f, "too many fixture inputs"),
            Self::OutOfRange => write!(f, "fixture input tick or body out of range"),
            Self::BadVelocity => write!(f, "fixture velocity not finite or over speed"),
            Self::Unordered => write!(f, "fixture inputs not ordered by (tick, body)"),
            Self::EmptyScript => write!(f, "fixture inputs array is empty"),
            Self::MissingKey(key) => write!(f, "fixture rules missing key ({key})"),
            Self::BadValue(key) => write!(f, "fixture rules bad value for ({key})"),
            Self::BadRules(site) => write!(f, "fixture rules malformed ({site})"),
            Self::Count => write!(f, "fixture checkpoint count does not match ticks"),
            Self::Divergence {
                tick,
                expected,
                actual,
            } => write!(
                f,
                "fixture replay diverged at tick {tick}: expected {expected:016x}, got {actual:016x}"
            ),
            Self::Win(tick) => write!(f, "fixture win outcome differs at tick {tick}"),
        }
    }
}

impl std::error::Error for FixtureError {}

/// Follow-camera parameters as data (presentation only, never hashed).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FixtureCamera {
    /// Camera height over the plane in meters, `2.0..=30.0`.
    pub height_m: f32,
    /// Lead toward the current leader in meters, `0.0..=10.0`.
    pub lead_m: f32,
}

impl FixtureCamera {
    /// Range-check the camera data; a bad table fails before use. Pure.
    pub fn validate(self) -> Result<(), FixtureError> {
        if !self.height_m.is_finite()
            || !(FIXTURE_CAM_HEIGHT_MIN_M..=FIXTURE_CAM_HEIGHT_MAX_M).contains(&self.height_m)
        {
            return Err(FixtureError::BadCamera("height_m"));
        }
        if !self.lead_m.is_finite() || !(0.0..=FIXTURE_CAM_LEAD_MAX_M).contains(&self.lead_m) {
            return Err(FixtureError::BadCamera("lead_m"));
        }
        Ok(())
    }

    /// Follow-center over planar runner positions: mean plus a lead toward
    /// the goal-nearest runner. Pure presentation; never hashed, never sim
    /// input. Uses one `sqrt` for the lead direction under the same
    /// presentation-only license as the arena director's trig; same inputs
    /// give the same center twice. Budget: O(runners) time, no heap.
    #[must_use]
    pub fn frame(&self, positions: &[[f32; 2]], goal: [f32; 2]) -> [f32; 3] {
        let count = positions.len().max(1) as f32;
        let (mut sx, mut sy) = (0.0f32, 0.0f32);
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
        if len2 <= 1e-12 || self.lead_m == 0.0 {
            [avg[0], avg[1], self.height_m]
        } else {
            let len = len2.sqrt();
            [
                avg[0] + self.lead_m * ox / len,
                avg[1] + self.lead_m * oy / len,
                self.height_m,
            ]
        }
    }
}

/// One fixture velocity assignment applied before the named zero-based tick.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FixtureInput {
    /// Application tick, strictly below the match ticks.
    pub tick: u32,
    /// Runner slot below the match runners.
    pub body: u8,
    /// Horizontal velocity in m/s, `|v| <= 20.0`.
    pub vx: f32,
    /// Vertical velocity in m/s, `|v| <= 20.0`.
    pub vy: f32,
}

/// Validated fixture match rules as data: everything a mode boot needs, no
/// code fork per rule set. Built only by [`FixtureRules::parse`], so every
/// instance is already range-checked.
#[derive(Clone, PartialEq, Debug)]
pub struct FixtureRules {
    pub(super) version: u16,
    pub(super) seed: u64,
    pub(super) ticks: u32,
    pub(super) runners: u8,
    pub(super) goal: [f32; 2],
    pub(super) goal_radius_m: f32,
    pub(super) half_extent_m: f32,
    pub(super) camera: FixtureCamera,
    pub(super) inputs: Vec<FixtureInput>,
}

/// Parse one JSON number after `"key":` (whitespace-tolerant, std only; the
/// headless default stays std-only, so no serde here).
pub(super) fn fixture_num_after(text: &str, key: &'static str) -> Result<f64, FixtureError> {
    // One small parse-time allocation at the load boundary (never per tick).
    let pattern = ["\"", key, "\""].concat();
    let start = text.find(&pattern).ok_or(FixtureError::MissingKey(key))?;
    let colon = text[start..].find(':').ok_or(FixtureError::BadValue(key))?;
    let rest = text[start + colon + 1..].trim_start();
    let mut end = 0usize;
    for (index, ch) in rest.char_indices() {
        if ch.is_ascii_digit() || matches!(ch, '-' | '+' | '.' | 'e' | 'E') {
            end = index + ch.len_utf8();
        } else {
            break;
        }
    }
    rest[..end]
        .parse::<f64>()
        .map_err(|_| FixtureError::BadValue(key))
}

/// Parse one JSON `[a, b]` pair after `"key":`.
pub(super) fn fixture_pair_after(text: &str, key: &'static str) -> Result<[f64; 2], FixtureError> {
    // One small parse-time allocation at the load boundary (never per tick).
    let pattern = ["\"", key, "\""].concat();
    let start = text.find(&pattern).ok_or(FixtureError::MissingKey(key))?;
    let open = text[start..].find('[').ok_or(FixtureError::BadRules(key))?;
    let rest = &text[start + open + 1..];
    let close = rest.find(']').ok_or(FixtureError::BadRules(key))?;
    let mut values = [0.0f64; 2];
    let mut count = 0usize;
    for part in rest[..close].split(',') {
        if count >= 2 {
            return Err(FixtureError::BadValue(key));
        }
        values[count] = part
            .trim()
            .parse::<f64>()
            .map_err(|_| FixtureError::BadValue(key))?;
        count += 1;
    }
    if count != 2 {
        return Err(FixtureError::BadValue(key));
    }
    Ok(values)
}

impl FixtureRules {
    /// Parse plus validate one tag-schema fixture rule-set text (the
    /// `fixtures/games/second-game/rules.json` schema). Unknown keys are
    /// ignored so newer tables still boot while the rows this loader needs
    /// are present; every needed row is range-checked and the input script
    /// must be non-empty and strictly ordered. Budget: O(inputs) time, one
    /// input `Vec` at the load boundary (never per tick).
    pub fn parse(text: &str) -> Result<Self, FixtureError> {
        let version = fixture_num_after(text, "version")? as u16;
        if version != FIXTURE_RULES_VERSION {
            return Err(FixtureError::BadVersion(version));
        }
        let mode_start = text
            .find("\"mode\"")
            .ok_or(FixtureError::MissingKey("mode"))?;
        let mode_colon = text[mode_start..]
            .find(':')
            .ok_or(FixtureError::BadRules("mode"))?;
        let mode_rest = text[mode_start + mode_colon + 1..].trim_start();
        if !mode_rest.starts_with("\"tag\"") {
            return Err(FixtureError::UnknownMode);
        }
        let seed = fixture_num_after(text, "seed")? as u64;
        let ticks = fixture_num_after(text, "ticks")? as u32;
        let runners = fixture_num_after(text, "runners")? as u8;
        let goal_pair = fixture_pair_after(text, "goal_m")?;
        let goal = [goal_pair[0] as f32, goal_pair[1] as f32];
        let goal_radius_m = fixture_num_after(text, "goal_radius_m")? as f32;
        let half_extent_m = fixture_num_after(text, "half_extent_m")? as f32;
        check_fixture_config(ticks, runners, goal, goal_radius_m, half_extent_m)?;
        let camera_block = text
            .find("\"camera\"")
            .and_then(|start| {
                text[start..]
                    .find('{')
                    .map(|open| &text[start + open + 1..])
            })
            .ok_or(FixtureError::MissingKey("camera"))?;
        let camera_end = camera_block
            .find('}')
            .ok_or(FixtureError::BadRules("camera"))?;
        let camera_text = &camera_block[..camera_end];
        let camera = FixtureCamera {
            height_m: fixture_num_after(camera_text, "height_m")? as f32,
            lead_m: fixture_num_after(camera_text, "lead_m")? as f32,
        };
        camera.validate()?;
        let inputs_start = text
            .find("\"inputs\"")
            .ok_or(FixtureError::MissingKey("inputs"))?;
        let inputs_open = text[inputs_start..]
            .find('[')
            .ok_or(FixtureError::BadRules("inputs"))?;
        let inputs_text = &text[inputs_start + inputs_open + 1..];
        let mut inputs = Vec::new();
        let mut cursor = 0usize;
        while let Some(open) = inputs_text[cursor..].find('{') {
            let obj_start = cursor + open;
            let obj_end = inputs_text[obj_start..]
                .find('}')
                .ok_or(FixtureError::BadRules("inputs"))?;
            let obj = &inputs_text[obj_start..obj_start + obj_end + 1];
            inputs.push(FixtureInput {
                tick: fixture_num_after(obj, "tick")? as u32,
                body: fixture_num_after(obj, "body")? as u8,
                vx: fixture_num_after(obj, "vx")? as f32,
                vy: fixture_num_after(obj, "vy")? as f32,
            });
            cursor = obj_start + obj_end + 1;
            if inputs.len() > FIXTURE_MAX_INPUTS {
                return Err(FixtureError::TooManyInputs);
            }
        }
        if inputs.is_empty() {
            return Err(FixtureError::EmptyScript);
        }
        check_fixture_inputs(ticks, runners, &inputs)?;
        Ok(Self {
            version,
            seed,
            ticks,
            runners,
            goal,
            goal_radius_m,
            half_extent_m,
            camera,
            inputs,
        })
    }
}

/// Range-check one fixture match config; NaN or out-of-range fails loudly.
pub(super) fn check_fixture_config(
    ticks: u32,
    runners: u8,
    goal: [f32; 2],
    goal_radius_m: f32,
    half_extent_m: f32,
) -> Result<(), FixtureError> {
    if ticks == 0 || ticks > FIXTURE_MAX_TICKS {
        return Err(FixtureError::BadTicks(ticks));
    }
    if !(2..=FIXTURE_MAX_RUNNERS).contains(&runners) {
        return Err(FixtureError::BadRunners(runners));
    }
    if !half_extent_m.is_finite()
        || !(FIXTURE_EXTENT_MIN_M..=FIXTURE_EXTENT_MAX_M).contains(&half_extent_m)
    {
        return Err(FixtureError::BadExtent);
    }
    if !goal_radius_m.is_finite()
        || !(FIXTURE_GOAL_RADIUS_MIN_M..=FIXTURE_GOAL_RADIUS_MAX_M).contains(&goal_radius_m)
        || !goal[0].is_finite()
        || !goal[1].is_finite()
        || goal[0].abs() > half_extent_m
        || goal[1].abs() > half_extent_m
    {
        return Err(FixtureError::BadGoal);
    }
    Ok(())
}

/// Validate one fixture move list: bounds, finite in-speed velocities and
/// strict `(tick, body)` order.
pub(super) fn check_fixture_inputs(
    ticks: u32,
    runners: u8,
    inputs: &[FixtureInput],
) -> Result<(), FixtureError> {
    if inputs.len() > FIXTURE_MAX_INPUTS {
        return Err(FixtureError::TooManyInputs);
    }
    for input in inputs {
        if input.tick >= ticks || input.body >= runners {
            return Err(FixtureError::OutOfRange);
        }
        if !input.vx.is_finite()
            || !input.vy.is_finite()
            || input.vx.abs() > FIXTURE_MAX_SPEED_MPS
            || input.vy.abs() > FIXTURE_MAX_SPEED_MPS
        {
            return Err(FixtureError::BadVelocity);
        }
    }
    if inputs
        .windows(2)
        .any(|pair| (pair[0].tick, pair[0].body) >= (pair[1].tick, pair[1].body))
    {
        return Err(FixtureError::Unordered);
    }
    Ok(())
}

/// One SplitMix64 step for non-aliasing fixture seed expansion behind the
/// core owner: the output equals `splitmix64` of the pre-step state, so
/// neighboring seeds draw disjoint streams and bytes don't move.
pub(super) fn fixture_splitmix_next(state: &mut u64) -> u64 {
    let word = splitmix64(*state);
    *state = state.wrapping_add(GOLDEN_GAMMA);
    word
}

/// Map one stream word uniformly into `[0, 1]` as `f32` (the tag-schema recipe;
/// same binary gives the same spawns twice).
pub(super) fn fixture_unit_f32(word: u64) -> f32 {
    (word as f64 / u64::MAX as f64) as f32
}

/// Normalize signed zero so `0.0` and `-0.0` hash identically.
pub(super) fn fixture_canon_zero(value: f32) -> f32 {
    if value == 0.0 {
        0.0
    } else {
        value
    }
}

/// Multiplier of the fixture hashes. Recorded fixture tapes pin hashes made
/// with it, so it stays distinct from the standard FNV-1a prime
/// (`engine_core::hash::FNV64_PRIME`); changing it re-records every tape.
pub(super) const FIXTURE_PRIME: u64 = 0x1000_0000_01b3;

/// Byte fold (xor, then multiply by [`FIXTURE_PRIME`]) into the running
/// fixture hash.
pub(super) fn fixture_hash_write(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FIXTURE_PRIME);
    }
}

/// Seed-expanded start positions: two stream words per runner mapped into
/// `[-limit, limit]`; yaw-free planar spawns in index order.
pub(super) fn fixture_spawns(seed: u64, runners: u8, limit: f32) -> [[f32; 2]; 8] {
    let mut state = seed;
    let mut out = [[0.0f32; 2]; 8];
    for slot in out.iter_mut().take(usize::from(runners)) {
        let x = fixture_unit_f32(fixture_splitmix_next(&mut state)) * 2.0 - 1.0;
        let y = fixture_unit_f32(fixture_splitmix_next(&mut state)) * 2.0 - 1.0;
        slot[0] = fixture_canon_zero(x * limit);
        slot[1] = fixture_canon_zero(y * limit);
    }
    out
}

/// Checkpoint after one fixture tick.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FixtureCheckpoint {
    /// Zero-based fixed tick.
    pub tick: u32,
    /// State hash after the tick.
    pub hash: u64,
}

/// Recorded fixture match: one checkpoint per tick plus the sticky tag win.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FixtureTape {
    /// One post-step checkpoint per tick, in tick order.
    pub checkpoints: Vec<FixtureCheckpoint>,
    /// First tick a runner entered the goal disc, if any.
    pub win_tick: Option<u32>,
    /// Lowest-index runner inside the disc on `win_tick`, if any.
    pub winner: Option<u8>,
}

impl FixtureTape {
    /// Digest over checkpoints plus the win outcome: equal tapes give equal
    /// digests twice; different rules give different digests. Pure.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut hash = FNV64_OFFSET;
        for point in &self.checkpoints {
            fixture_hash_write(&mut hash, &point.tick.to_le_bytes());
            fixture_hash_write(&mut hash, &point.hash.to_le_bytes());
        }
        let win_bits = match (self.win_tick, self.winner) {
            (Some(tick), Some(winner)) => {
                u64::from(tick).wrapping_mul(FIXTURE_PRIME) ^ u64::from(winner).wrapping_add(1)
            }
            _ => 0,
        };
        fixture_hash_write(&mut hash, &win_bits.to_le_bytes());
        hash
    }

    /// Replay a tape under its tag rules: re-simulate and stop at the first
    /// divergent tick, a count mismatch or a changed win. Pure booking over
    /// a fresh deterministic run.
    pub fn replay(&self, rules: &FixtureRules) -> Result<(), FixtureError> {
        if self.checkpoints.len() != rules.ticks as usize {
            return Err(FixtureError::Count);
        }
        let fresh = FixtureSession::boot(rules).run_to_tape();
        self.check_against(rules.ticks, &fresh)
    }

    /// Replay a tape under its course rules: same gates over a course boot
    /// (no goal disc, so the win arms only when both runs agree it is set).
    pub fn replay_course(&self, rules: &CourseRules) -> Result<(), FixtureError> {
        if self.checkpoints.len() != rules.ticks as usize {
            return Err(FixtureError::Count);
        }
        let fresh = FixtureSession::boot_course(rules).run_to_tape();
        self.check_against(rules.ticks, &fresh)
    }

    /// Shared replay tail: first-divergent-tick naming plus the win gate.
    fn check_against(&self, ticks: u32, fresh: &FixtureTape) -> Result<(), FixtureError> {
        for (want, got) in self.checkpoints.iter().zip(fresh.checkpoints.iter()) {
            if want != got {
                return Err(FixtureError::Divergence {
                    tick: want.tick,
                    expected: want.hash,
                    actual: got.hash,
                });
            }
        }
        if self.win_tick != fresh.win_tick || self.winner != fresh.winner {
            return Err(FixtureError::Win(self.win_tick.unwrap_or(ticks)));
        }
        Ok(())
    }
}

/// Live fixture match: planar runners with fixed-dt integration, a sticky
/// goal-disc win for tag rule sets (courses run open, no disc) and an FNV
/// hash chain over every future-affecting byte. Authoritative math is
/// `+ - * /` and comparisons only (no trig, no wall clock); positions stay
/// bounded (`|pos| <= extent + speed * ticks * dt`, far below overflow), so
/// every step is total. Built only from validated rules, hence infallible
/// construction.
pub struct FixtureSession {
    pub(super) ticks: u32,
    pub(super) runners: u8,
    pub(super) goal: [f32; 2],
    pub(super) goal_radius_m: f32,
    /// False for course rule sets: no goal disc, [`FixtureSession::tag_winner`]
    /// stays `None` (the goal fields below are inert then).
    pub(super) has_goal: bool,
    pub(super) inputs: Vec<FixtureInput>,
    pub(super) cursor: usize,
    pub(super) tick: u32,
    pub(super) pos: [[f32; 2]; 8],
    pub(super) vel: [[f32; 2]; 8],
    pub(super) clock: u64,
    pub(super) context: u64,
    pub(super) checkpoints: Vec<FixtureCheckpoint>,
    pub(super) win_tick: Option<u32>,
    pub(super) winner: Option<u8>,
}

impl FixtureSession {
    /// Validated constructor: spawns, hash context and sized checkpoint log.
    /// Budget: O(runners + inputs) time, two small `Vec`s at the boot
    /// boundary (never per tick).
    #[must_use]
    pub fn boot(rules: &FixtureRules) -> Self {
        let mut context = FNV64_OFFSET;
        fixture_hash_write(&mut context, &rules.version.to_le_bytes());
        fixture_hash_write(&mut context, &rules.seed.to_le_bytes());
        fixture_hash_write(&mut context, &rules.ticks.to_le_bytes());
        fixture_hash_write(&mut context, &[rules.runners]);
        for axis in rules.goal {
            fixture_hash_write(&mut context, &axis.to_bits().to_le_bytes());
        }
        fixture_hash_write(&mut context, &rules.goal_radius_m.to_bits().to_le_bytes());
        fixture_hash_write(&mut context, &rules.half_extent_m.to_bits().to_le_bytes());
        fixture_hash_write(&mut context, b"tag");
        for input in &rules.inputs {
            fixture_hash_write(&mut context, &input.tick.to_le_bytes());
            fixture_hash_write(&mut context, &[input.body]);
            fixture_hash_write(&mut context, &input.vx.to_bits().to_le_bytes());
            fixture_hash_write(&mut context, &input.vy.to_bits().to_le_bytes());
        }
        Self {
            ticks: rules.ticks,
            runners: rules.runners,
            goal: rules.goal,
            goal_radius_m: rules.goal_radius_m,
            has_goal: true,
            inputs: rules.inputs.clone(),
            cursor: 0,
            tick: 0,
            pos: fixture_spawns(rules.seed, rules.runners, rules.half_extent_m - 0.4),
            vel: [[0.0; 2]; 8],
            clock: mix64(rules.seed),
            context,
            checkpoints: Vec::with_capacity(rules.ticks as usize),
            win_tick: None,
            winner: None,
        }
    }

    /// Course constructor: same planar script run with no goal disc; cruise
    /// speed, ledges and the chase table fold into the context as data, so
    /// different furniture gives different receipts. Vault and ledge
    /// resolution belong to the GE1 sim, not this loader: the course run
    /// proves the rule set loads, the camera tracks and the tape replays.
    /// Budget: O(runners + moves) time, two small `Vec`s at the boot
    /// boundary (never per tick).
    #[must_use]
    pub fn boot_course(rules: &CourseRules) -> Self {
        let mut context = FNV64_OFFSET;
        fixture_hash_write(&mut context, &rules.version.to_le_bytes());
        fixture_hash_write(&mut context, &rules.seed.to_le_bytes());
        fixture_hash_write(&mut context, &rules.ticks.to_le_bytes());
        fixture_hash_write(&mut context, &[rules.runners]);
        fixture_hash_write(&mut context, &rules.run_speed_mps.to_bits().to_le_bytes());
        fixture_hash_write(
            &mut context,
            &rules.camera.distance_m.to_bits().to_le_bytes(),
        );
        fixture_hash_write(&mut context, &rules.camera.height_m.to_bits().to_le_bytes());
        fixture_hash_write(&mut context, b"course");
        fixture_hash_write(&mut context, &(rules.ledges.len() as u64).to_le_bytes());
        for ledge in &rules.ledges {
            fixture_hash_write(&mut context, &ledge.x_m.to_bits().to_le_bytes());
            fixture_hash_write(&mut context, &ledge.top_m.to_bits().to_le_bytes());
        }
        for input in &rules.moves {
            fixture_hash_write(&mut context, &input.tick.to_le_bytes());
            fixture_hash_write(&mut context, &[input.body]);
            fixture_hash_write(&mut context, &input.vx.to_bits().to_le_bytes());
            fixture_hash_write(&mut context, &input.vy.to_bits().to_le_bytes());
        }
        Self {
            ticks: rules.ticks,
            runners: rules.runners,
            goal: [0.0, 0.0],
            goal_radius_m: 0.0,
            has_goal: false,
            inputs: rules.moves.clone(),
            cursor: 0,
            tick: 0,
            pos: fixture_spawns(rules.seed, rules.runners, COURSE_SPAWN_LIMIT_M),
            vel: [[0.0; 2]; 8],
            clock: mix64(rules.seed),
            context,
            checkpoints: Vec::with_capacity(rules.ticks as usize),
            win_tick: None,
            winner: None,
        }
    }

    /// Zero-based fixed tick about to simulate.
    #[must_use]
    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// True once every configured tick has simulated.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.tick >= self.ticks
    }

    /// Planar runner positions for the presentation cameras.
    #[must_use]
    pub fn positions(&self) -> &[[f32; 2]] {
        &self.pos[..usize::from(self.runners)]
    }

    /// Goal disc center `[x, y]` in meters.
    #[must_use]
    pub fn goal(&self) -> [f32; 2] {
        self.goal
    }

    /// First runner inside the goal disc after this tick, lowest index wins
    /// ties by strict `<` over squared distance (no square root in sim).
    /// Course runs carry no disc and always answer `None`.
    fn tag_winner(&self) -> Option<u8> {
        if !self.has_goal {
            return None;
        }
        let radius2 = self.goal_radius_m * self.goal_radius_m;
        let mut best: Option<u8> = None;
        let mut best_d2 = f32::INFINITY;
        for (index, pos) in self.pos.iter().enumerate().take(usize::from(self.runners)) {
            let dx = pos[0] - self.goal[0];
            let dy = pos[1] - self.goal[1];
            let d2 = dx * dx + dy * dy;
            if d2 <= radius2 && d2 < best_d2 {
                best_d2 = d2;
                best = Some(index as u8);
            }
        }
        best
    }

    /// One fixed tick: apply this tick's moves, integrate both planes,
    /// record the sticky tag, fold every future-affecting byte into the
    /// checkpoint. No heap growth: fixed arrays plus a pre-sized log.
    pub(super) fn step(&mut self) {
        while self.cursor < self.inputs.len() && self.inputs[self.cursor].tick == self.tick {
            let input = self.inputs[self.cursor];
            let slot = usize::from(input.body);
            self.vel[slot] = [input.vx, input.vy];
            self.cursor += 1;
        }
        for index in 0..usize::from(self.runners) {
            self.pos[index][0] += self.vel[index][0] * FIXTURE_DT_S;
            self.pos[index][1] += self.vel[index][1] * FIXTURE_DT_S;
        }
        if self.win_tick.is_none() {
            if let Some(winner) = self.tag_winner() {
                self.win_tick = Some(self.tick);
                self.winner = Some(winner);
            }
        }
        let win_bits = match (self.win_tick, self.winner) {
            (Some(tick), Some(winner)) => {
                u64::from(tick).wrapping_mul(FIXTURE_PRIME) ^ u64::from(winner).wrapping_add(1)
            }
            _ => 0,
        };
        let mut step_hash = FNV64_OFFSET;
        fixture_hash_write(&mut step_hash, &self.tick.to_le_bytes());
        for index in 0..usize::from(self.runners) {
            for value in [
                self.pos[index][0],
                self.pos[index][1],
                self.vel[index][0],
                self.vel[index][1],
            ] {
                fixture_hash_write(
                    &mut step_hash,
                    &fixture_canon_zero(value).to_bits().to_le_bytes(),
                );
            }
        }
        fixture_hash_write(&mut step_hash, &win_bits.to_le_bytes());
        fixture_hash_write(&mut step_hash, &self.context.to_le_bytes());
        self.clock = mix64(self.clock ^ step_hash);
        self.checkpoints.push(FixtureCheckpoint {
            tick: self.tick,
            hash: self.clock,
        });
        self.tick += 1;
    }

    /// Drive the match to completion and seal the tape.
    fn run_to_tape(mut self) -> FixtureTape {
        while !self.is_done() {
            self.step();
        }
        FixtureTape {
            checkpoints: self.checkpoints,
            win_tick: self.win_tick,
            winner: self.winner,
        }
    }
}

/// One booted fixture mode: the authoritative [`FixtureSession`] plus the
/// presentation [`Director`] framing it and the follow-camera table from the
/// same rule set. The framework boots this from rule-set text (or tables)
/// and drives it tick by tick; camera records never enter the sim hash.
pub struct FixtureMode {
    pub(super) session: FixtureSession,
    pub(super) director: Director,
    pub(super) camera: FixtureCamera,
    pub(super) frames: Vec<CameraFrame>,
    pub(super) follow: Vec<[f32; 3]>,
}

impl FixtureMode {
    /// Boot from validated rules under the traversal-course camera table.
    #[must_use]
    pub fn boot(rules: &FixtureRules) -> Self {
        // Traversal tables validate by construction (covered by the preset
        // test); a mode boot from validated rules is total.
        Self::boot_with_camera(rules, DirectorParams::traversal())
            .expect("traversal camera table is valid")
    }

    /// Boot from validated rules under an explicit director table (mode
    /// rule sets pick cameras as data): a bad table fails before any tick.
    /// Budget: O(runners + inputs) time, three small `Vec`s at the boot
    /// boundary (never per tick).
    pub fn boot_with_camera(
        rules: &FixtureRules,
        params: DirectorParams,
    ) -> Result<Self, CameraError> {
        let capacity = rules.ticks as usize;
        Ok(Self {
            session: FixtureSession::boot(rules),
            director: Director::new(0, params)?,
            camera: rules.camera,
            frames: Vec::with_capacity(capacity),
            follow: Vec::with_capacity(capacity),
        })
    }

    /// Parse rule-set text then boot under the traversal-course table.
    pub fn boot_from_text(rules_text: &str) -> Result<Self, FixtureError> {
        Ok(Self::boot(&FixtureRules::parse(rules_text)?))
    }

    /// Parse rule-set text plus a director data table
    /// ([`DirectorParams::decode`]) then boot: a bad table names its reason
    /// instead of simulating.
    pub fn boot_from_tables(rules_text: &str, camera_table: &[u8]) -> Result<Self, FixtureError> {
        let rules = FixtureRules::parse(rules_text)?;
        let params = DirectorParams::decode(camera_table).map_err(|err| match err {
            CameraError::BadLength(_) => FixtureError::BadCameraTable("length"),
            CameraError::FutureVersion(_) => FixtureError::BadCameraTable("version"),
            CameraError::BadParam(row) => FixtureError::BadCamera(row),
            _ => FixtureError::BadCameraTable("row"),
        })?;
        Self::boot_with_camera(&rules, params).map_err(|_| FixtureError::BadCameraTable("row"))
    }

    /// True once every configured tick has simulated.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.session.is_done()
    }

    /// Swap the presentation camera table mid-run at a tick boundary (data,
    /// not code): validates first, sim state untouched. Deterministic for
    /// the same retune tick.
    pub fn retune_camera(&mut self, params: DirectorParams) -> Result<(), CameraError> {
        self.director.retune(params)
    }

    /// Simulate one fixed tick plus both presentation cameras: the director
    /// frames the runners as [`FighterView`]s (focus 0) and the follow table
    /// centers them. Returns false once the match is complete. A non-finite
    /// sim position would fail loudly here; sim math keeps positions finite
    /// by construction (bounded well below overflow).
    pub fn step(&mut self) -> Result<bool, CameraError> {
        if self.session.is_done() {
            return Ok(false);
        }
        let tick = self.session.tick;
        self.session.step();
        let mut views = [FighterView {
            id: 0,
            pos_m: Vec3::ZERO,
            alive: true,
        }; 8];
        let runners = fixture_views_into(self.session.positions(), &mut views);
        let frame = self.director.update(&views[..runners], tick)?;
        self.frames.push(frame);
        self.follow.push(
            self.camera
                .frame(self.session.positions(), self.session.goal),
        );
        Ok(!self.session.is_done())
    }

    /// Drive the mode to completion: sealed tape plus both camera records.
    pub fn run(mut self) -> Result<FixturePlayback, CameraError> {
        while self.step()? {}
        let session = self.session;
        Ok(FixturePlayback {
            tape: FixtureTape {
                checkpoints: session.checkpoints,
                win_tick: session.win_tick,
                winner: session.winner,
            },
            frames: self.frames,
            follow: self.follow,
        })
    }
}

/// Played fixture mode: sealed sim tape plus the director frames and the
/// follow-camera centers recorded alongside it (presentation receipts).
#[derive(Clone, PartialEq, Debug)]
pub struct FixturePlayback {
    /// Authoritative sim record: replays byte-exact under the same rules.
    pub tape: FixtureTape,
    /// Director frame per tick (presentation only, never hashed).
    pub frames: Vec<CameraFrame>,
    /// Follow-camera center per tick (presentation only, never hashed).
    pub follow: Vec<[f32; 3]>,
}

impl FixturePlayback {
    /// Replay the authoritative tape under its rules with first-divergence
    /// naming; camera receipts are presentation and replay by equality
    /// (run twice, compare).
    pub fn replay(&self, rules: &FixtureRules) -> Result<(), FixtureError> {
        self.tape.replay(rules)
    }
}

/// Map planar runner positions onto [`FighterView`]s for the director
/// (focus 0, runner height 1 m, all alive): stack buffer, no heap. Returns
/// the runner count.
pub(super) fn fixture_views_into(positions: &[[f32; 2]], views: &mut [FighterView; 8]) -> usize {
    let count = positions.len().min(views.len());
    for (index, plane) in positions.iter().enumerate().take(count) {
        views[index] = FighterView {
            id: index as u8,
            pos_m: Vec3::new(plane[0], 1.0, plane[1]),
            alive: true,
        };
    }
    count
}
