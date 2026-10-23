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

use crate::startup::StartupError;
use std::collections::BTreeSet;
use std::sync::{Mutex, MutexGuard};

use engine_editor::capability::{
    self, approve, canonical_action_hash, Approval, ApprovalLedger, Capability,
};
use engine_editor::commands::{self, Batch, Document, Journal, MaterialName, Op, ProvenanceStamp};
use engine_scene::component::{
    Component, ComponentKind, SCHEMA_VERSION as COMPONENT_SCHEMA_VERSION,
};
use engine_scene::ids::ObjectId;
use engine_scene::schema::{SchemaRegistry, VersionedComponent};
use engine_scene::transform::Transform;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, InitializeResult, JsonObject, Prompt, ProtocolVersion,
};
use rmcp::serde_json::Value;
use rmcp::service::{RequestContext, SubscriptionSink};
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

mod handler;
mod notes;

pub(crate) use self::handler::{read_only_annotations, write_annotations};
use self::notes::ProgressNote;
#[cfg(test)]
pub(crate) use self::notes::{is_long_call, progress_note_params};
#[cfg(test)]
use super::build_level::TOOL_BUILD_LEVEL;

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
/// Per-tool descriptions stay one line. On 2026-10-18 the spatial lane's schema docs and `scene.next` took the default list to 44347 and the `view`-loaded one to 48205: 43514 → 51000. Same day the SPR6 `blocked_by` and SPX3 `spatial_delta` schemas (and two `delta` flags) took the default list to 47111 and the loaded one to 51276: 51000 → 54000. On 2026-10-18 (backlog wave 2) five query kinds, role triage and verdict rows took the default list to 48292 and the loaded one to 52457: 54000 → 55000. On 2026-10-18 (endpoint wave) `scene.pick` added 2562 to each (default 50854, loaded 55019): 55000 → 57500. On 2026-10-18 the AX24 `tx.apply` change list (`ChangeListDto`: revisions, counts and rows) took the default list to 52348 and the loaded one to 56513 (988 headroom): 57500 → 59000. On 2026-10-19 SP62's two view-truth verbs (`view.observe`, `view.laws`: camera, per-body act/clip/tint/root/bounds/rect/cover and the laws line) took the loaded list to 58703 (297 headroom): 59000 → 61500. On 2026-10-19 PF24's opt-in `timing` receipt (a flag on both play args, plus label/p50/p99/samples on both play outputs: 931 chars) took the default list to 53873 and the loaded one to 60228: 61500 → 62500. On 2026-10-20 GS46's `world.query` (nested data/filter, strict, limit) plus the day's other verbs took the default list to 56345 and the loaded one to 62869: 62500 → 65000. Same day GS48's `ai.decisions` (show/diff over hex tapes) took the loaded one to 63830 (1170 headroom): 65000 → 66500. On 2026-10-21 the AI-native MCP lane landed D19's `verify` string on the three build outputs (+160 chars) and D17's `scene.diff` and `session.activity` in a pack that is not default-loaded (0 chars of the default list; resources and prompts cost 0 chars too): the default list measures 56897 (was 56737 at HEAD 37c33077) and the `view`-loaded one 64382 (was 64222; the view pack is untouched by this lane, its 7485 chars measured on both trees). The budget stays at 66500 with 2118 chars of headroom over the 2048 floor. On 2026-10-23 the default list measured 58112 (was 56897 on 2026-10-21) and the loaded ones 65601 (`view`) and 70454 (`gym`); the floor binds on the largest loadable pack, so 66500 → 73000 keeps 498 chars over the 2048 floor on `gym`, 5351 on `view` and 12840 on the default list. The default list is unchanged: a session pays the raise only when it asks for a big pack.
/// 2026-10-23 (later window): the same growth the door test measures (AD2's
/// apply receipt digests, SP66's dark/unknown rows, view.compare's diff image)
/// took the default list to 60,036 and the gym-loaded one to 72,378, over the
/// 70,952 ceiling the 73,000 budget leaves. 73,000 -> 75,000 keeps 2,622
/// chars over the 2,048 floor on the largest loadable pack.
/// 2026-10-23 (door budget): AX23's `ChangeListDto` was advertised twice, on
/// `tx.apply` and on `scene.query`; `scene.query` now names the shape, so the
/// default list measures 61,742 (view 70,653; gym 74,084). `api.load` admits a
/// pack while the raw list fits the 75,000 hard budget; the guardrail trims the
/// advertisement to 72,952, so a gym load advertises 68,506 with `sim.step`
/// hidden and still callable by exact name. The budget stays 75,000.
pub const TOOL_LIST_BUDGET_CHARS: usize = 75_000;

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
    ToolCostRow {
        name: tool.name.to_string(),
        chars: tool_cost_chars(tool),
    }
}

