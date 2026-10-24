//! AD26: the one advice table over every refusal code, plus the multi-field
//! reply for a bad argument list.
//!
//! A refusal that leaves the door as a tool result carries a code, a message,
//! an [`Advice`] code from a short closed list, a retry flag and the one next
//! call. The retry flag and the next call come from the code tables in
//! `mcp_refusal`; this module owns the advice code, so a model branches on a
//! value from a fixed list instead of reading prose.
//!
//! A bad argument list answers once: every bad field rides in one `bad_args`
//! refusal as a JSON pointer, a reason code and a hint, with any secret-shaped
//! value redacted. Nothing is auto-corrected; the reply names what to send.

use super::mcp_refusal::{classify, Kind};
use super::OpDto;
use engine_editor::commands::MaterialName;
use engine_scene::component::ComponentKind;
use engine_scene::ids::ObjectId;
use rmcp::serde_json::{json, Value};

/// What a model should do next, as a value from a fixed list. The list is
/// closed on purpose: a new advice means a new `match` arm here, never a
/// free-form sentence at a refusal site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Advice {
    /// The call named a value outside its legal range: fix that argument.
    FixArgument,
    /// The verb's shape is unclear: `api.describe` it.
    DescribeVerb,
    /// The verb name is unknown: `api.search` for the name you meant.
    SearchVerb,
    /// Re-read the state the refusal named, then decide again.
    Reread,
    /// Re-run `tx.preview` at the current revision for a fresh approval.
    Repreview,
    /// Apply the previewed batch again under a fresh key.
    Reapply,
    /// The refusal is transient: the same intent can succeed shortly.
    RetryLater,
    /// The call is allowed but too wide or too costly: send less.
    NarrowCall,
    /// The caller cannot fix this from outside: report it to the operator.
    Escalate,
    /// No table row names the code. The completeness test forbids this for a
    /// code the crate produces, so it only ever marks a foreign string.
    Unknown,
}

impl Advice {
    /// Every advice code, in list order. A client reads this as the closed set.
    pub const ALL: [Self; 10] = [
        Self::FixArgument,
        Self::DescribeVerb,
        Self::SearchVerb,
        Self::Reread,
        Self::Repreview,
        Self::Reapply,
        Self::RetryLater,
        Self::NarrowCall,
        Self::Escalate,
        Self::Unknown,
    ];

    /// The wire word for this advice code.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FixArgument => "fix_argument",
            Self::DescribeVerb => "describe_verb",
            Self::SearchVerb => "search_verb",
            Self::Reread => "reread",
            Self::Repreview => "repreview",
            Self::Reapply => "reapply",
            Self::RetryLater => "retry_later",
            Self::NarrowCall => "narrow_call",
            Self::Escalate => "escalate",
            Self::Unknown => "unknown",
        }
    }
}

/// Codes whose kind default would mislead a model, sorted by code. Every other
/// code takes its kind default below, so a new code needs no row here.
const OVERRIDES: &[(&str, Advice)] = &[
    ("bad_action", Advice::DescribeVerb),
    ("bad_site", Advice::FixArgument),
    ("bad_threat", Advice::FixArgument),
    ("duplicate_key", Advice::Repreview),
    ("expired_approval", Advice::Repreview),
    ("forged_approval", Advice::Repreview),
    ("lock_poisoned", Advice::Escalate),
    ("nothing_to_undo", Advice::Reread),
    ("pinned_object", Advice::Repreview),
    ("reused_approval", Advice::Repreview),
    ("rome_table", Advice::Escalate),
    ("stale_revision", Advice::Repreview),
    ("stale_scene_rev", Advice::Repreview),
    ("stale_tick", Advice::Repreview),
    ("undone_key", Advice::Reapply),
    ("unknown_module", Advice::DescribeVerb),
    ("unknown_pack", Advice::DescribeVerb),
    ("unknown_tool", Advice::SearchVerb),
    ("unknown_verb", Advice::DescribeVerb),
    ("unscreened_call", Advice::Escalate),
];

/// The advice code for one refusal code: an explicit row when the kind default
/// would mislead, else the retry flag (a transient refusal is retried later),
/// else the kind default. `Advice::Unknown` only marks a code no table names.
pub fn advice_for_code(code: &str) -> Advice {
    if let Some((_, advice)) = OVERRIDES.iter().find(|(row, _)| *row == code) {
        return *advice;
    }
    let (kind, retryable) = classify(code);
    if kind == Kind::Unknown {
        return Advice::Unknown;
    }
    if retryable {
        return Advice::RetryLater;
    }
    match kind {
        Kind::Input => Advice::FixArgument,
        Kind::State => Advice::Reread,
        Kind::Permission | Kind::Budget => Advice::NarrowCall,
        Kind::Feature | Kind::Internal => Advice::Escalate,
        Kind::Unknown => Advice::Unknown,
    }
}

