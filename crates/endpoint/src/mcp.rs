//! MCP1: rmcp stdio server over the editor command service.
//!
//! Consumer: MCP2 play/capture tools, EA1/MCP3 follow-ups, one external MCP
//! client over the `e2040-mcp` stdio binary (MCP5). Data owner is the editor command service
//! (`engine-editor::commands` plus `engine-editor::capability`); this module
//! owns only transport DTOs, validation and rmcp wiring. Units follow
//! `engine-scene`: meters for positions. Determinism scope: none at this
//! boundary; the command service owns atomicity, monotonic revisions and
//! idempotency, and every mutating call below goes through it.
//!
//! Door v2: a small core plus verb packs. The router holds the thirteen core
//! tools ([`CORE_TOOL_NAMES`]): `scene.read` (bounded snapshot), `tx.preview`
//! (dry-run diff, no side effects), `tx.apply` (idempotency-keyed,
//! revision-checked, capability-gated), `tx.undo` (journal undo token),
//! `play.run` (headless arena match job with poll/cancel/expiry plus bounded
//! logs), `capture` (renderer receipt, structured unsupported unless a
//! capture callback is wired), `api.search` (full-text search over the
//! Insight index, which also lists every pack and verb), `api.describe` (one
//! full index entry), `api.call` (one pack verb by exact name) and
//! `api.load` (advertise or hide one pack for this session). Every other
//! verb lives in a pack (`super::packs`: scene, break, anim, material, asset,
//! rules, geo, view): `tools/list` shows the core plus the loaded packs
//! (`scene` by default), while every verb stays callable by exact name
//! whether or not its pack is listed. The former standalone tools
//! (`scene.brief`, `scene.spatial`, `scene.place`, `level.build`,
//! `scene.query`, `scene.watch`, `sim.plan_goals`, `play.step`, `log.digest`,
//! `log.query`, `code.callers`, `asset.refs`, `rpc.discover`) are pack verbs
//! now and answer exactly as before.
//! There is no arbitrary execute/shell/file-write tool by construction: the
//! router holds exactly the core [`SyncTool`]s, every pack verb registers
//! through the guarded `packs::VerbSpec::of` (annotation plus capability card
//! or no registration), and [`tool_names`] names the whole reachable set.
//!
//! Flow: `scene.read` for the revision, `tx.preview` for the diff plus a
//! single-use approval, `tx.apply` with that approval, `tx.undo` to revert.
//! Denied, stale, duplicate or spatially-rejected calls write NOTHING: the
//! document and both journal stacks are untouched (journal-position asserts in
//! tests). A poisoned state lock fails closed with `lock_poisoned` instead of
//! serving possibly inconsistent state.
//!
//! Approval binding (CAP1): each `tx.preview` mints an approval bound to this
//! session, the current revision, the canonical hash of the exact batch and an
//! expiry equal to the current revision, so any interleaved write voids
//! outstanding approvals. `tx.apply` replays through
//! [`apply_gated`](engine_editor::capability::apply_gated), which replays a
//! committed key, then stages plus spatial-checks on a private copy, then
//! single-use authorizes, then atomically applies. The session an approval
//! binds to is the engine's own [`ServerState::session`] id, fixed when the
//! host builds the server (the stdio door uses one constant per process); it
//! is never the MCP protocol session, which the 2026-07-28 revision removed,
//! so a client that reconnects without a protocol session changes nothing
//! and an approval minted on one engine session never authorizes another
//! (`approvals_bind_the_engine_session_not_the_protocol_session`).

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard};

use engine_editor::capability::{
    self, approve, canonical_action_hash, Approval, ApprovalLedger, CapError, Capability, OpMask,
};
use engine_editor::commands::{self, Batch, Document, Journal, MaterialName, Op, ProvenanceStamp};
use engine_scene::component::{
    Component, ComponentKind, SCHEMA_VERSION as COMPONENT_SCHEMA_VERSION,
};
use engine_scene::ids::ObjectId;
use engine_scene::schema::{SchemaRegistry, VersionedComponent};
use engine_scene::transform::Transform;

use rmcp::handler::server::router::tool::{SyncTool, ToolBase, ToolRouter};
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, InitializeResult, JsonObject,
    ListToolsResult, PaginatedRequestParams, ProtocolVersion, ResultType, ServerCapabilities,
    ServerConfig, ToolAnnotations, ToolsCapability,
};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};
use rmcp::serde_json::Value;
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};

use super::audit::DoorAudit;
use super::insight::mcp_tools::{
    ApiDescribeTool, ApiSearchTool, TOOL_API_DESCRIBE, TOOL_API_SEARCH,
};
use super::log_tools::{LogStore, LOG_RING_CAP};
use super::packs::{
    ApiCallTool, ApiLoadTool, PackRegistry, PACK_NAMES, TOOL_API_CALL, TOOL_API_LOAD,
};
use super::play::{
    frame_call_result, CaptureArgs, CaptureOutput, CaptureTool, PlayJobs, PlayRunArgs,
    PlayRunOutput, PlayRunTool, TOOL_CAPTURE, TOOL_PLAY_RUN,
};
use super::tool_macro::{TestRunTool, TOOL_TEST_RUN};
use super::witness::EvidenceReceipt;

/// Tool: bounded scene snapshot (read-only).
pub const TOOL_SCENE_READ: &str = "scene.read";
/// Tool: dry-run diff with no side effects (read-only).
pub const TOOL_TX_PREVIEW: &str = "tx.preview";
/// Tool: atomic capability-gated apply.
pub const TOOL_TX_APPLY: &str = "tx.apply";
/// Tool: journal undo as a new revision.
pub const TOOL_TX_UNDO: &str = "tx.undo";
/// Tool: brief text to validated encounter plan (read-only).
pub const TOOL_SIM_BUILD_ENCOUNTER: &str = super::build_encounter::MCP_TOOL_NAME;
/// Tool: brief text to packaged playable slice (read-only).
pub const TOOL_GAME_BUILD: &str = super::build_game::TOOL_BUILD_GAME;

/// Every protocol revision this SDK knows, oldest first: the door serves
/// legacy `initialize` handshakes (2024-11-05 through 2025-11-25) and the
/// 2026-07-28 inline lifecycle, so no exact match ever answers -32022.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[ProtocolVersion] = ProtocolVersion::KNOWN_VERSIONS;

/// Transport bound: at most this many ops per preview/apply call.
pub const MAX_TX_OPS: usize = 256;
/// Default `scene.read` page size.
pub const SCENE_READ_DEFAULT_LIMIT: u32 = 64;
/// Hard `scene.read` page cap.
pub const SCENE_READ_MAX_ROWS: u32 = 128;
/// Discovery budget: the full tool list (names, descriptions, input and
/// output schemas) must stay under this many chars. Raised 16384 → 20480
/// when the 9-tool MCP3 surface measured 16919, then 20480 → 24576 when the
/// 4-tool API1 Insight surface landed, then 24576 → 28672 when the MCP9 row
/// registered the two MCP7 self-description tools (`rpc.discover` carries a
/// manifest schema, `scene.watch` a subscription schema): the 18-tool list
/// measured 26557, so the budget held ~2k headroom. Then 28672 → 32768 when
/// the SP11 row registered `scene.query`: the 19-tool list measures 28486,
/// so the budget holds ~4k headroom. Then the AB5 row registered
/// `sim.plan_goals` with no raise: the 20-tool list measures 29544, so the
/// budget holds ~3k headroom. Then the ledger gate measured the live
/// 20-tool list at 32714 (schemas grew since the 29544 reading), leaving 54
/// chars of headroom under the 2048 minimum: 32768 → 36864 on 2026-09-27,
/// restoring ~4k headroom. Then on 2026-09-30 the live 20-tool list measured 35872 (992 headroom): 36864 → 38944, restoring ~3k headroom.
/// Then on 2026-09-30 the gate lands (+2012 chars tool_capabilities cards/docs):
/// the live 20-tool list measures 37884, so 38944 → 40944 restores ~3k
/// headroom (3060 chars over the 2048 minimum). Then on 2026-10-01 the
/// level builder registers (+2630 chars of brief/placement schemas): the
/// live 21-tool list measures 40514, so 40944 → 43514 restores ~3k headroom.
/// Then on 2026-10-04 door v2 lands: the advertised list is the thirteen core
/// tools plus the loaded packs (`scene` by default), measured well under
/// the budget with every former tool still reachable by name; the budget
/// stays at 43514 so the number is comparable to the old 21-tool door, and the
/// ledger gate now measures whatever a session advertises. The frozen
/// [`crate::tools::TOOLS`] registry now holds 26 tools.
/// Per-tool descriptions stay one line so discovery stays near 6k tokens.
/// OB3's two log tools, `scene.query` and `sim.plan_goals` keep field docs
/// off their DTOs to stay inside it.
pub const TOOL_LIST_BUDGET_CHARS: usize = 43_514;

/// The thirteen core tools, in contract order. The router holds exactly these;
/// every other verb is a pack verb.
pub const CORE_TOOL_NAMES: [&str; 13] = [
    TOOL_SCENE_READ,
    TOOL_TX_PREVIEW,
    TOOL_TX_APPLY,
    TOOL_TX_UNDO,
    TOOL_PLAY_RUN,
    TOOL_CAPTURE,
    TOOL_API_SEARCH,
    TOOL_API_DESCRIBE,
    TOOL_API_CALL,
    TOOL_API_LOAD,
    TOOL_TEST_RUN,
    TOOL_SIM_BUILD_ENCOUNTER,
    TOOL_GAME_BUILD,
];

/// Every reachable tool name: the core in contract order, then every pack
/// verb pack by pack. Callable by exact name whether or not advertised.
pub fn tool_names() -> Vec<&'static str> {
    #[cfg(test)]
    {
        use super::build_level::TOOL_BUILD_LEVEL;
        use super::discover::{TOOL_RPC_DISCOVER, TOOL_SCENE_WATCH};
        use super::insight::mcp_tools::{TOOL_ASSET_REFS, TOOL_CODE_CALLERS};
        use super::log_tools::{TOOL_LOG_DIGEST, TOOL_LOG_QUERY};
        use super::plan_goals::MCP_TOOL_NAME as TOOL_SIM_PLAN_GOALS;
        use super::play::play_session::TOOL_PLAY_STEP;
        use super::scene_query::TOOL_SCENE_QUERY;
        use super::scene_tools::{TOOL_SCENE_BRIEF, TOOL_SCENE_PLACE, TOOL_SCENE_SPATIAL};
        let _door_pin: [&'static str; 21] = [
            TOOL_SCENE_READ,
            TOOL_SCENE_BRIEF,
            TOOL_SCENE_SPATIAL,
            TOOL_TX_PREVIEW,
            TOOL_TX_APPLY,
            TOOL_TX_UNDO,
            TOOL_SCENE_PLACE,
            TOOL_BUILD_LEVEL,
            TOOL_SCENE_QUERY,
            TOOL_SIM_PLAN_GOALS,
            TOOL_PLAY_RUN,
            TOOL_PLAY_STEP,
            TOOL_CAPTURE,
            TOOL_LOG_DIGEST,
            TOOL_LOG_QUERY,
            TOOL_API_SEARCH,
            TOOL_API_DESCRIBE,
            TOOL_CODE_CALLERS,
            TOOL_ASSET_REFS,
            TOOL_RPC_DISCOVER,
            TOOL_SCENE_WATCH,
        ];
        debug_assert!(_door_pin
            .iter()
            .all(|pinned| CORE_TOOL_NAMES.contains(pinned)
                || PackRegistry::shipped().verb_names().contains(pinned)));
    }
    let mut names: Vec<&'static str> = CORE_TOOL_NAMES.to_vec();
    names.extend(PackRegistry::shipped().verb_names());
    names
}

/// Tool names a fresh session advertises: the [`CORE_TOOL_NAMES`] core plus
/// the default-loaded packs, in contract order (not the `tools::SLIM8` search default).
pub fn default_tool_names() -> Vec<&'static str> {
    let registry = PackRegistry::shipped();
    let loaded = registry.default_loaded();
    let mut names: Vec<&'static str> = CORE_TOOL_NAMES.to_vec();
    for pack in registry.packs() {
        if loaded.contains(pack.name()) {
            names.extend(pack.verb_names());
        }
    }
    names
}

/// Minimum discovery headroom in chars: the ledger gate refuses a list
/// with less room, forcing the author to shrink the surface or raise
/// [`TOOL_LIST_BUDGET_CHARS`] with a dated reason comment.
pub const TOOL_LIST_MIN_HEADROOM_CHARS: usize = 2_048;
/// Seeded bloat probe in chars: the ledger test seeds a near-ceiling total
/// plus this probe and requires the gate to refuse.
pub const TOOL_BUDGET_PROBE_BLOAT_CHARS: usize = 500;

