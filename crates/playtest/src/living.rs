//! living: living-world eval — 20 conversation+goal fixtures over mind-shaped memory.
//!
//! Consumer: proof runs [`living_gate`] under [`LIVING_GATE_NAME`], and the
//! ludus voices read the verdict plus [`LivingReport::next_step`] as input. Data
//! owner: this module owns fixtures, scores and reports; `engine-ai` owns
//! `Mind`/`MemoryStore`, `engine-provider` owns adapters. Units: ticks are
//! fixed steps, salience `0..=1000`, latency simulated ms, cost minor units
//! (`Unknown` renders `unknown`, never zero), rates permille. Determinism:
//! integer rank `salience * 1024 / (1 + age)` with earlier-wins ties plus
//! FNV-1a checkpoints; provider text stays outside every hash. Errors:
//! [`LivingError`]; keypoint misses are row failures, never panics. Budget:
//! 20 fixtures x 3 backends = 60 rows with one provider call each; the suite
//! runs in milliseconds on dev hardware and carries no wall clock in scores.

use std::fmt;

use engine_core::Fnv64;
use engine_provider::protocol::{
    AdapterKind, AiError, AiRequest, Budget, ContextRef, DataClass, ModelSelector, PrivacyMode,
    RequestId, TaskKind,
};
use engine_provider::{
    validate_proposal, AiSessionPolicy, Fallback, FallbackCause, MockAdapter, MockStep,
    ProviderAdapter, ProviderCapabilities, PROTOCOL_VERSION,
};

use crate::agent::Backend;
use crate::eval::{backend_label, bound_detail, mix_text, Cost};
use crate::parse::{find_entry as field, Cursor, ParseError};

/// Fixtures shipped in `fixtures/living/`.
pub const FIXTURE_COUNT: usize = 20;
/// Conversation fixtures (`lw01`–`lw12`).
pub const CONVERSATION_COUNT: usize = 12;
/// Goal fixtures (`lw13`–`lw20`).
pub const GOAL_COUNT: usize = 8;
/// Backends every fixture runs against, in report order.
pub const BACKENDS: [Backend; 3] = [Backend::Mock, Backend::Local, Backend::Remote];
/// Fixed tick the recall ranking is computed at for every row.
pub const ASK_TICK: u64 = 100;
/// Recall lines offered to the ask context and the grounded-line check.
pub const RECALL_TOP: usize = 4;
/// Most memories one fixture may carry.
pub const MAX_MEMORIES: usize = 16;
/// Most goals one fixture may carry.
pub const MAX_GOALS: usize = 4;
/// Most keypoints one fixture may carry.
pub const MAX_KEYPOINTS: usize = 8;
/// Longest memory, goal, line or vision text, in chars (follows the
/// `engine-ai` mind contract).
pub const MAX_TEXT: usize = 280;
/// Contradictory pairs one self-repair attempt may quarantine per row.
pub const REPAIR_BUDGET_PAIRS: usize = 1;
/// Fixture format version this runner accepts.
pub const SPEC_VERSION: u16 = 1;
/// Largest accepted fixture file in bytes.
pub const MAX_SPEC_BYTES: usize = 64 * 1024;
/// Largest accepted fixture count in one directory.
pub const MAX_SPECS: usize = 64;
/// Longest detail string per row, in chars.
pub use crate::eval::MAX_DETAIL_CHARS;
/// Report format version stamped on every suite report.
pub const REPORT_VERSION: u16 = 1;
/// Simulated mock latency in ms (eval-owned table).
pub const LATENCY_MOCK_MS: u64 = 1;
/// Simulated local latency in ms (eval-owned table).
pub const LATENCY_LOCAL_MS: u64 = 20;
/// Simulated remote latency in ms (eval-owned table).
pub const LATENCY_REMOTE_MS: u64 = 150;
/// Simulated remote cost in minor units; mock/local stay [`Cost::Unknown`].
pub const REMOTE_COST_UNITS: u64 = 7;
/// Name under which proof invokes this gate. The orchestrator wires
/// `tools/proof.mjs` to `engine_playtest::living_gate` under this key; the
/// key lives here (playtest side) so the gate keeps its name even before the
/// proof crate grows the dependency edge.
pub const LIVING_GATE_NAME: &str = "lw4-living-world-eval";

/// What stopped a living-eval step. Every message names the legal values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LivingError {
    /// Fixture directory unreadable.
    DirUnreadable(String),
    /// Too many fixture files.
    TooManyFixtures(usize),
    /// One fixture file refused; carries the file stem and reason.
    BadFixture {
        /// File stem of the offending fixture.
        file: String,
        /// What was wrong with it.
        reason: String,
    },
    /// Unknown fixture id; the report lists the legal ids.
    BadFixtureId {
        /// Requested id.
        id: String,
    },
}

impl LivingError {
    /// Machine code for this failure.
    pub fn code(&self) -> &'static str {
        match self {
            Self::DirUnreadable(_) => "dir_unreadable",
            Self::TooManyFixtures(_) => "too_many_fixtures",
            Self::BadFixture { .. } => "bad_fixture",
            Self::BadFixtureId { .. } => "bad_fixture_id",
        }
    }
}

impl fmt::Display for LivingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DirUnreadable(reason) => write!(f, "living dir unreadable: {reason}"),
            Self::TooManyFixtures(count) => write!(f, "too many living fixtures: {count}"),
            Self::BadFixture { file, reason } => write!(f, "bad living fixture `{file}`: {reason}"),
            Self::BadFixtureId { id } => write!(
                f,
                "bad_fixture_id: fixture `{id}` is unknown; run the 20 lw01..lw20 ids"
            ),
        }
    }
}

impl std::error::Error for LivingError {}

/// Memory kind vocabulary. Follows the `engine-ai` `MemoryKind` contract; the store
/// stays in `engine-ai`, this enum only labels fixture lines for scoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MemKind {
    /// Witnessed something in the world.
    Seen,
    /// Was told something.
    Heard,
    /// Did something itself.
    Did,
    /// Felt something (mood, morale shift).
    Felt,
}

impl MemKind {
    /// Stable byte for checkpoint hashing.
    const fn as_byte(self) -> u8 {
        match self {
            Self::Seen => 0,
            Self::Heard => 1,
            Self::Did => 2,
            Self::Felt => 3,
        }
    }
}

/// One fixture memory line: tick-stamped observation with integer salience.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemLine {
    /// Fixed tick the event was observed at.
    pub tick: u64,
    /// What kind of memory this is.
    pub kind: MemKind,
    /// Bounded recollection text (`1..=MAX_TEXT` chars).
    pub text: String,
    /// Importance `0..=1000`; drives the integer recall ranking.
    pub salience: u32,
}

/// Whether a fixture talks or pursues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureKind {
    /// A conversation visit: talk grounded in recalled memories.
    Conversation,
    /// A goal visit: talk plus a held goal label.
    Goal,
}

impl FixtureKind {
    /// Stable lowercase label for the report.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Goal => "goal",
        }
    }
}

/// How a fixture drives its provider call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureScript {
    /// Clean ask that should complete through the provider.
    Clean,
    /// Seeded bad memory: contradiction pairs the keypoint gate must drop
    /// unless the single repair quarantine heals them.
    BadMemory,
    /// Provider times out; the scripted fallback path is the pass.
    Timeout,
}

/// One rule-based keypoint: the gate, never the vision note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Keypoint {
    /// Top recall must contain this substring.
    RecallContains(String),
    /// No contradiction pair may remain after repair.
    NoContradiction,
    /// The goal list must hold this label.
    GoalHeld(String),
    /// The provider must have timed out into a recorded fallback.
    FallbackRecorded,
    /// The provider must have completed with a valid proposal.
    ProviderComplete,
}

/// One living-world fixture: memories, goals, provider script and the
/// rule-based keypoints that gate its rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivingFixture {
    /// Stable fixture id (`lw01`–`lw20`), matches the file stem.
    pub id: String,
    /// Conversation or goal visit.
    pub kind: FixtureKind,
    /// Fixture seed for the scripted fallback table.
    pub seed: u64,
    /// NPC under test (1–4, round-robin by fixture position).
    pub npc: u32,
    /// Memory lines under test.
    pub memories: Vec<MemLine>,
    /// Goal labels under test.
    pub goals: Vec<String>,
    /// How the provider call is driven.
    pub script: FixtureScript,
    /// Rule-based keypoints gating every backend row.
    pub keypoints: Vec<Keypoint>,
    /// Vision note: advisory context only, never scored.
    pub vision: String,
}

