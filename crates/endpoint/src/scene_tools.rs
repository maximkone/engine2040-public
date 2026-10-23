//! MCP3 scene tools: `scene.brief`, `scene.spatial` and `scene.place`.
//!
//! Consumer: the LG1 variant loop and the A6 spatial proof, over any MCP
//! client. Data owner stays the editor command service
//! (`engine-editor::commands` plus `engine-editor::capability`); this module
//! owns only MCP DTOs, validation and the scene-to-actor mapping. Spatial
//! computation (brief text, checks, intent solving) is `engine-spatial`'s;
//! placement application goes through the command service exactly like
//! `tx.apply` (validate preview, single-use CAP1 approval, atomic gated
//! apply), never through direct scene writes.
//!
//! Mapping convention (shared with the CAP1 staged gate): every document
//! node with a finite world position becomes one spatial actor, a box
//! centered on its world position (sized from its collider component, else a
//! 1 m cube). Node order is the id-text sort, so the
//! same scene always maps to the same actor ids; the wire carries the
//! index-to-UUID table alongside every numeric finding, plus one
//! `name [id]` label per object (`lamp_2 [<uuid>]`, blender-mcp style)
//! so duplicate display names never mis-place: the bracketed id is the only
//! resolution key and `scene.place` accepts either the bare UUID or the full
//! label. Actor names are `<mesh asset>_<n>` (`prop_<n>` without a mesh) with
//! `n` counted per name, so an unrelated insert never relabels a node; stable
//! identity still travels in the id table, not the name. Yaw
//! round-trips through the world quaternion: snapshot extracts the Y yaw,
//! place writes a pure-Y rotation. `scene.place` moves roots only: without a
//! public world-to-local helper in the scene crate the endpoint cannot
//! retarget a parented node exactly, so a parented object fails loudly
//! instead of landing somewhere approximate.
//!
//! MCP8 guards: scene applies are single-flight (one at a time, try-lock
//! only, so a second apply fails fast instead of queueing and `play.run`
//! never stalls on a scene write), scene-tool writes allow-list to the `move`
//! op only (the endpoint instantiates, deletes and reparents nothing; general
//! edits belong to `tx.preview` / `tx.apply`), and every refusal names the
//! violated invariant plus one recovery.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard};

use engine_editor::capability::{apply_gated, approve};
use engine_editor::commands::{self, Batch, Document, Op, ProvenanceStamp};
use engine_scene::codec::Scene;
use engine_scene::ids::ObjectId;
use engine_scene::pin::{PinError, ScenePin};
use engine_scene::transform::Transform;
use engine_spatial::{Intent, ObjectRef, Relation, Target, Wall};

use rmcp::handler::server::router::tool::{SyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::injection::score::import_screen;
use super::injection::Category;
use super::mcp::{
    batch_error, cap_error, granted_ops, McpServer, McpToolError, ServerState, SpatialDeltaDto,
};
use super::play::world_load::match_world;
use super::play::PinnedWorld;
use super::scene_query::QueryFormat;

mod facing_wants;
#[cfg(test)]
mod handle_tests;
mod labels;
mod place_blocks;
#[cfg(test)]
mod place_facing_tests;
mod read_guard;
pub(crate) mod refusals;
mod spatial_checks;
mod spatial_delta;
#[cfg(test)]
pub(crate) mod spatial_verdict_tests;
mod spatial_verdicts;
use crate::spatial_glue::{conflict_texts, quat_of_yaw, ParsedWant};
pub use labels::object_label;
use place_blocks::block_output;
pub use place_blocks::WantBlockDto;
pub(crate) use read_guard::no_repeat;
pub use read_guard::BriefSpatial;
use spatial_checks::{index_of, partition, table_allowance, table_page, wide, worst_actors};
/// Live document snapshot, sized by the one scene adapter
/// ([`engine_editor::spatial_view`]); the body sits with the spatial read path.
pub(crate) use spatial_checks::{snapshot, Snapshot};
pub use spatial_checks::{SizeSource, SkippedCheck, SpatialCounts, WorstActor};
pub(crate) use spatial_delta::{change_list, op_ids, spatial_delta};
use spatial_verdicts::{check_epsilon, staged_roles, view, Asks};

/// MCP tool name for the bounded scene brief (read-only).
pub const TOOL_SCENE_BRIEF: &str = "scene.brief";
/// MCP tool name for the spatial findings page (read-only).
pub const TOOL_SCENE_SPATIAL: &str = "scene.spatial";
/// MCP tool name for relation-intent placement through the command gate.
pub const TOOL_SCENE_PLACE: &str = "scene.place";

/// Default `budget_bytes` for `scene.brief`: bytes of brief text, digest line always kept.
pub const SCENE_BRIEF_DEFAULT_BUDGET: u32 = 4096;
/// Largest `budget_bytes` for `scene.brief`: bigger budgets are refused with `bad_budget`.
pub const SCENE_BRIEF_MAX_BUDGET: u32 = 16384;
/// Default `limit` for `scene.spatial`: findings per page.
pub const SCENE_SPATIAL_DEFAULT_LIMIT: u32 = 64;
/// Largest `limit` for `scene.spatial`: bigger pages are refused with `bad_limit`.
pub const SCENE_SPATIAL_MAX_FINDINGS: u32 = 256;
/// Most check names per `scene.spatial` call; more are refused with `too_many_checks`.
pub const SCENE_SPATIAL_MAX_CHECKS: usize = 32;
/// Most want lines per `scene.place` call; more are refused with `too_many_wants`.
pub const MAX_PLACE_WANTS: usize = 16;
/// Longest single want line in chars; longer lines are refused with `bad_want`.
pub const MAX_WANT_CHARS: usize = 256;

// ---------------------------------------------------------------------------
// MCP8 guards: `name [id]` labels, the scene-op allow-list and the
// single-flight apply lock (patterns from blender-mcp, godot-mcp and
// Unreal_mcp, reimplemented here, never vendored).
// ---------------------------------------------------------------------------

/// Split a `name [id]` label back to its id. Accepts a bare UUID too, so
/// older clients keep working; the name part (if any) is ignored and the
/// bracketed id decides. Anything else names the expected shapes.
pub fn parse_object_ref(field: &str, text: &str) -> Result<ObjectId, McpToolError> {
    let trimmed = text.trim();
    if let Some((_, bracket)) = trimmed.rsplit_once('[') {
        if let Some(id_text) = bracket.strip_suffix(']') {
            return ObjectId::parse(id_text.trim()).map_err(|_| {
                McpToolError::new(
                    "bad_id",
                    format!(
                        "{field} `{text}` holds bracket text that is not an object id \
                         (expected `name [uuid]` or bare UUID text; re-read scene.brief \
                         for the staged labels)"
                    ),
                )
            });
        }
    }
    ObjectId::parse(trimmed).map_err(|_| {
        McpToolError::new(
            "bad_id",
            format!(
                "{field} `{text}` is not an object id (expected bare UUID text or \
                 `name [uuid]`; re-read scene.brief for the staged labels)"
            ),
        )
    })
}

/// The only scene-tool write operation: a relation-intent root move through
/// `scene.place`. Scene tools instantiate nothing, delete nothing and
/// reparent nothing; general edits belong to `tx.preview` / `tx.apply`
/// behind their own CAP1 gate.
pub const SCENE_PLACE_OP_MOVE: &str = "move";
/// Allow-list of scene operations, by exact name.
pub const SCENE_ALLOWED_OPS: &[&str] = &[SCENE_PLACE_OP_MOVE];

/// Allow-list check for one scene op name. Allowed names pass; anything else
/// is refused with the legal set plus one recovery attached.
pub fn check_scene_op(op: &str) -> Result<(), McpToolError> {
    if SCENE_ALLOWED_OPS.contains(&op) {
        return Ok(());
    }
    Err(McpToolError::new(
        "denied_op",
        format!(
            "scene op `{op}` is denied; allowed scene ops: [{}] \
             (scene tools move staged roots only and instantiate nothing; \
             submit a move intent through scene.place, or a general edit \
             through tx.preview)",
            SCENE_ALLOWED_OPS.join(", ")
        ),
    ))
}

/// Batch allow-list for the scene write path: every op in a scene-tool batch
/// must be a root move (`SetLocal`). Insert, remove, reparent and material
/// binds are refused with the legal operation plus one recovery, so no scene
/// tool can ever instantiate, delete, reparent or rebind a node no matter
/// who calls it.
fn check_batch_allowed(batch: &Batch) -> Result<(), McpToolError> {
    for op in &batch.ops {
        match *op {
            Op::SetLocal { .. } => {}
            Op::Insert { id, .. } => {
                return Err(McpToolError::new(
                    "denied_op",
                    format!(
                        "scene op `insert` for node {id} is denied; allowed scene ops: [{}] \
                         (scene tools move staged roots only and instantiate nothing; \
                         submit a move intent through scene.place, or a general edit \
                         through tx.preview)",
                        SCENE_ALLOWED_OPS.join(", ")
                    ),
                ));
            }
            Op::Remove { id } => {
                return Err(McpToolError::new(
                    "denied_op",
                    format!(
                        "scene op `remove` for node {id} is denied; allowed scene ops: [{}] \
                         (scene tools move staged roots only and delete nothing; \
                         submit a move intent through scene.place, or a general edit \
                         through tx.preview)",
                        SCENE_ALLOWED_OPS.join(", ")
                    ),
                ));
            }
            Op::Reparent { id, .. } => {
                return Err(McpToolError::new(
                    "denied_op",
                    format!(
                        "scene op `reparent` for node {id} is denied; allowed scene ops: [{}] \
                         (scene.place moves roots only and reparents nothing; \
                         reparent through tx.preview, then place again)",
                        SCENE_ALLOWED_OPS.join(", ")
                    ),
                ));
            }
            Op::SetMaterial { id, .. } => {
                return Err(McpToolError::new(
                    "denied_op",
                    format!(
                        "scene op `set_material` for node {id} is denied; allowed scene ops: [{}] \
                         (scene.place moves roots only and binds nothing; \
                         bind materials through tx.preview, then place again)",
                        SCENE_ALLOWED_OPS.join(", ")
                    ),
                ));
            }
            Op::SetComponent { id, .. } | Op::RemoveComponent { id, .. } => {
                return Err(McpToolError::new(
                    "denied_op",
                    format!(
                        "scene op `component` for node {id} is denied; allowed scene ops: [{}] \
                         (scene.place moves roots only and edits no components; \
                         edit components through tx.preview, then place again)",
                        SCENE_ALLOWED_OPS.join(", ")
                    ),
                ));
            }
            Op::SetBond { id, .. } => {
                return Err(McpToolError::new(
                    "denied_op",
                    format!(
                        "scene op `bond` for node {id} is denied; allowed scene ops: [{}] \
                         (scene.place moves roots only and paints no bonds; \
                         author fracture bonds through tx.preview)",
                        SCENE_ALLOWED_OPS.join(", ")
                    ),
                ));
            }
            Op::PutMaterial { .. } | Op::SetMaterialParam { .. } | Op::RemoveLastMaterial => {
                return Err(McpToolError::new(
                    "denied_op",
                    format!(
                        "scene op `material_table` is denied; allowed scene ops: [{}] \
                         (scene.place moves roots only and edits no material records; \
                         edit records through tx.preview, then place again)",
                        SCENE_ALLOWED_OPS.join(", ")
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Single-flight guard for scene applies: at most one scene write stages or
/// applies at a time, process-wide. Acquired with `try_lock` only, so a
/// second apply fails fast with `apply_in_flight` instead of queueing behind
/// the first. `play.run`, `capture` and every read-only tool never take this
/// lock, so a scene apply can never stall them.
pub(crate) static SCENE_APPLY_LOCK: Mutex<()> = Mutex::new(());

/// Acquire the single-flight scene-apply guard without blocking. A busy
/// guard answers `apply_in_flight` (retry after the first apply lands); a
/// poisoned guard fails closed with `lock_poisoned`.
pub(crate) fn try_scene_apply_guard() -> Result<MutexGuard<'static, ()>, McpToolError> {
    SCENE_APPLY_LOCK.try_lock().map_err(|error| match error {
        std::sync::TryLockError::WouldBlock => McpToolError::new(
            "apply_in_flight",
            "another scene apply is already in flight; retry after it lands \
             (single-flight guard: one scene apply at a time; reads and play.run \
             never wait on it)",
        ),
        std::sync::TryLockError::Poisoned(_) => McpToolError::new(
            "lock_poisoned",
            "scene apply lock poisoned by a failed call; restart the server",
        ),
    })
}

/// Pin the scene a `play.run` start plays, or refuse before any simulation.
///
/// `revision` is what the caller offered (`scene_rev`); the live pin is the
/// document's current revision plus the canonical-bytes digest. An empty
/// arena (revision 0) never pins, an absent revision is `unpinned_run` and a
/// moved revision is `stale_scene_rev`. The returned live pin is what the run
/// manifest carries. Pure; the document is only read.
pub(crate) fn run_pin(doc: &Document, revision: Option<u64>) -> Result<ScenePin, McpToolError> {
    let live = Scene::encode(&doc.hierarchy)
        .pin(doc.revision)
        .map_err(pin_refusal)?;
    let offered = ScenePin::require(revision.map(|revision| ScenePin {
        revision,
        ..live.clone()
    }))
    .map_err(pin_refusal)?;
    offered
        .check(live.revision, &live.digest)
        .map_err(pin_refusal)?;
    Ok(live)
}

/// The match world a `play.run` start plays: the pinned scene (see
/// [`run_pin`]) built into a world, with the pin digest for the manifest.
pub(crate) fn pinned_world(
    doc: &Document,
    revision: Option<u64>,
) -> Result<PinnedWorld, McpToolError> {
    let pin = run_pin(doc, revision)?;
    Ok(PinnedWorld {
        world: match_world(doc)?,
        digest: pin.digest,
    })
}

/// Door code for each pin refusal; the message is the pin's own recovery.
fn pin_refusal(error: PinError) -> McpToolError {
    let code = match error {
        PinError::MissingPin => "unpinned_run",
        PinError::EmptyArena => "empty_arena",
        PinError::StalePin { .. } => "stale_scene_rev",
        PinError::DigestMismatch => "scene_digest_drift",
    };
    McpToolError::new(code, error.to_string())
}

/// Arguments for `scene.brief`: optional brief-text budget in bytes.
/// `None` selects [`SCENE_BRIEF_DEFAULT_BUDGET`]; `0` or above
/// [`SCENE_BRIEF_MAX_BUDGET`] fails with `bad_budget`. Read-only.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SceneBriefArgs {
    /// Brief-text budget in bytes (1..=[`SCENE_BRIEF_MAX_BUDGET`]).
    pub budget_bytes: Option<u32>,
    /// First object index of the `ids`/`objects`/`poses` page (default 0);
    /// pass the previous reply's `next_cursor` to read on.
    pub cursor: Option<u32>,
    /// True adds `spatial`: the verdict, finding count and hash of a
    /// default `scene.spatial` call, plus the findings the text left out.
    pub include_spatial: Option<bool>,
}

/// Output of `scene.brief`: document revision, spatial digest, bounded text rows,
/// and one page of the index-to-UUID table, the index-aligned `name [id]`
/// labels and the pose rows. The page holds about a quarter of the text
/// budget in bytes; `next_cursor` continues it. Read-only: nothing is staged,
/// applied or locked.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SceneBriefOutput {
    /// Document revision the brief was read at.
    pub revision: u64,
    /// Spatial graph digest over the snapshot (budget-independent). It counts
    /// nodes and edges, so any insert moves it; a per-object `#digest` in the
    /// place rows (and `poses`) is the stable one.
    pub digest: u64,
    /// Brief text rows within the budget, always starting with the digest line:
    /// extent and problem rows, folded look-alike rows, then one place row per
    /// object (`lamp_2 [index] x y z sx sy sz dir #digest`, whole cm) while it
    /// fits, then groups, relations and findings.
    pub text: String,
    /// True when groups, relations or findings were cut to fit the budget.
    pub truncated: bool,
    /// Index-to-UUID table page: bare UUID text per staged object, id-text sorted.
    pub ids: Vec<String>,
    /// MCP8 `name [id]` labels (`lamp_2 [<uuid>]`), index-aligned with
    /// `ids`: the bracketed id resolves, the name never does.
    pub objects: Vec<String>,
    /// One pose row per object, aligned with `ids`:
    /// `name [index] x y z sx sy sz dir #digest`, whole cm, a compass facing
    /// and the object's own place digest (stable when another object moves).
    pub poses: Vec<String>,
    /// First object index of the next page; absent on the last page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<u32>,
    /// Spatial summary, when `include_spatial` was true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spatial: Option<BriefSpatial>,
    /// How many times in a row this exact call was made (2..=4); absent on a first call.
    #[serde(default, skip_serializing_if = "no_repeat")]
    pub repeat_count: u32,
    /// Session-local latest watch digest, when a `scene.watch` is live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watch_digest: Option<u64>,
    /// Server-owned live subscription id, when a `scene.watch` is live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watch_subscription: Option<u64>,
}

/// Arguments for `scene.spatial`: optional check filter plus a findings page limit.
/// `None` checks run every offline check with [`SCENE_SPATIAL_DEFAULT_LIMIT`]
/// rows. Unknown names fail with `bad_check`, nav-only names with
/// `needs_nav_view`, over-long lists with `too_many_checks`, and a `0` or
/// over-[`SCENE_SPATIAL_MAX_FINDINGS`] limit with `bad_limit`. Read-only.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SceneSpatialArgs {
    /// Checks to run, at most [`SCENE_SPATIAL_MAX_CHECKS`] (`None` = every
    /// offline check); opt-in names run only when listed here.
    pub checks: Option<Vec<String>>,
    /// Findings rows per page, 1..=[`SCENE_SPATIAL_MAX_FINDINGS`].
    pub limit: Option<u32>,
    /// First object index of the `ids`/`objects` page (default 0).
    pub cursor: Option<u32>,
    /// Answer shape: rows (default), ids (only the objects that carry a
    /// finding, no finding rows), count (verdict and counts only).
    pub format: Option<String>,
    /// True keeps only red findings (rows, total and ids).
    pub matched_only: Option<bool>,
    /// Z-fight tolerance in cm, 0..=50, default 1 (zfight).
    pub epsilon_cm: Option<i32>,
    /// True judges overlaps by component role (collider on its mesh is intentional).
    pub triage: Option<bool>,
    /// Verdict rows, max 256: `check a b|- verdict rev expiry reason` (uuids, 0 = none). Matched findings move to `suppressed`.
    pub allow: Option<Vec<String>>,
}

/// Output of `scene.spatial`: verdict, spatial hash, paged brief-style finding
/// lines, the id table plus `name [id]` labels, the checks that ran and the
/// catalog checks the door could not drive, and each node's size source
/// ([`Self::checks_run`], [`Self::skipped`], [`Self::size_source`]). Read-only.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SceneSpatialOutput {
    /// Document revision read.
    pub revision: u64,
    /// `clean`, `warn` or `red`.
    pub verdict: String,
    /// Hash of the full finding set, page-independent.
    pub hash: u64,
    /// Finding count across all pages.
    pub total: u64,
    /// Findings were cut to the page limit.
    pub truncated: bool,
    /// Finding lines, `finding <check> <a> <b|-> <severity> <pct>`.
    pub findings: Vec<String>,
    /// One page of bare UUID text per staged object, id-text sorted.
    pub ids: Vec<String>,
    /// `name [id]` labels, index-aligned with `ids`.
    pub objects: Vec<String>,
    /// First object index of the next `ids`/`objects` page; absent on the last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<u32>,
    /// How many times in a row this exact call was made (2..=4); absent on a first call.
    #[serde(default, skip_serializing_if = "no_repeat")]
    pub repeat_count: u32,
    /// Finding counts: total, returned in this reply, suppressed.
    pub counts: SpatialCounts,
    /// The ten actors with the most findings, across all pages.
    pub worst_actors: Vec<WorstActor>,
    /// Checks that ran, catalog order.
    pub checks_run: Vec<String>,
    /// Catalog checks that did not run, each with one reason.
    pub skipped: Vec<SkippedCheck>,
    /// Where each staged node's size came from.
    pub size_source: SizeSource,
    /// One line per node on this page whose box is the fallback, not a
    /// collider measurement: `label x.xx x.xx x.xx m: default size ...`.
    /// Empty when every box on the page was measured.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_sized: Vec<String>,
    /// Findings set aside by an allow row or the learner, with the reason.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suppressed: Vec<String>,
    /// Light reach (SP66): nodes at least one light covers, counted, and the
    /// indexes of the nodes no light reaches — `dark` and `unknown` apart.
    /// Absent when the document carries no light.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<LightReachOut>,
}

/// `scene.spatial` light reach (SP66); indexes are into [`SceneSpatialOutput::ids`]
/// (id-text order), ascending.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LightReachOut {
    /// Nodes at least one light covers.
    pub lit: u32,
    /// Nodes judged (a node with no readable pose is not judged).
    pub judged: u32,
    /// Indexes no light reaches: dark.
    pub dark: Vec<u32>,
    /// Indexes no light reaches while a range was unreadable.
    pub unknown: Vec<u32>,
}

impl LightReachOut {
    /// The report as the wire DTO.
    pub(crate) fn of(report: &engine_spatial::ReachReport) -> Self {
        Self {
            judged: report.judged,
            lit: report.lit,
            dark: report.dark.clone(),
            unknown: report.unknown.clone(),
        }
    }
}

/// `scene.spatial` limit gate: `bad_limit` outside 1..=`SCENE_SPATIAL_MAX_FINDINGS`.
fn check_spatial_limit(limit: u32) -> Result<(), McpToolError> {
    if limit == 0 || limit > SCENE_SPATIAL_MAX_FINDINGS {
        return Err(McpToolError::new(
            "bad_limit",
            format!(
                "limit {limit} is outside 1..={SCENE_SPATIAL_MAX_FINDINGS} \
                 (default {SCENE_SPATIAL_DEFAULT_LIMIT})"
            ),
        ));
    }
    Ok(())
}

/// The `checks` filter of a `scene.spatial` call, validated: `None` runs every
/// offline check; too many names refuse `too_many_checks`, unknown ones `bad_check`.
fn selected_checks(args: &SceneSpatialArgs) -> Result<Option<Vec<&str>>, McpToolError> {
    Ok(match &args.checks {
        None => None,
        Some(names) => {
            if names.len() > SCENE_SPATIAL_MAX_CHECKS {
                return Err(McpToolError::new(
                    "too_many_checks",
                    format!(
                        "checks holds {} entries; at most {SCENE_SPATIAL_MAX_CHECKS}",
                        names.len()
                    ),
                ));
            }
            for name in names {
                if !engine_spatial::SPATIAL_CHECKS.contains(&name.as_str())
                    && !engine_spatial::OPT_IN_CHECKS.contains(&name.as_str())
                {
                    return Err(McpToolError::new(
                        "bad_check",
                        format!(
                            "check `{name}` is unknown; submit any of {}",
                            engine_spatial::SPATIAL_CHECKS.join(", ")
                        ),
                    ));
                }
            }
            Some(names.iter().map(String::as_str).collect())
        }
    })
}

/// Arguments for `scene.place`: the revision the intent was built against, an
/// idempotency key, the object to move (bare UUID or `name [id]` label), and
/// up to [`MAX_PLACE_WANTS`] want lines in the
/// `<relation> <target> [<lo>,<hi>]?` grammar. Malformed wants fail with
/// `bad_want`/`bad_relation`/`bad_wall`/`bad_id`, stale revisions with
/// `stale_revision`, injected lines with `injection_refused`, parented nodes
/// with `not_a_root`, and a busy scene guard with `apply_in_flight`.
/// Unsolvable intents answer the conflict shape, never an error.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ScenePlaceArgs {
    /// Document revision the intent was built against; must equal the live revision.
    pub expected_revision: u64,
    /// Idempotency key for the gated apply; doubles as the undo token.
    pub idempotency_key: u64,
    /// Object to move: bare UUID text or a `name [id]` label (id decides).
    pub object: String,
    /// Want lines, at most [`MAX_PLACE_WANTS`], each at most [`MAX_WANT_CHARS`] chars.
    pub wants: Vec<String>,
    /// `false` skips the spatial delta (default true).
    #[serde(default)]
    pub delta: Option<bool>,
}

/// One solved placement: the staged node id plus its grounded world pose in
/// meters and Y yaw in radians (pure-Y rotation round-trip).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PlacedDto {
    /// Staged node id (bare UUID text).
    pub id: String,
    /// World x in meters.
    pub x: f32,
    /// World y in meters (grounded rest height).
    pub y: f32,
    /// World z in meters.
    pub z: f32,
    /// Y yaw in radians.
    pub yaw: f32,
}

