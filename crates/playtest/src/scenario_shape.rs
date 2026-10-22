//! Declarative scenario fixtures.
//!
//! Shape from flax-mcp `scenarios/*.json` (`determinism_test.json`, MIT,
//! Maxim's own code), read by this small std-only reader (no new dep):
//! seed + input/wait steps in order + assertions + tags. Same seed twice
//! replays identically; every refusal names the legal shape.
//!
//! A scenario that is replayed carries an optional `bout` block (fixed arena
//! settings) and may pin each input step to a slot, world wishes, facing and
//! an absolute tick. Both are additions to the same file shape: a fixture
//! without them still parses and keeps its shape digest.

use engine_arena::MAX_TICKS;
use engine_core::hash_bytes;

/// Most fighters one step may name (`engine_input::MAX_PARTICIPANTS`).
pub const MAX_SLOTS: u8 = 32;

/// One step: a named input held `ticks` long, or a wait of `ticks`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Step {
    /// Hold the named input for `ticks` ticks.
    Input {
        /// Input name (for example `"jab"`); the replay maps it to buttons.
        name: String,
        /// Ticks to hold (`1..=600`).
        ticks: u32,
        /// Slot the input applies to (`0..MAX_SLOTS`, default `0`).
        fighter: u8,
        /// Forward wish `-100..=100` percent, default `0`.
        mx: i8,
        /// Right wish `-100..=100` percent, default `0`.
        mz: i8,
        /// Absolute facing in centidegrees, default `0`.
        facing: i16,
        /// Absolute first tick, or `None` for the running cursor.
        at: Option<u32>,
    },
    /// Advance `ticks` ticks with neutral input.
    Wait {
        /// Ticks to wait (`1..=600`).
        ticks: u32,
    },
}

/// One bout block: the fixed arena settings a scenario replays under.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Bout {
    /// Fixed steps the bout runs, `1..=engine_arena::MAX_TICKS`.
    pub ticks: u32,
    /// Fighters in the bout, `2..=MAX_SLOTS`.
    pub fighters: u8,
    /// Arena half-extent in whole meters, `5..=100`.
    pub half_extent_m: u8,
    /// Move-table version, `1..=2`.
    pub table_version: u32,
}

/// One assertion: a named check with its expected value.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Assertion {
    /// Check name (for example `"digest"` or `"hits"`).
    pub name: String,
    /// Expected value as written in the fixture.
    pub expect: String,
}

/// One declarative scenario: seed, ordered steps, assertions, tags.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Scenario {
    /// Scenario name, unique in the corpus.
    pub name: String,
    /// System under test (for example `"duel"`); defaults to the name.
    pub system: String,
    /// Replay seed.
    pub seed: u64,
    /// Arena settings when the scenario is replayed; `None` is shape only.
    pub bout: Option<Bout>,
    /// Ordered steps (input/wait as written).
    pub steps: Vec<Step>,
    /// Checks with expected values.
    pub assertions: Vec<Assertion>,
    /// Tags (at least one, for example `"determinism"`).
    pub tags: Vec<String>,
}

/// What stopped a scenario parse. Every message names the legal shape.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ShapeError {
    /// A required top-level key is missing (`seed`, `steps` or `tags`).
    Missing(&'static str),
    /// A value has the wrong shape (carries field and hint).
    BadValue {
        /// Field that failed.
        field: &'static str,
        /// What was legal instead.
        hint: &'static str,
    },
    /// A step row has the wrong shape (carries 1-based index and hint).
    BadStep {
        /// 1-based step index.
        index: usize,
        /// What was legal instead.
        hint: &'static str,
    },
}

impl core::fmt::Display for ShapeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Missing(key) => write!(
                f,
                "scenario missing `{key}`: need seed (u64), steps (input/wait array), tags (string array)"
            ),
            Self::BadValue { field, hint } => {
                write!(f, "scenario field `{field}` is bad: {hint}")
            }
            Self::BadStep { index, hint } => {
                write!(f, "scenario step {index} is bad: {hint}")
            }
        }
    }
}

/// Longest fixture text accepted, in bytes.
pub const MAX_TEXT_BYTES: usize = 64 * 1024;
/// Longest single name/tag/assert string, in chars.
pub const MAX_TOKEN_CHARS: usize = 128;
/// Most steps per scenario.
pub const MAX_STEPS: usize = 600;