/// One row of the per-tool discovery cost table: tool name plus its share
/// of the discovery budget in chars (name, description, input and output
/// schemas). Units: chars. Determinism: rows follow router order and the
/// name set always equals [`tool_names`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCostRow {
    /// Tool name, exactly one entry of [`tool_names`].
    pub name: String,
    /// Discovery cost in chars for this tool alone.
    pub chars: usize,
}

/// Tool-budget ledger: per-tool costs plus the gate verdict inputs.
/// Read-only snapshot; [`check_tool_budget`] owns the pass/fail rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolBudgetLedger {
    /// One row per advertised tool, in router order.
    pub rows: Vec<ToolCostRow>,
    /// Sum of `rows`; always equals `tool_list_budget_chars`.
    pub total_chars: usize,
    /// Copy of [`TOOL_LIST_BUDGET_CHARS`] at measure time.
    pub budget_chars: usize,
    /// `budget_chars` minus `total_chars` (saturating); the gate requires
    /// at least [`TOOL_LIST_MIN_HEADROOM_CHARS`].
    pub headroom_chars: usize,
}

/// Discovery chars for one advertised tool: name, description, input and
/// output schemas. Shared by the total and the per-tool table so the rows
/// always sum to the total.
fn tool_cost_row(tool: &rmcp::model::Tool) -> ToolCostRow {
    fn schema_chars(schema: &rmcp::model::JsonObject) -> usize {
        rmcp::serde_json::to_string(schema).map_or(0, |text| text.len())
    }
    ToolCostRow {
        name: tool.name.to_string(),
        chars: crate::discover::discovery_cost_chars(
            tool.name.len(),
            tool.description.as_deref().map_or(0, str::len),
            schema_chars(&tool.input_schema),
            tool.output_schema
                .as_ref()
                .map_or(0, |schema| schema_chars(schema)),
        ),
    }
}

/// Pure ledger gate over a measured total plus seeded extra chars. Returns
/// the remaining headroom in chars. Errors: `tool_list_bloated` when
/// `total + extra` exceeds [`TOOL_LIST_BUDGET_CHARS`],
/// `budget_headroom_exhausted` when the remainder drops under
/// [`TOOL_LIST_MIN_HEADROOM_CHARS`]; both messages name budget, total and
/// headroom plus the raise-with-reason rule. Deterministic over its inputs.
pub fn check_tool_budget(total_chars: usize, extra_chars: usize) -> Result<usize, McpToolError> {
    let total = total_chars.saturating_add(extra_chars);
    if total > TOOL_LIST_BUDGET_CHARS {
        return Err(McpToolError::new(
            "tool_list_bloated",
            format!(
                "tool list costs {total} chars with +{extra_chars} seeded bloat, \
                 budget is {TOOL_LIST_BUDGET_CHARS}; shrink the surface or raise \
                 the budget with a dated reason comment"
            ),
        ));
    }
    let headroom = TOOL_LIST_BUDGET_CHARS - total;
    if headroom < TOOL_LIST_MIN_HEADROOM_CHARS {
        return Err(McpToolError::new(
            "budget_headroom_exhausted",
            format!(
                "tool list costs {total} chars, headroom is {headroom} chars under the \
                 {TOOL_LIST_MIN_HEADROOM_CHARS}-char minimum; shrink the surface or raise \
                 {TOOL_LIST_BUDGET_CHARS} with a dated reason comment"
            ),
        ));
    }
    Ok(headroom)
}

/// Negotiate the protocol version per the MCP spec: an exact match echoes
/// the requested version, any mismatch answers the newest legacy version
/// the door supports (2025-11-25) instead of -32022. The rmcp transport
/// re-runs the same rule on the wire, so a direct call and the live
/// handshake agree and no exact match ever fails.
pub fn negotiate_version(requested: &ProtocolVersion) -> Result<ProtocolVersion, ErrorData> {
    if SUPPORTED_PROTOCOL_VERSIONS.contains(requested) {
        Ok(requested.clone())
    } else {
        Ok(ProtocolVersion::V_2025_11_25)
    }
}

// Transport DTOs live in `mcp_dto` (structure-only split, behaviour
// identical): the shapes mirror `engine-editor::commands` one to one; the
// drift test asserts the generated schemas still match the command shapes.
#[path = "mcp_dto.rs"]
mod mcp_dto;

pub use self::mcp_dto::{
    ApprovalDto, OpDto, PoseDto, SceneObjectDto, SceneReadArgs, SceneReadOutput, TxApplyArgs,
    TxApplyOutput, TxPreviewArgs, TxPreviewOutput, TxUndoArgs, TxUndoOutput,
};

// ---------------------------------------------------------------------------
// Errors: machine code plus a message that always names the legal values.
// ---------------------------------------------------------------------------

/// Tool failure: a machine `code`, a message naming the allowed values and
/// optional `data` for the offending payload. This is the canonical door
/// error shape `{code, message, data}`, shared with
/// [`super::discover::RpcError`]: codes are always `&'static str` constants
/// and every conversion preserves `data`, so a refusal crosses the wire
/// with its culprit attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpToolError {
    /// Machine code (`stale_revision`, `forged_approval`, ...).
    pub code: &'static str,
    /// Human detail with the legal values embedded.
    pub message: String,
    /// Offending payload (item index, method name, ...), when it fits.
    pub data: Option<rmcp::serde_json::Value>,
}

impl McpToolError {
    /// New tool error with a static code and no data.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
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

    /// Re-code a wire refusal under a new static code, keeping its message
    /// and data: layering a code never silently drops the culprit.
    pub fn wrap(code: &'static str, error: super::discover::RpcError) -> Self {
        Self {
            code,
            message: error.message,
            data: error.data,
        }
    }
}

impl std::fmt::Display for McpToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for McpToolError {}

impl From<McpToolError> for ErrorData {
    fn from(error: McpToolError) -> Self {
        Self::from(super::discover::RpcError::from(error))
    }
}

/// Parse UUID text for one op field.
fn parse_id(index: usize, field: &str, text: &str) -> Result<ObjectId, McpToolError> {
    ObjectId::parse(text).map_err(|_| {
        McpToolError::new(
            "bad_id",
            format!(
                "ops[{index}].{field} `{text}` is not an object id (expected UUID text; \
                 the nil id `{nil}` means 'no parent' on parent fields)",
                nil = ObjectId::NIL,
            ),
        )
    })
}

/// Check one pose for finite lanes.
fn pose_from_dto(index: usize, local: &PoseDto) -> Result<Transform, McpToolError> {
    let finite = local
        .pos
        .iter()
        .chain(local.rot.iter())
        .chain(local.scale.iter())
        .all(|lane| lane.is_finite());
    if !finite {
        return Err(McpToolError::new(
            "bad_pose",
            format!(
                "ops[{index}].local holds a non-finite lane; \
                 poses must be finite (meters position, quaternion, scale)"
            ),
        ));
    }
    Ok(Transform::new(local.pos, local.rot, local.scale))
}

/// Convert one wire op into a command op.
fn op_from_dto(index: usize, dto: &OpDto) -> Result<Op, McpToolError> {
    match dto {
        OpDto::Insert { id, parent, local } => {
            let id = parse_id(index, "id", id)?;
            if id == ObjectId::NIL {
                return Err(McpToolError::new(
                    "bad_id",
                    format!("ops[{index}].id must not be nil; insert needs a fresh id"),
                ));
            }
            Ok(Op::Insert {
                id,
                parent: parse_id(index, "parent", parent)?,
                local: pose_from_dto(index, local)?,
            })
        }
        OpDto::Remove { id } => Ok(Op::Remove {
            id: parse_id(index, "id", id)?,
        }),
        OpDto::SetLocal { id, local } => Ok(Op::SetLocal {
            id: parse_id(index, "id", id)?,
            local: pose_from_dto(index, local)?,
        }),
        OpDto::Reparent { id, new_parent } => Ok(Op::Reparent {
            id: parse_id(index, "id", id)?,
            new_parent: parse_id(index, "new_parent", new_parent)?,
        }),
        OpDto::SetMaterial { id, material } => {
            let id = parse_id(index, "id", id)?;
            let material = match material {
                None => None,
                Some(text) => Some(
                    MaterialName::try_from(text.as_str()).map_err(|error| {
                        McpToolError::new(
                            "bad_material",
                            format!(
                                "ops[{index}].material `{text}` is not a material name ({error}; \
                                 expected 1..=64 bytes of 0-9 A-Z a-z - _ .; null clears the binding)"
                            ),
                        )
                    })?,
                ),
            };
            Ok(Op::SetMaterial { id, material })
        }
        OpDto::SetComponent {
            id,
            component,
            value,
            version,
        } => {
            let id = parse_id(index, "id", id)?;
            let kind = component_kind(index, component)?;
            let versioned = VersionedComponent {
                component: kind.name().to_owned(),
                version: version.unwrap_or(COMPONENT_SCHEMA_VERSION),
                payload: value.to_string(),
            };
            let component = Component::from_versioned(&SchemaRegistry::builtin(), &versioned)
                .map_err(|error| {
                    McpToolError::new(
                        "bad_component",
                        format!(
                            "ops[{index}].value for `{}` is refused: {error}",
                            kind.name()
                        ),
                    )
                })?;
            Ok(Op::SetComponent { id, component })
        }
        OpDto::RemoveComponent { id, component } => Ok(Op::RemoveComponent {
            id: parse_id(index, "id", id)?,
            kind: component_kind(index, component)?,
        }),
    }
}

/// Component kind for a wire name, refusing with the legal names.
fn component_kind(index: usize, name: &str) -> Result<ComponentKind, McpToolError> {
    ComponentKind::from_name(name).ok_or_else(|| {
        McpToolError::new(
            "bad_component",
            format!(
                "ops[{index}].component `{name}` is not a component schema; the schemas are \
                 [{}]; re-run with one of those",
                ComponentKind::name_list()
            ),
        )
    })
}

/// Build the command batch both preview and apply construct identically, so
/// their canonical hashes match. MCP batches always stamp `mcp` script
/// source (the server holds no provider credentials to stamp more).
/// Crate-visible: the `scene.spatial` staging path builds the same batch.
pub(crate) fn build_batch(
    expected_revision: u64,
    idempotency_key: u64,
    ops: &[OpDto],
) -> Result<Batch, McpToolError> {
    if ops.is_empty() {
        return Err(McpToolError::new(
            "empty_batch",
            format!("ops holds 0 entries; submit 1..={MAX_TX_OPS} ops"),
        ));
    }
    if ops.len() > MAX_TX_OPS {
        return Err(McpToolError::new(
            "too_many_ops",
            format!(
                "ops holds {} entries; at most {MAX_TX_OPS} per transaction",
                ops.len()
            ),
        ));
    }
    let mut converted = Vec::with_capacity(ops.len());
    for (index, dto) in ops.iter().enumerate() {
        converted.push(op_from_dto(index, dto)?);
    }
    Ok(Batch {
        ops: converted,
        base_revision: expected_revision,
        idempotency_key,
        provenance: Some(ProvenanceStamp::script("mcp")),
    })
}