// ---------------------------------------------------------------------------
// The multi-field reply: one pointer, reason code and redacted hint per field.
// ---------------------------------------------------------------------------

/// Why one argument field was refused. Closed like [`Advice`]: a client
/// branches on the reason code, and the hint says how to fix it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The field is not an object id (UUID text).
    NotAnId,
    /// An insert id is the nil id, which names "no parent", never a new node.
    NilId,
    /// A pose lane is not finite.
    NotFinite,
    /// The same inserted id appears twice in one batch.
    DuplicateId,
    /// The value is outside the field's fixed set or shape.
    UnknownValue,
}

impl Reason {
    /// The wire word for this reason.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotAnId => "not_an_id",
            Self::NilId => "nil_id",
            Self::NotFinite => "not_finite",
            Self::DuplicateId => "duplicate_id",
            Self::UnknownValue => "unknown_value",
        }
    }
}

/// Most field problems one refusal lists; the reply flags when it clipped.
pub const MAX_FIELD_PROBLEMS: usize = 12;

/// One bad argument field: where it is, why it was refused and how to fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldProblem {
    /// RFC 6901 JSON pointer into the call arguments (`/ops/2/local/pos`).
    pub pointer: String,
    /// Closed reason code.
    pub reason: Reason,
    /// How to fix it, with any secret-shaped value redacted.
    pub hint: String,
}

impl FieldProblem {
    /// One problem on the field at `pointer`.
    fn new(pointer: String, reason: Reason, hint: String) -> Self {
        Self {
            pointer,
            reason,
            hint,
        }
    }

    /// The wire row: `{pointer, reason, hint}`.
    pub fn to_json(&self) -> Value {
        json!({
            "pointer": self.pointer,
            "reason": self.reason.as_str(),
            "hint": self.hint,
        })
    }
}

/// The text a hint may echo: the value itself, or `<redacted>` when it looks
/// like a credential. An object id is never a secret, so it still echoes.
pub fn redact(text: &str) -> String {
    if is_secret_shaped(text) {
        "<redacted>".to_owned()
    } else {
        text.to_owned()
    }
}

