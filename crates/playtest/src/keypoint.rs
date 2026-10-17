//! Keypoint verifier: gameplay claims as versioned data specs.
//!
//! Consumer: the proof harness runs [`run_suite`]; future `sim.verify` maps
//! one spec to one tool call. Data owner: the caller owns spec files; this
//! module borrows tapes and owns reports. Units: fixed ticks, hp and damage
//! in permille, integer pixel bytes in captures. Determinism scope:
//! same-machine reproducibility — the same spec files twice give the
//! identical [`SuiteReport`]; digests hash integers only, never floats.
//! Errors: bad specs fail as [`KeypointError`]; gameplay mismatches are
//! [`KeypointFailure`] strings inside a passing report, never panics.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use engine_arena::fork::{Fork, ForkProposal};
use engine_arena::recorder::{FighterEntries, ForkObservation};
use engine_arena::{run_dataset, run_prefix, ArenaConfig, FighterInput, FighterTape};
use engine_core::hash::FNV64_OFFSET;

use crate::causal::Cause;
use crate::frame_check::digest_hex;
use crate::parse::{find_entry as field, Cursor, ParseError};

/// Spec format version this runner accepts.
pub const SPEC_VERSION: u16 = 1;
/// Gameplay keypoints shipped in `fixtures/keypoints/`.
pub const KEYPOINT_COUNT: usize = 30;
/// Seeded gameplay bugs paired with their keypoints.
pub const BUG_COUNT: usize = 5;
/// Largest accepted spec file in bytes.
pub const MAX_SPEC_BYTES: usize = 64 * 1024;
/// Largest accepted spec count in one directory.
pub const MAX_SPECS: usize = 64;
/// Longest setup prefix in fixed ticks.
pub const MAX_PREFIX_TICKS: u32 = 240;
/// Longest scripted window in fixed ticks.
pub const MAX_WINDOW_TICKS: u32 = 120;
/// Longest claim string in chars.
pub const MAX_CLAIM_CHARS: usize = 280;
/// Capture edge in pixels; one deterministic raster per keypoint.
pub const CAPTURE_EDGE: u32 = 64;
/// Whole-suite simulated-tick budget (deterministic, no wall clock inside).
pub const SUITE_TICK_BUDGET: u64 = 20_000;
/// Report format version stamped on every suite report.
pub const REPORT_VERSION: u16 = 1;

// ---- strict JSON subset ---------------------------------------------------

/// Strict JSON value: objects keep insertion order, numbers stay split so
/// integer fields never pass through a float.
#[derive(Clone, PartialEq, Debug)]
enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
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
        let value = parser.value(0)?;
        parser.cursor.ws();
        parser.cursor.finish()?;
        Ok(value)
    }

    fn value(&mut self, depth: usize) -> Result<Json, ParseError> {
        if depth > 16 {
            return Err(ParseError::NestingTooDeep {
                pos: self.cursor.pos(),
            });
        }
        match self.cursor.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => {
                self.cursor.lit("true")?;
                Ok(Json::Bool(true))
            }
            Some(b'f') => {
                self.cursor.lit("false")?;
                Ok(Json::Bool(false))
            }
            Some(b'n') => {
                self.cursor.lit("null")?;
                Ok(Json::Null)
            }
            Some(byte) if byte == b'-' || byte.is_ascii_digit() => self.number(),
            Some(byte) => Err(ParseError::UnexpectedByte {
                pos: self.cursor.pos(),
                byte,
            }),
            None => Err(ParseError::EndOfInput {
                pos: self.cursor.pos(),
            }),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, ParseError> {
        self.cursor.bump();
        let mut entries = Vec::new();
        self.cursor.ws();
        if self.cursor.eat(b'}') {
            return Ok(Json::Obj(entries));
        }
        loop {
            self.cursor.ws();
            if self.cursor.peek() != Some(b'"') {
                return Err(ParseError::ExpectedKey {
                    pos: self.cursor.pos(),
                });
            }
            let key = self.string()?;
            self.cursor.ws();
            if !self.cursor.eat(b':') {
                return Err(ParseError::MissingColon {
                    pos: self.cursor.pos(),
                });
            }
            self.cursor.ws();
            let value = self.value(depth + 1)?;
            if entries.iter().any(|(known, _)| *known == key) {
                return Err(ParseError::DuplicateKey { key });
            }
            entries.push((key, value));
            self.cursor.ws();
            if self.cursor.eat(b'}') {
                return Ok(Json::Obj(entries));
            }
            if !self.cursor.eat(b',') {
                return Err(ParseError::MissingComma {
                    pos: self.cursor.pos(),
                });
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, ParseError> {
        self.cursor.bump();
        let mut items = Vec::new();
        self.cursor.ws();
        if self.cursor.eat(b']') {
            return Ok(Json::Arr(items));
        }
        loop {
            self.cursor.ws();
            items.push(self.value(depth + 1)?);
            if items.len() > 8192 {
                return Err(ParseError::ArrayTooLong);
            }
            self.cursor.ws();
            if self.cursor.eat(b']') {
                return Ok(Json::Arr(items));
            }
            if !self.cursor.eat(b',') {
                return Err(ParseError::MissingComma {
                    pos: self.cursor.pos(),
                });
            }
        }
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.cursor.bump();
        let mut out = String::new();
        loop {
            let Some(byte) = self.cursor.peek() else {
                return Err(ParseError::UnterminatedString {
                    pos: self.cursor.pos(),
                });
            };
            match byte {
                b'"' => {
                    self.cursor.bump();
                    return Ok(out);
                }
                b'\\' => {
                    self.cursor.bump();
                    match self.cursor.peek() {
                        Some(b'"') => out.push('"'),
                        Some(b'\\') => out.push('\\'),
                        Some(b'/') => out.push('/'),
                        Some(b'b') => out.push('\u{0008}'),
                        Some(b'f') => out.push('\u{000C}'),
                        Some(b'n') => out.push('\n'),
                        Some(b'r') => out.push('\r'),
                        Some(b't') => out.push('\t'),
                        Some(b'u') => out.push(self.unicode()?),
                        Some(other) => {
                            return Err(ParseError::BadEscape {
                                pos: self.cursor.pos(),
                                byte: Some(other),
                            });
                        }
                        None => {
                            return Err(ParseError::UnterminatedEscape {
                                pos: self.cursor.pos(),
                            });
                        }
                    }
                    if self.cursor.peek().is_some() {
                        self.cursor.bump();
                    }
                }
                0x00..=0x1F => {
                    return Err(ParseError::ControlByte {
                        pos: self.cursor.pos(),
                    });
                }
                _ => {
                    let rest = self.cursor.rest();
                    let text = core::str::from_utf8(rest).map_err(|_| ParseError::BadUtf8 {
                        pos: self.cursor.pos(),
                    })?;
                    let Some(next) = text.chars().next() else {
                        return Err(ParseError::BadUtf8 {
                            pos: self.cursor.pos(),
                        });
                    };
                    out.push(next);
                    self.cursor.advance(next.len_utf8());
                }
            }
        }
    }

    fn unicode(&mut self) -> Result<char, ParseError> {
        self.cursor.bump();
        if self.cursor.len() - self.cursor.pos() < 4 {
            return Err(ParseError::ShortUnicode {
                pos: self.cursor.pos(),
            });
        }
        let mut unit: u32 = 0;
        for offset in 0..4 {
            let Some(digit) = self.cursor.ahead(offset) else {
                return Err(ParseError::ShortUnicode {
                    pos: self.cursor.pos(),
                });
            };
            let nibble = match digit {
                b'0'..=b'9' => u32::from(digit - b'0'),
                b'a'..=b'f' => u32::from(digit - b'a') + 10,
                b'A'..=b'F' => u32::from(digit - b'A') + 10,
                _ => {
                    return Err(ParseError::BadHex {
                        pos: self.cursor.pos() + offset,
                    });
                }
            };
            unit = unit * 16 + nibble;
        }
        self.cursor.advance(3);
        char::from_u32(unit).ok_or(ParseError::BadScalar { unit })
    }

    fn number(&mut self) -> Result<Json, ParseError> {
        let start = self.cursor.pos();
        if self.cursor.peek() == Some(b'-') {
            self.cursor.bump();
        }
        let digits = self.cursor.pos();
        while self.cursor.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.cursor.bump();
        }
        if self.cursor.pos() == digits {
            return Err(ParseError::BadNumber { pos: start });
        }
        let mut is_float = false;
        if self.cursor.peek() == Some(b'.') {
            is_float = true;
            self.cursor.bump();
            let frac = self.cursor.pos();
            while self.cursor.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.cursor.bump();
            }
            if self.cursor.pos() == frac {
                return Err(ParseError::BadFraction { pos: start });
            }
        }
        if matches!(self.cursor.peek(), Some(b'e' | b'E')) {
            is_float = true;
            self.cursor.bump();
            if matches!(self.cursor.peek(), Some(b'+' | b'-')) {
                self.cursor.bump();
            }
            let exp = self.cursor.pos();
            while self.cursor.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.cursor.bump();
            }
            if self.cursor.pos() == exp {
                return Err(ParseError::BadExponent { pos: start });
            }
        }
        let token = core::str::from_utf8(self.cursor.lexeme(start))
            .map_err(|_| ParseError::BadNumber { pos: start })?;
        if is_float {
            let value: f64 = token.parse().map_err(|_| ParseError::BadToken {
                token: token.to_owned(),
            })?;
            if !value.is_finite() {
                return Err(ParseError::NonFinite {
                    token: token.to_owned(),
                });
            }
            Ok(Json::Float(value))
        } else {
            let value: i64 = token.parse().map_err(|_| ParseError::BadToken {
                token: token.to_owned(),
            })?;
            Ok(Json::Int(value))
        }
    }
}

