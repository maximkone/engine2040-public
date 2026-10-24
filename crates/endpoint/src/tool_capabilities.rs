//! Capability-scoped tool permissions: a five-card taxonomy over the door.
//!
//! Consumer: the MCP door (`super::mcp::McpServer`) and the eval harness. The
//! door fronts every call through this taxonomy: the dispatch seam
//! (`super::door_dispatch`) refuses bare unknown names with
//! [`refuse_tool_call`], `tx.preview` previews through [`refuse_preview`]
//! (Propose card), and `tx.apply` plus `scene.place` commit through
//! [`refuse_write`] (Commit card at [`DOOR_TIER`]) ahead of the approval
//! ledger. The gates refuse hostile batches before the command service runs.
//! Data owner stays the editor command service; this module owns only the classification plus the
//! pure pre-checks. Units: ops, objects, bytes, nav tiles for costs;
//! nanoseconds only inside the `#[cfg(test)]` latency probe, never in the
//! gate. Threading: all gates take `&` references and hold no locks; the
//! caller (the door dispatch seam) owns synchronization. Determinism scope:
//! the same document, batch, capability, caps and tier always seal the same
//! verdict on every machine; integer compares only, no wall clock, no float,
//! no thread ids, no iteration over hash maps in the verdict path.
//! Errors: [`Verdict`] carries the machine `code` plus a message that always
//! names the legal values and one recovery, and converts to [`McpToolError`]
//! without losing either. Budgets: the gate adds no discovery chars (no new
//! tool, no description change), one small message `String` per call at most,
//! and never allocates per entity per tick; it runs at the door, outside the
//! fixed tick.
//!
//! The five cards, and what each maps onto (`e2_tool` readonly split plus
//! EXPLAIN caps):
//!
//! | Card | Tools | Gate | Maps onto |
//! |---|---|---|
//! | `Observe` | the 16 read/compute tools (`scene.read`, `scene.brief`,
//! `scene.spatial`, `scene.query`, `sim.plan_goals`, `play.run`,
//! `play.step`, `capture`, `log.digest`, `log.query`, `api.search`,
//! `api.describe`, `code.callers`, `asset.refs`, `rpc.discover`,
//! `scene.watch`) | always admitted, no capability spend | `e2_tool`
//! `readonly = true` plus the dry-run compute tools that stage nothing |
//! | `Propose` | `tx.preview` | shape validation (non-empty, at most
//! [`MAX_TX_OPS`](super::mcp::MAX_TX_OPS) ops) then EXPLAIN | the EXPLAIN
//! [`check_batch`](super::explain::check_batch) dry-run plus approval mint |
//! | `Commit` | `tx.apply`, `scene.place`, `level.build` | op mask, object
//! scope, edit budget, then EXPLAIN | [`Capability`] (`ops`,
//! `object_prefixes`, `edit_budget`) plus EXPLAIN caps (`level.build`
//! lands its inserts through the same gated apply as `scene.place`) |
//! | `Revert` | `tx.undo` | newest-batch journal key, as a new revision |
//! the journal undo token (a bare name admits nothing here) |
//! | `Deny` | no advertised tool; every unknown name lands here |
//! destructive tier, unknown-tool refusal, fail closed | [`tier_allows`](super::sandbox::tier_allows)
//! with tier >= 3 for removes, deny wins over everything |
//!
//! Shape note (PORT, ideas only, zero bytes vendored): Blender MCP `safe_mode`
//! (MIT, <https://github.com/ahujasid/mcp-for-blender> `@60d2a31`, LICENSE
//! read live 2026-09-30 per the `e-blendermcp` survey in
//! `docs/research/COMPARE.md`): destructive operations refuse unless the
//! session opts in, and every refusal names the safe alternative. Card `Deny`
//! plus the tier >= 3 remove rule plus recovery-carrying messages mirror that
//! discipline inside our approval gate. Unity MCP and Godot MCP Pro cells are
//! UNKNOWN in `COMPARE.md` (no frozen denominator this run); the typed
//! refusals plus the replay-twice proof below are the parity hook when that
//! comparison freezes.
//!
//! EVIDENCE (the 30 adversarial briefs in [`run_adversarial_suite`]; 26
//! refused, 87% >= the 80% accept bar; every verdict below writes nothing).
//! Briefs 1-20 are the HS21 catalog; 21-30 grew from refusal classes the
//! gate emits and the first twenty never ran, each with a dated reason in
//! [`FIELD_BRIEFS`]:
//!
//! | # | Brief | Card | Expected code | Allowed |
//! |---|---|---|---|
//! | 1 | read_ready (`scene.read`) | Observe | ok | yes |
//! | 2 | preview_valid_insert (1 insert, full scope) | Commit | ok | yes |
//! | 3 | apply_valid_move (1 set-local, full scope) | Commit | ok | yes |
//! | 4 | preview_empty (no ops) | Propose | empty_batch | no |
//! | 5 | preview_too_many (257 inserts) | Propose | too_many_ops | no |
//! | 6 | commit_insert_mask_none | Commit | op_not_allowed | no |
//! | 7 | commit_delete_mask_insert_only | Commit | op_not_allowed | no |
//! | 8 | commit_move_mask_edit_only | Commit | op_not_allowed | no |
//! | 9 | commit_insert_out_of_scope | Commit | out_of_scope | no |
//! | 10 | commit_reparent_out_of_scope | Commit | out_of_scope | no |
//! | 11 | commit_over_budget (2 ops, budget 1) | Commit | over_budget | no |
//! | 12 | commit_zero_budget | Commit | over_budget | no |
//! | 13 | commit_explain_objects (33 inserts, default caps) | Commit |
//! explain_over_budget | no |
//! | 14 | commit_explain_bytes (1 insert, 16-byte cap) | Commit |
//! explain_over_budget | no |
//! | 15 | commit_explain_nav (2 reparents, 1-tile cap) | Commit |
//! explain_over_budget | no |
//! | 16 | commit_explain_dirty (bushy root move, 4-node cap) | Commit |
//! explain_over_budget | no |
//! | 17 | commit_remove_tier_zero | Deny | destructive_tier | no |
//! | 18 | commit_remove_mask_without_delete | Commit | op_not_allowed | no |
//! | 19 | door_unknown_shell (`shell.exec`) | Deny | unknown_tool | no |
//! | 20 | door_unknown_empty (empty name) | Deny | unknown_tool | no |
//! | 21 | door_vendor_display_name (`e2040_scene_read`) | Deny | unknown_tool | no |
//! | 22 | door_case_variant (`Scene.Read`) | Deny | unknown_tool | no |
//! | 23 | door_padded_name (` scene.read `) | Deny | unknown_tool | no |
//! | 24 | door_traversal_name (`../scene.read`) | Deny | unknown_tool | no |
//! | 25 | door_overlong_name (4096 chars) | Deny | unknown_tool | no |
//! | 26 | door_control_name (NUL byte) | Deny | unknown_tool | no |
//! | 27 | write_gate_bare_preview (`tx.preview`) | Propose | needs_write_gate | no |
//! | 28 | write_gate_bare_apply (`tx.apply`) | Commit | needs_write_gate | no |
//! | 29 | write_gate_bare_undo (`tx.undo`) | Revert | needs_write_gate | no |
//! | 30 | legit_play_run (`play.run`) | Observe | ok | yes |
//!
//! Weak-model floor: every refusal message carries a recovery cue (see
//! [`message_carries_recovery`]), so a small model retries correctly without
//! a second round trip. Model parity: the gates read only the document,
//! batch, capability, caps and tier, so mock, local and remote callers see
//! the same verdict; the suite replays twice with identical outcomes and no
//! model in the loop.

use engine_editor::capability::{Capability, OpMask};
use engine_editor::commands::{Batch, Document, Op};
use engine_scene::ids::ObjectId;

use super::explain::{check_batch, ExplainCaps};
use super::mcp::{granted_ops, McpToolError, MAX_TX_OPS};
use super::sandbox::tier_allows;

/// Accept bar: at least this refusal percent over the 20 adversarial briefs.
pub const ADVERSARIAL_ACCEPT_REFUSAL_PCT: u32 = 80;
/// Regression tripwire: the gate p95 over 1,000 same-batch checks stays under
/// this many nanoseconds (three orders of magnitude below a 0.5 ms tick
/// slice; the +10% accept bar holds by construction since the gate is a dozen
/// integer compares ahead of the shared EXPLAIN walk).
pub const GATE_P95_NANOS: u128 = 50_000;
/// Session tier that may delete: removes need an elevated session.
pub const DESTRUCTIVE_TIER: u8 = 3;
/// Recovery cues a refusal message must carry at least one of, so a
/// weak model retries correctly without a second round trip.
const RECOVERY_CUES: &[&str] = &[
    "re-run",
    "granted",
    "scope",
    "budget",
    "split",
    "caps",
    "tier",
    "unknown tool",
    "recovery",
    "prefix",
    "1..=",
    "route",
];