/// Parse one scenario fixture. Steps keep file order.
pub fn parse(text: &str) -> Result<Scenario, ShapeError> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(ShapeError::BadValue {
            field: "text",
            hint: "fixture over 64 KiB",
        });
    }
    let name = string_field(text, "\"name\"").unwrap_or_else(|| "scenario".to_string());
    let system = string_field(text, "\"system\"").unwrap_or_else(|| name.clone());
    let seed = u64_field(text, "\"seed\"").ok_or(ShapeError::Missing("seed"))?;
    let bout = parse_bout(text)?;
    let tags = string_array(text, "\"tags\"").ok_or(ShapeError::Missing("tags"))?;
    if tags.is_empty() {
        return Err(ShapeError::BadValue {
            field: "tags",
            hint: "need at least one tag string",
        });
    }
    let steps_text = array_text(text, "\"steps\"").ok_or(ShapeError::Missing("steps"))?;
    let steps = parse_steps(&steps_text)?;
    if steps.is_empty() {
        return Err(ShapeError::BadValue {
            field: "steps",
            hint: "need at least one input/wait step",
        });
    }
    let assertions = match array_text(text, "\"assertions\"") {
        Some(body) => parse_assertions(&body)?,
        None => Vec::new(),
    };
    Ok(Scenario {
        name,
        system,
        seed,
        bout,
        steps,
        assertions,
        tags,
    })
}

/// Read the optional `bout` block. Absent means shape-only (no replay).
fn parse_bout(text: &str) -> Result<Option<Bout>, ShapeError> {
    let bad = |hint| ShapeError::BadValue {
        field: "bout",
        hint,
    };
    if !text.contains("\"bout\"") {
        return Ok(None);
    }
    let body = object_text(text, "\"bout\"").ok_or_else(|| bad("bout must be an object"))?;
    let ticks = u32_from_u64_wrap(
        u64_field(&body, "\"ticks\"").ok_or_else(|| bad("need ticks 1..=100000"))?,
    );
    if ticks == 0 || ticks > MAX_TICKS {
        return Err(bad("ticks must be 1..=100000"));
    }
    let fighters =
        u8_from_u64(u64_field(&body, "\"fighters\"").ok_or_else(|| bad("need fighters 2..=32"))?);
    if !(2..=MAX_SLOTS).contains(&fighters) {
        return Err(bad("fighters must be 2..=32"));
    }
    let half_extent_m = u8_from_u64(
        u64_field(&body, "\"half_extent_m\"").ok_or_else(|| bad("need half_extent_m 5..=100"))?,
    );
    if !(5..=100).contains(&half_extent_m) {
        return Err(bad("half_extent_m must be 5..=100"));
    }
    let table_version = u32_from_u64_wrap(
        u64_field(&body, "\"table_version\"").ok_or_else(|| bad("need table_version 1..=2"))?,
    );
    if !(1..=2).contains(&table_version) {
        return Err(bad("table_version must be 1..=2"));
    }
    Ok(Some(Bout {
        ticks,
        fighters,
        half_extent_m,
        table_version,
    }))
}