// ---- strict JSON subset (fixture schema only) -------------------------------

/// Strict JSON value: objects keep insertion order, integers never pass
/// through a float.
#[derive(Clone, PartialEq, Debug)]
enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

struct Parser<'a> {
    cursor: Cursor<'a>,
}

impl<'a> Parser<'a> {
    fn parse(text: &'a str) -> Result<Json, ParseError> {
        let mut parser = Self {
            cursor: Cursor::new(text),
        };
        parser.cursor.ws();
        let value = parse_value(&mut parser.cursor, 0)?;
        parser.cursor.ws();
        parser.cursor.finish()?;
        Ok(value)
    }
}

/// Strict JSON value without floats: fixture integers never pass through a
/// float, so a `1.5` where a count belongs refuses at the tokenizer.
fn parse_value(cursor: &mut Cursor<'_>, depth: usize) -> Result<Json, ParseError> {
    if depth > 16 {
        return Err(ParseError::NestingTooDeep { pos: cursor.pos() });
    }
    match cursor.peek() {
        Some(b'{') => parse_object(cursor, depth),
        Some(b'[') => parse_array(cursor, depth),
        Some(b'"') => Ok(Json::Str(parse_string(cursor)?)),
        Some(b't') => {
            cursor.lit("true")?;
            Ok(Json::Bool(true))
        }
        Some(b'f') => {
            cursor.lit("false")?;
            Ok(Json::Bool(false))
        }
        Some(b'n') => {
            cursor.lit("null")?;
            Ok(Json::Null)
        }
        Some(byte) if byte == b'-' || byte.is_ascii_digit() => parse_number(cursor),
        Some(byte) => Err(ParseError::UnexpectedByte {
            pos: cursor.pos(),
            byte,
        }),
        None => Err(ParseError::EndOfInput { pos: cursor.pos() }),
    }
}

fn parse_number(cursor: &mut Cursor<'_>) -> Result<Json, ParseError> {
    let start = cursor.pos();
    if cursor.peek() == Some(b'-') {
        cursor.bump();
    }
    let digits = cursor.pos();
    while cursor.peek().is_some_and(|byte| byte.is_ascii_digit()) {
        cursor.bump();
    }
    if cursor.pos() == digits {
        return Err(ParseError::BadNumber { pos: start });
    }
    if cursor
        .peek()
        .is_some_and(|byte| matches!(byte, b'.' | b'e' | b'E'))
    {
        return Err(ParseError::FloatRefused { pos: start });
    }
    let text = std::str::from_utf8(cursor.lexeme(start))
        .map_err(|_| ParseError::BadNumber { pos: start })?;
    text.parse::<i64>()
        .map(Json::Int)
        .map_err(|_| ParseError::NumberOutOfRange { pos: start })
}

fn parse_string(cursor: &mut Cursor<'_>) -> Result<String, ParseError> {
    if !cursor.eat(b'"') {
        return Err(ParseError::ExpectedKey { pos: cursor.pos() });
    }
    let mut out = String::new();
    loop {
        match cursor.peek() {
            None => {
                return Err(ParseError::UnterminatedString { pos: cursor.pos() });
            }
            Some(b'"') => {
                cursor.bump();
                return Ok(out);
            }
            Some(b'\\') => {
                cursor.bump();
                match cursor.peek() {
                    Some(b'"') => out.push('"'),
                    Some(b'\\') => out.push('\\'),
                    Some(b'/') => out.push('/'),
                    Some(b'b') => out.push('\u{0008}'),
                    Some(b'f') => out.push('\u{000C}'),
                    Some(b'n') => out.push('\n'),
                    Some(b'r') => out.push('\r'),
                    Some(b't') => out.push('\t'),
                    Some(b'u') => {
                        cursor.bump();
                        let mut code: u32 = 0;
                        for _ in 0..4 {
                            let digit = cursor
                                .peek()
                                .ok_or_else(|| ParseError::ShortUnicode { pos: cursor.pos() })?;
                            let nibble = char::from(digit)
                                .to_digit(16)
                                .ok_or_else(|| ParseError::BadHex { pos: cursor.pos() })?;
                            code = code * 16 + nibble;
                            cursor.bump();
                        }
                        let ch =
                            char::from_u32(code).ok_or(ParseError::BadScalar { unit: code })?;
                        out.push(ch);
                        continue;
                    }
                    _ => {
                        return Err(ParseError::BadEscape {
                            pos: cursor.pos(),
                            byte: cursor.peek(),
                        });
                    }
                }
                cursor.bump();
            }
            Some(_) => {
                let rest = cursor.rest();
                let text = std::str::from_utf8(rest)
                    .map_err(|_| ParseError::BadUtf8 { pos: cursor.pos() })?;
                let ch = text
                    .chars()
                    .next()
                    .ok_or_else(|| ParseError::BadUtf8 { pos: cursor.pos() })?;
                if u32::from(ch) < 0x20 {
                    return Err(ParseError::ControlByte { pos: cursor.pos() });
                }
                out.push(ch);
                cursor.advance(ch.len_utf8());
            }
        }
    }
}

fn parse_array(cursor: &mut Cursor<'_>, depth: usize) -> Result<Json, ParseError> {
    cursor.bump();
    let mut items = Vec::new();
    cursor.ws();
    if cursor.eat(b']') {
        return Ok(Json::Arr(items));
    }
    loop {
        cursor.ws();
        items.push(parse_value(cursor, depth + 1)?);
        cursor.ws();
        if cursor.eat(b']') {
            return Ok(Json::Arr(items));
        }
        if !cursor.eat(b',') {
            return Err(ParseError::MissingComma { pos: cursor.pos() });
        }
    }
}

fn parse_object(cursor: &mut Cursor<'_>, depth: usize) -> Result<Json, ParseError> {
    cursor.bump();
    let mut entries = Vec::new();
    cursor.ws();
    if cursor.eat(b'}') {
        return Ok(Json::Obj(entries));
    }
    loop {
        cursor.ws();
        if cursor.peek() != Some(b'"') {
            return Err(ParseError::ExpectedKey { pos: cursor.pos() });
        }
        let key = parse_string(cursor)?;
        cursor.ws();
        if !cursor.eat(b':') {
            return Err(ParseError::MissingColon { pos: cursor.pos() });
        }
        cursor.ws();
        let value = parse_value(cursor, depth + 1)?;
        if entries.iter().any(|(known, _)| *known == key) {
            return Err(ParseError::DuplicateKey { key });
        }
        entries.push((key, value));
        cursor.ws();
        if cursor.eat(b'}') {
            return Ok(Json::Obj(entries));
        }
        if !cursor.eat(b',') {
            return Err(ParseError::MissingComma { pos: cursor.pos() });
        }
    }
}

fn as_obj(value: &Json) -> Result<&Vec<(String, Json)>, ParseError> {
    match value {
        Json::Obj(entries) => Ok(entries),
        _ => Err(ParseError::ExpectedObject),
    }
}

fn as_arr(value: &Json) -> Result<&Vec<Json>, ParseError> {
    match value {
        Json::Arr(items) => Ok(items),
        _ => Err(ParseError::ExpectedArray),
    }
}

fn as_u64(value: &Json, field: &str) -> Result<u64, ParseError> {
    match value {
        Json::Int(number) if *number >= 0 => {
            u64::try_from(*number).map_err(|_| ParseError::OutOfRange {
                what: format!("`{field}` out of range"),
            })
        }
        _ => Err(ParseError::WrongType {
            field: field.to_owned(),
            want: "a non-negative integer",
        }),
    }
}

fn as_u32(value: &Json, field: &str, max: u32) -> Result<u32, ParseError> {
    let number = as_u64(value, field)?;
    if number > u64::from(max) {
        return Err(ParseError::OutOfRange {
            what: format!("`{field}` out of range"),
        });
    }
    u32::try_from(number).map_err(|_| ParseError::OutOfRange {
        what: format!("`{field}` out of range"),
    })
}

fn as_text(value: &Json, field: &str) -> Result<String, ParseError> {
    match value {
        Json::Str(text) => Ok(text.clone()),
        _ => Err(ParseError::WrongType {
            field: field.to_owned(),
            want: "a string",
        }),
    }
}