// ---- spec accessors -------------------------------------------------------

fn as_obj(value: &Json) -> Result<&[(String, Json)], ParseError> {
    match value {
        Json::Obj(entries) => Ok(entries),
        _ => Err(ParseError::ExpectedObject),
    }
}

fn optional<'j>(entries: &'j [(String, Json)], key: &str) -> Result<Option<&'j Json>, ParseError> {
    Ok(entries
        .iter()
        .find(|(known, _)| known == key)
        .map(|(_, value)| value))
}

fn as_i64(value: &Json, key: &str) -> Result<i64, ParseError> {
    match value {
        Json::Int(number) => Ok(*number),
        _ => Err(ParseError::WrongType {
            field: key.to_owned(),
            want: "an integer",
        }),
    }
}

fn as_u32(value: &Json, key: &str, max: u32) -> Result<u32, ParseError> {
    let number = as_i64(value, key)?;
    if number < 0 || u64::try_from(number).map_or(true, |wide| wide > u64::from(max)) {
        return Err(ParseError::OutOfRange {
            what: format!("field `{key}` out of range 0..={max}"),
        });
    }
    let narrowed = u32::try_from(number).map_err(|_| ParseError::OutOfRange {
        what: format!("field `{key}` out of range 0..={max}"),
    })?;
    Ok(narrowed)
}

fn as_bool(value: &Json, key: &str) -> Result<bool, ParseError> {
    match value {
        Json::Bool(flag) => Ok(*flag),
        _ => Err(ParseError::WrongType {
            field: key.to_owned(),
            want: "a boolean",
        }),
    }
}

fn as_str<'j>(value: &'j Json, key: &str) -> Result<&'j str, ParseError> {
    match value {
        Json::Str(text) => Ok(text),
        _ => Err(ParseError::WrongType {
            field: key.to_owned(),
            want: "a string",
        }),
    }
}

fn as_arr<'j>(value: &'j Json, key: &str) -> Result<&'j [Json], ParseError> {
    match value {
        Json::Arr(items) => Ok(items),
        _ => Err(ParseError::WrongType {
            field: key.to_owned(),
            want: "an array",
        }),
    }
}

// ---- spec types -----------------------------------------------------------

/// One scripted input: an explicit per-tick override. Unlisted
/// `(tick, fighter)` slots run idle, so specs stay short and explicit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ScriptInput {
    /// Absolute match tick the input applies before.
    pub tick: u32,
    /// Fighter slot, `0..fighters`.
    pub fighter: u8,
    /// Forward wish, `-100..=100`.
    pub mx: i8,
    /// Right wish, `-100..=100`.
    pub mz: i8,
    /// Facing in centidegrees, `-18000..=18000`.
    pub facing: i16,
    /// Button byte; the fighter contract accepts the three low bits.
    pub buttons: u8,
}

/// Typed assertions over the scripted window. Every field is optional;
///
/// a missing field is unchecked, so each keypoint asserts only its claim.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct KeypointAsserts {
    /// Final health per slot in permille.
    pub hp: Vec<(u8, u16)>,
    /// Window damage dealt per slot in permille.
    pub damage: Vec<(u8, u16)>,
    /// Window stuck ticks per slot.
    pub stuck: Vec<(u8, u32)>,
    /// True when any fighter is knocked out inside the window.
    pub killed: Option<bool>,
    /// Window-relative ticks with fresh damage, exact.
    pub damage_ticks: Option<Vec<u32>>,
    /// Fork-relative kill tick (1-based), `None` when nobody drops.
    pub ticks_to_kill: Option<Option<u32>>,
}

/// One gameplay claim as data: a setup prefix that injects mid-match state
/// into a fork, then a bounded scripted window with typed assertions.
#[derive(Clone, PartialEq, Debug)]
pub struct KeypointSpec {
    /// File stem and registry id, e.g. `kp06-parry-riposte`.
    pub id: String,
    /// Human claim under test, for reasons only.
    pub claim: String,
    /// Match seed; spawn and prefix trajectories derive from it.
    pub seed: u64,
    /// Fighter count, `2..=MAX_PARTICIPANTS`.
    pub fighters: u8,
    /// Arena half-extent in meters, `5.0..=100.0`.
    pub half_extent_m: f32,
    /// Setup prefix length in ticks; the fork starts here.
    pub prefix_ticks: u32,
    /// Setup inputs over `0..prefix_ticks`, absolute ticks.
    pub setup: Vec<ScriptInput>,
    /// Scripted window length in ticks.
    pub window_ticks: u32,
    /// Window inputs over `prefix..prefix+window`, absolute ticks.
    pub window: Vec<ScriptInput>,
    /// Slots frozen for the whole window via a fork proposal.
    pub freeze: Vec<u8>,
    /// Typed assertions over the window.
    pub asserts: KeypointAsserts,
}

/// Keypoint failures: bad specs fail typed, never partial and never panics.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum KeypointError {
    /// The spec directory is missing or unreadable.
    DirUnreadable(String),
    /// Too many spec files; the bound carried.
    TooManySpecs(usize),
    /// One file failed to parse or validate; file stem plus reason.
    BadSpec {
        /// File stem of the offending spec.
        file: String,
        /// What was wrong with it.
        reason: String,
    },
    /// The arena refused the built tapes; spec id carried.
    ArenaRefused(String),
}

impl core::fmt::Display for KeypointError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DirUnreadable(reason) => write!(f, "keypoint dir unreadable: {reason}"),
            Self::TooManySpecs(count) => write!(f, "too many specs: {count}"),
            Self::BadSpec { file, reason } => write!(f, "bad spec `{file}`: {reason}"),
            Self::ArenaRefused(id) => write!(f, "arena refused keypoint `{id}`"),
        }
    }
}

impl std::error::Error for KeypointError {}

// ---- spec loading ---------------------------------------------------------

/// Directory holding the versioned keypoint specs, resolved from the crate
/// location so the suite runs from any working directory.
pub fn spec_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/keypoints")
}

/// Load every `*.json` spec in [`spec_dir`], sorted by file stem. Strict:
/// unknown fields, duplicate keys and out-of-range values all fail.
pub fn load_specs() -> Result<Vec<KeypointSpec>, KeypointError> {
    let dir = spec_dir();
    let entries =
        std::fs::read_dir(&dir).map_err(|error| KeypointError::DirUnreadable(error.to_string()))?;
    let mut stems = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| KeypointError::DirUnreadable(error.to_string()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".json") {
            stems.push(name);
        }
    }
    stems.sort();
    if stems.len() > MAX_SPECS {
        return Err(KeypointError::TooManySpecs(stems.len()));
    }
    let mut specs = Vec::with_capacity(stems.len());
    for name in &stems {
        let stem = name.trim_end_matches(".json").to_owned();
        let bytes = std::fs::read(dir.join(name)).map_err(|error| KeypointError::BadSpec {
            file: stem.clone(),
            reason: error.to_string(),
        })?;
        if bytes.len() > MAX_SPEC_BYTES {
            return Err(KeypointError::BadSpec {
                file: stem.clone(),
                reason: format!("file over {MAX_SPEC_BYTES} bytes"),
            });
        }
        let text = String::from_utf8(bytes).map_err(|error| KeypointError::BadSpec {
            file: stem.clone(),
            reason: error.to_string(),
        })?;
        let parsed = Parser::parse(&text).map_err(|error| KeypointError::BadSpec {
            file: stem.clone(),
            reason: error.to_string(),
        })?;
        let spec = parse_spec(&parsed, &stem).map_err(|error| KeypointError::BadSpec {
            file: stem.clone(),
            reason: error.to_string(),
        })?;
        specs.push(spec);
    }
    let mut ids: Vec<&str> = specs.iter().map(|spec| spec.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() != specs.len() {
        return Err(KeypointError::BadSpec {
            file: String::new(),
            reason: "duplicate keypoint id".to_owned(),
        });
    }
    Ok(specs)
}

