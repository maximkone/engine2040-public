//! ladder: the structured-output ladder. A model answer is asked for on the
//! highest rung its backend offers and falls one rung at a time when the
//! answer does not pass the strict proposal grammar:
//!
//! 1. [`Rung::Native`]: the vendor's own structured output, fed the engine
//!    schema (`json_schema`, a forced tool, `responseJsonSchema`, `format`).
//! 2. [`Rung::JsonMode`]: the vendor's JSON mode, no schema.
//! 3. [`Rung::Text`]: plain text, strict parse first, then a bounded repair
//!    ([`repair_text`]) and the same strict parse again.
//! 4. [`Rung::Scripted`]: no model answer is used. The typed error and the
//!    scripted fallback stand in, and nothing is applied.
//!
//! Every rung ends in the same strict grammar check, so a repaired or
//! schema-constrained answer is no more trusted than a plain one. A drop is a
//! logged [`RungDrop`]; the [`LadderReport`] says which rung served. Only
//! output the model got wrong ([`DropCause::Malformed`]) or a request shape
//! the vendor refused ([`DropCause::Rejected`]) descends: auth, quota,
//! timeouts and unreachable hosts go straight to scripted, because another
//! rung would not help and would spend more of the budget.
//!
//! One engine schema per action type lives here ([`action_schema`]); the
//! strict parser in `vendor.rs` stays the authority, and a test holds the
//! two together.

use serde_json::{json, Value};

use super::fallback::ScriptedFallback;
#[cfg(test)]
use super::protocol::ProposalAction;
use super::protocol::{AiError, RequestId};

/// Most rung drops one adapter keeps; the oldest go first.
pub const MAX_LADDER_LOG: usize = 256;
/// Name the engine schema carries on vendors that ask for one.
pub const SCHEMA_NAME: &str = "engine_proposal";
/// Name of the forced tool on vendors whose structured output is a tool call.
pub const NATIVE_TOOL: &str = "emit_proposal";
/// Longest model text the repair pass reads, in bytes.
const MAX_REPAIR_BYTES: usize = 65_536;

/// One rung of the ladder, best first.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Rung {
    /// The vendor's native structured output, constrained by the schema.
    Native,
    /// The vendor's JSON mode.
    JsonMode,
    /// Plain text with a strict parse and a bounded repair.
    Text,
    /// No model answer: the scripted fallback.
    Scripted,
}

impl Rung {
    /// Short name for logs and cassette entries.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Rung::Native => "native",
            Rung::JsonMode => "json",
            Rung::Text => "text",
            Rung::Scripted => "scripted",
        }
    }

    /// The rung below this one; scripted is the floor.
    #[must_use]
    pub fn below(self) -> Rung {
        match self {
            Rung::Native => Rung::JsonMode,
            Rung::JsonMode => Rung::Text,
            Rung::Text | Rung::Scripted => Rung::Scripted,
        }
    }
}

/// Which model rungs a backend can serve. Text is always served.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RungSupport {
    /// The backend has a native structured-output feature.
    pub native: bool,
    /// The backend has a JSON mode.
    pub json_mode: bool,
}

impl RungSupport {
    /// Native structured output and JSON mode, besides text.
    pub const FULL: Self = Self {
        native: true,
        json_mode: true,
    };

    /// Text only.
    pub const TEXT_ONLY: Self = Self {
        native: false,
        json_mode: false,
    };

    /// True when the backend can serve `rung`.
    #[must_use]
    pub const fn offers(self, rung: Rung) -> bool {
        match rung {
            Rung::Native => self.native,
            Rung::JsonMode => self.json_mode,
            Rung::Text | Rung::Scripted => true,
        }
    }
}

/// Where on the ladder an adapter starts. The default is the text rung,
/// which is what every adapter did before the ladder existed; a caller opts
/// into a higher start per adapter.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LadderPlan {
    start: Rung,
}

impl Default for LadderPlan {
    fn default() -> Self {
        Self::text()
    }
}

impl LadderPlan {
    /// Start at the vendor's native structured output.
    #[must_use]
    pub const fn native() -> Self {
        Self {
            start: Rung::Native,
        }
    }

    /// Start at the vendor's JSON mode.
    #[must_use]
    pub const fn json_mode() -> Self {
        Self {
            start: Rung::JsonMode,
        }
    }

    /// Start at plain text.
    #[must_use]
    pub const fn text() -> Self {
        Self { start: Rung::Text }
    }

    /// The first rung tried.
    #[must_use]
    pub const fn start(self) -> Rung {
        self.start
    }
}

/// Why a rung was left.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DropCause {
    /// The backend does not offer the rung.
    Unsupported,
    /// The model's answer failed the strict grammar.
    Malformed,
    /// The vendor refused the request shape (HTTP 400 or 422).
    Rejected,
    /// Something a lower rung cannot fix: auth, quota, timeout, no host.
    Error(AiError),
}

/// Why one attempt failed, and what it says about the next step.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RungFailure {
    /// The contract error this failure becomes if it is the last.
    pub error: AiError,
    /// What to log, and whether the next rung is worth trying.
    pub cause: DropCause,
    /// True when an HTTP exchange completed before the failure.
    pub exchanged: bool,
}

impl RungFailure {
    /// The model answered, and the answer failed the grammar.
    #[must_use]
    pub fn malformed() -> Self {
        Self {
            error: AiError::MalformedOutput,
            cause: DropCause::Malformed,
            exchanged: true,
        }
    }

    /// The vendor refused the request shape.
    #[must_use]
    pub fn rejected(error: AiError) -> Self {
        Self {
            error,
            cause: DropCause::Rejected,
            exchanged: true,
        }
    }

    /// A failure no lower rung can fix.
    #[must_use]
    pub fn fatal(error: AiError, exchanged: bool) -> Self {
        Self {
            error,
            cause: DropCause::Error(error),
            exchanged,
        }
    }

    fn descends(self) -> bool {
        matches!(self.cause, DropCause::Malformed | DropCause::Rejected)
    }
}