/// Map a command refusal to a tool error with the legal values attached.
/// Crate-visible: the scene tools map staging failures the same way.
pub(crate) fn batch_error(error: &commands::BatchError) -> McpToolError {
    match *error {
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
/// snapshotted before the gated apply, so denials name what is allowed.
/// Crate-visible: `scene.place` applies through the same gate.
pub(crate) fn cap_error(error: &CapError, granted: &[&str]) -> McpToolError {
    match *error {
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
        CapError::Spatial { check } => McpToolError::new(
            "spatial_rejected",
            format!(
                "staged spatial check `{check}` failed: the proposal adds {check}; \
                 document unchanged"
            ),
        ),
        CapError::Command(inner) => batch_error(&inner),
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
// Server state: thin sync core over the command service. The rmcp tools call
// these; unit tests call them too, with no executor needed.
// ---------------------------------------------------------------------------

/// Mutable MCP session: document, journal, capability token, ledger, the
/// play-job table, the observe log, the Insight index and the
/// tamper-evident door log (one record per dispatched call, chained by
/// [`McpServer::dispatch`](crate::mcp::McpServer)).
/// Fields are crate-visible so the scene, log and Insight tools run
/// against the same session.
pub struct ServerState {
    pub(crate) doc: Document,
    pub(crate) journal: Journal,
    pub(crate) session: u64,
    pub(crate) cap: Capability,
    pub(crate) ledger: ApprovalLedger,
    pub(crate) jobs: PlayJobs,
    /// The session's play slot: the live bout held across `play.step`,
    /// `sim.step` and `sim.observe` (a longer prefix only simulates its new
    /// ticks), plus the capture hook and re-query cache of this session.
    pub(crate) play: super::play::play_session::PlaySlot,
    pub(crate) logs: LogStore,
    pub(crate) insight: super::insight::Index,
    pub(crate) audit: DoorAudit,
    /// Evidence receipt of the newest state-changing `tx.apply` on this
    /// session, if any: the witnessed-plan gate seals one per landed write.
    /// Ledger replays move no revision and keep the retained receipt.
    pub(crate) last_witness_receipt: Option<EvidenceReceipt>,
    /// Server-owned watch slot: the live subscription id, if any.
    pub(crate) watch_subscription: Option<u64>,
    /// Session-local latest digest cache, updated on watch and every scene mutation.
    pub(crate) watch_digest: Option<u64>,
    /// Verb packs this session advertises in `tools/list` (`api.load`
    /// moves them); every verb stays callable by name regardless.
    pub(crate) loaded_packs: BTreeSet<&'static str>,
    /// The session's thread-confined rules worker (feature `rules`, off by default).
    #[cfg(feature = "rules")]
    pub(crate) rules: super::packs::rules::RulesSlot,
}

impl ServerState {
    /// Fresh session over an empty document.
    pub fn new(session: u64, cap: Capability) -> Self {
        Self::with_document(Document::new(), Journal::new(), session, cap)
    }

    /// Session over a host-provided document and journal. The Insight index
    /// opens seeded with every pack and verb, so `api.search` finds the door's
    /// own surface before any source file is indexed.
    pub fn with_document(doc: Document, journal: Journal, session: u64, cap: Capability) -> Self {
        let registry = PackRegistry::shipped();
        let mut insight = super::insight::Index::new();
        registry.seed_index(&mut insight);
        Self {
            doc,
            journal,
            session,
            cap,
            ledger: ApprovalLedger::new(),
            jobs: PlayJobs::new(),
            play: super::play::play_session::PlaySlot::default(),
            logs: LogStore::new(LOG_RING_CAP),
            insight,
            audit: DoorAudit::new(),
            last_witness_receipt: None,
            watch_subscription: None,
            watch_digest: None,
            loaded_packs: registry.default_loaded(),
            #[cfg(feature = "rules")]
            rules: super::packs::rules::RulesSlot::default(),
        }
    }

    /// The engine session id approvals bind to (never the protocol's).
    pub fn session(&self) -> u64 {
        self.session
    }

    /// Evidence receipt of the newest state-changing `tx.apply`, if any.
    pub fn witness_receipt(&self) -> Option<&EvidenceReceipt> {
        self.last_witness_receipt.as_ref()
    }

    /// Install a rehydrated document plus journal after a restart (no IO).
    ///
    /// The host reads the sealed pair with `engine_editor::durable`
    /// (`reopen_sealed` plus `rehydrate`), then hands the pair here; single-use
    /// approvals never survive a restart, so the ledger reopens empty while
    /// the session watch cache clears (the client re-subscribes).
    pub fn rehydrate(&mut self, doc: Document, journal: Journal) {
        self.doc = doc;
        self.journal = journal;
        self.ledger = ApprovalLedger::new();
        self.last_witness_receipt = None;
        self.watch_subscription = None;
        self.watch_digest = None;
    }

    /// Bounded snapshot: revision plus one page of stable-id rows. Read-only.
    pub fn scene_read(&self, args: &SceneReadArgs) -> Result<SceneReadOutput, McpToolError> {
        let limit = args.limit.unwrap_or(SCENE_READ_DEFAULT_LIMIT);
        if limit == 0 || limit > SCENE_READ_MAX_ROWS {
            return Err(McpToolError::new(
                "bad_limit",
                format!(
                    "limit {limit} is outside 1..={SCENE_READ_MAX_ROWS} \
                     (default {SCENE_READ_DEFAULT_LIMIT})"
                ),
            ));
        }
        // Every node hangs under a root, so a roots-down walk covers all.
        let mut ids = Vec::with_capacity(self.doc.hierarchy.len());
        let mut stack = self.doc.hierarchy.roots();
        while let Some(id) = stack.pop() {
            ids.push(id);
            stack.extend(self.doc.hierarchy.children(id));
        }
        ids.sort_by_key(|id| id.to_string());
        let total = ids.len() as u64;
        let cursor = u64::from(args.cursor.unwrap_or(0));
        if cursor > total {
            return Err(McpToolError::new(
                "bad_cursor",
                format!("cursor {cursor} is beyond {total} objects"),
            ));
        }
        let start = cursor as usize;
        let end = start.saturating_add(limit as usize).min(ids.len());
        let mut objects = Vec::with_capacity(end - start);
        for id in &ids[start..end] {
            let Some(local) = self.doc.hierarchy.local(*id) else {
                return Err(McpToolError::new(
                    "snapshot_failed",
                    format!("node {id} vanished mid-read; retry scene.read"),
                ));
            };
            let finite = local
                .position
                .iter()
                .chain(local.rotation.iter())
                .chain(local.scale.iter())
                .all(|lane| lane.is_finite());
            if !finite {
                return Err(McpToolError::new(
                    "snapshot_failed",
                    format!("node {id} holds a non-finite pose; refusing to serialize it"),
                ));
            }
            let parent = match self.doc.hierarchy.parent(*id) {
                None | Some(ObjectId::NIL) => None,
                Some(parent) => Some(parent.to_string()),
            };
            objects.push(SceneObjectDto {
                id: id.to_string(),
                parent,
                local: PoseDto {
                    pos: local.position,
                    rot: local.rotation,
                    scale: local.scale,
                },
            });
        }
        Ok(SceneReadOutput {
            revision: self.doc.revision,
            total,
            cursor,
            returned: objects.len() as u64,
            truncated: end < ids.len(),
            objects,
        })
    }

    /// Dry run: validate and diff with no side effects, minting the
    /// single-use approval `tx.apply` needs. The Propose-card gate fronts
    /// the path (shape plus the EXPLAIN walk, mapped exactly as below), so
    /// every preview runs under the capability taxonomy while staying
    /// scope-free and budget-free by door contract.
    pub fn tx_preview(&self, args: &TxPreviewArgs) -> Result<TxPreviewOutput, McpToolError> {
        let batch = build_batch(args.expected_revision, args.idempotency_key, &args.ops)?;
        let caps = super::explain::ExplainCaps::default_caps();
        if let Some(refusal) = super::tool_capabilities::refuse_preview(&self.doc, &batch, &caps) {
            return Err(refusal);
        }
        let preview = commands::validate(&self.doc, &batch).map_err(|error| batch_error(&error))?;
        let action_hash = canonical_action_hash(&batch);
        // Tightest binding: any interleaved write voids this approval.
        let approval = approve(self.session, &batch, self.doc.revision, self.doc.revision);
        debug_assert_eq!(approval.action_hash, action_hash);
        Ok(TxPreviewOutput {
            effects: preview.effects,
            resulting_revision: preview.resulting_revision,
            action_hash,
            approval: ApprovalDto {
                revision: approval.revision,
                action_hash: approval.action_hash,
                expiry_tick: approval.expiry_tick,
            },
        })
    }

    /// Atomic capability-gated apply. Denied, stale and duplicate calls
    /// return an error with the document and journal untouched. The
    /// Commit-card gate fronts the path ahead of the approval ledger, so a
    /// capability refusal (mask, scope, budget) wins over a stale or forged
    /// approval with the same code the gated apply already seals.
    pub fn tx_apply(&mut self, args: &TxApplyArgs) -> Result<TxApplyOutput, McpToolError> {
        let batch = build_batch(args.expected_revision, args.idempotency_key, &args.ops)?;
        let caps = super::explain::ExplainCaps::default_caps();
        if let Some(refusal) =
            super::tool_capabilities::refuse_write(&self.doc, &batch, &self.cap, &caps)
        {
            return Err(refusal);
        }
        super::explain::check_batch(&self.doc, &batch, &caps)
            .map_err(|refusal| refusal.to_tool_error(&caps))?;
        // Witnessed-plan gate: bind this batch to the observed digest plus
        // revision before the gated apply, then seal the receipt after it
        // lands (same lock, no interleaved write, so the pre-state gate is
        // fresh by construction; ledger replays move no revision and keep
        // the retained receipt).
        let pre_revision = self.doc.revision;
        let proposal = super::door_dispatch::witness_proposal_for_apply(self, &batch)?;
        let approval = Approval {
            session: self.session,
            revision: args.approval.revision,
            action_hash: args.approval.action_hash,
            expiry_tick: args.approval.expiry_tick,
        };
        let granted = granted_ops(&self.cap);
        let applied = capability::apply_gated(
            &mut self.doc,
            &mut self.journal,
            &batch,
            &mut self.cap,
            &approval,
            &mut self.ledger,
        )
        .map_err(|error| cap_error(&error, &granted))?;
        // A landed write is a watch event: the session-local digest cache
        // moves, and the hub fans the digest out to live slots.
        self.refresh_watch();
        super::door_dispatch::witness_seal_apply(self, pre_revision, &proposal)?;
        Ok(TxApplyOutput {
            revision: applied.revision,
            effects: applied.effects,
            key: applied.key,
            journal_index: applied.journal_index as u64,
            undo_token: applied.key,
        })
    }

    /// Undo the newest journaled batch as a new revision. A given token must
    /// match the newest batch key, so a client cannot undo another writer's
    /// edit by accident.
    pub fn tx_undo(&mut self, args: &TxUndoArgs) -> Result<TxUndoOutput, McpToolError> {
        let Some(newest) = self.journal.past.last() else {
            return Err(McpToolError::new(
                "nothing_to_undo",
                "the journal holds no applied batches",
            ));
        };
        if let Some(token) = args.undo_token {
            if newest.key != token {
                return Err(McpToolError::new(
                    "undo_mismatch",
                    format!(
                        "newest undoable batch is key {}, not token {token}; \
                         omit undo_token to undo the newest batch",
                        newest.key,
                    ),
                ));
            }
        }
        let applied = commands::undo(&mut self.doc, &mut self.journal).ok_or_else(|| {
            McpToolError::new("nothing_to_undo", "the journal holds no applied batches")
        })?;
        // Undo publishes a new revision, so the watch cache moves with it.
        self.refresh_watch();
        Ok(TxUndoOutput {
            revision: applied.revision,
            undone_key: applied.key,
            effects: applied.effects,
        })
    }

    /// MCP2 headless arena match job: `start` simulates a bounded match and
    /// files a handle, `poll` walks progress to the checkpoints, `cancel`
    /// stops a running handle. The scene document is untouched. A start with
    /// `scene_rev` plays the current scene as the match world.
    pub fn play_run(&mut self, args: &PlayRunArgs) -> Result<PlayRunOutput, McpToolError> {
        let world = match args.scene_rev {
            Some(rev) if args.action == "start" => {
                Some(super::play::world_load::world_for_revision(&self.doc, rev)?)
            }
            _ => None,
        };
        #[cfg(feature = "rules")]
        if args.action == "start" {
            if let Some(scripted) = self.rules.scripted() {
                let run =
                    |config: &engine_arena::ArenaConfig,
                     tape: &engine_arena::controller::SlotTape,
                     world: Option<&engine_arena::world::MatchWorld>| {
                        scripted.run(config, tape, world)
                    };
                let runner = super::play::MatchRunner {
                    note: scripted.note(),
                    run: &run,
                };
                return self.jobs.play_run_with(args, world, Some(&runner));
            }
        }
        self.jobs.play_run_in(args, world)
    }

    /// MCP2 renderer receipt. Unwired by construction: structured
    /// `unsupported_capture`, never a success shape. Hosts with a renderer
    /// call [`ServerState::capture_with`] with their one-shot callback.
    pub fn capture(&self, args: &CaptureArgs) -> Result<CaptureOutput, McpToolError> {
        super::play::capture_unsupported(args)
    }

    /// Wired capture: run the caller's one-shot renderer callback and
    /// validate its render-free receipt. The endpoint never imports render;
    /// the callback owns the pixels.
    pub fn capture_with(
        &self,
        args: &CaptureArgs,
        render: super::play::CaptureRenderer<'_>,
    ) -> Result<CaptureOutput, McpToolError> {
        super::play::capture_with(args, render)
    }
}

// ---------------------------------------------------------------------------
// rmcp tools: one SyncTool per contract tool, all sync so tests need no
// executor. Schemas generate from the DTOs above via schemars.
// ---------------------------------------------------------------------------

/// `scene.read` tool: bounded snapshot, read-only.
pub struct SceneReadTool;

impl ToolBase for SceneReadTool {
    type Parameter = SceneReadArgs;
    type Output = SceneReadOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_SCENE_READ.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Bounded scene snapshot: document revision plus stable-id rows \
             (id, parent, local pose). Paged by cursor/limit; read-only, no side effects."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for SceneReadTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch(TOOL_SCENE_READ, |state| state.scene_read(&param))
    }
}

/// `tx.preview` tool: dry-run diff, read-only.
pub struct TxPreviewTool;

impl ToolBase for TxPreviewTool {
    type Parameter = TxPreviewArgs;
    type Output = TxPreviewOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_TX_PREVIEW.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Dry-run a command batch: validate and diff with no side effects. \
             Returns effects, the resulting revision and a single-use approval for tx.apply."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for TxPreviewTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch(TOOL_TX_PREVIEW, |state| state.tx_preview(&param))
    }
}

/// `tx.apply` tool: atomic capability-gated apply.
pub struct TxApplyTool;

impl ToolBase for TxApplyTool {
    type Parameter = TxApplyArgs;
    type Output = TxApplyOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_TX_APPLY.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Atomically apply a previewed batch through the capability gate. \
             Requires the tx.preview approval; idempotency-keyed and revision-checked. \
             Denied, stale or duplicate calls write nothing."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        let mut annotations = ToolAnnotations::default();
        annotations.read_only_hint = Some(false);
        annotations.idempotent_hint = Some(true);
        annotations.open_world_hint = Some(false);
        Some(annotations)
    }
}

impl SyncTool<McpServer> for TxApplyTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch(TOOL_TX_APPLY, |state| state.tx_apply(&param))
    }
}

