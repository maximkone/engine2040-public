//! Refusal mapping for the MCP door: command and capability refusals become
//! tool errors with the legal values or the culprit named. Re-exported from
//! `mcp` so the door and its sibling tools keep their existing paths.
//!
//! DOOR6: a refusal is a record, not a sentence. Every code carries a kind, a
//! retryable flag, up to three suggestions and the one call to make next;
//! `McpToolError::new` fills them from the tables at the end of this file, so
//! the existing refusal sites gained the fields without an edit.

use engine_editor::capability::{CapError, Capability, OpMask};
use engine_editor::commands::{self, Document};

/// Tool failure: a machine `code`, a message naming the allowed values,
/// optional `data` for the offending payload, and the advice record a model
/// branches on (`kind`, `retryable`, `suggestions`, `next`). The code, message
/// and data are the canonical door shape shared with
/// [`crate::discover::RpcError`]; the four advice fields are filled from the
/// code tables in this module, so a refusal site never builds them by hand and
/// a new code is classified in one place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpToolError {
    /// Machine code (`stale_revision`, `forged_approval`, ...).
    pub code: &'static str,
    /// Human detail with the legal values embedded.
    pub message: String,
    /// Offending payload (item index, method name, ...), when it fits.
    pub data: Option<rmcp::serde_json::Value>,
    /// What kind of refusal this is: input, state, permission, budget,
    /// feature or internal.
    pub kind: Kind,
    /// True when a plain retry of the same intent can succeed.
    pub retryable: bool,
    /// Up to three concrete next moves; empty for the long-tail codes.
    pub suggestions: &'static [&'static str],
    /// The one call to make next, when the code names one. A shared table row:
    /// the struct stays small enough that a `Result<_, McpToolError>` return
    /// does not trip the large-error lint on every door function.
    pub next: Option<&'static NextCall>,
}

impl McpToolError {
    /// New tool error with a static code and no data. The advice fields come
    /// from the code tables, so they are never missing and never guessed.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        let (kind, retryable) = classify(code);
        Self {
            code,
            message: message.into(),
            data: None,
            kind,
            retryable,
            suggestions: suggestions_for(code),
            next: next_for(code),
        }
    }

    /// Attach a JSON payload to this error.
    pub fn with_data(mut self, data: rmcp::serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }

    /// Machine code.
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Offending payload, when one fits.
    pub fn data(&self) -> Option<&rmcp::serde_json::Value> {
        self.data.as_ref()
    }

    /// What kind of refusal this is.
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// True when a plain retry of the same intent can succeed.
    pub fn retryable(&self) -> bool {
        self.retryable
    }

    /// Up to three concrete next moves.
    pub fn suggestions(&self) -> &'static [&'static str] {
        self.suggestions
    }

    /// The one call to make next, when the code names one.
    pub fn next_call(&self) -> Option<NextCall> {
        self.next.copied()
    }

    /// The payload a client receives: `{code, message, data}` plus the advice
    /// `{kind, retryable, suggestions, next}`. `next` is written only when the
    /// code names one, so the long tail carries no empty call.
    pub fn wire_payload(&self) -> rmcp::serde_json::Value {
        let mut payload = rmcp::serde_json::json!({
            "code": self.code,
            "message": self.message,
            "data": self.data,
            "kind": self.kind.as_str(),
            "retryable": self.retryable,
            "suggestions": self.suggestions,
        });
        if let rmcp::serde_json::Value::Object(map) = &mut payload {
            if let Some(next) = self.next {
                map.insert("next".to_owned(), next.to_json());
            }
        }
        payload
    }

    /// Re-code a wire refusal under a new static code, keeping its message
    /// and data: layering a code never silently drops the culprit. The advice
    /// fields follow the new code, because that is the code the client reads.
    pub fn wrap(code: &'static str, error: crate::discover::RpcError) -> Self {
        let mut wrapped = Self::new(code, error.message);
        wrapped.data = error.data;
        wrapped
    }
}

impl std::fmt::Display for McpToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for McpToolError {}

impl From<McpToolError> for rmcp::ErrorData {
    fn from(error: McpToolError) -> Self {
        // The MCP result path: the payload carries the code, the culprit and
        // the four advice fields, so a model reads the refusal as a record.
        Self::invalid_params(error.to_string(), Some(error.wire_payload()))
    }
}