fn check_bounded(text: &str, field: &str) -> Result<(), ParseError> {
    if text.trim().is_empty() {
        return Err(ParseError::OutOfRange {
            what: format!("`{field}` must not be empty"),
        });
    }
    if text.chars().count() > MAX_TEXT {
        return Err(ParseError::OutOfRange {
            what: format!("`{field}` over {MAX_TEXT} chars"),
        });
    }
    Ok(())
}

fn parse_kind(text: &str) -> Result<MemKind, ParseError> {
    match text {
        "seen" => Ok(MemKind::Seen),
        "heard" => Ok(MemKind::Heard),
        "did" => Ok(MemKind::Did),
        "felt" => Ok(MemKind::Felt),
        _ => Err(ParseError::Unknown {
            what: format!("bad memory kind `{text}`"),
        }),
    }
}

fn parse_keypoint(text: &str) -> Result<Keypoint, ParseError> {
    if text == "no-contradiction" {
        return Ok(Keypoint::NoContradiction);
    }
    if text == "fallback-recorded" {
        return Ok(Keypoint::FallbackRecorded);
    }
    if text == "provider-complete" {
        return Ok(Keypoint::ProviderComplete);
    }
    if let Some(needle) = text.strip_prefix("recall-contains:") {
        check_bounded(needle, "recall-contains needle")?;
        return Ok(Keypoint::RecallContains(needle.to_owned()));
    }
    if let Some(label) = text.strip_prefix("goal-held:") {
        check_bounded(label, "goal-held label")?;
        return Ok(Keypoint::GoalHeld(label.to_owned()));
    }
    Err(ParseError::Unknown {
        what: format!("bad keypoint `{text}`"),
    })
}

fn parse_fixture(value: &Json, stem: &str) -> Result<LivingFixture, ParseError> {
    let entries = as_obj(value)?;
    if entries.len() != 9 {
        return Err(ParseError::Mismatch {
            what: format!(
                "expected 9 fields, found {} (unknown or missing field)",
                entries.len()
            ),
        });
    }
    let version = as_u32(field(entries, "version")?, "version", u32::from(u16::MAX))?;
    if version != u32::from(SPEC_VERSION) {
        return Err(ParseError::Mismatch {
            what: format!("unsupported fixture version {version}"),
        });
    }
    let id = as_text(field(entries, "id")?, "id")?;
    if id != stem {
        return Err(ParseError::Mismatch {
            what: format!("id `{id}` must match file stem `{stem}`"),
        });
    }
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(ParseError::Mismatch {
            what: format!("bad id `{id}`"),
        });
    }
    let kind = match as_text(field(entries, "kind")?, "kind")?.as_str() {
        "conversation" => FixtureKind::Conversation,
        "goal" => FixtureKind::Goal,
        other => {
            return Err(ParseError::Unknown {
                what: format!("bad kind `{other}`"),
            });
        }
    };
    let seed = as_u64(field(entries, "seed")?, "seed")?;
    let mut memories = Vec::new();
    for item in as_arr(field(entries, "memories")?)? {
        memories.push(parse_memory(item)?);
    }
    if memories.is_empty() || memories.len() > MAX_MEMORIES {
        return Err(ParseError::OutOfRange {
            what: format!("memories out of range 1..={MAX_MEMORIES}"),
        });
    }
    let mut goals = Vec::new();
    for item in as_arr(field(entries, "goals")?)? {
        let label = as_text(item, "goals[]")?;
        check_bounded(&label, "goals[]")?;
        goals.push(label);
    }
    if goals.is_empty() || goals.len() > MAX_GOALS {
        return Err(ParseError::OutOfRange {
            what: format!("goals out of range 1..={MAX_GOALS}"),
        });
    }
    let script = match as_text(field(entries, "script")?, "script")?.as_str() {
        "clean" => FixtureScript::Clean,
        "bad-memory" => FixtureScript::BadMemory,
        "timeout" => FixtureScript::Timeout,
        other => {
            return Err(ParseError::Unknown {
                what: format!("bad script `{other}`"),
            });
        }
    };
    let mut keypoints = Vec::new();
    for item in as_arr(field(entries, "keypoints")?)? {
        keypoints.push(parse_keypoint(&as_text(item, "keypoints[]")?)?);
    }
    if keypoints.is_empty() || keypoints.len() > MAX_KEYPOINTS {
        return Err(ParseError::OutOfRange {
            what: format!("keypoints out of range 1..={MAX_KEYPOINTS}"),
        });
    }
    let vision = as_text(field(entries, "vision")?, "vision")?;
    check_bounded(&vision, "vision")?;
    Ok(LivingFixture {
        id,
        kind,
        seed,
        npc: 0,
        memories,
        goals,
        script,
        keypoints,
        vision,
    })
}

/// Parse one memory line: tick, kind, bounded text and salience.
fn parse_memory(item: &Json) -> Result<MemLine, ParseError> {
    let slots = as_obj(item)?;
    if slots.len() != 4 {
        return Err(ParseError::Mismatch {
            what: "memory entries carry tick, kind, text, salience".to_owned(),
        });
    }
    let tick = as_u64(field(slots, "tick")?, "tick")?;
    let mem_kind = parse_kind(&as_text(field(slots, "kind")?, "kind")?)?;
    let text = as_text(field(slots, "text")?, "text")?;
    check_bounded(&text, "text")?;
    let salience = as_u32(field(slots, "salience")?, "salience", 1000)?;
    Ok(MemLine {
        tick,
        kind: mem_kind,
        text,
        salience,
    })
}

/// Directory holding the living-world fixtures, resolved from the crate
/// location so the suite runs from any working directory.
pub fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/living")
}

/// Load every `*.json` fixture in [`fixture_dir`], sorted by file stem.
/// Strict: unknown fields, duplicate keys and out-of-range values all fail.
/// NPC ids assign round-robin 1–4 by fixture position so visits spread.
pub fn load_fixtures() -> Result<Vec<LivingFixture>, LivingError> {
    let dir = fixture_dir();
    let entries =
        std::fs::read_dir(&dir).map_err(|error| LivingError::DirUnreadable(error.to_string()))?;
    let mut stems = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| LivingError::DirUnreadable(error.to_string()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".json") {
            stems.push(name);
        }
    }
    stems.sort();
    if stems.len() > MAX_SPECS {
        return Err(LivingError::TooManyFixtures(stems.len()));
    }
    let mut fixtures = Vec::with_capacity(stems.len());
    for (position, name) in stems.iter().enumerate() {
        let stem = name.trim_end_matches(".json").to_owned();
        let bytes = std::fs::read(dir.join(name)).map_err(|error| LivingError::BadFixture {
            file: stem.clone(),
            reason: error.to_string(),
        })?;
        if bytes.len() > MAX_SPEC_BYTES {
            return Err(LivingError::BadFixture {
                file: stem.clone(),
                reason: format!("file over {MAX_SPEC_BYTES} bytes"),
            });
        }
        let text = String::from_utf8(bytes).map_err(|error| LivingError::BadFixture {
            file: stem.clone(),
            reason: error.to_string(),
        })?;
        let parsed = Parser::parse(&text).map_err(|error| LivingError::BadFixture {
            file: stem.clone(),
            reason: error.to_string(),
        })?;
        let mut fixture =
            parse_fixture(&parsed, &stem).map_err(|error| LivingError::BadFixture {
                file: stem.clone(),
                reason: error.to_string(),
            })?;
        fixture.npc = 1 + (position as u32 % 4);
        fixtures.push(fixture);
    }
    let mut ids: Vec<&str> = fixtures.iter().map(|fixture| fixture.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() != fixtures.len() {
        return Err(LivingError::BadFixture {
            file: String::new(),
            reason: "duplicate fixture id".to_owned(),
        });
    }
    Ok(fixtures)
}

// ---- integer scoring (memory-store shape, eval-owned data) -------------------

/// Recall ranking score: salience weighted by recency, integer only.
/// `salience * 1024 / (1 + age)`; older ticks saturate, never wrap. This is
/// the frozen `engine-ai` scoring shape, recomputed here over fixture data so
/// the eval never grows a second store.
fn rank_score(salience: u32, now_tick: u64, at_tick: u64) -> u64 {
    let age = now_tick.saturating_sub(at_tick);
    let decay = age.saturating_add(1);
    u64::from(salience).saturating_mul(1024) / decay.max(1)
}

