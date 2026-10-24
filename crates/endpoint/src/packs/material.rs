//! `material` pack: the one material record an AI writes as text.
//!
//! Ten verbs over the shipped records (`MaterialSet::shipped`: the fourteen
//! siege materials and the reference set):
//!
//! - `material.list` (observe): every record with its wire id, digests and
//!   shader structure key.
//! - `material.get` (observe): one record as canonical text, to copy and edit.
//! - `material.schema` (observe): the JSON Schema of a record (at most 8 KB).
//! - `material.preview` (observe): parse record text (it may `extends` a
//!   shipped record), then report both digests, the compiled shader's
//!   structure key and size, and the lobes and layers used. A bad record is
//!   refused with the field path and nothing changes: this is the loop an AI
//!   edits in.
//! - `material.param` (observe): what-if of one dotted path edit
//!   (`look.specular_roughness`, `layers[0].darken`) on a shipped record,
//!   returning the undo value and the new digests; the sim digest moves only
//!   when a body field moved.
//! - `material.compare` (observe): two records (a shipped name or record text
//!   each) side by side: CIEDE2000 between the base colors, luminance and
//!   highlight ratios, roughness and metalness differences, and the sim half
//!   apart (toughness, sound, shard), so a look edit never hides a gameplay one.
//! - `material.set` (commit): make one node use a shipped record, look and
//!   physics together: the `material` component takes the record name and, when
//!   the node has a `destructible` component, its physical material follows,
//!   in one previewed, capability-gated batch (the same path as `tx.apply`), so
//!   one undo restores both.
//! - `material.graph.set` (commit): store a typed procedural graph (nodes:
//!   noise, tile, gradient, curve, warp, blend) and fold it into the record:
//!   base color, roughness, metalness, the five coverage channels, a height
//!   field and four texture refs. It returns the graph id (the digest of the
//!   canonical text) and the folded record's look digest, so the same graph
//!   twice is a no-op with the same digest.
//! - `material.graph.get` (observe): one stored graph back as canonical text.
//! - `material.bake` (commit): sample a graph at 512 or 1024 into four PNGs
//!   (base_color, orm, normal, emissive) in `out_dir`, with each map's digest,
//!   bytes and per-channel min/max; a look map flat on every channel is
//!   refused. `coverage: true` also writes the five coverage channel maps and
//!   the 4+1 packed sheets (`coverage`: wetness, dirt, blood, strain in RGBA;
//!   `coverage_extra`: heat in red); a flat coverage channel is written and
//!   named in `warnings` instead of refused. A build with the `intake` feature
//!   also cooks each map to KTX2 and reports the copy's bytes and digest.
//!
//! The record table has no document of its own yet, so an edit lives in the
//! text the caller holds; `material.set` is the one verb that writes the
//! scene, and a graph lives in this session's table until the door restarts.
//! Units: unit-range factors, meters, nits (see `material.schema`).
//! Determinism: the same text gives the same digests on every machine (the
//! record codec is integer-exact for sim fields and canonical for look
//! fields; a bake is integer-hash noise plus stored-block PNG, so the same
//! graph gives the same bytes). Errors: [`McpToolError`] codes `bad_record`
//! (the message starts with the field path), `unknown_material`, `bad_edit`,
//! `bad_graph` and `unknown_graph`, each naming the legal values.

use engine_material::compile::{compile, lobe, slot, wgsl_digest, Structure};
use engine_material::{MaterialRecord, MaterialSet, SCHEMA};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};
use rmcp::serde_json::json;

use super::{PackDef, PackError, VerbSpec};
use crate::mcp::{McpToolError, OpDto, TxApplyArgs, TxPreviewArgs};
use crate::tool_capabilities::CapabilityCard;
use engine_scene::component::{Component, ComponentKind};

/// Pack name.
pub const PACK: &str = "material";
/// Verb: list the shipped records.
pub const TOOL_MATERIAL_LIST: &str = "material.list";
/// Verb: one shipped record as canonical text.
pub const TOOL_MATERIAL_GET: &str = "material.get";
/// Verb: the record JSON Schema.
pub const TOOL_MATERIAL_SCHEMA: &str = "material.schema";
/// Verb: validate and describe record text.
pub const TOOL_MATERIAL_PREVIEW: &str = "material.preview";
/// Verb: what-if of one parameter edit.
pub const TOOL_MATERIAL_PARAM: &str = "material.param";
/// Verb: two records side by side.
pub const TOOL_MATERIAL_COMPARE: &str = "material.compare";
/// Verb: make a node use a shipped record (look and physics).
pub const TOOL_MATERIAL_SET: &str = "material.set";
/// Verb: store a typed procedural graph and report the record it folds into.
pub const TOOL_MATERIAL_GRAPH_SET: &str = "material.graph.set";
/// Verb: read a stored graph back as canonical text.
pub const TOOL_MATERIAL_GRAPH_GET: &str = "material.graph.get";
/// Verb: bake a graph into four PNG maps.
pub const TOOL_MATERIAL_BAKE: &str = "material.bake";

/// Longest record text accepted by `material.preview`, bytes.
pub const MAX_RECORD_TEXT: usize = 16 * 1024;

/// One record in a listing.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialInfo {
    /// Record name.
    pub name: String,
    /// Wire id: the position bond arrays carry.
    pub wire_id: u8,
    /// Lowercase hex digest of the sim half (body, sound, fx).
    pub sim_digest: String,
    /// Lowercase hex digest of the look half (look, layers).
    pub look_digest: String,
    /// Lowercase hex shader structure key: equal keys share one pipeline.
    pub structure_key: String,
}

/// Args for `material.list`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialListArgs {
    /// Only records whose name starts with this text (default: all).
    #[serde(default)]
    pub prefix: Option<String>,
}

/// Result of `material.list`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialListOutput {
    /// Records in wire order.
    pub materials: Vec<MaterialInfo>,
    /// Set-wide sim digest.
    pub sim_digest: String,
    /// Set-wide look digest.
    pub look_digest: String,
}

/// Args for `material.get`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialGetArgs {
    /// Record name from `material.list`.
    pub name: String,
}

/// Result of `material.get`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialGetOutput {
    /// Canonical JSON text of the record.
    pub text: String,
    /// The record's listing entry.
    pub info: MaterialInfo,
}

/// Args for `material.schema`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialSchemaArgs {
    /// One top-level part of the record (`look`, `layers`, `body`, ...)
    /// instead of the whole schema.
    #[serde(default)]
    pub part: Option<String>,
}

/// Result of `material.schema`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialSchemaOutput {
    /// The JSON Schema text (draft 2020-12).
    pub schema: String,
}

/// Args for `material.preview`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialPreviewArgs {
    /// Record JSON text (at most 16 KB). It may name a shipped record in
    /// `extends` and then only write what differs.
    pub record: String,
}

/// Result of `material.preview`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialPreviewOutput {
    /// Record name.
    pub name: String,
    /// Canonical text: what parsing then writing the record gives.
    pub canonical: String,
    /// Sim digest (hex).
    pub sim_digest: String,
    /// Look digest (hex).
    pub look_digest: String,
    /// Shader structure key (hex).
    pub structure_key: String,
    /// Lobes the shader uses (`coat`, `sheen`, ...), including the ones a
    /// wet or blood layer adds.
    pub lobes: Vec<String>,
    /// Texture slots the shader samples.
    pub textures: Vec<String>,
    /// Gameplay layers in order (`wet<-wetness`).
    pub layers: Vec<String>,
    /// Generated WGSL size in bytes.
    pub wgsl_bytes: u32,
    /// Generated WGSL digest (hex): equal text gives equal digest.
    pub wgsl_digest: String,
    /// True when the surface draws in the blended pass (transmission).
    pub blended: bool,
}

/// Args for `material.param`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialParamArgs {
    /// Shipped record name.
    pub name: String,
    /// Dotted path (`look.specular_roughness`, `layers[0].darken`,
    /// `body.toughness`, `sound.impact`).
    pub path: String,
    /// New value as JSON text (`0.08`, `[0.3,0.1,0.1]`, `"metal"`, `null`
    /// to clear a lobe).
    pub value: String,
}

/// Result of `material.param`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialParamOutput {
    /// Value before, as JSON text (the undo value).
    pub before: String,
    /// Value after, as JSON text.
    pub after: String,
    /// The record after the edit.
    pub info: MaterialInfo,
    /// Sim digest before the edit.
    pub sim_digest_before: String,
    /// True when the shader structure changed (a new pipeline is needed).
    pub structure_changed: bool,
}