/// Map a command refusal to a tool error with the legal values attached.
/// Crate-visible: the scene tools map staging failures the same way.
pub(crate) fn batch_error(error: &commands::BatchError) -> McpToolError {
    match error {
        commands::BatchError::StaleRevision { expected, got } => McpToolError::new(
            "stale_revision",
            format!(
                "document is at revision {expected}, batch expects {got}; \
                 re-read scene.read, re-preview, then apply"
            ),
        ),
        commands::BatchError::DuplicateKey { key } => McpToolError::new(
            "duplicate_key",
            format!(
                "idempotency key {key} was already used with a different payload; \
                 submit a new batch with a fresh key"
            ),
        ),
        commands::BatchError::UndoneKey { key } => McpToolError::new(
            "undone_key",
            format!(
                "idempotency key {key} belongs to an undone batch; \
                 submit a new batch with a fresh key to reapply"
            ),
        ),
        _ => McpToolError::new("invalid_batch", error.to_string()),
    }
}

/// Map a capability refusal to a tool error. `granted` is the op-class list
/// snapshotted before the gated apply, so denials name what is allowed. `doc`
/// is the document the refused apply left unchanged: spatial refusals name
/// their culprit objects from it. Crate-visible: `scene.place` applies through
/// the same gate.
pub(crate) fn cap_error(error: &CapError, granted: &[&str], doc: &Document) -> McpToolError {
    match error {
        CapError::ForgedApproval => McpToolError::new(
            "forged_approval",
            "approval does not match this session and batch; \
             re-run tx.preview for a fresh approval",
        ),
        CapError::StaleRevision { expected, got } => McpToolError::new(
            "stale_revision",
            format!(
                "document is at revision {expected}, approval expects {got}; \
                 re-read scene.read, re-preview, then apply"
            ),
        ),
        CapError::Expired { expiry, current } => McpToolError::new(
            "expired_approval",
            format!(
                "approval expired at revision {expiry}, document is at {current}; \
                 re-run tx.preview for a fresh approval"
            ),
        ),
        CapError::ReusedApproval => McpToolError::new(
            "reused_approval",
            "approval was already spent; re-run tx.preview for a fresh approval",
        ),
        CapError::OpNotAllowed { op } => McpToolError::new(
            "op_not_allowed",
            format!(
                "capability does not grant `{op}` ops; granted: [{}]",
                granted.join(", ")
            ),
        ),
        CapError::OutOfScope { id } => McpToolError::new(
            "out_of_scope",
            format!("node {id} is outside this session's object scope"),
        ),
        CapError::OverBudget { remaining, needed } => McpToolError::new(
            "over_budget",
            format!("batch needs {needed} ops, {remaining} left in this session"),
        ),
        CapError::Spatial { check, culprits } => {
            crate::scene_tools::refusals::spatial_refusal(doc, check, culprits)
        }
        CapError::Command(inner) => batch_error(inner),
    }
}

/// Op classes granted by a capability, for denial messages.
/// Crate-visible: `scene.place` snapshots the same list before its apply.
pub(crate) fn granted_ops(cap: &Capability) -> Vec<&'static str> {
    let mut granted = Vec::with_capacity(4);
    if cap.ops.contains(OpMask::INSERT) {
        granted.push("insert");
    }
    if cap.ops.contains(OpMask::MOVE) {
        granted.push("move");
    }
    if cap.ops.contains(OpMask::EDIT) {
        granted.push("edit");
    }
    if cap.ops.contains(OpMask::DELETE) {
        granted.push("delete");
    }
    granted
}

// ---------------------------------------------------------------------------
// The refusal record: kind, retryable, suggestions and next.
// ---------------------------------------------------------------------------

/// What kind of refusal this is: the first branch a client takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The call is malformed or names a value outside its legal range.
    Input,
    /// The call is well formed but the document, journal or process refuses it.
    State,
    /// The capability, approval or scope does not allow the call.
    Permission,
    /// The call costs more than its budget allows.
    Budget,
    /// This build or host lacks what the call needs.
    Feature,
    /// The door failed in a way the caller cannot fix from the outside.
    Internal,
    /// No table row names the code. The completeness test forbids this for a
    /// code the crate produces, so it only ever marks a foreign string.
    Unknown,
}

impl Kind {
    /// The wire word for this kind.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::State => "state",
            Self::Permission => "permission",
            Self::Budget => "budget",
            Self::Feature => "feature",
            Self::Internal => "internal",
            Self::Unknown => "unknown",
        }
    }
}