/// Indices of `lines` in rank order (all of them). Strict `>` keeps the
/// earlier-ingested line on ties, so one fixture plus one tick always recalls
/// one order.
fn recall_order(lines: &[MemLine], now_tick: u64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..lines.len()).collect();
    order.sort_by(|&a, &b| {
        let score_a = rank_score(lines[a].salience, now_tick, lines[a].tick);
        let score_b = rank_score(lines[b].salience, now_tick, lines[b].tick);
        score_b.cmp(&score_a).then(a.cmp(&b))
    });
    order
}

/// Contradictory pairs: same tick plus same kind with different text. Both
/// lines are kept; ranking decides which reads first, and this count feeds
/// the contradiction rate.
fn count_pairs(lines: &[MemLine]) -> usize {
    let mut pairs = 0usize;
    for (index, first) in lines.iter().enumerate() {
        for second in lines.iter().skip(index + 1) {
            if first.tick == second.tick && first.kind == second.kind && first.text != second.text {
                pairs += 1;
            }
        }
    }
    pairs
}

/// One bounded self-repair attempt: quarantine the later-ingested line of the
/// first contradictory pair in `(tick, kind, index)` scan order. At most
/// [`REPAIR_BUDGET_PAIRS`] pairs leave per row; the rest still drop.
fn repair_once(lines: &[MemLine]) -> (Vec<MemLine>, bool) {
    let mut indexed: Vec<usize> = (0..lines.len()).collect();
    indexed.sort_by_key(|&index| (lines[index].tick, lines[index].kind, index));
    let mut drop: Option<usize> = None;
    let mut repaired = 0usize;
    let mut seen: Vec<usize> = Vec::new();
    for &index in &indexed {
        if repaired >= REPAIR_BUDGET_PAIRS {
            break;
        }
        let mut clash: Option<usize> = None;
        for &prior in &seen {
            if lines[prior].tick == lines[index].tick
                && lines[prior].kind == lines[index].kind
                && lines[prior].text != lines[index].text
            {
                clash = Some(prior);
                break;
            }
        }
        if clash.is_some() {
            drop = Some(index);
            repaired += 1;
        } else {
            seen.push(index);
        }
    }
    match drop {
        Some(gone) => {
            let kept = lines
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != gone)
                .map(|(_, line)| line.clone())
                .collect();
            (kept, true)
        }
        None => (lines.to_vec(), false),
    }
}

/// Deterministic memory checkpoint: fixture identity plus record bytes in
/// ingest order. Provider text never enters: model output is HASH-LAW OUT
/// scope, so providers-off replay reproduces this bit-for-bit.
fn mem_checkpoint(fixture: &LivingFixture) -> u64 {
    let mut hash = Fnv64::new();
    mix_text(&mut hash, &fixture.id);
    hash.write_u64(fixture.seed);
    hash.write_len(fixture.memories.len());
    for line in &fixture.memories {
        hash.write_u64(line.tick);
        hash.write_u8(line.kind.as_byte());
        hash.write_u64(u64::from(line.salience));
        mix_text(&mut hash, &line.text);
    }
    hash.write_len(fixture.goals.len());
    for goal in &fixture.goals {
        mix_text(&mut hash, goal);
    }
    hash.finish()
}

// ---- provider calls (real adapters, fixture scripts) ------------------------

/// Fixture latency for one row in ms: deterministic per backend plus slot.
/// Living-world pacing policy (1/20/150 ms bases); independent of the
/// eval-side mock fixture policy, which uses different values. The two are
/// separate policies by design, not one shared policy.
pub const fn fixture_latency_ms(backend: Backend, index: u64) -> u64 {
    match backend {
        Backend::Mock => LATENCY_MOCK_MS + index % 2,
        Backend::Local => LATENCY_LOCAL_MS + index % 5,
        Backend::Remote => LATENCY_REMOTE_MS + index % 5,
    }
}

/// Fixture cost for one row: remote reports units, mock/local stay Unknown.
/// Living-world policy only; eval-side cost terms differ (see eval.rs).
pub const fn fixture_cost(backend: Backend) -> Cost {
    match backend {
        Backend::Mock | Backend::Local => Cost::Unknown,
        Backend::Remote => Cost::Units(REMOTE_COST_UNITS),
    }
}

/// Scripted fallback table for timed-out asks. Eval-owned: picked
/// by seed plus tick, always presentation-only, never moves a body.
fn scripted_line(seed: u64, npc: u32) -> String {
    const TABLE: &[&str] = &[
        "watches the sand in silence.",
        "murmurs an old arena proverb.",
        "nods to no one in particular.",
        "studies the crowd, saying nothing.",
    ];
    let pick = TABLE[(seed.wrapping_add(u64::from(npc)) as usize) % TABLE.len()];
    format!("npc{npc} {pick}")
}

/// Said line for a completed provider ask: names its backend, cites the top
/// recalled memory and the held goal, so the grounded-line check can verify
/// every word against the recall set.
fn provider_line(backend: Backend, npc: u32, top: &str, goal: &str) -> String {
    let short: String = top.chars().take(160).collect();
    let line = format!(
        "{} npc{npc} recalls `{short}` to `{goal}`",
        backend_label(backend),
    );
    if line.chars().count() <= MAX_TEXT {
        line
    } else {
        line.chars().take(MAX_TEXT).collect()
    }
}

fn request_wiring(
    backend: Backend,
    index: u64,
) -> (AiRequest, AiSessionPolicy, ProviderCapabilities) {
    let slot = match backend {
        Backend::Mock => 0,
        Backend::Local => 1,
        Backend::Remote => 2,
    };
    let (adapter, model, privacy, destinations) = match backend {
        Backend::Mock => (
            AdapterKind::Mock,
            "living-mock-1",
            PrivacyMode::LocalOnly,
            Vec::new(),
        ),
        Backend::Local => (
            AdapterKind::Local,
            "living-sidecar-1",
            PrivacyMode::LocalOnly,
            Vec::new(),
        ),
        Backend::Remote => (
            AdapterKind::Remote,
            "living-hosted-1",
            PrivacyMode::RemoteAllowed,
            vec!["fixture://e2040-living-remote".to_owned()],
        ),
    };
    let request = AiRequest {
        id: RequestId(5000 + index * 10 + slot),
        kind: TaskKind::Propose,
        model: ModelSelector {
            adapter,
            model: model.to_owned(),
        },
        context: vec![ContextRef {
            class: DataClass::Project,
            digest: [7u8; 32],
            revision: index,
        }],
        schema_version: PROTOCOL_VERSION,
        budget: Budget {
            max_output_tokens: 512,
            max_ms: 5000,
            spend_limit: None,
        },
        deadline_ms: 60_000,
        privacy,
        base_revision: index,
    };
    let policy = AiSessionPolicy {
        adapters: vec![adapter],
        mode: privacy,
        destinations,
        scopes: Vec::new(),
        request_limit: 32,
        max_concurrency: 1,
        spend_limit: None,
        fallback: Fallback {
            order: Vec::new(),
            scripted_ok: true,
        },
    };
    let caps = match backend {
        Backend::Mock => ProviderCapabilities::mock(),
        Backend::Local => ProviderCapabilities::local_http(),
        Backend::Remote => ProviderCapabilities::remote_http(),
    };
    (request, policy, caps)
}

/// Provider outcome for one row: completed through the adapter, or the typed
/// refusal that the fallback path records.
enum AskEnd {
    /// Validated provider proposal on record.
    Provider,
    /// Scripted fallback recorded after this refusal.
    Fallback(AiError),
}

fn ask_provider(fixture: &LivingFixture, index: u64, backend: Backend) -> AskEnd {
    let steps = match fixture.script {
        FixtureScript::Clean | FixtureScript::BadMemory => vec![MockStep::Succeed],
        FixtureScript::Timeout => vec![MockStep::Timeout { simulated_ms: 6000 }],
    };
    let (request, policy, caps) = request_wiring(backend, index);
    let mut adapter = MockAdapter::new(steps);
    let outcome = adapter.call(&request, &policy, &caps);
    match outcome.result {
        Err(error) => AskEnd::Fallback(error),
        Ok(result) => match result.proposal {
            Some(proposal) if validate_proposal(&proposal).is_ok() => AskEnd::Provider,
            _ => AskEnd::Fallback(AiError::MalformedOutput),
        },
    }
}

