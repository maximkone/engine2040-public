//! Declarative scenario fixtures.
//!
//! Shape from flax-mcp `scenarios/*.json` (`determinism_test.json`, MIT,
//! Maxim's own code), read by this small std-only reader (no new dep):
//! seed + input/wait steps in order + assertions + tags. Same seed twice
//! replays identically; every refusal names the legal shape.

use engine_core::hash_bytes;

/// One step: a named input held `ticks` long, or a wait of `ticks`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Step {
    /// Hold the named input for `ticks` ticks.
    Input {
        /// Input name (for example `"jab"`).
        name: String,
        /// Ticks to hold (`1..=600`).
        ticks: u32,
    },
    /// Advance `ticks` ticks with neutral input.
    Wait {
        /// Ticks to wait (`1..=600`).
        ticks: u32,
    },
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
    /// Scenario name.
    pub name: String,
    /// Replay seed.
    pub seed: u64,
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
    let seed = u64_field(text, "\"seed\"").ok_or(ShapeError::Missing("seed"))?;
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
        seed,
        steps,
        assertions,
        tags,
    })
}

/// Deterministic replay digest: seed plus ordered steps folded in order.
/// Same scenario twice gives the same digest.
pub fn replay_digest(scenario: &Scenario) -> u64 {
    let mut words: Vec<u64> = Vec::with_capacity(2 + scenario.steps.len() * 3);
    words.push(scenario.seed);
    words.push(u64::try_from(scenario.tags.len()).unwrap_or(u64::MAX));
    for step in &scenario.steps {
        match step {
            Step::Input { name, ticks } => {
                words.push(1);
                words.push(hash_bytes(name.as_bytes()));
                words.push(u64::from(*ticks));
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
            let ticks = u32_from_u64_wrap(u64_field(row, "\"ticks\"").unwrap_or(1));
            if name.is_empty() || name.len() > MAX_TOKEN_CHARS {
                return Err(ShapeError::BadStep {
                    index,
                    hint: "input needs a 1..=128 char name",
                });
            }
            if ticks == 0 || ticks > 600 {
                return Err(ShapeError::BadStep {
                    index,
                    hint: "input ticks must be 1..=600",
                });
            }
            steps.push(Step::Input { name, ticks });
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