/// The one call to make next. `args` is JSON object text when the call takes
/// fixed arguments the model need not invent; `None` when the model fills
/// them from what the refusal named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NextCall {
    /// Verb name, always one of the door's reachable tool names.
    pub verb: &'static str,
    /// Arguments as JSON object text, or `None` for a call the model fills.
    pub args: Option<&'static str>,
}

impl NextCall {
    /// A call whose arguments the model supplies.
    const fn of(verb: &'static str) -> Self {
        Self { verb, args: None }
    }

    /// A call with fixed arguments, as JSON object text.
    const fn with(verb: &'static str, args: &'static str) -> Self {
        Self {
            verb,
            args: Some(args),
        }
    }

    /// The wire shape `{"verb": ..., "args": ...}`; `args` is null when the
    /// model fills them. Table rows are tested to parse as a JSON object.
    pub fn to_json(&self) -> rmcp::serde_json::Value {
        let args = self
            .args
            .and_then(|text| rmcp::serde_json::from_str(text).ok())
            .unwrap_or(rmcp::serde_json::Value::Null);
        rmcp::serde_json::json!({ "verb": self.verb, "args": args })
    }
}

/// One kind's codes. Every code the crate produces sits in exactly one of
/// these tables; the completeness test fails when a new literal is missing.
const INPUT_CODES: &[&str] = &[
    "ambiguous_name",
    "bad_action",
    "bad_allow",
    "bad_args",
    "bad_brief",
    "bad_camera",
    "bad_caps",
    "bad_capsule",
    "bad_cell",
    "bad_chain",
    "bad_check",
    "bad_clip",
    "bad_colliders",
    "bad_component",
    "bad_config",
    "bad_csv",
    "bad_cursor",
    "bad_edit",
    "bad_epsilon",
    "bad_events",
    "bad_evidence",
    "bad_extent",
    "bad_fighters",
    "bad_floor",
    "bad_format",
    "bad_gap",
    "bad_graph",
    "bad_id",
    "bad_image",
    "bad_input",
    "bad_job_id",
    "bad_joint",
    "bad_kind",
    "bad_limit",
    "bad_material",
    "bad_metric",
    "bad_origin",
    "bad_output",
    "bad_part",
    "bad_participants",
    "bad_pick",
    "bad_pixel",
    "bad_png",
    "bad_point",
    "bad_pose",
    "bad_radius",
    "bad_receipt",
    "bad_receipts",
    "bad_recipe",
    "bad_record",
    "bad_relation",
    "bad_root",
    "bad_rule",
    "bad_scene",
    "bad_scope",
    "bad_size",
    "bad_slots",
    "bad_stat",
    "bad_tape",
    "bad_ticks",
    "bad_ttl",
    "bad_variant",
    "bad_view",
    "bad_wall",
    "bad_want",
    "bad_word",
    "brief_too_long",
    "cook_failed",
    "corrupt_bundle",
    "corrupt_tape",
    "duplicate_input",
    "empty_batch",
    "empty_brief",
    "empty_checks",
    "empty_query",
    "injection_refused",
    "invalid_action",
    "invalid_batch",
    "invalid_config",
    "invalid_tape",
    "malformed_output",
    "missing_arg",
    "missing_clip",
    "not_a_root",
    "rules_refused",
    "spatial_rejected",
    "too_many_allow",
    "too_many_checks",
    "too_many_inputs",
    "too_many_ops",
    "too_many_wants",
    "unknown_code",
    "unknown_pack",
    "unknown_path",
    "unknown_subscription",
    "unknown_tool",
    "unknown_verb",
    "unsorted_tape",
    "wfc_disconnected",
];