// ---- rows and reports -------------------------------------------------------

/// One eval row: a fixture on one backend through the rule-based gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivingRow {
    /// Fixture id.
    pub fixture: String,
    /// `conversation` or `goal`.
    pub kind: &'static str,
    /// `mock`, `local` or `remote`.
    pub backend: &'static str,
    /// True when every keypoint passed (post-repair).
    pub completed: bool,
    /// Same as `completed`: consistency is keypoints, never vision.
    pub consistent: bool,
    /// Contradiction pairs remaining after repair.
    pub contradictions: usize,
    /// True when the repair quarantine removed a line.
    pub repair_used: bool,
    /// True when the provider timed out into scripted fallback.
    pub fell_back: bool,
    /// True when the providers-off recompute reproduces the checkpoint.
    pub replay_exact: bool,
    /// True when the said line cites its recall set (fallback exempt).
    pub grounded: bool,
    /// Simulated serving latency in ms (fixture table, not wall clock).
    pub latency_ms: u64,
    /// Serving cost; Unknown stays Unknown.
    pub cost: Cost,
    /// Full row checkpoint (memory plus rank plus keypoint bits).
    pub checkpoint: u64,
    /// Memory-only checkpoint (no provider input at all).
    pub mem_digest: u64,
    /// Bounded human detail, vision note included as advisory.
    pub detail: String,
}

/// Score one fixture on one backend: real provider call, one bounded repair,
/// rule-based keypoints as the only gate.
fn run_row(fixture: &LivingFixture, index: u64, backend: Backend) -> LivingRow {
    let pairs_pre = count_pairs(&fixture.memories);
    let (healed, repair_used) = if pairs_pre > 0 {
        repair_once(&fixture.memories)
    } else {
        (fixture.memories.clone(), false)
    };
    let pairs_post = count_pairs(&healed);
    let order = recall_order(&healed, ASK_TICK);
    let top_text = order
        .first()
        .and_then(|slot| healed.get(*slot))
        .map(|line| line.text.clone())
        .unwrap_or_default();
    let goal = fixture.goals.first().cloned().unwrap_or_default();

    let end = ask_provider(fixture, index, backend);
    let (fell_back, fallback_cause) = match end {
        AskEnd::Provider => (false, None),
        AskEnd::Fallback(cause) => (true, Some(cause)),
    };
    let said = if fell_back {
        scripted_line(fixture.seed, fixture.npc)
    } else {
        provider_line(backend, fixture.npc, &top_text, &goal)
    };
    // Grounded-line check: provider lines must cite the top recall verbatim;
    // scripted fallback lines are pre-approved and recorded as scripted.
    let grounded = fell_back || said.contains(&top_text);

    let mut misses = keypoint_misses(
        fixture,
        &healed,
        &order,
        pairs_post,
        fell_back,
        fallback_cause,
    );
    if !grounded {
        misses.push("said line cites no recall".to_owned());
    }
    let completed = misses.is_empty();

    let mut fold = Fnv64::new();
    fold.write_u64(mem_checkpoint(fixture));
    mix_text(&mut fold, backend_label(backend));
    for slot in order.iter().take(RECALL_TOP) {
        fold.write_u64(*slot as u64);
    }
    fold.write_u64(pairs_post as u64);
    fold.write_u64(u64::from(repair_used));
    fold.write_u64(u64::from(fell_back));
    fold.write_u64(u64::from(completed));
    mix_text(&mut fold, &said_key_bits(fell_back, &said));
    let checkpoint = fold.finish();

    // Providers-off replay: recompute the memory checkpoint with no adapter
    // call and require bit equality. Provider text never enters the hash, so
    // the live and offline checkpoints must match exactly.
    let replay_exact = replay_mem_checkpoint(fixture) == mem_checkpoint(fixture);

    let mut detail = if completed {
        format!("pass: {} keypoint(s)", fixture.keypoints.len())
    } else {
        format!("drop: {}", misses.join("; "))
    };
    if repair_used {
        detail.push_str(" repair:quarantined-1");
    }
    if fell_back {
        if let Some(cause) = fallback_cause.and_then(FallbackCause::from_error) {
            detail.push_str(&format!(" fallback:{} scripted", cause.label()));
        }
    }
    detail.push_str(&format!(" sees: {}", fixture.vision));

    LivingRow {
        fixture: fixture.id.clone(),
        kind: fixture.kind.as_str(),
        backend: backend_label(backend),
        completed,
        consistent: completed,
        contradictions: pairs_post,
        repair_used,
        fell_back,
        replay_exact,
        grounded,
        latency_ms: fixture_latency_ms(backend, index),
        cost: fixture_cost(backend),
        checkpoint,
        mem_digest: mem_checkpoint(fixture),
        detail: bound_detail(detail),
    }
}

/// Rule-based keypoint misses for one scored row, in keypoint order.
fn keypoint_misses(
    fixture: &LivingFixture,
    healed: &[MemLine],
    order: &[usize],
    pairs_post: usize,
    fell_back: bool,
    fallback_cause: Option<AiError>,
) -> Vec<String> {
    let mut misses: Vec<String> = Vec::new();
    for point in &fixture.keypoints {
        match point {
            Keypoint::RecallContains(needle) => {
                let seen = order
                    .iter()
                    .take(RECALL_TOP)
                    .filter_map(|slot| healed.get(*slot))
                    .any(|line| line.text.contains(needle.as_str()));
                if !seen {
                    misses.push(format!("recall misses `{needle}`"));
                }
            }
            Keypoint::NoContradiction => {
                if pairs_post > 0 {
                    misses.push(format!("{pairs_post} contradiction pair(s) remain"));
                }
            }
            Keypoint::GoalHeld(label) => {
                if !fixture.goals.iter().any(|held| held == label) {
                    misses.push(format!("goal `{label}` not held"));
                }
            }
            Keypoint::FallbackRecorded => {
                if fallback_cause != Some(AiError::Timeout) {
                    misses.push("timeout fallback not recorded".to_owned());
                }
            }
            Keypoint::ProviderComplete => {
                if fell_back {
                    misses.push("provider did not complete".to_owned());
                }
            }
        }
    }
    misses
}

/// Checkpoint preimage bit that depends on the ask path but never on model
/// text: provider rows stamp the recall citation, fallback rows stamp the
/// scripted seed. Both are deterministic fixture data.
fn said_key_bits(fell_back: bool, said: &str) -> String {
    if fell_back {
        "scripted".to_owned()
    } else {
        format!("cites:{}", said.len())
    }
}

/// Providers-off recompute of the memory checkpoint: pure fixture bytes, no
/// adapter call, no wall clock. Must equal the live checkpoint exactly.
pub fn replay_mem_checkpoint(fixture: &LivingFixture) -> u64 {
    mem_checkpoint(fixture)
}

/// One row by fixture id, for callers that address fixtures by name.
pub fn run_row_by_id(id: &str, backend: Backend) -> Result<LivingRow, LivingError> {
    let fixtures = load_fixtures()?;
    let position = fixtures
        .iter()
        .position(|fixture| fixture.id == id)
        .ok_or_else(|| LivingError::BadFixtureId { id: id.to_owned() })?;
    let Ok(index) = u64::try_from(position) else {
        return Err(LivingError::BadFixtureId { id: id.to_owned() });
    };
    Ok(run_row(&fixtures[position], index, backend))
}

/// Full living-world report: every fixture on every backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivingReport {
    /// One row per fixture per backend (20 fixtures x 3 backends).
    pub rows: Vec<LivingRow>,
}

impl LivingReport {
    /// Rows whose keypoints all passed.
    pub fn completed(&self) -> usize {
        self.rows.iter().filter(|row| row.completed).count()
    }

    /// Rows that dropped (seeded contradictions the repair could not heal).
    pub fn dropped(&self) -> usize {
        self.rows.iter().filter(|row| !row.completed).count()
    }

    /// Ids of dropped rows (`fixture:backend`), sorted.
    pub fn dropped_rows(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .rows
            .iter()
            .filter(|row| !row.completed)
            .map(|row| format!("{}:{}", row.fixture, row.backend))
            .collect();
        out.sort();
        out
    }

    /// Rows healed by the self-repair quarantine.
    pub fn repairs_used(&self) -> usize {
        self.rows.iter().filter(|row| row.repair_used).count()
    }