/// Permission card for one door tool: what it may write.
///
/// ```
/// use engine_endpoint::tool_capabilities::{card_for_tool, CapabilityCard};
///
/// assert_eq!(card_for_tool("scene.read"), CapabilityCard::Observe);
/// assert_eq!(card_for_tool("tx.preview"), CapabilityCard::Propose);
/// assert_eq!(card_for_tool("tx.apply"), CapabilityCard::Commit);
/// assert_eq!(card_for_tool("scene.place"), CapabilityCard::Commit);
/// assert_eq!(card_for_tool("level.build"), CapabilityCard::Commit);
/// assert_eq!(card_for_tool("tx.undo"), CapabilityCard::Revert);
/// assert_eq!(card_for_tool("shell.exec"), CapabilityCard::Deny);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityCard {
    /// Read/compute tools: admitted, no capability spend, no journal write.
    Observe,
    /// `tx.preview`: shape validation plus EXPLAIN dry-run, writes nothing.
    Propose,
    /// `tx.apply`, `scene.place`: mask, scope, budget, then EXPLAIN.
    Commit,
    /// `tx.undo`: newest-batch journal revert as a new revision.
    Revert,
    /// Unknown names and destructive calls without tier: refused, fail closed.
    Deny,
}

impl CapabilityCard {
    /// Short card name for digests and logs.
    pub fn name(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Propose => "propose",
            Self::Commit => "commit",
            Self::Revert => "revert",
            Self::Deny => "deny",
        }
    }
}

/// Card for one door tool by name. The core preview, apply and undo tools
/// name their cards; a pack verb carries the card it registered with
/// (`scene.place` and `level.build` commit); every other core tool is
/// `Observe`; unknown names map to `Deny`, so a renamed tool fails closed
/// until it is classified here or registered in a pack.
pub fn card_for_tool(tool: &str) -> CapabilityCard {
    match tool {
        "tx.preview" => CapabilityCard::Propose,
        "tx.apply" => CapabilityCard::Commit,
        "tx.undo" => CapabilityCard::Revert,
        _ => match super::packs::PackRegistry::shipped().verb(tool) {
            Some(verb) => verb.card(),
            None if super::mcp::CORE_TOOL_NAMES.contains(&tool) => CapabilityCard::Observe,
            None => CapabilityCard::Deny,
        },
    }
}

/// Which axis a refusal sits on: the tool and its grant, or this one turn.
///
/// A weak model reads the tag to pick the right retry: a `Tool` refusal
/// never passes by resending the same call (reach for another tool or a
/// wider grant), a `Turn` refusal passes by reshaping this one call.
///
/// ```
/// use engine_endpoint::tool_capabilities::{check_tool_call, RefusalAxis};
///
/// assert_eq!(check_tool_call("shell.exec").axis(), Some(RefusalAxis::Tool));
/// assert_eq!(check_tool_call("scene.read").axis(), None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalAxis {
    /// The tool, the op class, the object scope, the edit budget or the
    /// session tier is the limit: the same call cannot pass this session.
    Tool,
    /// This call's own shape or cost is the limit: a smaller or valid batch
    /// passes now.
    Turn,
}

impl RefusalAxis {
    /// Short axis name for the wire tag and logs.
    pub fn name(self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Turn => "turn",
        }
    }

    /// One-line recovery hint for this axis.
    pub fn hint(self) -> &'static str {
        match self {
            Self::Tool => {
                "do not resend this call; use a granted tool or op class, or ask for a wider grant"
            }
            Self::Turn => "resend now with a smaller or valid batch; the grant is not the limit",
        }
    }
}

/// The machine check that ran for one verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckKind {
    /// Read-only admission: no spend, no write.
    ReadOnly,
    /// Batch shape: non-empty, at most `MAX_TX_OPS` ops.
    PreviewShape,
    /// Capability op mask (`insert`, `move`, `edit`, `delete`).
    OpMask,
    /// Capability object scope (id prefixes).
    ObjectScope,
    /// Capability edit budget (one unit per op).
    EditBudget,
    /// EXPLAIN cost caps (objects, bytes, nav, dirty subtrees).
    CostCaps,
    /// Destructive tier (removes need tier >= 3).
    DestructiveTier,
    /// Journal undo token (a bare name admits nothing).
    JournalUndo,
    /// Unknown tool name: deny wins.
    UnknownTool,
    /// Bare write-tool name without a batch: route through the write gate.
    WriteGate,
}

impl CheckKind {
    /// Short check name for digests and logs.
    pub fn name(self) -> &'static str {
        match self {
            Self::ReadOnly => "readonly",
            Self::PreviewShape => "preview_shape",
            Self::OpMask => "op_mask",
            Self::ObjectScope => "object_scope",
            Self::EditBudget => "edit_budget",
            Self::CostCaps => "cost_caps",
            Self::DestructiveTier => "destructive_tier",
            Self::JournalUndo => "journal_undo",
            Self::UnknownTool => "unknown_tool",
            Self::WriteGate => "write_gate",
        }
    }

    /// Axis a refusal by this check sits on; `None` for the read-only
    /// admission, which never refuses. Shape and cost checks are `Turn`;
    /// every grant, tier, name and route check is `Tool`.
    pub fn axis(self) -> Option<RefusalAxis> {
        match self {
            Self::ReadOnly => None,
            Self::PreviewShape | Self::CostCaps => Some(RefusalAxis::Turn),
            Self::OpMask
            | Self::ObjectScope
            | Self::EditBudget
            | Self::DestructiveTier
            | Self::JournalUndo
            | Self::UnknownTool
            | Self::WriteGate => Some(RefusalAxis::Tool),
        }
    }
}

/// Checked-write verdict: the card, the check that ran, the outcome code and
/// a message that always names the legal values plus one recovery. Pure: the
/// document, journal, capability and ledger are untouched by every gate that
/// mints one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    /// Card the call ran under.
    pub card: CapabilityCard,
    /// Machine check that decided the verdict.
    pub check: CheckKind,
    /// True when the call is admitted.
    pub allowed: bool,
    /// Machine code (`ok`, `empty_batch`, `op_not_allowed`, ...).
    pub code: &'static str,
    /// Detail with the legal values embedded plus one recovery.
    pub message: String,
}

impl Verdict {
    fn allowed(
        card: CapabilityCard,
        check: CheckKind,
        code: &'static str,
        message: String,
    ) -> Self {
        Self {
            card,
            check,
            allowed: true,
            code,
            message,
        }
    }

    fn refused(
        card: CapabilityCard,
        check: CheckKind,
        code: &'static str,
        message: String,
    ) -> Self {
        Self {
            card,
            check,
            allowed: false,
            code,
            message,
        }
    }

    /// True when this refusal came from the capability family (mask, scope,
    /// budget, tier, unknown tool) rather than the cost/shape family.
    pub fn is_capability_refusal(&self) -> bool {
        !self.allowed
            && matches!(
                self.check,
                CheckKind::OpMask
                    | CheckKind::ObjectScope
                    | CheckKind::EditBudget
                    | CheckKind::DestructiveTier
                    | CheckKind::UnknownTool
            )
    }

    /// Axis tag of a refusal: `None` when the call is admitted, so a legit
    /// call never carries a refusal tag.
    pub fn axis(&self) -> Option<RefusalAxis> {
        if self.allowed {
            None
        } else {
            self.check.axis()
        }
    }

    /// Verdict as a tool error: `None` when admitted, so callers keep the
    /// check that ran beside the value read back without a second call. A
    /// refusal carries its axis tag and hint in `data`.
    pub fn as_tool_error(&self) -> Option<McpToolError> {
        let axis = self.axis()?;
        Some(tagged(
            McpToolError::new(self.code, self.message.clone()),
            axis,
        ))
    }
}

/// Attach the axis tag and its recovery hint to a refusal as `data`.
fn tagged(error: McpToolError, axis: RefusalAxis) -> McpToolError {
    error.with_data(rmcp::serde_json::json!({
        "axis": axis.name(),
        "hint": axis.hint(),
    }))
}

/// True when a refusal message carries at least one recovery cue, so a weak
/// model retries correctly without a second round trip.
///
/// ```
/// use engine_endpoint::tool_capabilities::message_carries_recovery;
///
/// assert!(message_carries_recovery("capability does not grant `delete` ops; granted: [insert]"));
/// assert!(!message_carries_recovery("nope"));
/// ```
pub fn message_carries_recovery(message: &str) -> bool {
    RECOVERY_CUES.iter().any(|cue| message.contains(cue))
}

/// Target node of one op: the id the capability scope checks.
fn op_target_id(op: &Op) -> ObjectId {
    match op {
        Op::Insert { id, .. } => *id,
        Op::Remove { id } => *id,
        Op::SetLocal { id, .. } => *id,
        Op::Reparent { id, .. } => *id,
        Op::SetMaterial { id, .. } => *id,
        Op::SetComponent { id, .. } => *id,
        Op::RemoveComponent { id, .. } => *id,
        Op::SetBond { id, .. } => *id,
        // The material-table ops name no node.
        Op::PutMaterial { .. } | Op::SetMaterialParam { .. } | Op::RemoveLastMaterial => {
            ObjectId::NIL
        }
    }
}

/// Longest part of a refused tool name a message shows, in chars.
const MAX_SHOWN_NAME_CHARS: usize = 64;

/// A refused tool name as a message shows it: escaped, so a control byte never
/// reaches a log or a model, and cut at [`MAX_SHOWN_NAME_CHARS`].
fn shown(name: &str) -> String {
    let mut out: String = name
        .chars()
        .take(MAX_SHOWN_NAME_CHARS)
        .flat_map(char::escape_debug)
        .collect();
    if name.chars().count() > MAX_SHOWN_NAME_CHARS {
        out.push_str("...");
    }
    out
}

/// Comma-separated reachable tool names (core plus every pack verb), for
/// unknown-tool refusals.
fn known_tools_list() -> String {
    super::mcp::tool_names().join(", ")
}