/// Discovery chars for one advertised tool: name, description, input and
/// output schemas as JSON text. The API6 guardrail budgets in this measure, so
/// an advertised list passes [`check_tool_budget`] by construction.
pub(crate) fn tool_cost_chars(tool: &rmcp::model::Tool) -> usize {
    fn schema_chars(schema: &rmcp::model::JsonObject) -> usize {
        rmcp::serde_json::to_string(schema).map_or(0, |text| text.len())
    }
    crate::discover::discovery_cost_chars(
        tool.name.len(),
        tool.description.as_deref().map_or(0, str::len),
        schema_chars(&tool.input_schema),
        tool.output_schema
            .as_ref()
            .map_or(0, |schema| schema_chars(schema)),
    )
}

#[cfg(test)]
pub(crate) fn hidden_tool_names(server: &McpServer) -> BTreeSet<String> {
    // The verbs the API6 guardrail hid from the advertisement: they sit on
    // the door's listed surface (the core plus the packs this session loaded)
    // and are absent from the advertised one. A hidden verb keeps its
    // definition and stays callable by exact name, so a test that wants such a
    // verb asserts reachability instead of advertisement.
    let listed: BTreeSet<String> = server
        .listed_tools()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect();
    let advertised: BTreeSet<String> = server
        .tools()
        .iter()
        .map(|tool| tool.name.to_string())
        .collect();
    listed.difference(&advertised).cloned().collect()
}

#[cfg(test)]
pub(crate) fn assert_advertised_or_reachable(server: &McpServer, name: &'static str) {
    // The API6 contract for one verb: it is advertised, or the guardrail hid
    // it and it stays reachable by exact name. A verb that is neither
    // advertised nor hidden is a real gap and fails here.
    if server.tools().iter().any(|tool| tool.name == name) {
        return;
    }
    assert!(
        hidden_tool_names(server).contains(name),
        "{name} is neither advertised nor hidden by the guardrail"
    );
    assert!(
        tool_names().contains(&name),
        "{name} is hidden and must stay reachable by exact name"
    );
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
    ApprovalDto, ChangeListDto, OpDto, PoseDto, SceneObjectDto, SceneReadArgs, SceneReadOutput,
    SpatialDeltaDto, TxApplyArgs, TxApplyOutput, TxPreviewArgs, TxPreviewOutput, TxUndoArgs,
    TxUndoOutput,
};

// ---------------------------------------------------------------------------
// Errors: machine code plus a message that always names the legal values.
// ---------------------------------------------------------------------------

// The refusal record - the type, its code tables and its wire payload - lives
// in `mcp_refusal`, so the advice contract has one owner. `McpToolError` keeps
// this path for every caller.
pub use self::mcp_refusal::{Kind, McpToolError, NextCall};

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
/// their canonical hashes match. An explicit origin names the writer:
/// `model:<provider>/<model>` carries the provider plus model and a nonzero
/// request digest over the canonical action hash, `script:<name>` carries its
/// own name, and no origin keeps the door's own `mcp` script stamp every
/// pre-provenance call meant. Crate-visible: the `scene.spatial` staging path
/// builds the same batch.
pub(crate) fn build_batch(
    expected_revision: u64,
    idempotency_key: u64,
    ops: &[OpDto],
    origin: Option<&str>,
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
    let mut batch = Batch {
        ops: converted,
        base_revision: expected_revision,
        idempotency_key,
        provenance: None,
    };
    batch.provenance = Some(stamp_for(origin, &batch)?);
    Ok(batch)
}