/// Args for `material.compare`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialCompareArgs {
    /// Record `a`: a shipped name, or record JSON text (starts with `{`).
    pub a: String,
    /// Record `b`, same forms. The deltas are `b` against `a`.
    pub b: String,
}

/// Result of `material.compare`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialCompareOutput {
    /// Name of record `a`.
    pub a: String,
    /// Name of record `b`.
    pub b: String,
    /// CIEDE2000 between the base colors (about 1 is just noticeable).
    pub delta_e: f64,
    /// Relative luminance of `b` over `a`.
    pub luminance_ratio: f64,
    /// Brightest-highlight ratio, `b` over `a`.
    pub specular_peak_ratio: f64,
    /// `b - a` perceptual roughness.
    pub roughness: f32,
    /// `b - a` metalness.
    pub metalness: f32,
    /// `b - a` coat weight.
    pub coat_weight: f32,
    /// `b - a` index of refraction.
    pub ior: f32,
    /// `b - a` emission in nits.
    pub emission_nits: f32,
    /// `b - a` toughness.
    pub toughness: f64,
    /// `b - a` brittleness.
    pub brittleness: f64,
    /// True when the impact sound differs.
    pub sound_changed: bool,
    /// True when the shard rule differs.
    pub shard_changed: bool,
    /// True when anything the tick reads differs (sim digest).
    pub sim_changed: bool,
    /// True when anything a frame reads differs (look digest).
    pub look_changed: bool,
    /// True when the shaders need different pipelines.
    pub structure_changed: bool,
}

/// Args for `material.set`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialSetArgs {
    /// Node (UUID text) that receives the `material` component.
    pub node: String,
    /// Shipped record name.
    pub material: String,
    /// Linear RGB tint, each channel in 0..=1; default white.
    #[serde(default)]
    pub tint: Option<[f32; 3]>,
    /// Must equal the document revision, else nothing runs.
    pub expected_revision: u64,
    /// Retry identity of the committed batch.
    pub idempotency_key: u64,
}

/// Result of `material.set`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialSetOutput {
    /// Record now referenced by the node.
    pub material: String,
    /// True when the node's `destructible` physical material followed.
    pub physics_followed: bool,
    /// Document revision after the commit.
    pub revision: u64,
    /// The batch effect lines.
    pub effects: Vec<String>,
}

/// Longest graph text the graph verbs accept, bytes.
pub const MAX_GRAPH_TEXT: usize = engine_material::graph::MAX_GRAPH_BYTES;

/// Args for `material.graph.set`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialGraphSetArgs {
    /// Graph JSON text (at most 16 KB): nodes, output fields and layers.
    pub graph: String,
}

/// Result of `material.graph.set`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialGraphSetOutput {
    /// Graph id: the digest of the canonical graph text.
    pub id: String,
    /// Material name the folded record takes.
    pub material: String,
    /// Shipped record the folded record extends.
    pub base: String,
    /// Canonical graph text (what `material.graph.get` returns).
    pub canonical: String,
    /// Node count.
    pub nodes: u32,
    /// Layer count the record carries.
    pub layers: u32,
    /// Coverage channels the graph drives, by name.
    pub channels: Vec<String>,
    /// Texture slots the folded record fills.
    pub textures: Vec<String>,
    /// Look digest of the folded record (hex).
    pub record_digest: String,
    /// Sim digest of the folded record (hex): the parent's, never moved.
    pub sim_digest: String,
    /// False when the same graph was already set: the second call is a no-op.
    pub applied: bool,
}

/// Args for `material.graph.get`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialGraphGetArgs {
    /// Graph id from `material.graph.set`.
    #[serde(default)]
    pub id: Option<String>,
    /// Graph JSON text, instead of an id.
    #[serde(default)]
    pub graph: Option<String>,
}

/// Result of `material.graph.get`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialGraphGetOutput {
    /// Graph id: the digest of the canonical text.
    pub id: String,
    /// Material name the folded record takes.
    pub material: String,
    /// Shipped record the folded record extends.
    pub base: String,
    /// Node count.
    pub nodes: u32,
    /// Canonical graph text, to edit and set again.
    pub canonical: String,
    /// Look digest of the folded record (hex).
    pub record_digest: String,
}

/// Args for `material.bake`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialBakeArgs {
    /// Graph id from `material.graph.set`.
    #[serde(default)]
    pub id: Option<String>,
    /// Graph JSON text, instead of an id.
    #[serde(default)]
    pub graph: Option<String>,
    /// Map side in pixels: 512 or 1024; default 512.
    #[serde(default)]
    pub resolution: Option<u32>,
    /// Also bake the five coverage channel maps and the two packed sheets;
    /// default false. A channel the graph drives flat is written anyway and
    /// named in `warnings`.
    #[serde(default)]
    pub coverage: bool,
    /// Folder the PNGs are written into (created when absent).
    pub out_dir: String,
}

/// One baked map in a bake receipt.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialBakeMap {
    /// Map name: `base_color`, `orm`, `normal` or `emissive`.
    pub name: String,
    /// Written file path.
    pub path: String,
    /// Digest of the raw samples (hex): equal graphs give equal digests.
    pub digest: String,
    /// File size, bytes.
    pub bytes: u64,
    /// Smallest value per channel (R, G, B, A) in 0..=1.
    pub min: [f32; 4],
    /// Largest value per channel (R, G, B, A) in 0..=1.
    pub max: [f32; 4],
    /// Size of the KTX2 copy of this map, bytes; `None` when this build has no
    /// cook path (the `intake` feature carries it).
    #[serde(default)]
    pub ktx2_bytes: Option<u64>,
    /// Digest of the KTX2 copy (hex): the same map twice cooks equal bytes.
    #[serde(default)]
    pub ktx2_digest: Option<String>,
}

/// Result of `material.bake`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialBakeOutput {
    /// Graph id.
    pub graph: String,
    /// Material name the graph folds into.
    pub material: String,
    /// Map side in pixels.
    pub resolution: u32,
    /// The four maps in write order.
    pub maps: Vec<MaterialBakeMap>,
    /// The coverage set when `coverage` was on: the five channel maps then the
    /// two packed sheets (`coverage`: wetness/dirt/blood/strain in RGBA,
    /// `coverage_heat`: heat in red). Empty when the bake skipped coverage.
    pub coverage: Vec<MaterialBakeMap>,
    /// Warning rows: channels that came out flat on every sample, by map name.
    /// A flat coverage channel is written and reported, never refused.
    pub warnings: Vec<String>,
    /// Look digest of the folded record (hex).
    pub record_digest: String,
    /// Texture references the folded record now names.
    pub textures: Vec<String>,
}

/// This session's graphs: material id to graph id to graph.
type GraphTable =
    std::collections::BTreeMap<u64, std::collections::BTreeMap<u64, engine_material::graph::Graph>>;

/// This session's graphs by id, so `graph.get` and `bake` can name one.
fn graphs() -> &'static std::sync::Mutex<GraphTable> {
    static GRAPHS: std::sync::OnceLock<std::sync::Mutex<GraphTable>> = std::sync::OnceLock::new();
    GRAPHS.get_or_init(std::sync::Mutex::default)
}

fn lock_graphs() -> Result<std::sync::MutexGuard<'static, GraphTable>, McpToolError> {
    graphs().lock().map_err(|_| {
        McpToolError::new(
            "lock_poisoned",
            "material graph table is poisoned; restart the door",
        )
    })
}

/// A graph refusal names the path, the typed kind and one recovery step.
fn bad_graph(error: engine_material::graph::GraphError) -> McpToolError {
    McpToolError::new(
        "bad_graph",
        format!(
            "{error}; next: fix the path the message names, or api.describe \
             material.graph.set for the graph shape"
        ),
    )
    .with_data(json!({ "path": error.path }))
}

fn parse_graph(text: &str) -> Result<engine_material::graph::Graph, McpToolError> {
    engine_material::graph::Graph::parse(text).map_err(bad_graph)
}

/// Fold a graph against the shipped record its `base` names.
fn fold_graph(graph: &engine_material::graph::Graph) -> Result<MaterialRecord, McpToolError> {
    let set = MaterialSet::shipped();
    let parent = set
        .get(&graph.base)
        .ok_or_else(|| unknown(&graph.base, &set))?;
    graph.to_record(parent).map_err(bad_graph)
}