/// Largest coordinate magnitude a placement may carry, in meters: the SP1
/// world bound (+-100 m).
pub const PLACED_LIMIT_M: f32 = 100.0;

impl PlacedDto {
    /// Build a placement from meters and a Y yaw in radians, refusing typed
    /// instead of passing a bad pose on: every lane must be finite, each
    /// coordinate within +-[`PLACED_LIMIT_M`], and the yaw within one full
    /// turn (the solver's 8 steps and `atan2` both stay inside it, so a yaw
    /// outside it is a degrees-for-radians unit mix-up).
    pub fn checked(id: String, x: f32, y: f32, z: f32, yaw: f32) -> Result<Self, McpToolError> {
        for (lane, value) in [("x", x), ("y", y), ("z", z)] {
            if !value.is_finite() || value.abs() > PLACED_LIMIT_M {
                return Err(McpToolError::new(
                    "bad_pose",
                    format!(
                        "placement {lane} {value} m is not a finite value within                          +-{PLACED_LIMIT_M} m; re-run with a pose inside the world bound"
                    ),
                ));
            }
        }
        if !yaw.is_finite() || yaw.abs() > std::f32::consts::TAU + 1e-3 {
            return Err(McpToolError::new(
                "bad_pose",
                format!(
                    "placement yaw {yaw} rad is not a finite angle within +-2*pi;                      re-run with a yaw in radians"
                ),
            ));
        }
        Ok(Self { id, x, y, z, yaw })
    }
}

/// One solver-computed repair for a `scene.place` conflict: relax the
/// dropped want and take this placement instead. Meters in `placement`,
/// integer cm in `move_cm`; mapped from `ConflictSet.hints`, so the same
/// conflict always maps the same DTOs in solver order.
/// `dropped` indexes the failed intent's `wants`, `placement` is the
/// quantized solver output satisfying every other want, and `note` is a
/// short stable code like `drop NextTo wall North`. The caller applies a
/// hint by re-placing without the dropped want; the hinted placement then
/// verifies clean.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RepairHintDto {
    /// Index into the failed intent's `wants` that was relaxed.
    pub dropped: usize,
    /// The relaxed want's source text, copied so callers need no lookup.
    pub dropped_want: String,
    /// Solver placement satisfying every other want, self-verified clean.
    pub placement: PlacedDto,
    /// Integer-cm XZ move to `placement`; `(0, 0)` when unknown.
    pub move_cm: [i32; 2],
    /// Short stable repair code for briefs and logs, never a newline.
    pub note: String,
}

/// Output of `scene.place`: either a solved gated apply or the conflict shape.
/// Solved answers carry the new revision, the placement, the preview effects
/// and the undo key; conflict answers carry `solved: false` plus the culprit
/// want texts, one recovery reason, the solver-computed repair hints and one
/// dry-run summary per hint, writing nothing.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ScenePlaceOutput {
    /// Document revision after the call (unchanged on conflicts and refusals).
    pub revision: u64,
    /// True when the intent solved and the gated apply landed.
    pub solved: bool,
    /// Solved placement; `None` on conflicts.
    pub placement: Option<PlacedDto>,
    /// Staged preview effects (one per applied op); empty on conflicts.
    pub effects: Vec<String>,
    /// Undo token for the applied batch; `0` on conflicts.
    pub key: u64,
    /// Culprit want source texts, in intent order; empty when solved.
    pub conflicts: Vec<String>,
    /// Recovery reason; empty when solved.
    pub reason: String,
    /// Solver repairs, one per viable single-want relaxation, in solver
    /// order; empty when solved or unrepairable. Each hint verifies clean.
    pub hints: Vec<RepairHintDto>,
    /// Dry-run summary per hint: one `dry_run <i> drop ...` header plus
    /// that hint's `dry_run <i> ...` relation rows, diffed in memory
    /// without touching the scene. Applying the hint replays the same
    /// move, so the rows equal the applied diff. Empty when solved.
    pub dry_run: Vec<String>,
    /// Why each unmet want failed (SPR6); empty when solved or without geometry.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<WantBlockDto>,
    /// Spatial findings this move changed (SPX3); absent with `delta: false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spatial_delta: Option<SpatialDeltaDto>,
}

fn parse_relation(text: &str) -> Result<Relation, McpToolError> {
    match text {
        "supports" => Ok(Relation::Supports),
        "on_top_of" => Ok(Relation::OnTopOf),
        "inside" => Ok(Relation::Inside),
        "next_to" => Ok(Relation::NextTo),
        "facing" => Ok(Relation::Facing),
        other => Err(McpToolError::new(
            "bad_relation",
            format!(
                "relation `{other}` is unknown; \
                 submit supports, on_top_of, inside, next_to or facing (or in_front_of, behind, left_of, right_of, outside)"
            ),
        )),
    }
}

fn parse_wall(text: &str) -> Result<Wall, McpToolError> {
    match text {
        "north" => Ok(Wall::North),
        "south" => Ok(Wall::South),
        "east" => Ok(Wall::East),
        "west" => Ok(Wall::West),
        other => Err(McpToolError::new(
            "bad_wall",
            format!("wall `{other}` is unknown; submit north, south, east or west"),
        )),
    }
}

fn parse_object_id(field: &str, text: &str) -> Result<ObjectId, McpToolError> {
    ObjectId::parse(text).map_err(|_| {
        McpToolError::new(
            "bad_id",
            format!("{field} `{text}` is not an object id (expected UUID text)"),
        )
    })
}

/// The want lines of one call, parsed: the classic wants and the facing and
/// outside ones with their source lines, both in call order; or the conflict
/// answer when a want names an object that is not staged.
type Wired = Result<(Vec<ParsedWant>, Vec<(facing_wants::Extra, String)>), ScenePlaceOutput>;

