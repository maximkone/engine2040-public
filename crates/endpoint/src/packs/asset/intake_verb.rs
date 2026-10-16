//! `asset.intake`: a generated GLB becomes a game-ready prop.
//!
//! Wraps `engine_assets::gen_intake`. The caller names the GLB and either the
//! runner's `kaggle-lane.json` (model, licence, seed, image digest and conditions
//! are read from the row of that file) or the same facts by hand. Writes
//! `<out_dir>/<name>.e204bndl` and `<out_dir>/<name>.provenance.json`, and keeps a
//! cache under `<out_dir>/cache` keyed by model id, licence, prompt or image digest,
//! seed and settings, so the same inputs return the same bundle hash. Disk and CPU
//! only, never the tick.

use std::path::{Path, PathBuf};

use engine_assets::gen_intake::{
    digest_text, intake, IntakeCache, IntakeError, IntakeOutput, IntakeRequest, IntakeSettings,
    ModelRef, Provenance, UpAxis, DEFAULT_SIZE_MM,
};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};
use rmcp::serde_json::{self, Value};

use super::TOOL_ASSET_INTAKE;
use crate::mcp::{McpServer, McpToolError};

/// Default output folder, relative to the door's working directory.
const DEFAULT_OUT_DIR: &str = "target/asset-intake";

/// One model or tool the result depends on.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ModelArg {
    /// Hub id or tool name (`VAST-AI/TripoSG`).
    pub id: String,
    /// SPDX identifier or expression (`MIT`).
    pub license: String,
}

/// Args for `asset.intake`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct IntakeArgs {
    /// Asset name: a-z, 0-9, `_`, `-`, at most 64 chars.
    pub name: String,
    /// Path of the generated `.glb`.
    pub glb_path: String,
    /// The runner's `kaggle-lane.json`: fills model, components, seed, image digest,
    /// conditions and code ref from the item whose file is this GLB's name.
    #[serde(default)]
    pub lane_json: Option<String>,
    /// The shape model (overrides the lane file).
    #[serde(default)]
    pub model: Option<ModelArg>,
    /// Cutout, encoder and other parts the output depends on (overrides the lane file).
    #[serde(default)]
    pub components: Vec<ModelArg>,
    /// The prompt text (its digest is recorded) when no image digest is given.
    #[serde(default)]
    pub prompt: Option<String>,
    /// Lowercase hex digest of the source image (overrides the lane file).
    #[serde(default)]
    pub source_digest: Option<String>,
    /// Seed the runner used (overrides the lane file).
    #[serde(default)]
    pub seed: Option<u64>,
    /// Conditions the generator declared (added to the lane file's).
    #[serde(default)]
    pub conditions: Vec<String>,
    /// Conditions you accept, by exact text; every declared condition must be listed.
    #[serde(default)]
    pub accept_conditions: Vec<String>,
    /// Repository and commit of the generator code.
    #[serde(default)]
    pub code_ref: Option<String>,
    /// Longest side after posing, millimetres (default 1000).
    #[serde(default)]
    pub size_mm: Option<u32>,
    /// `y` (default) or `z`: the up axis of the generated mesh.
    #[serde(default)]
    pub up_axis: Option<String>,
    /// Output folder (default `target/asset-intake`).
    #[serde(default)]
    pub out_dir: Option<String>,
}

/// LOD0 collision hull.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct HullDoc {
    /// AABB minimum corner, metres.
    pub min: [f32; 3],
    /// AABB maximum corner, metres.
    pub max: [f32; 3],
    /// Bounding sphere centre, metres.
    pub center: [f32; 3],
    /// Bounding sphere radius, metres.
    pub radius: f32,
    /// Support points (extreme vertices), at most 64.
    pub support: Vec<[f32; 3]>,
}

/// Result of `asset.intake`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct IntakeOutputDoc {
    /// Asset name.
    pub asset: String,
    /// Blake3 hex of the bundle bytes.
    pub bundle_hash: String,
    /// Where the bundle was written.
    pub bundle_path: String,
    /// Where the provenance row was written.
    pub row_path: String,
    /// True when the cache answered (same inputs, same GLB).
    pub cache_hit: bool,
    /// Triangles after repair, before simplification.
    pub tris_in: u32,
    /// LOD0 triangles (at most 5,000).
    pub tris: u32,
    /// Triangles per LOD.
    pub lod_tris: Vec<u32>,
    /// LOD0 collision hull.
    pub hull: HullDoc,
    /// Prop cook digest (16 hex digits).
    pub cook_digest: String,
    /// Cache key.
    pub cache_key: String,
    /// Conditions that were declared and accepted.
    pub accepted_conditions: Vec<String>,
    /// The provenance row as written.
    pub provenance: Value,
}