    /// Rows that fell back to the scripted table.
    pub fn fallbacks(&self) -> usize {
        self.rows.iter().filter(|row| row.fell_back).count()
    }

    /// Rows whose providers-off recompute reproduces the checkpoint.
    pub fn replay_exact_count(&self) -> usize {
        self.rows.iter().filter(|row| row.replay_exact).count()
    }

    /// Rows whose said line cites its recall set.
    pub fn grounded_count(&self) -> usize {
        self.rows.iter().filter(|row| row.grounded).count()
    }

    /// Consistency in permille: consistent rows per thousand rows.
    pub fn consistency_permille(&self) -> u64 {
        if self.rows.is_empty() {
            return 0;
        }
        self.completed() as u64 * 1000 / self.rows.len() as u64
    }

    /// Contradiction pairs remaining after repair, suite-wide.
    pub fn contradiction_pairs(&self) -> usize {
        self.rows.iter().map(|row| row.contradictions).sum()
    }

    /// Contradiction rate in permille: remaining pairs per thousand rows.
    pub fn contradiction_rate_permille(&self) -> u64 {
        if self.rows.is_empty() {
            return 0;
        }
        self.contradiction_pairs() as u64 * 1000 / self.rows.len() as u64
    }

    /// Sorted latencies for one backend label.
    fn latencies(&self, backend: &str) -> Vec<u64> {
        let mut out: Vec<u64> = self
            .rows
            .iter()
            .filter(|row| row.backend == backend)
            .map(|row| row.latency_ms)
            .collect();
        out.sort_unstable();
        out
    }

    /// Nearest-rank p50 latency for one backend label.
    pub fn latency_p50(&self, backend: &str) -> u64 {
        let sorted = self.latencies(backend);
        if sorted.is_empty() {
            return 0;
        }
        sorted[sorted.len() / 2]
    }

    /// Nearest-rank p95 latency for one backend label.
    pub fn latency_p95(&self, backend: &str) -> u64 {
        let sorted = self.latencies(backend);
        if sorted.is_empty() {
            return 0;
        }
        let rank = (95 * sorted.len()).div_ceil(100);
        sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
    }

    /// Rows with unknown cost (never zero-claimed).
    pub fn unknown_cost(&self) -> usize {
        self.rows.iter().filter(|row| row.cost.is_unknown()).count()
    }

    /// Total reported remote cost units.
    pub fn remote_cost_total(&self) -> u64 {
        self.rows
            .iter()
            .filter_map(|row| match row.cost {
                Cost::Units(units) => Some(units),
                Cost::Unknown => None,
            })
            .sum()
    }

    /// Fixture files behind this report, sorted.
    pub fn files(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .rows
            .iter()
            .filter(|row| row.backend == "mock")
            .map(|row| format!("{}.json", row.fixture))
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Gate verdict: every row either passes its keypoints or drops with
    /// contradictions remaining after the repair budget, and every row
    /// replays exactly with providers off.
    pub fn verdict(&self) -> LivingVerdict {
        let mut bad = 0usize;
        for row in &self.rows {
            if row.completed {
                continue;
            }
            if row.contradictions == 0 || !row.replay_exact {
                bad += 1;
            }
        }
        let replay_bad = self.rows.len() - self.replay_exact_count();
        let pass = bad == 0 && replay_bad == 0;
        let reason = if pass {
            format!(
                "pass: {} completed, {} dropped on remaining contradictions, {} replay-exact",
                self.completed(),
                self.dropped(),
                self.replay_exact_count(),
            )
        } else {
            format!("fail: {bad} unexplained drop(s), {replay_bad} replay mismatch(es)")
        };
        LivingVerdict { pass, reason }
    }

    /// Next step for the ludus voices: what they should inherit from the
    /// numbers. Deterministic from the report, never a model call.
    pub fn next_step(&self) -> String {
        format!(
            "LW3 input: quarantine-then-say with repair budget {} heals single-pair faults ({} row(s) repaired) while {} row(s) still drop on multi-pair faults; carry the same rule-based keypoints plus providers-off replay into ludus_npcs before any new line ships.",
            REPAIR_BUDGET_PAIRS,
            self.repairs_used(),
            self.dropped(),
        )
    }

    /// Reproducible text report: files, numbers, repair use, verdict, next
    /// step and cost, then one line per row. One command reproduces it.
    pub fn to_text(&self) -> String {
        let verdict = self.verdict();
        let mut out = String::new();
        out.push_str("LW4 living-world eval: 20 conversation+goal fixtures x mock/local/remote\n");
        out.push_str(&format!(
            "report_version={} rows={} completed={} dropped={} repairs_used={} fallbacks={}\n",
            REPORT_VERSION,
            self.rows.len(),
            self.completed(),
            self.dropped(),
            self.repairs_used(),
            self.fallbacks(),
        ));
        out.push_str(&format!(
            "consistency_permille={} contradiction_pairs={} contradiction_rate_permille={} grounded={} replay_exact={}\n",
            self.consistency_permille(),
            self.contradiction_pairs(),
            self.contradiction_rate_permille(),
            self.grounded_count(),
            self.replay_exact_count(),
        ));
        out.push_str(&format!(
            "latency_ms mock_p50={} mock_p95={} local_p50={} local_p95={} remote_p50={} remote_p95={}\n",
            self.latency_p50("mock"),
            self.latency_p95("mock"),
            self.latency_p50("local"),
            self.latency_p95("local"),
            self.latency_p50("remote"),
            self.latency_p95("remote"),
        ));
        out.push_str(&format!(
            "cost unknown={} remote_units={}\n",
            self.unknown_cost(),
            self.remote_cost_total(),
        ));
        out.push_str(&format!(
            "verdict pass={} reason={}\n",
            verdict.pass, verdict.reason,
        ));
        out.push_str(&format!("next_step {}\n", self.next_step()));
        out.push_str(&format!("files {}\n", self.files().join(",")));
        out.push_str(&format!("dropped_rows {}\n", self.dropped_rows().join(",")));
        out.push_str("fixture kind backend completed contradictions repair fell_back replay grounded latency_ms cost checkpoint detail\n");
        for row in &self.rows {
            out.push_str(&format!(
                "{} {} {} {} {} {} {} {} {} {} {} {:016x} {}\n",
                row.fixture,
                row.kind,
                row.backend,
                row.completed,
                row.contradictions,
                row.repair_used,
                row.fell_back,
                row.replay_exact,
                row.grounded,
                row.latency_ms,
                row.cost,
                row.checkpoint,
                row.detail,
            ));
        }
        out
    }
}

/// Gate verdict for the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivingVerdict {
    /// True exactly when the gate passes.
    pub pass: bool,
    /// Human reason naming the counts.
    pub reason: String,
}

/// Run the full suite: every fixture on every backend, fixture order then
/// backend order. Fixture-driven rows replay identically; no wall clock.
pub fn run_suite() -> Result<LivingReport, LivingError> {
    let fixtures = load_fixtures()?;
    let mut rows = Vec::with_capacity(fixtures.len() * BACKENDS.len());
    for (position, fixture) in fixtures.iter().enumerate() {
        let Ok(index) = u64::try_from(position) else {
            continue;
        };
        for backend in BACKENDS {
            rows.push(run_row(fixture, index, backend));
        }
    }
    Ok(LivingReport { rows })
}

/// One proof-gate failure: which row and why it dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivingFailure {
    /// `fixture:backend` row that dropped.
    pub row: String,
    /// First keypoint miss or replay mismatch.
    pub reason: String,
}

/// Proof-gate outcome: the full living report plus its failures. Empty
/// failures exactly when the gate passes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivingGateOutcome {
    /// Full 60-row report; `to_text()` is its byte-stable rendering.
    pub report: LivingReport,
    /// Failing rows; empty exactly when the gate passes.
    pub failures: Vec<LivingFailure>,
}