/// One logged step down the ladder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RungDrop {
    /// Request the drop belongs to.
    pub request: RequestId,
    /// Rung left.
    pub from: Rung,
    /// Rung entered.
    pub to: Rung,
    /// Why.
    pub cause: DropCause,
}

impl RungDrop {
    /// One bounded log line: fixed labels and integers only, no model text.
    #[must_use]
    pub fn notice(&self) -> String {
        let cause = match self.cause {
            DropCause::Unsupported => String::from("unsupported"),
            DropCause::Malformed => String::from("malformed"),
            DropCause::Rejected => String::from("rejected"),
            DropCause::Error(error) => format!("error:{error:?}"),
        };
        format!(
            "provider.ladder request={} {}->{} cause={cause}",
            self.request.0,
            self.from.label(),
            self.to.label()
        )
    }
}

/// What one request did on the ladder.
#[derive(Clone, PartialEq, Debug)]
pub struct LadderReport {
    /// Request reported on.
    pub request: RequestId,
    /// The rung that served, or [`Rung::Scripted`] when none did.
    pub used: Rung,
    /// Every step down, in order.
    pub drops: Vec<RungDrop>,
    /// The scripted service standing in when `used` is scripted and the
    /// session policy allows one. Nothing from the model is in it.
    pub scripted: Option<ScriptedFallback>,
}

/// The ladder's verdict: the value or the last failure, plus the report.
#[derive(Clone, PartialEq, Debug)]
pub struct Descent<T> {
    /// The first rung's value that passed, or the failure that ended it.
    pub result: Result<T, RungFailure>,
    /// Rung used and drops logged.
    pub report: LadderReport,
}

/// Run the ladder. `attempt` asks the backend on one rung and returns a
/// value that already passed the strict grammar, or why it did not. Rungs the
/// backend lacks are skipped with a logged drop; the loop ends at the first
/// value or on the scripted floor.
pub fn descend<T>(
    request: RequestId,
    plan: LadderPlan,
    support: RungSupport,
    mut attempt: impl FnMut(Rung) -> Result<T, RungFailure>,
) -> Descent<T> {
    let mut drops = Vec::new();
    let mut rung = plan.start();
    loop {
        while !support.offers(rung) {
            let to = rung.below();
            drops.push(RungDrop {
                request,
                from: rung,
                to,
                cause: DropCause::Unsupported,
            });
            rung = to;
        }
        let failure = match attempt(rung) {
            Ok(value) => return Descent::served(request, rung, drops, value),
            Err(failure) => failure,
        };
        let to = if failure.descends() {
            rung.below()
        } else {
            Rung::Scripted
        };
        drops.push(RungDrop {
            request,
            from: rung,
            to,
            cause: failure.cause,
        });
        if to == Rung::Scripted {
            return Descent {
                result: Err(failure),
                report: LadderReport {
                    request,
                    used: Rung::Scripted,
                    drops,
                    scripted: None,
                },
            };
        }
        rung = to;
    }
}

impl<T> Descent<T> {
    fn served(request: RequestId, used: Rung, drops: Vec<RungDrop>, value: T) -> Self {
        Self {
            result: Ok(value),
            report: LadderReport {
                request,
                used,
                drops,
                scripted: None,
            },
        }
    }
}

/// The name of an action type, as the grammar spells it. The match is
/// exhaustive: a new [`ProposalAction`] cannot compile the test below until it
/// has a name, and [`action_schema`] cannot be forgotten once it does.
#[cfg(test)]
fn action_name(action: &ProposalAction) -> &'static str {
    match action {
        ProposalAction::Noop => "noop",
        ProposalAction::SetPosition { .. } => "set_position",
        ProposalAction::SetLabel { .. } => "set_label",
    }
}

/// Every action type's name, in grammar order.
pub const ACTION_NAMES: [&str; 3] = ["noop", "set_position", "set_label"];

/// The engine schema for one action type, or `None` for an unknown name. The
/// schema states shape and types; the bounds (coordinate range, label length)
/// are in the descriptions and enforced by the strict parser, because strict
/// vendor schema modes accept only a keyword subset.
#[must_use]
pub fn action_schema(name: &str) -> Option<Value> {
    let (body, required): (Value, &[&str]) = match name {
        "noop" => (json!({"type": "boolean", "enum": [true]}), &["noop"]),
        "set_position" => (
            json!({
                "type": "object",
                "properties": {
                    "object": {"type": "integer", "description": "stable object id, 0 or more"},
                    "xyz_cm": {
                        "type": "array",
                        "items": {"type": "integer"},
                        "description": "x, y, z in centimeters, three values, each at most 1000000 in magnitude"
                    }
                },
                "required": ["object", "xyz_cm"],
                "additionalProperties": false
            }),
            &["set_position"],
        ),
        "set_label" => (
            json!({
                "type": "object",
                "properties": {
                    "object": {"type": "integer", "description": "stable object id, 0 or more"},
                    "label": {"type": "string", "description": "1 to 128 characters, no control characters"}
                },
                "required": ["object", "label"],
                "additionalProperties": false
            }),
            &["set_label"],
        ),
        _ => return None,
    };
    let key = required[0];
    Some(json!({
        "type": "object",
        "properties": {key: body},
        "required": required,
        "additionalProperties": false
    }))
}

/// The engine schema for a whole proposal: `actions` (each one action type)
/// and `diagnostics`.
#[must_use]
pub fn proposal_schema() -> Value {
    let actions: Vec<Value> = ACTION_NAMES
        .iter()
        .filter_map(|name| action_schema(name))
        .collect();
    json!({
        "type": "object",
        "properties": {
            "actions": {"type": "array", "items": {"anyOf": actions}},
            "diagnostics": {
                "type": "array",
                "items": {"type": "string"},
                "description": "single-line notes"
            }
        },
        "required": ["actions", "diagnostics"],
        "additionalProperties": false
    })
}