/// Scalar header of one spec: identity, seed, shape and phase lengths. Split
/// out of [`parse_spec`] so each fn stays a named step.
struct SpecHead {
    id: String,
    claim: String,
    seed: u64,
    fighters: u8,
    half_extent_m: f32,
    prefix_ticks: u32,
    window_ticks: u32,
}

fn parse_spec_head(entries: &[(String, Json)], stem: &str) -> Result<SpecHead, ParseError> {
    let version = field(entries, "version")
        .and_then(|value| as_u32(value, "version", u32::from(u16::MAX)))?;
    if version != u32::from(SPEC_VERSION) {
        return Err(ParseError::Mismatch {
            what: format!("unsupported spec version {version}"),
        });
    }
    let id = field(entries, "id").and_then(|value| as_str(value, "id"))?;
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
    let claim = field(entries, "claim").and_then(|value| as_str(value, "claim"))?;
    if claim.is_empty() || claim.chars().count() > MAX_CLAIM_CHARS {
        return Err(ParseError::OutOfRange {
            what: "claim length out of range".to_owned(),
        });
    }
    let seed = field(entries, "seed").and_then(|value| as_i64(value, "seed"))?;
    if seed < 0 {
        return Err(ParseError::OutOfRange {
            what: "seed must be non-negative".to_owned(),
        });
    }
    let fighters = field(entries, "fighters").and_then(|value| as_u32(value, "fighters", 255))?;
    if fighters < 2 || fighters > u32::from(engine_arena::MAX_PARTICIPANTS) {
        return Err(ParseError::OutOfRange {
            what: format!("fighters {fighters} outside 2..=4"),
        });
    }
    let half_extent_m = field(entries, "half_extent_m").and_then(|value| match value {
        Json::Int(number) => {
            i32::try_from(*number)
                .map(f64::from)
                .map_err(|_| ParseError::WrongType {
                    field: "half_extent_m".to_owned(),
                    want: "a number",
                })
        }
        Json::Float(number) => Ok(*number),
        _ => Err(ParseError::WrongType {
            field: "half_extent_m".to_owned(),
            want: "a number",
        }),
    })?;
    if !(5.0..=100.0).contains(&half_extent_m) {
        return Err(ParseError::OutOfRange {
            what: "half_extent_m outside 5.0..=100.0".to_owned(),
        });
    }
    let prefix_ticks = field(entries, "prefix_ticks")
        .and_then(|value| as_u32(value, "prefix_ticks", MAX_PREFIX_TICKS))?;
    let window_ticks = field(entries, "window_ticks")
        .and_then(|value| as_u32(value, "window_ticks", MAX_WINDOW_TICKS))?;
    if window_ticks == 0 {
        return Err(ParseError::OutOfRange {
            what: "window_ticks must be positive".to_owned(),
        });
    }
    Ok(SpecHead {
        id: id.to_owned(),
        claim: claim.to_owned(),
        seed: u64::try_from(seed).map_err(|_| ParseError::OutOfRange {
            what: "seed must be non-negative".to_owned(),
        })?,
        fighters: u8::try_from(fighters).map_err(|_| ParseError::OutOfRange {
            what: format!("fighters {fighters} outside 2..=4"),
        })?,
        // Range-checked to 5.0..=100.0 above, so the narrowing stays finite
        // and deterministic.
        half_extent_m: half_extent_m as f32,
        prefix_ticks,
        window_ticks,
    })
}

fn parse_spec(value: &Json, stem: &str) -> Result<KeypointSpec, ParseError> {
    let entries = as_obj(value)?;
    let head = parse_spec_head(entries, stem)?;
    let setup = field(entries, "setup").and_then(|value| as_arr(value, "setup"))?;
    let setup = parse_inputs(setup, 0, head.prefix_ticks, head.fighters, "setup")
        .map_err(|error| ParseError::context("setup", error))?;
    let window = field(entries, "window").and_then(|value| as_arr(value, "window"))?;
    let window = parse_inputs(
        window,
        head.prefix_ticks,
        head.prefix_ticks.saturating_add(head.window_ticks),
        head.fighters,
        "window",
    )
    .map_err(|error| ParseError::context("window", error))?;
    let freeze = match optional(entries, "freeze")? {
        None => Vec::new(),
        Some(value) => {
            let items = as_arr(value, "freeze")?;
            parse_slots(items)?
        }
    };
    if freeze.iter().any(|slot| *slot >= head.fighters) {
        return Err(ParseError::OutOfRange {
            what: "freeze slot outside fighters".to_owned(),
        });
    }
    let asserts = match optional(entries, "assert")? {
        None => KeypointAsserts::default(),
        Some(value) => parse_asserts(value, head.fighters, head.window_ticks)
            .map_err(|error| ParseError::context("assert", error))?,
    };
    if entries.iter().any(|(key, _)| {
        ![
            "version",
            "id",
            "claim",
            "seed",
            "fighters",
            "half_extent_m",
            "prefix_ticks",
            "window_ticks",
            "setup",
            "window",
            "freeze",
            "assert",
        ]
        .contains(&key.as_str())
    }) {
        return Err(ParseError::Unknown {
            what: "unknown field".to_owned(),
        });
    }
    Ok(KeypointSpec {
        id: head.id,
        claim: head.claim,
        seed: head.seed,
        fighters: head.fighters,
        half_extent_m: head.half_extent_m,
        prefix_ticks: head.prefix_ticks,
        setup,
        window_ticks: head.window_ticks,
        window,
        freeze,
        asserts,
    })
}

fn parse_slots(items: &[Json]) -> Result<Vec<u8>, ParseError> {
    let mut slots = Vec::new();
    for item in items {
        let slot = as_u32(item, "freeze slot", 255)?;
        let slot = u8::try_from(slot).map_err(|_| ParseError::OutOfRange {
            what: "freeze slot out of range".to_owned(),
        })?;
        if slots.contains(&slot) {
            return Err(ParseError::Mismatch {
                what: "duplicate freeze slot".to_owned(),
            });
        }
        slots.push(slot);
    }
    Ok(slots)
}

/// One validated script entry: fighter, wishes, facing and buttons. Split out
/// of [`parse_inputs`] so each fn stays a named step.
struct ScriptShape {
    fighter: u8,
    mx: i8,
    mz: i8,
    facing: i16,
    buttons: u8,
}

fn parse_script_shape(entries: &[(String, Json)], fighters: u8) -> Result<ScriptShape, ParseError> {
    let fighter = field(entries, "fighter").and_then(|value| as_u32(value, "fighter", 255))?;
    if fighter >= u32::from(fighters) {
        return Err(ParseError::OutOfRange {
            what: format!("fighter {fighter} outside slots"),
        });
    }
    let mx = field(entries, "mx").and_then(|value| as_i64(value, "mx"))?;
    let mz = field(entries, "mz").and_then(|value| as_i64(value, "mz"))?;
    if !(-100..=100).contains(&mx) || !(-100..=100).contains(&mz) {
        return Err(ParseError::OutOfRange {
            what: "wish outside -100..=100".to_owned(),
        });
    }
    let facing = field(entries, "facing").and_then(|value| as_i64(value, "facing"))?;
    if !(-18_000..=18_000).contains(&facing) {
        return Err(ParseError::OutOfRange {
            what: "facing outside -18000..=18000".to_owned(),
        });
    }
    let buttons = field(entries, "buttons").and_then(|value| as_u32(value, "buttons", 255))?;
    if buttons > 7 {
        return Err(ParseError::OutOfRange {
            what: format!("buttons {buttons} use unknown bits"),
        });
    }
    let narrow_fighter = u8::try_from(fighter).map_err(|_| ParseError::OutOfRange {
        what: format!("fighter {fighter} outside slots"),
    })?;
    let narrow_mx = i8::try_from(mx).map_err(|_| ParseError::OutOfRange {
        what: "wish outside -100..=100".to_owned(),
    })?;
    let narrow_mz = i8::try_from(mz).map_err(|_| ParseError::OutOfRange {
        what: "wish outside -100..=100".to_owned(),
    })?;
    let narrow_facing = i16::try_from(facing).map_err(|_| ParseError::OutOfRange {
        what: "facing outside -18000..=18000".to_owned(),
    })?;
    let narrow_buttons = u8::try_from(buttons).map_err(|_| ParseError::OutOfRange {
        what: format!("buttons {buttons} use unknown bits"),
    })?;
    Ok(ScriptShape {
        fighter: narrow_fighter,
        mx: narrow_mx,
        mz: narrow_mz,
        facing: narrow_facing,
        buttons: narrow_buttons,
    })
}