fn refuse(error: &IntakeError) -> McpToolError {
    let next = match error {
        IntakeError::LicenseRefused { .. }
        | IntakeError::LicenseNotSpdx { .. }
        | IntakeError::ModelRestricted { .. } => {
            "use a model whose weights and parts are commercial-use (see the licences in asset.generate)"
        }
        IntakeError::ConditionsUnaccepted { .. } => {
            "read the conditions, then pass each one you accept in accept_conditions"
        }
        IntakeError::Provenance { .. } => {
            "pass lane_json from the runner, or model, source_digest (or prompt) and seed"
        }
        IntakeError::Settings { .. } => "fix the named argument",
        IntakeError::Import(_) | IntakeError::Empty { .. } => {
            "re-run the generator on a clearer picture, or check the GLB opens"
        }
        _ => "report the code and the message",
    };
    McpToolError::new(error.code(), format!("{error}. Next: {next}."))
}

fn bad(message: impl Into<String>) -> McpToolError {
    McpToolError::new("bad_input", message)
}

fn str_of<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// Facts the runner recorded for one GLB.
#[derive(Default)]
struct LaneFacts {
    model: Option<ModelRef>,
    components: Vec<ModelRef>,
    source_digest: Option<String>,
    seed: Option<u64>,
    conditions: Vec<String>,
    code_ref: Option<String>,
}

fn model_ref(entry: &Value) -> Option<ModelRef> {
    Some(ModelRef {
        id: str_of(entry, "id")?.to_owned(),
        license: str_of(entry, "license")?.to_owned(),
    })
}