/// Parse the want lines of a `scene.place` call. A want that names an unknown
/// target id cannot reach the solver: it answers the conflict shape directly,
/// naming that one line.
fn wire_wants(
    revision: u64,
    wants: &[String],
    index_of: &BTreeMap<String, u32>,
) -> Result<Wired, McpToolError> {
    let (mut parsed, mut extras) = (Vec::with_capacity(wants.len()), Vec::new());
    for source in wants {
        let missing = match facing_wants::parse(source, index_of)? {
            Some(facing_wants::Parsed::Extra(extra)) => {
                extras.push((extra, source.clone()));
                continue;
            }
            Some(facing_wants::Parsed::Missing { reason }) => reason,
            None => match parse_want(source, index_of)? {
                WantOutcome::Ready(want) => {
                    let source = source.clone();
                    parsed.push(ParsedWant { want, source });
                    continue;
                }
                WantOutcome::MissingTarget { reason } => reason,
            },
        };
        let culprit = vec![source.clone()];
        let conflict = conflict_output(revision, culprit, missing, Vec::new(), Vec::new());
        return Ok(Err(conflict));
    }
    Ok(Ok((parsed, extras)))
}

// Want grammar, one line per want: `<relation> <target> [<lo>,<hi>]?` where
// relation is supports|on_top_of|inside|next_to|facing and target is
// `wall:<north|south|east|west>`, `object:<uuid>` or `point:<x_cm>,<z_cm>`.
// Distances are inclusive integer centimeters. Anything else is a `bad_want`
// error naming the grammar; an unknown target id is a conflict, not an
// error, so impossible intents always answer with the conflict shape.
enum WantOutcome {
    Ready(engine_spatial::Want),
    MissingTarget { reason: String },
}

fn parse_int(value: &str, what: &str, source: &str) -> Result<i32, McpToolError> {
    value.parse::<i32>().map_err(|_| {
        McpToolError::new(
            "bad_want",
            format!(
                "want `{source}` has a bad {what} `{value}`; \
                 want grammar is `<relation> <target> [<lo>,<hi>]?` with \
                 target `wall:<north|south|east|west>`, `object:<uuid>` or \
                 `point:<x_cm>,<z_cm>`"
            ),
        )
    })
}

fn bad_want(source: &str, detail: &str) -> McpToolError {
    McpToolError::new(
        "bad_want",
        format!(
            "want `{source}` is malformed ({detail}); \
             want grammar is `<relation> <target> [<lo>,<hi>]?` with \
             target `wall:<north|south|east|west>`, `object:<uuid>` or \
             `point:<x_cm>,<z_cm>`"
        ),
    )
}

fn parse_want(source: &str, index_of: &BTreeMap<String, u32>) -> Result<WantOutcome, McpToolError> {
    if source.len() > MAX_WANT_CHARS {
        return Err(bad_want(
            source,
            &format!("longer than {MAX_WANT_CHARS} chars"),
        ));
    }
    let mut parts = source.split_whitespace();
    let relation_text = parts.next().ok_or_else(|| bad_want(source, "empty"))?;
    let target_text = parts
        .next()
        .ok_or_else(|| bad_want(source, "missing target"))?;
    let range_text = parts.next();
    if parts.next().is_some() {
        return Err(bad_want(source, "too many parts"));
    }
    let relation = parse_relation(relation_text).map_err(|error| {
        McpToolError::new(
            "bad_relation",
            format!("want `{source}`: {}", error.message),
        )
    })?;
    let target = match target_text.split_once(':') {
        Some(("wall", name)) => Target::Wall(parse_wall(name).map_err(|error| {
            McpToolError::new("bad_wall", format!("want `{source}`: {}", error.message))
        })?),
        Some(("object", id)) => {
            let target_id = parse_object_id("target", id).map_err(|error| {
                McpToolError::new("bad_id", format!("want `{source}`: {}", error.message))
            })?;
            match index_of.get(&target_id.to_string()).copied() {
                Some(index) => Target::Object(index),
                None => {
                    return Ok(WantOutcome::MissingTarget {
                        reason: format!(
                            "unknown-target {id}: no staged node with that id; \
                             re-read scene.brief for the staged ids"
                        ),
                    });
                }
            }
        }
        Some(("point", coords)) => {
            let (x_text, z_text) = coords
                .split_once(',')
                .ok_or_else(|| bad_want(source, "point needs `<x_cm>,<z_cm>`"))?;
            Target::Point {
                x_cm: parse_int(x_text, "x_cm", source)?,
                z_cm: parse_int(z_text, "z_cm", source)?,
            }
        }
        _ => {
            return Err(bad_want(
                source,
                "target needs a `wall:|object:|point:` kind",
            ))
        }
    };
    let distance_cm = match range_text {
        None => None,
        Some(range) => {
            let inner = range
                .strip_prefix('[')
                .and_then(|text| text.strip_suffix(']'))
                .ok_or_else(|| bad_want(source, "range needs `[<lo>,<hi>]`"))?;
            let (lo_text, hi_text) = inner
                .split_once(',')
                .ok_or_else(|| bad_want(source, "range needs `[<lo>,<hi>]`"))?;
            Some((
                parse_int(lo_text, "lo", source)?,
                parse_int(hi_text, "hi", source)?,
            ))
        }
    };
    Ok(WantOutcome::Ready(engine_spatial::Want {
        relation,
        target,
        distance_cm,
    }))
}

// Solver repair hints plus their dry-run diffs, mapped to the door shape.
// Graph building and the in-memory hint application are pure
// `engine-spatial` calls over the same snapshot the solve ran on, so the
// dry-run rows of a hint and the applied diff after taking that hint through
// the door always agree. Units: placements ride in meters, `move_cm` in
// integer centimeters. Determinism: the same snapshot and conflict always
// map to the same DTOs and lines in solver order. A hint whose placement id
// falls outside the snapshot (a solver bug, never observed) still reports
// against the moved node instead of vanishing.
fn hint_output(
    snap: &Snapshot,
    node: ObjectId,
    spatial_id: u32,
    parsed: &[ParsedWant],
    conflict: &engine_spatial::ConflictSet,
) -> Result<(Vec<RepairHintDto>, Vec<String>), McpToolError> {
    let before = engine_spatial::build(&snap.actors);
    let mut hints = Vec::with_capacity(conflict.hints.len());
    let mut dry_run = Vec::new();
    for (index, hint) in conflict.hints.iter().enumerate() {
        let dropped_text = parsed
            .get(hint.dropped)
            .map(|entry| entry.source.clone())
            .unwrap_or_else(|| format!("want {}", hint.dropped));
        let placement_id = usize::try_from(hint.placement.id)
            .ok()
            .and_then(|actor| snap.ids.get(actor))
            .map(ObjectId::to_string)
            .unwrap_or_else(|| node.to_string());
        // Dry-run: the hinted move applied in memory, diffed against the
        // pre-solve graph. The door apply replays the same single move, so
        // these rows equal the applied diff below.
        let mut after_actors = snap.actors.clone();
        if let Some(actor) = usize::try_from(spatial_id)
            .ok()
            .and_then(|at| after_actors.get_mut(at))
        {
            actor.x = hint.placement.x;
            actor.y = hint.placement.y;
            actor.z = hint.placement.z;
            actor.yaw = hint.placement.yaw;
        }
        let diff = engine_spatial::dry_run_diff(&before, &after_actors);
        let summary = engine_spatial::summarize(&diff);
        dry_run.push(format!(
            "dry_run {index} drop `{dropped_text}` -> [{:.2}, {:.2}, {:.2}] yaw {:.2} {summary}",
            hint.placement.x, hint.placement.y, hint.placement.z, hint.placement.yaw,
        ));
        let row_prefix = format!("dry_run {index} ");
        for line in engine_spatial::describe(&diff) {
            dry_run.push(format!("{row_prefix}{line}"));
        }
        hints.push(RepairHintDto {
            dropped: hint.dropped,
            dropped_want: dropped_text,
            placement: PlacedDto::checked(
                placement_id,
                hint.placement.x,
                hint.placement.y,
                hint.placement.z,
                hint.placement.yaw,
            )?,
            move_cm: hint.move_cm,
            note: hint.note.clone(),
        });
    }
    Ok((hints, dry_run))
}

fn conflict_output(
    revision: u64,
    conflicts: Vec<String>,
    reason: String,
    hints: Vec<RepairHintDto>,
    dry_run: Vec<String>,
) -> ScenePlaceOutput {
    ScenePlaceOutput {
        revision,
        solved: false,
        placement: None,
        effects: Vec::new(),
        key: 0,
        conflicts,
        reason,
        hints,
        dry_run,
        blocked_by: Vec::new(),
        spatial_delta: None,
    }
}

// Pre-solve gates for `scene.place`: the stacked injection screen over
// every untrusted line, the `move`-only allow-list, the want-count bound,
// the revision check and the shared intent-length gate. Runs before
// the flight guard, the parse and the solve, so refused intents never lock,
// stage or search.
fn screen_place_args(args: &ScenePlaceArgs, revision: u64) -> Result<(), McpToolError> {
    // Injection screen first: every untrusted line is data, never
    // instructions. The object ref arrives through asset metadata and the
    // wants through scene text; both screen before any lock, parse or
    // stage, and only the redacted entry is ever logged.
    ServerState::refuse_injected(0, Category::AssetName, &args.object)?;
    let mut entry_id = 1u64;
    for want in &args.wants {
        ServerState::refuse_injected(entry_id, Category::SceneText, want)?;
        entry_id = entry_id.saturating_add(1);
    }
    // Allow-list: the only scene-tool write is a root move.
    check_scene_op(SCENE_PLACE_OP_MOVE)?;
    if args.wants.len() > MAX_PLACE_WANTS {
        return Err(McpToolError::new(
            "too_many_wants",
            format!(
                "wants holds {} entries; at most {MAX_PLACE_WANTS} per placement",
                args.wants.len()
            ),
        ));
    }
    if args.expected_revision != revision {
        return Err(McpToolError::new(
            "stale_revision",
            format!(
                "document is at revision {revision}, intent expects {}; \
                 re-read scene.brief, then place again",
                args.expected_revision
            ),
        ));
    }
    // Shared format gate: the whole intent (all wants) fits the shared
    // intent bound, so the Mutating verb refuses exactly like every other
    // write. Runs last, so stale intents still answer `stale_revision`.
    // Fail closed on the verb table: `scene.place` must classify as
    // Mutating here, so an unlisted verb never slips past the gate.
    if super::guards::verb_class(TOOL_SCENE_PLACE) != Some(super::guards::VerbClass::Mutating) {
        return Err(McpToolError::new(
            "unknown_tool",
            format!(
                "tool `{TOOL_SCENE_PLACE}` is not a classified Mutating write; \
                 re-run through scene.place or tx.preview"
            ),
        ));
    }
    super::guards::check_intent_len(&args.wants.join("\n"))
        .map_err(|guard| McpToolError::new(guard.code, guard.message))?;
    Ok(())
}

// Gated apply of one solved placement through the command service: keep
// the node's scale, stage a root-`move` batch through the allow-list, the
// dry-run cost gate, the staged preview and the single-use session
// approval, then the atomic capability-gated apply exactly like `tx.apply`.
// `key` doubles as the undo token, like the apply receipt. Units: meters
// in, yaw radians.
fn apply_placement(
    state: &mut ServerState,
    node: ObjectId,
    placed: &engine_spatial::TransformOut,
    idempotency_key: u64,
    before: Option<&Snapshot>,
) -> Result<ScenePlaceOutput, McpToolError> {
    // Refuse a non-finite or out-of-range pose before anything is staged.
    let placement = PlacedDto::checked(node.to_string(), placed.x, placed.y, placed.z, placed.yaw)?;
    // Roots carry world poses as their local pose; keep the node's scale.
    let scale = state
        .doc
        .hierarchy
        .local(node)
        .map(|pose| pose.scale)
        .unwrap_or([1.0, 1.0, 1.0]);
    let batch = Batch {
        ops: vec![Op::SetLocal {
            id: node,
            local: Transform::new(
                [placed.x, placed.y, placed.z],
                quat_of_yaw(placed.yaw),
                scale,
            ),
        }],
        base_revision: state.doc.revision,
        idempotency_key,
        provenance: Some(ProvenanceStamp::script("scene.place")),
    };
    // Allow-list: the constructed batch must be root moves only, so
    // scene tools instantiate, delete and reparent nothing by construction.
    check_batch_allowed(&batch)?;
    // Explain dry-run cost gate before any validation or apply.
    let explain_caps = super::explain::ExplainCaps::default_caps();
    // Commit-card gate ahead of validation and the ledger: mask, scope and
    // budget refuse here with the same codes `tx.apply` seals.
    if let Some(refusal) =
        super::tool_capabilities::refuse_write(&state.doc, &batch, &state.cap, &explain_caps)
    {
        return Err(refusal);
    }
    super::explain::check_batch(&state.doc, &batch, &explain_caps)
        .map_err(|refusal| refusal.to_tool_error(&explain_caps))?;
    // Staged preview first (no side effects), then the capability-scoped
    // apply: scope, budget, approval and the staged spatial delta all still
    // the write, exactly like tx.apply. `key` doubles as the undo token,
    // like tx.apply's receipt.
    let preview = commands::validate(&state.doc, &batch).map_err(|error| batch_error(&error))?;
    let approval = approve(
        state.session,
        &batch,
        state.doc.revision,
        state.doc.revision,
    );
    let granted = granted_ops(&state.cap);
    let applied = apply_gated(
        &mut state.doc,
        &mut state.journal,
        &batch,
        &mut state.cap,
        &approval,
        &mut state.ledger,
    )
    .map_err(|error| cap_error(&error, &granted, &state.doc))?;
    // A solved placement is a watch event: move the session-local cache.
    state.refresh_watch();
    let spatial_delta = before.map(|before| spatial_delta(before, &snapshot(&state.doc), &[node]));
    Ok(ScenePlaceOutput {
        revision: applied.revision,
        solved: true,
        placement: Some(placement),
        effects: preview.effects,
        key: applied.key,
        conflicts: Vec::new(),
        reason: String::new(),
        hints: Vec::new(),
        dry_run: Vec::new(),
        blocked_by: Vec::new(),
        spatial_delta,
    })
}