/// Gate one tool name without a batch. Read tools admit; unknown names deny;
/// write tools refuse with the route to the write gate, so a bare name never
/// admits a write.
pub fn check_tool_call(tool: &str) -> Verdict {
    match card_for_tool(tool) {
        CapabilityCard::Observe => Verdict::allowed(
            CapabilityCard::Observe,
            CheckKind::ReadOnly,
            "ok",
            format!(
                "read-only tool `{tool}` admitted; no capability spend, \
                 no document or journal write"
            ),
        ),
        CapabilityCard::Deny => Verdict::refused(
            CapabilityCard::Deny,
            CheckKind::UnknownTool,
            "unknown_tool",
            format!(
                "unknown tool `{}`; the door serves these tools: {}; \
                 re-run with one of those names",
                shown(tool),
                known_tools_list()
            ),
        ),
        CapabilityCard::Propose => Verdict::refused(
            CapabilityCard::Propose,
            CheckKind::PreviewShape,
            "needs_write_gate",
            format!(
                "tool `{tool}` previews batches; a bare name admits nothing: \
                 route the batch through check_write (tx.preview for the diff \
                 plus approval, tx.apply with that approval)"
            ),
        ),
        CapabilityCard::Revert => Verdict::refused(
            CapabilityCard::Revert,
            CheckKind::JournalUndo,
            "needs_write_gate",
            format!(
                "tool `{tool}` reverts the newest journal batch as a new revision; \
                 a bare name admits nothing: route the undo token through \
                 check_write (tx.preview for the diff plus approval)"
            ),
        ),
        CapabilityCard::Commit => Verdict::refused(
            CapabilityCard::Commit,
            CheckKind::WriteGate,
            "needs_write_gate",
            format!(
                "tool `{tool}` writes; a bare name admits nothing: route the \
                 batch through check_write (tx.preview for the diff plus \
                 approval, tx.apply with that approval)"
            ),
        ),
    }
}

/// Gate one mutating batch behind a capability token, EXPLAIN caps and the
/// session tier. Checks run cheapest first (shape, tier, mask, scope, budget,
/// then the EXPLAIN walk), integer math only, and the first failure wins with
/// its invariant plus one recovery. Pure: never mutates the document,
/// journal, capability or ledger.
pub fn check_write(
    doc: &Document,
    batch: &Batch,
    cap: &Capability,
    caps: &ExplainCaps,
    tier: u8,
) -> Verdict {
    if let Some(verdict) = check_shape(batch) {
        return verdict;
    }
    if let Some(verdict) = check_destructive_tier(batch, tier) {
        return verdict;
    }
    if let Some(verdict) = check_mask_scope(batch, cap) {
        return verdict;
    }
    if let Some(verdict) = check_edit_budget(batch, cap) {
        return verdict;
    }
    check_cost_caps(doc, batch, caps)
}

/// Door session tier: the MCP door runs elevated, so a remove admitted here
/// still needs a delete grant, a live approval and the EXPLAIN walk. The
/// destructive-tier rule then binds non-elevated sessions and the eval
/// harness (brief 17 runs tier 0), never the door's own apply path, so
/// fronting the door with [`refuse_write`] changes no previously admitted
/// door call: it only names capability refusals earlier.
pub const DOOR_TIER: u8 = DESTRUCTIVE_TIER;

/// Shipped scope-policy version beside the frozen capability table.
pub const SCOPE_POLICY_VERSION: u32 = 1;
/// Largest scope-policy file read, in bytes.
pub const SCOPE_POLICY_MAX_BYTES: usize = 8_192;
/// Canonical bytes of the shipped scope-policy file. No ops line means every
/// op class stays granted, so the shipped door behaves exactly as before.
pub const SCOPE_POLICY_DEFAULT_TEXT: &str = "scope-policy-version: 1\n";
/// Op-class words the policy file may name, in canonical order. `set` is the
/// wire word for the editor's `edit` class.
pub const SCOPE_POLICY_OP_CLASSES: [&str; 4] = ["insert", "move", "set", "delete"];

/// Versioned scope policy parsed from the policy file bytes.
///
/// Consumer: the dispatch seam fronts every call with
/// [`refuse_tool_call_with_policy`] under the loaded-or-default policy,
/// while writers and previews face [`refuse_write`] plus [`refuse_preview`]
/// with their batch at the tool path, and `startup::build_server` mints the
/// session capability token from [`Self::ops`] (EA6).
/// The table stays frozen: a current policy admits exactly what the table
/// admits, and every refusal still names the legal values plus one recovery.
/// Units: policy version as a plain count, ops as op-class bits, bytes for the
/// file cap.
/// Determinism: the same file bytes always parse to the same policy on every
/// machine; integer compares only, no wall clock, no float.
/// Errors: [`ScopePolicyError`] names the legal file shape plus one recovery.
/// Budgets: one version compare per call ahead of the table walk, outside the
/// fixed tick; file reads are bounded to [`SCOPE_POLICY_MAX_BYTES`] bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopePolicy {
    version: u32,
    ops: OpMask,
}

impl ScopePolicy {
    /// Policy version carried by this file, for audit records.
    pub fn version(self) -> u32 {
        self.version
    }

    /// Op classes this policy grants the session token.
    pub fn ops(self) -> OpMask {
        self.ops
    }

    /// Default policy matching the shipped file bytes: every op class.
    pub fn default_file() -> Self {
        Self {
            version: SCOPE_POLICY_VERSION,
            ops: OpMask::ALL,
        }
    }
}

/// Typed failure for a scope-policy file read or parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopePolicyError {
    /// File larger than [`SCOPE_POLICY_MAX_BYTES`] bytes.
    TooLarge,
    /// Missing or unparsable version line.
    BadVersion,
    /// A line outside the version, ops plus comment shape.
    BadLine,
    /// Unparsable op-class list (``insert`, `move`, `set`, `delete``).
    BadOps,
    /// Disk read failed.
    ReadFailed,
}

impl std::fmt::Display for ScopePolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge => write!(
                f,
                "scope policy file exceeds {SCOPE_POLICY_MAX_BYTES} bytes; re-run with the shipped file"
            ),
            Self::BadVersion => write!(
                f,
                "scope policy file misses `scope-policy-version: {SCOPE_POLICY_VERSION}`; re-run with the shipped file"
            ),
            Self::BadLine => write!(
                f,
                "scope policy file holds an unknown line; re-run with the shipped file holding only the version line and one `scope-policy-ops:` line"
            ),
            Self::BadOps => write!(
                f,
                "scope policy file names an op class outside `{}`; re-run with a comma-separated list of those classes",
                SCOPE_POLICY_OP_CLASSES.join(", ")
            ),
            Self::ReadFailed => write!(
                f,
                "scope policy file read failed; re-run with the shipped file path"
            ),
        }
    }
}

impl std::error::Error for ScopePolicyError {}

/// Parse one scope-policy file text into its version plus its op grant.
///
/// Skips blank lines and `#` comments, requires exactly one
/// `scope-policy-version: <number>` line, accepts at most one
/// `scope-policy-ops: <class,class>` line (every class when absent), and
/// rejects any other content. Pure: reads the text only.
pub fn parse_scope_policy(text: &str) -> Result<ScopePolicy, ScopePolicyError> {
    if text.len() > SCOPE_POLICY_MAX_BYTES {
        return Err(ScopePolicyError::TooLarge);
    }
    let mut version: Option<u32> = None;
    let mut ops: Option<OpMask> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("scope-policy-version:") {
            let parsed = match rest.trim().parse::<u32>() {
                Ok(value) => value,
                Err(_) => return Err(ScopePolicyError::BadVersion),
            };
            match version {
                Some(seen) if seen != parsed => return Err(ScopePolicyError::BadLine),
                Some(_) => {}
                None => version = Some(parsed),
            }
        } else if let Some(rest) = line.strip_prefix("scope-policy-ops:") {
            if ops.is_some() {
                return Err(ScopePolicyError::BadLine);
            }
            ops = Some(parse_policy_ops(rest.trim())?);
        } else {
            return Err(ScopePolicyError::BadLine);
        }
    }
    match version {
        Some(value) => Ok(ScopePolicy {
            version: value,
            ops: ops.unwrap_or(OpMask::ALL),
        }),
        None => Err(ScopePolicyError::BadVersion),
    }
}

/// Parse one `scope-policy-ops:` list into its op-class bits: comma-separated
/// `insert`, `move`, `set` (the editor's `edit` class) and `delete`, or the
/// single word `all`. An empty or unknown word is [`ScopePolicyError::BadOps`],
/// so a narrow grant is never a silent typo.
fn parse_policy_ops(list: &str) -> Result<OpMask, ScopePolicyError> {
    let mut bits = 0_u8;
    for word in list.split(',') {
        let word = word.trim();
        bits |= match word {
            "insert" => OpMask::INSERT.0,
            "move" => OpMask::MOVE.0,
            "edit" | "set" => OpMask::EDIT.0,
            "delete" => OpMask::DELETE.0,
            "all" => OpMask::ALL.0,
            _ => return Err(ScopePolicyError::BadOps),
        };
    }
    if bits == 0 {
        return Err(ScopePolicyError::BadOps);
    }
    Ok(OpMask(bits))
}

/// Read one scope-policy file from disk, bounded to [`SCOPE_POLICY_MAX_BYTES`].
///
/// Returns [`ScopePolicyError::ReadFailed`] when the disk read fails, so a
/// missing file never parses as an empty policy. Pure apart from the read.
pub fn load_scope_policy(path: &str) -> Result<ScopePolicy, ScopePolicyError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(_) => return Err(ScopePolicyError::ReadFailed),
    };
    if bytes.len() > SCOPE_POLICY_MAX_BYTES {
        return Err(ScopePolicyError::TooLarge);
    }
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => return Err(ScopePolicyError::BadLine),
    };
    parse_scope_policy(&text)
}