fn push_script_tick(
    out: &mut Vec<ScriptInput>,
    shape: &ScriptShape,
    tick: u32,
    lower: u32,
    upper: u32,
) -> Result<(), ParseError> {
    if tick < lower || tick >= upper {
        return Err(ParseError::OutOfRange {
            what: format!("tick {tick} outside phase"),
        });
    }
    out.push(ScriptInput {
        tick,
        fighter: shape.fighter,
        mx: shape.mx,
        mz: shape.mz,
        facing: shape.facing,
        buttons: shape.buttons,
    });
    if out.len() > 4096 {
        return Err(ParseError::OutOfRange {
            what: "too many inputs".to_owned(),
        });
    }
    Ok(())
}

/// Expand one script entry: a single `tick` or an inclusive `from`/`to` hold.
fn expand_script_entry(
    entries: &[(String, Json)],
    shape: &ScriptShape,
    lower: u32,
    upper: u32,
    out: &mut Vec<ScriptInput>,
) -> Result<(), ParseError> {
    match (optional(entries, "tick")?, optional(entries, "from")?) {
        (Some(tick), None) => {
            let tick = as_u32(tick, "tick", u32::MAX)?;
            push_script_tick(out, shape, tick, lower, upper)?;
        }
        (None, Some(from)) => {
            let to = field(entries, "to")?;
            let from = as_u32(from, "from", u32::MAX)?;
            let to = as_u32(to, "to", u32::MAX)?;
            if from > to {
                return Err(ParseError::OutOfRange {
                    what: format!("hold from {from} past to {to}"),
                });
            }
            if to - from > 512 {
                return Err(ParseError::OutOfRange {
                    what: "hold over 512 ticks".to_owned(),
                });
            }
            for tick in from..=to {
                push_script_tick(out, shape, tick, lower, upper)?;
            }
        }
        _ => {
            return Err(ParseError::Mismatch {
                what: "entry needs `tick` or `from`+`to`".to_owned(),
            });
        }
    }
    if entries.iter().any(|(key, _)| {
        ![
            "fighter", "mx", "mz", "facing", "buttons", "tick", "from", "to",
        ]
        .contains(&key.as_str())
    }) {
        return Err(ParseError::Unknown {
            what: "unknown entry field".to_owned(),
        });
    }
    Ok(())
}

/// Parse one script phase: single-tick entries carry `tick`, hold entries
/// carry inclusive `from`/`to`. Bounds are phase-relative so a window entry
/// can never name a setup tick.
fn parse_inputs(
    items: &[Json],
    lower: u32,
    upper: u32,
    fighters: u8,
    phase: &str,
) -> Result<Vec<ScriptInput>, ParseError> {
    let mut out = Vec::new();
    for item in items {
        let entries = as_obj(item).map_err(|error| ParseError::context(phase, error))?;
        let shape = parse_script_shape(entries, fighters)
            .map_err(|error| ParseError::context(phase, error))?;
        expand_script_entry(entries, &shape, lower, upper, &mut out)
            .map_err(|error| ParseError::context(phase, error))?;
    }
    out.sort_by_key(|input| (input.tick, input.fighter));
    let mut seen = BTreeMap::new();
    for input in &out {
        if seen.insert((input.tick, input.fighter), true).is_some() {
            return Err(ParseError::context(
                phase,
                ParseError::Mismatch {
                    what: format!(
                        "duplicate input at tick {} fighter {}",
                        input.tick, input.fighter
                    ),
                },
            ));
        }
    }
    Ok(out)
}

fn parse_pairs(items: &[Json], key: &str, max: u32) -> Result<Vec<(u8, u32)>, ParseError> {
    let mut out = Vec::new();
    for item in items {
        let pair = as_arr(item, key).map_err(|error| ParseError::context(key, error))?;
        if pair.len() != 2 {
            return Err(ParseError::Mismatch {
                what: format!("{key}: pair must have two entries"),
            });
        }
        let slot = as_u32(&pair[0], key, 255)?;
        let value = as_u32(&pair[1], key, max)?;
        let slot = u8::try_from(slot).map_err(|_| ParseError::OutOfRange {
            what: format!("{key}: slot out of range"),
        })?;
        if out.iter().any(|(known, _)| *known == slot) {
            return Err(ParseError::Mismatch {
                what: format!("{key}: duplicate slot {slot}"),
            });
        }
        out.push((slot, value));
    }
    out.sort_by_key(|(slot, _)| *slot);
    Ok(out)
}

/// One checked `(slot, value)` assert list: optional field, pair shape and
/// fighter range in one step. Split out of [`parse_asserts`].
fn checked_pairs(
    entries: &[(String, Json)],
    key: &str,
    fighters: u8,
    max: u32,
) -> Result<Vec<(u8, u32)>, ParseError> {
    let Some(value) = optional(entries, key)? else {
        return Ok(Vec::new());
    };
    let items = as_arr(value, key).map_err(|error| ParseError::context("assert", error))?;
    let pairs =
        parse_pairs(items, key, max).map_err(|error| ParseError::context("assert", error))?;
    for (slot, _) in &pairs {
        if *slot >= fighters {
            return Err(ParseError::OutOfRange {
                what: format!("assert {key} slot outside fighters"),
            });
        }
    }
    Ok(pairs)
}

/// Narrow one `(slot, value)` list to `u16` values for the report.
fn narrow_pairs(pairs: &[(u8, u32)], key: &str) -> Result<Vec<(u8, u16)>, ParseError> {
    let mut out = Vec::with_capacity(pairs.len());
    for (slot, value) in pairs {
        let narrowed = u16::try_from(*value).map_err(|_| ParseError::OutOfRange {
            what: format!("assert {key} value {value} outside u16"),
        })?;
        out.push((*slot, narrowed));
    }
    Ok(out)
}

/// Parse the window-relative damage ticks, exact and in range.
fn parse_damage_ticks(items: &[Json], window_ticks: u32) -> Result<Vec<u32>, ParseError> {
    let mut list = Vec::with_capacity(items.len());
    for item in items {
        let tick = as_u32(item, "damage_ticks", u32::MAX)?;
        if tick >= window_ticks {
            return Err(ParseError::OutOfRange {
                what: "assert damage tick outside window".to_owned(),
            });
        }
        if list.contains(&tick) {
            return Err(ParseError::Mismatch {
                what: "assert duplicate damage tick".to_owned(),
            });
        }
        list.push(tick);
    }
    list.sort_unstable();
    Ok(list)
}

fn parse_asserts(
    value: &Json,
    fighters: u8,
    window_ticks: u32,
) -> Result<KeypointAsserts, ParseError> {
    let entries = as_obj(value)?;
    let hp = narrow_pairs(&checked_pairs(entries, "hp", fighters, 1000)?, "hp")?;
    let damage = narrow_pairs(
        &checked_pairs(entries, "damage", fighters, 65_535)?,
        "damage",
    )?;
    let stuck = checked_pairs(entries, "stuck", fighters, u32::MAX)?;
    let mut asserts = KeypointAsserts {
        hp,
        damage,
        stuck,
        ..KeypointAsserts::default()
    };
    if let Some(killed) = optional(entries, "killed")? {
        asserts.killed = Some(as_bool(killed, "killed")?);
    }
    if let Some(ticks) = optional(entries, "damage_ticks")? {
        let items = as_arr(ticks, "damage_ticks")?;
        asserts.damage_ticks = Some(parse_damage_ticks(items, window_ticks)?);
    }
    if let Some(ttk) = optional(entries, "ticks_to_kill")? {
        match ttk {
            Json::Null => asserts.ticks_to_kill = Some(None),
            _ => {
                let tick = as_u32(ttk, "ticks_to_kill", u32::MAX)?;
                if tick == 0 || tick > window_ticks + 1 {
                    return Err(ParseError::OutOfRange {
                        what: "assert kill tick outside window".to_owned(),
                    });
                }
                asserts.ticks_to_kill = Some(Some(tick));
            }
        }
    }
    if entries.iter().any(|(key, _)| {
        ![
            "hp",
            "damage",
            "stuck",
            "killed",
            "damage_ticks",
            "ticks_to_kill",
        ]
        .contains(&key.as_str())
    }) {
        return Err(ParseError::context(
            "assert",
            ParseError::Unknown {
                what: "unknown field".to_owned(),
            },
        ));
    }
    Ok(asserts)
}

// ---- tape building ----------------------------------------------------------

/// Canonical template input every synthetic tape derives from via
/// struct-update syntax, so this module never mirrors button constants.
fn template_input() -> FighterInput {
    static TEMPLATE: OnceLock<FighterInput> = OnceLock::new();
    *TEMPLATE.get_or_init(|| {
        let config = ArenaConfig {
            seed: 2040,
            ticks: 8,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: 1,
        };
        run_dataset(&config, &FighterTape::default())
            .and_then(|dataset| dataset.rows.into_iter().next().map(|row| row.action.input))
            .unwrap_or_else(|| FighterInput {
                tick: 0,
                fighter: engine_arena::FighterId::new(0).unwrap_or(engine_arena::FighterId(0)),
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: Default::default(),
            })
    })
}

