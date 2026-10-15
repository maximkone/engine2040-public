//! `material` pack: the one material record an AI writes as text.
//!
//! Seven verbs over the shipped records (`MaterialSet::shipped`: the fourteen
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
//!
//! The record table has no document of its own yet, so an edit lives in the
//! text the caller holds; `material.set` is the one verb that writes the
//! scene. Units: unit-range factors, meters, nits (see `material.schema`).
//! Determinism: the same text gives the same digests on every machine (the
//! record codec is integer-exact for sim fields and canonical for look
//! fields). Errors: [`McpToolError`] codes `bad_record` (the message starts
//! with the field path), `unknown_material` and `bad_edit`, each naming the
//! legal values.

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

/// The material pack: seven verbs, loaded on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "materials: one record for look, physics, sound and damage layers, written as text; \
         list, read, schema, validate-and-describe, what-if edits, compare and setting a record on a node",
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

    #[test]
    fn the_pack_registers_seven_verbs_with_their_cards() {
        let registry = super::super::PackRegistry::shipped();
        assert!(registry.errors().is_empty(), "{:?}", registry.errors());
        for name in [
            TOOL_MATERIAL_LIST,
            TOOL_MATERIAL_GET,
            TOOL_MATERIAL_SCHEMA,
            TOOL_MATERIAL_PREVIEW,
            TOOL_MATERIAL_PARAM,
            TOOL_MATERIAL_COMPARE,
        ] {
            let verb = registry.verb(name).expect("registered");
            assert_eq!(verb.card(), CapabilityCard::Observe, "{name}");
            assert!(verb.readonly(), "{name}");
        }
        let set = registry.verb(TOOL_MATERIAL_SET).expect("set");
        assert_eq!(set.card(), CapabilityCard::Commit);
        assert!(!set.readonly());
    }
}