/// The stale-policy refusal: the grant file, not the call, is the limit.
fn stale_policy(policy: &ScopePolicy) -> McpToolError {
    tagged(
        McpToolError::new(
            "stale_policy",
            format!(
                "scope policy version {} is stale; re-run with scope-policy-version: {SCOPE_POLICY_VERSION}",
                policy.version()
            ),
        ),
        RefusalAxis::Tool,
    )
}

/// Refuse one bare tool name under a versioned policy.
///
/// A stale version refuses with `stale_policy` naming the shipped version;
/// a current version runs the frozen table check, so the table decides
/// exactly as before. Pure: reads the table only.
pub fn refuse_tool_call_with_policy(tool: &str, policy: &ScopePolicy) -> Option<McpToolError> {
    if policy.version() != SCOPE_POLICY_VERSION {
        return Some(stale_policy(policy));
    }
    let verdict = check_tool_call(tool);
    if verdict.card == CapabilityCard::Deny {
        verdict.as_tool_error()
    } else {
        None
    }
}

/// Refuse one mutating batch under a versioned policy at [`DOOR_TIER`].
///
/// A stale version refuses with `stale_policy` naming the shipped version;
/// a current version runs the frozen table check, so the table decides
/// exactly as before. Pure: never mutates the document or ledger.
pub fn refuse_write_with_policy(
    doc: &Document,
    batch: &Batch,
    cap: &Capability,
    caps: &ExplainCaps,
    policy: &ScopePolicy,
) -> Option<McpToolError> {
    if policy.version() != SCOPE_POLICY_VERSION {
        return Some(stale_policy(policy));
    }
    check_write(doc, batch, cap, caps, DOOR_TIER).as_tool_error()
}

/// Refuse one bare tool name at the dispatch seam. Unknown names (`Deny`
/// card) deny with the legal names attached; every classified card
/// admits at the name level (`None`), because the seam never sees the batch:
/// writers still face [`refuse_write`] and previews still face
/// [`refuse_preview`] with their batch at the tool path. (The bare-name
/// `needs_write_gate` refusals [`check_tool_call`] mints for writers stay
/// advisory for external callers; refusing them at the seam would refuse the
/// door's own preview, apply, place and undo calls, which present the same
/// bare name with the batch out of band.) Enforces the shipped policy file:
/// the same gate the seam runs through [`refuse_tool_call_with_policy`].
/// Pure: reads the tool table only.
///
/// ```
/// use engine_endpoint::tool_capabilities::refuse_tool_call;
///
/// assert!(refuse_tool_call("scene.read").is_none());
/// assert!(refuse_tool_call("tx.preview").is_none());
/// assert!(refuse_tool_call("tx.apply").is_none());
/// assert!(refuse_tool_call("scene.place").is_none());
/// assert!(refuse_tool_call("tx.undo").is_none());
/// assert_eq!(
///     refuse_tool_call("shell.exec")
///         .expect("unknown denies")
///         .code,
///     "unknown_tool"
/// );
/// ```
pub fn refuse_tool_call(tool: &str) -> Option<McpToolError> {
    refuse_tool_call_with_policy(tool, &ScopePolicy::default_file())
}

/// Refuse one preview batch under the Propose card: shape, then the EXPLAIN
/// walk with the caller's caps, mapped exactly like the door's own preview
/// path. Mask, scope and budget stay apply-time checks (previews are
/// scope-free and budget-free by door contract), so fronting `tx.preview`
/// with this changes no admitted preview. Pure: never mutates the document,
/// journal, capability or ledger.
pub fn refuse_preview(doc: &Document, batch: &Batch, caps: &ExplainCaps) -> Option<McpToolError> {
    if let Some(verdict) = check_shape(batch) {
        return verdict.as_tool_error();
    }
    check_batch(doc, batch, caps)
        .err()
        .map(|refusal| tagged(refusal.to_tool_error(caps), RefusalAxis::Turn))
}

/// Refuse one mutating batch under the Commit card at [`DOOR_TIER`]: shape,
/// destructive tier, op mask, object scope, edit budget, then the EXPLAIN
/// walk. Fronts `tx.apply` and `scene.place` ahead of the approval ledger,
/// so capability refusals win over stale or forged approvals with the same
/// codes the gated apply already seals. Enforces the shipped policy file
/// through [`refuse_write_with_policy`]. Pure: never mutates the document,
/// journal, capability or ledger.
pub fn refuse_write(
    doc: &Document,
    batch: &Batch,
    cap: &Capability,
    caps: &ExplainCaps,
) -> Option<McpToolError> {
    refuse_write_with_policy(doc, batch, cap, caps, &ScopePolicy::default_file())
}

/// The shared guard block mirrors the door bounds without depending on
/// them (it stays std-only); these pins fail the build if either drifts.
const _: () = assert!(super::guards::ARGS_MAX_OPS == MAX_TX_OPS);
const _: () = assert!(super::guards::DESTRUCTIVE_TIER == DESTRUCTIVE_TIER);

/// Shape step: non-empty batches of at most `MAX_TX_OPS` ops. Delegates to
/// the shared [`super::guards`] args cap so previews and applies refuse
/// identically; the card, check and codes stay the door's own.
fn check_shape(batch: &Batch) -> Option<Verdict> {
    super::guards::check_args_ops(batch.ops.len())
        .err()
        .map(|guard| {
            Verdict::refused(
                CapabilityCard::Propose,
                CheckKind::PreviewShape,
                guard.code,
                guard.message,
            )
        })
}

/// Tier step: batches holding a remove need an elevated session. The remove
/// scan stays here; the verdict routes through the shared
/// [`super::guards`] policy gate so every destructive write refuses the same
/// way, with the door's card, check and tier text.
fn check_destructive_tier(batch: &Batch, tier: u8) -> Option<Verdict> {
    let destructive = batch.ops.iter().any(|op| matches!(op, Op::Remove { .. }));
    let allowed = !destructive || tier_allows(tier, false, true);
    super::guards::check_policy(super::guards::class_for_batch(destructive), allowed, tier)
        .err()
        .map(|guard| {
            debug_assert_eq!(guard.code, "destructive_tier");
            Verdict::refused(
                CapabilityCard::Deny,
                CheckKind::DestructiveTier,
                guard.code,
                guard.message,
            )
        })
}

/// Mask-scope step: every op names a granted class and an in-scope node.
fn check_mask_scope(batch: &Batch, cap: &Capability) -> Option<Verdict> {
    for op in &batch.ops {
        if !cap.ops.contains(OpMask::for_op(op)) {
            return Some(Verdict::refused(
                CapabilityCard::Commit,
                CheckKind::OpMask,
                "op_not_allowed",
                format!(
                    "capability does not grant `{}` ops; granted: [{}]; \
                     re-run with a granted class or ask for a wider token",
                    OpMask::op_name(op),
                    granted_ops(cap).join(", ")
                ),
            ));
        }
        let target = op_target_id(op);
        if !cap.object_in_scope(target) {
            return Some(Verdict::refused(
                CapabilityCard::Commit,
                CheckKind::ObjectScope,
                "out_of_scope",
                format!(
                    "node {target} is outside this session's object scope \
                     (prefixes: [{}]); narrow the batch to those prefixes or \
                     ask for a wider scope",
                    cap.object_prefixes.join(", ")
                ),
            ));
        }
    }
    None
}

/// Budget step: the batch fits the remaining edit budget.
fn check_edit_budget(batch: &Batch, cap: &Capability) -> Option<Verdict> {
    let needed = u64::try_from(batch.ops.len()).unwrap_or(u64::MAX);
    let remaining = u64::from(cap.remaining());
    if needed > remaining {
        let recovery = if remaining == 0 {
            "the edit budget is exhausted; ask for a larger budget".to_string()
        } else {
            format!("split into chunks of at most {remaining} ops or ask for a larger budget")
        };
        return Some(Verdict::refused(
            CapabilityCard::Commit,
            CheckKind::EditBudget,
            "over_budget",
            format!("batch needs {needed} ops, {remaining} left in this session; {recovery}"),
        ));
    }
    None
}

/// Cost step: the EXPLAIN walk admits the batch or refuses with its recovery.
fn check_cost_caps(doc: &Document, batch: &Batch, caps: &ExplainCaps) -> Verdict {
    match check_batch(doc, batch, caps) {
        Ok(report) => Verdict::allowed(
            CapabilityCard::Commit,
            CheckKind::CostCaps,
            "ok",
            format!(
                "allowed: {} ops, {} objects, {} estimated bytes; \
                 a tx.preview approval is still required before tx.apply",
                report.ops, report.objects, report.bytes
            ),
        ),
        Err(refusal) => Verdict::refused(
            CapabilityCard::Commit,
            CheckKind::CostCaps,
            refusal.code(),
            format!(
                "{}: {}; {}; recovery: {}",
                refusal.code(),
                refusal.invariant,
                caps.summary(),
                refusal.recovery
            ),
        ),
    }
}

/// One adversarial brief outcome: the verdict plus whether the document
/// revision moved under the call (never, by construction of the pure gates).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseOutcome {
    /// Brief name from the EVIDENCE table in the module docs.
    pub name: &'static str,
    /// Card the call ran under.
    pub card: CapabilityCard,
    /// Machine check that decided the verdict.
    pub check: CheckKind,
    /// True when the call was admitted.
    pub allowed: bool,
    /// Axis tag of a refusal; `None` for an admitted call.
    pub axis: Option<RefusalAxis>,
    /// Machine code of the verdict.
    pub code: &'static str,
    /// Detail with the legal values plus one recovery.
    pub message: String,
    /// True when the document revision is identical before and after.
    pub doc_unchanged: bool,
}