fn material_graph_set(
    server: &crate::mcp::McpServer,
    args: &MaterialGraphSetArgs,
) -> Result<MaterialGraphSetOutput, McpToolError> {
    let graph = parse_graph(&args.graph)?;
    let record = fold_graph(&graph)?;
    let id = format!("{:016x}", graph.digest());
    let session = server.dispatch(TOOL_MATERIAL_GRAPH_SET, |state| Ok(state.session()))?;
    let applied = {
        let mut table = lock_graphs()?;
        let entry = table.entry(session).or_default();
        let key = graph.digest();
        let first = !entry.contains_key(&key);
        entry.insert(key, graph.clone());
        first
    };
    Ok(MaterialGraphSetOutput {
        id,
        material: graph.material.clone(),
        base: graph.base.clone(),
        canonical: graph.canonical(),
        nodes: u32::try_from(graph.nodes.len()).unwrap_or(u32::MAX),
        layers: u32::try_from(record.layers.len()).unwrap_or(u32::MAX),
        channels: graph
            .output
            .channels
            .keys()
            .map(|channel| channel.name().to_owned())
            .collect(),
        textures: [
            record.look.textures.base_color.clone(),
            record.look.textures.orm.clone(),
            record.look.textures.normal.clone(),
            record.look.textures.emissive.clone(),
        ]
        .into_iter()
        .flatten()
        .collect(),
        record_digest: format!("{:016x}", record.look_digest()),
        sim_digest: format!("{:016x}", record.sim_digest()),
        applied,
    })
}

fn material_graph_get(
    server: &crate::mcp::McpServer,
    args: &MaterialGraphGetArgs,
) -> Result<MaterialGraphGetOutput, McpToolError> {
    let graph = resolve_graph(server, TOOL_MATERIAL_GRAPH_GET, &args.id, &args.graph)?;
    let record = fold_graph(&graph)?;
    Ok(MaterialGraphGetOutput {
        id: format!("{:016x}", graph.digest()),
        material: graph.material.clone(),
        base: graph.base.clone(),
        nodes: u32::try_from(graph.nodes.len()).unwrap_or(u32::MAX),
        canonical: graph.canonical(),
        record_digest: format!("{:016x}", record.look_digest()),
    })
}

/// A stored graph by id, or the graph text the caller sent.
fn resolve_graph(
    server: &crate::mcp::McpServer,
    tool: &str,
    id: &Option<String>,
    text: &Option<String>,
) -> Result<engine_material::graph::Graph, McpToolError> {
    if let Some(text) = text {
        return parse_graph(text);
    }
    let Some(id) = id else {
        return Err(McpToolError::new(
            "bad_graph",
            "name a graph with `id` from material.graph.set, or pass `graph` text",
        ));
    };
    let key = u64::from_str_radix(id.trim(), 16).map_err(|_| {
        McpToolError::new(
            "unknown_graph",
            format!("`{id}` is not a graph id; the ids are hex digests from material.graph.set"),
        )
    })?;
    let session = server.dispatch(tool, |state| Ok(state.session()))?;
    let table = lock_graphs()?;
    table
        .get(&session)
        .and_then(|graphs| graphs.get(&key))
        .cloned()
        .ok_or_else(|| {
            McpToolError::new(
                "unknown_graph",
                format!("no graph `{id}` in this session; set it with material.graph.set first"),
            )
        })
}

fn material_bake(
    server: &crate::mcp::McpServer,
    args: &MaterialBakeArgs,
) -> Result<MaterialBakeOutput, McpToolError> {
    let graph = resolve_graph(server, TOOL_MATERIAL_BAKE, &args.id, &args.graph)?;
    let record = fold_graph(&graph)?;
    let resolution = args.resolution.unwrap_or(512);
    let baked = graph.bake(resolution).map_err(bad_graph)?;
    let dir = std::path::Path::new(&args.out_dir);
    std::fs::create_dir_all(dir).map_err(|error| {
        McpToolError::new(
            "bake_io",
            format!(
                "cannot make `{}`: {error}; next: name an out_dir this process may write",
                dir.display()
            ),
        )
    })?;
    let mut maps = Vec::with_capacity(baked.maps.len());
    for map in &baked.maps {
        maps.push(write_map(dir, &graph.material, map)?);
    }
    let mut coverage = Vec::new();
    let mut warnings = Vec::new();
    if args.coverage {
        let set = graph.bake_coverage(resolution).map_err(bad_graph)?;
        for map in set.maps.iter().chain(set.sheets.iter()) {
            coverage.push(write_map(dir, &graph.material, map)?);
        }
        warnings.extend(set.flat.iter().map(|name| {
            format!(
                "{name} is flat on every sample: the graph drives it with nothing; \
                 next: point output.channels at a node for that channel, or leave it flat"
            )
        }));
    }
    let textures: Vec<String> = [
        record.look.textures.base_color.clone(),
        record.look.textures.orm.clone(),
        record.look.textures.normal.clone(),
        record.look.textures.emissive.clone(),
    ]
    .into_iter()
    .flatten()
    .collect();
    Ok(MaterialBakeOutput {
        graph: format!("{:016x}", baked.graph),
        material: graph.material.clone(),
        resolution,
        maps,
        coverage,
        warnings,
        record_digest: format!("{:016x}", record.look_digest()),
        textures,
    })
}

/// Write one baked map as `{material}_{name}.png` and receipt it.
fn write_map(
    dir: &std::path::Path,
    material: &str,
    map: &engine_material::graph::BakeMap,
) -> Result<MaterialBakeMap, McpToolError> {
    let path = dir.join(format!("{material}_{}.png", map.name));
    std::fs::write(&path, &map.png).map_err(|error| {
        McpToolError::new(
            "bake_io",
            format!(
                "cannot write `{}`: {error}; next: name a writable out_dir",
                path.display()
            ),
        )
    })?;
    let ktx2 = ktx2_receipt(map)?;
    Ok(MaterialBakeMap {
        name: map.name.to_owned(),
        path: path.to_string_lossy().into_owned(),
        digest: format!("{:016x}", map.digest),
        bytes: u64::try_from(map.png.len()).unwrap_or(u64::MAX),
        min: map.min,
        max: map.max,
        ktx2_bytes: ktx2.as_ref().map(|(bytes, _)| *bytes),
        ktx2_digest: ktx2.map(|(_, digest)| digest),
    })
}

/// The KTX2 copy of one map, when this build carries the cook path.
///
/// The cook is `engine_assets::ktx2_cook::cook_ktx2`: BC blocks, mips and the
/// KTX2 container, all CPU and deterministic. It rides the `intake` feature
/// because `engine-assets` is only a dependency of the door under that
/// feature; a plain `mcp` build reports `None` for both KTX2 fields.
#[cfg(feature = "intake")]
fn ktx2_receipt(
    map: &engine_material::graph::BakeMap,
) -> Result<Option<(u64, String)>, McpToolError> {
    use engine_assets::ktx2_cook::{cook_ktx2, MapKind};
    // The cook knows the look shapes; the coverage and emissive maps are plain
    // color data, so they take the base-color shape.
    let kind = match map.name {
        "normal" => MapKind::Normal,
        "orm" => MapKind::Orm,
        _ => MapKind::BaseColor,
    };
    let bytes = cook_ktx2(&map.rgba, map.width, map.height, kind, 0).map_err(|error| {
        McpToolError::new(
            "bake_io",
            format!(
                "cannot cook `{}` to KTX2: {error}; next: bake at 512 or 1024, \
                 the sizes the cook accepts",
                map.name
            ),
        )
    })?;
    let digest = format!("{:016x}", engine_core::hash_bytes(&bytes));
    Ok(Some((
        u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        digest,
    )))
}

/// No cook path in this build: the KTX2 fields stay empty.
#[cfg(not(feature = "intake"))]
fn ktx2_receipt(
    _map: &engine_material::graph::BakeMap,
) -> Result<Option<(u64, String)>, McpToolError> {
    Ok(None)
}

fn info_of(set: &MaterialSet, record: &MaterialRecord) -> MaterialInfo {
    MaterialInfo {
        name: record.name.clone(),
        wire_id: set.wire_id(&record.name).unwrap_or(u8::MAX),
        sim_digest: format!("{:016x}", record.sim_digest()),
        look_digest: format!("{:016x}", record.look_digest()),
        structure_key: format!("{:016x}", Structure::of(record).key()),
    }
}

fn unknown(name: &str, set: &MaterialSet) -> McpToolError {
    let legal: Vec<&str> = set.records().iter().map(|r| r.name.as_str()).collect();
    McpToolError::new(
        "unknown_material",
        format!(
            "no material `{name}`; the shipped ones are {}",
            legal.join(", ")
        ),
    )
}