/// The writer a wire origin names.
enum OriginWriter {
    /// A model proposal: provider and model ids, never a prompt.
    Model { provider: String, model: String },
    /// Script or automation, named.
    Script(String),
}

/// Parse one wire origin. The two forms are `model:<provider>/<model>` and
/// `script:<name>`; a model id may itself carry `/` (`vendor/model`), so only
/// the first slash splits. Both names are bounded by the provenance codec, and
/// no other prefix is accepted, so a caller never stamps a source the journal
/// cannot store.
fn parse_origin(text: &str) -> Result<OriginWriter, McpToolError> {
    use engine_scene::provenance::MAX_TEXT_LEN;
    let refuse = || {
        McpToolError::new(
            "bad_origin",
            "origin must be `model:<provider>/<model>` or `script:<name>`, with both names \
             1..=256 bytes; omit it to stamp the door's own `script:mcp`",
        )
    };
    if text.len() > MAX_TEXT_LEN.saturating_mul(2) + "model:/".len() {
        return Err(refuse());
    }
    if let Some(rest) = text.strip_prefix("model:") {
        let (provider, model) = rest.split_once('/').ok_or_else(refuse)?;
        if provider.is_empty()
            || model.is_empty()
            || provider.len() > MAX_TEXT_LEN
            || model.len() > MAX_TEXT_LEN
        {
            return Err(refuse());
        }
        return Ok(OriginWriter::Model {
            provider: provider.to_owned(),
            model: model.to_owned(),
        });
    }
    if let Some(name) = text.strip_prefix("script:") {
        if name.is_empty() || name.len() > MAX_TEXT_LEN {
            return Err(refuse());
        }
        return Ok(OriginWriter::Script(name.to_owned()));
    }
    Err(refuse())
}

/// One batch's provenance stamp: the source the caller named plus, for a
/// model, a request digest over the canonical action hash. The digest names
/// the batch bytes only: never a prompt, credential or session secret.
fn stamp_for(origin: Option<&str>, batch: &Batch) -> Result<ProvenanceStamp, McpToolError> {
    Ok(match origin {
        Some(text) => match parse_origin(text)? {
            OriginWriter::Model { provider, model } => {
                let action = format!("{:016x}", canonical_action_hash(batch));
                ProvenanceStamp::model(provider, model, super::assistant::request_digest(&action))
            }
            OriginWriter::Script(name) => ProvenanceStamp::script(name),
        },
        None => ProvenanceStamp::script("mcp"),
    })
}

/// Compact read-side label for one stored provenance source.
fn origin_label(source: &engine_scene::provenance::Source) -> String {
    use engine_scene::provenance::Source;
    match source {
        Source::Human => "human".to_owned(),
        Source::Script(name) => format!("script:{name}"),
        Source::Model { provider, model } => format!("model:{provider}/{model}"),
    }
}

#[path = "mcp_refusal.rs"]
mod mcp_refusal;
pub(crate) use self::mcp_refusal::{batch_error, cap_error, granted_ops};