impl ServerState {
    /// Bounded scene brief: digest plus stable text rows over the staged scene.
    /// Units: `budget_bytes` counts brief-text bytes (digest line always kept).
    /// Ownership: reads the live document, writes nothing, takes no locks.
    /// Determinism: same scene and budget always render the same text and the
    /// same digest. Errors: `bad_budget` for `0` or budgets past
    /// [`SCENE_BRIEF_MAX_BUDGET`].
    pub fn scene_brief(&self, args: &SceneBriefArgs) -> Result<SceneBriefOutput, McpToolError> {
        let budget = args.budget_bytes.unwrap_or(SCENE_BRIEF_DEFAULT_BUDGET);
        if budget == 0 || budget > SCENE_BRIEF_MAX_BUDGET {
            return Err(McpToolError::new(
                "bad_budget",
                format!(
                    "budget_bytes {budget} is outside 1..={SCENE_BRIEF_MAX_BUDGET} \
                     (default {SCENE_BRIEF_DEFAULT_BUDGET})"
                ),
            ));
        }
        let snap = snapshot(&self.doc);
        let graph = engine_spatial::build(&snap.actors);
        let report = engine_spatial::run(&snap.actors);
        let budget_bytes = budget as usize;
        let roles = staged_roles(&self.doc, &snap.ids);
        let kept = engine_spatial::brief::brief_folded(
            &snap.actors,
            &graph,
            &report,
            &roles,
            budget_bytes,
        );
        let mut text = kept.text;
        // Affordance rows ride along with the brief: geometry-derived tags per
        // actor, id-ordered, whole rows only within the same byte budget.
        let (tags, _) = engine_spatial::afford::derive_all(&snap.actors, &[], None);
        let mut afford_dropped = false;
        for line in engine_spatial::afford::afford_lines(&tags) {
            if text.len() + line.len() <= budget_bytes {
                text.push_str(&line);
            } else {
                afford_dropped = true;
                break;
            }
        }
        // SP66: one light-reach row rides along too, within the same budget: the
        // nodes no light reaches and the ones no range could be read for stay
        // apart. A document with no light gets no row.
        let mut light_dropped = false;
        let lights = spatial_checks::reach_lights(&self.doc, &snap.ids, &snap.actors);
        if !lights.is_empty() {
            let line = engine_spatial::reach_report(&snap.actors, &lights).line();
            let row = format!("{line}\n");
            if text.len() + row.len() <= budget_bytes {
                text.push_str(&row);
            } else {
                light_dropped = true;
            }
        }
        let truncated = kept.truncated || afford_dropped || light_dropped;
        let page = table_page(
            &snap.ids,
            &snap.actors,
            args.cursor.unwrap_or(0),
            table_allowance(budget),
            None,
        );
        let spatial = if args.include_spatial.unwrap_or(false) {
            let counts = self.scene_spatial(&SceneSpatialArgs {
                format: Some("count".to_owned()),
                ..Default::default()
            })?;
            Some(BriefSpatial {
                verdict: counts.verdict,
                total: counts.total,
                hash: counts.hash,
                focus_gap: wide(kept.findings_cut),
            })
        } else {
            None
        };
        Ok(SceneBriefOutput {
            revision: self.doc.revision,
            digest: engine_spatial::digest(&graph),
            text,
            truncated,
            ids: page.ids,
            objects: page.objects,
            poses: page.poses,
            next_cursor: page.next_cursor,
            spatial,
            repeat_count: 0,
            watch_digest: self.watch_digest,
            watch_subscription: self.watch_subscription,
        })
    }

    /// Spatial findings for the staged scene as brief-style lines.
    /// Units: `limit` counts finding rows per page. Ownership: reads the live
    /// document, writes nothing, takes no locks. Determinism: same scene,
    /// checks and limit always page the same rows with the same hash.
    /// Honesty: `checks_run` names what ran, `skipped` what could not, with a reason.
    /// Errors: `bad_limit` for `0` or limits past
    /// [`SCENE_SPATIAL_MAX_FINDINGS`], `too_many_checks` past
    /// [`SCENE_SPATIAL_MAX_CHECKS`], `bad_check` for unknown names,
    /// `needs_nav_view` for an explicit nav check with no baked view.
    pub fn scene_spatial(
        &self,
        args: &SceneSpatialArgs,
    ) -> Result<SceneSpatialOutput, McpToolError> {
        let limit = args.limit.unwrap_or(SCENE_SPATIAL_DEFAULT_LIMIT);
        check_spatial_limit(limit)?;
        let selected = selected_checks(args)?;
        // Read-only over the live document: staging a proposed batch is
        // tx.preview's job, and the CAP1 gate replays the spatial delta on
        // every apply, so findings here can never drift from the gate.
        let snap = snapshot(&self.doc);
        let (run, skipped) = partition(selected.as_deref(), snap.missing_pose > 0);
        let checks_run: Vec<String> = run.iter().map(|name| (*name).to_string()).collect();
        let epsilon_cm = check_epsilon(args.epsilon_cm)?;
        let report = spatial_report(&snap.actors, &run, epsilon_cm)?;
        // SP66: the lights the document carries, joined with the node poses, so
        // the dark nodes and the ones no range could be read for stay apart.
        let lights = spatial_checks::reach_lights(&self.doc, &snap.ids, &snap.actors);
        let light = if lights.is_empty() {
            None
        } else {
            Some(LightReachOut::of(&engine_spatial::reach_report(
                &snap.actors,
                &lights,
            )))
        };
        let format = QueryFormat::parse(args.format.as_deref())?;
        let asks = Asks {
            triage: args.triage.unwrap_or(false),
            allow: args.allow.as_deref().unwrap_or_default(),
            run: &run,
            epsilon_cm,
            revision: self.doc.revision,
        };
        let seen = view(&snap.actors, &snap.ids, &self.doc, &report, &asks)?;
        let shown: Vec<&engine_spatial::Finding> = seen
            .kept
            .iter()
            .filter(|finding| !args.matched_only.unwrap_or(false) || finding.severity == "red")
            .collect();
        let total = wide(shown.len());
        let rows = format == QueryFormat::Rows;
        let end = if rows {
            index_of(limit).min(shown.len())
        } else {
            0
        };
        let findings: Vec<String> = shown[..end].iter().map(|f| seen.line(f)).collect();
        let findings_len = wide(findings.len());
        let involved: BTreeSet<usize> = shown
            .iter()
            .flat_map(|finding| [finding.a, finding.b])
            .filter_map(|index| usize::try_from(index).ok())
            .filter(|index| *index < snap.actors.len())
            .collect();
        let page = match format {
            QueryFormat::Rows => table_page(
                &snap.ids,
                &snap.actors,
                args.cursor.unwrap_or(0),
                table_allowance(SCENE_BRIEF_DEFAULT_BUDGET),
                None,
            ),
            QueryFormat::Ids => table_page(
                &snap.ids,
                &snap.actors,
                args.cursor.unwrap_or(0),
                table_allowance(SCENE_BRIEF_MAX_BUDGET),
                Some(&involved),
            ),
            QueryFormat::Count => table_page(&[], &[], 0, 0, None),
        };
        // A box that was not measured is named: the page rows are the only
        // nodes a caller can act on, so the note rides with them.
        let default_sized = spatial_checks::default_sized(&page.ids, &page.objects, &snap.defaults);
        Ok(SceneSpatialOutput {
            revision: self.doc.revision,
            verdict: seen.verdict().to_string(),
            hash: report.hash,
            total,
            truncated: format == QueryFormat::Rows && end < shown.len(),
            findings,
            ids: page.ids,
            objects: page.objects,
            next_cursor: page.next_cursor,
            repeat_count: 0,
            counts: SpatialCounts {
                total,
                returned: findings_len,
                suppressed: total - findings_len,
            },
            worst_actors: worst_actors(&shown, &snap.ids, &snap.actors),
            checks_run,
            skipped,
            size_source: snap.sizes,
            default_sized,
            suppressed: seen.suppressed,
            light,
        })
    }

    /// Screen one untrusted `scene.place` line through the stacked RV1+RV2
    /// score gate. On a hit the returned error carries only the redacted
    /// [`super::injection::Entry`] verdict, reason and note plus the layer
    /// and score; the raw line never reaches a log or message.
    /// Integer ids only: `object` is entry 0, `wants[i]` is entry `1 + i`.
    fn refuse_injected(id: u64, category: Category, line: &str) -> Result<(), McpToolError> {
        // RV3 stacked screen (pattern-first inside import_screen, then the
        // mock score gate): replaces the RV2-only screen_entry call.
        if let Some(hit) = import_screen(id, category, line) {
            return Err(McpToolError::new(
                "injection_refused",
                format!(
                    "untrusted scene text refused ({} {} score {}; redacted note `{}`); \
                 discard the line, re-read scene.brief, then place again with clean \
                 wants (grammar `<relation> <target> [<lo>,<hi>]?`)",
                    hit.layer, hit.entry.reason, hit.score, hit.entry.note
                ),
            ));
        }
        Ok(())
    }

    /// Solve relation wants like `next_to wall:north [0,60]` and apply the
    /// resulting root move through the command gate; conflicts on failure.
    /// Units: distances in integer centimeters, yaw in radians, positions in
    /// meters. Ownership: the editor command service owns the scene; this
    /// method stages one root-`move` batch through preview plus the
    /// single-use session capability approval, exactly like `tx.apply`. Threading:
    /// single-flight via the scene-apply guard (fail-fast `apply_in_flight`);
    /// reads and `play.run` never take the guard. Determinism: same document,
    /// object and wants always solve the same placement or the same conflict
    /// set. Score gate: every untrusted line is screened through the stacked
    /// tape plus pattern plus score injection screens first and refused with
    /// `injection_refused` carrying only the redacted verdict. Errors:
    /// `bad_want`/`bad_relation`/`bad_wall`/`bad_id` for malformed wants,
    /// `too_many_wants` past [`MAX_PLACE_WANTS`], `stale_revision` for a
    /// moved document, `not_a_root` for parented nodes, `denied_op` outside
    /// the `move` allow-list, and the capability gate errors on apply.
    pub fn scene_place(&mut self, args: &ScenePlaceArgs) -> Result<ScenePlaceOutput, McpToolError> {
        screen_place_args(args, self.doc.revision)?;
        // MCP8 single flight: fail fast when another scene apply is in
        // flight, never queue behind it. Read-only tools and play.run never
        // take this lock, so they never stall on a scene write.
        let _flight = try_scene_apply_guard()?;
        let snap = snapshot(&self.doc);
        let index_of: BTreeMap<String, u32> = snap
            .ids
            .iter()
            .enumerate()
            .map(|(index, id)| (id.to_string(), index as u32))
            .collect();
        let object = parse_object_ref("object", &args.object)?;
        let Some(spatial_id) = index_of.get(&object.to_string()).copied() else {
            return Ok(conflict_output(
                self.doc.revision,
                vec![format!("move object {}", args.object)],
                format!(
                    "unknown-object {}: no staged node with that id; \
                     re-read scene.brief for the staged ids",
                    args.object
                ),
                Vec::new(),
                Vec::new(),
            ));
        };
        let node = snap.ids[spatial_id as usize];
        let parented = self
            .doc
            .hierarchy
            .parent(node)
            .is_some_and(|parent| parent != ObjectId::NIL);
        if parented {
            return Err(McpToolError::new(
                "not_a_root",
                format!(
                    "node {object} is parented; scene.place moves roots only \
                     (reparent it to a root first, then place)"
                ),
            ));
        }
        // Wire wants to solver wants. A want that names an unknown target id
        // cannot reach the solver, so it returns the conflict shape directly
        // with the single culprit want.
        let (parsed, extras) = match wire_wants(self.doc.revision, &args.wants, &index_of)? {
            Ok(wired) => wired,
            Err(conflict) => return Ok(conflict),
        };
        let intent = Intent {
            object: ObjectRef::Id(spatial_id),
            wants: parsed.iter().map(|entry| entry.want).collect(),
        };
        if !extras.is_empty() {
            let solved =
                facing_wants::solve(self.doc.revision, &intent, &parsed, &extras, &snap.actors)?;
            return match solved {
                Ok(placed) => {
                    let before = (args.delta != Some(false)).then_some(&snap);
                    apply_placement(self, node, &placed, args.idempotency_key, before)
                }
                Err(conflict) => Ok(conflict),
            };
        }
        let solved = match engine_spatial::solve(&intent, &snap.actors) {
            Ok(out) => out,
            Err(conflict) => {
                let (hints, dry_run) = hint_output(&snap, node, spatial_id, &parsed, &conflict)?;
                let texts = conflict_texts(&parsed, &conflict.unmet);
                let blocked_by = block_output(&snap, &conflict, spatial_id);
                let mut out =
                    conflict_output(self.doc.revision, texts, conflict.reason, hints, dry_run);
                out.blocked_by = blocked_by;
                return Ok(out);
            }
        };
        let sources: Vec<String> = parsed.iter().map(|entry| entry.source.clone()).collect();
        let Some(placed) = solved.first() else {
            return Ok(conflict_output(
                self.doc.revision,
                sources,
                "solver returned no placement and no conflict; report this intent verbatim"
                    .to_string(),
                Vec::new(),
                Vec::new(),
            ));
        };
        debug_assert_eq!(placed.id, spatial_id);
        // SPX3: the snapshot the solve ran on is the scene as it was; `delta:
        // false` skips the second snapshot and the catalog runs.
        let before = (args.delta != Some(false)).then_some(&snap);
        apply_placement(self, node, placed, args.idempotency_key, before)
    }
}

/// Run the selected catalog checks over staged actors, mapping a refusal to
/// the door's own codes: nav checks without a baked view refuse
/// `needs_nav_view`, every other refusal is `bad_check` with the legal names.
fn spatial_report(
    actors: &[engine_spatial::Actor],
    run: &[&str],
    epsilon_cm: i32,
) -> Result<engine_spatial::Report, McpToolError> {
    engine_spatial::run_checks_eps(actors, run, epsilon_cm).map_err(|legal| {
        if legal == ["needs-nav-view"] {
            McpToolError::new(
                "needs_nav_view",
                "nav checks need a baked nav view which this tool has no access to; \
                 submit offline checks only",
            )
        } else {
            McpToolError::new(
                "bad_check",
                format!("check refused; submit any of {}", legal.join(", ")),
            )
        }
    })
}

/// MCP tool registration for `scene.brief` (read-only brief page).
pub struct SceneBriefTool;

impl ToolBase for SceneBriefTool {
    type Parameter = SceneBriefArgs;
    type Output = SceneBriefOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_SCENE_BRIEF.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some("Bounded scene brief: digest plus stable text rows. Read-only.".into())
    }

    fn annotations() -> Option<ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for SceneBriefTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch_screened(TOOL_SCENE_BRIEF, &param, |state| {
            let repeat = state.note_read(TOOL_SCENE_BRIEF, &param)?;
            let mut out = state.scene_brief(&param)?;
            out.repeat_count = repeat;
            Ok(out)
        })
    }
}

/// MCP tool registration for `scene.spatial` (read-only findings page).
pub struct SceneSpatialTool;

impl ToolBase for SceneSpatialTool {
    type Parameter = SceneSpatialArgs;
    type Output = SceneSpatialOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_SCENE_SPATIAL.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some("Spatial findings for the staged scene as brief-style lines. Read-only.".into())
    }

    fn annotations() -> Option<ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for SceneSpatialTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch_screened(TOOL_SCENE_SPATIAL, &param, |state| {
            let repeat = state.note_read(TOOL_SCENE_SPATIAL, &param)?;
            let mut out = state.scene_spatial(&param)?;
            out.repeat_count = repeat;
            Ok(out)
        })
    }
}

/// MCP tool registration for `scene.place` (gated root move; idempotent).
pub struct ScenePlaceTool;

impl ToolBase for ScenePlaceTool {
    type Parameter = ScenePlaceArgs;
    type Output = ScenePlaceOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_SCENE_PLACE.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Solve relation wants like `next_to wall:north [0,60]` and apply \
             through the command gate; conflicts carry solver repair hints \
             plus dry-run diffs."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        crate::mcp::write_annotations(true)
    }
}