fn material_list(
    _server: &crate::mcp::McpServer,
    args: &MaterialListArgs,
) -> Result<MaterialListOutput, McpToolError> {
    let set = MaterialSet::shipped();
    let prefix = args.prefix.as_deref().unwrap_or("");
    Ok(MaterialListOutput {
        materials: set
            .records()
            .iter()
            .filter(|r| r.name.starts_with(prefix))
            .map(|r| info_of(&set, r))
            .collect(),
        sim_digest: format!("{:016x}", set.sim_digest()),
        look_digest: format!("{:016x}", set.look_digest()),
    })
}

fn material_get(
    _server: &crate::mcp::McpServer,
    args: &MaterialGetArgs,
) -> Result<MaterialGetOutput, McpToolError> {
    let set = MaterialSet::shipped();
    let record = set
        .get(&args.name)
        .ok_or_else(|| unknown(&args.name, &set))?;
    Ok(MaterialGetOutput {
        text: record.to_text(),
        info: info_of(&set, record),
    })
}

fn material_schema(
    _server: &crate::mcp::McpServer,
    args: &MaterialSchemaArgs,
) -> Result<MaterialSchemaOutput, McpToolError> {
    let Some(part) = args.part.as_deref() else {
        return Ok(MaterialSchemaOutput {
            schema: SCHEMA.to_owned(),
        });
    };
    let doc: rmcp::serde_json::Value = rmcp::serde_json::from_str(SCHEMA)
        .map_err(|e| McpToolError::new("internal", format!("shipped schema is not JSON: {e}")))?;
    let props = doc.get("properties").and_then(|p| p.as_object());
    match props.and_then(|p| p.get(part)) {
        Some(found) => {
            // A part that points into `$defs` is returned as the definition.
            let resolved = found
                .get("$ref")
                .and_then(|r| r.as_str())
                .and_then(|r| r.strip_prefix("#/$defs/"))
                .and_then(|name| doc.get("$defs")?.get(name))
                .unwrap_or(found);
            Ok(MaterialSchemaOutput {
                schema: resolved.to_string(),
            })
        }
        None => {
            let legal: Vec<&str> =
                props.map_or_else(Vec::new, |p| p.keys().map(String::as_str).collect());
            Err(McpToolError::new(
                "bad_part",
                format!(
                    "no schema part `{part}`; the parts are {}",
                    legal.join(", ")
                ),
            ))
        }
    }
}

/// The `extends` name of a record text, read without a full parse.
fn extends_of(text: &str) -> Option<String> {
    let doc: rmcp::serde_json::Value = rmcp::serde_json::from_str(text).ok()?;
    doc.get("extends")?.as_str().map(str::to_owned)
}

fn describe(record: &MaterialRecord) -> MaterialPreviewOutput {
    let compiled = compile(record);
    let structure = &compiled.structure;
    let mut lobes = Vec::new();
    for (bit, name) in [
        (lobe::COAT, "coat"),
        (lobe::SHEEN, "sheen"),
        (lobe::ANISOTROPY, "anisotropy"),
        (lobe::TRANSMISSION, "transmission"),
        (lobe::SUBSURFACE, "subsurface"),
        (lobe::EMISSION, "emission"),
    ] {
        if structure.lobes & bit != 0 {
            lobes.push(name.to_owned());
        }
    }
    let mut textures = Vec::new();
    for (bit, name) in [
        (slot::BASE_COLOR, "base_color"),
        (slot::ORM, "orm"),
        (slot::NORMAL, "normal"),
        (slot::EMISSIVE, "emissive"),
    ] {
        if structure.textures & bit != 0 {
            textures.push(name.to_owned());
        }
    }
    MaterialPreviewOutput {
        name: record.name.clone(),
        canonical: record.to_text(),
        sim_digest: format!("{:016x}", record.sim_digest()),
        look_digest: format!("{:016x}", record.look_digest()),
        structure_key: format!("{:016x}", compiled.key),
        lobes,
        textures,
        layers: structure
            .layers
            .iter()
            .map(|(kind, channel)| format!("{}<-{}", kind.name(), channel.name()))
            .collect(),
        wgsl_bytes: u32::try_from(compiled.wgsl.len()).unwrap_or(u32::MAX),
        wgsl_digest: format!("{:016x}", wgsl_digest(&compiled.wgsl)),
        blended: structure.blended(),
    }
}

fn material_preview(
    _server: &crate::mcp::McpServer,
    args: &MaterialPreviewArgs,
) -> Result<MaterialPreviewOutput, McpToolError> {
    if args.record.len() > MAX_RECORD_TEXT {
        return Err(McpToolError::new(
            "bad_record",
            format!(
                "$: record text is {} bytes; the limit is {MAX_RECORD_TEXT}",
                args.record.len()
            ),
        ));
    }
    let set = MaterialSet::shipped();
    let parent = match extends_of(&args.record) {
        Some(name) => Some(set.get(&name).ok_or_else(|| unknown(&name, &set))?),
        None => None,
    };
    let record = MaterialRecord::parse_with(&args.record, parent).map_err(|e| {
        McpToolError::new("bad_record", format!("{}: {}", e.path, e))
            .with_data(json!({ "path": e.path }))
    })?;
    Ok(describe(&record))
}

fn material_param(
    _server: &crate::mcp::McpServer,
    args: &MaterialParamArgs,
) -> Result<MaterialParamOutput, McpToolError> {
    let mut set = MaterialSet::shipped();
    let before_record = set
        .get(&args.name)
        .cloned()
        .ok_or_else(|| unknown(&args.name, &set))?;
    let edit = set
        .set_param(&args.name, &args.path, &args.value)
        .map_err(|e| {
            McpToolError::new("bad_edit", format!("{}: {}", e.path, e))
                .with_data(json!({ "path": e.path }))
        })?;
    let after = set
        .get(&args.name)
        .ok_or_else(|| unknown(&args.name, &set))?;
    Ok(MaterialParamOutput {
        before: edit.before,
        after: edit.after,
        info: info_of(&set, after),
        sim_digest_before: format!("{:016x}", before_record.sim_digest()),
        structure_changed: Structure::of(&before_record) != Structure::of(after),
    })
}

/// A record named by a shipped name or given as JSON text.
fn resolve_record(set: &MaterialSet, spec: &str) -> Result<MaterialRecord, McpToolError> {
    if !spec.trim_start().starts_with('{') {
        return set.get(spec).cloned().ok_or_else(|| unknown(spec, set));
    }
    if spec.len() > MAX_RECORD_TEXT {
        return Err(McpToolError::new(
            "bad_record",
            format!(
                "$: record text is {} bytes; the limit is {MAX_RECORD_TEXT}",
                spec.len()
            ),
        ));
    }
    let parent = match extends_of(spec) {
        Some(name) => Some(set.get(&name).ok_or_else(|| unknown(&name, set))?),
        None => None,
    };
    MaterialRecord::parse_with(spec, parent).map_err(|e| {
        McpToolError::new("bad_record", format!("{}: {}", e.path, e))
            .with_data(json!({ "path": e.path }))
    })
}

fn material_compare(
    _server: &crate::mcp::McpServer,
    args: &MaterialCompareArgs,
) -> Result<MaterialCompareOutput, McpToolError> {
    let set = MaterialSet::shipped();
    let a = resolve_record(&set, &args.a)?;
    let b = resolve_record(&set, &args.b)?;
    let d = engine_material::compare::compare(&a, &b);
    Ok(MaterialCompareOutput {
        a: a.name,
        b: b.name,
        delta_e: d.delta_e,
        luminance_ratio: d.luminance_ratio,
        specular_peak_ratio: d.specular_peak_ratio,
        roughness: d.roughness,
        metalness: d.metalness,
        coat_weight: d.coat_weight,
        ior: d.ior,
        emission_nits: d.emission_nits,
        toughness: d.toughness,
        brittleness: d.brittleness,
        sound_changed: d.sound_changed,
        shard_changed: d.shard_changed,
        sim_changed: d.sim_changed,
        look_changed: d.look_changed,
        structure_changed: d.structure_changed,
    })
}