/// True when a value must not be echoed back: a known credential prefix, a
/// credential word, or a long opaque token. A UUID is an id, never a secret.
fn is_secret_shaped(text: &str) -> bool {
    const PREFIXES: [&str; 6] = ["sk-", "sk_", "ghp_", "gho_", "xoxb-", "AKIA"];
    const WORDS: [&str; 5] = ["password", "token=", "api_key", "secret", "bearer "];
    if ObjectId::parse(text).is_ok() {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    PREFIXES.iter().any(|prefix| text.starts_with(prefix))
        || WORDS.iter().any(|word| lower.contains(word))
        || (text.chars().count() >= 32
            && text
                .chars()
                .all(|lane| lane.is_ascii_alphanumeric() || matches!(lane, '_' | '-' | '+' | '=')))
}

/// Build the one refusal for a bad argument list: every problem in one
/// `bad_args` error, each with its pointer, reason and redacted hint.
pub fn field_refusal(problems: &[FieldProblem]) -> super::McpToolError {
    let listed = &problems[..problems.len().min(MAX_FIELD_PROBLEMS)];
    let named: Vec<&str> = listed.iter().take(3).map(|p| p.pointer.as_str()).collect();
    let message = format!(
        "{} argument field(s) are refused: {}; fix every field this reply names, \
         then call again (nothing was applied)",
        problems.len(),
        named.join(", "),
    );
    super::McpToolError::new("bad_args", message).with_data(json!({
        "fields": listed.iter().map(FieldProblem::to_json).collect::<Vec<Value>>(),
        "field_count": problems.len(),
        "fields_truncated": problems.len() > listed.len(),
    }))
}

/// Every bad field of one op list, or `None` when at most one field is bad.
///
/// One bad field keeps the precise code `op_from_dto` returns (`bad_id`,
/// `bad_pose`, `bad_component`); two or more answer once as `bad_args` with
/// every pointer, so a model repairs the whole list in one call.
pub fn ops_refusal(ops: &[OpDto]) -> Option<super::McpToolError> {
    let problems = ops_problems(ops);
    if problems.len() < 2 {
        return None;
    }
    Some(field_refusal(&problems))
}

/// Walk every op and collect each bad field, stopping one past the cap so the
/// reply can flag that it clipped.
fn ops_problems(ops: &[OpDto]) -> Vec<FieldProblem> {
    let mut problems = Vec::new();
    let mut inserted: Vec<String> = Vec::new();
    for (index, op) in ops.iter().enumerate() {
        if problems.len() > MAX_FIELD_PROBLEMS {
            break;
        }
        match op {
            OpDto::Insert { id, parent, local } => {
                check_id(&mut problems, index, "id", id, true);
                check_id(&mut problems, index, "parent", parent, false);
                check_pose(&mut problems, index, local);
                if let Ok(parsed) = ObjectId::parse(id) {
                    let text = parsed.to_string();
                    if inserted.contains(&text) {
                        problems.push(FieldProblem::new(
                            format!("/ops/{index}/id"),
                            Reason::DuplicateId,
                            format!("`{text}` is inserted twice in one batch; every insert needs its own id"),
                        ));
                    }
                    inserted.push(text);
                }
            }
            OpDto::Remove { id } => check_id(&mut problems, index, "id", id, false),
            OpDto::SetLocal { id, local } => {
                check_id(&mut problems, index, "id", id, false);
                check_pose(&mut problems, index, local);
            }
            OpDto::Reparent { id, new_parent } => {
                check_id(&mut problems, index, "id", id, false);
                check_id(&mut problems, index, "new_parent", new_parent, false);
            }
            OpDto::SetMaterial { id, material } => {
                check_id(&mut problems, index, "id", id, false);
                if let Some(name) = material {
                    if MaterialName::try_from(name.as_str()).is_err() {
                        problems.push(FieldProblem::new(
                            format!("/ops/{index}/material"),
                            Reason::UnknownValue,
                            format!(
                                "`{}` is not a material name (1..=64 bytes of 0-9 A-Z a-z - _ .; \
                                 null clears the binding)",
                                redact(name)
                            ),
                        ));
                    }
                }
            }
            OpDto::SetComponent { id, component, .. } => {
                check_id(&mut problems, index, "id", id, false);
                check_component(&mut problems, index, component);
            }
            OpDto::RemoveComponent { id, component } => {
                check_id(&mut problems, index, "id", id, false);
                check_component(&mut problems, index, component);
            }
        }
    }
    problems
}

/// Refuse an id field that is not UUID text, or an insert id that is nil.
fn check_id(
    problems: &mut Vec<FieldProblem>,
    index: usize,
    field: &str,
    text: &str,
    must_not_be_nil: bool,
) {
    let pointer = format!("/ops/{index}/{field}");
    let Ok(id) = ObjectId::parse(text) else {
        problems.push(FieldProblem::new(
            pointer,
            Reason::NotAnId,
            format!(
                "`{}` is not an object id (expected UUID text; the nil id `{}` means \
                 'no parent' on parent fields)",
                redact(text),
                ObjectId::NIL
            ),
        ));
        return;
    };
    if must_not_be_nil && id == ObjectId::NIL {
        problems.push(FieldProblem::new(
            pointer,
            Reason::NilId,
            "an insert needs a fresh id, never the nil id".to_owned(),
        ));
    }
}

/// Refuse a pose with a non-finite lane, naming the lane that carries it.
fn check_pose(problems: &mut Vec<FieldProblem>, index: usize, local: &super::PoseDto) {
    let lanes = [
        ("pos", &local.pos[..]),
        ("rot", &local.rot[..]),
        ("scale", &local.scale[..]),
    ];
    for (lane, values) in lanes {
        if values.iter().all(|value| value.is_finite()) {
            continue;
        }
        problems.push(FieldProblem::new(
            format!("/ops/{index}/local/{lane}"),
            Reason::NotFinite,
            format!("every `{lane}` lane must be finite (meters, quaternion, scale)"),
        ));
    }
}

/// Refuse a component field whose name is not a component schema.
fn check_component(problems: &mut Vec<FieldProblem>, index: usize, name: &str) {
    if ComponentKind::from_name(name).is_some() {
        return;
    }
    problems.push(FieldProblem::new(
        format!("/ops/{index}/component"),
        Reason::UnknownValue,
        format!(
            "`{}` is not a component schema; the schemas are [{}]",
            redact(name),
            ComponentKind::name_list()
        ),
    ));
}

#[cfg(test)]
#[path = "advice_tests.rs"]
mod tests;