impl SyncTool<McpServer> for ScenePlaceTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch_screened(TOOL_SCENE_PLACE, &param, |state| state.scene_place(&param))
    }
}

#[cfg(test)]
mod tests {
    use super::super::mcp::{SceneReadArgs, SceneReadTool};
    use super::super::play::{PlayRunArgs, PlayRunTool};
    use super::*;
    use crate::spatial_glue::yaw_of_quat;
    use engine_editor::capability::{Capability, OpMask};
    use engine_editor::commands::Journal;
    use engine_scene::ids::ObjectId;
    use std::sync::Mutex;

    /// A bare staged name is not an object id: the refusal names the label form.
    #[test]
    fn bare_name_refusal_names_the_label_form() {
        let Err(refused) = parse_object_ref("anchor", "prop_1") else {
            panic!("a bare name is not an object id");
        };
        assert_eq!(refused.code, "bad_id");
        assert!(
            refused.message.contains("name [uuid]"),
            "{}",
            refused.message
        );
    }

    // Serializes every test that stages a scene apply through the MCP8
    // single-flight guard. The single-flight test holds the process-wide
    // guard while asserting; without this serial, a parallel place test
    // could land inside that window and spuriously answer `apply_in_flight`.
    // Lock order everywhere is serial, then the server lock, then the flight
    // guard (try-only, never blocking), so this cannot deadlock.
    static PLACE_TEST_SERIAL: Mutex<()> = Mutex::new(());

    fn revision(server: &McpServer) -> u64 {
        SceneReadTool::invoke(server, SceneReadArgs::default())
            .expect("scene reads")
            .revision
    }

    fn world_of(server: &McpServer, id: &ObjectId) -> [f32; 3] {
        let page = SceneReadTool::invoke(
            server,
            SceneReadArgs {
                limit: Some(128),
                cursor: None,
            },
        )
        .expect("scene reads");
        page.objects
            .iter()
            .find(|row| row.id == id.to_string())
            .expect("inserted node reads back")
            .local
            .pos
    }

    /// A value whose serialization always fails, so the fail-closed payload
    /// path can prove it refuses instead of screening an empty string.
    struct Unserializable;