fn material_set(
    server: &crate::mcp::McpServer,
    args: &MaterialSetArgs,
) -> Result<MaterialSetOutput, McpToolError> {
    let set = MaterialSet::shipped();
    if set.get(&args.material).is_none() {
        return Err(unknown(&args.material, &set));
    }
    let tint = args.tint.unwrap_or([1.0; 3]);
    let mut ops = vec![OpDto::SetComponent {
        id: args.node.clone(),
        component: "material".to_owned(),
        value: json!({ "asset": args.material, "tint": tint }),
        version: None,
    }];
    // The physical half follows when the node already has a destructible
    // body: same health and chunk budget, the new record's name.
    let mut physics_followed = false;
    {
        let state = server.lock()?;
        if let Some(Component::Destructible(d)) = state
            .doc
            .components
            .get(&args.node)
            .and_then(|slot| slot.get(&ComponentKind::Destructible))
        {
            ops.push(OpDto::SetComponent {
                id: args.node.clone(),
                component: "destructible".to_owned(),
                value: json!({ "material": args.material, "health": d.health, "chunks": d.chunks }),
                version: None,
            });
            physics_followed = true;
        }
    }
    let applied = server.dispatch(TOOL_MATERIAL_SET, |state| {
        let preview = state.tx_preview(&TxPreviewArgs {
            expected_revision: args.expected_revision,
            idempotency_key: args.idempotency_key,
            ops: ops.clone(),
        })?;
        state.tx_apply(&TxApplyArgs {
            expected_revision: args.expected_revision,
            idempotency_key: args.idempotency_key,
            ops,
            approval: preview.approval,
            delta: Some(false),
            origin: None,
        })
    })?;
    Ok(MaterialSetOutput {
        material: args.material.clone(),
        physics_followed,
        revision: applied.revision,
        effects: applied.effects,
    })
}

crate::e2_tool!(
    /// `material.list`: every shipped record with digests and structure key.
    MaterialListTool,
    name = TOOL_MATERIAL_LIST,
    description = "List the shipped material records: name, wire id, sim digest, look digest, shader structure key, plus the set-wide digests. No side effects.",
    readonly = true,
    args = MaterialListArgs,
    output = MaterialListOutput,
    invoke = material_list
);

crate::e2_tool!(
    /// `material.get`: one shipped record as canonical JSON text.
    MaterialGetTool,
    name = TOOL_MATERIAL_GET,
    description = "Read one shipped material record as canonical JSON text to copy and edit, with its digests. No side effects.",
    readonly = true,
    args = MaterialGetArgs,
    output = MaterialGetOutput,
    invoke = material_get
);

crate::e2_tool!(
    /// `material.schema`: the record JSON Schema.
    MaterialSchemaTool,
    name = TOOL_MATERIAL_SCHEMA,
    description = "The JSON Schema of a material record (look, gameplay layers, body, sound, fx). No side effects.",
    readonly = true,
    args = MaterialSchemaArgs,
    output = MaterialSchemaOutput,
    invoke = material_schema
);

crate::e2_tool!(
    /// `material.preview`: validate record text and describe what it compiles to.
    MaterialPreviewTool,
    name = TOOL_MATERIAL_PREVIEW,
    description = "Validate material record JSON text (it may extend a shipped record) and report canonical text, sim and look digests, shader structure key, lobes, layers and generated shader size. A bad record is refused with the field path. No side effects.",
    readonly = true,
    args = MaterialPreviewArgs,
    output = MaterialPreviewOutput,
    invoke = material_preview
);

crate::e2_tool!(
    /// `material.param`: what-if of one dotted-path edit on a shipped record.
    MaterialParamTool,
    name = TOOL_MATERIAL_PARAM,
    description = "What-if of one parameter edit on a shipped record by dotted path (look.specular_roughness, layers[0].darken, body.toughness). Returns the undo value, new digests and whether the shader structure changed. No side effects.",
    readonly = true,
    args = MaterialParamArgs,
    output = MaterialParamOutput,
    invoke = material_param
);

crate::e2_tool!(
    /// `material.compare`: two records side by side.
    MaterialCompareTool,
    name = TOOL_MATERIAL_COMPARE,
    description = "Compare two material records (each a shipped name or record JSON text): CIEDE2000 between base colors, luminance and highlight ratios, roughness, metalness, coat, ior, emission differences, and the sim half apart (toughness, brittleness, sound, shard). No side effects.",
    readonly = true,
    args = MaterialCompareArgs,
    output = MaterialCompareOutput,
    invoke = material_compare
);

crate::e2_tool!(
    /// `material.set`: make a node use a shipped record, look and physics together.
    MaterialSetTool,
    name = TOOL_MATERIAL_SET,
    description = "Make one node use a shipped material record: its material component takes the record (with an optional tint) and, if the node has a destructible body, its physical material follows, in one previewed, capability-gated batch; one undo restores both.",
    readonly = false,
    args = MaterialSetArgs,
    output = MaterialSetOutput,
    invoke = material_set
);

crate::e2_tool!(
    /// `material.graph.set`: store a procedural graph and fold it into a record.
    MaterialGraphSetTool,
    name = TOOL_MATERIAL_GRAPH_SET,
    description = "Store a typed procedural material graph (nodes: noise, tile, gradient, curve, warp, blend) as JSON text and fold it into the existing material record: base color, roughness, metalness, the five coverage channels, a height field and four texture refs. Returns the graph id, the canonical text and the record digest; the same graph twice is a no-op with the same id. Graph refusals name the path, the typed kind and one recovery step.",
    readonly = false,
    args = MaterialGraphSetArgs,
    output = MaterialGraphSetOutput,
    invoke = material_graph_set
);

crate::e2_tool!(
    /// `material.graph.get`: one stored graph as canonical text.
    MaterialGraphGetTool,
    name = TOOL_MATERIAL_GRAPH_GET,
    description = "Read a stored material graph back as canonical JSON text (by id from material.graph.set, or by passing graph text), with its id, node count and the digest of the record it folds into. No side effects.",
    readonly = true,
    args = MaterialGraphGetArgs,
    output = MaterialGraphGetOutput,
    invoke = material_graph_get
);

crate::e2_tool!(
    /// `material.bake`: write the four maps of a graph.
    MaterialBakeTool,
    name = TOOL_MATERIAL_BAKE,
    description = "Bake a material graph (by id or inline text) at 512 or 1024 into four PNGs: base_color, orm, normal, emissive, written into out_dir. Returns each map's path, digest, bytes and per-channel min/max, plus the record digest. The same graph twice gives equal digests and equal bytes; a look map that is flat on every channel is refused. `coverage: true` also writes the five coverage channel maps and the packed sheets (`coverage`: wetness, dirt, blood, strain in RGBA; `coverage_extra`: heat in red), and a flat coverage channel becomes a warning row, not a refusal.",
    readonly = false,
    args = MaterialBakeArgs,
    output = MaterialBakeOutput,
    invoke = material_bake
);

/// The material pack: ten verbs, loaded on demand.
/// Verb: fold a graph into its record text and digest.
pub const TOOL_MATERIAL_FROM_GRAPH: &str = "material.from_graph";

/// The graph form `material.from_graph` folds: the same document
/// `material.graph.set` takes.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialFromGraphArgs {
    /// The graph as JSON text: `material`, `base`, `nodes` and `output`.
    pub graph: String,
}

/// What `material.from_graph` answers: the record in its canonical form.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialFromGraphOutput {
    /// Canonical record text; parsing it back as a TOML record gives the same
    /// bytes, which is the graph-to-record contract.
    pub record: String,
    /// FNV-1a over `record`, 16 hex digits.
    pub digest: String,
    /// Record name the fold wrote.
    pub name: String,
    /// Parent record the fold extended.
    pub base: String,
    /// Nodes the graph folds.
    pub nodes: u32,
    /// Gameplay layers the record carries.
    pub layers: u32,
}

/// `material.from_graph`: one graph in, its record text and digest out.
/// Nothing is stored: the same graph always folds to the same bytes.
fn material_from_graph(
    _server: &crate::mcp::McpServer,
    args: &MaterialFromGraphArgs,
) -> Result<MaterialFromGraphOutput, McpToolError> {
    let graph = parse_graph(&args.graph)?;
    let record = fold_graph(&graph)?;
    let text = record.to_text();
    Ok(MaterialFromGraphOutput {
        digest: format!("{:016x}", engine_core::hash_bytes(text.as_bytes())),
        record: text,
        name: record.name.clone(),
        base: graph.base.clone(),
        nodes: u32::try_from(graph.nodes.len()).unwrap_or(u32::MAX),
        layers: u32::try_from(record.layers.len()).unwrap_or(u32::MAX),
    })
}

crate::e2_tool!(
    /// `material.from_graph`: fold a node graph into its record.
    MaterialFromGraphTool,
    name = TOOL_MATERIAL_FROM_GRAPH,
    description = "Fold a node graph (the same `material`, `base`, `nodes` and `output` document material.graph.set takes) into its material record and answer the canonical record text, its digest, the name and base it carries, and its node and layer counts. The record text parses back to the same bytes, so a graph and a TOML record that mean the same thing fold equal. An unsupported node kind refuses by name with the legal kinds.",
    readonly = true,
    args = MaterialFromGraphArgs,
    output = MaterialFromGraphOutput,
    invoke = material_from_graph
);