/// `tx.undo` tool: revert the newest batch as a new revision.
pub struct TxUndoTool;

impl ToolBase for TxUndoTool {
    type Parameter = TxUndoArgs;
    type Output = TxUndoOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_TX_UNDO.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Undo the newest journaled batch as a new revision. \
             A given undo_token must match the newest batch key."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        let mut annotations = ToolAnnotations::default();
        annotations.read_only_hint = Some(false);
        annotations.open_world_hint = Some(false);
        Some(annotations)
    }
}

impl SyncTool<McpServer> for TxUndoTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch(TOOL_TX_UNDO, |state| state.tx_undo(&param))
    }
}

/// `sim.build_encounter` args: brief text plus seed.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SimBuildEncounterArgs {
    pub brief: String,
    pub seed: u64,
}

/// `sim.build_encounter` result: the landed plan or the refusal set.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SimBuildEncounterOutput {
    pub solved: bool,
    pub plan_id: u64,
    pub rung: u8,
    pub fork_id: u64,
    pub repairs_used: u8,
    pub reason: String,
    pub conflicts: Vec<String>,
}

/// `sim.build_encounter` handler: brief text to validated encounter plan
/// under the deterministic oracle. Pure and read-only: a refusal rides the
/// output (`solved: false`), only a malformed brief errors.
fn sim_build_encounter(
    service: &McpServer,
    args: &SimBuildEncounterArgs,
) -> Result<SimBuildEncounterOutput, McpToolError> {
    service.dispatch(TOOL_SIM_BUILD_ENCOUNTER, |_| {
        let brief = super::build_encounter::EncounterBrief::new(&args.brief, args.seed).map_err(
            |error| {
                McpToolError::new(
                    "bad_brief",
                    format!(
                        "brief is refused ({}); submit 1..={} chars of brief text",
                        error,
                        super::build_encounter::BRIEF_MAX_CHARS,
                    ),
                )
            },
        )?;
        let oracle = super::build_encounter::DeterministicOracle;
        match super::build_encounter::build_encounter(&brief, &oracle) {
            Ok(built) => Ok(SimBuildEncounterOutput {
                solved: true,
                plan_id: built.plan_id,
                rung: built.rung,
                fork_id: built.fork_id,
                repairs_used: u8::try_from(built.repairs_used.len()).unwrap_or(u8::MAX),
                reason: String::new(),
                conflicts: Vec::new(),
            }),
            Err(refusal) => Ok(SimBuildEncounterOutput {
                solved: false,
                plan_id: 0,
                rung: 0,
                fork_id: 0,
                repairs_used: u8::try_from(refusal.repairs_used.len()).unwrap_or(u8::MAX),
                reason: refusal.to_string(),
                conflicts: refusal.conflicts,
            }),
        }
    })
}

crate::e2_tool!(
    /// `sim.build_encounter` tool: brief text to validated encounter plan. Read-only.
    SimBuildEncounterTool,
    name = TOOL_SIM_BUILD_ENCOUNTER,
    description = super::build_encounter::MCP_TOOL_SUMMARY,
    readonly = true,
    args = SimBuildEncounterArgs,
    output = SimBuildEncounterOutput,
    invoke = sim_build_encounter
);

/// `game.build` args: brief text, seed, kit variant and bloodied flag.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GameBuildArgs {
    pub brief: String,
    pub seed: u64,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub bloodied: Option<bool>,
}

/// `game.build` result: the packaged slice proof or the refusal reason.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GameBuildOutput {
    pub solved: bool,
    pub play_digest: u64,
    pub replay_digest: u64,
    pub ticks_played: u32,
    pub package_bytes: u32,
    pub verdict: String,
    pub reason: String,
}

/// `game.build` handler: brief text to packaged playable slice (level,
/// character, encounter, five-minute play plus exact replay). Pure and
/// read-only: a refusal rides the output (`solved: false`), only a malformed
/// brief or unknown variant errors.
fn game_build(service: &McpServer, args: &GameBuildArgs) -> Result<GameBuildOutput, McpToolError> {
    service.dispatch(TOOL_GAME_BUILD, |_| {
        let variant_name = args.variant.as_deref().unwrap_or("hero");
        let variant = super::build_character::VariantId::parse(variant_name).map_err(|error| {
            McpToolError::new(
                "bad_variant",
                format!(
                    "variant `{variant_name}` is refused ({error}); re-run with one of \
                     [hero, opp_male_light, opp_female_medium, opp_female_light]",
                ),
            )
        })?;
        let slice = super::build_game::build_slice(
            &args.brief,
            args.seed,
            variant,
            args.bloodied.unwrap_or(false),
        )
        .map_err(|error| {
            McpToolError::new(
                error.code,
                format!("slice refused ({error}); fix the brief and re-run"),
            )
        })?;
        Ok(GameBuildOutput {
            solved: true,
            play_digest: slice.play_digest,
            replay_digest: slice.eval.replay_digest,
            ticks_played: slice.eval.ticks_played,
            package_bytes: slice.eval.package_bytes,
            verdict: slice.eval.verdict.clone(),
            reason: String::new(),
        })
    })
}

crate::e2_tool!(
    /// `game.build` tool: brief text to packaged playable slice. Read-only.
    GameBuildTool,
    name = TOOL_GAME_BUILD,
    description = super::build_game::TOOL_SUMMARY,
    readonly = true,
    args = GameBuildArgs,
    output = GameBuildOutput,
    invoke = game_build
);

// ---------------------------------------------------------------------------
// Server: negotiated protocol version, core router plus packs, stdio runner.
// ---------------------------------------------------------------------------

/// schemars emits `u64` fields as `{"type": "integer", "format": "uint64"}`;
/// some clients log `unknown format uint64 ignored` per tool. Rewrite that
/// emit to plain `integer` so serve.log stays quiet.
fn sanitize_schema_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            // AX15 surface trim (2026-10-17): every advertised schema repeated
            // the same 57-char `$schema` dialect key, 2622 chars on the loaded
            // list. MCP clients read tool schemas in the protocol's own
            // dialect, so the key carried no information a caller needs; the
            // door drops it and spends the room on verbs instead of
            // boilerplate. No field, verb or budget changes.
            map.remove("$schema");
            for (key, entry) in map.iter_mut() {
                if (key == "format" || key == "type")
                    && entry.as_str().is_some_and(|format| format == "uint64")
                {
                    *entry = Value::String("integer".to_owned());
                } else {
                    sanitize_schema_value(entry);
                }
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                sanitize_schema_value(item);
            }
        }
        _ => {}
    }
}

/// Trim one advertised tool's schemas to what a caller needs: the `uint64`
/// emit becomes plain `integer`, and the repeated `$schema` dialect key is
/// dropped (see [`sanitize_schema_value`]). Both shrink the ledger the
/// discovery budget counts, without changing one field.
fn sanitize_tool_schema(tool: &mut rmcp::model::Tool) {
    let mut input = Value::Object((*tool.input_schema).clone());
    sanitize_schema_value(&mut input);
    if let Value::Object(map) = input {
        tool.input_schema = Arc::new(map);
    }
    if let Some(output) = tool.output_schema.as_mut() {
        let mut replacement = Value::Object((**output).clone());
        sanitize_schema_value(&mut replacement);
        if let Value::Object(map) = replacement {
            *output = Arc::new(map);
        }
    }
}

/// MCP stdio server over one document plus its capability session.
pub struct McpServer {
    state: Mutex<ServerState>,
    tool_router: ToolRouter<Self>,
}

impl McpServer {
    /// Session over an empty document with the given capability token.
    pub fn new(session: u64, cap: Capability) -> Self {
        Self::with_state(ServerState::new(session, cap))
    }

    /// Session over a host-provided document and journal.
    pub fn with_document(doc: Document, journal: Journal, session: u64, cap: Capability) -> Self {
        Self::with_state(ServerState::with_document(doc, journal, session, cap))
    }

    /// Session over a fully built state.
    pub fn with_state(state: ServerState) -> Self {
        Self {
            state: Mutex::new(state),
            tool_router: Self::router(),
        }
    }

    /// The core router. Exactly the [`CORE_TOOL_NAMES`] tools: no execute,
    /// shell or file-write tool exists on this server; pack verbs dispatch
    /// through `super::packs` by exact name.
    fn router() -> ToolRouter<Self> {
        ToolRouter::new()
            .with_sync_tool::<SceneReadTool>()
            .with_sync_tool::<TxPreviewTool>()
            .with_sync_tool::<TxApplyTool>()
            .with_sync_tool::<TxUndoTool>()
            .with_sync_tool::<PlayRunTool>()
            .with_sync_tool::<CaptureTool>()
            .with_sync_tool::<ApiSearchTool>()
            .with_sync_tool::<ApiDescribeTool>()
            .with_sync_tool::<ApiCallTool>()
            .with_sync_tool::<ApiLoadTool>()
            .with_sync_tool::<TestRunTool>()
            .with_sync_tool::<SimBuildEncounterTool>()
            .with_sync_tool::<GameBuildTool>()
    }

    /// Advertised tools: the core plus the packs this session loaded, name
    /// sorted. A poisoned session lock advertises the core alone (fail
    /// closed); every verb stays callable by name either way.
    pub fn tools(&self) -> Vec<rmcp::model::Tool> {
        let loaded = self
            .lock()
            .map(|state| state.loaded_packs.clone())
            .unwrap_or_default();
        self.tools_with(&loaded)
    }

    /// Every reachable tool: the core plus every pack verb, name sorted.
    pub fn all_tools(&self) -> Vec<rmcp::model::Tool> {
        let all: BTreeSet<&'static str> = PACK_NAMES.iter().copied().collect();
        self.tools_with(&all)
    }

    /// The core tools alone, name sorted.
    pub fn core_tools(&self) -> Vec<rmcp::model::Tool> {
        let mut tools = self.tool_router.list_all();
        for tool in tools.iter_mut() {
            sanitize_tool_schema(tool);
        }
        tools
    }

    /// Core tools plus the verbs of `loaded` packs, name sorted.
    fn tools_with(&self, loaded: &BTreeSet<&'static str>) -> Vec<rmcp::model::Tool> {
        let mut tools = self.tool_router.list_all();
        tools.extend(PackRegistry::shipped().models_for(loaded));
        for tool in tools.iter_mut() {
            sanitize_tool_schema(tool);
        }
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        tools
    }

    /// Dispatch one `tools/call` by exact name: core tools through the
    /// router, pack verbs through the registry (loaded or not), anything
    /// else through the audited name gate (`unknown_tool`, fail closed).
    async fn call_by_name(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if self.tool_router.has_route(&request.name) {
            let tcc = ToolCallContext::new(self, request, context);
            let response = self.tool_router.call(tcc).await?;
            // AX27: a frame receipt answers with an image block plus a caption
            // instead of a raw `png_bytes` number array. Every other value and
            // every error result passes through untouched.
            return match response {
                CallToolResponse::Complete(result) if result.is_error != Some(true) => {
                    match result.structured_content {
                        Some(value) => Ok(frame_call_result(value).into()),
                        None => Ok(result.into()),
                    }
                }
                other => Ok(other),
            };
        }
        let name = request.name.to_string();
        let Some(verb) = PackRegistry::shipped().verb(&name) else {
            return match self.dispatch(&name, |_| Ok::<(), McpToolError>(())) {
                Ok(()) => Err(ErrorData::invalid_params("tool not found", None)),
                Err(error) => Err(ErrorData::from(error)),
            };
        };
        let args = request
            .arguments
            .map_or_else(|| Value::Object(JsonObject::new()), Value::Object);
        match verb.invoke(self, args) {
            // AX27: `play.step` returns its frame the same way `capture` does.
            Ok(value) => Ok(frame_call_result(value).into()),
            Err(error) => Err(ErrorData::from(error)),
        }
    }

    /// Discovery cost in chars: names, descriptions, input and output
    /// schemas. The budget test keeps this bounded. The per-tool rows in
    /// [`Self::tool_cost_table`] sum to this total.
    pub fn tool_list_budget_chars(&self) -> usize {
        self.tools()
            .iter()
            .map(|tool| tool_cost_row(tool).chars)
            .sum()
    }