fn make_input(
    tick: u32,
    fighter: u8,
    mx: i8,
    mz: i8,
    facing: i16,
    buttons: u8,
) -> Option<FighterInput> {
    let id = engine_arena::FighterId::new(fighter)?;
    let template = template_input();
    let mut input = template;
    input.tick = tick;
    input.fighter = id;
    input.mx = mx;
    input.mz = mz;
    input.facing = facing;
    input.buttons.0 = buttons;
    input.validate().ok()?;
    Some(input)
}

/// Materialize full-coverage tapes: every `(tick, slot)` runs the scripted
/// override or idle, so brains are never consulted and the run is exact.
fn materialize(
    overrides: &BTreeMap<(u32, u8), ScriptInput>,
    lower: u32,
    upper: u32,
    fighters: u8,
) -> Option<FighterTape> {
    let mut inputs = Vec::new();
    for tick in lower..upper {
        for slot in 0..fighters {
            let (mx, mz, facing, buttons) = match overrides.get(&(tick, slot)) {
                Some(entry) => (entry.mx, entry.mz, entry.facing, entry.buttons),
                None => (0, 0, 0, 0),
            };
            inputs.push(make_input(tick, slot, mx, mz, facing, buttons)?);
        }
    }
    let tape = FighterTape(inputs);
    tape.validate().ok()?;
    Some(tape)
}

// ---- seeded bugs ------------------------------------------------------------

/// One synthetic tape fault (sensitivity harness, not a paper-accuracy claim):
/// an input-level fault emulating a gameplay bug class inside a single
/// keypoint: the observable end state matches what the real defect would
/// produce (wrong outcome at the asserted tick), scoped so sibling
/// keypoints keep passing. Detection rates here measure sensitivity on
/// these 5 synthetic faults, not GameGen-Verifier accuracy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FaultOp {
    /// Shift one fighter's window inputs in `[from, to]` by `delta` ticks.
    MoveRange {
        /// Fighter slot to mutate.
        fighter: u8,
        /// First absolute tick to move, inclusive.
        from: u32,
        /// Last absolute tick to move, inclusive.
        to: u32,
        /// Signed shift in ticks.
        delta: i32,
    },
    /// Drop one window input (a swallowed press the claim needed).
    DropInput {
        /// Fighter slot to mutate.
        fighter: u8,
        /// Absolute tick to drop.
        tick: u32,
    },
}

/// One seeded bug: id, paired keypoint and the fault to inject.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BugSeed {
    /// Stable bug id for reports.
    pub id: &'static str,
    /// Bug class under test.
    pub class: &'static str,
    /// Paired keypoint id; exactly this keypoint must fail.
    pub target: &'static str,
    /// Fault to apply to the target's tapes.
    pub op: FaultOp,
}

/// The five seeded bugs: wrong window, missing hit-stop, stuck state, a
/// dead stamina regen and a dropped kill confirm.
pub const BUG_SEEDS: [BugSeed; BUG_COUNT] = [
    BugSeed {
        id: "bug-window-parry-late",
        class: "wrong window",
        target: "kp06-parry-riposte",
        op: FaultOp::DropInput {
            fighter: 1,
            tick: 8,
        },
    },
    BugSeed {
        id: "bug-hitstop-clock-runs",
        class: "missing hit-stop",
        target: "kp10-freeze-parry-clock",
        op: FaultOp::MoveRange {
            fighter: 2,
            from: 22,
            to: 22,
            delta: 1,
        },
    },
    BugSeed {
        id: "bug-react-stuck-early",
        class: "stuck state",
        target: "kp21-counter-after-react",
        op: FaultOp::MoveRange {
            fighter: 1,
            from: 18,
            to: 18,
            delta: -5,
        },
    },
    BugSeed {
        id: "bug-stamina-regen-dead",
        class: "missing stamina regen",
        target: "kp14-stamina-regen",
        op: FaultOp::DropInput {
            fighter: 0,
            tick: 54,
        },
    },
    BugSeed {
        id: "bug-kill-confirm-dropped",
        class: "dropped kill confirm",
        target: "kp15-kill-confirm",
        op: FaultOp::DropInput {
            fighter: 0,
            tick: 192,
        },
    },
];

fn apply_fault(
    window: &mut BTreeMap<(u32, u8), ScriptInput>,
    prefix: u32,
    window_end: u32,
    fault: &BugSeed,
) {
    match fault.op {
        FaultOp::MoveRange {
            fighter,
            from,
            to,
            delta,
        } => {
            let mut moved = Vec::new();
            window.retain(|(tick, slot), entry| {
                if *slot == fighter && *tick >= from && *tick <= to {
                    let shifted = i64::from(*tick) + i64::from(delta);
                    moved.push((*entry, shifted));
                    false
                } else {
                    true
                }
            });
            for (mut entry, shifted) in moved {
                if shifted >= i64::from(prefix) && shifted < i64::from(window_end) {
                    entry.tick = shifted as u32;
                    window.insert((entry.tick, entry.fighter), entry);
                }
            }
        }
        FaultOp::DropInput { fighter, tick } => {
            window.remove(&(tick, fighter));
        }
    }
}

// ---- captures and digests ---------------------------------------------------

/// Multiplier of the keypoint digests. Recorded keypoint evidence pins digests
/// made with it, so it stays distinct from the standard FNV-1a prime
/// (`engine_core::hash::FNV64_PRIME`); changing it re-records every digest.
const DIGEST_PRIME: u64 = 0x1000_0000_01b3;

/// Byte-fold digest builder; every digest in this module folds integers only.
struct Digester {
    hash: u64,
}

impl Digester {
    fn new() -> Self {
        Self { hash: FNV64_OFFSET }
    }

    fn bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.hash ^= u64::from(*byte);
            self.hash = self.hash.wrapping_mul(DIGEST_PRIME);
        }
    }

    fn u8(&mut self, value: u8) {
        self.bytes(&[value]);
    }

    fn u16(&mut self, value: u16) {
        self.bytes(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }

    fn text(&mut self, value: &str) {
        self.u64(value.len() as u64);
        self.bytes(value.as_bytes());
    }

    fn finish(self) -> u64 {
        self.hash
    }
}

/// One deterministic top-down capture: floor, walls, the crate prop and one
/// disc per fighter shaded by health with integer math only. Positions map
/// through integer millimeters; no raw float bits are hashed (placement
/// passes an f32-to-int cast first, same-machine scope per HASH-LAW).
fn capture_rgba(observation: &ForkObservation, half_extent_m: f32, edge: u32) -> Vec<u8> {
    let edge = edge.clamp(8, 256) as usize;
    let mut pixels = vec![0u8; edge * edge * 4];
    for (index, pixel) in pixels.chunks_exact_mut(4).enumerate() {
        let x = index % edge;
        let y = index / edge;
        let border = x == 0 || y == 0 || x + 1 == edge || y + 1 == edge;
        if border {
            pixel.copy_from_slice(&[90, 96, 110, 255]);
        } else {
            pixel.copy_from_slice(&[16, 18, 24, 255]);
        }
    }
    let cell = |pos_mm: i32, half_mm: i32| -> Option<usize> {
        if half_mm <= 0 {
            return None;
        }
        let scaled = ((pos_mm + half_mm) as i64 * edge as i64) / (2 * half_mm as i64);
        if scaled < 0 || scaled >= edge as i64 {
            None
        } else {
            Some(scaled as usize)
        }
    };
    let half_mm = (half_extent_m * 1000.0) as i32;
    let dot = |pixels: &mut [u8], cx: usize, cy: usize, color: [u8; 4]| {
        for dy in -2..=2 {
            for dx in -2..=2 {
                if dx * dx + dy * dy > 5 {
                    continue;
                }
                let (x, y) = (cx as i32 + dx, cy as i32 + dy);
                if x < 0 || y < 0 || x >= edge as i32 || y >= edge as i32 {
                    continue;
                }
                let start = (y as usize * edge + x as usize) * 4;
                pixels[start..start + 4].copy_from_slice(&color);
            }
        }
    };
    const COLORS: [[u8; 3]; 4] = [
        [255, 170, 40],
        [80, 220, 230],
        [230, 110, 220],
        [150, 230, 120],
    ];
    for entry in &observation.fighters {
        let x_mm = (entry.pos[0] * 1000.0) as i32;
        let z_mm = (entry.pos[2] * 1000.0) as i32;
        let (Some(cx), Some(cy)) = (cell(x_mm, half_mm), cell(z_mm, half_mm)) else {
            continue;
        };
        let shade = 64 + 191 * u32::from(entry.hp.min(1000)) / 1000;
        let base = COLORS[usize::from(entry.id) % COLORS.len()];
        let color = [
            (u32::from(base[0]) * shade / 255) as u8,
            (u32::from(base[1]) * shade / 255) as u8,
            (u32::from(base[2]) * shade / 255) as u8,
            255,
        ];
        dot(&mut pixels, cx, cy, color);
    }
    if let (Some(cx), Some(cy)) = (cell(0, half_mm), cell(0, half_mm)) {
        dot(&mut pixels, cx, cy, [110, 110, 118, 255]);
    }
    pixels
}