/// Deterministic replay digest: seed plus ordered steps folded in order.
/// Same scenario twice gives the same digest.
pub fn replay_digest(scenario: &Scenario) -> u64 {
    let mut words: Vec<u64> = Vec::with_capacity(2 + scenario.steps.len() * 3);
    words.push(scenario.seed);
    words.push(u64::try_from(scenario.tags.len()).unwrap_or(u64::MAX));
    for step in &scenario.steps {
        match step {
            Step::Input {
                name,
                ticks,
                fighter,
                mx,
                mz,
                facing,
                at,
            } => {
                words.push(1);
                words.push(hash_bytes(name.as_bytes()));
                words.push(u64::from(*ticks));
                words.push(u64::from(*fighter));
                words.push(u64::from(u8::from_le_bytes(mx.to_le_bytes())));
                words.push(u64::from(u8::from_le_bytes(mz.to_le_bytes())));
                words.push(u64::from(u16::from_le_bytes(facing.to_le_bytes())));
                words.push(at.map_or(u64::MAX, u64::from));
            }
            Step::Wait { ticks } => {
                words.push(2);
                words.push(0);
                words.push(u64::from(*ticks));
            }
        }
    }
    let mut bytes = Vec::with_capacity(words.len() * 8);
    for w in words {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    hash_bytes(&bytes)
}

fn parse_steps(body: &str) -> Result<Vec<Step>, ShapeError> {
    let mut steps = Vec::new();
    for (n, row) in split_objects(body).iter().enumerate() {
        let index = n + 1;
        if steps.len() >= MAX_STEPS {
            return Err(ShapeError::BadStep {
                index,
                hint: "over 600 steps",
            });
        }
        if let Some(name) = string_field(row, "\"input\"") {
            steps.push(parse_input_step(row, &name, index)?);
        } else if let Some(ticks) = u64_field(row, "\"wait\"") {
            let ticks = u32_from_u64_wrap(ticks);
            if ticks == 0 || ticks > 600 {
                return Err(ShapeError::BadStep {
                    index,
                    hint: "wait ticks must be 1..=600",
                });
            }
            steps.push(Step::Wait { ticks });
        } else {
            return Err(ShapeError::BadStep {
                index,
                hint: "each step needs input (name) or wait (ticks)",
            });
        }
    }
    Ok(steps)
}

/// One input step: the named action plus the optional slot, wishes, facing and
/// absolute tick. Every field is range-checked here so the replay can trust it.
fn parse_input_step(row: &str, name: &str, index: usize) -> Result<Step, ShapeError> {
    let bad = |hint| ShapeError::BadStep { index, hint };
    if name.is_empty() || name.len() > MAX_TOKEN_CHARS {
        return Err(bad("input needs a 1..=128 char name"));
    }
    let ticks = u32_from_u64_wrap(u64_field(row, "\"ticks\"").unwrap_or(1));
    if ticks == 0 || ticks > 600 {
        return Err(bad("input ticks must be 1..=600"));
    }
    let fighter = u8_from_u64(u64_field(row, "\"fighter\"").unwrap_or(0));
    if fighter >= MAX_SLOTS {
        return Err(bad("fighter must be 0..32"));
    }
    let Some(mx) = signed_range(row, "\"mx\"", -100, 100) else {
        return Err(bad("mx must be -100..=100"));
    };
    let Some(mz) = signed_range(row, "\"mz\"", -100, 100) else {
        return Err(bad("mz must be -100..=100"));
    };
    let Some(facing) = signed_range(row, "\"facing\"", -18_000, 18_000) else {
        return Err(bad("facing must be -18000..=18000"));
    };
    let at = match u64_field(row, "\"at\"") {
        Some(raw) => {
            let tick = u32_from_u64_wrap(raw);
            if tick > MAX_TICKS {
                return Err(bad("at must be 0..=100000"));
            }
            Some(tick)
        }
        None => None,
    };
    Ok(Step::Input {
        name: name.to_string(),
        ticks,
        fighter,
        mx: narrow_i8(mx),
        mz: narrow_i8(mz),
        facing: narrow_i16(facing),
        at,
    })
}

/// Optional signed field inside `range`; `None` when present but out of range.
fn signed_range(row: &str, key: &str, low: i64, high: i64) -> Option<i64> {
    match i64_field(row, key) {
        Some(value) if (low..=high).contains(&value) => Some(value),
        Some(_) => None,
        None if row.contains(key) => None,
        None => Some(0),
    }
}

fn narrow_i8(value: i64) -> i8 {
    i8::try_from(value).unwrap_or(0)
}

fn narrow_i16(value: i64) -> i16 {
    i16::try_from(value).unwrap_or(0)
}

fn parse_assertions(body: &str) -> Result<Vec<Assertion>, ShapeError> {
    let mut out = Vec::new();
    for row in split_objects(body) {
        let name = string_field(&row, "\"check\"")
            .or_else(|| string_field(&row, "\"name\""))
            .ok_or(ShapeError::BadValue {
                field: "assertions",
                hint: "each assertion needs a check name",
            })?;
        let expect = string_field(&row, "\"expect\"")
            .or_else(|| string_field(&row, "\"value\""))
            .unwrap_or_default();
        out.push(Assertion { name, expect });
    }
    Ok(out)
}

/// Split a `[...]` body into its top-level `{...}` rows in order.
fn split_objects(body: &str) -> Vec<String> {
    let mut rows = Vec::new();
    let mut depth = 0usize;
    let mut start: Option<usize> = None;
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if let Some(s) = start.take() {
                        rows.push(body[s..=i].to_string());
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    rows
}

/// Raw text inside the first `"key": {...}` object (without the braces).
/// `None` when the key is absent or its value is not an object.
fn object_text(text: &str, key: &str) -> Option<String> {
    let k = text.find(key)?;
    let after = &text[k + key.len()..];
    let colon = after.find(':')?;
    let rest = after[colon + 1..].trim_start();
    if !rest.starts_with('{') {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(rest[1..i].to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Raw text of the array value for `"key": [...]` (without brackets).
fn array_text(text: &str, key: &str) -> Option<String> {
    let k = text.find(key)?;
    let after = &text[k + key.len()..];
    let open = after.find('[')?;
    let mut depth = 0usize;
    let bytes = after.as_bytes();
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(after[open + 1..i].to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// First `"key": "value"` string (no escapes besides `\/` kept as-is).
fn string_field(text: &str, key: &str) -> Option<String> {
    let k = text.find(key)?;
    let after = &text[k + key.len()..];
    let colon = after.find(':')?;
    let rest = after[colon + 1..].trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    let mut out = String::new();
    let mut chars = rest[1..].chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if out.len() > MAX_TOKEN_CHARS {
                    return None;
                }
                return Some(out);
            }
            '\\' => {
                if let Some(e) = chars.next() {
                    out.push(e);
                }
            }
            _ => out.push(c),
        }
        if out.len() > MAX_TOKEN_CHARS + 1 {
            return None;
        }
    }
    None
}

/// First `"key": 123` unsigned integer.
fn u64_field(text: &str, key: &str) -> Option<u64> {
    let k = text.find(key)?;
    let after = &text[k + key.len()..];
    let colon = after.find(':')?;
    let rest = after[colon + 1..].trim_start();
    let mut digits = String::new();
    for c in rest.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            break;
        }
    }
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u64>().ok()
}

/// First `"key": -12` signed integer.
fn i64_field(text: &str, key: &str) -> Option<i64> {
    let k = text.find(key)?;
    let after = &text[k + key.len()..];
    let colon = after.find(':')?;
    let rest = after[colon + 1..].trim_start();
    let bytes = rest.as_bytes();
    let mut i = 0;
    if bytes.first() == Some(&b'-') {
        i = 1;
    }
    let start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == start {
        return None;
    }
    rest[..i].parse::<i64>().ok()
}

/// Narrow a parsed unsigned value to one byte; the caller range-checks it.
fn u8_from_u64(value: u64) -> u8 {
    u8::try_from(value).unwrap_or(u8::MAX)
}

/// Narrow a parsed tick count to the step range with wrapping modulo 2^32,
/// the same value a numeric cast would keep. The range check at the call
/// site rejects anything outside 1..=600, so huge inputs still fail.
fn u32_from_u64_wrap(value: u64) -> u32 {
    u32::try_from(value % (u64::from(u32::MAX) + 1)).unwrap_or(u32::MAX)
}

/// All strings of a `"key": ["a", "b"]` array in order.
fn string_array(text: &str, key: &str) -> Option<Vec<String>> {
    let body = array_text(text, key)?;
    let mut tags = Vec::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c == '"' {
            let mut out = String::new();
            while let Some(e) = chars.next() {
                if e == '"' {
                    break;
                }
                if e == '\\' {
                    if let Some(n) = chars.next() {
                        out.push(n);
                    }
                } else {
                    out.push(e);
                }
            }
            if !out.is_empty() && out.len() <= MAX_TOKEN_CHARS {
                tags.push(out);
            }
        }
    }
    Some(tags)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUEL: &str = include_str!("../../../fixtures/scenarios/duel_seed95.json");
    const GAUNTLET: &str = include_str!("../../../fixtures/scenarios/gauntlet_seed7.json");
    const MIRROR: &str = include_str!("../../../fixtures/scenarios/mirror_seed7.json");

    #[test]
    fn three_samples_parse_steps_in_order_with_seed_and_tags() {
        for (text, name, seed) in [
            (DUEL, "duel_seed95", 95u64),
            (GAUNTLET, "gauntlet_seed7", 7u64),
            (MIRROR, "mirror_seed7", 7u64),
        ] {
            let s = parse(text).expect("sample must parse");
            assert_eq!(s.name, name);
            assert_eq!(s.seed, seed);
            assert!(!s.tags.is_empty(), "tags must be present");
            assert!(!s.steps.is_empty(), "steps must be present");
            assert!(s.steps.len() >= 3, "need 3+ steps");
            // First step is the scripted jab, order kept.
            assert!(matches!(&s.steps[0], Step::Input { name, .. } if name == "jab"));
        }
        // Order is file order: input, wait, input.
        let duel = parse(DUEL).unwrap();
        assert!(matches!(duel.steps[1], Step::Wait { .. }));
    }

    #[test]
    fn same_seed_replays_identical_x2() {
        for text in [DUEL, GAUNTLET, MIRROR] {
            let first = parse(text).unwrap();
            let second = parse(text).unwrap();
            assert_eq!(first, second);
            assert_eq!(replay_digest(&first), replay_digest(&second));
        }
        // Wait/input order matters: swapping two steps moves the digest.
        let mut swapped = parse(DUEL).unwrap();
        swapped.steps.swap(0, 1);
        assert_ne!(
            replay_digest(&swapped),
            replay_digest(&parse(DUEL).unwrap())
        );
    }

    #[test]
    fn missing_seed_steps_or_tags_refused_by_name() {
        assert_eq!(parse("{}"), Err(ShapeError::Missing("seed")));
        assert_eq!(parse(r#"{"seed": 1}"#), Err(ShapeError::Missing("tags")));
        assert_eq!(
            parse(r#"{"seed": 1, "tags": ["determinism"]}"#),
            Err(ShapeError::Missing("steps"))
        );
        assert!(parse(r#"{"seed": 1, "tags": [], "steps": []}"#).is_err());
        assert!(matches!(
            parse(r#"{"seed": 1, "tags": ["t"], "steps": [{"hold": 1}]}"#),
            Err(ShapeError::BadStep { .. })
        ));
    }
}