    /// Per-tool discovery cost table, one row per advertised tool in router
    /// order. Read-only; rows sum to [`Self::tool_list_budget_chars`].
    pub fn tool_cost_table(&self) -> Vec<ToolCostRow> {
        self.tools().iter().map(tool_cost_row).collect()
    }

    /// Snapshot the tool-budget ledger: per-tool rows plus total, budget
    /// and headroom. Read-only.
    pub fn tool_budget_ledger(&self) -> ToolBudgetLedger {
        let rows = self.tool_cost_table();
        let total_chars: usize = rows.iter().map(|row| row.chars).sum();
        ToolBudgetLedger {
            rows,
            total_chars,
            budget_chars: TOOL_LIST_BUDGET_CHARS,
            headroom_chars: TOOL_LIST_BUDGET_CHARS.saturating_sub(total_chars),
        }
    }

    /// Ledger gate: snapshot the ledger and run [`check_tool_budget`] with
    /// `extra_chars` of seeded bloat. `Ok` carries the passing ledger;
    /// `Err` is `tool_list_bloated` or `budget_headroom_exhausted` with the
    /// raise-with-reason rule attached. Read-only.
    pub fn tool_budget_gate_with_bloat(
        &self,
        extra_chars: usize,
    ) -> Result<ToolBudgetLedger, McpToolError> {
        let ledger = self.tool_budget_ledger();
        check_tool_budget(ledger.total_chars, extra_chars)?;
        Ok(ledger)
    }

    /// Lock the session state; a poisoned lock fails closed, never serving
    /// possibly inconsistent state. Crate-visible so the MCP2 tools in
    /// [`super::play`] invoke through the same gate.
    pub(crate) fn lock(&self) -> Result<MutexGuard<'_, ServerState>, McpToolError> {
        self.state.lock().map_err(|_| {
            McpToolError::new(
                "lock_poisoned",
                "server state lock poisoned by a failed call; restart the server",
            )
        })
    }

    /// Route one shim batch through the group lane: one gesture id per batch, seal at batch end.
    /// Runs the execute lane only; previews stay on the dry-run path.
    pub fn brp_batch(
        &self,
        batch: &[crate::remote_shim::BrpRequest],
        gesture_key: u64,
    ) -> crate::discover::BatchExecuteOutput {
        crate::remote_shim::execute_via_shim(self, batch, gesture_key)
    }

    /// Serve one stdio session until the client closes stdin. The caller
    /// provides the async runtime (tokio): `server.run_stdio().await`.
    pub async fn run_stdio(self) -> Result<String, String> {
        use rmcp::{transport::io::stdio, ServiceExt};
        let running = self
            .serve(stdio())
            .await
            .map_err(|error| format!("serve failed: {error:?}"))?;
        let reason = running
            .waiting()
            .await
            .map_err(|error| format!("server task failed: {error:?}"))?;
        Ok(format!("{reason:?}"))
    }
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerConfig {
        let mut capabilities = ServerCapabilities::default();
        capabilities.tools = Some(ToolsCapability::default());
        let mut info = InitializeResult::new(capabilities);
        info.protocol_version = ProtocolVersion::V_2026_07_28;
        "engine-endpoint".clone_into(&mut info.server_info.name);
        env!("CARGO_PKG_VERSION").clone_into(&mut info.server_info.version);
        info.instructions = Some(
            "Scene authoring door: scene.read for the revision, tx.preview for the diff \
             plus a single-use approval, tx.apply with that approval, tx.undo to revert. \
             More verbs live in packs: api.search \"pack\" lists them, api.load advertises \
             one, api.call runs any verb by name. No shell, code execution or file writes."
                .to_owned(),
        );
        info
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(SUPPORTED_PROTOCOL_VERSIONS)
    }

    async fn initialize(
        &self,
        request: rmcp::model::InitializeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, ErrorData> {
        self.initialize_result(&request.protocol_version)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        self.call_by_name(request, context).await
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        // The list moves with `api.load`, so 2026-07-28 clients get a zero
        // cache hint: re-list after loading a pack.
        let supports_cache_hints = context
            .protocol_version()
            .is_some_and(|version| version >= ProtocolVersion::V_2026_07_28);
        Ok(ListToolsResult {
            result_type: Some(ResultType::COMPLETE),
            tools: self.tools(),
            meta: None,
            next_cursor: None,
            ttl_ms: supports_cache_hints.then_some(0),
            cache_scope: supports_cache_hints.then_some(CacheScope::Public),
        })
    }

    fn get_tool(&self, name: &str) -> Option<rmcp::model::Tool> {
        self.tool_router.get(name).cloned().or_else(|| {
            PackRegistry::shipped()
                .verb(name)
                .map(super::packs::VerbSpec::model)
        })
    }
}