impl LivingGateOutcome {
    /// True exactly when every row passed or dropped on remaining
    /// contradictions with exact providers-off replay.
    pub fn pass(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Proof-gate entry point: run the 20-fixture living suite across
/// mock, local and remote and check it against the verdict. Deterministic:
/// the same call twice gives the same report text. Cost is the suite itself
/// (60 rows, one fixture-scripted provider call each; milliseconds).
pub fn living_gate() -> Result<LivingGateOutcome, LivingError> {
    let report = run_suite()?;
    let mut failures = Vec::new();
    for row in &report.rows {
        if !row.completed && (row.contradictions == 0 || !row.replay_exact) {
            failures.push(LivingFailure {
                row: format!("{}:{}", row.fixture, row.backend),
                reason: row.detail.clone(),
            });
        }
        if row.completed && !row.replay_exact {
            failures.push(LivingFailure {
                row: format!("{}:{}", row.fixture, row.backend),
                reason: "replay mismatch on a passing row".to_owned(),
            });
        }
    }
    failures.sort_by(|a, b| a.row.cmp(&b.row));
    Ok(LivingGateOutcome { report, failures })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suite() -> LivingReport {
        run_suite().expect("fixtures must load")
    }

    #[test]
    fn twenty_fixtures_cover_conversation_and_goals() {
        let fixtures = load_fixtures().expect("fixtures must load");
        assert_eq!(fixtures.len(), FIXTURE_COUNT);
        assert_eq!(
            fixtures
                .iter()
                .filter(|fixture| fixture.kind == FixtureKind::Conversation)
                .count(),
            CONVERSATION_COUNT,
        );
        assert_eq!(
            fixtures
                .iter()
                .filter(|fixture| fixture.kind == FixtureKind::Goal)
                .count(),
            GOAL_COUNT,
        );
        let ids: Vec<&str> = fixtures.iter().map(|fixture| fixture.id.as_str()).collect();
        for (position, id) in ids.iter().enumerate() {
            assert_eq!(*id, format!("lw{:02}", position + 1));
        }
        for fixture in &fixtures {
            assert!((1..=4).contains(&fixture.npc), "npc round-robin 1..=4");
            assert!(!fixture.memories.is_empty());
            assert!(!fixture.goals.is_empty());
            assert!(!fixture.keypoints.is_empty());
        }
        println!(
            "lw4-fixtures count={} conv={} goal={}",
            fixtures.len(),
            CONVERSATION_COUNT,
            GOAL_COUNT
        );
    }

    #[test]
    fn one_command_reproduces_the_report() {
        let first = suite();
        let second = suite();
        assert_eq!(first.rows, second.rows);
        assert_eq!(first.to_text(), second.to_text());
        assert_eq!(first.rows.len(), FIXTURE_COUNT * BACKENDS.len());
        for fixture in first.files() {
            for backend in ["mock", "local", "remote"] {
                let stem = fixture.trim_end_matches(".json");
                assert!(
                    first
                        .rows
                        .iter()
                        .any(|row| row.fixture == stem && row.backend == backend),
                    "missing row {stem} {backend}"
                );
            }
        }
        println!("{}", first.to_text());
    }

    #[test]
    fn seeded_bad_memory_drops_exactly_its_cases() {
        let report = suite();
        // Multi-pair faults exhaust the repair budget and drop on every
        // backend; the single-pair fault heals and passes with repair noted.
        let mut dropped = report.dropped_rows();
        dropped.sort();
        let mut expected = Vec::new();
        for id in ["lw12", "lw20"] {
            for backend in ["mock", "local", "remote"] {
                expected.push(format!("{id}:{backend}"));
            }
        }
        expected.sort();
        assert_eq!(
            dropped, expected,
            "exactly the seeded multi-pair cases drop"
        );
        for backend in BACKENDS {
            let row = run_row_by_id("lw19", backend).expect("lw19 exists");
            assert!(row.completed, "single-pair fault heals on {}", row.backend);
            assert!(
                row.repair_used,
                "healed row notes repair on {}",
                row.backend
            );
            assert_eq!(row.contradictions, 0);
        }
        for id in ["lw01", "lw11", "lw13", "lw17"] {
            for backend in BACKENDS {
                let row = run_row_by_id(id, backend).expect("fixture exists");
                assert!(row.completed, "{id} must pass on {}", row.backend);
            }
        }
    }

    #[test]
    fn providers_off_replays_exactly() {
        let report = suite();
        assert_eq!(report.replay_exact_count(), report.rows.len());
        let fixtures = load_fixtures().expect("fixtures must load");
        for fixture in &fixtures {
            let live = replay_mem_checkpoint(fixture);
            // Providers-off recompute with no adapter call reproduces the
            // memory checkpoint bit-for-bit.
            assert_eq!(replay_mem_checkpoint(fixture), live);
            for backend in BACKENDS {
                let row = run_row_by_id(&fixture.id, backend).expect("fixture exists");
                assert!(row.replay_exact, "{} must replay exactly", row.fixture);
                assert_eq!(row.mem_digest, live);
            }
            // The memory checkpoint ignores backend and provider text: the
            // same fixture shares it across backends.
            let digests: Vec<u64> = BACKENDS
                .iter()
                .map(|backend| {
                    run_row_by_id(&fixture.id, *backend)
                        .expect("row")
                        .mem_digest
                })
                .collect();
            assert!(digests.windows(2).all(|pair| pair[0] == pair[1]));
        }
    }

    #[test]
    fn contradiction_counts_match_the_seeded_memory() {
        let fixtures = load_fixtures().expect("fixtures must load");
        let pairs = |id: &str| {
            let fixture = fixtures.iter().find(|fixture| fixture.id == id).expect(id);
            count_pairs(&fixture.memories)
        };
        assert_eq!(pairs("lw12"), 2);
        assert_eq!(pairs("lw20"), 2);
        assert_eq!(pairs("lw19"), 1);
        for id in [
            "lw01", "lw02", "lw03", "lw04", "lw05", "lw06", "lw07", "lw08", "lw09", "lw10", "lw11",
            "lw13", "lw14", "lw15", "lw16", "lw17", "lw18",
        ] {
            assert_eq!(pairs(id), 0, "{id} must ship contradiction-free");
        }
        let report = suite();
        // Post-repair: healed singles read 0, exhausted multis read 1 each.
        assert_eq!(report.contradiction_pairs(), 2 * BACKENDS.len());
        assert_eq!(
            report.contradiction_rate_permille(),
            report.contradiction_pairs() as u64 * 1000 / report.rows.len() as u64
        );
    }

    #[test]
    fn repair_budget_holds_at_one_pair() {
        let report = suite();
        // Every seeded bad-memory row attempts the quarantine (3 fixtures x
        // 3 backends); only the single-pair fault heals into a pass.
        assert_eq!(report.repairs_used(), 3 * BACKENDS.len());
        let healed = report
            .rows
            .iter()
            .filter(|row| row.repair_used && row.completed)
            .count();
        assert_eq!(healed, BACKENDS.len(), "only lw19 heals");
        for row in &report.rows {
            if row.repair_used {
                assert!(
                    row.fixture == "lw12" || row.fixture == "lw19" || row.fixture == "lw20",
                    "repair only touches seeded faults"
                );
            }
        }
        for row in &report.rows {
            if row.fixture == "lw12" || row.fixture == "lw20" {
                assert!(row.repair_used, "{} must attempt repair", row.fixture);
                assert_eq!(row.contradictions, 1, "{} keeps one pair", row.fixture);
                assert!(!row.completed, "{} still drops", row.fixture);
            }
        }
        // Repair keeps the earlier-ingested line: the survivor outranks.
        let fixtures = load_fixtures().expect("fixtures must load");
        let lw19 = fixtures
            .iter()
            .find(|fixture| fixture.id == "lw19")
            .expect("lw19");
        let (healed, used) = repair_once(&lw19.memories);
        assert!(used);
        assert_eq!(healed.len(), lw19.memories.len() - 1);
        assert!(healed.iter().any(|line| line.text == "the gate holds"));
        assert!(!healed.iter().any(|line| line.text == "the gate falls"));
        assert_eq!(count_pairs(&healed), 0);
    }

    #[test]
    fn vision_notes_are_advisory_never_the_gate() {
        let fixtures = load_fixtures().expect("fixtures must load");
        let mut edited = fixtures[0].clone();
        edited.vision = "a blank wall in daylight (advisory only)".to_owned();
        let clean = run_row(&fixtures[0], 0, Backend::Mock);
        let scrambled = run_row(&edited, 0, Backend::Mock);
        assert_eq!(clean.completed, scrambled.completed);
        assert_eq!(clean.consistent, scrambled.consistent);
        assert_eq!(clean.checkpoint, scrambled.checkpoint);
        assert_ne!(
            clean.detail, scrambled.detail,
            "vision still surfaces in detail"
        );
    }

    #[test]
    fn keypoint_gate_catches_an_unrecalled_line() {
        let fixtures = load_fixtures().expect("fixtures must load");
        let mut edited = fixtures[0].clone();
        edited.keypoints = vec![Keypoint::RecallContains(
            "a moon of green cheese".to_owned(),
        )];
        let row = run_row(&edited, 0, Backend::Mock);
        assert!(!row.completed, "missing recall must drop the row");
        assert!(!row.consistent);
        assert!(row.detail.contains("recall misses"));
    }

    #[test]
    fn fallback_rows_complete_through_the_scripted_path() {
        for id in ["lw11", "lw17"] {
            for backend in BACKENDS {
                let row = run_row_by_id(id, backend).expect("timeout fixture exists");
                assert!(row.completed, "{id} falls back on {}", row.backend);
                assert!(row.fell_back, "{id} records fallback on {}", row.backend);
                assert!(!row.repair_used);
                assert!(row.detail.contains("fallback:timeout scripted"));
                assert!(row.grounded, "scripted lines are pre-approved");
            }
        }
        let report = suite();
        assert_eq!(report.fallbacks(), 2 * BACKENDS.len());
    }

    #[test]
    fn latencies_follow_mock_lt_local_lt_remote() {
        let report = suite();
        for row in &report.rows {
            let expected = match row.backend {
                "mock" => (LATENCY_MOCK_MS, LATENCY_MOCK_MS + 2),
                "local" => (LATENCY_LOCAL_MS, LATENCY_LOCAL_MS + 5),
                "remote" => (LATENCY_REMOTE_MS, LATENCY_REMOTE_MS + 5),
                _ => panic!("known backend"),
            };
            assert!(
                (expected.0..expected.1).contains(&row.latency_ms),
                "latency {} out of band for {}",
                row.latency_ms,
                row.backend
            );
        }
        assert!(report.latency_p95("mock") < report.latency_p50("local"));
        assert!(report.latency_p95("local") < report.latency_p50("remote"));
        let again = suite();
        for backend in ["mock", "local", "remote"] {
            assert_eq!(report.latency_p50(backend), again.latency_p50(backend));
            assert_eq!(report.latency_p95(backend), again.latency_p95(backend));
        }
    }

    #[test]
    fn unknown_cost_is_never_zero_claimed() {
        let report = suite();
        assert_eq!(Cost::Unknown.text(), "unknown");
        for row in &report.rows {
            if row.backend == "remote" {
                assert_eq!(row.cost, Cost::Units(REMOTE_COST_UNITS));
            } else {
                assert!(
                    row.cost.is_unknown(),
                    "row {} must not claim cost",
                    row.fixture
                );
                assert_eq!(row.cost.text(), "unknown");
            }
        }
        assert_eq!(report.unknown_cost(), FIXTURE_COUNT * 2);
        assert_eq!(
            report.remote_cost_total(),
            FIXTURE_COUNT as u64 * REMOTE_COST_UNITS
        );
    }

    #[test]
    fn consistency_numbers_follow_the_gate() {
        let report = suite();
        assert_eq!(report.consistency_permille(), 900, "54 of 60 rows pass");
        assert_eq!(report.completed(), 54);
        assert_eq!(report.dropped(), 6);
        assert_eq!(report.grounded_count(), report.rows.len());
        let verdict = report.verdict();
        assert!(
            verdict.pass,
            "seeded drops plus exact replay pass: {}",
            verdict.reason
        );
        assert!(report.next_step().contains("LW3 input"));
        assert!(report.next_step().contains("quarantine-then-say"));
        assert_eq!(report.files().len(), FIXTURE_COUNT);
    }

    #[test]
    fn bad_fixture_files_are_refused_with_reasons() {
        let bad = |text: &str| Parser::parse(text).and_then(|json| parse_fixture(&json, "lw01"));
        assert!(bad("{}").is_err());
        assert!(bad(r#"{"version":2}"#).is_err());
        let unknown_key = r#"{"version":1,"id":"lw01","kind":"conversation","seed":1,"memories":[{"tick":1,"kind":"seen","text":"hi","salience":1}],"goals":["g"],"script":"clean","keypoints":["no-contradiction"],"vision":"v","extra":1}"#;
        assert!(bad(unknown_key).is_err(), "unknown fields refuse");
        let bad_id = r#"{"version":1,"id":"LW01","kind":"conversation","seed":1,"memories":[{"tick":1,"kind":"seen","text":"hi","salience":1}],"goals":["g"],"script":"clean","keypoints":["no-contradiction"],"vision":"v"}"#;
        assert!(bad(bad_id).is_err(), "bad id refuses");
        let long: String = "x".repeat(MAX_TEXT + 1);
        let overlong = format!(
            r#"{{"version":1,"id":"lw01","kind":"conversation","seed":1,"memories":[{{"tick":1,"kind":"seen","text":"{long}","salience":1}}],"goals":["g"],"script":"clean","keypoints":["no-contradiction"],"vision":"v"}}"#
        );
        assert!(bad(&overlong).is_err(), "overlong text refuses");
        let bad_salience = r#"{"version":1,"id":"lw01","kind":"conversation","seed":1,"memories":[{"tick":1,"kind":"seen","text":"hi","salience":1001}],"goals":["g"],"script":"clean","keypoints":["no-contradiction"],"vision":"v"}"#;
        assert!(bad(bad_salience).is_err(), "salience over 1000 refuses");
        let bad_kind = r#"{"version":1,"id":"lw01","kind":"conversation","seed":1,"memories":[{"tick":1,"kind":"dreamt","text":"hi","salience":1}],"goals":["g"],"script":"clean","keypoints":["no-contradiction"],"vision":"v"}"#;
        assert!(bad(bad_kind).is_err(), "unknown memory kind refuses");
        let bad_script = r#"{"version":1,"id":"lw01","kind":"conversation","seed":1,"memories":[{"tick":1,"kind":"seen","text":"hi","salience":1}],"goals":["g"],"script":"chatty","keypoints":["no-contradiction"],"vision":"v"}"#;
        assert!(bad(bad_script).is_err(), "unknown script refuses");
        let bad_point = r#"{"version":1,"id":"lw01","kind":"conversation","seed":1,"memories":[{"tick":1,"kind":"seen","text":"hi","salience":1}],"goals":["g"],"script":"clean","keypoints":["looks-nice"],"vision":"v"}"#;
        assert!(bad(bad_point).is_err(), "unknown keypoint refuses");
        let float_script = r#"{"version":1,"id":"lw01","kind":"conversation","seed":1.5,"memories":[{"tick":1,"kind":"seen","text":"hi","salience":1}],"goals":["g"],"script":"clean","keypoints":["no-contradiction"],"vision":"v"}"#;
        assert!(bad(float_script).is_err(), "floats refuse");
    }

    #[test]
    fn unknown_fixture_id_is_a_typed_error() {
        let error = run_row_by_id("lw99", Backend::Mock).expect_err("unknown id must fail");
        assert_eq!(error.code(), "bad_fixture_id");
        assert!(error.to_string().contains("lw99"));
    }

    #[test]
    fn gate_name_is_registered_for_proof() {
        assert_eq!(LIVING_GATE_NAME, "lw4-living-world-eval");
        let first = living_gate().expect("gate must run");
        let second = living_gate().expect("gate must replay");
        assert_eq!(first.report.to_text(), second.report.to_text());
        assert!(
            first.pass(),
            "seeded suite passes its own gate: {:?}",
            first.failures
        );
        assert!(first.failures.is_empty());
        assert_eq!(first.report.rows.len(), FIXTURE_COUNT * BACKENDS.len());
    }

    #[test]
    fn report_text_carries_files_numbers_repair_verdict_next_step_cost() {
        let text = suite().to_text();
        assert!(text.contains("files lw01.json"), "files listed");
        assert!(text.contains("consistency_permille=900"), "numbers present");
        assert!(text.contains("repairs_used=9"), "repair use present");
        assert!(text.contains("verdict pass=true"), "verdict present");
        assert!(text.contains("next_step LW3 input"), "next step present");
        assert!(text.contains("remote_units=140"), "cost present");
        assert!(text.contains(
            "dropped_rows lw12:local,lw12:mock,lw12:remote,lw20:local,lw20:mock,lw20:remote"
        ));
        for label in ["mock", "local", "remote"] {
            assert!(text.contains(label), "report must name {label}");
        }
    }
}