/// The shipped material pack: eleven verbs, five of which commit.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "materials: one record for look, physics, sound and damage layers, written as text; \
         list, read, schema, validate-and-describe, what-if edits, compare, setting a record on a \
         node, and a typed procedural graph with a bake to four maps",
        false,
    );
    pack.add(VerbSpec::of::<MaterialListTool>(
        TOOL_MATERIAL_LIST,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<MaterialGetTool>(
        TOOL_MATERIAL_GET,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<MaterialSchemaTool>(
        TOOL_MATERIAL_SCHEMA,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<MaterialPreviewTool>(
        TOOL_MATERIAL_PREVIEW,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<MaterialParamTool>(
        TOOL_MATERIAL_PARAM,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<MaterialCompareTool>(
        TOOL_MATERIAL_COMPARE,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<MaterialSetTool>(
        TOOL_MATERIAL_SET,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<MaterialGraphSetTool>(
        TOOL_MATERIAL_GRAPH_SET,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<MaterialGraphGetTool>(
        TOOL_MATERIAL_GRAPH_GET,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<MaterialFromGraphTool>(
        TOOL_MATERIAL_FROM_GRAPH,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<MaterialBakeTool>(
        TOOL_MATERIAL_BAKE,
        PACK,
        CapabilityCard::Commit,
    )?);
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::{McpServer, TxApplyTool, TxPreviewTool, TxUndoArgs, TxUndoTool};
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;

    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    #[test]
    fn list_holds_every_shipped_record_and_is_stable() {
        let out = MaterialListTool::invoke(&server(), MaterialListArgs::default()).expect("list");
        assert!(out.materials.len() >= 19);
        assert_eq!(out.materials[0].name, "stone");
        assert_eq!(out.materials[0].wire_id, 0);
        let again =
            MaterialListTool::invoke(&server(), MaterialListArgs::default()).expect("list x2");
        assert_eq!(out, again);
    }

    #[test]
    fn get_round_trips_through_preview_with_equal_digests() {
        let got = MaterialGetTool::invoke(
            &server(),
            MaterialGetArgs {
                name: "marble".to_owned(),
            },
        )
        .expect("get");
        let previewed = MaterialPreviewTool::invoke(
            &server(),
            MaterialPreviewArgs {
                record: got.text.clone(),
            },
        )
        .expect("preview");
        assert_eq!(previewed.canonical, got.text);
        assert_eq!(previewed.sim_digest, got.info.sim_digest);
        assert_eq!(previewed.look_digest, got.info.look_digest);
        assert!(previewed.lobes.contains(&"coat".to_owned()));
        assert!(previewed.layers.contains(&"wet<-wetness".to_owned()));
    }

    #[test]
    fn a_child_record_extends_a_shipped_one() {
        let text = r#"{"material":"marble_dark","version":1,"extends":"marble","look":{"base_color":[0.2,0.2,0.2]}}"#;
        let out = MaterialPreviewTool::invoke(
            &server(),
            MaterialPreviewArgs {
                record: text.to_owned(),
            },
        )
        .expect("preview");
        assert_eq!(out.name, "marble_dark");
        let marble = MaterialSet::shipped()
            .get("marble")
            .cloned()
            .expect("marble");
        // A colour edit moves only the look.
        assert_eq!(out.sim_digest, format!("{:016x}", marble.sim_digest()));
        assert_ne!(out.look_digest, format!("{:016x}", marble.look_digest()));
    }

    #[test]
    fn bad_records_are_refused_with_the_path() {
        for (text, path) in [
            (
                r#"{"material":"x","version":1,"look":{"specular_roughness":1.7}}"#,
                "look.specular_roughness",
            ),
            (r#"{"material":"x","version":9}"#, "version"),
            (r#"{"material":"x","version":1,"extends":"nope"}"#, "nope"),
        ] {
            let err = MaterialPreviewTool::invoke(
                &server(),
                MaterialPreviewArgs {
                    record: text.to_owned(),
                },
            )
            .expect_err("refused");
            assert!(
                err.code == "bad_record" || err.code == "unknown_material",
                "{}",
                err.code
            );
            assert!(err.message.contains(path), "{}", err.message);
        }
    }

    #[test]
    fn param_edit_reports_the_undo_value_and_the_sim_split() {
        let edit = |path: &str, value: &str| {
            MaterialParamTool::invoke(
                &server(),
                MaterialParamArgs {
                    name: "marble".to_owned(),
                    path: path.to_owned(),
                    value: value.to_owned(),
                },
            )
        };
        let look = edit("look.specular_roughness", "0.08").expect("param");
        assert_eq!(look.before, "0.35");
        assert_eq!(look.after, "0.08");
        assert_eq!(look.info.sim_digest, look.sim_digest_before);
        assert!(!look.structure_changed);
        let body = edit("body.toughness", "9").expect("param");
        assert_ne!(body.info.sim_digest, body.sim_digest_before);
        let bad = edit("look.nope", "1").expect_err("refused");
        assert_eq!(bad.code, "bad_edit");
    }

    #[test]
    fn unknown_names_list_the_legal_ones() {
        let err = MaterialGetTool::invoke(
            &server(),
            MaterialGetArgs {
                name: "granite".to_owned(),
            },
        )
        .expect_err("unknown");
        assert_eq!(err.code, "unknown_material");
        assert!(err.message.contains("marble"));
    }

    #[test]
    fn schema_verb_hands_back_the_schema_under_the_cap() {
        let out =
            MaterialSchemaTool::invoke(&server(), MaterialSchemaArgs::default()).expect("schema");
        assert!(out.schema.len() <= 8 * 1024);
    }

    #[test]
    fn list_filters_by_prefix_and_schema_returns_one_part() {
        let marbles = MaterialListTool::invoke(
            &server(),
            MaterialListArgs {
                prefix: Some("marble".to_owned()),
            },
        )
        .expect("list");
        let names: Vec<&str> = marbles.materials.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["marble", "marble_wet"]);
        let look = MaterialSchemaTool::invoke(
            &server(),
            MaterialSchemaArgs {
                part: Some("look".to_owned()),
            },
        )
        .expect("part");
        assert!(look.schema.contains("specular_roughness"));
        let bad = MaterialSchemaTool::invoke(
            &server(),
            MaterialSchemaArgs {
                part: Some("nope".to_owned()),
            },
        )
        .expect_err("unknown part");
        assert_eq!(bad.code, "bad_part");
        assert!(bad.message.contains("look"));
    }

    fn server_with_node() -> (McpServer, String) {
        let server = server();
        let id = engine_scene::ids::ObjectId::new().to_string();
        let ops = vec![OpDto::Insert {
            id: id.clone(),
            parent: engine_scene::ids::ObjectId::NIL.to_string(),
            local: crate::mcp::PoseDto {
                pos: [0.0; 3],
                rot: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0; 3],
            },
        }];
        let drafted = TxPreviewTool::invoke(
            &server,
            TxPreviewArgs {
                expected_revision: 0,
                idempotency_key: 1,
                ops: ops.clone(),
            },
        )
        .expect("insert previews");
        TxApplyTool::invoke(
            &server,
            TxApplyArgs {
                expected_revision: 0,
                idempotency_key: 1,
                ops,
                approval: drafted.approval,
                delta: None,
                origin: None,
            },
        )
        .expect("insert lands");
        (server, id)
    }

    #[test]
    fn set_commits_look_and_physics_in_one_batch_and_undoes_both() {
        let (server, id) = server_with_node();
        // Give the node a destructible stone body first (revision 2).
        let seed_ops = vec![OpDto::SetComponent {
            id: id.clone(),
            component: "destructible".to_owned(),
            value: json!({ "material": "stone", "health": 40, "chunks": 8 }),
            version: None,
        }];
        let drafted = TxPreviewTool::invoke(
            &server,
            TxPreviewArgs {
                expected_revision: 1,
                idempotency_key: 2,
                ops: seed_ops.clone(),
            },
        )
        .expect("destructible previews");
        TxApplyTool::invoke(
            &server,
            TxApplyArgs {
                expected_revision: 1,
                idempotency_key: 2,
                ops: seed_ops,
                approval: drafted.approval,
                delta: None,
                origin: None,
            },
        )
        .expect("destructible lands");
        let applied = MaterialSetTool::invoke(
            &server,
            MaterialSetArgs {
                node: id.clone(),
                material: "marble_wet".to_owned(),
                tint: None,
                expected_revision: 2,
                idempotency_key: 3,
            },
        )
        .expect("sets");
        assert_eq!(applied.revision, 3);
        assert!(applied.physics_followed);
        {
            let state = server.lock().expect("lock");
            let slot = state
                .doc
                .components
                .get(&id)
                .expect("node carries components");
            assert!(matches!(
                slot.get(&ComponentKind::Material),
                Some(Component::Material(m)) if m.asset.to_string() == "marble_wet"
            ));
            assert!(matches!(
                slot.get(&ComponentKind::Destructible),
                Some(Component::Destructible(d)) if d.material.to_string() == "marble_wet" && d.health == 40
            ));
        }
        let unknown = MaterialSetTool::invoke(
            &server,
            MaterialSetArgs {
                node: id.clone(),
                material: "granite".to_owned(),
                tint: None,
                expected_revision: 3,
                idempotency_key: 4,
            },
        )
        .expect_err("unknown record");
        assert_eq!(unknown.code, "unknown_material");
        assert_eq!(server.lock().expect("lock").doc.revision, 3);
        // One undo restores both components: the look reference is gone and
        // the physical material is stone again.
        TxUndoTool::invoke(&server, TxUndoArgs { undo_token: None }).expect("undo");
        let state = server.lock().expect("lock");
        let slot = state.doc.components.get(&id).expect("destructible stays");
        assert!(!slot.contains_key(&ComponentKind::Material));
        assert!(matches!(
            slot.get(&ComponentKind::Destructible),
            Some(Component::Destructible(d)) if d.material.to_string() == "stone"
        ));
    }

    #[test]
    fn compare_reports_wet_marble_as_darker_shinier_and_softer() {
        let ask = |a: &str, b: &str| {
            MaterialCompareTool::invoke(
                &server(),
                MaterialCompareArgs {
                    a: a.to_owned(),
                    b: b.to_owned(),
                },
            )
        };
        let out = ask("marble", "marble_wet").expect("compares");
        assert!(out.luminance_ratio < 0.95);
        assert!(out.specular_peak_ratio >= 2.0);
        assert!(out.sound_changed && out.sim_changed && out.look_changed);
        assert_eq!(out, ask("marble", "marble_wet").expect("compares x2"));
        // Record text works on either side.
        let text =
            r#"{"material":"x","version":1,"extends":"marble","look":{"specular_roughness":0.1}}"#;
        let mixed = ask("marble", text).expect("mixed");
        assert!(!mixed.sim_changed && mixed.look_changed);
        assert_eq!(
            ask("marble", "granite").expect_err("unknown").code,
            "unknown_material"
        );
    }

    const BRICK: &str = r#"{
      "material": "brick_wall",
      "base": "stone",
      "nodes": [
        {"id": 1, "kind": "gradient", "params": {"axis": "y", "from": 0.2, "to": 1.0}},
        {"id": 2, "kind": "tile", "inputs": {"a": 1}, "params": {"rows": 8, "cols": 3, "mortar": 0.08}},
        {"id": 3, "kind": "noise", "params": {"scale": 24, "seed": 1}},
        {"id": 4, "kind": "blend", "inputs": {"a": 2, "b": 3, "mask": 3}, "params": {"mode": "multiply"}},
        {"id": 5, "kind": "curve", "inputs": {"a": 2}, "params": {"exponent": 2}}
      ],
      "output": {
        "color": 2, "color_low": [0.45, 0.33, 0.28], "color_high": [0.72, 0.6, 0.48],
        "roughness": 4, "metalness": 3, "height": 2,
        "channels": {"wetness": 3, "dirt": 2, "heat": 5},
        "emissive": 5
      },
      "layers": [{"kind": "wet", "driver": "wetness", "strength": 0.6}]
    }"#;

    fn scratch(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("e2040-material-{tag}-{}", std::process::id()))
    }

    #[test]
    fn graph_set_folds_the_brick_graph_and_a_second_set_is_a_no_op() {
        let server = server();
        // The graph table is process global and every test server names the
        // same session, so this test folds its own noise seed: a graph another
        // test already stored would read as a no-op here.
        let graph = BRICK.replace("\"seed\": 1", "\"seed\": 7");
        let first = MaterialGraphSetTool::invoke(
            &server,
            MaterialGraphSetArgs {
                graph: graph.clone(),
            },
        )
        .expect("graph sets");
        assert_eq!(first.material, "brick_wall");
        assert_eq!(first.base, "stone");
        assert_eq!(first.nodes, 5);
        assert_eq!(first.layers, 1);
        assert!(first.applied);
        assert!(first.channels.contains(&"wetness".to_owned()));
        assert_eq!(first.textures.len(), 4);
        // The sim half never moves: the graph is a look document.
        let stone = MaterialSet::shipped().get("stone").cloned().expect("stone");
        assert_eq!(first.sim_digest, format!("{:016x}", stone.sim_digest()));
        let second = MaterialGraphSetTool::invoke(
            &server,
            MaterialGraphSetArgs {
                graph: graph.clone(),
            },
        )
        .expect("graph sets twice");
        assert!(!second.applied, "the second set is a no-op");
        assert_eq!(second.id, first.id);
        assert_eq!(second.record_digest, first.record_digest);
        assert_eq!(second.canonical, first.canonical);
    }

    #[test]
    fn graph_get_reads_a_stored_graph_back_as_canonical_text() {
        let server = server();
        let set = MaterialGraphSetTool::invoke(
            &server,
            MaterialGraphSetArgs {
                graph: BRICK.to_owned(),
            },
        )
        .expect("graph sets");
        let got = MaterialGraphGetTool::invoke(
            &server,
            MaterialGraphGetArgs {
                id: Some(set.id.clone()),
                graph: None,
            },
        )
        .expect("graph gets");
        assert_eq!(got.id, set.id);
        assert_eq!(got.canonical, set.canonical);
        assert_eq!(got.record_digest, set.record_digest);
        // The canonical text sets again to the same id.
        let again = MaterialGraphSetTool::invoke(
            &server,
            MaterialGraphSetArgs {
                graph: got.canonical.clone(),
            },
        )
        .expect("canonical sets");
        assert_eq!(again.id, set.id);
        let missing = MaterialGraphGetTool::invoke(
            &server,
            MaterialGraphGetArgs {
                id: Some("0123456789abcdef".to_owned()),
                graph: None,
            },
        )
        .expect_err("unknown id");
        assert_eq!(missing.code, "unknown_graph");
        let none = MaterialGraphGetTool::invoke(&server, MaterialGraphGetArgs::default())
            .expect_err("nothing named");
        assert_eq!(none.code, "bad_graph");
    }

    #[test]
    fn a_bad_graph_names_the_legal_kinds_and_one_recovery() {
        let broken = BRICK.replace(r#""kind": "tile""#, r#""kind": "voronoi""#);
        let err = MaterialGraphSetTool::invoke(&server(), MaterialGraphSetArgs { graph: broken })
            .expect_err("unknown kind");
        assert_eq!(err.code, "bad_graph");
        assert!(err
            .message
            .contains("noise, tile, gradient, curve, warp, blend"));
        assert!(err.message.contains("next:"));
    }

    #[test]
    fn bake_writes_four_pngs_and_two_runs_are_byte_equal() {
        let server = server();
        let set = MaterialGraphSetTool::invoke(
            &server,
            MaterialGraphSetArgs {
                graph: BRICK.to_owned(),
            },
        )
        .expect("graph sets");
        let out = scratch("bake");
        let ask = || {
            MaterialBakeTool::invoke(
                &server,
                MaterialBakeArgs {
                    id: Some(set.id.clone()),
                    graph: None,
                    resolution: Some(512),
                    coverage: false,
                    out_dir: out.to_string_lossy().into_owned(),
                },
            )
        };
        let first = ask().expect("bakes");
        assert_eq!(first.resolution, 512);
        let names: Vec<&str> = first.maps.iter().map(|map| map.name.as_str()).collect();
        assert_eq!(names, ["base_color", "orm", "normal", "emissive"]);
        assert_eq!(first.textures.len(), 4);
        for map in &first.maps {
            assert!(map.bytes > 0, "{} bytes", map.name);
            let path = std::path::Path::new(&map.path);
            assert!(path.exists(), "{} written", map.name);
            let bytes = std::fs::read(path).expect("map reads back");
            assert_eq!(u64::try_from(bytes.len()).unwrap_or(0), map.bytes);
            assert!(map.max.iter().zip(&map.min).any(|(high, low)| high > low));
        }
        let second = ask().expect("bakes twice");
        assert_eq!(second.maps, first.maps);
        let flat = MaterialBakeTool::invoke(
            &server,
            MaterialBakeArgs {
                id: None,
                graph: Some(
                    r#"{"material": "flat", "nodes": [{"id": 1, "kind": "gradient"},
                      {"id": 2, "kind": "gradient", "params": {"from": 0.5, "to": 0.5}}],
                      "output": {"color": 1, "roughness": 2, "metalness": 2, "height": 2}}"#
                        .to_owned(),
                ),
                resolution: Some(512),
                coverage: false,
                out_dir: out.to_string_lossy().into_owned(),
            },
        )
        .expect_err("a flat bake is refused");
        assert_eq!(flat.code, "bad_graph");
        assert!(flat.message.contains("flat"), "{}", flat.message);
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn bake_coverage_writes_five_channels_and_two_sheets() {
        let server = server();
        let set = MaterialGraphSetTool::invoke(
            &server,
            MaterialGraphSetArgs {
                graph: BRICK.to_owned(),
            },
        )
        .expect("graph sets");
        let out = scratch("bake-coverage");
        let ask = |coverage: bool| {
            MaterialBakeTool::invoke(
                &server,
                MaterialBakeArgs {
                    id: Some(set.id.clone()),
                    graph: None,
                    resolution: Some(512),
                    coverage,
                    out_dir: out.to_string_lossy().into_owned(),
                },
            )
        };
        let first = ask(true).expect("bakes coverage");
        let names: Vec<&str> = first.coverage.iter().map(|map| map.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "coverage_wetness",
                "coverage_dirt",
                "coverage_blood",
                "coverage_strain",
                "coverage_heat",
                "coverage",
                "coverage_extra",
            ]
        );
        for map in &first.coverage {
            let path = std::path::Path::new(&map.path);
            assert!(path.exists(), "{} written", map.name);
            let bytes = std::fs::read(path).expect("map reads back");
            assert_eq!(u64::try_from(bytes.len()).unwrap_or(0), map.bytes);
        }
        let find = |name: &str| {
            first
                .coverage
                .iter()
                .find(|map| map.name == name)
                .expect("map named")
        };
        // The brick graph drives wetness, dirt and heat; blood is flat, and a
        // flat channel is a warning row, never a refusal.
        assert!(find("coverage_dirt").max[0] > find("coverage_dirt").min[0]);
        assert!(find("coverage_wetness").max[0] > find("coverage_wetness").min[0]);
        let blood = find("coverage_blood");
        assert_eq!(blood.min[0], blood.max[0]);
        assert!(
            first
                .warnings
                .iter()
                .any(|row| row.contains("coverage_blood")),
            "{:?}",
            first.warnings
        );
        // Twice: equal digests and equal bytes.
        let second = ask(true).expect("bakes coverage twice");
        assert_eq!(second.coverage, first.coverage);
        assert_eq!(second.warnings, first.warnings);
        // Coverage off: a plain bake writes four maps and no coverage set.
        let plain = ask(false).expect("bakes four");
        assert!(plain.coverage.is_empty());
        assert!(plain.warnings.is_empty());
        let _ = std::fs::remove_dir_all(&out);
    }

    #[cfg(feature = "intake")]
    #[test]
    fn every_baked_map_cooks_the_same_ktx2_bytes_twice() {
        let server = server();
        let out = scratch("bake-ktx2");
        let ask = || {
            MaterialBakeTool::invoke(
                &server,
                MaterialBakeArgs {
                    id: None,
                    graph: Some(BRICK.to_owned()),
                    resolution: Some(512),
                    coverage: true,
                    out_dir: out.to_string_lossy().into_owned(),
                },
            )
            .expect("bakes")
        };
        let receipts = |bake: &MaterialBakeOutput| -> Vec<(String, Option<u64>, Option<String>)> {
            bake.maps
                .iter()
                .chain(bake.coverage.iter())
                .map(|map| (map.name.clone(), map.ktx2_bytes, map.ktx2_digest.clone()))
                .collect()
        };
        let first = ask();
        let rows = receipts(&first);
        assert_eq!(rows.len(), 11, "four look maps and seven coverage maps");
        for (name, bytes, digest) in &rows {
            assert!(bytes.unwrap_or(0) > 0, "{name} cooked");
            assert_eq!(digest.as_ref().map(String::len), Some(16), "{name}");
        }
        let second = ask();
        assert_eq!(
            receipts(&second),
            rows,
            "the same graph twice cooks equal bytes"
        );
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn the_pack_registers_its_verbs_with_their_cards() {
        let registry = super::super::PackRegistry::shipped();
        assert!(registry.errors().is_empty(), "{:?}", registry.errors());
        for name in [
            TOOL_MATERIAL_LIST,
            TOOL_MATERIAL_GET,
            TOOL_MATERIAL_SCHEMA,
            TOOL_MATERIAL_PREVIEW,
            TOOL_MATERIAL_PARAM,
            TOOL_MATERIAL_COMPARE,
            TOOL_MATERIAL_GRAPH_GET,
            TOOL_MATERIAL_FROM_GRAPH,
        ] {
            let verb = registry.verb(name).expect("registered");
            assert_eq!(verb.card(), CapabilityCard::Observe, "{name}");
            assert!(verb.readonly(), "{name}");
        }
        for name in [
            TOOL_MATERIAL_SET,
            TOOL_MATERIAL_GRAPH_SET,
            TOOL_MATERIAL_BAKE,
        ] {
            let verb = registry.verb(name).expect("registered");
            assert_eq!(verb.card(), CapabilityCard::Commit, "{name}");
            assert!(!verb.readonly(), "{name}");
        }
    }

    #[test]
    fn from_graph_answers_the_record_and_the_text_round_trips_byte_equal() {
        let server = server();
        let out = MaterialFromGraphTool::invoke(
            &server,
            MaterialFromGraphArgs {
                graph: BRICK.to_owned(),
            },
        )
        .expect("the graph folds");
        assert_eq!(out.name, "brick_wall");
        assert_eq!(out.base, "stone");
        assert_eq!(out.nodes, 5);
        assert_eq!(out.layers, 1);
        assert_eq!(out.digest.len(), 16);
        // The record text is the record: parsing it back gives the same bytes,
        // so a graph and a record text that mean the same thing fold equal.
        let record = MaterialSet::shipped()
            .parse_against(&out.record)
            .expect("the record parses against the shipped set");
        assert_eq!(
            record.to_text(),
            out.record,
            "the graph's record and the parsed text are byte-equal"
        );
        assert_eq!(
            format!("{:016x}", engine_core::hash_bytes(out.record.as_bytes())),
            out.digest
        );
        // The same graph twice answers the same bytes.
        let again = MaterialFromGraphTool::invoke(
            &server,
            MaterialFromGraphArgs {
                graph: BRICK.to_owned(),
            },
        )
        .expect("the graph folds twice");
        assert_eq!(again, out);
    }

    #[test]
    fn from_graph_refuses_an_unsupported_node_kind_by_name() {
        let server = server();
        let broken = BRICK.replace(r#""kind": "noise""#, r#""kind": "voronoi""#);
        assert_ne!(broken, BRICK, "the fixture carries the noise kind");
        let err = MaterialFromGraphTool::invoke(&server, MaterialFromGraphArgs { graph: broken })
            .expect_err("an unknown kind refuses");
        assert_eq!(err.code, "bad_graph");
        assert!(
            err.message.contains("voronoi"),
            "the refusal names the kind: {}",
            err.message
        );
        assert!(
            err.message
                .contains("noise, tile, gradient, curve, warp, blend"),
            "the refusal names the legal kinds: {}",
            err.message
        );
        // Text that is not a graph at all refuses the same way.
        let err = MaterialFromGraphTool::invoke(
            &server,
            MaterialFromGraphArgs {
                graph: "not a graph".to_owned(),
            },
        )
        .expect_err("bad text refuses");
        assert_eq!(err.code, "bad_graph");
    }
}

// WG2: this pack publishes its own row; `packs/mod.rs` keeps no hand list.
#[expect(
    unsafe_code,
    reason = "linkme publishes a pack row through a link section"
)]
#[linkme::distributed_slice(super::PACK_MODULES)]
static PACK_ROW: super::PackModule = super::PackModule {
    order: 4,
    name: PACK,
    default_loaded: false,
    tools: &[
        "material.list",
        "material.get",
        "material.schema",
        "material.preview",
        "material.param",
        "material.compare",
        "material.set",
        "material.graph.set",
        "material.graph.get",
        "material.from_graph",
        "material.bake",
    ],
    build: pack,
};