// ---- runner -----------------------------------------------------------------

/// How a keypoint executes: injected mid-match state, the same script
/// without injection (ablation), or with one seeded fault applied.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RunMode<'a> {
    Inject,
    Ablate,
    Fault(&'a BugSeed),
    FaultAblated(&'a BugSeed),
}

/// One keypoint outcome: pass plus the evidence that decided it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct KeypointResult {
    /// Registry id.
    pub id: String,
    /// True when every asserted field matched.
    pub pass: bool,
    /// One line per mismatched field, `expected … observed …`.
    pub failures: Vec<String>,
    /// Causal inputs behind a fault failure, `tick … fighter …`.
    pub causes: Vec<String>,
    /// FNV digest over the integer run evidence.
    pub digest: u64,
    /// Hex digest of the one capture taken.
    pub capture: String,
}

/// Whole-suite verdict: every keypoint outcome plus the suite digest.
/// `pass` is true exactly when every result passes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SuiteReport {
    /// Report format version, always [`REPORT_VERSION`].
    pub version: u16,
    /// One outcome per keypoint, sorted by id.
    pub results: Vec<KeypointResult>,
    /// FNV digest over the ordered per-keypoint digests.
    pub digest: u64,
    /// Total simulated ticks behind the report (the tick budget input).
    pub sim_ticks: u64,
}

impl SuiteReport {
    /// True when every keypoint passed.
    pub fn pass(&self) -> bool {
        self.results.iter().all(|result| result.pass)
    }

    /// Count of passing keypoints.
    pub fn passed(&self) -> usize {
        self.results.iter().filter(|result| result.pass).count()
    }
}

/// Registry ids in run order, for the proof harness and `sim.verify`.
pub fn keypoint_ids() -> Result<Vec<String>, KeypointError> {
    Ok(load_specs()?.into_iter().map(|spec| spec.id).collect())
}

/// Run the full set with state injection: prefix each match, fork at the
/// prefix tick, run the bounded window, assert typed state, capture once.
pub fn run_suite() -> Result<SuiteReport, KeypointError> {
    let specs = load_specs()?;
    let mut results = Vec::with_capacity(specs.len());
    let mut sim_ticks: u64 = 0;
    for spec in &specs {
        let (result, ticks) = run_keypoint(spec, RunMode::Inject)?;
        sim_ticks += u64::from(ticks);
        results.push(result);
    }
    Ok(finish_suite(results, sim_ticks))
}

/// Run the full set with one seeded fault applied to its target.
pub fn run_suite_with_fault(fault: &BugSeed) -> Result<SuiteReport, KeypointError> {
    let specs = load_specs()?;
    let mut results = Vec::with_capacity(specs.len());
    let mut sim_ticks: u64 = 0;
    for spec in &specs {
        let mode = if spec.id == fault.target {
            RunMode::Fault(fault)
        } else {
            RunMode::Inject
        };
        let (result, ticks) = run_keypoint(spec, mode)?;
        sim_ticks += u64::from(ticks);
        results.push(result);
    }
    Ok(finish_suite(results, sim_ticks))
}

/// Ablation verdict for one seeded bug without state injection.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AblationRow {
    /// Seeded bug id.
    pub bug: String,
    /// Paired keypoint id.
    pub target: String,
    /// The unmodified window passes blind from the default state.
    pub baseline_pass: bool,
    /// The faulted window still passes blind: the bug is missed.
    pub fault_pass: bool,
}

impl AblationRow {
    /// True when the bug is caught without injection (baseline green,
    /// fault red). Anything else misses the bug.
    pub fn caught(&self) -> bool {
        self.baseline_pass && !self.fault_pass
    }
}

/// Ablation report: every seeded bug rerun without injection.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AblationReport {
    /// One row per seeded bug, in [`BUG_SEEDS`] order.
    pub rows: Vec<AblationRow>,
}

impl AblationReport {
    /// Bugs missed without injection.
    pub fn missed(&self) -> usize {
        self.rows.iter().filter(|row| !row.caught()).count()
    }
}

/// Run every seeded bug with and without its fault, all without injection:
/// the window script runs from the default state, so injection-armed traps
/// cannot stage and the bugs must go undetected.
pub fn run_ablation() -> Result<AblationReport, KeypointError> {
    let specs = load_specs()?;
    let by_id: BTreeMap<&str, &KeypointSpec> =
        specs.iter().map(|spec| (spec.id.as_str(), spec)).collect();
    let mut rows = Vec::with_capacity(BUG_SEEDS.len());
    for fault in &BUG_SEEDS {
        let Some(spec) = by_id.get(fault.target) else {
            return Err(KeypointError::BadSpec {
                file: fault.target.to_owned(),
                reason: "fault target missing from suite".to_owned(),
            });
        };
        let (baseline, _) = run_keypoint(spec, RunMode::Ablate)?;
        let (faulted, _) = run_keypoint(spec, RunMode::FaultAblated(fault))?;
        rows.push(AblationRow {
            bug: fault.id.to_owned(),
            target: fault.target.to_owned(),
            baseline_pass: baseline.pass,
            fault_pass: faulted.pass,
        });
    }
    Ok(AblationReport { rows })
}

fn finish_suite(mut results: Vec<KeypointResult>, sim_ticks: u64) -> SuiteReport {
    results.sort_by(|a, b| a.id.cmp(&b.id));
    let mut digester = Digester::new();
    digester.u16(REPORT_VERSION);
    for result in &results {
        digester.text(&result.id);
        digester.u8(u8::from(result.pass));
        digester.u64(result.digest);
    }
    SuiteReport {
        version: REPORT_VERSION,
        results,
        digest: digester.finish(),
        sim_ticks,
    }
}

fn run_keypoint(
    spec: &KeypointSpec,
    mode: RunMode<'_>,
) -> Result<(KeypointResult, u32), KeypointError> {
    let ablate = matches!(mode, RunMode::Ablate | RunMode::FaultAblated(_));
    let fault = match mode {
        RunMode::Fault(fault) | RunMode::FaultAblated(fault) => Some(fault),
        _ => None,
    };
    let prefix = if ablate { 0 } else { spec.prefix_ticks };
    let window = spec.window_ticks;
    let total = prefix.saturating_add(window).saturating_add(1);
    let config = ArenaConfig {
        seed: spec.seed,
        ticks: total,
        fighters: spec.fighters,
        half_extent_m: spec.half_extent_m,
        table_version: 1,
    };
    let setup_map: BTreeMap<(u32, u8), ScriptInput> = if ablate {
        BTreeMap::new()
    } else {
        spec.setup
            .iter()
            .map(|entry| ((entry.tick, entry.fighter), *entry))
            .collect()
    };
    let mut window_map: BTreeMap<(u32, u8), ScriptInput> = spec
        .window
        .iter()
        .map(|entry| {
            let tick = if ablate {
                entry.tick.saturating_sub(spec.prefix_ticks)
            } else {
                entry.tick
            };
            let mut shifted = *entry;
            shifted.tick = tick;
            ((tick, shifted.fighter), shifted)
        })
        .collect();
    if let Some(fault) = fault {
        if ablate {
            let base = spec.prefix_ticks;
            let rebased = BugSeed {
                id: fault.id,
                class: fault.class,
                target: fault.target,
                op: rebase_op(fault.op, base),
            };
            apply_fault(&mut window_map, 0, window, &rebased);
        } else {
            apply_fault(
                &mut window_map,
                prefix,
                prefix.saturating_add(window),
                fault,
            );
        }
    }
    let setup_tape = materialize(&setup_map, 0, prefix, spec.fighters)
        .ok_or_else(|| KeypointError::ArenaRefused(spec.id.clone()))?;
    // Full window coverage plus one idle settle tick: every `(tick, slot)`
    // is scripted, so brains are never consulted and the run is exact.
    let window_tape = materialize(&window_map, prefix, prefix + window + 1, spec.fighters)
        .ok_or_else(|| KeypointError::ArenaRefused(spec.id.clone()))?;
    let mut window_full: BTreeMap<u8, Vec<FighterInput>> = BTreeMap::new();
    for input in &window_tape.0 {
        window_full.entry(input.fighter.0).or_default().push(*input);
    }
    let (snapshot, _) = run_prefix(&config, &setup_tape, prefix)
        .ok_or_else(|| KeypointError::ArenaRefused(spec.id.clone()))?;
    let mut fork = Fork::from_snapshot(snapshot, prefix)
        .ok_or_else(|| KeypointError::ArenaRefused(spec.id.clone()))?;
    for slot in &spec.freeze {
        fork.apply_proposal(ForkProposal::FreezeFighter { fighter: *slot })
            .map_err(|_| KeypointError::ArenaRefused(spec.id.clone()))?;
    }
    for slot in 0..spec.fighters {
        let slice = window_full
            .remove(&slot)
            .ok_or_else(|| KeypointError::ArenaRefused(spec.id.clone()))?;
        if fork
            .apply_proposal(ForkProposal::SwapTape {
                fighter: slot,
                inputs: slice,
            })
            .is_err()
        {
            return Err(KeypointError::ArenaRefused(spec.id.clone()));
        }
    }
    let metrics = fork.run_n(window);
    let (recorded, dataset) = fork.run_recorded(window + 1);
    let _ = recorded;
    let failures = check_asserts(spec, prefix, window, &dataset, &metrics);
    let settle = dataset
        .rows
        .iter()
        .filter(|row| row.observation.tick == prefix + window)
        .map(|row| row.observation.clone())
        .next()
        .unwrap_or(ForkObservation {
            tick: prefix + window,
            fighters: FighterEntries::empty(),
        });
    let pixels = capture_rgba(&settle, spec.half_extent_m, CAPTURE_EDGE);
    let capture = digest_hex(&pixels);
    if pixels.iter().min() == pixels.iter().max() {
        let mut extended = failures;
        extended.push("capture is a flat field".to_owned());
        return Ok((
            finish_result(
                spec,
                extended,
                Vec::new(),
                &dataset,
                &metrics,
                &pixels,
                capture,
            ),
            total,
        ));
    }
    let mut causes = Vec::new();
    if fault.is_some() && !failures.is_empty() {
        causes = attribute_fault(spec, prefix, window, &setup_map, &window_map);
    }
    Ok((
        finish_result(spec, failures, causes, &dataset, &metrics, &pixels, capture),
        total,
    ))
}