const STATE_CODES: &[&str] = &[
    "apply_in_flight",
    "arena_view_failed",
    "audit_exhausted",
    "black_capture",
    "bout_failed",
    "browse_failed",
    "build_failed",
    "build_incomplete",
    "clean_replay",
    "duplicate_key",
    "incomplete_build",
    "job_already_done",
    "job_not_running",
    "job_table_full",
    "lock_poisoned",
    "look_judge_registered",
    "manifest_mismatch",
    "missing_component",
    "needs_nav_view",
    "no_build",
    "no_chain",
    "no_frames",
    "no_game",
    "no_goal",
    "no_live_bout",
    "no_perf_csv",
    "no_pose",
    "no_project_dir",
    "no_receipts",
    "no_saved_scene",
    "no_scene",
    "no_script",
    "no_seat",
    "no_snapshot",
    "no_space",
    "not_served",
    "nothing_to_undo",
    "pinned_object",
    "play_refused",
    "prefab_failed",
    "read_repeat",
    "scene_needs_door",
    "seat_already_open",
    "seat_closed",
    "seat_late_input",
    "seat_missed_tick",
    "seat_protocol",
    "seat_start_failed",
    "seat_timeout",
    "snapshot_failed",
    "stale_policy",
    "stale_revision",
    "stale_scene_rev",
    "stale_tick",
    "subscribe_failed",
    "undo_mismatch",
    "undone_key",
    "unknown_actor",
    "unknown_build",
    "unknown_check",
    "unknown_component",
    "unknown_graph",
    "unknown_job",
    "unknown_material",
    "unknown_prefab",
    "unknown_receipt",
    "unknown_revision",
    "view_host_busy",
    "view_laws_registered",
    "view_source_registered",
];

const PERMISSION_CODES: &[&str] = &[
    "denied_op",
    "destructive_tier",
    "expired_approval",
    "forged_approval",
    "op_not_allowed",
    "out_of_scope",
    "reused_approval",
];

const BUDGET_CODES: &[&str] = &[
    "bad_budget",
    "budget_headroom_exhausted",
    "explain_over_budget",
    "graph_too_large",
    "over_budget",
    "scene_too_big",
    "too_big",
    "tool_budget_exceeded",
    "tool_list_bloated",
];

const FEATURE_CODES: &[&str] = &[
    "arena_exe_missing",
    "feature_off",
    "no_adapter",
    "no_chrome",
    "seeded_bug",
    "spawn_failed",
    "tool_missing",
    "unsupported_capture",
    "unsupported_look_judge",
    "unsupported_view_source",
];

const INTERNAL_CODES: &[&str] = &[
    "bad_terrain",
    "bake_io",
    "counters_unwritten",
    "evidence_unwritten",
    "internal",
    "io_failed",
    "load_failed",
    "persist_failed",
    "prefab_store",
    "read_failed",
    "rules_worker_down",
    "save_failed",
    "unscreened_call",
    "walk_failed",
    "write_failed",
];

/// Codes where a plain retry of the same intent can succeed: the refusal is
/// transient or repaired by a read the model was already told to make. Every
/// other code needs a changed call, so a retry alone cannot help.
const RETRYABLE_CODES: &[&str] = &[
    "apply_in_flight",
    "counters_unwritten",
    "evidence_unwritten",
    "expired_approval",
    "internal",
    "io_failed",
    "load_failed",
    "persist_failed",
    "rules_worker_down",
    "save_failed",
    "seat_closed",
    "seat_timeout",
    "spawn_failed",
    "stale_policy",
    "stale_revision",
    "stale_scene_rev",
    "stale_tick",
    "subscribe_failed",
    "view_host_busy",
    "walk_failed",
    "write_failed",
];

/// Classify one code: its kind plus whether a plain retry can succeed. A code
/// no table names answers `Kind::Unknown`: the completeness test keeps that
/// from happening to a code the crate produces.
pub(crate) fn classify(code: &str) -> (Kind, bool) {
    let retryable = RETRYABLE_CODES.contains(&code);
    let tables = [
        (Kind::Input, INPUT_CODES),
        (Kind::State, STATE_CODES),
        (Kind::Permission, PERMISSION_CODES),
        (Kind::Budget, BUDGET_CODES),
        (Kind::Feature, FEATURE_CODES),
        (Kind::Internal, INTERNAL_CODES),
    ];
    for (kind, codes) in tables {
        if codes.contains(&code) {
            return (kind, retryable);
        }
    }
    (Kind::Unknown, false)
}

/// One hand-pinned advice row: the code, its concrete next moves and the one
/// call to make next. Only the codes a client hits first get a row; every
/// other code still carries its kind and retryable flag.
struct Advice {
    code: &'static str,
    suggestions: &'static [&'static str],
    next: NextCall,
}