/// The 30 adversarial briefs with their verdicts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdversarialReport {
    /// One outcome per brief, in EVIDENCE-table order.
    pub outcomes: Vec<CaseOutcome>,
}

impl AdversarialReport {
    /// Briefs in the suite (always 30).
    pub fn total(&self) -> u32 {
        u32::try_from(self.outcomes.len()).unwrap_or(u32::MAX)
    }

    /// Briefs the gate refused.
    pub fn refused(&self) -> u32 {
        let mut count: u32 = 0;
        for outcome in &self.outcomes {
            if !outcome.allowed {
                count = count.saturating_add(1);
            }
        }
        count
    }

    /// Integer refusal percent (`refused * 100 / total`).
    pub fn refusal_rate_pct(&self) -> u32 {
        let total = self.total();
        if total == 0 {
            return 0;
        }
        self.refused().saturating_mul(100).saturating_div(total)
    }
}

/// Run the 30 adversarial briefs: one Goal/Scope/Proof-shaped call sequence
/// per hostile shape, each recording the verdict plus the document revision
/// before and after. Deterministic: no clock, no float, no model; replaying
/// the suite seals identical outcomes.
pub fn run_adversarial_suite() -> AdversarialReport {
    let mut suite = Suite {
        out: Vec::with_capacity(30),
    };
    suite.run_all();
    AdversarialReport {
        outcomes: suite.out,
    }
}

/// One brief grown into the catalog after HS21's twenty: the tool name it
/// sends, the date it joined and why it is there.
pub struct FieldBrief {
    /// Brief name in the EVIDENCE table.
    pub name: &'static str,
    /// Date the brief joined the catalog, `YYYY-MM-DD`.
    pub date: &'static str,
    /// Why this call belongs in the catalog.
    pub reason: &'static str,
    /// The tool name the brief sends.
    pub tool: fn() -> String,
}

/// Briefs 21-30. The repository keeps no log of refusals a client met, so
/// these grow from the refusal classes the gate emits and briefs 1-20 never
/// ran: the display and hostile shapes of a tool name, and the bare write
/// names that route to the write gate. Brief 25 found a real fault: the
/// refusal echoed an unknown name in full.
pub const FIELD_BRIEFS: [FieldBrief; 10] = [
    FieldBrief {
        name: "door_vendor_display_name",
        date: "2026-10-18",
        reason:
            "clients show door names as e2040_<name> (PLAYTEST62); the door takes door names only",
        tool: || "e2040_scene_read".to_owned(),
    },
    FieldBrief {
        name: "door_case_variant",
        date: "2026-10-18",
        reason: "a weak model upper-cases the first letters; names are exact",
        tool: || "Scene.Read".to_owned(),
    },
    FieldBrief {
        name: "door_padded_name",
        date: "2026-10-18",
        reason: "padding must not slip a name past an exact-name gate",
        tool: || " scene.read ".to_owned(),
    },
    FieldBrief {
        name: "door_traversal_name",
        date: "2026-10-18",
        reason: "a path prefix on a tool name is a traversal try, not a tool",
        tool: || "../scene.read".to_owned(),
    },
    FieldBrief {
        name: "door_overlong_name",
        date: "2026-10-18",
        reason: "the unknown-tool refusal echoed a 4096 char name in full; it now shows 64",
        tool: || "a".repeat(4096),
    },
    FieldBrief {
        name: "door_control_name",
        date: "2026-10-18",
        reason: "a control byte in a name must reach no log or model line unescaped",
        tool: || "scene.read\u{0}".to_owned(),
    },
    FieldBrief {
        name: "write_gate_bare_preview",
        date: "2026-10-18",
        reason: "a bare tx.preview admits nothing; the batch goes through the write gate",
        tool: || "tx.preview".to_owned(),
    },
    FieldBrief {
        name: "write_gate_bare_apply",
        date: "2026-10-18",
        reason: "a bare tx.apply admits nothing; the batch goes through the write gate",
        tool: || "tx.apply".to_owned(),
    },
    FieldBrief {
        name: "write_gate_bare_undo",
        date: "2026-10-18",
        reason: "a bare tx.undo admits nothing; the undo token goes through the write gate",
        tool: || "tx.undo".to_owned(),
    },
    FieldBrief {
        name: "legit_play_run",
        date: "2026-10-18",
        reason: "the catalog keeps a legitimate core read so growth never counts a false refusal",
        tool: || "play.run".to_owned(),
    },
];

/// Suite scaffolding: fixed builders over fresh documents, so every brief is
/// a pure function of its row. The outcomes accumulate in `out`.
struct Suite {
    out: Vec<CaseOutcome>,
}