    impl rmcp::serde::Serialize for Unserializable {
        fn serialize<S: rmcp::serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
            Err(rmcp::serde::ser::Error::custom(
                "unserializable for the audit test",
            ))
        }
    }

    #[test]
    fn place_payload_serializes_valid_args_and_refuses_failures_audited() {
        use super::super::audit::{code_hash, AuditOutcome};
        use super::super::door_dispatch::untrusted_text;

        // Valid args yield the call's own text for the hostile-text screen.
        let args = ScenePlaceArgs {
            expected_revision: 3,
            idempotency_key: 77,
            object: "brazier".to_owned(),
            wants: vec!["next_to wall:north [0,60]".to_owned()],
            delta: None,
        };
        let payload = untrusted_text(TOOL_SCENE_PLACE, &args).expect("valid args serialize");
        assert!(payload.contains("brazier"), "got: {payload}");
        assert!(payload.contains("next_to"), "got: {payload}");

        // A serialization failure refuses `bad_args` and names the tool plus
        // the recovery, never an empty screened string.
        let refused = untrusted_text(TOOL_SCENE_PLACE, &Unserializable)
            .expect_err("unserializable args must refuse fail-closed");
        assert_eq!(refused.code, "bad_args");
        assert!(
            refused.message.contains(TOOL_SCENE_PLACE),
            "got: {}",
            refused.message
        );
        assert!(
            refused.message.contains("re-run"),
            "got: {}",
            refused.message
        );

        // The invoke failure branch seals that refusal through the dispatch
        // seam: one `Refused` audit record bound to `bad_args`, revision
        // unmoved, so the failure is never a silent drop. The revision reads
        // straight off the lock so the check itself chains no audit record.
        let server = McpServer::new(7, Capability::unrestricted(7));
        let before = server.lock().expect("test lock holds").doc.revision;
        let sealed = server
            .dispatch_with_payload(TOOL_SCENE_PLACE, "", |_| {
                Err::<ScenePlaceOutput, _>(refused)
            })
            .expect_err("sealed refusal still refuses");
        assert_eq!(sealed.code, "bad_args");
        assert_eq!(
            server.lock().expect("test lock holds").doc.revision,
            before,
            "refusal writes nothing"
        );
        let log = server.audit_log().expect("audit reads");
        assert_eq!(log.len(), 1);
        assert_eq!(log.total_appended(), 1);
        let record = log.iter().next().expect("one record");
        assert_eq!(record.tool, TOOL_SCENE_PLACE);
        assert_eq!(record.outcome, AuditOutcome::Refused);
        assert_eq!(record.detail, code_hash("bad_args"));
    }

    // Four nodes straight through the command service (which, unlike the
    // CAP1-gated MCP apply, stages no spatial delta): the first two share
    // one XZ footprint, seeding exactly one overlap pair.
    fn overlap_scene() -> (McpServer, ObjectId, ObjectId) {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let a = ObjectId::new();
        let b = ObjectId::new();
        let c = ObjectId::new();
        let d = ObjectId::new();
        let spots = [
            (a, [0.0, 0.0, 0.0], 101u64),
            (b, [0.0, 0.0, 0.0], 102u64),
            (c, [10.0, 0.0, 10.0], 103u64),
            (d, [-10.0, 0.0, -10.0], 104u64),
        ];
        for (id, pos, key) in spots {
            let batch = Batch {
                ops: vec![Op::Insert {
                    id,
                    parent: ObjectId::NIL,
                    local: Transform::new(pos, [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
                }],
                base_revision: doc.revision,
                idempotency_key: key,
                provenance: None,
            };
            commands::apply(&mut doc, &mut journal, &batch).expect("overlap seed applies");
        }
        (
            McpServer::with_document(doc, journal, 7, Capability::unrestricted(7)),
            a,
            b,
        )
    }

    // Walled room of unit cubes (CAP1 convention): thin-wall geometry is
    // impossible here, so adjacency means edge-gap to the wall block.
    // North wall block at z=-5, gate block at (4,-4), brazier starter at (0,2).
    struct RoomIds {
        wall_north: ObjectId,
        gate: ObjectId,
        brazier: ObjectId,
    }

    struct Room {
        server: McpServer,
        wall_north: ObjectId,
        gate: ObjectId,
        brazier: ObjectId,
    }

    // Six grounded roots straight through the command service, so both the
    // live server and a capability-scoped server can share one layout.
    fn seed_room(doc: &mut Document, journal: &mut Journal) -> RoomIds {
        let wall_north = ObjectId::new();
        let wall_south = ObjectId::new();
        let wall_west = ObjectId::new();
        let wall_east = ObjectId::new();
        let gate = ObjectId::new();
        let brazier = ObjectId::new();
        let spots = [
            (wall_north, [0.0, 0.0, -5.0], 201u64),
            (wall_south, [0.0, 0.0, 5.0], 202u64),
            (wall_west, [-5.0, 0.0, 0.0], 203u64),
            (wall_east, [5.0, 0.0, 0.0], 204u64),
            (gate, [4.0, 0.0, -4.0], 205u64),
            (brazier, [0.0, 0.0, 2.0], 206u64),
        ];
        for (id, pos, key) in spots {
            let batch = Batch {
                ops: vec![Op::Insert {
                    id,
                    parent: ObjectId::NIL,
                    local: Transform::new(pos, [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
                }],
                base_revision: doc.revision,
                idempotency_key: key,
                provenance: None,
            };
            commands::apply(doc, journal, &batch).expect("room seed applies");
        }
        RoomIds {
            wall_north,
            gate,
            brazier,
        }
    }

    fn room_scene() -> Room {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let ids = seed_room(&mut doc, &mut journal);
        let server = McpServer::with_document(doc, journal, 7, Capability::unrestricted(7));
        Room {
            server,
            wall_north: ids.wall_north,
            gate: ids.gate,
            brazier: ids.brazier,
        }
    }

    fn brazier_intent(brazier: &ObjectId, wall: &ObjectId, gate: &ObjectId) -> ScenePlaceArgs {
        ScenePlaceArgs {
            expected_revision: 0,
            idempotency_key: 301,
            object: brazier.to_string(),
            wants: vec![
                format!("next_to object:{wall}"),
                format!("next_to object:{gate} [150,250]"),
            ],
            delta: None,
        }
    }

    #[test]
    fn brief_returns_digest_stable_text() {
        let (server, _, _) = overlap_scene();
        let first =
            SceneBriefTool::invoke(&server, SceneBriefArgs::default()).expect("brief reads");
        let second =
            SceneBriefTool::invoke(&server, SceneBriefArgs::default()).expect("brief reads again");
        assert_eq!(first.text, second.text, "same scene twice, same text");
        assert_eq!(first.digest, second.digest);
        assert!(first.text.starts_with(&format!("digest {}", first.digest)));
        assert_eq!(first.revision, revision(&server));
        assert_eq!(first.ids.len(), 4);
        assert!(!first.truncated, "four props fit the default budget");
        // Digest matches the spatial graph digest over the same snapshot.
        let state = server.lock().expect("test lock holds");
        let snap = snapshot(&state.doc);
        drop(state);
        assert_eq!(
            first.digest,
            engine_spatial::digest(&engine_spatial::build(&snap.actors))
        );
        // A tight budget keeps the digest line and fits the budget.
        let tight = SceneBriefTool::invoke(
            &server,
            SceneBriefArgs {
                budget_bytes: Some(64),
                ..Default::default()
            },
        )
        .expect("tight brief reads");
        assert_eq!(tight.digest, first.digest, "digest survives any budget");
        assert!(tight.text.starts_with("digest "));
        assert!(tight.text.len() <= 64);
        assert!(tight.truncated);
    }

    #[test]
    fn brief_rejects_bad_budgets() {
        let (server, _, _) = overlap_scene();
        for budget in [Some(0), Some(SCENE_BRIEF_MAX_BUDGET + 1)] {
            let refused = SceneBriefTool::invoke(
                &server,
                SceneBriefArgs {
                    budget_bytes: budget,
                    ..Default::default()
                },
            )
            .expect_err("bad budget must fail");
            assert_eq!(refused.code, "bad_budget");
            assert!(refused.message.contains("16384"));
        }
    }

    #[test]
    fn spatial_surfaces_seeded_overlap() {
        let (server, a, b) = overlap_scene();
        let report = SceneSpatialTool::invoke(
            &server,
            SceneSpatialArgs {
                checks: None,
                limit: None,
                ..Default::default()
            },
        )
        .expect("spatial reads");
        assert_eq!(report.verdict, "red");
        let line = report
            .findings
            .iter()
            .find(|line| line.starts_with("finding overlap "))
            .expect("seeded overlap surfaces");
        let parts: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(parts.len(), 6, "finding line shape: {line}");
        assert_eq!((parts[1], parts[4]), ("overlap", "red"));
        let a_index: usize = parts[2].parse().expect("numeric a");
        let b_index: usize = parts[3].parse().expect("numeric b");
        let pair = [report.ids[a_index].clone(), report.ids[b_index].clone()];
        assert!(pair.contains(&a.to_string()));
        assert!(pair.contains(&b.to_string()));
        assert_eq!(report.revision, revision(&server));
        assert_eq!(report.total, report.findings.len() as u64);
        assert!(!report.truncated);
    }

    #[test]
    fn spatial_is_read_only_and_bounded() {
        // Six cubes on one footprint: every pair overlaps, so a small limit
        // must truncate with the total kept.
        let mut doc = Document::new();
        let mut journal = Journal::new();
        for key in 401u64..407 {
            let batch = Batch {
                ops: vec![Op::Insert {
                    id: ObjectId::new(),
                    parent: ObjectId::NIL,
                    local: Transform::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
                }],
                base_revision: doc.revision,
                idempotency_key: key,
                provenance: None,
            };
            commands::apply(&mut doc, &mut journal, &batch).expect("crowd seed applies");
        }
        let server = McpServer::with_document(doc, journal, 7, Capability::unrestricted(7));
        let before = SceneReadTool::invoke(&server, SceneReadArgs::default()).expect("reads");
        let page = SceneSpatialTool::invoke(
            &server,
            SceneSpatialArgs {
                checks: Some(vec!["overlap".to_string()]),
                limit: Some(4),
                ..Default::default()
            },
        )
        .expect("bounded check reads");
        assert!(page.total > 4, "crowd must overflow the page");
        assert_eq!(page.findings.len(), 4);
        assert!(page.truncated);
        for line in &page.findings {
            assert!(
                line.starts_with("finding overlap "),
                "filtered check only, got: {line}"
            );
        }
        let after = SceneReadTool::invoke(&server, SceneReadArgs::default()).expect("reads");
        assert_eq!(before.objects, after.objects, "spatial reads write nothing");
        assert_eq!(before.revision, after.revision);
    }

    #[test]
    fn spatial_check_filter_and_limits() {
        let (server, _, _) = overlap_scene();
        let only = SceneSpatialTool::invoke(
            &server,
            SceneSpatialArgs {
                checks: Some(vec!["overlap".to_string()]),
                limit: Some(1),
                ..Default::default()
            },
        )
        .expect("filtered check reads");
        assert!(only
            .findings
            .iter()
            .all(|line| line.starts_with("finding overlap ")));
        assert!(only.findings.len() <= 1);
        assert!(only.total >= only.findings.len() as u64);
        let refused = SceneSpatialTool::invoke(
            &server,
            SceneSpatialArgs {
                checks: Some(vec!["teleport".to_string()]),
                limit: None,
                ..Default::default()
            },
        )
        .expect_err("unknown check must fail");
        assert_eq!(refused.code, "bad_check");
        assert!(refused.message.contains("overlap"));
        let nav = SceneSpatialTool::invoke(
            &server,
            SceneSpatialArgs {
                checks: Some(vec!["navConnect".to_string()]),
                limit: None,
                ..Default::default()
            },
        )
        .expect_err("nav check without a view must fail");
        assert_eq!(nav.code, "needs_nav_view");
        let over = SceneSpatialTool::invoke(
            &server,
            SceneSpatialArgs {
                checks: None,
                limit: Some(SCENE_SPATIAL_MAX_FINDINGS + 1),
                ..Default::default()
            },
        )
        .expect_err("over-limit page must fail");
        assert_eq!(over.code, "bad_limit");
    }

    #[test]
    fn spatial_opt_in_relation_cycle_is_legal_but_off_the_default_run() {
        let (server, _, _) = overlap_scene();
        let page = SceneSpatialTool::invoke(
            &server,
            SceneSpatialArgs {
                checks: Some(vec!["relationCycle".to_string()]),
                limit: None,
                ..Default::default()
            },
        )
        .expect("opt-in check is legal when listed");
        assert_eq!(
            page.total, 0,
            "built graphs are acyclic: the check is silent"
        );
        let refused = SceneSpatialTool::invoke(
            &server,
            SceneSpatialArgs {
                checks: Some(vec!["teleport".to_string()]),
                limit: None,
                ..Default::default()
            },
        )
        .expect_err("unknown check must fail");
        assert!(
            !refused.message.contains("relationCycle"),
            "the frozen catalog list stays unchanged: {}",
            refused.message
        );
    }

    // AD3 fixture: one stage fires every check the door can drive — an
    // overlapping crowd over the density cap, a floater (support too), a sunk
    // node, a far node, and a finite 1e30-scaled chain whose last `0 * inf`
    // world pose is NaN (staged as a finding, never a silent drop).
    fn ad3_server() -> McpServer {
        fn add(ops: &mut Vec<Op>, parent: ObjectId, pos: [f32; 3], scale: [f32; 3]) -> ObjectId {
            let id = ObjectId::new();
            ops.push(Op::Insert {
                id,
                parent,
                local: Transform::new(pos, [0.0, 0.0, 0.0, 1.0], scale),
            });
            id
        }
        let nil = ObjectId::NIL;
        let mut ops = Vec::new();
        for i in 0..9u8 {
            add(&mut ops, nil, [0.05 * f32::from(i), 0.0, 0.0], [1.0; 3]);
        }
        for pos in [[0.0, 5.0, 0.0], [0.0, -5.0, 0.0], [150.0, 0.0, 0.0]] {
            add(&mut ops, nil, pos, [1.0; 3]);
        }
        let root = add(&mut ops, nil, [0.0; 3], [1.0e30; 3]);
        let mid = add(&mut ops, root, [0.0; 3], [1.0e30; 3]);
        add(&mut ops, mid, [0.0; 3], [1.0; 3]);
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let batch = Batch {
            ops,
            base_revision: 0,
            idempotency_key: 901,
            provenance: None,
        };
        commands::apply(&mut doc, &mut journal, &batch).expect("fixture applies");
        McpServer::with_document(doc, journal, 7, Capability::unrestricted(7))
    }

    fn door_read(server: &McpServer) -> SceneSpatialOutput {
        SceneSpatialTool::invoke(
            server,
            SceneSpatialArgs {
                checks: None,
                limit: Some(SCENE_SPATIAL_MAX_FINDINGS),
                ..Default::default()
            },
        )
        .expect("read")
    }

    // AD3 door fire test + output golden (receipt 2026-10-17): the output
    // gained `checks_run`, `skipped` and `size_source`. Every `SPATIAL_CHECKS`
    // entry either fires on this fixture or is named in `skipped` with a
    // non-empty reason, ran plus skipped covers the catalog exactly once, a
    // filtered call reports the same honesty, a bare scene counts its default
    // boxes, and the JSON golden carries all three keys and round-trips.
    #[test]
    fn spatial_door_fire() {
        let server = ad3_server();
        let page = door_read(&server);
        for check in engine_spatial::SPATIAL_CHECKS {
            let fired = page
                .findings
                .iter()
                .any(|line| line.starts_with(&format!("finding {check} ")));
            let named = page.skipped.iter().find(|row| row.check == check);
            assert!(
                fired || named.is_some(),
                "{check}: neither fired nor skipped"
            );
            assert!(
                !(fired && named.is_some()),
                "{check}: fired checks are not also reported skipped"
            );
            if let Some(row) = named {
                assert!(!row.reason.is_empty(), "{check}: skip carries a reason");
                assert!(
                    engine_spatial::SPATIAL_CHECKS.contains(&row.check.as_str()),
                    "{}: skipped name is off the frozen catalog",
                    row.check
                );
            }
        }
        assert_eq!(
            page.checks_run.len() + page.skipped.len(),
            engine_spatial::SPATIAL_CHECKS.len(),
            "ran + skipped must cover the catalog"
        );
        // The fixture's non-finite world pose is a finding, not a dropped node.
        assert!(
            page.checks_run
                .iter()
                .any(|name| name.as_str() == "unknown-position"),
            "a non-finite pose must run unknown-position: {:?}",
            page.checks_run
        );
        assert!(page
            .findings
            .iter()
            .any(|line| line.starts_with("finding unknown-position ")));
        assert_eq!(page.size_source.collider, 0, "no colliders yet");
        assert!(page.size_source.default > 0, "nodes are default-sized");
        let filtered = SceneSpatialTool::invoke(
            &server,
            SceneSpatialArgs {
                checks: Some(vec!["overlap".to_string()]),
                limit: None,
                ..Default::default()
            },
        )
        .expect("filtered read");
        assert_eq!(filtered.checks_run, vec!["overlap".to_string()]);
        assert_eq!(
            filtered.skipped.len(),
            engine_spatial::SPATIAL_CHECKS.len() - 1,
            "every unrequested catalog name is named"
        );
        assert!(filtered
            .skipped
            .iter()
            .all(|row| row.reason == "not_requested"));
        let (bare, _, _) = overlap_scene();
        assert_eq!(door_read(&bare).size_source.default, 4, "four bare nodes");
        let json = rmcp::serde_json::to_value(&page).expect("output serializes");
        for key in ["checks_run", "skipped", "size_source"] {
            assert!(json.get(key).is_some(), "golden key {key} missing");
        }
        let back: SceneSpatialOutput =
            rmcp::serde_json::from_value(json).expect("golden round-trips");
        assert_eq!(back, page, "output golden is stable");
    }

    #[test]
    fn place_solves_brazier_style_through_preview_to_apply() {
        let _serial = PLACE_TEST_SERIAL.lock().expect("place tests serialize");
        let room = room_scene();
        let rev = revision(&room.server);
        let mut args = brazier_intent(&room.brazier, &room.wall_north, &room.gate);
        args.expected_revision = rev;
        let out = ScenePlaceTool::invoke(&room.server, args).expect("place solves");
        assert!(out.solved, "board-style intent must solve: {}", out.reason);
        assert!(out.conflicts.is_empty());
        assert_eq!(out.effects.len(), 1);
        assert_eq!(out.revision, rev + 1);
        let placed = out.placement.expect("solved placement rides along");
        assert_eq!(placed.id, room.brazier.to_string());
        assert_eq!(placed.y, 0.5, "placements rest grounded");
        // Against the north wall block (edge gap within the default 0..100)
        // and about 2 m edge gap from the gate block.
        let wall_gap_x = (placed.x - 0.0).abs() - 1.0;
        let wall_gap_z = (placed.z + 5.0).abs() - 1.0;
        let wall_gap = wall_gap_x.max(wall_gap_z).max(0.0);
        assert!(
            wall_gap * 100.0 <= 100.0,
            "wall edge gap {wall_gap} m out of range"
        );
        let gate_gap_x = (placed.x - 4.0).abs() - 1.0;
        let gate_gap_z = (placed.z + 4.0).abs() - 1.0;
        let gate_gap2 = gate_gap_x.max(0.0).powi(2) + gate_gap_z.max(0.0).powi(2);
        assert!(
            (1.5f32.powi(2)..=2.5f32.powi(2)).contains(&gate_gap2),
            "gate gap^2 {gate_gap2} out of range"
        );
        // The move landed through the command service: world pose matches and
        // the staged scene now verifies clean.
        let pos = world_of(&room.server, &room.brazier);
        assert_eq!(pos, [placed.x, placed.y, placed.z]);
        let after = SceneSpatialTool::invoke(
            &room.server,
            SceneSpatialArgs {
                checks: None,
                limit: None,
                ..Default::default()
            },
        )
        .expect("post-place check reads");
        assert_eq!(after.verdict, "clean", "solved scene runs clean");
        let brief = SceneBriefTool::invoke(&room.server, SceneBriefArgs::default())
            .expect("post-place brief reads");
        assert_eq!(brief.revision, rev + 1);
    }

    #[test]
    fn place_impossible_intent_returns_conflicts() {
        let _serial = PLACE_TEST_SERIAL.lock().expect("place tests serialize");
        let room = room_scene();
        let rev = revision(&room.server);
        let against = |name: &str| format!("next_to wall:{name} [0,50]");
        let args = ScenePlaceArgs {
            expected_revision: rev,
            idempotency_key: 302,
            object: room.brazier.to_string(),
            wants: vec![against("north"), against("south")],
            delta: None,
        };
        let out = ScenePlaceTool::invoke(&room.server, args).expect("conflicts answer");
        assert!(!out.solved);
        assert_eq!(out.conflicts.len(), 2, "minimal conflict keeps both wants");
        assert!(
            out.reason.contains("minimal conflict"),
            "got: {}",
            out.reason
        );
        assert!(out.placement.is_none());
        assert_eq!(out.revision, rev, "conflicts write nothing");
        assert_eq!(
            world_of(&room.server, &room.brazier),
            [0.0, 0.0, 2.0],
            "brazier never moved"
        );
        // Minimal: each want alone solves.
        for name in ["north", "south"] {
            let single = ScenePlaceArgs {
                expected_revision: revision(&room.server),
                idempotency_key: 303 + u64::from(name == "south"),
                object: room.brazier.to_string(),
                wants: vec![against(name)],
                delta: None,
            };
            let solo = ScenePlaceTool::invoke(&room.server, single).expect("single solves");
            assert!(solo.solved, "{name} alone must solve: {}", solo.reason);
        }
    }

    #[test]
    fn place_with_exhausted_budget_refuses_before_apply() {
        let _serial = PLACE_TEST_SERIAL.lock().expect("place tests serialize");
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let ids = seed_room(&mut doc, &mut journal);
        let server =
            McpServer::with_document(doc, journal, 7, Capability::new(7, OpMask::ALL, vec![], 0));
        // Live revision: the revision gate fires before the batch gate by
        // door contract, so a stale revision would mask the budget refusal.
        let rev = revision(&server);
        let refused = ScenePlaceTool::invoke(
            &server,
            ScenePlaceArgs {
                expected_revision: rev,
                idempotency_key: 340,
                object: ids.brazier.to_string(),
                wants: vec!["next_to wall:north".to_string()],
                delta: None,
            },
        )
        .expect_err("exhausted budget must fail before applying");
        assert_eq!(refused.code, "over_budget");
        assert_eq!(revision(&server), rev, "refusal writes nothing");
    }

    #[test]
    fn place_unknown_target_is_a_conflict_not_a_write() {
        let _serial = PLACE_TEST_SERIAL.lock().expect("place tests serialize");
        let room = room_scene();
        let rev = revision(&room.server);
        let args = ScenePlaceArgs {
            expected_revision: rev,
            idempotency_key: 304,
            object: room.brazier.to_string(),
            wants: vec![format!("next_to object:{}", ObjectId::new())],
            delta: None,
        };
        let out = ScenePlaceTool::invoke(&room.server, args).expect("conflicts answer");
        assert!(!out.solved);
        assert_eq!(out.conflicts.len(), 1);
        assert!(out.reason.contains("unknown-target"), "got: {}", out.reason);
        assert_eq!(revision(&room.server), rev);
    }

    #[test]
    fn place_denied_or_stale_writes_nothing() {
        let _serial = PLACE_TEST_SERIAL.lock().expect("place tests serialize");
        let room = room_scene();
        let rev = revision(&room.server);
        // Malformed wants fail before any solve, naming the grammar.
        let cases = [
            ("teleport wall:north", "bad_relation"),
            ("next_to wall:up", "bad_wall"),
            ("next_to object:not-a-uuid", "bad_id"),
            ("next_to", "bad_want"),
            ("next_to wall:north wastespace extra", "bad_want"),
            ("next_to point:10", "bad_want"),
            ("next_to point:10,x", "bad_want"),
            ("next_to wall:north 0,60", "bad_want"),
            ("next_to wall:north [0]", "bad_want"),
        ];
        for (index, (want, code)) in cases.iter().enumerate() {
            let refused = ScenePlaceTool::invoke(
                &room.server,
                ScenePlaceArgs {
                    expected_revision: rev,
                    idempotency_key: 305 + index as u64,
                    object: room.brazier.to_string(),
                    wants: vec![(*want).to_string()],
                    delta: None,
                },
            )
            .expect_err("malformed want must fail");
            assert_eq!(refused.code, *code, "want `{want}`");
        }
        let flooded = ScenePlaceTool::invoke(
            &room.server,
            ScenePlaceArgs {
                expected_revision: rev,
                idempotency_key: 320,
                object: room.brazier.to_string(),
                wants: vec!["next_to wall:north".to_string(); MAX_PLACE_WANTS + 1],
                delta: None,
            },
        )
        .expect_err("oversize intent must fail");
        assert_eq!(flooded.code, "too_many_wants");
        // Parented nodes are refused: place moves roots only.
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let root = ObjectId::new();
        let child = ObjectId::new();
        for (id, parent, key) in [(root, ObjectId::NIL, 330u64), (child, root, 331u64)] {
            let base = doc.revision;
            commands::apply(
                &mut doc,
                &mut journal,
                &Batch {
                    ops: vec![Op::Insert {
                        id,
                        parent,
                        local: Transform::new(
                            [0.0, 0.0, 0.0],
                            [0.0, 0.0, 0.0, 1.0],
                            [1.0, 1.0, 1.0],
                        ),
                    }],
                    base_revision: base,
                    idempotency_key: key,
                    provenance: None,
                },
            )
            .expect("nested seed applies");
        }
        let nested = McpServer::with_document(doc, journal, 7, Capability::unrestricted(7));
        let rooted = ScenePlaceTool::invoke(
            &nested,
            ScenePlaceArgs {
                expected_revision: revision(&nested),
                idempotency_key: 332,
                object: child.to_string(),
                wants: vec!["next_to wall:north".to_string()],
                delta: None,
            },
        )
        .expect_err("parented object must fail");
        assert_eq!(rooted.code, "not_a_root");
        let stale = ScenePlaceArgs {
            expected_revision: rev + 1,
            idempotency_key: 306,
            object: room.brazier.to_string(),
            wants: Vec::new(),
            delta: None,
        };
        let stale_refused =
            ScenePlaceTool::invoke(&room.server, stale).expect_err("stale intent must fail");
        assert_eq!(stale_refused.code, "stale_revision");
        assert_eq!(revision(&room.server), rev);
        // Out-of-scope capability: the intent solves, then the CAP1 gate
        // fires on apply, writing nothing.
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let ids = seed_room(&mut doc, &mut journal);
        let scoped = McpServer::with_document(
            doc,
            journal,
            9,
            Capability::new(9, OpMask::ALL, vec![ids.wall_north.to_string()], 8),
        );
        let scoped_rev = revision(&scoped);
        let mut scoped_args = brazier_intent(&ids.brazier, &ids.wall_north, &ids.gate);
        scoped_args.expected_revision = scoped_rev;
        scoped_args.idempotency_key = 307;
        let denied =
            ScenePlaceTool::invoke(&scoped, scoped_args).expect_err("out-of-scope place must fail");
        assert_eq!(denied.code, "out_of_scope");
        assert!(denied.message.contains(&ids.brazier.to_string()));
        assert_eq!(revision(&scoped), scoped_rev, "denied place writes nothing");
    }

    // MCP8: `scene.brief` (and `scene.spatial`) name every staged object as
    // `name [id]`, index-aligned with the `ids` table, so an agent holding
    // two same-named objects can still tell them apart. The brief text itself
    // is untouched (the assistant transcript golden pins it byte for byte).
    #[test]
    fn brief_labels_every_object_as_name_bracket_id() {
        let (server, _, _) = overlap_scene();
        let brief =
            SceneBriefTool::invoke(&server, SceneBriefArgs::default()).expect("brief reads");
        assert_eq!(brief.objects.len(), brief.ids.len());
        assert_eq!(brief.objects.len(), 4);
        for (index, (label, id)) in brief.objects.iter().zip(brief.ids.iter()).enumerate() {
            let id = ObjectId::parse(id).expect("ids table stays bare UUID text");
            assert_eq!(*label, object_label(&format!("prop_{}", index + 1), &id));
            assert!(
                label.ends_with(&format!("[{id}]")),
                "label carries its id: {label}"
            );
        }
        let report = SceneSpatialTool::invoke(
            &server,
            SceneSpatialArgs {
                checks: None,
                limit: None,
                ..Default::default()
            },
        )
        .expect("spatial reads");
        assert_eq!(
            report.objects, brief.objects,
            "spatial labels match brief labels"
        );
        assert_eq!(report.ids, brief.ids);
    }

    // MCP8: duplicate display names never mis-place. Two labels sharing one
    // name part but bracketing different ids resolve to different nodes, a
    // garbage name with a good bracketed id still resolves by id, and the
    // untouched node never moves.
    #[test]
    fn place_resolves_labels_by_id_never_by_name() {
        let _serial = PLACE_TEST_SERIAL.lock().expect("place tests serialize");
        let room = room_scene();
        let gate_home = world_of(&room.server, &room.gate);
        let brazier_home = world_of(&room.server, &room.brazier);
        // Same name, different bracketed ids: each label moves its own node.
        let cases = [
            (
                format!("crate [{}]", room.brazier),
                room.brazier,
                "next_to wall:north",
            ),
            (
                format!("crate [{}]", room.gate),
                room.gate,
                "next_to wall:south",
            ),
        ];
        for (index, (label, target, want)) in cases.into_iter().enumerate() {
            let rev = revision(&room.server);
            let out = ScenePlaceTool::invoke(
                &room.server,
                ScenePlaceArgs {
                    expected_revision: rev,
                    idempotency_key: 410 + index as u64,
                    object: label.clone(),
                    wants: vec![want.to_string()],
                    delta: None,
                },
            )
            .expect("label form places");
            assert!(out.solved, "label `{label}` must solve: {}", out.reason);
            assert_eq!(
                out.placement.expect("placement rides along").id,
                target.to_string(),
                "label `{label}` moved the wrong node"
            );
        }
        assert_ne!(
            world_of(&room.server, &room.brazier),
            brazier_home,
            "brazier moved"
        );
        assert_ne!(world_of(&room.server, &room.gate), gate_home, "gate moved");
        // Garbage name, good bracketed id: the name part never decides.
        let rev = revision(&room.server);
        let out = ScenePlaceTool::invoke(
            &room.server,
            ScenePlaceArgs {
                expected_revision: rev,
                idempotency_key: 412,
                object: format!("not-a-crate [{}]", room.brazier),
                wants: vec!["next_to wall:east".to_string()],
                delta: None,
            },
        )
        .expect("mismatched name still places by id");
        assert!(out.solved, "id decides, not the name: {}", out.reason);
        assert_eq!(
            out.placement.expect("placement rides along").id,
            room.brazier.to_string()
        );
        // Bracketed garbage is a bad id with the legal shapes attached, and
        // writes nothing.
        let rev = revision(&room.server);
        let refused = ScenePlaceTool::invoke(
            &room.server,
            ScenePlaceArgs {
                expected_revision: rev,
                idempotency_key: 413,
                object: "crate [not-a-uuid]".to_string(),
                wants: vec!["next_to wall:north".to_string()],
                delta: None,
            },
        )
        .expect_err("bracketed garbage must fail");
        assert_eq!(refused.code, "bad_id");
        assert!(
            refused.message.contains("name [uuid]"),
            "got: {}",
            refused.message
        );
        assert_eq!(revision(&room.server), rev);
    }

    // MCP8: the scene-op allow-list. `move` passes; everything else is
    // refused with the legal op plus one recovery. The batch gate behind
    // `scene.place` admits root moves only, so scene tools instantiate,
    // delete, reparent and rebind nothing.
    #[test]
    fn scene_op_allow_list_denies_with_reason() {
        assert_eq!(SCENE_ALLOWED_OPS, &["move"]);
        assert!(check_scene_op(SCENE_PLACE_OP_MOVE).is_ok());
        for denied in [
            "insert",
            "remove",
            "reparent",
            "set_material",
            "spawn",
            "execute",
            "",
            "MOVE",
            "move ",
        ] {
            let refused = check_scene_op(denied).expect_err("non-move scene op must fail");
            assert_eq!(refused.code, "denied_op", "op `{denied}`");
            assert!(
                refused.message.contains("move"),
                "names the legal op: {}",
                refused.message
            );
            assert!(
                refused.message.contains("scene.place") || refused.message.contains("tx.preview"),
                "names one recovery: {}",
                refused.message
            );
        }
        let pose = Transform::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]);
        let batch_of = |op: Op| Batch {
            ops: vec![op],
            base_revision: 0,
            idempotency_key: 420,
            provenance: None,
        };
        let moves = batch_of(Op::SetLocal {
            id: ObjectId::new(),
            local: pose,
        });
        assert!(check_batch_allowed(&moves).is_ok());
        for (name, op) in [
            (
                "insert",
                Op::Insert {
                    id: ObjectId::new(),
                    parent: ObjectId::NIL,
                    local: pose,
                },
            ),
            (
                "remove",
                Op::Remove {
                    id: ObjectId::new(),
                },
            ),
            (
                "reparent",
                Op::Reparent {
                    id: ObjectId::new(),
                    new_parent: ObjectId::NIL,
                },
            ),
            (
                "set_material",
                Op::SetMaterial {
                    id: ObjectId::new(),
                    material: None,
                },
            ),
            (
                "component",
                Op::RemoveComponent {
                    id: ObjectId::new(),
                    kind: engine_scene::component::ComponentKind::Mesh,
                },
            ),
        ] {
            let refused =
                check_batch_allowed(&batch_of(op)).expect_err("scene batch must refuse {name}");
            assert_eq!(refused.code, "denied_op", "op `{name}`");
            assert!(
                refused.message.contains(name),
                "names the denied op: {}",
                refused.message
            );
            assert!(
                refused.message.contains("move"),
                "names the legal op: {}",
                refused.message
            );
        }
    }

    // MCP8: single-flight applies. A held flight guard makes the next scene
    // apply fail fast with `apply_in_flight` (never a queue, never a stall)
    // writing nothing, while a full `play.run` lifecycle still completes on
    // the same server: play jobs never take the scene guard.
    #[test]
    fn scene_apply_single_flight_fails_fast_and_never_stalls_play() {
        let _serial = PLACE_TEST_SERIAL.lock().expect("place tests serialize");
        let room = room_scene();
        let held = SCENE_APPLY_LOCK
            .lock()
            .expect("test holds the flight guard");
        let rev = revision(&room.server);
        let mut args = brazier_intent(&room.brazier, &room.wall_north, &room.gate);
        args.expected_revision = rev;
        let busy =
            ScenePlaceTool::invoke(&room.server, args).expect_err("second flight must fail fast");
        assert_eq!(busy.code, "apply_in_flight");
        assert!(
            busy.message.contains("retry"),
            "names the recovery: {}",
            busy.message
        );
        assert_eq!(revision(&room.server), rev, "refused flight writes nothing");
        // play.run never takes the scene guard: start, progress and result
        // all land while the flight is held.
        let pinned = revision(&room.server);
        let play = |action: &str, job_id: Option<u64>| {
            PlayRunTool::invoke(
                &room.server,
                PlayRunArgs {
                    action: action.to_owned(),
                    seed: Some(7),
                    ticks: Some(8),
                    fighters: Some(2),
                    half_extent_m: Some(20.0),
                    inputs: None,
                    ttl_polls: None,
                    job_id,
                    scene_rev: Some(pinned),
                    timing: None,
                },
            )
        };
        let started = play("start", None).expect("start runs under a held scene guard");
        assert_eq!(started.status, "running");
        let id = started.job_id;
        let mid = play("poll", Some(id)).expect("poll runs under a held scene guard");
        assert_eq!(mid.status, "running");
        let done = play("poll", Some(id)).expect("result lands under a held scene guard");
        assert_eq!(done.status, "completed");
        assert_eq!(done.checkpoints.len(), 8);
        drop(held);
        // After release the flight lands again on the same server.
        let rev = revision(&room.server);
        let mut args = brazier_intent(&room.brazier, &room.wall_north, &room.gate);
        args.expected_revision = rev;
        args.idempotency_key = 414;
        let out = ScenePlaceTool::invoke(&room.server, args).expect("flight lands after release");
        assert!(out.solved, "released flight solves: {}", out.reason);
        assert_eq!(out.revision, rev + 1);
    }

    // Twenty seeded opposite-wall conflicts through the door. Seeds
    // vary the moved object, the wall axis and the gap range; every conflict
    // carries solver hints, and every hint verifies clean when applied on a
    // fresh room (dropping its want solves to exactly the hinted placement).
    #[test]
    fn place_conflict_hints_verify_clean_through_the_door() {
        let _serial = PLACE_TEST_SERIAL.lock().expect("place tests serialize");
        let mut verified = 0u32;
        for seed in 0..20u32 {
            let room = room_scene();
            let rev = revision(&room.server);
            let (first, second) = if (seed / 2) % 2 == 0 {
                ("north", "south")
            } else {
                ("east", "west")
            };
            let hi = 10 + 10 * (seed / 4);
            let mover_is_brazier = seed % 2 == 0;
            let mover = if mover_is_brazier {
                room.brazier
            } else {
                room.gate
            };
            let against = |name: &str| format!("next_to wall:{name} [0,{hi}]");
            let wants = vec![against(first), against(second)];
            let out = ScenePlaceTool::invoke(
                &room.server,
                ScenePlaceArgs {
                    expected_revision: rev,
                    idempotency_key: 1000 + u64::from(seed),
                    object: mover.to_string(),
                    wants: wants.clone(),
                    delta: None,
                },
            )
            .expect("conflicts answer");
            assert!(!out.solved, "seed {seed}: opposite walls never solve");
            assert_eq!(
                out.conflicts, wants,
                "seed {seed}: minimal conflict keeps both wants"
            );
            assert!(!out.hints.is_empty(), "seed {seed}: conflict carries hints");
            assert!(
                out.dry_run.len() >= out.hints.len(),
                "seed {seed}: every hint has a dry-run header"
            );
            assert_eq!(out.revision, rev, "seed {seed}: conflicts write nothing");
            for hint in &out.hints {
                let fresh = room_scene();
                let fresh_rev = revision(&fresh.server);
                let fresh_mover = if mover_is_brazier {
                    fresh.brazier
                } else {
                    fresh.gate
                };
                let mut repair = wants.clone();
                repair.remove(hint.dropped);
                let repaired = ScenePlaceTool::invoke(
                    &fresh.server,
                    ScenePlaceArgs {
                        expected_revision: fresh_rev,
                        idempotency_key: 2000 + u64::from(seed),
                        object: fresh_mover.to_string(),
                        wants: repair,
                        delta: None,
                    },
                )
                .expect("hint repair places");
                assert!(
                    repaired.solved,
                    "seed {seed}: hint {} must solve: {}",
                    hint.dropped, repaired.reason
                );
                let landed = repaired.placement.expect("repair solves");
                assert_eq!(landed.id, fresh_mover.to_string());
                assert_eq!(landed.x, hint.placement.x, "seed {seed}: hint x verifies");
                assert_eq!(landed.y, hint.placement.y, "seed {seed}: hint y verifies");
                assert_eq!(landed.z, hint.placement.z, "seed {seed}: hint z verifies");
                assert_eq!(
                    landed.yaw, hint.placement.yaw,
                    "seed {seed}: hint yaw verifies"
                );
                verified += 1;
            }
        }
        assert!(verified >= 20, "every seed verified at least one hint");
    }

    // Ten seeded dry-run diffs equal the applied diffs through the
    // door. The hint-0 describe rows captured at conflict time must match the
    // graph diff measured after applying that hint on the same scene.
    #[test]
    fn place_hint_dry_run_diff_equals_applied_diff() {
        let _serial = PLACE_TEST_SERIAL.lock().expect("place tests serialize");
        let row_prefix = "dry_run 0 ";
        for seed in 0..10u32 {
            let room = room_scene();
            let rev = revision(&room.server);
            let (first, second) = if (seed / 2) % 2 == 0 {
                ("north", "south")
            } else {
                ("east", "west")
            };
            let hi = 10 + 10 * (seed / 4);
            let mover = if seed % 2 == 0 {
                room.brazier
            } else {
                room.gate
            };
            let against = |name: &str| format!("next_to wall:{name} [0,{hi}]");
            let wants = vec![against(first), against(second)];
            let out = ScenePlaceTool::invoke(
                &room.server,
                ScenePlaceArgs {
                    expected_revision: rev,
                    idempotency_key: 3000 + u64::from(seed),
                    object: mover.to_string(),
                    wants: wants.clone(),
                    delta: None,
                },
            )
            .expect("conflicts answer");
            let hint = out.hints.first().expect("conflict carries hints").clone();
            let dry: Vec<String> = out
                .dry_run
                .iter()
                .filter_map(|line| line.strip_prefix(row_prefix))
                .filter(|rest| !rest.starts_with("drop "))
                .map(str::to_string)
                .collect();
            let state = room.server.lock().expect("test lock holds");
            let before = engine_spatial::build(&snapshot(&state.doc).actors);
            drop(state);
            let mut repair = wants.clone();
            repair.remove(hint.dropped);
            let repaired = ScenePlaceTool::invoke(
                &room.server,
                ScenePlaceArgs {
                    expected_revision: rev,
                    idempotency_key: 3100 + u64::from(seed),
                    object: mover.to_string(),
                    wants: repair,
                    delta: None,
                },
            )
            .expect("hint applies");
            assert!(
                repaired.solved,
                "seed {seed}: hint must solve: {}",
                repaired.reason
            );
            let state = room.server.lock().expect("test lock holds");
            let after = engine_spatial::build(&snapshot(&state.doc).actors);
            drop(state);
            let applied = engine_spatial::describe(&engine_spatial::diff_graph(&before, &after));
            assert_eq!(
                dry, applied,
                "seed {seed}: dry-run rows equal the applied diff"
            );
        }
    }

    // Conflicts write nothing observable — revision, brief text and
    // brief digest are identical before and after, and non-solver refusals
    // (unknown target) carry no hints and no dry-run rows.
    #[test]
    fn place_conflict_keeps_revision_and_brief_digest() {
        let _serial = PLACE_TEST_SERIAL.lock().expect("place tests serialize");
        let room = room_scene();
        let rev = revision(&room.server);
        let before =
            SceneBriefTool::invoke(&room.server, SceneBriefArgs::default()).expect("brief reads");
        let out = ScenePlaceTool::invoke(
            &room.server,
            ScenePlaceArgs {
                expected_revision: rev,
                idempotency_key: 3200,
                object: room.brazier.to_string(),
                wants: vec![
                    "next_to wall:north [0,50]".to_string(),
                    "next_to wall:south [0,50]".to_string(),
                ],
                delta: None,
            },
        )
        .expect("conflicts answer");
        assert!(!out.solved);
        assert!(!out.hints.is_empty(), "solver conflicts carry hints");
        assert!(
            !out.dry_run.is_empty(),
            "solver conflicts carry dry-run rows"
        );
        let after = SceneBriefTool::invoke(&room.server, SceneBriefArgs::default())
            .expect("brief reads again");
        assert_eq!(after.revision, rev, "conflicts write nothing");
        assert_eq!(after.text, before.text, "brief text unchanged");
        assert_eq!(after.digest, before.digest, "brief digest unchanged");
        let unknown = ScenePlaceTool::invoke(
            &room.server,
            ScenePlaceArgs {
                expected_revision: rev,
                idempotency_key: 3201,
                object: room.brazier.to_string(),
                wants: vec![format!("next_to object:{}", ObjectId::new())],
                delta: None,
            },
        )
        .expect("unknown target answers");
        assert!(!unknown.solved);
        assert!(
            unknown.hints.is_empty(),
            "non-solver refusals carry no hints"
        );
        assert!(
            unknown.dry_run.is_empty(),
            "non-solver refusals carry no dry-run rows"
        );
    }

    // Strong units: a placement is meters and radians or it refuses typed.
    #[test]
    fn placed_dto_refuses_bad_poses_typed() {
        let ok = PlacedDto::checked("n".to_owned(), 1.0, 0.5, -99.0, 5.5).expect("sane pose");
        assert_eq!((ok.x, ok.y, ok.z, ok.yaw), (1.0, 0.5, -99.0, 5.5));
        for (x, y, z, yaw) in [
            (f32::NAN, 0.0, 0.0, 0.0),
            (0.0, f32::INFINITY, 0.0, 0.0),
            (0.0, 0.0, 100.5, 0.0),
            (0.0, 0.0, 0.0, f32::NAN),
            (0.0, 0.0, 0.0, 90.0),
        ] {
            let error = PlacedDto::checked("n".to_owned(), x, y, z, yaw)
                .expect_err("bad pose refuses typed");
            assert_eq!(error.code, "bad_pose", "pose ({x}, {y}, {z}) yaw {yaw}");
            assert!(error.message.contains("re-run"), "recovery cue");
        }
    }

    // Roots-only plus yaw round-trip: parented nodes fail loudly with the
    // violated invariant and one recovery, never a silent flatten; yaw
    // extracted from a world quaternion writes back a pure-Y rotation.
    #[test]
    fn parented_place_refuses_loudly_and_yaw_roundtrips() {
        let _serial = PLACE_TEST_SERIAL.lock().expect("place tests serialize");
        assert_eq!(yaw_of_quat([0.0, 0.0, 0.0, 1.0]), 0.0);
        for yaw in [0.0, 0.7, -0.9, 1.2] {
            let quat = quat_of_yaw(yaw);
            assert_eq!(quat[0], 0.0, "pure-Y keeps x at zero");
            assert_eq!(quat[2], 0.0, "pure-Y keeps z at zero");
            let back = yaw_of_quat(quat);
            assert!(
                (back - yaw).abs() <= 1e-5,
                "yaw {yaw} round-trips, got {back}"
            );
        }
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let spun = ObjectId::new();
        let spun_yaw = 0.7;
        let spun_base = doc.revision;
        commands::apply(
            &mut doc,
            &mut journal,
            &Batch {
                ops: vec![Op::Insert {
                    id: spun,
                    parent: ObjectId::NIL,
                    local: Transform::new([1.0, 0.0, 2.0], quat_of_yaw(spun_yaw), [1.0, 1.0, 1.0]),
                }],
                base_revision: spun_base,
                idempotency_key: 500,
                provenance: None,
            },
        )
        .expect("spun seed applies");
        let snap = snapshot(&doc);
        assert_eq!(snap.ids.len(), 1);
        assert!((snap.actors[0].yaw - spun_yaw).abs() <= 1e-5);
        let mut nested_doc = Document::new();
        let mut nested_journal = Journal::new();
        let root = ObjectId::new();
        let child = ObjectId::new();
        for (id, parent, key) in [(root, ObjectId::NIL, 501u64), (child, root, 502u64)] {
            let base = nested_doc.revision;
            commands::apply(
                &mut nested_doc,
                &mut nested_journal,
                &Batch {
                    ops: vec![Op::Insert {
                        id,
                        parent,
                        local: Transform::new(
                            [0.0, 0.0, 0.0],
                            [0.0, 0.0, 0.0, 1.0],
                            [1.0, 1.0, 1.0],
                        ),
                    }],
                    base_revision: base,
                    idempotency_key: key,
                    provenance: None,
                },
            )
            .expect("nested seed applies");
        }
        let nested =
            McpServer::with_document(nested_doc, nested_journal, 7, Capability::unrestricted(7));
        let rev = revision(&nested);
        let refused = ScenePlaceTool::invoke(
            &nested,
            ScenePlaceArgs {
                expected_revision: rev,
                idempotency_key: 503,
                object: child.to_string(),
                wants: vec!["next_to wall:north".to_string()],
                delta: None,
            },
        )
        .expect_err("parented object must fail loudly");
        assert_eq!(refused.code, "not_a_root");
        assert!(
            refused.message.contains("roots only"),
            "got: {}",
            refused.message
        );
        assert!(
            refused.message.contains("reparent"),
            "got: {}",
            refused.message
        );
        assert_eq!(revision(&nested), rev, "loud refusal writes nothing");
        let room = room_scene();
        let rev = revision(&room.server);
        let out = ScenePlaceTool::invoke(
            &room.server,
            ScenePlaceArgs {
                expected_revision: rev,
                idempotency_key: 504,
                object: room.brazier.to_string(),
                wants: vec!["next_to wall:north".to_string()],
                delta: None,
            },
        )
        .expect("root place solves");
        assert!(out.solved, "root place must solve: {}", out.reason);
        let placed = out.placement.expect("solved placement rides along");
        let state = room.server.lock().expect("test lock holds");
        let local = state
            .doc
            .hierarchy
            .local(room.brazier)
            .expect("brazier still staged");
        drop(state);
        assert!(
            local.rotation[0].abs() <= 1e-6,
            "pure-Y write keeps x at zero"
        );
        assert!(
            local.rotation[2].abs() <= 1e-6,
            "pure-Y write keeps z at zero"
        );
        assert!((yaw_of_quat(local.rotation) - placed.yaw).abs() <= 1e-5);
    }

    /// SC2: a run pins the live revision and digest, or refuses with a named
    /// code before any simulation starts.
    #[test]
    fn run_pin_pins_or_refuses_by_name() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let code = |result: Result<ScenePin, McpToolError>| result.expect_err("refuses").code;
        // The empty arena never pins, offered or not.
        assert_eq!(code(run_pin(&doc, None)), "empty_arena");
        assert_eq!(code(run_pin(&doc, Some(0))), "empty_arena");
        let insert = |doc: &mut Document, journal: &mut Journal, key: u64| {
            let batch = Batch {
                ops: vec![Op::Insert {
                    id: ObjectId::new(),
                    parent: ObjectId::NIL,
                    local: Transform::identity(),
                }],
                base_revision: doc.revision,
                idempotency_key: key,
                provenance: None,
            };
            commands::apply(doc, journal, &batch).expect("insert applies");
        };
        insert(&mut doc, &mut journal, 1);
        assert_eq!(code(run_pin(&doc, None)), "unpinned_run");
        let first = run_pin(&doc, Some(doc.revision)).expect("current revision pins");
        assert_eq!(first.revision, doc.revision);
        assert_eq!(first.digest, Scene::encode(&doc.hierarchy).digest());
        // Same bytes, same pin; an edit moves both revision and digest.
        assert_eq!(run_pin(&doc, Some(doc.revision)), Ok(first.clone()));
        insert(&mut doc, &mut journal, 2);
        let stale = run_pin(&doc, Some(first.revision)).expect_err("old revision is stale");
        assert_eq!(stale.code, "stale_scene_rev");
        assert!(stale.message.contains("re-read"), "{}", stale.message);
        let second = run_pin(&doc, Some(doc.revision)).expect("new revision pins");
        assert_ne!(second.digest, first.digest);
    }

    #[test]
    fn spatial_delta_rides_on_scene_place_and_the_flag_skips_it() {
        let _serial = PLACE_TEST_SERIAL.lock().expect("place tests serialize");
        for (skip, key) in [(false, 830u64), (true, 831)] {
            let room = room_scene();
            let mut args = brazier_intent(&room.brazier, &room.wall_north, &room.gate);
            args.expected_revision = revision(&room.server);
            args.idempotency_key = key;
            args.delta = skip.then_some(false);
            let out = ScenePlaceTool::invoke(&room.server, args).expect("place solves");
            assert!(out.solved, "{}", out.reason);
            if skip {
                assert!(out.spatial_delta.is_none());
            } else {
                let delta = out.spatial_delta.expect("delta rides on the placement");
                assert_eq!(delta.ids, vec![room.brazier.to_string()]);
                assert!(delta.new_findings.is_empty() && delta.resolved.is_empty());
            }
        }
    }

    #[test]
    fn spatial_delta_stays_cheap_on_a_200_object_scene() {
        use engine_scene::component::{Collider, ColliderShape, Component};
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let ids: Vec<ObjectId> = (0..200).map(|_| ObjectId::new()).collect();
        let ops = ids
            .iter()
            .enumerate()
            .map(|(n, id)| Op::Insert {
                id: *id,
                parent: ObjectId::NIL,
                local: Transform::new(
                    [(n % 20) as f32 * 3.0, 0.0, (n / 20) as f32 * 3.0],
                    [0.0, 0.0, 0.0, 1.0],
                    [1.0, 1.0, 1.0],
                ),
            })
            .collect();
        let seed = Batch {
            ops,
            base_revision: doc.revision,
            idempotency_key: 900,
            provenance: None,
        };
        commands::apply(&mut doc, &mut journal, &seed).expect("200 cubes apply");
        let before = snapshot(&doc);
        // Widen cube 0 to 6 m: it now overlaps its east neighbour (3 m away).
        let widen = Batch {
            ops: vec![Op::SetComponent {
                id: ids[0],
                component: Component::Collider(Collider {
                    shape: ColliderShape::Box,
                    half: [3.0, 0.5, 0.5],
                    sensor: false,
                }),
            }],
            base_revision: doc.revision,
            idempotency_key: 901,
            provenance: None,
        };
        commands::apply(&mut doc, &mut journal, &widen).expect("collider applies");
        // What the write adds: the second snapshot plus the staged check (the
        // before snapshot is paid the same way), median of 15 runs.
        let mut micros = Vec::new();
        let mut last = None;
        for _ in 0..15 {
            let start = std::time::Instant::now();
            let _prior = snapshot(&doc);
            let after = snapshot(&doc);
            last = Some(spatial_delta(&before, &after, &[ids[0]]));
            micros.push(start.elapsed().as_micros());
        }
        micros.sort_unstable();
        let median = micros[7];
        eprintln!("spatial_delta 200 objects: median {median} us (two snapshots + staged check)");
        let delta = last.expect("ran");
        assert_eq!(delta.new_findings.len(), 1, "{:?}", delta.new_findings);
        assert!(median < 5_000, "delta added {median} us, bar is 5000");
    }

    #[test]
    fn place_conflict_blockers_name_the_taken_spot() {
        let _serial = PLACE_TEST_SERIAL.lock().expect("place tests serialize");
        let room = room_scene();
        let rev = revision(&room.server);
        // The point sits exactly on the gate cube (4, -4): the only pose that
        // meets the want is taken.
        let args = ScenePlaceArgs {
            expected_revision: rev,
            idempotency_key: 840,
            object: room.brazier.to_string(),
            wants: vec!["next_to point:400,-400 [0,0]".to_string()],
            delta: None,
        };
        let out = ScenePlaceTool::invoke(&room.server, args.clone()).expect("conflicts answer");
        assert!(!out.solved);
        assert_eq!(out.revision, rev, "conflicts write nothing");
        assert!(out.reason.contains("minimal conflict"), "{}", out.reason);
        assert!(
            !out.reason.contains(&room.gate.to_string()),
            "reason stays one fixed sentence of counts: {}",
            out.reason
        );
        assert_eq!(out.blocked_by.len(), out.conflicts.len());
        let block = &out.blocked_by[0];
        assert_eq!(out.conflicts[block.want], "next_to point:400,-400 [0,0]");
        assert!(
            block.blocker.ends_with(&format!("[{}]", room.gate)),
            "label names the gate: {}",
            block.blocker
        );
        assert_eq!(block.overlap_cm, 100, "a unit cube on a unit cube");
        assert_eq!(block.miss_cm, 100, "nearest clean pose is one cube away");
        let [x, z] = block.near_miss_cm;
        assert_eq!((x - 400).pow(2) + (z + 400).pow(2), 100 * 100);
        assert!(block.fix.contains("100 cm -x"), "{}", block.fix);
        // Both boxes here are bare nodes, so the figure rests on the labelled
        // default and the fix says so instead of posing as a measurement.
        assert!(
            block.fix.contains("default size"),
            "a default-sized pair is named: {}",
            block.fix
        );
        assert!(
            block.fix.contains("this node"),
            "the mover is one of them: {}",
            block.fix
        );
        let again = ScenePlaceTool::invoke(&room.server, args).expect("conflicts answer again");
        assert_eq!(again.blocked_by, out.blocked_by, "x2 identical");
        assert_eq!(revision(&room.server), rev);
    }

    #[test]
    fn place_conflict_blockers_name_a_wall_and_a_miss_on_twenty_seeds() {
        let _serial = PLACE_TEST_SERIAL.lock().expect("place tests serialize");
        for seed in 0..20u32 {
            let room = room_scene();
            let rev = revision(&room.server);
            let (first, second) = if (seed / 2) % 2 == 0 {
                ("north", "south")
            } else {
                ("east", "west")
            };
            let hi = 10 + 10 * (seed / 4);
            let mover = if seed % 2 == 0 {
                room.brazier
            } else {
                room.gate
            };
            let against = |name: &str| format!("next_to wall:{name} [0,{hi}]");
            let wants = vec![against(first), against(second)];
            let out = ScenePlaceTool::invoke(
                &room.server,
                ScenePlaceArgs {
                    expected_revision: rev,
                    idempotency_key: 4000 + u64::from(seed),
                    object: mover.to_string(),
                    wants: wants.clone(),
                    delta: None,
                },
            )
            .expect("conflicts answer");
            assert!(!out.solved, "seed {seed}");
            assert_eq!(out.blocked_by.len(), 2, "seed {seed}: one per unmet want");
            for (n, block) in out.blocked_by.iter().enumerate() {
                assert_eq!(block.want, n, "seed {seed}");
                assert_eq!(out.conflicts[block.want], wants[n], "seed {seed}");
                assert_eq!(
                    block.overlap_cm, 0,
                    "seed {seed}: opposite walls contradict"
                );
                let word = if n == 0 { first } else { second };
                assert_eq!(block.blocker, format!("wall {word}"), "seed {seed}");
                assert!(block.miss_cm > 0, "seed {seed}: a miss in cm");
                assert!(block.fix.contains("cm"), "seed {seed}: {}", block.fix);
            }
        }
    }
}