/// Bounded repair of model text that is not itself a JSON object: cut the
/// first balanced `{...}` out of surrounding prose or a Markdown fence, then
/// drop trailing commas. Nothing is added, renamed or guessed, the input is
/// read once per step and capped at 64 KiB, and the result is still only
/// text: the strict grammar decides. `None` when there is nothing to repair
/// (no balanced object, input too large, or the repair changed nothing).
#[must_use]
pub fn repair_text(content: &str) -> Option<String> {
    if content.len() > MAX_REPAIR_BYTES {
        return None;
    }
    let object = first_object(content)?;
    let fixed = without_trailing_commas(object);
    (fixed != content.trim()).then_some(fixed)
}

/// The first balanced `{...}` in `text`, string- and escape-aware.
fn first_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let mut depth = 0usize;
    let (mut in_string, mut escaped) = (false, false);
    for (offset, c) in text[start..].char_indices() {
        if in_string {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..start + offset + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

/// `text` without any `,` that sits outside a string and is followed, past
/// whitespace, by `}` or `]`.
fn without_trailing_commas(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let (mut in_string, mut escaped) = (false, false);
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if in_string {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_string = false,
                _ => {}
            }
            out.push(c);
            continue;
        }
        if c == '"' {
            in_string = true;
        } else if c == ',' {
            let mut ahead = chars.clone();
            let next = ahead.find(|n| !n.is_whitespace());
            if matches!(next, Some('}' | ']')) {
                continue;
            }
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: RequestId = RequestId(7);
    const FULL: RungSupport = RungSupport {
        native: true,
        json_mode: true,
    };

    /// A scripted backend: answers per rung from `outcomes`, recording the
    /// rungs it was asked on.
    fn run(
        plan: LadderPlan,
        support: RungSupport,
        outcomes: &[(Rung, Result<u32, RungFailure>)],
    ) -> (Descent<u32>, Vec<Rung>) {
        let mut asked = Vec::new();
        let descent = descend(ID, plan, support, |rung| {
            asked.push(rung);
            outcomes
                .iter()
                .find(|(r, _)| *r == rung)
                .map(|(_, out)| *out)
                .unwrap_or(Err(RungFailure::malformed()))
        });
        (descent, asked)
    }

    #[test]
    fn the_first_good_rung_serves_and_nothing_is_logged() {
        let (descent, asked) = run(LadderPlan::native(), FULL, &[(Rung::Native, Ok(1))]);
        assert_eq!(descent.result, Ok(1));
        assert_eq!(asked, vec![Rung::Native]);
        assert_eq!(descent.report.used, Rung::Native);
        assert!(descent.report.drops.is_empty());
    }

    #[test]
    fn malformed_output_drops_one_rung_and_logs_it() {
        let (descent, asked) = run(
            LadderPlan::native(),
            FULL,
            &[
                (Rung::Native, Err(RungFailure::malformed())),
                (Rung::JsonMode, Ok(2)),
            ],
        );
        assert_eq!(descent.result, Ok(2), "the lower rung's value serves");
        assert_eq!(asked, vec![Rung::Native, Rung::JsonMode]);
        assert_eq!(descent.report.used, Rung::JsonMode);
        assert_eq!(
            descent.report.drops,
            vec![RungDrop {
                request: ID,
                from: Rung::Native,
                to: Rung::JsonMode,
                cause: DropCause::Malformed
            }]
        );
        assert_eq!(
            descent.report.drops[0].notice(),
            "provider.ladder request=7 native->json cause=malformed"
        );
    }

    #[test]
    fn garbage_at_every_rung_ends_scripted_with_no_value() {
        let (descent, asked) = run(LadderPlan::native(), FULL, &[]);
        assert_eq!(asked, vec![Rung::Native, Rung::JsonMode, Rung::Text]);
        assert_eq!(descent.result, Err(RungFailure::malformed()));
        assert_eq!(descent.report.used, Rung::Scripted);
        let path: Vec<(Rung, Rung)> = descent
            .report
            .drops
            .iter()
            .map(|drop| (drop.from, drop.to))
            .collect();
        assert_eq!(
            path,
            vec![
                (Rung::Native, Rung::JsonMode),
                (Rung::JsonMode, Rung::Text),
                (Rung::Text, Rung::Scripted)
            ]
        );
    }

    #[test]
    fn rungs_a_backend_lacks_are_skipped_and_logged() {
        let no_json = RungSupport {
            native: true,
            json_mode: false,
        };
        let (descent, asked) = run(
            LadderPlan::native(),
            no_json,
            &[
                (Rung::Native, Err(RungFailure::malformed())),
                (Rung::Text, Ok(3)),
            ],
        );
        assert_eq!(
            asked,
            vec![Rung::Native, Rung::Text],
            "json mode never asked"
        );
        assert_eq!(descent.report.used, Rung::Text);
        let causes: Vec<DropCause> = descent.report.drops.iter().map(|d| d.cause).collect();
        assert_eq!(
            causes,
            vec![DropCause::Malformed, DropCause::Unsupported],
            "both the miss and the skipped rung are logged"
        );
        // A plan that starts above what the backend offers lands lower.
        let text_only = RungSupport {
            native: false,
            json_mode: false,
        };
        let (descent, asked) = run(LadderPlan::native(), text_only, &[(Rung::Text, Ok(4))]);
        assert_eq!(asked, vec![Rung::Text]);
        assert_eq!(descent.report.drops.len(), 2);
        assert!(descent
            .report
            .drops
            .iter()
            .all(|d| d.cause == DropCause::Unsupported));
    }

    #[test]
    fn errors_a_lower_rung_cannot_fix_go_straight_to_scripted() {
        for error in [
            AiError::QuotaAuth,
            AiError::Timeout,
            AiError::Unavailable,
            AiError::UnsupportedCapability,
        ] {
            let (descent, asked) = run(
                LadderPlan::native(),
                FULL,
                &[(Rung::Native, Err(RungFailure::fatal(error, false)))],
            );
            assert_eq!(asked, vec![Rung::Native], "{error:?}: no second attempt");
            assert_eq!(descent.report.used, Rung::Scripted);
            assert_eq!(descent.result.map_err(|f| f.error), Err(error));
            assert_eq!(
                descent.report.drops[0].cause,
                DropCause::Error(error),
                "{error:?}"
            );
        }
        let (descent, asked) = run(
            LadderPlan::native(),
            FULL,
            &[
                (
                    Rung::Native,
                    Err(RungFailure::rejected(AiError::Unavailable)),
                ),
                (Rung::JsonMode, Ok(5)),
            ],
        );
        assert_eq!(
            asked,
            vec![Rung::Native, Rung::JsonMode],
            "a refused shape descends"
        );
        assert_eq!(descent.report.drops[0].cause, DropCause::Rejected);
    }

    #[test]
    fn the_default_plan_starts_at_text() {
        assert_eq!(LadderPlan::default().start(), Rung::Text);
        assert_eq!(Rung::Text.below(), Rung::Scripted);
        assert_eq!(Rung::Scripted.below(), Rung::Scripted, "the floor holds");
        assert!(Rung::Native < Rung::JsonMode && Rung::JsonMode < Rung::Text);
    }

    #[test]
    fn repair_cuts_the_object_out_of_prose_fences_and_trailing_commas() {
        let obj = r#"{"actions": [{"noop": true}], "diagnostics": ["ok"]}"#;
        let cases = [
            format!("Here is the proposal:\n{obj}\nHope that helps."),
            format!("```json\n{obj}\n```"),
            format!("```\n{obj}\n```\nthanks"),
            String::from(r#"{"actions": [{"noop": true},], "diagnostics": ["ok",]}"#),
            String::from(
                "text {\"actions\": [ {\"noop\": true} , ] , \"diagnostics\": [\"ok\"] , } text",
            ),
        ];
        for case in cases {
            let fixed = repair_text(&case).unwrap_or_default();
            let value: Value = serde_json::from_str(&fixed)
                .unwrap_or_else(|e| panic!("repair of {case:?} gave {fixed:?}: {e}"));
            assert_eq!(value["actions"][0]["noop"], true, "{case:?}");
        }
    }

    #[test]
    fn repair_respects_strings_and_adds_nothing() {
        // Braces and commas inside strings are not structure.
        let tricky = r#"note {"actions": [], "diagnostics": ["a } b", "c, ]", "q\"}"],} end"#;
        let fixed = repair_text(tricky).expect("repairs");
        let value: Value = serde_json::from_str(&fixed).expect("valid JSON");
        assert_eq!(value["diagnostics"][0], "a } b");
        assert_eq!(value["diagnostics"][1], "c, ]");
        assert_eq!(value["diagnostics"][2], "q\"}");
        // An already clean object is not a repair; wrong content stays wrong.
        let clean = r#"{"actions": [{"teleport": 1}], "diagnostics": []}"#;
        assert_eq!(repair_text(clean), None);
        let fixed = repair_text(&format!("x {clean} y")).expect("cut out");
        assert_eq!(fixed, clean, "the grammar, not the repair, rejects it");
    }

    #[test]
    fn repair_refuses_what_it_cannot_bound_or_close() {
        assert_eq!(repair_text(""), None);
        assert_eq!(repair_text("no braces at all"), None);
        assert_eq!(repair_text(r#"{"actions": ["#), None, "unbalanced");
        assert_eq!(repair_text(r#"{"a": "unterminated}"#), None);
        let huge = format!("{{\"d\": \"{}\"}}", "x".repeat(MAX_REPAIR_BYTES));
        assert_eq!(repair_text(&huge), None, "over the byte cap");
        // Pathological nesting is one linear pass, and unclosed.
        let deep = "{".repeat(MAX_REPAIR_BYTES);
        assert_eq!(repair_text(&deep), None);
    }

    #[test]
    fn every_action_type_has_one_schema() {
        let samples = [
            ProposalAction::Noop,
            ProposalAction::SetPosition {
                object: 1,
                xyz_cm: [0, 0, 0],
            },
            ProposalAction::SetLabel {
                object: 1,
                label: String::from("gate"),
            },
        ];
        let names: Vec<&str> = samples.iter().map(action_name).collect();
        assert_eq!(names, ACTION_NAMES, "grammar order, every variant");
        for name in ACTION_NAMES {
            let schema = action_schema(name).expect("schema per action type");
            assert_eq!(schema["required"], json!([name]));
            assert_eq!(schema["additionalProperties"], false);
        }
        assert_eq!(action_schema("teleport"), None);
        let whole = proposal_schema();
        let any_of = whole["properties"]["actions"]["items"]["anyOf"]
            .as_array()
            .expect("anyOf list");
        assert_eq!(any_of.len(), ACTION_NAMES.len());
        assert_eq!(whole["required"], json!(["actions", "diagnostics"]));
    }

    /// A small structural check for the keyword subset the engine schema
    /// uses: enough to hold the schema and the strict parser together.
    fn conforms(schema: &Value, value: &Value) -> bool {
        if let Some(options) = schema.get("anyOf").and_then(Value::as_array) {
            return options.iter().any(|option| conforms(option, value));
        }
        if let Some(allowed) = schema.get("enum").and_then(Value::as_array) {
            if !allowed.contains(value) {
                return false;
            }
        }
        match schema.get("type").and_then(Value::as_str) {
            Some("object") => {
                let Some(object) = value.as_object() else {
                    return false;
                };
                let props = schema["properties"].as_object();
                let known = |key: &str| props.is_some_and(|p| p.contains_key(key));
                let required = schema["required"].as_array().cloned().unwrap_or_default();
                required
                    .iter()
                    .filter_map(Value::as_str)
                    .all(|key| object.contains_key(key))
                    && object.keys().all(|key| known(key))
                    && object
                        .iter()
                        .all(|(key, item)| conforms(&schema["properties"][key], item))
            }
            Some("array") => value
                .as_array()
                .is_some_and(|items| items.iter().all(|item| conforms(&schema["items"], item))),
            Some("integer") => value.is_i64() || value.is_u64(),
            Some("string") => value.is_string(),
            Some("boolean") => value.is_boolean(),
            _ => true,
        }
    }

    /// The schema is a superset of the grammar: whatever the strict parser
    /// accepts the schema accepts, and structural nonsense fails both.
    #[test]
    fn the_schema_and_the_strict_parser_agree() {
        let schema = proposal_schema();
        let ask = crate::mock::fixtures::request(1, 3);
        let good = [
            r#"{"actions": [], "diagnostics": []}"#,
            r#"{"actions": [{"noop": true}], "diagnostics": ["a"]}"#,
            r#"{"actions": [{"set_position": {"object": 2, "xyz_cm": [10, -20, 30]}}], "diagnostics": []}"#,
            r#"{"actions": [{"set_label": {"object": 3, "label": "gate"}}, {"noop": true}], "diagnostics": ["x", "y"]}"#,
        ];
        for body in good {
            let value: Value = serde_json::from_str(body).expect("json");
            assert!(conforms(&schema, &value), "schema accepts {body}");
            assert!(
                crate::vendor::parse_proposal_content(body, &ask, "").is_ok(),
                "parser accepts {body}"
            );
        }
        let structurally_bad = [
            r#"{"actions": [{"teleport": {"object": 1}}], "diagnostics": []}"#,
            r#"{"actions": [{"noop": true, "extra": 1}], "diagnostics": []}"#,
            r#"{"actions": [{"noop": false}], "diagnostics": []}"#,
            r#"{"actions": [{"set_position": {"object": 1, "xyz_cm": "origin"}}], "diagnostics": []}"#,
            r#"{"actions": [{"set_label": {"object": 1}}], "diagnostics": []}"#,
            r#"{"actions": "none", "diagnostics": []}"#,
            r#"{"actions": [], "diagnostics": [1]}"#,
            r#"{"actions": [], "diagnostics": [], "free_text": "hi"}"#,
            r#"[1, 2]"#,
        ];
        for body in structurally_bad {
            let value: Value = serde_json::from_str(body).expect("json");
            assert!(!conforms(&schema, &value), "schema rejects {body}");
            assert!(
                crate::vendor::parse_proposal_content(body, &ask, "").is_err(),
                "parser rejects {body}"
            );
        }
    }
}

/// The ladder through the real adapters: loopback stubs answer per rung, the
/// bodies they received say what each rung asked for.
#[cfg(test)]
mod wire_tests {
    use super::*;
    use crate::fallback::FallbackCause;
    use crate::mock::fixtures;
    use crate::pins::RequestPins;
    use crate::policy::{AiSessionPolicy, Fallback, ProviderCapabilities};
    use crate::protocol::{AdapterKind, PrivacyMode, ProposalAction};
    use crate::stub::{chat_ok, settle, StubServer};
    use crate::transport::parse_endpoint;
    use crate::vendor::VendorSecret;
    use crate::{
        AnthropicAdapter, GeminiAdapter, LocalAdapter, MockStep, OllamaAdapter, OpenAiAdapter,
        ProviderAdapter, RemoteAdapter,
    };

    const NOOP: &str = r#"{"actions": [{"noop": true}], "diagnostics": ["ladder:stub"]}"#;

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Vendor {
        OpenAi,
        Remote,
        Local,
        Anthropic,
        Gemini,
        Ollama,
    }

    const ALL: [Vendor; 6] = [
        Vendor::OpenAi,
        Vendor::Remote,
        Vendor::Local,
        Vendor::Anthropic,
        Vendor::Gemini,
        Vendor::Ollama,
    ];

    impl Vendor {
        fn is_remote(self) -> bool {
            !matches!(self, Vendor::Local | Vendor::Ollama)
        }

        fn support(self) -> RungSupport {
            match self {
                Vendor::Anthropic => RungSupport {
                    native: true,
                    json_mode: false,
                },
                _ => RungSupport::FULL,
            }
        }

        /// The vendor's 200 body carrying `text` as the assistant answer.
        fn reply(self, text: &str) -> String {
            match self {
                Vendor::OpenAi | Vendor::Remote | Vendor::Local => chat_ok(text, Some((3, 2))),
                Vendor::Anthropic => json!({
                    "content": [{"type": "text", "text": text}],
                    "usage": {"input_tokens": 3, "output_tokens": 2},
                })
                .to_string(),
                Vendor::Gemini => json!({
                    "candidates": [{"content": {"parts": [{"text": text}]}}],
                    "usageMetadata": {"promptTokenCount": 3, "candidatesTokenCount": 2},
                })
                .to_string(),
                Vendor::Ollama => json!({
                    "message": {"content": text},
                    "prompt_eval_count": 3,
                    "eval_count": 2,
                })
                .to_string(),
            }
        }

        /// The 200 body this vendor sends on `rung`: Anthropic's native rung
        /// answers with a forced tool call, everyone else with text.
        fn reply_on(self, rung: Rung, text: &str) -> String {
            if self == Vendor::Anthropic && rung == Rung::Native {
                let input: Value = serde_json::from_str(text).expect("tool input is JSON");
                return json!({
                    "content": [{"type": "tool_use", "name": NATIVE_TOOL, "input": input}],
                    "usage": {"input_tokens": 3, "output_tokens": 2},
                })
                .to_string();
            }
            self.reply(text)
        }

        /// Does `body` carry the request shape `rung` asks this vendor for?
        fn asks_for(self, rung: Rung, body: &Value) -> bool {
            match (self, rung) {
                (Vendor::Anthropic, Rung::Native) => {
                    body["tool_choice"]["name"] == NATIVE_TOOL
                        && body["tools"]
                            .as_array()
                            .is_some_and(|t| t.iter().any(|t| t["name"] == NATIVE_TOOL))
                }
                (Vendor::Anthropic, _) => body.get("tool_choice").is_none(),
                (Vendor::Gemini, Rung::Native) => {
                    body["generationConfig"]["responseJsonSchema"] == proposal_schema()
                }
                (Vendor::Gemini, Rung::JsonMode) => {
                    body["generationConfig"]["responseMimeType"] == "application/json"
                        && body["generationConfig"].get("responseJsonSchema").is_none()
                }
                (Vendor::Gemini, _) => body["generationConfig"].get("responseMimeType").is_none(),
                (Vendor::Ollama, Rung::Native) => body["format"] == proposal_schema(),
                (Vendor::Ollama, Rung::JsonMode) => body["format"] == "json",
                (Vendor::Ollama, _) => body.get("format").is_none(),
                (_, Rung::Native) => {
                    body["response_format"]["type"] == "json_schema"
                        && body["response_format"]["json_schema"]["strict"] == true
                        && body["response_format"]["json_schema"]["schema"] == proposal_schema()
                }
                (_, Rung::JsonMode) => body["response_format"]["type"] == "json_object",
                (_, _) => body.get("response_format").is_none(),
            }
        }
    }

    /// What one request did, read off the adapter and the stub.
    struct Run {
        outcome: crate::AdapterOutcome,
        report: Option<LadderReport>,
        events: Vec<RungDrop>,
        mutations: u64,
        attempts: u64,
        bodies: Vec<Value>,
    }

    /// Snapshot the stub's request bodies after one adapter call. The stub
    /// records on its own thread before answering, so the snapshot usually
    /// already holds every attempt; a short poll covers scheduling delay.
    /// Bodies that are still missing or not JSON are setup races (a truncated
    /// loopback read under parallel load), not ladder behavior: they become
    /// `Value::Null` so the caller retries with a fresh stub instead of
    /// panicking on an empty string. Real request shapes are JSON objects,
    /// never null.
    fn capture_bodies(stub: &StubServer, attempts: u64) -> Vec<Value> {
        let mut raw = stub.bodies();
        for _ in 0..20 {
            if raw.len() as u64 >= attempts {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            raw = stub.bodies();
        }
        raw.iter()
            .map(|b| serde_json::from_str(b).unwrap_or(Value::Null))
            .collect()
    }

    /// True when `run` holds a complete setup: every socket attempt was
    /// recorded and every recorded body is a JSON object. False means the
    /// loopback stub truncated a read under load; the caller retries.
    fn bodies_complete(run: &Run) -> bool {
        u64::try_from(run.bodies.len()).unwrap_or(u64::MAX) == run.attempts
            && !run.bodies.iter().any(Value::is_null)
    }

    macro_rules! drive {
        ($adapter:expr, $ask:expr, $policy:expr, $caps:expr, $stub:expr) => {{
            let mut adapter = $adapter;
            let outcome = adapter.call($ask, $policy, $caps);
            let attempts = adapter.socket_attempts();
            let bodies = capture_bodies(&$stub, attempts);
            Run {
                outcome,
                report: adapter.ladder_report().cloned(),
                events: adapter.ladder_events().to_vec(),
                mutations: adapter.mutations(),
                attempts,
                bodies,
            }
        }};
    }

    /// One request of `vendor` on `plan` against a stub that answers
    /// `script` in order (the last entry repeats).
    fn run_with(
        vendor: Vendor,
        plan: LadderPlan,
        script: &[(u16, String)],
        max_ms: u64,
    ) -> (Run, StubServer) {
        settle(
            |_| {
                let stub = StubServer::spawn_script(script.to_vec());
                let url = stub.url();
                let endpoint = parse_endpoint(&url).expect("stub url parses");
                let secret = || VendorSecret::Direct(String::from("ladder-token"));
                let pins = RequestPins::default().with_ladder(plan);
                let (kind, mode, caps) = if vendor.is_remote() {
                    (
                        AdapterKind::Remote,
                        PrivacyMode::RemoteAllowed,
                        ProviderCapabilities::remote_http(),
                    )
                } else {
                    (
                        AdapterKind::Local,
                        PrivacyMode::LocalOnly,
                        ProviderCapabilities::local_http(),
                    )
                };
                let mut ask = fixtures::request(7, 3);
                ask.model.adapter = kind;
                ask.privacy = mode;
                ask.budget.max_ms = max_ms;
                let policy = AiSessionPolicy {
                    adapters: vec![kind],
                    mode,
                    destinations: if vendor.is_remote() {
                        vec![url]
                    } else {
                        Vec::new()
                    },
                    scopes: Vec::new(),
                    request_limit: 8,
                    max_concurrency: 1,
                    spend_limit: None,
                    fallback: Fallback {
                        order: Vec::new(),
                        scripted_ok: true,
                    },
                };
                let s = vec![MockStep::Succeed];
                let run = match vendor {
                    Vendor::OpenAi => {
                        let a = OpenAiAdapter::new(s, endpoint, secret()).with_pins(pins);
                        drive!(a, &ask, &policy, &caps, stub)
                    }
                    Vendor::Remote => {
                        let a = RemoteAdapter::new(s, endpoint, secret()).with_pins(pins);
                        drive!(a, &ask, &policy, &caps, stub)
                    }
                    Vendor::Local => {
                        let a = LocalAdapter::new(s, endpoint)
                            .expect("loopback")
                            .with_pins(pins);
                        drive!(a, &ask, &policy, &caps, stub)
                    }
                    Vendor::Anthropic => {
                        let a = AnthropicAdapter::new(s, endpoint, secret()).with_pins(pins);
                        drive!(a, &ask, &policy, &caps, stub)
                    }
                    Vendor::Gemini => {
                        let a = GeminiAdapter::new(s, endpoint, secret()).with_pins(pins);
                        drive!(a, &ask, &policy, &caps, stub)
                    }
                    Vendor::Ollama => {
                        let a = OllamaAdapter::new(s, endpoint)
                            .expect("loopback")
                            .with_pins(pins);
                        drive!(a, &ask, &policy, &caps, stub)
                    }
                };
                (run, stub)
            },
            |(run, stub)| {
                if !bodies_complete(run) {
                    return false;
                }
                if run.outcome.result == Err(AiError::Unavailable) {
                    return stub.hits() > 0;
                }
                true
            },
        )
    }

    fn run(vendor: Vendor, plan: LadderPlan, script: &[(u16, String)]) -> (Run, StubServer) {
        run_with(vendor, plan, script, 5_000)
    }

    fn ok(body: String) -> (u16, String) {
        (200, body)
    }

    /// The first rung a backend with `support` is asked on under `plan`.
    fn first_rung(support: RungSupport, plan: LadderPlan) -> Rung {
        let mut rung = plan.start();
        while !support.offers(rung) {
            rung = rung.below();
        }
        rung
    }

    #[test]
    fn every_backend_reports_the_rung_it_used_and_asks_for_that_shape() {
        for vendor in ALL {
            for plan in [
                LadderPlan::native(),
                LadderPlan::json_mode(),
                LadderPlan::text(),
            ] {
                let want = first_rung(vendor.support(), plan);
                let (got, stub) = run(vendor, plan, &[ok(vendor.reply_on(want, NOOP))]);
                let label = format!("{vendor:?} from {:?}", plan.start());
                let result = got
                    .outcome
                    .result
                    .as_ref()
                    .unwrap_or_else(|e| panic!("{label}: {e:?}"));
                assert_eq!(
                    result.proposal.as_ref().map(|p| p.actions.clone()),
                    Some(vec![ProposalAction::Noop]),
                    "{label}"
                );
                let report = got.report.as_ref().expect("report");
                assert_eq!(report.used, want, "{label}");
                assert_eq!(report.scripted, None, "{label}");
                let skipped = got
                    .events
                    .iter()
                    .filter(|d| d.cause == DropCause::Unsupported)
                    .count();
                assert_eq!(skipped, got.events.len(), "{label}: only skips, no misses");
                assert_eq!(stub.hits(), 1, "{label}: served on the first ask");
                assert_eq!(got.mutations, 1, "{label}");
                assert!(
                    vendor.asks_for(want, &got.bodies[0]),
                    "{label}: {}",
                    got.bodies[0]
                );
            }
        }
    }

    #[test]
    fn a_backend_that_returns_malformed_json_drops_one_rung_and_logs_it() {
        for vendor in ALL {
            let second = first_rung(vendor.support(), LadderPlan::json_mode());
            let script = [
                ok(vendor.reply("{ this is not json")),
                ok(vendor.reply_on(second, NOOP)),
            ];
            let (got, stub) = run(vendor, LadderPlan::native(), &script);
            let label = format!("{vendor:?}");
            assert!(got.outcome.result.is_ok(), "{label}: the lower rung serves");
            assert!(got.outcome.mutated, "{label}");
            assert_eq!(stub.hits(), 2, "{label}");
            assert_eq!(got.report.as_ref().map(|r| r.used), Some(second), "{label}");
            assert_eq!(
                got.events[0],
                RungDrop {
                    request: RequestId(7),
                    from: Rung::Native,
                    to: Rung::JsonMode,
                    cause: DropCause::Malformed
                },
                "{label}: the first drop is logged with its cause"
            );
            assert!(got.events.iter().all(|d| d.to != Rung::Scripted), "{label}");
            assert!(
                vendor.asks_for(Rung::Native, &got.bodies[0]),
                "{label}: first ask native"
            );
            assert!(
                vendor.asks_for(second, &got.bodies[1]),
                "{label}: then {second:?}"
            );
        }
    }

    #[test]
    fn garbage_at_every_rung_ends_scripted_and_applies_nothing() {
        for vendor in ALL {
            let script = [ok(vendor.reply("complete garbage, no braces"))];
            let (got, stub) = run(vendor, LadderPlan::native(), &script);
            let label = format!("{vendor:?}");
            let rungs = [Rung::Native, Rung::JsonMode, Rung::Text]
                .iter()
                .filter(|r| vendor.support().offers(**r))
                .count();
            assert_eq!(got.outcome.result, Err(AiError::MalformedOutput), "{label}");
            assert!(!got.outcome.mutated, "{label}: nothing applied");
            assert_eq!(got.mutations, 0, "{label}");
            assert_eq!(stub.hits(), rungs as u64, "{label}: every rung asked once");
            assert_eq!(got.attempts, rungs as u64, "{label}");
            let report = got.report.as_ref().expect("report");
            assert_eq!(report.used, Rung::Scripted, "{label}");
            assert_eq!(
                report.drops.last().map(|d| d.to),
                Some(Rung::Scripted),
                "{label}"
            );
            let scripted = report
                .scripted
                .as_ref()
                .expect("scripted service stands in");
            assert_eq!(scripted.cause, FallbackCause::Refused, "{label}");
            assert_eq!(
                scripted.proposal.actions,
                vec![ProposalAction::Noop],
                "{label}"
            );
            let notices: Vec<String> = got.events.iter().map(RungDrop::notice).collect();
            assert!(
                notices
                    .iter()
                    .all(|n| n.starts_with("provider.ladder request=7 ")),
                "{label}: {notices:?}"
            );
        }
    }

    #[test]
    fn a_refused_shape_descends_but_auth_and_quota_end_the_ladder() {
        let valid = Vendor::OpenAi.reply(NOOP);
        for status in [400u16, 422] {
            let (got, stub) = run(
                Vendor::OpenAi,
                LadderPlan::native(),
                &[(status, String::from("{}")), ok(valid.clone())],
            );
            assert!(got.outcome.result.is_ok(), "{status}: JSON mode serves");
            assert_eq!(stub.hits(), 2, "{status}");
            assert_eq!(got.events[0].cause, DropCause::Rejected, "{status}");
            assert_eq!(got.report.as_ref().map(|r| r.used), Some(Rung::JsonMode));
        }
        for (status, error) in [
            (401u16, AiError::QuotaAuth),
            (403, AiError::QuotaAuth),
            (429, AiError::QuotaAuth),
            (408, AiError::Timeout),
            (503, AiError::Unavailable),
        ] {
            let (got, stub) = run_with(
                Vendor::OpenAi,
                LadderPlan::native(),
                &[(status, String::from("{}")), ok(valid.clone())],
                5_000,
            );
            assert_eq!(got.outcome.result, Err(error), "{status}");
            assert_eq!(stub.hits(), 1, "{status}: no second ask");
            let report = got.report.as_ref().expect("report");
            assert_eq!(report.used, Rung::Scripted, "{status}");
            assert_eq!(
                got.events,
                vec![RungDrop {
                    request: RequestId(7),
                    from: Rung::Native,
                    to: Rung::Scripted,
                    cause: DropCause::Error(error)
                }],
                "{status}"
            );
        }
    }

    #[test]
    fn the_request_budget_is_shared_across_rungs() {
        let slow = StubServer::spawn_delayed(Vendor::OpenAi.reply(NOOP), 400);
        let url = slow.url();
        let endpoint = parse_endpoint(&url).expect("stub url parses");
        let mut adapter = OpenAiAdapter::new(
            vec![MockStep::Succeed],
            endpoint,
            VendorSecret::Direct(String::from("ladder-token")),
        )
        .with_pins(RequestPins::default().with_ladder(LadderPlan::native()));
        let mut ask = fixtures::request(7, 3);
        ask.model.adapter = AdapterKind::Remote;
        ask.privacy = PrivacyMode::RemoteAllowed;
        ask.budget.max_ms = 100;
        let policy = AiSessionPolicy {
            adapters: vec![AdapterKind::Remote],
            mode: PrivacyMode::RemoteAllowed,
            destinations: vec![url],
            scopes: Vec::new(),
            request_limit: 8,
            max_concurrency: 1,
            spend_limit: None,
            fallback: Fallback {
                order: Vec::new(),
                scripted_ok: true,
            },
        };
        let out = adapter.call(&ask, &policy, &ProviderCapabilities::remote_http());
        assert!(
            matches!(out.result, Err(AiError::Timeout | AiError::Unavailable)),
            "{:?}",
            out.result
        );
        assert_eq!(
            adapter.socket_attempts(),
            1,
            "the spent budget ends the ladder"
        );
        assert_eq!(adapter.last_rung(), Some(Rung::Scripted));
    }

    #[test]
    fn the_text_rung_repairs_fenced_output_and_the_grammar_still_decides() {
        let fenced = format!("Sure!\n```json\n{NOOP}\n```\n");
        let script = [ok(Vendor::OpenAi.reply(&fenced))];
        let (got, stub) = run(Vendor::OpenAi, LadderPlan::text(), &script);
        assert!(got.outcome.result.is_ok(), "repaired on the text rung");
        assert_eq!(got.report.as_ref().map(|r| r.used), Some(Rung::Text));
        assert!(got.events.is_empty());
        assert_eq!(stub.hits(), 1);
        // Higher rungs get no repair: fenced output drops them to text.
        for plan in [LadderPlan::native(), LadderPlan::json_mode()] {
            let (got, stub) = run(Vendor::OpenAi, plan, &script);
            assert!(got.outcome.result.is_ok());
            assert_eq!(got.report.as_ref().map(|r| r.used), Some(Rung::Text));
            assert!(got.events.iter().all(|d| d.cause == DropCause::Malformed));
            assert_eq!(stub.hits(), got.events.len() as u64 + 1);
        }
        // Repair cuts text out; it never makes a wrong action right.
        let wrong = r#"Here: {"actions": [{"teleport": {"object": 1}}], "diagnostics": []}"#;
        let (got, _) = run(
            Vendor::OpenAi,
            LadderPlan::text(),
            &[ok(Vendor::OpenAi.reply(wrong))],
        );
        assert_eq!(got.outcome.result, Err(AiError::MalformedOutput));
        assert!(!got.outcome.mutated);
        assert_eq!(got.report.as_ref().map(|r| r.used), Some(Rung::Scripted));
    }

    #[test]
    fn anthropic_native_reads_only_the_forced_tool_call() {
        let tool = Vendor::Anthropic.reply_on(Rung::Native, NOOP);
        let (got, _) = run(Vendor::Anthropic, LadderPlan::native(), &[ok(tool)]);
        let result = got.outcome.result.as_ref().expect("tool call serves");
        assert_eq!(result.usage.input_tokens, Some(3));
        assert_eq!(got.report.as_ref().map(|r| r.used), Some(Rung::Native));
        let other_tool = json!({
            "content": [{"type": "tool_use", "name": "something_else", "input": {"actions": [], "diagnostics": []}}],
        })
        .to_string();
        let script = [ok(other_tool), ok(Vendor::Anthropic.reply(NOOP))];
        let (got, stub) = run(Vendor::Anthropic, LadderPlan::native(), &script);
        assert_eq!(stub.hits(), 2, "a foreign tool call is not the proposal");
        assert_eq!(got.events[0].cause, DropCause::Malformed);
        assert_eq!(got.report.as_ref().map(|r| r.used), Some(Rung::Text));
    }

    #[test]
    fn the_ladder_log_is_bounded() {
        let mut http = crate::vendor::HttpAdapter::new(
            Vec::new(),
            parse_endpoint("http://127.0.0.1:9/v1/chat/completions").expect("url parses"),
        );
        let template = RungDrop {
            request: RequestId(1),
            from: Rung::Native,
            to: Rung::JsonMode,
            cause: DropCause::Malformed,
        };
        for n in 0..(MAX_LADDER_LOG as u64 + 40) {
            http.note_ladder(LadderReport {
                request: RequestId(n),
                used: Rung::JsonMode,
                drops: vec![RungDrop {
                    request: RequestId(n),
                    ..template
                }],
                scripted: None,
            });
        }
        assert_eq!(http.ladder_log.len(), MAX_LADDER_LOG);
        assert_eq!(
            http.ladder_log[0].request,
            RequestId(40),
            "oldest dropped first"
        );
    }
}