impl McpServer {
    /// Synchronous handshake core: echo exact matches, answer mismatches
    /// with a supported version. [`ServerHandler::initialize`] routes
    /// through here, and tests call this directly with no executor.
    pub fn initialize_result(
        &self,
        requested: &ProtocolVersion,
    ) -> Result<InitializeResult, ErrorData> {
        let mut info = self.get_info();
        info.protocol_version = negotiate_version(requested)?;
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::OpMask;

    fn pose(x: f32) -> PoseDto {
        PoseDto {
            pos: [x, 0.0, 0.0],
            rot: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
        }
    }

    fn nil_id() -> String {
        ObjectId::NIL.to_string()
    }

    fn insert_dto(id: &ObjectId, parent: &str, x: f32) -> OpDto {
        OpDto::Insert {
            id: id.to_string(),
            parent: parent.to_owned(),
            local: pose(x),
        }
    }

    fn test_server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    /// Observable session surface for no-write asserts.
    struct Snapshot {
        doc: Document,
        past: usize,
        future: usize,
        budget: u32,
        spent: usize,
    }

    fn snapshot(server: &McpServer) -> Snapshot {
        let state = server.state.lock().expect("test lock holds");
        Snapshot {
            doc: state.doc.clone(),
            past: state.journal.past_len(),
            future: state.journal.future_len(),
            budget: state.cap.remaining(),
            spent: state.ledger.len(),
        }
    }

    impl Snapshot {
        fn assert_same_doc(&self, server: &McpServer) {
            let after = snapshot(server);
            assert_eq!(self.doc, after.doc, "document changed");
            assert_eq!(self.past, after.past, "journal past moved");
            assert_eq!(self.future, after.future, "journal future moved");
            assert_eq!(self.doc.revision, after.doc.revision, "revision moved");
        }

        fn assert_same_all(&self, server: &McpServer) {
            self.assert_same_doc(server);
            let after = snapshot(server);
            assert_eq!(self.budget, after.budget, "capability budget moved");
            assert_eq!(self.spent, after.spent, "approval ledger moved");
        }
    }

    fn preview(
        server: &McpServer,
        rev: u64,
        key: u64,
        ops: Vec<OpDto>,
    ) -> Result<TxPreviewOutput, McpToolError> {
        TxPreviewTool::invoke(
            server,
            TxPreviewArgs {
                expected_revision: rev,
                idempotency_key: key,
                ops,
            },
        )
    }

    fn apply(
        server: &McpServer,
        rev: u64,
        key: u64,
        ops: Vec<OpDto>,
        approval: ApprovalDto,
    ) -> Result<TxApplyOutput, McpToolError> {
        TxApplyTool::invoke(
            server,
            TxApplyArgs {
                expected_revision: rev,
                idempotency_key: key,
                ops,
                approval,
            },
        )
    }

    fn read(server: &McpServer, limit: Option<u32>, cursor: Option<u32>) -> SceneReadOutput {
        SceneReadTool::invoke(server, SceneReadArgs { limit, cursor }).expect("valid page reads")
    }

    #[test]
    fn scripted_insert_preview_apply_undo_roundtrip() {
        let server = test_server();
        // Empty document at revision zero.
        let empty = read(&server, None, None);
        assert_eq!(empty.revision, 0);
        assert_eq!(empty.total, 0);
        assert!(!empty.truncated);

        let id = ObjectId::new();
        let ops = vec![insert_dto(&id, &nil_id(), 1.0)];

        // Preview diffs with no side effects: document, journal, budget and
        // ledger are all identical afterwards.
        let before = snapshot(&server);
        let drafted = preview(&server, 0, 1, ops.clone()).expect("preview inserts");
        assert_eq!(drafted.effects.len(), 1);
        assert!(drafted.effects[0].contains(&id.to_string()));
        assert_eq!(drafted.resulting_revision, 1);
        assert_eq!(drafted.approval.revision, 0);
        assert_eq!(drafted.approval.expiry_tick, 0);
        assert_eq!(drafted.approval.action_hash, drafted.action_hash);
        before.assert_same_all(&server);

        // Apply lands exactly one revision with the previewed effects.
        let landed =
            apply(&server, 0, 1, ops.clone(), drafted.approval.clone()).expect("apply inserts");
        assert_eq!(landed.revision, 1);
        assert_eq!(landed.effects, drafted.effects);
        assert_eq!(landed.key, 1);
        assert_eq!(landed.undo_token, 1);

        let page = read(&server, None, None);
        assert_eq!(page.revision, 1);
        assert_eq!(page.total, 1);
        assert_eq!(page.objects[0].id, id.to_string());
        assert_eq!(page.objects[0].parent, None);
        assert_eq!(page.objects[0].local, pose(1.0));

        // Same key plus same payload replays the recorded receipt.
        let replayed =
            apply(&server, 0, 1, ops.clone(), drafted.approval.clone()).expect("idempotent replay");
        assert_eq!(replayed, landed);
        assert_eq!(read(&server, None, None).revision, 1);

        // Undo reverts the hierarchy as a new revision, keeping a token link.
        let undone = TxUndoTool::invoke(
            &server,
            TxUndoArgs {
                undo_token: Some(landed.undo_token),
            },
        )
        .expect("undo reverts");
        assert_eq!(undone.revision, 2);
        assert_eq!(undone.undone_key, 1);
        let back = read(&server, None, None);
        assert_eq!(back.revision, 2);
        assert_eq!(back.total, 0);
        assert_eq!(back.objects, empty.objects);
    }

    #[test]
    fn set_material_preview_apply_roundtrip_with_typed_bad_name() {
        let server = test_server();
        let node = ObjectId::new();
        let seed_ops = vec![insert_dto(&node, &nil_id(), 0.0)];
        let seeded = preview(&server, 0, 60, seed_ops.clone()).expect("seed preview");
        apply(&server, 0, 60, seed_ops, seeded.approval).expect("seed apply");

        // Bind: preview diffs with no side effects, apply lands the binding.
        let bind = vec![OpDto::SetMaterial {
            id: node.to_string(),
            material: Some("hero-mat".to_owned()),
        }];
        let before = snapshot(&server);
        let drafted = preview(&server, 1, 61, bind.clone()).expect("bind previews");
        assert_eq!(drafted.effects.len(), 1);
        assert!(
            drafted.effects[0].contains("hero-mat"),
            "effect names the bound asset: {}",
            drafted.effects[0]
        );
        before.assert_same_all(&server);
        let landed =
            apply(&server, 1, 61, bind.clone(), drafted.approval.clone()).expect("bind lands");
        assert_eq!(landed.revision, 2);
        assert_eq!(landed.effects, drafted.effects);

        // Clear: `None` unbinds through the same preview/apply gate.
        let clear = vec![OpDto::SetMaterial {
            id: node.to_string(),
            material: None,
        }];
        let drafted = preview(&server, 2, 62, clear.clone()).expect("clear previews");
        let landed = apply(&server, 2, 62, clear, drafted.approval).expect("clear lands");
        assert_eq!(landed.revision, 3);

        // Bad name: typed refusal names the legal charset, writes nothing.
        let before = snapshot(&server);
        let bad = vec![OpDto::SetMaterial {
            id: node.to_string(),
            material: Some("hero mat!".to_owned()),
        }];
        let refused = preview(&server, 3, 63, bad).expect_err("bad name must fail");
        assert_eq!(refused.code, "bad_material");
        assert!(
            refused.message.contains("0-9 A-Z a-z - _ ."),
            "names the legal charset: {}",
            refused.message
        );
        before.assert_same_all(&server);
    }

    #[test]
    fn denied_stale_duplicate_calls_write_nothing() {
        let server = test_server();
        let seed = ObjectId::new();
        let seed_ops = vec![insert_dto(&seed, &nil_id(), 0.0)];
        let seeded = preview(&server, 0, 10, seed_ops.clone()).expect("seed preview");
        apply(&server, 0, 10, seed_ops, seeded.approval).expect("seed apply");
        assert_eq!(read(&server, None, None).revision, 1);

        // Forged approval: hash from another batch.
        let other = ObjectId::new();
        let other_ops = vec![insert_dto(&other, &nil_id(), 5.0)];
        let other_preview = preview(&server, 1, 11, other_ops.clone()).expect("other preview");
        let before = snapshot(&server);
        let mut forged = other_preview.approval.clone();
        forged.action_hash ^= 0xdead_beef;
        let refused = apply(&server, 1, 11, other_ops.clone(), forged);
        assert_eq!(
            refused.expect_err("forged approval must fail").code,
            "forged_approval"
        );
        before.assert_same_all(&server);

        // Stale revision: batch prepared before the seed landed.
        let stale_ops = vec![insert_dto(&ObjectId::new(), &nil_id(), 2.0)];
        let stale_preview = preview(&server, 0, 12, stale_ops);
        assert_eq!(
            stale_preview.expect_err("stale preview must fail").code,
            "stale_revision"
        );
        before.assert_same_all(&server);

        // Expired approval: a short-lived grant presented after its expiry.
        // (Preview mints expiry == current revision; a host may narrow it.
        // CAP1 checks staleness before expiry, so the batch stays current.)
        let filler_ops = vec![insert_dto(&ObjectId::new(), &nil_id(), 4.0)];
        let filler = preview(&server, 1, 14, filler_ops.clone()).expect("filler preview");
        apply(&server, 1, 14, filler_ops, filler.approval).expect("filler lands");
        assert_eq!(read(&server, None, None).revision, 2);
        let expiring_ops = vec![insert_dto(&ObjectId::new(), &nil_id(), 3.0)];
        let expiring = preview(&server, 2, 13, expiring_ops.clone()).expect("expiring preview");
        let mut short_lived = expiring.approval.clone();
        short_lived.expiry_tick = 1;
        let before = snapshot(&server);
        let expired = apply(&server, 2, 13, expiring_ops, short_lived);
        assert_eq!(
            expired.expect_err("expired approval must fail").code,
            "expired_approval"
        );
        before.assert_same_all(&server);

        // Duplicate key with a different payload: refused as a conflict.
        // (The ledger spend on this path is CAP1 design; document and both
        // journal stacks stay put.)
        let clash_ops = vec![insert_dto(&ObjectId::new(), &nil_id(), 6.0)];
        let clash = preview(&server, 2, 15, clash_ops.clone()).expect("clash preview");
        apply(&server, 2, 15, clash_ops.clone(), clash.approval.clone()).expect("clash lands");
        let different = vec![insert_dto(&ObjectId::new(), &nil_id(), 7.0)];
        let different_preview =
            preview(&server, 3, 15, different.clone()).expect("conflict preview");
        let before = snapshot(&server);
        let conflict = apply(&server, 3, 15, different, different_preview.approval);
        assert_eq!(
            conflict.expect_err("key conflict must fail").code,
            "duplicate_key"
        );
        before.assert_same_doc(&server);
        // The recorded key still replays its original receipt.
        let replayed = apply(&server, 2, 15, clash_ops, clash.approval).expect("replay");
        assert_eq!(replayed.revision, 3);

        // Reused approval after undo: the retry is refused, never resurrected.
        let cycle_ops = vec![insert_dto(&ObjectId::new(), &nil_id(), 8.0)];
        let cycle = preview(&server, 3, 16, cycle_ops.clone()).expect("cycle preview");
        apply(&server, 3, 16, cycle_ops.clone(), cycle.approval.clone()).expect("cycle lands");
        TxUndoTool::invoke(&server, TxUndoArgs { undo_token: None }).expect("cycle undone");
        let before = snapshot(&server);
        let resurrected = apply(&server, 3, 16, cycle_ops, cycle.approval);
        assert_eq!(
            resurrected.expect_err("spent approval must fail").code,
            "reused_approval"
        );
        before.assert_same_all(&server);

        // Undone key with a fresh approval: refused, never resurrected.
        // (Same CAP1 ledger-spend note as the duplicate-key path.)
        let current = read(&server, None, None).revision;
        let rehash_ops = vec![insert_dto(&ObjectId::new(), &nil_id(), 9.0)];
        let rehash = preview(&server, current, 16, rehash_ops.clone())
            .expect("fresh preview for an undone key");
        let before = snapshot(&server);
        let undone = apply(&server, current, 16, rehash_ops, rehash.approval);
        assert_eq!(undone.expect_err("undone key must fail").code, "undone_key");
        before.assert_same_doc(&server);

        // Out-of-scope object: the gate fires on apply (preview is a dry
        // run), writing nothing.
        let scoped = McpServer::new(
            9,
            Capability::new(9, OpMask::ALL, vec![seed.to_string()], 1),
        );
        let guided_ops = vec![insert_dto(&seed, &nil_id(), 0.0)];
        assert!(
            preview(&scoped, 0, 20, guided_ops).is_ok(),
            "in-scope insert previews"
        );
        let before = snapshot(&scoped);
        let far = ObjectId::new();
        let far_ops = vec![insert_dto(&far, &nil_id(), 0.0)];
        let far_approval = preview(&scoped, 0, 21, far_ops.clone())
            .expect("preview is scope-free")
            .approval;
        let denied = apply(&scoped, 0, 21, far_ops, far_approval);
        let denied_error = denied.expect_err("out-of-scope apply must fail");
        assert_eq!(denied_error.code, "out_of_scope");
        assert!(denied_error.message.contains(&far.to_string()));
        before.assert_same_all(&scoped);

        // Exhausted budget: same no-write guarantee.
        let broke = McpServer::new(11, Capability::new(11, OpMask::ALL, vec![], 0));
        let tight_ops = vec![insert_dto(&ObjectId::new(), &nil_id(), 0.0)];
        let tight_approval = preview(&broke, 0, 22, tight_ops.clone())
            .expect("preview is budget-free")
            .approval;
        let before = snapshot(&broke);
        let broke_apply = apply(&broke, 0, 22, tight_ops, tight_approval);
        let broke_error = broke_apply.expect_err("over-budget apply must fail");
        assert_eq!(broke_error.code, "over_budget");
        assert!(broke_error.message.contains('0'));
        before.assert_same_all(&broke);
    }

    #[test]
    fn capability_gate_fronts_apply_ahead_of_approval_checks() {
        // Out-of-scope batch plus a stale approval: the taxonomy gate fires
        // first, so the door names the scope, not the approval.
        let seed = ObjectId::new();
        let scoped = McpServer::new(
            9,
            Capability::new(9, OpMask::ALL, vec![seed.to_string()], 8),
        );
        let far = ObjectId::new();
        let far_ops = vec![insert_dto(&far, &nil_id(), 0.0)];
        let stale_approval = ApprovalDto {
            revision: 99,
            action_hash: 1,
            expiry_tick: 99,
        };
        let before = snapshot(&scoped);
        let refused = apply(&scoped, 0, 60, far_ops, stale_approval)
            .expect_err("out-of-scope apply must fail on scope first");
        assert_eq!(refused.code, "out_of_scope");
        assert!(refused.message.contains(&far.to_string()));
        before.assert_same_all(&scoped);

        // Over-budget batch plus a forged approval: budget wins the same way.
        let broke = McpServer::new(11, Capability::new(11, OpMask::ALL, vec![], 0));
        let tight_ops = vec![insert_dto(&ObjectId::new(), &nil_id(), 0.0)];
        let forged = ApprovalDto {
            revision: 0,
            action_hash: 0xBEEF,
            expiry_tick: 0,
        };
        let before = snapshot(&broke);
        let refused = apply(&broke, 0, 61, tight_ops, forged)
            .expect_err("over-budget apply must fail on budget first");
        assert_eq!(refused.code, "over_budget");
        before.assert_same_all(&broke);
    }

    #[test]
    fn spatial_rejection_writes_nothing() {
        let server = test_server();
        // Two grounded roots: x=0 and x=10, unit cubes, no overlap.
        let a = ObjectId::new();
        let b = ObjectId::new();
        for (key, id, x) in [(30u64, a, 0.0f32), (31u64, b, 10.0f32)] {
            let ops = vec![insert_dto(&id, &nil_id(), x)];
            let rev = read(&server, None, None).revision;
            let drafted = preview(&server, rev, key, ops.clone()).expect("seed preview");
            apply(&server, rev, key, ops, drafted.approval).expect("seed apply");
        }
        assert_eq!(read(&server, None, None).revision, 2);

        // Drag `b` onto `a`: the staged overlap check refuses, writing nothing.
        let before = snapshot(&server);
        let clash = vec![OpDto::SetLocal {
            id: b.to_string(),
            local: pose(0.0),
        }];
        let drafted = preview(&server, 2, 32, clash.clone()).expect("overlap previews");
        let refused = apply(&server, 2, 32, clash, drafted.approval);
        let error = refused.expect_err("overlap must fail");
        assert_eq!(error.code, "spatial_rejected");
        assert!(error.message.contains("overlap"));
        before.assert_same_all(&server);

        // A clean grounded nudge nearby still applies through the same gate.
        let nudge = vec![OpDto::SetLocal {
            id: b.to_string(),
            local: pose(5.0),
        }];
        let drafted = preview(&server, 2, 33, nudge.clone()).expect("nudge previews");
        apply(&server, 2, 33, nudge, drafted.approval).expect("nudge lands");
        assert_eq!(read(&server, None, None).revision, 3);
    }

    #[test]
    fn tool_schema_drift_matches_command_shapes() {
        // Every tx input schema advertises the seven command ops.
        for tool in [TxPreviewTool::input_schema(), TxApplyTool::input_schema()] {
            let schema = tool.expect("tx tools carry input schemas");
            let text = rmcp::serde_json::to_string(&*schema).expect("schema serializes");
            let value: rmcp::serde_json::Value =
                rmcp::serde_json::from_str(&text).expect("schema is JSON");
            // Exactly seven discriminator branches: one per Op variant.
            assert_eq!(collect_branches(&value), 7, "schema drift: {text}");
            // Exactly the seven op tags, matching Op's snake_case variants.
            let mut tags = collect_op_tags(&value);
            tags.sort();
            tags.dedup();
            assert_eq!(
                tags,
                [
                    "insert",
                    "remove",
                    "remove_component",
                    "reparent",
                    "set_component",
                    "set_local",
                    "set_material"
                ]
            );
        }

        // Every advertised op converts to a real command and validates alone.
        let server = test_server();
        let target = ObjectId::new();
        let seed_ops = vec![insert_dto(&target, &nil_id(), 0.0)];
        let seed = preview(&server, 0, 40, seed_ops.clone()).expect("seed preview");
        apply(&server, 0, 40, seed_ops, seed.approval).expect("seed apply");
        let fresh = ObjectId::new();
        let all_ops = [
            insert_dto(&fresh, &nil_id(), 1.0),
            OpDto::SetLocal {
                id: target.to_string(),
                local: pose(2.0),
            },
            OpDto::Reparent {
                id: target.to_string(),
                new_parent: nil_id(),
            },
            OpDto::Remove {
                id: target.to_string(),
            },
            OpDto::SetMaterial {
                id: target.to_string(),
                material: Some("hero-mat".to_owned()),
            },
            OpDto::SetComponent {
                id: target.to_string(),
                component: "mesh".to_owned(),
                value: rmcp::serde_json::json!({"asset": "meshes/wall"}),
                version: None,
            },
        ];
        for (index, dto) in all_ops.iter().enumerate() {
            let op = op_from_dto(index, dto).expect("advertised op converts");
            let discriminant = match op {
                Op::Insert { .. } => "insert",
                Op::Remove { .. } => "remove",
                Op::SetLocal { .. } => "set_local",
                Op::Reparent { .. } => "reparent",
                Op::SetMaterial { .. } => "set_material",
                Op::SetComponent { .. } => "set_component",
                Op::RemoveComponent { .. } => "remove_component",
            };
            let tag = match dto {
                OpDto::Insert { .. } => "insert",
                OpDto::Remove { .. } => "remove",
                OpDto::SetLocal { .. } => "set_local",
                OpDto::Reparent { .. } => "reparent",
                OpDto::SetMaterial { .. } => "set_material",
                OpDto::SetComponent { .. } => "set_component",
                OpDto::RemoveComponent { .. } => "remove_component",
            };
            assert_eq!(discriminant, tag, "DTO tag must match the Op variant");
        }
        // Each op alone must validate (all target live nodes here).
        for (key, dto) in all_ops.iter().enumerate() {
            let rev = read(&server, None, None).revision;
            preview(&server, rev, 50 + key as u64, vec![dto.clone()])
                .expect("advertised op shape must validate");
        }
        // The removal tag converts too (validation needs a set component).
        let removal = OpDto::RemoveComponent {
            id: target.to_string(),
            component: "mesh".to_owned(),
        };
        assert!(matches!(
            op_from_dto(0, &removal).expect("converts"),
            Op::RemoveComponent { .. }
        ));

        // Unknown op tag on the wire lists the allowed values, never an
        // empty pass.
        let wire = rmcp::serde_json::json!({
            "op": "teleport",
            "id": ObjectId::new().to_string(),
        });
        let error = rmcp::serde_json::from_value::<OpDto>(wire).expect_err("unknown op");
        let report = error.to_string();
        for allowed in [
            "insert",
            "remove",
            "set_local",
            "reparent",
            "set_material",
            "set_component",
            "remove_component",
        ] {
            assert!(
                report.contains(allowed),
                "unknown op must list `{allowed}`: {report}"
            );
        }
    }

    /// Count every `oneOf`/`anyOf` array length found in a schema value.
    fn collect_branches(value: &rmcp::serde_json::Value) -> usize {
        use rmcp::serde_json::Value;
        let mut total = 0;
        match value {
            Value::Array(items) => {
                for item in items {
                    total += collect_branches(item);
                }
            }
            Value::Object(map) => {
                for (key, item) in map {
                    if key == "oneOf" || key == "anyOf" {
                        if let Value::Array(branches) = item {
                            total += branches.len();
                        }
                    } else {
                        total += collect_branches(item);
                    }
                }
            }
            _ => {}
        }
        total
    }

    /// Collect every op tag under a `const`/`enum` key in a schema value.
    fn collect_op_tags(value: &rmcp::serde_json::Value) -> Vec<String> {
        use rmcp::serde_json::Value;
        const KNOWN: &[&str] = &[
            "insert",
            "remove",
            "set_local",
            "reparent",
            "set_material",
            "set_component",
            "remove_component",
        ];
        let mut out = Vec::new();
        match value {
            Value::Array(items) => {
                for item in items {
                    out.extend(collect_op_tags(item));
                }
            }
            Value::Object(map) => {
                for (key, item) in map {
                    match key.as_str() {
                        "const" => {
                            if let Value::String(text) = item {
                                if KNOWN.contains(&text.as_str()) {
                                    out.push(text.clone());
                                }
                            }
                        }
                        "enum" => {
                            if let Value::Array(options) = item {
                                for option in options {
                                    if let Value::String(text) = option {
                                        if KNOWN.contains(&text.as_str()) {
                                            out.push(text.clone());
                                        }
                                    }
                                }
                            }
                        }
                        _ => out.extend(collect_op_tags(item)),
                    }
                }
            }
            _ => {}
        }
        out
    }

    #[test]
    fn protocol_version_negotiation_answers_supported() {
        let server = test_server();
        assert_eq!(
            server.get_info().protocol_version,
            ProtocolVersion::V_2026_07_28
        );
        assert_eq!(
            server.supported_protocol_versions().as_ref(),
            ProtocolVersion::KNOWN_VERSIONS
        );

        // Every known version echoes: exact matches never answer -32022.
        for candidate in ProtocolVersion::KNOWN_VERSIONS {
            let agreed = negotiate_version(candidate).expect("exact match passes");
            assert_eq!(&agreed, candidate, "exact match echoes");
            let live = server
                .initialize_result(candidate)
                .expect("handshake answers exact match");
            assert_eq!(&live.protocol_version, candidate);
            assert!(live.capabilities.tools.is_some());
            assert!(server.supported_protocol_versions().contains(candidate));
        }

        // The EB-2026-09-26-79 client: 2025-06-18 handshakes and lists tools.
        let legacy = server
            .initialize_result(&ProtocolVersion::V_2025_06_18)
            .expect("2025-06-18 handshakes");
        assert_eq!(legacy.protocol_version, ProtocolVersion::V_2025_06_18);
        assert!(legacy.capabilities.tools.is_some());
        assert!(
            !server.tools().is_empty(),
            "legacy handshake still lists tools"
        );

        // Unknown versions fall back to a supported version, never -32022.
        let unknown: ProtocolVersion =
            rmcp::serde_json::from_str(r#""1999-01-01""#).expect("unknown version parses");
        let fallback = negotiate_version(&unknown).expect("mismatch answers");
        assert!(
            server.supported_protocol_versions().contains(&fallback),
            "fallback is supported: {fallback}"
        );
        let live = server
            .initialize_result(&unknown)
            .expect("handshake answers mismatch");
        assert!(
            server
                .supported_protocol_versions()
                .contains(&live.protocol_version),
            "handshake fallback is supported"
        );
    }

    /// The 21 tools of the old door, every one still reachable.
    const OLD_DOOR: [&str; 21] = [
        "api.describe",
        "api.search",
        "asset.refs",
        "capture",
        "code.callers",
        "level.build",
        "log.digest",
        "log.query",
        "play.run",
        "play.step",
        "rpc.discover",
        "scene.brief",
        "scene.place",
        "scene.query",
        "scene.read",
        "scene.spatial",
        "scene.watch",
        "sim.plan_goals",
        "tx.apply",
        "tx.preview",
        "tx.undo",
    ];

    /// Discovery chars of a tool list.
    fn chars_of(tools: &[rmcp::model::Tool]) -> usize {
        tools.iter().map(|tool| tool_cost_row(tool).chars).sum()
    }

    fn assert_core_tools(server: &McpServer) -> Vec<String> {
        let mut names: Vec<String> = server
            .tools()
            .iter()
            .map(|tool| tool.name.to_string())
            .collect();
        names.sort();
        let mut expected: Vec<String> = default_tool_names()
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        expected.sort();
        assert_eq!(
            names, expected,
            "fresh session lists the core plus the scene pack"
        );
        for core in CORE_TOOL_NAMES {
            assert!(
                names.contains(&core.to_owned()),
                "core tool `{core}` is listed"
            );
        }
        names
    }

    fn assert_default_packs(names: &[String]) {
        assert!(
            names.contains(&"scene.brief".to_owned()),
            "scene pack is loaded by default"
        );
        assert!(
            !names.contains(&"log.digest".to_owned()),
            "view pack waits for api.load"
        );
    }

    fn assert_back_compat(server: &McpServer) -> Vec<String> {
        // Every old tool is reachable: a contract name, a router or pack
        // entry, and advertised once its pack loads.
        let reachable = tool_names();
        let mut all: Vec<String> = server
            .all_tools()
            .iter()
            .map(|tool| tool.name.to_string())
            .collect();
        all.sort();
        for old in OLD_DOOR {
            assert!(
                reachable.contains(&old),
                "old tool `{old}` stays a contract name"
            );
            assert!(
                all.contains(&old.to_owned()),
                "old tool `{old}` is reachable"
            );
        }
        let mut contract: Vec<&str> = reachable.clone();
        contract.sort_unstable();
        contract.dedup();
        assert_eq!(contract.len(), reachable.len(), "contract names are unique");
        assert_eq!(
            all,
            contract
                .iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>(),
            "all_tools covers exactly the contract names"
        );

        // No arbitrary execute/shell/file-write surface, by name or by hint.
        // `play.run` is allowlisted by exact name: it runs a bounded headless
        // arena match through the job registry, never arbitrary code.
        // `code.callers` is allowlisted by exact name: it is a read-only
        // reverse lookup over the Insight callers graph, never code execution.
        // `test.run` is allowlisted by exact name: it runs the read-only
        // harvest checks (scene, preview, budget, manifest, log), never arbitrary code.
        for tool in server.all_tools() {
            let name = tool.name.to_lowercase();
            if name != "play.run" && name != "code.callers" && name != "test.run" {
                for forbidden in ["exec", "shell", "file", "run", "code", "eval", "terminal"] {
                    assert!(
                        !name.contains(forbidden),
                        "tool `{name}` looks like arbitrary execution"
                    );
                }
            }
            assert!(
                !tool.input_schema.is_empty(),
                "tool `{name}` must advertise its input schema"
            );
            assert!(
                tool.output_schema.is_some(),
                "tool `{name}` must advertise its output schema"
            );
        }

        all
    }

    fn assert_budget_grows(server: &McpServer, all: &[String]) {
        // Token budget: the advertised list is shorter than the 21-tool door
        // (43514 chars budgeted, 40514 measured on 2026-10-01) and shorter
        // than the whole reachable set, with the ledger headroom intact.
        let budget = server.tool_list_budget_chars();
        let everything = chars_of(&server.all_tools());
        let core_only = chars_of(&server.core_tools());
        eprintln!(
            "door v2: default list {budget} chars, core {core_only}, all {} tools {everything} \
             chars, budget {TOOL_LIST_BUDGET_CHARS}",
            all.len()
        );
        assert!(budget > 0, "budget counts something");
        assert!(
            budget < TOOL_LIST_BUDGET_CHARS,
            "default list costs {budget} chars, the old door budgeted {TOOL_LIST_BUDGET_CHARS}"
        );
        assert!(
            budget < 40_514,
            "default list {budget} is shorter than the measured 21-tool door"
        );
        assert!(budget < everything, "the default list hides something");
        assert!(core_only < budget, "the scene pack adds to the core");
        let ledger = server
            .tool_budget_gate_with_bloat(0)
            .expect("live tool list passes the ledger gate");
        assert_eq!(ledger.total_chars, budget, "ledger sums the live list");
        assert!(
            ledger.headroom_chars >= TOOL_LIST_MIN_HEADROOM_CHARS,
            "headroom {} chars under minimum {TOOL_LIST_MIN_HEADROOM_CHARS}",
            ledger.headroom_chars
        );

        // Loading a pack advertises its verbs and grows the list; the
        // ledger follows the live list.
        server
            .lock()
            .expect("test lock holds")
            .load_pack("view", false)
            .expect("view loads");
        let mut listed: Vec<String> = server
            .tools()
            .iter()
            .map(|tool| tool.name.to_string())
            .collect();
        listed.sort();
        assert!(listed.contains(&"log.digest".to_owned()));
        assert!(listed.contains(&"rpc.discover".to_owned()));
        let grown = server.tool_list_budget_chars();
        assert!(
            grown > budget,
            "loading view grows the list: {grown} vs {budget}"
        );
        assert_eq!(server.tool_budget_ledger().total_chars, grown);
        // The loaded list is a real advertised list too: the same ledger gate
        // must pass it, floor included, or `api.load` has no room to hand a
        // pack to a client (bug:endpoint-api-load-budget).
        let loaded_gate = server
            .tool_budget_gate_with_bloat(0)
            .expect("the loaded list passes the ledger gate with the floor intact");
        assert_eq!(loaded_gate.total_chars, grown);
        assert!(
            loaded_gate.headroom_chars >= TOOL_LIST_MIN_HEADROOM_CHARS,
            "loaded list leaves {} chars, floor is {TOOL_LIST_MIN_HEADROOM_CHARS}",
            loaded_gate.headroom_chars
        );
    }

    #[test]
    fn door_v2_lists_core_plus_default_packs_and_keeps_every_old_tool_reachable() {
        let server = test_server();
        let names = assert_core_tools(&server);
        assert_default_packs(&names);
        let all = assert_back_compat(&server);
        assert_budget_grows(&server, &all);
    }

    #[test]
    fn component_ops_set_undo_and_refuse_typed_through_the_door() {
        let server = test_server();
        let node = ObjectId::new();
        let seed = vec![insert_dto(&node, &nil_id(), 0.0)];
        let drafted = preview(&server, 0, 70, seed.clone()).expect("seed previews");
        apply(&server, 0, 70, seed, drafted.approval).expect("seed lands");

        // All eight kinds in one batch, authored as plain JSON.
        let kinds = [
            ("mesh", rmcp::serde_json::json!({"asset": "meshes/wall"})),
            (
                "collider",
                rmcp::serde_json::json!({"shape": "box", "half": [1.0, 2.0, 0.2]}),
            ),
            ("material", rmcp::serde_json::json!({"asset": "marble"})),
            (
                "light",
                rmcp::serde_json::json!({"kind": "point", "color": [1, 1, 1], "intensity": 5, "range": 10}),
            ),
            (
                "spawner",
                rmcp::serde_json::json!({"archetype": "grunt", "count": 2, "period_ticks": 300}),
            ),
            (
                "destructible",
                rmcp::serde_json::json!({"material": "stone", "health": 100, "chunks": 32}),
            ),
            (
                "anatomy",
                rmcp::serde_json::json!({"template": "humanoid", "limbs": 4}),
            ),
            (
                "rig",
                rmcp::serde_json::json!({"skeleton": "hero", "clips": "hero_clips"}),
            ),
        ];
        let ops: Vec<OpDto> = kinds
            .iter()
            .map(|(kind, value)| OpDto::SetComponent {
                id: node.to_string(),
                component: (*kind).to_owned(),
                value: value.clone(),
                version: None,
            })
            .collect();
        let drafted = preview(&server, 1, 71, ops.clone()).expect("components preview");
        assert_eq!(drafted.effects.len(), 8);
        assert!(
            drafted.effects[1].contains("collider"),
            "{:?}",
            drafted.effects
        );
        let landed = apply(&server, 1, 71, ops, drafted.approval).expect("components land");
        assert_eq!(landed.revision, 2);
        {
            let state = server.lock().expect("lock");
            let slot = state.doc.components.get(&node.to_string()).expect("slot");
            assert_eq!(slot.len(), 8);
        }

        // Bad kind, bad payload, bad range and unknown version refuse typed.
        let refuse = |component: &str, value: rmcp::serde_json::Value, version: Option<u32>| {
            preview(
                &server,
                2,
                80,
                vec![OpDto::SetComponent {
                    id: node.to_string(),
                    component: component.to_owned(),
                    value,
                    version,
                }],
            )
            .expect_err("refused")
        };
        let bad_kind = refuse("teleporter", rmcp::serde_json::json!({}), None);
        assert_eq!(bad_kind.code, "bad_component");
        assert!(
            bad_kind.message.contains("mesh, collider"),
            "{}",
            bad_kind.message
        );
        let bad_field = refuse(
            "mesh",
            rmcp::serde_json::json!({"asset": "a", "extra": 1}),
            None,
        );
        assert_eq!(bad_field.code, "bad_component");
        let bad_range = refuse(
            "spawner",
            rmcp::serde_json::json!({"archetype": "a", "count": 1, "period_ticks": 0}),
            None,
        );
        assert!(
            bad_range.message.contains("period_ticks"),
            "{}",
            bad_range.message
        );
        let future = refuse("mesh", rmcp::serde_json::json!({"asset": "a"}), Some(9));
        assert!(future.message.contains("v9"), "{}", future.message);

        // Undo restores the pre-batch document (no components).
        TxUndoTool::invoke(&server, TxUndoArgs { undo_token: None }).expect("undo");
        assert!(server.lock().expect("lock").doc.components.is_empty());
    }

    #[test]
    fn approvals_bind_the_engine_session_not_the_protocol_session() {
        // The wire approval carries no session at all: `tx.apply` stamps the
        // server's own engine session (fixed at construction), so nothing a
        // protocol handshake says can move the binding. Two engine sessions
        // keep two ledgers: a spend on one never consumes the other's.
        let seven = McpServer::new(7, Capability::unrestricted(7));
        let nine = McpServer::new(9, Capability::unrestricted(9));
        assert_eq!(seven.lock().expect("lock").session(), 7);
        assert_eq!(nine.lock().expect("lock").session(), 9);
        let id = ObjectId::new();
        let ops = vec![insert_dto(&id, &nil_id(), 1.0)];
        let minted = preview(&seven, 0, 500, ops.clone()).expect("preview mints");
        // The protocol handshake, at any version, never moves the engine
        // session or the minted binding.
        for candidate in ProtocolVersion::KNOWN_VERSIONS {
            seven
                .initialize_result(candidate)
                .expect("handshake answers");
        }
        assert_eq!(seven.lock().expect("lock").session(), 7);
        let landed =
            apply(&seven, 0, 500, ops.clone(), minted.approval.clone()).expect("own session lands");
        assert_eq!(landed.revision, 1);
        // Single use per (engine session, batch hash): replay refuses.
        let replay = apply(&seven, 0, 501, ops.clone(), minted.approval.clone())
            .expect_err("a spent approval refuses");
        assert!(
            !replay.code.is_empty(),
            "the replay refuses with a typed code"
        );
        // The other engine session spends from its own ledger.
        let own = preview(&nine, 0, 500, ops.clone()).expect("nine mints its own");
        let landed = apply(&nine, 0, 500, ops, own.approval).expect("nine lands");
        assert_eq!(landed.revision, 1);
        // The stdio door binds one fixed engine session per process.
        assert_eq!(super::super::startup::MCP_SESSION, 1);
    }

    #[test]
    fn tool_cost_table_sums_to_total_and_posts_rows() {
        let server = test_server();
        let ledger = server.tool_budget_ledger();
        // Posted: one ledger line per tool, so budget growth names its owner.
        for row in &ledger.rows {
            eprintln!("tool cost: {:<16} {:>6} chars", row.name, row.chars);
        }
        eprintln!(
            "tool budget: total {} chars of {}, headroom {}",
            ledger.total_chars, ledger.budget_chars, ledger.headroom_chars
        );
        assert_eq!(ledger.rows.len(), default_tool_names().len());
        let mut names: Vec<&str> = ledger.rows.iter().map(|row| row.name.as_str()).collect();
        names.sort_unstable();
        let mut expected = default_tool_names();
        expected.sort_unstable();
        assert_eq!(
            names.as_slice(),
            expected.as_slice(),
            "cost table covers exactly the advertised tools"
        );
        for row in &ledger.rows {
            assert!(row.chars > 0, "tool `{}` must cost something", row.name);
        }
        let sum: usize = ledger.rows.iter().map(|row| row.chars).sum();
        assert_eq!(sum, ledger.total_chars, "rows sum to the ledger total");
        assert_eq!(
            ledger.total_chars,
            server.tool_list_budget_chars(),
            "ledger total matches the advertised total"
        );
        assert_eq!(ledger.budget_chars, TOOL_LIST_BUDGET_CHARS);
        assert_eq!(
            ledger.headroom_chars,
            TOOL_LIST_BUDGET_CHARS.saturating_sub(ledger.total_chars)
        );
    }

    #[test]
    fn seeded_bloat_of_500_chars_fails_the_gate() {
        // Seed a total one char under the ceiling: the 500-char probe overflows.
        let seeded = TOOL_LIST_BUDGET_CHARS - (TOOL_BUDGET_PROBE_BLOAT_CHARS - 1);
        let refused = check_tool_budget(seeded, TOOL_BUDGET_PROBE_BLOAT_CHARS)
            .expect_err("seeded 500-char bloat must fail the gate");
        assert_eq!(refused.code, "tool_list_bloated");
        assert!(
            refused
                .message
                .contains(TOOL_LIST_BUDGET_CHARS.to_string().as_str()),
            "refusal names the budget: {}",
            refused.message
        );
        // The live list still passes unbloated, ledger in hand.
        let server = test_server();
        let ledger = server
            .tool_budget_gate_with_bloat(0)
            .expect("live list passes unbloated");
        assert_eq!(ledger.total_chars, server.tool_list_budget_chars());
    }

    #[test]
    fn headroom_below_2k_forces_raise_or_refusal() {
        // One char under the minimum headroom: refused, with the raise rule.
        let tight = TOOL_LIST_BUDGET_CHARS - (TOOL_LIST_MIN_HEADROOM_CHARS - 1);
        let refused = check_tool_budget(tight, 0).expect_err("sub-minimum headroom must fail");
        assert_eq!(refused.code, "budget_headroom_exhausted");
        assert!(
            refused
                .message
                .contains(TOOL_LIST_MIN_HEADROOM_CHARS.to_string().as_str()),
            "refusal names the minimum: {}",
            refused.message
        );
        assert!(
            refused.message.contains("raise"),
            "refusal forces raise-with-reason or shrink: {}",
            refused.message
        );
        // Exactly the minimum headroom passes and reports it.
        let floor = TOOL_LIST_BUDGET_CHARS - TOOL_LIST_MIN_HEADROOM_CHARS;
        let headroom = check_tool_budget(floor, 0).expect("exact minimum headroom passes");
        assert_eq!(headroom, TOOL_LIST_MIN_HEADROOM_CHARS);
    }

    #[test]
    fn scene_read_paging_and_bounds() {
        let server = test_server();
        let mut ids = Vec::new();
        for key in 60u64..63 {
            let id = ObjectId::new();
            let rev = read(&server, None, None).revision;
            let ops = vec![insert_dto(&id, &nil_id(), key as f32)];
            let drafted = preview(&server, rev, key, ops.clone()).expect("seed preview");
            apply(&server, rev, key, ops, drafted.approval).expect("seed apply");
            ids.push(id.to_string());
        }
        ids.sort();

        // Pages walk the id-sorted rows.
        let first = read(&server, Some(2), Some(0));
        assert_eq!(first.total, 3);
        assert_eq!(first.returned, 2);
        assert!(first.truncated);
        let second = read(&server, Some(2), Some(2));
        assert_eq!(second.returned, 1);
        assert!(!second.truncated);
        let rows: Vec<String> = first
            .objects
            .iter()
            .map(|row| row.id.clone())
            .chain(second.objects.iter().map(|row| row.id.clone()))
            .collect();
        assert_eq!(rows, ids);

        // Out-of-range paging fails with the legal values attached.
        let over_limit = SceneReadTool::invoke(
            &server,
            SceneReadArgs {
                limit: Some(999),
                cursor: None,
            },
        )
        .expect_err("limit over the cap must fail");
        assert_eq!(over_limit.code, "bad_limit");
        assert!(over_limit.message.contains("128"));

        let zero_limit = SceneReadTool::invoke(
            &server,
            SceneReadArgs {
                limit: Some(0),
                cursor: None,
            },
        )
        .expect_err("zero limit must fail");
        assert_eq!(zero_limit.code, "bad_limit");

        let past_end = SceneReadTool::invoke(
            &server,
            SceneReadArgs {
                limit: None,
                cursor: Some(99),
            },
        )
        .expect_err("cursor past the end must fail");
        assert_eq!(past_end.code, "bad_cursor");
        assert!(past_end.message.contains('3'));
    }

    #[test]
    fn empty_and_oversize_batches_are_rejected() {
        let server = test_server();
        let before = snapshot(&server);

        let empty = preview(&server, 0, 70, vec![]).expect_err("empty batch must fail");
        assert_eq!(empty.code, "empty_batch");
        assert!(empty.message.contains("256"));

        let id = ObjectId::new();
        let huge = vec![insert_dto(&id, &nil_id(), 0.0); MAX_TX_OPS + 1];
        let oversize = preview(&server, 0, 71, huge).expect_err("oversize batch must fail");
        assert_eq!(oversize.code, "too_many_ops");
        assert!(oversize.message.contains("256"));
        before.assert_same_all(&server);
    }

    #[test]
    fn undo_token_mismatch_and_empty_undo_fail() {
        let server = test_server();
        let empty_undo = TxUndoTool::invoke(&server, TxUndoArgs { undo_token: None })
            .expect_err("empty journal must fail");
        assert_eq!(empty_undo.code, "nothing_to_undo");

        let id = ObjectId::new();
        let rev = read(&server, None, None).revision;
        let ops = vec![insert_dto(&id, &nil_id(), 0.0)];
        let drafted = preview(&server, rev, 80, ops.clone()).expect("preview");
        apply(&server, rev, 80, ops, drafted.approval).expect("apply");

        let before = snapshot(&server);
        let mismatch = TxUndoTool::invoke(
            &server,
            TxUndoArgs {
                undo_token: Some(81),
            },
        )
        .expect_err("wrong token must fail");
        assert_eq!(mismatch.code, "undo_mismatch");
        assert!(mismatch.message.contains("80"));
        before.assert_same_all(&server);
    }

    #[test]
    fn service_rehydrate_replays_recorded_result_and_undoes() {
        let server = test_server();
        let id = ObjectId::new();
        let ops = vec![insert_dto(&id, &nil_id(), 9.0)];
        let drafted = preview(&server, 0, 6201, ops.clone()).expect("preview");
        let first =
            apply(&server, 0, 6201, ops.clone(), drafted.approval.clone()).expect("apply lands");
        assert_eq!(first.revision, 1);

        // Restart: the sealed pair lands in a fresh service via rehydrate.
        let (doc, journal) = {
            let state = server.lock().expect("test lock holds");
            (state.doc.clone(), state.journal.clone())
        };
        let fresh = test_server();
        fresh
            .lock()
            .expect("test lock holds")
            .rehydrate(doc, journal);

        // Same key plus same payload replays the recorded result without a
        // new revision; undo still works after rehydrate.
        let replayed = apply(&fresh, 0, 6201, ops, drafted.approval).expect("stored key replays");
        assert_eq!(replayed.revision, first.revision);
        assert_eq!(read(&fresh, None, None).revision, 1);
        let undone = TxUndoTool::invoke(&fresh, TxUndoArgs { undo_token: None })
            .expect("rehydrated undo works");
        assert_eq!(undone.revision, 2);
        assert_eq!(read(&fresh, None, None).revision, 2);
    }

    #[test]
    fn wrap_re_codes_new_family_with_data_preserved() {
        // The post-snapshot family (`bad_word`, `bad_wall`, `bad_receipt`,
        // `bad_brief`) layers a new code over a wire refusal without dropping
        // the culprit: `wrap` keeps message and data, and the result still
        // round-trips through the canonical `{code, message, data}` shape.
        for code in ["bad_word", "bad_wall", "bad_receipt", "bad_brief"] {
            let culprit = rmcp::serde_json::json!({"item": code});
            let wire = crate::discover::RpcError::new(code, format!("{code} probe"))
                .with_data(culprit.clone());
            let layered = McpToolError::wrap("subscribe_failed", wire);
            assert_eq!(layered.code(), "subscribe_failed");
            assert!(layered.message.contains(code));
            assert_eq!(layered.data(), Some(&culprit));
            let back = crate::discover::RpcError::from(layered);
            assert_eq!(back.code(), "subscribe_failed");
            assert_eq!(back.data, Some(culprit));
        }
    }
}

/// Annotations for read-only MCP tools: read-only on, open world off.
/// Owner of the pair every read-only tool's `annotations()` returns.
pub(crate) fn read_only_annotations() -> Option<ToolAnnotations> {
    let mut annotations = ToolAnnotations::default();
    annotations.read_only_hint = Some(true);
    annotations.open_world_hint = Some(false);
    Some(annotations)
}