/// The twelve most-hit codes, pinned by hand with advice a model can act on.
const ADVICE: &[Advice] = &[
    Advice {
        code: "stale_revision",
        suggestions: &[
            "call scene.read for the current revision",
            "re-run tx.preview at that revision for a fresh approval",
            "tx.apply the approval it returns",
        ],
        next: NextCall::with("scene.read", "{\"limit\":64}"),
    },
    Advice {
        code: "invalid_batch",
        suggestions: &[
            "send 1 to 256 ops per batch",
            "api.describe tx.preview for the exact batch shape",
        ],
        next: NextCall::with("api.describe", "{\"path\":\"tx.preview\"}"),
    },
    Advice {
        code: "unknown_tool",
        suggestions: &[
            "api.search the name you meant; every reachable verb is indexed by name",
            "call the exact tool name from the hit, not an alias",
        ],
        next: NextCall::of("api.search"),
    },
    Advice {
        code: "bad_args",
        suggestions: &[
            "api.describe the verb for its exact argument names, types and ranges",
            "fix the argument the message names, then call again",
        ],
        next: NextCall::of("api.describe"),
    },
    Advice {
        code: "over_budget",
        suggestions: &[
            "split the batch into smaller previews inside the session op budget",
            "tx.preview the smaller batch, then tx.apply its approval",
        ],
        next: NextCall::of("tx.preview"),
    },
    Advice {
        code: "op_not_allowed",
        suggestions: &[
            "stay inside the granted op classes the message lists",
            "api.describe the verb to see which op class it needs",
        ],
        next: NextCall::of("api.describe"),
    },
    Advice {
        code: "out_of_scope",
        suggestions: &[
            "work on ids inside the scope the message names",
            "scene.read the scope for the live ids it holds",
        ],
        next: NextCall::with("scene.read", "{\"limit\":64}"),
    },
    Advice {
        code: "expired_approval",
        suggestions: &[
            "re-run tx.preview at the current revision for a fresh approval",
            "tx.apply that approval before any other write lands",
        ],
        next: NextCall::of("tx.preview"),
    },
    Advice {
        code: "reused_approval",
        suggestions: &[
            "re-run tx.preview; the apply that used it spent the approval",
            "tx.apply the fresh approval once",
        ],
        next: NextCall::of("tx.preview"),
    },
    Advice {
        code: "forged_approval",
        suggestions: &[
            "re-run tx.preview in this session; an approval binds one session and batch",
            "tx.apply the approval this session minted",
        ],
        next: NextCall::of("tx.preview"),
    },
    Advice {
        code: "duplicate_key",
        suggestions: &[
            "tx.preview the batch under a fresh idempotency key",
            "reuse a key only to replay the exact same payload",
        ],
        next: NextCall::of("tx.preview"),
    },
    Advice {
        code: "undone_key",
        suggestions: &[
            "tx.preview the change under a fresh idempotency key",
            "tx.apply that preview to reapply the undone change",
        ],
        next: NextCall::of("tx.preview"),
    },
];

/// The advice row for one code, when one is pinned.
fn advice_for(code: &str) -> Option<&'static Advice> {
    ADVICE.iter().find(|row| row.code == code)
}

/// Up to three concrete next moves for one code; empty when the code has no
/// pinned row. Each move names a verb or an argument a model can act on.
pub(crate) fn suggestions_for(code: &str) -> &'static [&'static str] {
    match advice_for(code) {
        Some(row) => row.suggestions,
        None => &[],
    }
}