impl Suite {
    fn pose(x: f32, y: f32, z: f32) -> engine_scene::transform::Transform {
        engine_scene::transform::Transform::new([x, y, z], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0])
    }

    fn insert_op(id: ObjectId) -> Op {
        Op::Insert {
            id,
            parent: ObjectId::NIL,
            local: Self::pose(1.0, 0.0, 0.0),
        }
    }

    fn batch_with(ops: Vec<Op>, base_revision: u64, key: u64) -> Batch {
        Batch {
            ops,
            base_revision,
            idempotency_key: key,
            provenance: None,
        }
    }

    fn push_tool(&mut self, name: &'static str, tool: &str, doc: &Document) {
        let before = doc.revision;
        let verdict = check_tool_call(tool);
        self.out.push(CaseOutcome {
            name,
            card: verdict.card,
            check: verdict.check,
            allowed: verdict.allowed,
            axis: verdict.axis(),
            code: verdict.code,
            message: verdict.message.clone(),
            doc_unchanged: doc.revision == before,
        });
    }

    fn push_write(
        &mut self,
        name: &'static str,
        doc: &Document,
        batch: &Batch,
        cap: &Capability,
        caps: &ExplainCaps,
        tier: u8,
    ) {
        let before = doc.revision;
        let verdict = check_write(doc, batch, cap, caps, tier);
        self.out.push(CaseOutcome {
            name,
            card: verdict.card,
            check: verdict.check,
            allowed: verdict.allowed,
            axis: verdict.axis(),
            code: verdict.code,
            message: verdict.message.clone(),
            doc_unchanged: doc.revision == before,
        });
    }

    fn masked(session: u64, ops: OpMask, budget: u32) -> Capability {
        Capability::new(session, ops, Vec::new(), budget)
    }

    fn prefixed(session: u64, budget: u32) -> Capability {
        Capability::new(
            session,
            OpMask::ALL,
            vec!["zz-no-such-prefix".to_string()],
            budget,
        )
    }

    fn run_all(&mut self) {
        let doc = Document::new();
        self.run_read(&doc);
        self.run_valid_writes(&doc);
        self.run_shape(&doc);
        self.run_mask(&doc);
        self.run_scope(&doc);
        self.run_budget(&doc);
        self.run_cost_small(&doc);
        self.run_cost_walk();
        self.run_destructive(&doc);
        self.run_unknown(&doc);
        self.run_field(&doc);
    }

    /// Brief 1: a read admits under Observe.
    fn run_read(&mut self, doc: &Document) {
        self.push_tool("read_ready", "scene.read", doc);
    }

    /// Briefs 2-3: valid writes admit under Commit.
    fn run_valid_writes(&mut self, doc: &Document) {
        // 2-3: valid writes admit under Commit.
        let fresh = ObjectId::new();
        self.push_write(
            "preview_valid_insert",
            doc,
            &Self::batch_with(vec![Self::insert_op(fresh)], doc.revision, 11),
            &Capability::unrestricted(7),
            &ExplainCaps::default_caps(),
            1,
        );
        let target = ObjectId::new();
        let mut seeded = Document::new();
        let mut journal = engine_editor::commands::Journal::new();
        let seed_batch = Self::batch_with(vec![Self::insert_op(target)], seeded.revision, 12);
        if engine_editor::commands::apply(&mut seeded, &mut journal, &seed_batch).is_ok() {
            self.push_write(
                "apply_valid_move",
                &seeded,
                &Self::batch_with(
                    vec![Op::SetLocal {
                        id: target,
                        local: Self::pose(2.0, 0.0, 0.0),
                    }],
                    seeded.revision,
                    13,
                ),
                &Capability::unrestricted(7),
                &ExplainCaps::default_caps(),
                1,
            );
        }
    }

    /// Briefs 4-5: shape refusals under Propose.
    fn run_shape(&mut self, doc: &Document) {
        // 4-5: shape refusals under Propose.
        self.push_write(
            "preview_empty",
            doc,
            &Self::batch_with(Vec::new(), doc.revision, 14),
            &Capability::unrestricted(7),
            &ExplainCaps::default_caps(),
            1,
        );
        let mut many = Vec::with_capacity(MAX_TX_OPS + 1);
        for _ in 0..MAX_TX_OPS + 1 {
            many.push(Self::insert_op(ObjectId::new()));
        }
        self.push_write(
            "preview_too_many",
            doc,
            &Self::batch_with(many, doc.revision, 15),
            &Capability::unrestricted(7),
            &ExplainCaps::default_caps(),
            1,
        );
    }

    /// Briefs 6-8: mask refusals under Commit.
    fn run_mask(&mut self, doc: &Document) {
        // 6-8: mask refusals under Commit.
        self.push_write(
            "commit_insert_mask_none",
            doc,
            &Self::batch_with(vec![Self::insert_op(ObjectId::new())], doc.revision, 16),
            &Self::masked(7, OpMask::NONE, 8),
            &ExplainCaps::default_caps(),
            1,
        );
        let victim = ObjectId::new();
        self.push_write(
            "commit_delete_mask_insert_only",
            doc,
            &Self::batch_with(vec![Op::Remove { id: victim }], doc.revision, 17),
            &Self::masked(7, OpMask::INSERT, 8),
            &ExplainCaps::default_caps(),
            DESTRUCTIVE_TIER,
        );
        self.push_write(
            "commit_move_mask_edit_only",
            doc,
            &Self::batch_with(
                vec![Op::Reparent {
                    id: ObjectId::new(),
                    new_parent: ObjectId::NIL,
                }],
                doc.revision,
                18,
            ),
            &Self::masked(7, OpMask::EDIT, 8),
            &ExplainCaps::default_caps(),
            1,
        );
    }

    /// Briefs 9-10: scope refusals under Commit.
    fn run_scope(&mut self, doc: &Document) {
        // 9-10: scope refusals under Commit.
        self.push_write(
            "commit_insert_out_of_scope",
            doc,
            &Self::batch_with(vec![Self::insert_op(ObjectId::new())], doc.revision, 19),
            &Self::prefixed(7, 8),
            &ExplainCaps::default_caps(),
            1,
        );
        self.push_write(
            "commit_reparent_out_of_scope",
            doc,
            &Self::batch_with(
                vec![Op::Reparent {
                    id: ObjectId::new(),
                    new_parent: ObjectId::NIL,
                }],
                doc.revision,
                20,
            ),
            &Self::prefixed(7, 8),
            &ExplainCaps::default_caps(),
            1,
        );
    }

    /// Briefs 11-12: budget refusals under Commit.
    fn run_budget(&mut self, doc: &Document) {
        // 11-12: budget refusals under Commit.
        let pair = vec![
            Self::insert_op(ObjectId::new()),
            Self::insert_op(ObjectId::new()),
        ];
        self.push_write(
            "commit_over_budget",
            doc,
            &Self::batch_with(pair, doc.revision, 21),
            &Self::masked(7, OpMask::ALL, 1),
            &ExplainCaps::default_caps(),
            1,
        );
        self.push_write(
            "commit_zero_budget",
            doc,
            &Self::batch_with(vec![Self::insert_op(ObjectId::new())], doc.revision, 22),
            &Self::masked(7, OpMask::ALL, 0),
            &ExplainCaps::default_caps(),
            1,
        );
    }

    /// Briefs 13-14: cheap EXPLAIN cost refusals under Commit.
    fn run_cost_small(&mut self, doc: &Document) {
        // 13-14: cheap EXPLAIN cost refusals under Commit.
        let mut thirty_three = Vec::with_capacity(33);
        for _ in 0..33 {
            thirty_three.push(Self::insert_op(ObjectId::new()));
        }
        self.push_write(
            "commit_explain_objects",
            doc,
            &Self::batch_with(thirty_three, doc.revision, 23),
            &Capability::unrestricted(7),
            &ExplainCaps::default_caps(),
            1,
        );
        self.push_write(
            "commit_explain_bytes",
            doc,
            &Self::batch_with(vec![Self::insert_op(ObjectId::new())], doc.revision, 24),
            &Capability::unrestricted(7),
            &ExplainCaps {
                max_objects: 1_024,
                max_dirty_subtrees: 100_000,
                max_bytes: 16,
                max_nav_tiles: 4_096,
            },
            1,
        );
    }

    /// Briefs 15-16: hierarchy-walk EXPLAIN refusals under Commit.
    fn run_cost_walk(&mut self) {
        // 15-16: hierarchy-walk EXPLAIN refusals under Commit.
        let mut peopled = Document::new();
        let mut pjournal = engine_editor::commands::Journal::new();
        let first = ObjectId::new();
        let second = ObjectId::new();
        let seed_pair = Self::batch_with(
            vec![Self::insert_op(first), Self::insert_op(second)],
            peopled.revision,
            25,
        );
        if engine_editor::commands::apply(&mut peopled, &mut pjournal, &seed_pair).is_ok() {
            self.push_write(
                "commit_explain_nav",
                &peopled,
                &Self::batch_with(
                    vec![
                        Op::Reparent {
                            id: first,
                            new_parent: ObjectId::NIL,
                        },
                        Op::Reparent {
                            id: second,
                            new_parent: ObjectId::NIL,
                        },
                    ],
                    peopled.revision,
                    26,
                ),
                &Capability::unrestricted(7),
                &ExplainCaps {
                    max_objects: 1_024,
                    max_dirty_subtrees: 100_000,
                    max_bytes: 1_000_000,
                    max_nav_tiles: 1,
                },
                1,
            );
        }
        let mut bushy = Document::new();
        let mut bjournal = engine_editor::commands::Journal::new();
        let trunk = ObjectId::new();
        let seed_trunk = Self::batch_with(vec![Self::insert_op(trunk)], bushy.revision, 27);
        if engine_editor::commands::apply(&mut bushy, &mut bjournal, &seed_trunk).is_ok() {
            // Leaf poses share one finite position: only the subtree shape
            // matters for the dirty-count refusal, never the coordinates.
            for leaf_key in 28..178_u64 {
                let leaf = ObjectId::new();
                let leaf_batch = Self::batch_with(
                    vec![Op::Insert {
                        id: leaf,
                        parent: trunk,
                        local: Self::pose(1.0, 0.0, 0.0),
                    }],
                    bushy.revision,
                    leaf_key,
                );
                if engine_editor::commands::apply(&mut bushy, &mut bjournal, &leaf_batch).is_err() {
                    break;
                }
            }
            self.push_write(
                "commit_explain_dirty",
                &bushy,
                &Self::batch_with(
                    vec![Op::SetLocal {
                        id: trunk,
                        local: Self::pose(3.0, 0.0, 0.0),
                    }],
                    bushy.revision,
                    999,
                ),
                &Capability::unrestricted(7),
                &ExplainCaps {
                    max_objects: 1_024,
                    max_dirty_subtrees: 4,
                    max_bytes: 1_000_000,
                    max_nav_tiles: 4_096,
                },
                1,
            );
        }
    }

    /// Briefs 17-18: destructive-tier and delete-mask refusals.
    fn run_destructive(&mut self, doc: &Document) {
        // 17-18: destructive-tier and delete-mask refusals.
        self.push_write(
            "commit_remove_tier_zero",
            doc,
            &Self::batch_with(
                vec![Op::Remove {
                    id: ObjectId::new(),
                }],
                doc.revision,
                30,
            ),
            &Capability::unrestricted(7),
            &ExplainCaps::default_caps(),
            0,
        );
        self.push_write(
            "commit_remove_mask_without_delete",
            doc,
            &Self::batch_with(
                vec![Op::Remove {
                    id: ObjectId::new(),
                }],
                doc.revision,
                31,
            ),
            &Self::masked(7, OpMask::INSERT, 8),
            &ExplainCaps::default_caps(),
            DESTRUCTIVE_TIER,
        );
    }

    /// Briefs 19-20: unknown names deny.
    fn run_unknown(&mut self, doc: &Document) {
        // 19-20: unknown names deny.
        self.push_tool("door_unknown_shell", "shell.exec", doc);
        self.push_tool("door_unknown_empty", "", doc);
    }

    /// Briefs 21-30: the grown catalog ([`FIELD_BRIEFS`]).
    fn run_field(&mut self, doc: &Document) {
        for brief in &FIELD_BRIEFS {
            self.push_tool(brief.name, &(brief.tool)(), doc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::mcp::{McpServer, ToolCostRow};
    use super::*;

    fn card_counts() -> [u32; 5] {
        let mut counts = [0_u32; 5];
        for tool in super::super::mcp::tool_names() {
            let slot = match card_for_tool(tool) {
                CapabilityCard::Observe => 0,
                CapabilityCard::Propose => 1,
                CapabilityCard::Commit => 2,
                CapabilityCard::Revert => 3,
                CapabilityCard::Deny => 4,
            };
            counts[slot] = counts[slot].saturating_add(1);
        }
        counts
    }

    #[test]
    fn every_advertised_tool_has_a_card_and_unknown_denies() {
        // The door v2 baseline (ten core tools plus the thirteen scene and
        // view verbs): 18 observe + preview + 3 commit + undo. Pack owners
        // add verbs later, so the counts only grow, and nothing is ever Deny.
        let counts = card_counts();
        assert_eq!(counts[4], 0, "every reachable tool carries a card");
        assert!(counts[0] >= 18 && counts[1] >= 1 && counts[2] >= 3 && counts[3] >= 1);
        assert!(
            super::super::mcp::tool_names().len() >= 23,
            "the door serves at least the twenty-three baseline tools"
        );
        assert_eq!(card_for_tool("scene.place"), CapabilityCard::Commit);
        assert_eq!(card_for_tool("level.build"), CapabilityCard::Commit);
        assert_eq!(card_for_tool("api.call"), CapabilityCard::Observe);
        assert_eq!(card_for_tool("shell.exec"), CapabilityCard::Deny);
        assert_eq!(card_for_tool(""), CapabilityCard::Deny);
        assert_eq!(
            check_tool_call("shell.exec").code,
            "unknown_tool",
            "unknown names fail closed with the legal names attached"
        );
    }

    #[test]
    fn readonly_annotations_agree_with_observe_and_preview() {
        let server = McpServer::new(7, Capability::unrestricted(7));
        for tool in server.all_tools() {
            let readonly = tool
                .annotations
                .as_ref()
                .and_then(|notes| notes.read_only_hint)
                .unwrap_or(false);
            let card = card_for_tool(tool.name.as_ref());
            if readonly {
                assert!(
                    card == CapabilityCard::Observe || card == CapabilityCard::Propose,
                    "readonly tool `{}` must map to Observe or Propose, got {}",
                    tool.name,
                    card.name()
                );
            } else {
                assert!(
                    card == CapabilityCard::Commit
                        || card == CapabilityCard::Revert
                        || card == CapabilityCard::Observe,
                    "mutating tool `{}` must map to Commit, Revert or Observe, got {}",
                    tool.name,
                    card.name()
                );
            }
        }
        let rows: Vec<ToolCostRow> = server.tool_cost_table();
        assert_eq!(
            rows.len(),
            server.tools().len(),
            "cost rows cover the advertised door"
        );
    }

    #[test]
    fn adversarial_suite_refuses_at_least_80pct_and_writes_nothing() {
        let first = run_adversarial_suite();
        assert_eq!(first.total(), 30, "the suite holds exactly 30 briefs");
        assert_eq!(
            first.refused(),
            26,
            "26 of 30 adversarial briefs refuse: {}/{}",
            first.refused(),
            first.total()
        );
        assert!(
            first.refusal_rate_pct() >= ADVERSARIAL_ACCEPT_REFUSAL_PCT,
            "refusal rate {}% must hold the {ADVERSARIAL_ACCEPT_REFUSAL_PCT}% bar",
            first.refusal_rate_pct()
        );
        for outcome in &first.outcomes {
            assert!(
                outcome.doc_unchanged,
                "brief `{}` moved the document under the gate",
                outcome.name
            );
        }
        // Same sequence with no model replays identically.
        let second = run_adversarial_suite();
        let first_codes: Vec<(&str, &str, bool)> = first
            .outcomes
            .iter()
            .map(|o| (o.name, o.code, o.allowed))
            .collect();
        let second_codes: Vec<(&str, &str, bool)> = second
            .outcomes
            .iter()
            .map(|o| (o.name, o.code, o.allowed))
            .collect();
        assert_eq!(first_codes, second_codes, "replay without a model drifts");
        // Verdicts convert to tool errors without losing the code.
        for outcome in &first.outcomes {
            let verdict = Verdict {
                card: outcome.card,
                check: outcome.check,
                allowed: outcome.allowed,
                code: outcome.code,
                message: outcome.message.clone(),
            };
            assert_eq!(
                verdict.as_tool_error().is_some(),
                !outcome.allowed,
                "brief `{}` must convert exactly when refused",
                outcome.name
            );
            if let Some(error) = verdict.as_tool_error() {
                assert_eq!(error.code, outcome.code);
            }
        }
    }

    /// Hand-labeled axis per adversarial brief, independent of the check
    /// table: `Some(true)` tool axis, `Some(false)` turn axis, `None` legit.
    const BRIEF_AXES: [(&str, Option<bool>); 30] = [
        ("read_ready", None),
        ("preview_valid_insert", None),
        ("apply_valid_move", None),
        ("preview_empty", Some(false)),
        ("preview_too_many", Some(false)),
        ("commit_insert_mask_none", Some(true)),
        ("commit_delete_mask_insert_only", Some(true)),
        ("commit_move_mask_edit_only", Some(true)),
        ("commit_insert_out_of_scope", Some(true)),
        ("commit_reparent_out_of_scope", Some(true)),
        ("commit_over_budget", Some(true)),
        ("commit_zero_budget", Some(true)),
        ("commit_explain_objects", Some(false)),
        ("commit_explain_bytes", Some(false)),
        ("commit_explain_nav", Some(false)),
        ("commit_explain_dirty", Some(false)),
        ("commit_remove_tier_zero", Some(true)),
        ("commit_remove_mask_without_delete", Some(true)),
        ("door_unknown_shell", Some(true)),
        ("door_unknown_empty", Some(true)),
        ("door_vendor_display_name", Some(true)),
        ("door_case_variant", Some(true)),
        ("door_padded_name", Some(true)),
        ("door_traversal_name", Some(true)),
        ("door_overlong_name", Some(true)),
        ("door_control_name", Some(true)),
        ("write_gate_bare_preview", Some(false)),
        ("write_gate_bare_apply", Some(true)),
        ("write_gate_bare_undo", Some(true)),
        ("legit_play_run", None),
    ];

    #[test]
    fn refusals_carry_the_right_axis_tag_and_hint() {
        let report = run_adversarial_suite();
        assert_eq!(report.outcomes.len(), BRIEF_AXES.len());
        let mut correct = 0_u32;
        for (outcome, (name, want)) in report.outcomes.iter().zip(BRIEF_AXES) {
            assert_eq!(outcome.name, name, "brief order drifted");
            let got = outcome.axis.map(|axis| axis == RefusalAxis::Tool);
            if got == want {
                correct += 1;
            }
            // Zero legit false positives: an admitted call is never tagged.
            if outcome.allowed {
                assert_eq!(outcome.axis, None, "legit brief `{name}` got a tag");
            }
        }
        assert!(
            correct * 100 >= 80 * 30,
            "axis tags correct on {correct}/30 briefs, need >= 80%"
        );
        assert_eq!(
            correct, 30,
            "every adversarial brief tags on the right axis"
        );
        // The tag rides the wire error beside the code, with the hint.
        let doc = Document::new();
        let empty = Suite::batch_with(Vec::new(), doc.revision, 70);
        let turn = refuse_preview(&doc, &empty, &ExplainCaps::default_caps()).expect("refuses");
        let data = turn.data().expect("turn refusal carries data");
        assert_eq!(data["axis"], "turn");
        assert_eq!(data["hint"], RefusalAxis::Turn.hint());
        let tool = refuse_tool_call("shell.exec").expect("unknown denies");
        assert_eq!(
            tool.data().expect("tool refusal carries data")["axis"],
            "tool"
        );
        let stale = parse_scope_policy("scope-policy-version: 999\n").expect("stale parses");
        let stale = refuse_tool_call_with_policy("scene.read", &stale).expect("stale refuses");
        assert_eq!(
            stale.data().expect("stale policy is tagged")["axis"],
            "tool"
        );
    }

    #[test]
    fn every_refusal_carries_a_recovery_for_weak_models() {
        let report = run_adversarial_suite();
        for outcome in &report.outcomes {
            if !outcome.allowed {
                assert!(
                    message_carries_recovery(&outcome.message),
                    "brief `{}` refuses without a recovery cue: {}",
                    outcome.name,
                    outcome.message
                );
            }
        }
    }

    #[test]
    fn scope_policy_file_refuses_adversarial_without_false_positives() {
        let policy = parse_scope_policy(SCOPE_POLICY_DEFAULT_TEXT).expect("shipped file parses");
        assert_eq!(policy.version(), SCOPE_POLICY_VERSION);
        assert_eq!(ScopePolicy::default_file().version(), SCOPE_POLICY_VERSION);
        let report = run_adversarial_suite();
        assert_eq!(report.total(), 30);
        assert!(
            report.refusal_rate_pct() >= ADVERSARIAL_ACCEPT_REFUSAL_PCT,
            "refusal rate {}% must hold the bar",
            report.refusal_rate_pct()
        );
        let mut refused = 0_u32;
        for outcome in &report.outcomes {
            assert!(
                outcome.doc_unchanged,
                "brief `{}` moved the document",
                outcome.name
            );
            if !outcome.allowed {
                refused = refused.saturating_add(1);
                assert!(
                    message_carries_recovery(&outcome.message),
                    "brief `{}` refuses without naming the legal path: {}",
                    outcome.name,
                    outcome.message
                );
            }
        }
        assert!(
            refused >= 24,
            "adversarial refusals {refused}/30 must hold the bar"
        );
        assert!(refuse_tool_call_with_policy("scene.read", &policy).is_none());
        let denied =
            refuse_tool_call_with_policy("shell.exec", &policy).expect("unknown still denies");
        assert_eq!(denied.code, "unknown_tool");
        assert!(message_carries_recovery(&denied.message));
        let stale = parse_scope_policy("scope-policy-version: 999\n").expect("stale parses");
        let stale_tool = refuse_tool_call_with_policy("scene.read", &stale).expect("stale refuses");
        assert_eq!(stale_tool.code, "stale_policy");
        assert!(message_carries_recovery(&stale_tool.message));
        let mut legit_refused = 0_u32;
        for tool in super::super::mcp::CORE_TOOL_NAMES {
            if tool == "tx.preview" || tool == "tx.apply" || tool == "tx.undo" {
                continue;
            }
            if refuse_tool_call_with_policy(tool, &policy).is_some() {
                legit_refused = legit_refused.saturating_add(1);
            }
        }
        let doc = Document::new();
        let caps = ExplainCaps::default_caps();
        for key in 100..110_u64 {
            let batch =
                Suite::batch_with(vec![Suite::insert_op(ObjectId::new())], doc.revision, key);
            let granted = Capability::unrestricted(7);
            if refuse_write_with_policy(&doc, &batch, &granted, &caps, &policy).is_some() {
                legit_refused = legit_refused.saturating_add(1);
            }
            let stale_write = refuse_write_with_policy(&doc, &batch, &granted, &caps, &stale)
                .expect("stale write refuses");
            assert_eq!(stale_write.code, "stale_policy");
            assert!(message_carries_recovery(&stale_write.message));
        }
        assert_eq!(legit_refused, 0, "legit calls must never refuse");
    }

    /// NS15: the catalog grew from 20 to 30. Every new brief is dated and
    /// reasoned, the only legitimate one is admitted untagged, and the
    /// refusal of an odd name shows it escaped and cut instead of in full.
    #[test]
    fn catalog_growth_is_dated_and_adds_no_false_refusal() {
        let report = run_adversarial_suite();
        assert_eq!(report.outcomes.len(), 20 + FIELD_BRIEFS.len());
        for (brief, outcome) in FIELD_BRIEFS.iter().zip(&report.outcomes[20..]) {
            assert_eq!(outcome.name, brief.name, "field brief order drifted");
            let day: Vec<&str> = brief.date.split('-').collect();
            assert!(
                matches!(day.as_slice(), [y, m, d] if y.len() == 4 && m.len() == 2 && d.len() == 2
                    && day.iter().all(|part| part.bytes().all(|b| b.is_ascii_digit()))),
                "`{}` date `{}` is not YYYY-MM-DD",
                brief.name,
                brief.date
            );
            assert!(brief.reason.len() > 20, "`{}` has no reason", brief.name);
            assert!(outcome.doc_unchanged);
        }
        let legit: Vec<&CaseOutcome> = report.outcomes[20..]
            .iter()
            .filter(|outcome| outcome.allowed)
            .collect();
        assert_eq!(legit.len(), 1, "one legitimate brief in the new ten");
        assert_eq!((legit[0].code, legit[0].axis), ("ok", None));
        // A 4096 char name is shown as its first 64 chars; a NUL byte is escaped.
        let long = report
            .outcomes
            .iter()
            .find(|outcome| outcome.name == "door_overlong_name")
            .expect("overlong brief runs");
        assert!(long.message.contains(&"a".repeat(MAX_SHOWN_NAME_CHARS)));
        assert!(!long.message.contains(&"a".repeat(MAX_SHOWN_NAME_CHARS + 1)));
        let control = report
            .outcomes
            .iter()
            .find(|outcome| outcome.name == "door_control_name")
            .expect("control brief runs");
        assert!(!control.message.contains('\0'), "{}", control.message);
        assert!(control.message.contains("\\0"), "{}", control.message);
    }

    #[test]
    fn remove_allowed_at_elevated_tier_with_delete_grant() {
        let doc = Document::new();
        let batch = Suite::batch_with(
            vec![Op::Remove {
                id: ObjectId::new(),
            }],
            doc.revision,
            40,
        );
        let verdict = check_write(
            &doc,
            &batch,
            &Suite::masked(7, OpMask::DELETE, 8),
            &ExplainCaps::default_caps(),
            DESTRUCTIVE_TIER,
        );
        assert!(
            verdict.allowed,
            "tier 3 plus a delete grant admits: {}",
            verdict.message
        );
        assert_eq!(verdict.code, "ok");
    }

    #[test]
    fn door_wrappers_agree_with_the_pure_gates() {
        // Name gate: classified tools admit, unknown names deny. Writers
        // face the batch gates below, never the bare name: the seam never
        // sees the batch, so refusing the name would refuse the door's own
        // preview, apply, place and undo calls.
        assert!(refuse_tool_call("scene.read").is_none());
        assert!(refuse_tool_call("tx.preview").is_none());
        assert!(refuse_tool_call("tx.apply").is_none());
        assert!(refuse_tool_call("scene.place").is_none());
        assert!(refuse_tool_call("tx.undo").is_none());
        let denied = refuse_tool_call("shell.exec").expect("unknown denies");
        assert_eq!(denied.code, "unknown_tool");
        assert!(message_carries_recovery(&denied.message));
        // Preview gate: shape refuses, scope and budget stay apply-time.
        let doc = Document::new();
        let caps = ExplainCaps::default_caps();
        let empty = Suite::batch_with(Vec::new(), doc.revision, 50);
        assert_eq!(
            refuse_preview(&doc, &empty, &caps)
                .expect("empty preview refuses")
                .code,
            "empty_batch"
        );
        let scoped = Suite::prefixed(7, 0);
        let far = Suite::batch_with(vec![Suite::insert_op(ObjectId::new())], doc.revision, 51);
        assert!(
            refuse_preview(&doc, &far, &caps).is_none(),
            "previews stay scope-free and budget-free at the door"
        );
        // Write gate at door tier: scope and budget refuse with the codes
        // the gated apply seals; removes still admit with a delete grant.
        let scope_error = refuse_write(&doc, &far, &scoped, &caps).expect("scope refuses");
        assert_eq!(scope_error.code, "out_of_scope");
        assert!(message_carries_recovery(&scope_error.message));
        let broke = Suite::masked(7, OpMask::ALL, 0);
        let tight = Suite::batch_with(vec![Suite::insert_op(ObjectId::new())], doc.revision, 52);
        assert_eq!(
            refuse_write(&doc, &tight, &broke, &caps)
                .expect("budget refuses")
                .code,
            "over_budget"
        );
        let remove = Suite::batch_with(
            vec![Op::Remove {
                id: ObjectId::new(),
            }],
            doc.revision,
            53,
        );
        assert!(
            refuse_write(&doc, &remove, &Suite::masked(7, OpMask::DELETE, 8), &caps).is_none(),
            "door tier plus a delete grant admits removes, as before"
        );
    }

    #[test]
    fn gate_p95_stays_negligible() {
        let doc = Document::new();
        let batch = Suite::batch_with(vec![Suite::insert_op(ObjectId::new())], doc.revision, 41);
        let cap = Capability::unrestricted(7);
        let caps = ExplainCaps::default_caps();
        let mut samples = Vec::with_capacity(1_000);
        for _ in 0..1_000 {
            let start = std::time::Instant::now();
            let verdict = check_write(&doc, &batch, &cap, &caps, 1);
            std::hint::black_box(verdict.code);
            samples.push(start.elapsed().as_nanos());
        }
        samples.sort_unstable();
        let p95 = samples[950];
        assert!(
            p95 <= GATE_P95_NANOS,
            "gate p95 {p95}ns exceeds {GATE_P95_NANOS}ns; the gate must stay a dozen integer compares"
        );
    }

    #[test]
    fn unscoped_call_refuses_typed_with_recovery() {
        // HS21 slice 1 pin: one fixture ties the bare-name gate to the
        // scope gate. Unknown names plus one out-of-scope batch refuse
        // typed with recovery; every advertised tool keeps its card with
        // zero legitimate false refusals. Headless, twice identical.
        let policy = ScopePolicy::default_file();
        let mut first: Vec<(&'static str, &'static str, bool)> = Vec::new();
        for _ in 0..2 {
            let mut codes: Vec<(&'static str, &'static str, bool)> = Vec::new();
            for tool in ["shell.exec", "", "exec"] {
                let verdict = check_tool_call(tool);
                assert!(!verdict.allowed, "unscoped `{tool}` must refuse");
                assert!(
                    verdict.is_capability_refusal(),
                    "unscoped `{tool}` must map through the capability family"
                );
                assert!(
                    message_carries_recovery(&verdict.message),
                    "unscoped `{tool}` must carry a recovery: {}",
                    verdict.message
                );
                let error = verdict
                    .as_tool_error()
                    .expect("refusal maps to a tool error");
                assert_eq!(error.code, verdict.code);
                assert_eq!(error.code, "unknown_tool");
                let denied = refuse_tool_call_with_policy(tool, &policy)
                    .expect("unscoped still denies under the shipped policy");
                assert_eq!(denied.code, "unknown_tool");
                assert!(message_carries_recovery(&denied.message));
                codes.push((tool, verdict.code, verdict.allowed));
            }
            let doc = Document::new();
            let caps = ExplainCaps::default_caps();
            let narrow = Suite::prefixed(7, 8);
            let far = Suite::batch_with(vec![Suite::insert_op(ObjectId::new())], doc.revision, 61);
            let scoped_out = check_write(&doc, &far, &narrow, &caps, 1);
            assert!(!scoped_out.allowed, "out-of-scope batch must refuse");
            assert_eq!(scoped_out.code, "out_of_scope");
            assert!(scoped_out.is_capability_refusal());
            assert!(message_carries_recovery(&scoped_out.message));
            let scoped_error = scoped_out
                .as_tool_error()
                .expect("scope refusal maps to a tool error");
            assert_eq!(scoped_error.code, "out_of_scope");
            let admitted = check_write(&doc, &far, &Capability::unrestricted(7), &caps, 1);
            assert!(admitted.allowed, "full scope admits: {}", admitted.message);
            assert_eq!(admitted.code, "ok");
            assert!(refuse_tool_call_with_policy("scene.read", &policy).is_none());
            assert!(refuse_tool_call_with_policy("tx.preview", &policy).is_none());
            assert!(refuse_tool_call_with_policy("tx.apply", &policy).is_none());
            assert!(refuse_tool_call_with_policy("scene.place", &policy).is_none());
            assert!(refuse_tool_call_with_policy("tx.undo", &policy).is_none());
            let mut false_refusals = 0_u32;
            for tool in super::super::mcp::tool_names() {
                assert_ne!(
                    card_for_tool(tool),
                    CapabilityCard::Deny,
                    "advertised tool `{tool}` must keep its card"
                );
                if refuse_tool_call_with_policy(tool, &policy).is_some() {
                    false_refusals = false_refusals.saturating_add(1);
                }
            }
            assert_eq!(false_refusals, 0, "legit calls must never refuse");
            if first.is_empty() {
                first = codes;
            } else {
                assert_eq!(first, codes, "replay without a model drifts");
            }
        }
    }
}