/// Rebase a fault's absolute ticks into ablation space.
fn rebase_op(op: FaultOp, base: u32) -> FaultOp {
    match op {
        FaultOp::MoveRange {
            fighter,
            from,
            to,
            delta,
        } => FaultOp::MoveRange {
            fighter,
            from: from.saturating_sub(base),
            to: to.saturating_sub(base),
            delta,
        },
        FaultOp::DropInput { fighter, tick } => FaultOp::DropInput {
            fighter,
            tick: tick.saturating_sub(base),
        },
    }
}

/// Attribute a fault failure to its input causes: the causal replay names
/// the 1-minimal set for single-input faults, otherwise the tape diff
/// names the moved inputs (a move leaves residue no single reset removes).
fn attribute_fault(
    spec: &KeypointSpec,
    prefix: u32,
    window: u32,
    setup_map: &BTreeMap<(u32, u8), ScriptInput>,
    window_map: &BTreeMap<(u32, u8), ScriptInput>,
) -> Vec<String> {
    let total = prefix + window + 1;
    let config = ArenaConfig {
        seed: spec.seed,
        ticks: total,
        fighters: spec.fighters,
        half_extent_m: spec.half_extent_m,
        table_version: 1,
    };
    let baseline_setup: BTreeMap<(u32, u8), ScriptInput> = spec
        .setup
        .iter()
        .map(|entry| ((entry.tick, entry.fighter), *entry))
        .collect();
    let baseline_window: BTreeMap<(u32, u8), ScriptInput> = spec
        .window
        .iter()
        .map(|entry| ((entry.tick, entry.fighter), *entry))
        .collect();
    let Some(baseline) = full_tape(
        &baseline_setup,
        &baseline_window,
        0,
        prefix + window,
        spec.fighters,
    ) else {
        return Vec::new();
    };
    let Some(faulted) = full_tape(setup_map, window_map, 0, prefix + window, spec.fighters) else {
        return Vec::new();
    };
    match crate::causal::attribute(&config, &baseline, &faulted) {
        Ok(report) if !report.causes.is_empty() => {
            report.causes.into_iter().take(4).map(cause_text).collect()
        }
        _ => diff_inputs(&baseline, &faulted),
    }
}

/// Tape diff fallback: every `(tick, fighter)` where the faulted tape
/// differs from baseline, capped. Deterministic in key order.
fn diff_inputs(baseline: &FighterTape, faulted: &FighterTape) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut keys = BTreeSet::new();
    for input in baseline.0.iter().chain(faulted.0.iter()) {
        keys.insert((input.tick, input.fighter.0));
    }
    let base_by_key: BTreeMap<(u32, u8), FighterInput> = baseline
        .0
        .iter()
        .map(|input| ((input.tick, input.fighter.0), *input))
        .collect();
    let fault_by_key: BTreeMap<(u32, u8), FighterInput> = faulted
        .0
        .iter()
        .map(|input| ((input.tick, input.fighter.0), *input))
        .collect();
    keys.into_iter()
        .filter(|key| base_by_key.get(key) != fault_by_key.get(key))
        .take(4)
        .map(|(tick, fighter)| format!("tick {tick} fighter {fighter}"))
        .collect()
}

fn cause_text(cause: Cause) -> String {
    format!("tick {} fighter {}", cause.tick, cause.fighter)
}

fn full_tape(
    setup: &BTreeMap<(u32, u8), ScriptInput>,
    window: &BTreeMap<(u32, u8), ScriptInput>,
    lower: u32,
    upper: u32,
    fighters: u8,
) -> Option<FighterTape> {
    let mut merged = setup.clone();
    for (key, entry) in window {
        merged.insert(*key, *entry);
    }
    materialize(&merged, lower, upper, fighters)
}

fn finish_result(
    spec: &KeypointSpec,
    failures: Vec<String>,
    causes: Vec<String>,
    dataset: &engine_arena::recorder::Dataset,
    metrics: &engine_arena::fork::ForkMetrics,
    pixels: &[u8],
    capture: String,
) -> KeypointResult {
    let mut digester = Digester::new();
    digester.text(&spec.id);
    digester.u64(spec.seed);
    digester.u8(spec.fighters);
    digester.u32(spec.prefix_ticks);
    digester.u32(spec.window_ticks);
    for row in &dataset.rows {
        digester.u32(row.observation.tick);
        for entry in &row.observation.fighters {
            digester.u8(entry.id);
            digester.u16(entry.hp);
        }
        digester.u8(row.action.fighter);
        digester.u16(row.outcome.damage_before);
        digester.u16(row.outcome.damage_after);
        digester.u8(u8::from(row.outcome.killed));
    }
    for (slot, damage) in &metrics.damage_dealt {
        digester.u8(*slot);
        digester.u16(*damage);
    }
    for (slot, stuck) in &metrics.stuck_ticks {
        digester.u8(*slot);
        digester.u32(*stuck);
    }
    digester.u32(metrics.ticks_to_kill.unwrap_or(u32::MAX));
    digester.bytes(pixels);
    KeypointResult {
        id: spec.id.clone(),
        pass: failures.is_empty(),
        failures,
        causes,
        digest: digester.finish(),
        capture,
    }
}