fn read_lane(path: &str, glb_name: &str) -> Result<LaneFacts, McpToolError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| bad(format!("cannot read lane_json `{path}`: {e}")))?;
    let doc: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| bad(format!("lane_json is not JSON: {e}")))?;
    let items = doc.get("items").and_then(Value::as_array);
    let item = items
        .and_then(|rows| rows.iter().find(|r| str_of(r, "file") == Some(glb_name)))
        .ok_or_else(|| bad(format!("lane_json has no item for `{glb_name}`")))?;
    let key = str_of(item, "model").unwrap_or_default();
    let used = doc
        .get("models")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut facts = LaneFacts {
        source_digest: str_of(item, "source_sha256").map(str::to_owned),
        seed: item.get("seed").and_then(Value::as_u64),
        conditions: item
            .get("conditions")
            .and_then(Value::as_array)
            .map(|c| {
                c.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        code_ref: str_of(item, "code_commit").map(|c| format!("TripoSG@{c}")),
        ..LaneFacts::default()
    };
    for entry in &used {
        let Some(parsed) = model_ref(entry) else {
            continue;
        };
        if str_of(entry, "key") == Some(key) {
            facts.model = Some(parsed);
        } else {
            facts.components.push(parsed);
        }
    }
    Ok(facts)
}

fn provenance_of(args: &IntakeArgs, glb_name: &str) -> Result<Provenance, McpToolError> {
    let lane = match &args.lane_json {
        Some(path) => read_lane(path, glb_name)?,
        None => LaneFacts::default(),
    };
    let to_ref = |m: &ModelArg| ModelRef {
        id: m.id.clone(),
        license: m.license.clone(),
    };
    let model = args
        .model
        .as_ref()
        .map(to_ref)
        .or(lane.model)
        .ok_or_else(|| bad("no model: pass lane_json or model {id, license}"))?;
    let components = if args.components.is_empty() {
        lane.components
    } else {
        args.components.iter().map(to_ref).collect()
    };
    let source_digest = args
        .source_digest
        .clone()
        .or_else(|| args.prompt.as_deref().map(digest_text))
        .or(lane.source_digest)
        .ok_or_else(|| bad("no source: pass lane_json, source_digest or prompt"))?;
    let seed = args
        .seed
        .or(lane.seed)
        .ok_or_else(|| bad("no seed: pass lane_json or seed"))?;
    let mut conditions = lane.conditions;
    for c in &args.conditions {
        if !conditions.contains(c) {
            conditions.push(c.clone());
        }
    }
    Ok(Provenance {
        model,
        components,
        source_digest,
        seed,
        conditions,
        code_ref: args.code_ref.clone().or(lane.code_ref).unwrap_or_default(),
    })
}

fn settings_of(args: &IntakeArgs) -> Result<IntakeSettings, McpToolError> {
    let up_axis = match args.up_axis.as_deref() {
        None | Some("y" | "Y") => UpAxis::Y,
        Some("z" | "Z") => UpAxis::Z,
        Some(other) => return Err(bad(format!("up_axis wants `y` or `z`, got `{other}`"))),
    };
    Ok(IntakeSettings {
        size_mm: args.size_mm.unwrap_or(DEFAULT_SIZE_MM),
        up_axis,
        ..IntakeSettings::default()
    })
}

fn hull_doc(out: &IntakeOutput) -> HullDoc {
    HullDoc {
        min: out.hull.min,
        max: out.hull.max,
        center: out.hull.center,
        radius: out.hull.radius,
        support: out.hull.support.clone(),
    }
}

fn io_err(what: &str, path: &Path, e: &std::io::Error) -> McpToolError {
    McpToolError::new(
        "io_failed",
        format!(
            "cannot {what} `{}`: {e}. Next: check the folder is writable.",
            path.display()
        ),
    )
}

fn run(args: &IntakeArgs) -> Result<IntakeOutputDoc, McpToolError> {
    let glb_path = PathBuf::from(&args.glb_path);
    let ext = glb_path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if ext != "glb" && ext != "gltf" {
        return Err(bad("glb_path wants a .glb or .gltf file"));
    }
    let glb_name = glb_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_owned();
    let glb = std::fs::read(&glb_path).map_err(|e| io_err("read", &glb_path, &e))?;
    let provenance = provenance_of(args, &glb_name)?;
    let settings = settings_of(args)?;
    let out_dir = PathBuf::from(args.out_dir.as_deref().unwrap_or(DEFAULT_OUT_DIR));
    let mut cache = IntakeCache::open(&out_dir.join("cache")).map_err(|e| refuse(&e))?;
    let request = IntakeRequest {
        name: &args.name,
        glb: &glb,
        provenance: &provenance,
        accepted_conditions: &args.accept_conditions,
        settings,
    };
    let out = intake(&request, &mut cache).map_err(|e| refuse(&e))?;
    let bundle_path = out_dir.join(format!("{}.e204bndl", out.name));
    let row_path = out_dir.join(format!("{}.provenance.json", out.name));
    std::fs::write(&bundle_path, &out.bundle).map_err(|e| io_err("write", &bundle_path, &e))?;
    std::fs::write(&row_path, format!("{}\n", out.row))
        .map_err(|e| io_err("write", &row_path, &e))?;
    let row: Value = serde_json::from_str(&out.row).map_err(|e| {
        McpToolError::new(
            "bad_output",
            format!("the provenance row did not parse ({e}); report the verb"),
        )
    })?;
    Ok(IntakeOutputDoc {
        asset: out.name.clone(),
        bundle_hash: out.bundle_hash.clone(),
        bundle_path: bundle_path.display().to_string(),
        row_path: row_path.display().to_string(),
        cache_hit: out.cache_hit,
        tris_in: out.tris_in,
        tris: out.tris,
        lod_tris: out.lod_tris.clone(),
        hull: hull_doc(&out),
        cook_digest: format!("{:016x}", out.cook_digest),
        cache_key: out.cache_key.clone(),
        accepted_conditions: provenance.conditions.clone(),
        provenance: row,
    })
}

fn intake_verb(server: &McpServer, args: &IntakeArgs) -> Result<IntakeOutputDoc, McpToolError> {
    server.dispatch(TOOL_ASSET_INTAKE, |_state| run(args))
}

crate::e2_tool!(
    /// `asset.intake`: cook a generated GLB into a game-ready prop with provenance.
    IntakeTool,
    name = TOOL_ASSET_INTAKE,
    description = "Cook a generated GLB (center's kaggle mesh job) into a prop: refuses non-commercial or unknown-licence models, repairs, simplifies to at most 5,000 triangles, builds LODs, a collision hull and an E204BNDL bundle, writes a provenance row (model, SPDX licence, image or prompt digest, seed). Same inputs return the cached bundle hash. Declared conditions must be listed in accept_conditions.",
    readonly = false,
    args = IntakeArgs,
    output = IntakeOutputDoc,
    invoke = intake_verb
);

#[cfg(test)]
mod tests;