/// The one call to make next for one code, when a pinned row names one.
pub(crate) fn next_for(code: &str) -> Option<&'static NextCall> {
    advice_for(code).map(|row| &row.next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};

    /// The door's own code table in `discover.rs`, which carries the dynamic
    /// passthrough codes a constructor scan cannot see.
    const TABLE_MARKER: &str = "const TOOL_ERROR_CODES: &[&str] = &[";

    /// A code literal is a snake_case token: lowercase, digits and `_`.
    fn is_machine_code(code: &str) -> bool {
        !code.is_empty()
            && code
                .chars()
                .all(|lane| lane.is_ascii_lowercase() || lane.is_ascii_digit() || lane == '_')
    }

    fn crate_src() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
    }

    /// Code literals passed to the two refusal constructors, at any depth in
    /// the source tree.
    fn collect_constructors(dir: &Path, codes: &mut BTreeSet<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_constructors(&path, codes);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            for marker in ["McpToolError::new(", "McpToolError::wrap("] {
                for chunk in text.split(marker).skip(1) {
                    let Some(rest) = chunk.trim_start().strip_prefix('"') else {
                        continue;
                    };
                    let Some(end) = rest.find('"') else {
                        continue;
                    };
                    let code = &rest[..end];
                    if is_machine_code(code) {
                        codes.insert(code.to_owned());
                    }
                }
            }
        }
    }

    /// The codes of `discover.rs`'s `TOOL_ERROR_CODES` table: the door's own
    /// answer to "every code I emit", dynamic passthroughs included.
    fn collect_published_table(codes: &mut BTreeSet<String>) {
        let text =
            std::fs::read_to_string(crate_src().join("discover.rs")).expect("discover.rs reads");
        let start = text
            .find(TABLE_MARKER)
            .expect("the door keeps one TOOL_ERROR_CODES table");
        let body = &text[start + TABLE_MARKER.len()..];
        let end = body
            .find("];")
            .expect("the door's TOOL_ERROR_CODES table closes");
        for row in body[..end].lines() {
            let row = row.trim();
            let Some(rest) = row.strip_prefix('"') else {
                continue;
            };
            let Some(close) = rest.find('"') else {
                continue;
            };
            let code = &rest[..close];
            if is_machine_code(code) {
                codes.insert(code.to_owned());
            }
        }
    }

    /// Every machine code the crate produces, from both sources.
    fn crate_codes() -> BTreeSet<String> {
        let mut codes = BTreeSet::new();
        collect_constructors(&crate_src(), &mut codes);
        collect_published_table(&mut codes);
        codes
    }

    fn kind_groups() -> [(Kind, &'static [&'static str]); 6] {
        [
            (Kind::Input, INPUT_CODES),
            (Kind::State, STATE_CODES),
            (Kind::Permission, PERMISSION_CODES),
            (Kind::Budget, BUDGET_CODES),
            (Kind::Feature, FEATURE_CODES),
            (Kind::Internal, INTERNAL_CODES),
        ]
    }

    fn assert_sorted(name: &str, codes: &[&str]) {
        assert!(
            codes.windows(2).all(|pair| pair[0] < pair[1]),
            "{name} stays sorted and free of duplicates"
        );
    }

    /// A dotted word a suggestion names, e.g. `scene.read`.
    fn is_verb_token(word: &str) -> bool {
        let Some((head, tail)) = word.split_once('.') else {
            return false;
        };
        !head.is_empty()
            && !tail.is_empty()
            && head
                .chars()
                .all(|lane| lane.is_ascii_lowercase() || lane.is_ascii_digit() || lane == '_')
            && tail.chars().all(|lane| {
                lane.is_ascii_lowercase() || lane.is_ascii_digit() || lane == '_' || lane == '.'
            })
    }

    fn strip_word(word: &str) -> &str {
        word.trim_matches(|lane: char| !lane.is_ascii_alphanumeric() && lane != '.' && lane != '_')
    }

    #[test]
    fn refusal_every_code_the_crate_produces_is_classified() {
        let codes = crate_codes();
        assert!(
            codes.len() >= 150,
            "the scan found only {} codes, so it is not reading the tree",
            codes.len()
        );
        let mut unknown = Vec::new();
        for code in &codes {
            let (kind, _retryable) = classify(code);
            if kind == Kind::Unknown {
                unknown.push(code.clone());
            }
        }
        assert!(
            unknown.is_empty(),
            "these codes carry no kind, so a model cannot branch on them: {unknown:?}"
        );
    }

    #[test]
    fn refusal_each_code_has_exactly_one_kind_and_the_tables_stay_sorted() {
        assert_sorted("INPUT_CODES", INPUT_CODES);
        assert_sorted("STATE_CODES", STATE_CODES);
        assert_sorted("PERMISSION_CODES", PERMISSION_CODES);
        assert_sorted("BUDGET_CODES", BUDGET_CODES);
        assert_sorted("FEATURE_CODES", FEATURE_CODES);
        assert_sorted("INTERNAL_CODES", INTERNAL_CODES);
        assert_sorted("RETRYABLE_CODES", RETRYABLE_CODES);
        let mut seen: BTreeMap<&str, Kind> = BTreeMap::new();
        for (kind, codes) in kind_groups() {
            for code in codes {
                if let Some(first) = seen.insert(code, kind) {
                    panic!("`{code}` sits in both {first:?} and {kind:?}");
                }
            }
        }
        assert!(
            seen.len() >= 150,
            "the kind tables hold only {} codes",
            seen.len()
        );
        // No dead rows either: a kind for a code the crate never produces
        // would be advice no model can reach.
        let produced = crate_codes();
        let dead: Vec<&str> = seen
            .keys()
            .copied()
            .filter(|code| !produced.contains(*code))
            .collect();
        assert!(
            dead.is_empty(),
            "these kind rows name codes the crate never produces: {dead:?}"
        );
    }

    #[test]
    fn refusal_retryable_rows_all_name_a_classified_code() {
        // A retryable row that names no classified code would never reach a
        // model: `classify` only reads it for a code its kind table knows.
        let mut unclassified: Vec<&str> = RETRYABLE_CODES
            .iter()
            .copied()
            .filter(|code| classify(code).0 == Kind::Unknown)
            .collect();
        unclassified.sort_unstable();
        assert!(
            unclassified.is_empty(),
            "retryable codes with no kind: {unclassified:?}"
        );
        for code in RETRYABLE_CODES {
            assert!(
                classify(code).1,
                "`{code}` is listed retryable but classifies as no-retry"
            );
        }
    }

    #[test]
    fn refusal_pinned_rows_carry_advice_a_model_can_act_on() {
        assert_eq!(ADVICE.len(), 12, "the top twelve codes stay pinned");
        let mut codes: BTreeSet<&str> = BTreeSet::new();
        for row in ADVICE {
            assert!(codes.insert(row.code), "`{}` is pinned twice", row.code);
            assert_ne!(
                classify(row.code).0,
                Kind::Unknown,
                "pinned code `{}` has no kind",
                row.code
            );
            assert!(
                !row.suggestions.is_empty() && row.suggestions.len() <= 3,
                "`{}` carries 1..=3 suggestions, not {}",
                row.code,
                row.suggestions.len()
            );
            assert!(
                matches!(row.next, NextCall { verb, .. } if crate::mcp::tool_names().contains(&verb)),
                "`{}` next names a verb the door serves",
                row.code
            );
        }
    }

    /// One test per pinned top-twelve code: the exact suggestion text, the
    /// kind, the retryable flag and the exact next call.
    macro_rules! pin_refusal {
        ($name:ident, $code:literal, $kind:expr, $retryable:literal,
         [$($suggestion:literal),+ $(,)?], $verb:literal, $args:expr) => {
            #[test]
            fn $name() {
                let code = $code;
                assert_eq!(classify(code), ($kind, $retryable), "`{code}` kind");
                assert_eq!(
                    suggestions_for(code),
                    &[$($suggestion),+][..],
                    "`{code}` suggestion text"
                );
                for suggestion in suggestions_for(code) {
                    for word in suggestion.split_whitespace().map(strip_word) {
                        if !is_verb_token(word) {
                            continue;
                        }
                        assert!(
                            crate::mcp::tool_names().contains(&word),
                            "`{code}` suggests `{word}`, which the door does not serve: {suggestion}"
                        );
                    }
                }
                let next = next_for(code).expect("a pinned code names its next call");
                assert_eq!(next.verb, $verb, "`{code}` next verb");
                assert_eq!(next.args, $args, "`{code}` next args");
                assert!(
                    crate::mcp::tool_names().contains(&next.verb),
                    "`{code}` next names a real verb"
                );
                if let Some(args) = next.args {
                    let value: rmcp::serde_json::Value =
                        rmcp::serde_json::from_str(args).expect("next args parse as JSON");
                    assert!(value.is_object(), "`{code}` next args are a JSON object");
                }
            }
        };
    }

    pin_refusal!(
        refusal_pinned_stale_revision,
        "stale_revision",
        Kind::State,
        true,
        [
            "call scene.read for the current revision",
            "re-run tx.preview at that revision for a fresh approval",
            "tx.apply the approval it returns",
        ],
        "scene.read",
        Some("{\"limit\":64}")
    );
    pin_refusal!(
        refusal_pinned_invalid_batch,
        "invalid_batch",
        Kind::Input,
        false,
        [
            "send 1 to 256 ops per batch",
            "api.describe tx.preview for the exact batch shape",
        ],
        "api.describe",
        Some("{\"path\":\"tx.preview\"}")
    );
    pin_refusal!(
        refusal_pinned_unknown_tool,
        "unknown_tool",
        Kind::Input,
        false,
        [
            "api.search the name you meant; every reachable verb is indexed by name",
            "call the exact tool name from the hit, not an alias",
        ],
        "api.search",
        None
    );
    pin_refusal!(
        refusal_pinned_bad_args,
        "bad_args",
        Kind::Input,
        false,
        [
            "api.describe the verb for its exact argument names, types and ranges",
            "fix the argument the message names, then call again",
        ],
        "api.describe",
        None
    );
    pin_refusal!(
        refusal_pinned_over_budget,
        "over_budget",
        Kind::Budget,
        false,
        [
            "split the batch into smaller previews inside the session op budget",
            "tx.preview the smaller batch, then tx.apply its approval",
        ],
        "tx.preview",
        None
    );
    pin_refusal!(
        refusal_pinned_op_not_allowed,
        "op_not_allowed",
        Kind::Permission,
        false,
        [
            "stay inside the granted op classes the message lists",
            "api.describe the verb to see which op class it needs",
        ],
        "api.describe",
        None
    );
    pin_refusal!(
        refusal_pinned_out_of_scope,
        "out_of_scope",
        Kind::Permission,
        false,
        [
            "work on ids inside the scope the message names",
            "scene.read the scope for the live ids it holds",
        ],
        "scene.read",
        Some("{\"limit\":64}")
    );
    pin_refusal!(
        refusal_pinned_expired_approval,
        "expired_approval",
        Kind::Permission,
        true,
        [
            "re-run tx.preview at the current revision for a fresh approval",
            "tx.apply that approval before any other write lands",
        ],
        "tx.preview",
        None
    );
    pin_refusal!(
        refusal_pinned_reused_approval,
        "reused_approval",
        Kind::Permission,
        false,
        [
            "re-run tx.preview; the apply that used it spent the approval",
            "tx.apply the fresh approval once",
        ],
        "tx.preview",
        None
    );
    pin_refusal!(
        refusal_pinned_forged_approval,
        "forged_approval",
        Kind::Permission,
        false,
        [
            "re-run tx.preview in this session; an approval binds one session and batch",
            "tx.apply the approval this session minted",
        ],
        "tx.preview",
        None
    );
    pin_refusal!(
        refusal_pinned_duplicate_key,
        "duplicate_key",
        Kind::State,
        false,
        [
            "tx.preview the batch under a fresh idempotency key",
            "reuse a key only to replay the exact same payload",
        ],
        "tx.preview",
        None
    );
    pin_refusal!(
        refusal_pinned_undone_key,
        "undone_key",
        Kind::State,
        false,
        [
            "tx.preview the change under a fresh idempotency key",
            "tx.apply that preview to reapply the undone change",
        ],
        "tx.preview",
        None
    );

    #[test]
    fn refusal_error_stays_small_enough_to_ride_a_result() {
        // The advice record adds four fields; `next` rides as a shared table
        // row so the error does not cross clippy's 128-byte large-error bar
        // and make every door function a `result_large_err` warning.
        assert!(
            std::mem::size_of::<McpToolError>() < 128,
            "McpToolError grew to {} bytes",
            std::mem::size_of::<McpToolError>()
        );
    }

    #[test]
    fn refusal_payload_carries_the_four_fields() {
        let error = McpToolError::new("stale_revision", "document moved under the batch")
            .with_data(rmcp::serde_json::json!({ "revision": 7 }));
        let envelope = rmcp::ErrorData::from(error);
        let payload = envelope
            .data
            .as_ref()
            .and_then(rmcp::serde_json::Value::as_object)
            .expect("the tool error payload is a JSON object");
        assert_eq!(payload["code"], "stale_revision");
        assert_eq!(payload["message"], "document moved under the batch");
        assert_eq!(payload["kind"], "state");
        assert_eq!(payload["retryable"], true);
        assert_eq!(
            payload["suggestions"][0],
            "call scene.read for the current revision"
        );
        assert_eq!(payload["next"]["verb"], "scene.read");
        assert_eq!(payload["next"]["args"]["limit"], 64);
        assert_eq!(
            payload["data"]["revision"], 7,
            "the culprit payload rides beside the advice"
        );
    }

    #[test]
    fn refusal_payload_for_a_long_tail_code_has_the_fields_and_no_advice() {
        let envelope = rmcp::ErrorData::from(McpToolError::new("bad_ticks", "ticks 0 is refused"));
        let payload = envelope
            .data
            .as_ref()
            .and_then(rmcp::serde_json::Value::as_object)
            .expect("the tool error payload is a JSON object");
        assert_eq!(payload["kind"], "input");
        assert_eq!(payload["retryable"], false);
        assert_eq!(payload["suggestions"], rmcp::serde_json::json!([]));
        assert!(
            !payload.contains_key("next"),
            "a code with no next call writes no next key: {payload:?}"
        );
    }
}