fn check_asserts(
    spec: &KeypointSpec,
    prefix: u32,
    window: u32,
    dataset: &engine_arena::recorder::Dataset,
    metrics: &engine_arena::fork::ForkMetrics,
) -> Vec<String> {
    let mut failures = Vec::new();
    let end = prefix + window;
    let settle_tick = prefix + window;
    let settle = dataset
        .rows
        .iter()
        .find(|row| row.observation.tick == settle_tick)
        .map(|row| &row.observation);
    let hp_at_settle = |slot: u8| -> Option<u16> {
        settle?
            .fighters
            .iter()
            .find(|entry| entry.id == slot)
            .map(|entry| entry.hp)
    };
    for (slot, expected) in &spec.asserts.hp {
        match hp_at_settle(*slot) {
            Some(observed) if observed == *expected => {}
            Some(observed) => failures.push(format!(
                "hp fighter {slot}: expected {expected} observed {observed}"
            )),
            None => failures.push(format!("hp fighter {slot}: no observation")),
        }
    }
    let mut damage_after: BTreeMap<u8, u16> = BTreeMap::new();
    let mut damage_ticks: Vec<u32> = Vec::new();
    let mut killed = false;
    for row in &dataset.rows {
        if row.observation.tick >= end {
            continue;
        }
        damage_after.insert(row.action.fighter, row.outcome.damage_after);
        if row.outcome.damage_after > row.outcome.damage_before
            && !damage_ticks.contains(&(row.observation.tick - prefix))
        {
            damage_ticks.push(row.observation.tick - prefix);
        }
        killed = killed || row.outcome.killed;
    }
    damage_ticks.sort_unstable();
    for (slot, expected) in &spec.asserts.damage {
        let observed = damage_after.get(slot).copied().unwrap_or(0);
        if observed != *expected {
            failures.push(format!(
                "damage fighter {slot}: expected {expected} observed {observed}"
            ));
        }
    }
    for (slot, expected) in &spec.asserts.stuck {
        let observed = metrics
            .stuck_ticks
            .iter()
            .find(|(known, _)| *known == *slot)
            .map(|(_, stuck)| *stuck);
        match observed {
            Some(value) if value == *expected => {}
            Some(value) => failures.push(format!(
                "stuck fighter {slot}: expected {expected} observed {value}"
            )),
            None => failures.push(format!("stuck fighter {slot}: no metric")),
        }
    }
    if let Some(expected) = spec.asserts.killed {
        if killed != expected {
            failures.push(format!("killed: expected {expected} observed {killed}"));
        }
    }
    if let Some(expected) = &spec.asserts.damage_ticks {
        if &damage_ticks != expected {
            failures.push(format!(
                "damage_ticks: expected {expected:?} observed {damage_ticks:?}"
            ));
        }
    }
    if let Some(expected) = spec.asserts.ticks_to_kill {
        if metrics.ticks_to_kill != expected {
            failures.push(format!(
                "ticks_to_kill: expected {expected:?} observed {:?}",
                metrics.ticks_to_kill
            ));
        }
    }
    let _ = window;
    failures.sort();
    failures
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keypoint_suite_loads_thirty_specs() {
        let specs = load_specs().expect("specs load");
        assert_eq!(specs.len(), KEYPOINT_COUNT, "suite holds 30 keypoints");
        let mut ids: Vec<&str> = specs.iter().map(|spec| spec.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), KEYPOINT_COUNT, "ids are unique");
        for spec in &specs {
            assert!(
                spec.window_ticks > 0 && spec.window_ticks <= MAX_WINDOW_TICKS,
                "{} window bound",
                spec.id
            );
        }
    }

    #[test]
    fn keypoint_suite_passes_with_injection() {
        let report = run_suite().expect("suite runs");
        assert_eq!(report.results.len(), KEYPOINT_COUNT);
        assert_eq!(report.version, REPORT_VERSION);
        for result in &report.results {
            assert!(
                result.pass,
                "{} must pass: {:?}",
                result.id, result.failures
            );
        }
        assert!(report.pass());
        assert_eq!(report.passed(), KEYPOINT_COUNT);
        println!(
            "suite digest {:016x} over {} ticks",
            report.digest, report.sim_ticks
        );
        for result in &report.results {
            println!(
                "pass {} digest {:016x} capture {}",
                result.id, result.digest, result.capture
            );
        }
    }

    #[test]
    fn keypoint_suite_reproducible_run1_eq_run2() {
        let first = run_suite().expect("first run");
        let second = run_suite().expect("second run");
        assert_eq!(first, second, "run1 must equal run2");
    }

    #[test]
    fn keypoint_faults_fail_exactly_paired() {
        assert_eq!(BUG_SEEDS.len(), BUG_COUNT);
        for fault in &BUG_SEEDS {
            let report = run_suite_with_fault(fault).expect("fault run");
            let mut failed = Vec::new();
            for result in &report.results {
                if result.pass {
                    continue;
                }
                failed.push(result.id.clone());
            }
            assert_eq!(
                failed,
                vec![fault.target.to_owned()],
                "bug {} must fail exactly its keypoint",
                fault.id
            );
            let target = report
                .results
                .iter()
                .find(|result| result.id == fault.target)
                .expect("target present");
            assert!(!target.failures.is_empty(), "failure must say what broke");
            println!(
                "bug {} fails {}: {:?}",
                fault.id, fault.target, target.failures
            );
        }
    }

    #[test]
    fn keypoint_ablation_without_injection_misses() {
        let ablation = run_ablation().expect("ablation runs");
        assert_eq!(ablation.rows.len(), BUG_COUNT);
        for row in &ablation.rows {
            println!(
                "ablation {} baseline_pass={} fault_pass={} caught={}",
                row.bug,
                row.baseline_pass,
                row.fault_pass,
                row.caught()
            );
        }
        assert!(
            ablation.missed() >= 3,
            "ablation must miss at least 3 of 5 without injection, missed {}",
            ablation.missed()
        );
    }

    #[test]
    fn keypoint_specs_reject_bad_data() {
        let bad_version = serde_like(r#"{"version": 0}"#, "kp-bad");
        assert!(bad_version.is_err(), "version 0 must fail");
        let bad_buttons = Parser::parse(
            r#"{"version":1,"id":"x","claim":"c","seed":1,"fighters":2,"half_extent_m":20.0,"prefix_ticks":0,"window_ticks":4,"setup":[],"window":[{"tick":0,"fighter":0,"mx":0,"mz":0,"facing":0,"buttons":8}],"assert":{}}"#,
        );
        assert!(bad_buttons.is_ok(), "parses as json");
        let spec = parse_spec(&bad_buttons.expect("json"), "x");
        assert!(spec.is_err(), "button byte 8 must fail validation");
        let unknown_field = Parser::parse(r#"{"version":1,"nope":true}"#).expect("json");
        assert!(
            parse_spec(&unknown_field, "x").is_err(),
            "unknown shape fails"
        );
    }

    fn serde_like(text: &str, stem: &str) -> Result<KeypointSpec, KeypointError> {
        let parsed = Parser::parse(text).map_err(|error| KeypointError::BadSpec {
            file: stem.to_owned(),
            reason: error.to_string(),
        })?;
        parse_spec(&parsed, stem).map_err(|error| KeypointError::BadSpec {
            file: stem.to_owned(),
            reason: error.to_string(),
        })
    }

    #[test]
    fn keypoint_captures_cover_and_discern() {
        let report = run_suite().expect("suite runs");
        let mut digests = Vec::new();
        for result in &report.results {
            assert_eq!(result.capture.len(), 16, "capture digest is 16 hex chars");
            assert_ne!(result.capture, "0000000000000000", "capture digest set");
            digests.push(result.capture.clone());
        }
        digests.sort();
        digests.dedup();
        assert!(
            digests.len() >= report.results.len() / 2,
            "captures must discern scenarios, {} unique of {}",
            digests.len(),
            report.results.len()
        );
    }

    #[test]
    fn keypoint_failures_carry_causal_input() {
        let specs = load_specs().expect("specs load");
        let by_id: BTreeMap<&str, &KeypointSpec> =
            specs.iter().map(|spec| (spec.id.as_str(), spec)).collect();
        for fault in &BUG_SEEDS {
            let spec = by_id[fault.target];
            let (result, _) = run_keypoint(spec, RunMode::Fault(fault)).expect("fault runs");
            assert!(
                !result.pass,
                "{} must fail under {}",
                fault.target, fault.id
            );
            assert!(
                !result.causes.is_empty(),
                "{} must name a causal input",
                fault.id
            );
            println!("bug {} causes {:?}", fault.id, result.causes);
        }
        // Single-input drops attribute exactly to the dropped input.
        let drops = [
            ("bug-window-parry-late", "tick 8 fighter 1"),
            ("bug-stamina-regen-dead", "tick 54 fighter 0"),
            ("bug-kill-confirm-dropped", "tick 192 fighter 0"),
        ];
        for (bug, cause) in drops {
            let fault = BUG_SEEDS
                .iter()
                .find(|seed| seed.id == bug)
                .expect("bug listed");
            let spec = by_id[fault.target];
            let (result, _) = run_keypoint(spec, RunMode::Fault(fault)).expect("fault runs");
            assert_eq!(
                result.causes,
                vec![cause.to_owned()],
                "exact cause for {bug}"
            );
        }
    }

    #[test]
    fn keypoint_suite_within_tick_budget() {
        let report = run_suite().expect("suite runs");
        assert!(
            report.sim_ticks <= SUITE_TICK_BUDGET,
            "suite over tick budget: {}",
            report.sim_ticks
        );
        println!("suite ticks {} of {SUITE_TICK_BUDGET}", report.sim_ticks);
    }

    #[test]
    fn keypoint_registry_lists_every_spec() {
        let ids = keypoint_ids().expect("registry lists");
        assert_eq!(ids.len(), KEYPOINT_COUNT);
        let specs = load_specs().expect("specs load");
        for spec in &specs {
            assert!(ids.contains(&spec.id), "{} in registry", spec.id);
        }
    }
}