/// Map a gated-undo refusal to a tool error: the gate's own mapping for mask,
/// scope, budget and command refusals, plus the journal-level refusals a
/// grouped undo names (a pinned object, a receipt that left the journal).
pub(crate) fn undo_gate_error(
    error: &capability::UndoGateError,
    granted: &[&str],
    doc: &Document,
) -> McpToolError {
    match error {
        capability::UndoGateError::Cap(inner) => cap_error(inner, granted, doc),
        capability::UndoGateError::Pinned { id } => McpToolError::new(
            "pinned_object",
            format!(
                "node {id} was written by later work, so undoing this batch would clobber it; \
                 preview and apply a new batch instead"
            ),
        ),
        capability::UndoGateError::UnknownReceipt { key } => McpToolError::new(
            "unknown_receipt",
            format!(
                "no journaled batch carries session receipt {key}; \
                 it was already undone or evicted from the journal"
            ),
        ),
        capability::UndoGateError::NothingToUndo | capability::UndoGateError::InverseRefused => {
            McpToolError::new("nothing_to_undo", error.to_string())
        }
    }
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
    /// Packs this session loaded itself, oldest first, newest last. The API6
    /// guardrail spends its budget on the newest load first, so the pack a
    /// client just asked for is the one it sees advertised.
    pub(crate) load_order: Vec<&'static str>,
    /// BG1: the advertised pack set of the last answered call, so a call that
    /// moved it (`api.load`) carries one `tools/list_changed` and a call that
    /// did not stays silent.
    pub(crate) packs_pushed: BTreeSet<&'static str>,
    /// BG1: live `subscriptions/listen` streams that accepted the tool list,
    /// one per stream, so a load reaches a client on the stream it opened.
    pub(crate) list_subscribers: Vec<SubscriptionSink>,
    /// BG1: live streams that accepted the scene resource.
    pub(crate) scene_subscribers: Vec<SubscriptionSink>,
    /// BG1: the legacy `resources/subscribe` flag (protocols before
    /// 2026-07-28 have no listen stream); the note goes on the session peer.
    pub(crate) scene_subscribed: bool,
    /// BG1: the scene view last read or announced, so a scene update is sent
    /// only when the document actually moved.
    pub(crate) scene_pushed: Option<String>,
    /// The session's thread-confined rules worker (feature `rules`, off by default).
    #[cfg(feature = "rules")]
    pub(crate) rules: super::packs::rules::RulesSlot,
    /// The web pack's scope, runner, builds and undo stack (refused until the host allows web).
    pub(crate) web: super::packs::web::WebSlot,
    /// The settings pack's live knobs, replay tape and undo stack.
    pub(crate) settings: super::packs::physics_settings::SettingsSlot,
    /// D17: the session's scene snapshots, the revisions a `scene.diff` can
    /// be taken against.
    pub(crate) snapshots: super::packs::session::Snapshots,
    pub(crate) reads: super::guards::ReadClock,
    /// The baked nav mesh of the staged floor, when the host holds one. Without
    /// it `scene.query reachable` refuses `needs_nav_view` instead of guessing.
    pub(crate) nav: Option<engine_nav::mesh::NavMesh>,
    /// DOOR2: the project folder `scene.save` seals into and `scene.load` reads
    /// from; `None` refuses both verbs as `no_project_dir`.
    pub(crate) project_dir: Option<std::path::PathBuf>,
    /// AD12: the folder's single-writer lease, held while this server lives.
    pub(crate) project_lease: Option<crate::startup::ProjectLease>,
    /// DOOR1: the game parts the newest `level.build` placed, so `game.play` plays the
    /// level a caller just built (the document holds poses, not role names).
    #[cfg(feature = "rules")]
    pub(crate) game_parts: Vec<super::build_level::GamePart>,
    /// DOOR1: the scene revision the newest `level.build` landed at; a `game.play`
    /// against an older one plays the parts of a level the scene has since replaced.
    #[cfg(feature = "rules")]
    pub(crate) game_rev: u64,
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
        // D17: the session opens with a snapshot of the revision it started
        // at, so a client that read the scene once can ask for the deltas
        // since that read and pays for the delta, not for the scene.
        let mut snapshots = super::packs::session::Snapshots::new();
        snapshots.take(doc.revision);
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
            load_order: Vec::new(),
            packs_pushed: registry.default_loaded(),
            list_subscribers: Vec::new(),
            scene_subscribers: Vec::new(),
            scene_subscribed: false,
            scene_pushed: None,
            #[cfg(feature = "rules")]
            rules: super::packs::rules::RulesSlot::default(),
            web: super::packs::web::WebSlot::default(),
            settings: super::packs::physics_settings::SettingsSlot::default(),
            snapshots,
            reads: super::guards::ReadClock::new(),
            nav: None,
            project_dir: None,
            project_lease: None,
            #[cfg(feature = "rules")]
            game_parts: Vec::new(),
            #[cfg(feature = "rules")]
            game_rev: 0,
        }
    }

    /// The engine session id approvals bind to (never the protocol's).
    pub fn session(&self) -> u64 {
        self.session
    }

    /// D17: the session's scene snapshots, so `scene.diff` can answer the
    /// deltas since a named snapshot instead of the whole scene again.
    pub fn snapshots(&self) -> &super::packs::session::Snapshots {
        &self.snapshots
    }

    /// D17: take a snapshot of the current revision and return its id.
    ///
    /// A landed write and an undo both call this, so the snapshot table
    /// holds the revisions a diff can be taken against.
    pub fn take_snapshot(&mut self) -> u64 {
        self.snapshots.take(self.doc.revision)
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
        // The grouped undo groups this session's own live receipts; a
        // rehydrated journal holds work from before the restart, not this
        // token's in-memory batches.
        self.cap.clear_receipts();
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
                origin: commands::chain_for(&self.journal, *id)
                    .last()
                    .map(|record| origin_label(&record.source)),
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
        let batch = build_batch(
            args.expected_revision,
            args.idempotency_key,
            &args.ops,
            None,
        )?;
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
        let batch = build_batch(
            args.expected_revision,
            args.idempotency_key,
            &args.ops,
            args.origin.as_deref(),
        )?;
        let caps = super::explain::ExplainCaps::default_caps();
        if let Some(refusal) =
            super::tool_capabilities::refuse_write(&self.doc, &batch, &self.cap, &caps)
        {
            return Err(refusal);
        }
        super::explain::check_batch(&self.doc, &batch, &caps)
            .map_err(|refusal| refusal.to_tool_error(&caps))?;
        // Witnessed-plan gate: bind this batch to the observed digest plus revision
        // before the gated apply, then seal the receipt after it lands (same lock, no
        // interleaved write, so the gate is fresh; ledger replays keep the retained receipt).
        let pre_revision = self.doc.revision;
        // One before snapshot serves the change list and, when asked, the
        // spatial delta: a landed write takes one pair.
        let before = super::scene_tools::snapshot(&self.doc);
        let proposal = super::door_dispatch::witness_proposal_for_apply(self, &batch)?;
        let approval = Approval {
            session: self.session,
            revision: args.approval.revision,
            action_hash: args.approval.action_hash,
            expiry_tick: args.approval.expiry_tick,
        };
        let granted = granted_ops(&self.cap);
        let applied = match self.project_dir.clone() {
            // AD12: a door with a project folder seals before it answers;
            // the in-memory path stays for test doors.
            Some(dir) => super::startup::apply_durable(self, &batch, &approval, &granted, &dir)?,
            None => capability::apply_gated(
                &mut self.doc,
                &mut self.journal,
                &batch,
                &mut self.cap,
                &approval,
                &mut self.ledger,
            )
            .map_err(|error| cap_error(&error, &granted, &self.doc))?,
        };
        // A landed write is a watch event: the digest cache moves, the hub fans it out.
        self.refresh_watch();
        // D17: the landed revision is a snapshot a diff can be taken against.
        self.take_snapshot();
        super::door_dispatch::witness_seal_apply(self, pre_revision, &proposal)?;
        let after = super::scene_tools::snapshot(&self.doc);
        // What moved and which edges changed; a replayed key rebuilds its
        // original pair from the journal instead of the unmoved live one.
        let changes = super::scene_query::diff::applied_change(
            &self.doc,
            &self.journal,
            &applied,
            pre_revision,
            &before,
            &after,
        );
        let spatial_delta = (args.delta != Some(false)).then(|| {
            super::scene_tools::spatial_delta(&before, &after, &super::scene_tools::op_ids(&batch))
        });
        // Echo the stored stamp, not the request: a call that sent no origin
        // answers `script:mcp`, the source the journal actually holds.
        let origin = origin_label(&applied.provenance.source);
        Ok(TxApplyOutput {
            revision: applied.revision,
            effects: applied.effects,
            key: applied.key,
            journal_index: applied.journal_index as u64,
            before_digest: crate::play::hex_bytes(&applied.before_digest),
            staged_digest: crate::play::hex_bytes(&applied.staged_digest),
            after_digest: crate::play::hex_bytes(&applied.after_digest),
            undo_token: applied.key,
            changes,
            spatial_delta,
            origin,
        })
    }

    /// Undo the newest journaled batch as a new revision, through the gated
    /// path: the session token must grant the op class of the action being
    /// undone (an INSERT-only token cannot revert a DELETE), every touched
    /// node must be in its scope, and its budget must cover the inverse ops.
    /// A given token must match the newest batch key, so a client cannot undo
    /// another writer's edit by accident. Refusals write nothing.
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
        let granted = granted_ops(&self.cap);
        let applied = match self.project_dir.clone() {
            // AD12: a durable door seals the reverting state too.
            Some(dir) => super::startup::undo_durable(self, &granted, &dir)?,
            None => capability::undo_gated(&mut self.doc, &mut self.journal, &mut self.cap)
                .map_err(|error| undo_gate_error(&error, &granted, &self.doc))?,
        };
        // Undo publishes a new revision, so the watch cache moves with it.
        self.refresh_watch();
        // D17: the undone revision is a snapshot a diff can be taken against.
        self.take_snapshot();
        Ok(TxUndoOutput {
            revision: applied.revision,
            undone_key: applied.key,
            effects: applied.effects,
        })
    }

    /// MCP2 headless arena match job: `start` simulates a bounded match and
    /// files a handle, `poll` walks progress to the checkpoints, `cancel`
    /// stops a running handle. The scene document is untouched. A start must
    /// pin the scene it plays (`scene_rev`, the current revision): an
    /// unpinned, empty (revision 0) or stale pin is refused before any
    /// simulation, and the job manifest carries the pin's revision and digest.
    pub fn play_run(&mut self, args: &PlayRunArgs) -> Result<PlayRunOutput, McpToolError> {
        let world = match args.action.as_str() {
            "start" => Some(super::scene_tools::pinned_world(&self.doc, args.scene_rev)?),
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

// The rmcp tool shapes live in `mcp_tools` (structure-only split, behaviour
// identical), beside the `mcp_dto` shapes they schematize.
#[path = "mcp_tools.rs"]
mod mcp_tools;

use self::mcp_tools::sanitize_tool_schema;
pub use self::mcp_tools::{
    GameBuildArgs, GameBuildOutput, GameBuildTool, SceneReadTool, SimBuildEncounterArgs,
    SimBuildEncounterOutput, SimBuildEncounterTool, TxApplyTool, TxPreviewTool, TxUndoTool,
};

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

    /// Published skill prompts, sorted by name. These cost no tool budget.
    #[must_use]
    pub fn prompt_list(&self) -> Vec<Prompt> {
        let mut prompts = crate::skills::SkillRegistry::shipped().prompts();
        // D16: the five scenario prompts sit after the skills, in contract
        // order, so a client lists them the same way every build.
        prompts.extend(crate::scenarios::prompts());
        prompts
    }

    /// One prompt body for `prompts/get`; `None` for an unknown name.
    ///
    /// The skills answer by their own name; the D16 scenarios answer by
    /// theirs.
    #[must_use]
    pub fn prompt_text(&self, name: &str) -> Option<String> {
        if let Some(body) = crate::scenarios::body(name) {
            return Some(body.to_owned());
        }
        crate::skills::SkillRegistry::shipped()
            .body(name)
            .map(str::to_owned)
    }

    /// Session over a fully built state.
    pub fn with_state(mut state: ServerState) -> Self {
        let tool_router = Self::router();
        super::packs::seed_core_index(&mut state.insight, &tool_router.list_all());
        Self {
            state: Mutex::new(state),
            tool_router,
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
    /// sorted, then the API6 guardrail. A poisoned session lock advertises the
    /// core alone (fail closed); every verb stays callable by name either way.
    pub fn tools(&self) -> Vec<rmcp::model::Tool> {
        match self.lock() {
            Ok(state) => {
                let listed = self.tools_with(&state.loaded_packs);
                let order = state.load_order.clone();
                crate::collapse::guard(listed, &order)
            }
            Err(_) => crate::collapse::guard(self.core_tools(), &[]),
        }
    }

    /// Advertised tools before the API6 guardrail: the core plus the packs
    /// this session loaded, name sorted. [`Self::surface_chars`] measures this
    /// surface, so the growing contract stays measurable.
    fn listed_tools(&self) -> Vec<rmcp::model::Tool> {
        let loaded = self
            .lock()
            .map(|state| state.loaded_packs.clone())
            .unwrap_or_default();
        self.tools_with(&loaded)
    }

    /// Packs this session loaded itself, oldest first, newest last. Read-only.
    pub fn load_order(&self) -> Vec<&'static str> {
        self.lock()
            .map(|state| state.load_order.clone())
            .unwrap_or_default()
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

    /// The door reference rows: one row per reachable tool, with its pack
    /// and capability card, in name order.
    ///
    /// D16: the `door-reference` resource reads these rows, so the served
    /// reference cannot drift from the router.
    #[must_use]
    pub fn tool_reference_rows(&self) -> Vec<(String, String, String)> {
        let registry = PackRegistry::shipped();
        let mut rows: Vec<(String, String, String)> = self
            .all_tools()
            .iter()
            .map(|tool| {
                let pack = registry
                    .verb(&tool.name)
                    .map_or("core", super::packs::VerbSpec::pack);
                let card = crate::tool_capabilities::card_for_tool(&tool.name);
                (
                    tool.name.clone().into_owned(),
                    pack.to_owned(),
                    card.name().to_owned(),
                )
            })
            .collect();
        rows.sort();
        rows
    }

    /// Dispatch one `tools/call` by exact name: core tools through the
    /// router, pack verbs through the registry (loaded or not), anything
    /// else through the audited name gate (`unknown_tool`, fail closed).
    async fn call_by_name(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        // D16: progress notes. A client that sent a progress token gets one
        // note before a long call and one after it, so a slow build reads as
        // progress and not as a hang.
        let call_name = request.name.clone();
        let note = ProgressNote::for_call(&call_name, &context);
        if let Some(note) = &note {
            note.send(0.0, &format!("{call_name}: started")).await;
        }
        // BG1: the dispatch queues its change notes; they leave before the
        // answer, so a client that re-lists on the note sees the new surface.
        let notices = context.clone();
        let answer = self.dispatch_call(request, context).await;
        self.flush_notices(&notices).await;
        if let Some(note) = &note {
            note.send(1.0, &format!("{call_name}: done")).await;
        }
        answer
    }

    /// The body of [`Self::call_by_name`]: the dispatch itself.
    async fn dispatch_call(
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
    /// schemas, over the advertised list, so the ledger gate measures exactly
    /// what a client is handed. The per-tool rows in [`Self::tool_cost_table`]
    /// sum to this total; [`Self::surface_chars`] measures the untrimmed
    /// surface beside it.
    pub fn tool_list_budget_chars(&self) -> usize {
        self.tool_cost_table().iter().map(|row| row.chars).sum()
    }

    /// Discovery cost in chars of the surface before the API6 guardrail: what
    /// this session would advertise if the ledger had no ceiling. Read-only;
    /// the guardrail trims to [`crate::collapse::MAX_ADVERTISED_CHARS`] and
    /// this number is what proves the contract still grows.
    pub fn surface_chars(&self) -> usize {
        self.listed_tools()
            .iter()
            .map(|tool| tool_cost_row(tool).chars)
            .sum()
    }

    /// Discovery cost in chars of the surface that the given loaded packs
    /// would advertise, before the API6 guardrail. Takes no lock, so a caller
    /// may hold the session lock while it measures a load it has not made yet.
    pub(crate) fn surface_chars_with(&self, loaded: &BTreeSet<&'static str>) -> usize {
        self.tools_with(loaded).iter().map(tool_cost_chars).sum()
    }

    /// Advertised tool count for the given loaded packs, before the API6
    /// guardrail trims. Takes no lock, like [`Self::surface_chars_with`], so
    /// `door.status` can measure the session it already holds.
    pub(crate) fn tool_count_with(&self, loaded: &BTreeSet<&'static str>) -> usize {
        self.tools_with(loaded).len()
    }

    /// Per-tool discovery cost table, one row per advertised tool, in router
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
    pub async fn run_stdio(self) -> Result<String, StartupError> {
        use rmcp::{transport::io::stdio, ServiceExt};
        let running = Box::pin(self.serve(stdio()))
            .await
            .map_err(|error| StartupError::transport("serve failed", &error))?;
        let reason = running
            .waiting()
            .await
            .map_err(|error| StartupError::transport("server task failed", &error))?;
        Ok(format!("{reason:?}"))
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
mod receipt_tests;
#[cfg(test)]
mod tests;
