//! `world` pack: the door bakes, samples and paints the TVER terrain that
//! `engine-world` owns, and lays out the maps that sit on it.
//!
//! `world.terrain.bake` bakes a seeded map and reports its integer stats
//! (byte count, FNV digest, side, height range and mean, holes, per-layer
//! splat coverage, blocked cells at the default slope) and writes the TVER
//! file only when `out` names one. `world.terrain.sample` answers the height,
//! hole bit and splat of up to [`MAX_POINTS`] points in request order, from a
//! seed bake or a TVER file this door wrote. `world.terrain.paint` applies
//! bounded edits (rect, height delta, optional hole and splat) and reports the
//! same stats plus how many cells moved.
//!
//! `world.scatter` lays seeded Poisson-disk points over a square of 1 cm cells,
//! answers each point's height and hole from a named terrain, and writes a JSON
//! placement only when `out` names one. `world.wfc` solves a tile map from an
//! adjacency table, retries later seeds when the bake refuses, refuses a solve
//! whose passable kinds sit in more than one region, and writes a compact JSON
//! map only when `out` names one.
//!
//! `world.rewrite` runs a seeded rewrite-rule program from its rules text (the
//! format in `engine_world::rewrite`) and answers the grid as text, its hash and
//! the steps taken. A bad rules line refuses with its 1-based line number, a
//! grid over 128 cells a side refuses before any run starts, and a run that never
//! settles refuses instead of answering a partial grid; it writes the grid text
//! only when `out` names one.
//!
//! Units: texels for sides, rects and points, heights in texel steps
//! `0..=255`, hundredths for the mean, permille for splat coverage. File IO
//! only at the `out`/`path` edge and only under the project root: a relative
//! path with no `..`, never a fixed path. Determinism: the same seed and the
//! same edits give the same bytes and digest. Errors: [`McpToolError`] with
//! `bad_args` (the legal range plus one recovery), `read_failed` or
//! `write_failed` at the file edge.

use std::path::{Component, Path};

use engine_world::rewrite::{Program, RulesError, RunError};
use engine_world::scatter::poisson_disk;
use engine_world::terrain::{
    bake_terrain, paint_terrain, parse_terrain, terrain_hash, terrain_stats, write_terrain, Edit,
    Terrain,
};
use engine_world::wfc::{
    bake as wfc_bake, is_connected, Adjacency, WfcError, DIRECTIONS, MAX_KINDS, MAX_WFC_SIDE,
};
use engine_world::MAX_BAKE_SIZE;

use super::{bad_args, PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::tool_capabilities::CapabilityCard;
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

/// Pack name.
pub const PACK: &str = "world";
/// Verb: bake a seeded terrain and report its stats.
pub const TOOL_WORLD_TERRAIN_BAKE: &str = "world.terrain.bake";
/// Verb: read height, hole and splat at points.
pub const TOOL_WORLD_TERRAIN_SAMPLE: &str = "world.terrain.sample";
/// Verb: apply bounded edits to a seeded terrain.
pub const TOOL_WORLD_TERRAIN_PAINT: &str = "world.terrain.paint";

/// Most points one `world.terrain.sample` call may name.
pub const MAX_POINTS: usize = 4096;
/// Most edits one `world.terrain.paint` call may name.
pub const MAX_EDITS: usize = 4096;
/// Side a seed sample bakes when the call names no size.
pub const DEFAULT_SIZE: u16 = 64;

/// Args for `world.terrain.bake`: the seed, the side and an optional path to
/// write under the project root.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BakeArgs {
    /// Bake seed; the same seed and size always give the same bytes.
    pub seed: u64,
    /// Square side in texels, `1..=engine_world::MAX_BAKE_SIZE` (256).
    pub size: u16,
    /// Relative TVER path under the project root
    /// (`target/terrain/roman-camp.tver`). Omitted writes nothing.
    #[serde(default)]
    pub out: Option<String>,
}

/// `world.terrain.bake` result: the bytes it made and their integer stats.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TerrainSummary {
    /// Length of the TVER bytes.
    pub bytes: u64,
    /// FNV-1a 64 digest of those bytes as 16 lowercase hex chars; two calls
    /// with the same seed, size and edits answer the same digest.
    pub digest: String,
    /// Square side in texels.
    pub size: u16,
    /// Lowest height, in texel steps.
    pub height_min: u8,
    /// Highest height, in texel steps.
    pub height_max: u8,
    /// Mean height times 100, truncated.
    pub height_mean_x100: u32,
    /// Texels with a hole.
    pub holes: u32,
    /// Per-layer permille of texels where that layer is the largest weight;
    /// a tie goes to the lower layer.
    pub splat_coverage: [u32; 3],
    /// Cells `blocked` refuses at the default slope (`SLOPE_DEFAULT`).
    pub blocked_cells: u32,
    /// The relative path written, when the call named one.
    pub out: Option<String>,
}

/// Args for `world.terrain.sample`: a seed bake or a TVER path, plus points.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SampleArgs {
    /// Bake seed; pass either this (with an optional `size`) or `path`.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Side to bake when `seed` is given; defaults to [`DEFAULT_SIZE`].
    #[serde(default)]
    pub size: Option<u16>,
    /// Relative TVER path under the project root; pass instead of `seed`.
    #[serde(default)]
    pub path: Option<String>,
    /// Points as `[x, y]` texels, `1..=MAX_POINTS` of them, answered in
    /// request order.
    pub points: Vec<[u16; 2]>,
}

/// One sampled texel: where it was asked, what the map holds there.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SamplePoint {
    /// The point as requested, in texels.
    pub at: [u16; 2],
    /// Height at that texel, in texel steps.
    pub height: u8,
    /// True when that texel is a hole (no ground).
    pub hole: bool,
    /// The three splat weights there; they sum to 255.
    pub splat: [u8; 3],
}

/// `world.terrain.sample` result: the map it read and one row per point.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SampleOutput {
    /// Square side in texels.
    pub size: u16,
    /// Digest of the TVER bytes read or baked.
    pub digest: String,
    /// One row per requested point, in request order.
    pub points: Vec<SamplePoint>,
}

/// One edit in `world.terrain.paint`: a rect, a height delta, an optional
/// hole set and optional splat weights.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct EditArg {
    /// Rect as `[x, y, w, h]` in texels; the whole rect must fit inside the
    /// map and both sides must be at least 1.
    pub rect: [u16; 4],
    /// Added to every height in the rect, then clamped to `0..=255`.
    pub height_delta: i16,
    /// `Some` sets or clears every hole bit in the rect; `None` leaves them.
    #[serde(default)]
    pub hole: Option<bool>,
    /// `Some` sets three splat weights (sum exactly 255) in the rect; `None`
    /// leaves splat alone.
    #[serde(default)]
    pub splat: Option<[u8; 3]>,
}

impl EditArg {
    /// The world crate's plain edit; keeps one edit shape in the tree.
    fn to_edit(self) -> Edit {
        Edit {
            rect: self.rect,
            height_delta: self.height_delta,
            hole: self.hole,
            splat: self.splat,
        }
    }
}

/// Args for `world.terrain.paint`: the seed bake to edit, the edits in order,
/// and an optional path to write the result under the project root.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PaintArgs {
    /// Bake seed the edits apply to.
    pub seed: u64,
    /// Square side in texels, `1..=engine_world::MAX_BAKE_SIZE` (256).
    pub size: u16,
    /// Edits in order, `1..=MAX_EDITS` of them; they apply in the order given.
    pub edits: Vec<EditArg>,
    /// Relative TVER path under the project root for the painted map.
    /// Omitted writes nothing.
    #[serde(default)]
    pub out: Option<String>,
}

/// `world.terrain.paint` result: the painted bytes' stats plus what moved.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PaintOutput {
    /// Length of the painted TVER bytes.
    pub bytes: u64,
    /// Digest of those bytes; the same seed and edits answer the same digest.
    pub digest: String,
    /// Square side in texels.
    pub size: u16,
    /// Lowest height after the edits, in texel steps.
    pub height_min: u8,
    /// Highest height after the edits, in texel steps.
    pub height_max: u8,
    /// Mean height times 100 after the edits, truncated.
    pub height_mean_x100: u32,
    /// Texels with a hole after the edits.
    pub holes: u32,
    /// Per-layer permille coverage after the edits.
    pub splat_coverage: [u32; 3],
    /// Cells `blocked` refuses at the default slope after the edits.
    pub blocked_cells: u32,
    /// Cells whose height, hole or splat the edits changed; a cell counts
    /// once however many edits touch it.
    pub changed_cells: u32,
    /// How many edits the call applied.
    pub edits: u32,
    /// The relative path written, when the call named one.
    pub out: Option<String>,
}

/// 16 lowercase hex chars: the crate's one terrain digest.
fn digest_of(bytes: &[u8]) -> String {
    format!("{:016x}", terrain_hash(bytes))
}

/// Typed refusal for a side a bake or paint cannot take.
fn size_refused(size: u16) -> McpToolError {
    bad_args(format!(
        "size {size} is outside 1..={MAX_BAKE_SIZE}; re-run with the square side in texels, \
         for example size {DEFAULT_SIZE}"
    ))
}

/// Guard a requested side before any bake allocates.
fn check_size(size: u16) -> Result<(), McpToolError> {
    if size == 0 || size > MAX_BAKE_SIZE {
        return Err(size_refused(size));
    }
    Ok(())
}

/// The terrain a bake just made. Its own bytes always parse, so a miss means
/// the writer and the parser disagree (a bug, never caller input).
fn terrain_of(bytes: &[u8]) -> Result<Terrain, McpToolError> {
    parse_terrain(bytes).ok_or_else(|| {
        McpToolError::new(
            "bad_terrain",
            "the freshly baked bytes did not parse; report this verb with its seed and size",
        )
    })
}

/// The summary the bake and paint verbs report, from bytes and their terrain.
fn summary_of(bytes: &[u8], terrain: &Terrain) -> TerrainSummary {
    let stats = terrain_stats(terrain);
    TerrainSummary {
        bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        digest: digest_of(bytes),
        size: terrain.size,
        height_min: stats.height_min,
        height_max: stats.height_max,
        height_mean_x100: stats.height_mean_x100,
        holes: stats.holes,
        splat_coverage: stats.splat_coverage,
        blocked_cells: stats.blocked_cells,
        out: None,
    }
}

/// A relative path under the project root: no empty name, no root, no `..`.
fn relative_out(rel: &str) -> Result<&Path, McpToolError> {
    let path = Path::new(rel);
    let escaped = path.components().any(|part| {
        matches!(
            part,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    });
    if rel.trim().is_empty() || escaped {
        return Err(bad_args(format!(
            "path `{rel}` must be relative and stay under the project root with no `..`; \
             re-run with a path such as target/terrain/tver.bin"
        )));
    }
    Ok(path)
}

/// Write one payload at `rel` under `root`, creating parent folders.
fn write_out(root: &Path, rel: &str, bytes: &[u8]) -> Result<(), McpToolError> {
    let path = root.join(relative_out(rel)?);
    let failed = |why: std::io::Error| {
        McpToolError::new(
            "write_failed",
            format!(
                "could not write `{rel}` under the project root ({why}); re-run with a writable \
                 relative path such as target/terrain/tver.bin"
            ),
        )
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(failed)?;
    }
    std::fs::write(&path, bytes).map_err(failed)
}

/// Read one TVER payload at `rel` under `root`.
fn read_in(root: &Path, rel: &str) -> Result<Vec<u8>, McpToolError> {
    let path = root.join(relative_out(rel)?);
    std::fs::read(&path).map_err(|why| {
        McpToolError::new(
            "read_failed",
            format!(
                "could not read terrain `{rel}` under the project root ({why}); re-run with a \
                 TVER file this door wrote, for example world.terrain.bake out:\"target/terrain/tver.bin\""
            ),
        )
    })
}

/// Bake one terrain and optionally write it.
fn run_bake(root: &Path, args: &BakeArgs) -> Result<TerrainSummary, McpToolError> {
    check_size(args.size)?;
    let bytes = bake_terrain(args.seed, args.size).map_err(|_| size_refused(args.size))?;
    let terrain = terrain_of(&bytes)?;
    let mut summary = summary_of(&bytes, &terrain);
    if let Some(rel) = args.out.as_deref() {
        write_out(root, rel, &bytes)?;
        summary.out = Some(rel.to_owned());
    }
    Ok(summary)
}

/// Resolve one map from its source: a seed bake or a written TVER file.
fn resolve_source(
    root: &Path,
    seed: Option<u64>,
    size: Option<u16>,
    path: Option<&str>,
) -> Result<(Vec<u8>, Terrain), McpToolError> {
    match (seed, path) {
        (Some(seed), None) => {
            let size = size.unwrap_or(DEFAULT_SIZE);
            check_size(size)?;
            let bytes = bake_terrain(seed, size).map_err(|_| size_refused(size))?;
            let terrain = terrain_of(&bytes)?;
            Ok((bytes, terrain))
        }
        (None, Some(rel)) => {
            let bytes = read_in(root, rel)?;
            let terrain = parse_terrain(&bytes).ok_or_else(|| {
                bad_args(format!(
                    "path `{rel}` is not a TVER v1 terrain ({} bytes); re-run with a file \
                     world.terrain.bake wrote, named by its `out`",
                    bytes.len()
                ))
            })?;
            Ok((bytes, terrain))
        }
        _ => Err(bad_args(
            "pass either `seed` (with an optional size) or `path`, never both and never neither; \
             for example {seed: 7, size: 64, points: [[0, 0]]} or \
             {path: \"target/terrain/tver.bin\", points: [[0, 0]]}"
                .to_owned(),
        )),
    }
}

/// Resolve the map a sample reads: a seed bake or a written TVER file.
fn sampled_terrain(root: &Path, args: &SampleArgs) -> Result<(Vec<u8>, Terrain), McpToolError> {
    resolve_source(root, args.seed, args.size, args.path.as_deref())
}

/// Refusal for a point list that is empty or over [`MAX_POINTS`].
fn points_refused(count: usize) -> McpToolError {
    bad_args(format!(
        "points holds {count} point(s) outside 1..={MAX_POINTS}; re-run with that many `[x, y]` \
         texel pairs, for example [[0, 0], [1, 1]]"
    ))
}

/// Sample one terrain at the requested points, in request order.
fn run_sample(root: &Path, args: &SampleArgs) -> Result<SampleOutput, McpToolError> {
    if args.points.is_empty() || args.points.len() > MAX_POINTS {
        return Err(points_refused(args.points.len()));
    }
    let (bytes, terrain) = sampled_terrain(root, args)?;
    let n = usize::from(terrain.size);
    let mut points = Vec::with_capacity(args.points.len());
    for at in &args.points {
        let (x, y) = (usize::from(at[0]), usize::from(at[1]));
        if x >= n || y >= n {
            return Err(bad_args(format!(
                "point [{}, {}] is outside the {n} x {n} map (each coordinate 0..={}); re-run \
                 with x and y inside 0..={}, or bake size {} to cover it",
                at[0],
                at[1],
                n.saturating_sub(1),
                n.saturating_sub(1),
                u32::from(at[0].max(at[1]))
                    .saturating_add(1)
                    .min(u32::from(MAX_BAKE_SIZE))
            )));
        }
        let i = y * n + x;
        points.push(SamplePoint {
            at: *at,
            height: terrain.heights[i],
            hole: terrain.holes[i],
            splat: terrain.splat[i],
        });
    }
    Ok(SampleOutput {
        size: terrain.size,
        digest: digest_of(&bytes),
        points,
    })
}

/// Refusal for an edit list that is empty or over [`MAX_EDITS`].
fn edits_refused(count: usize) -> McpToolError {
    bad_args(format!(
        "edits holds {count} edit(s) outside 1..={MAX_EDITS}; re-run with at least one \
         {{rect: [x, y, w, h], height_delta: 0}} edit"
    ))
}

/// Bake, paint and optionally write one terrain.
fn run_paint(root: &Path, args: &PaintArgs) -> Result<PaintOutput, McpToolError> {
    if args.edits.is_empty() || args.edits.len() > MAX_EDITS {
        return Err(edits_refused(args.edits.len()));
    }
    check_size(args.size)?;
    let baked = bake_terrain(args.seed, args.size).map_err(|_| size_refused(args.size))?;
    let mut terrain = terrain_of(&baked)?;
    let edits: Vec<Edit> = args.edits.iter().copied().map(EditArg::to_edit).collect();
    let changed = paint_terrain(&mut terrain, &edits).map_err(|error| {
        bad_args(format!(
            "{error}; re-run with a rect inside the map and splat weights that sum to 255 \
             (for example [255, 0, 0])"
        ))
    })?;
    let bytes = write_terrain(&terrain).map_err(|_| size_refused(args.size))?;
    let mut out = PaintOutput {
        changed_cells: u32::try_from(changed).unwrap_or(u32::MAX),
        edits: u32::try_from(edits.len()).unwrap_or(u32::MAX),
        out: None,
        ..PaintOutput::default()
    };
    fill_paint(&mut out, &bytes, &terrain);
    if let Some(rel) = args.out.as_deref() {
        write_out(root, rel, &bytes)?;
        out.out = Some(rel.to_owned());
    }
    Ok(out)
}

/// Copy the shared terrain stats onto a paint result.
fn fill_paint(out: &mut PaintOutput, bytes: &[u8], terrain: &Terrain) {
    let stats = terrain_stats(terrain);
    out.bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    out.digest = digest_of(bytes);
    out.size = terrain.size;
    out.height_min = stats.height_min;
    out.height_max = stats.height_max;
    out.height_mean_x100 = stats.height_mean_x100;
    out.holes = stats.holes;
    out.splat_coverage = stats.splat_coverage;
    out.blocked_cells = stats.blocked_cells;
}

/// `world.terrain.bake` handler: the project root the door was built in.
fn bake_verb(_server: &McpServer, args: &BakeArgs) -> Result<TerrainSummary, McpToolError> {
    run_bake(&super::codex::project_root(None), args)
}

/// `world.terrain.sample` handler: the project root the door was built in.
fn sample_verb(_server: &McpServer, args: &SampleArgs) -> Result<SampleOutput, McpToolError> {
    run_sample(&super::codex::project_root(None), args)
}

/// `world.terrain.paint` handler: the project root the door was built in.
fn paint_verb(_server: &McpServer, args: &PaintArgs) -> Result<PaintOutput, McpToolError> {
    run_paint(&super::codex::project_root(None), args)
}

/// Verb: scatter seeded Poisson-disk points over a square.
pub const TOOL_WORLD_SCATTER: &str = "world.scatter";
/// Verb: solve a wave-function-collapse tile map.
pub const TOOL_WORLD_WFC: &str = "world.wfc";
/// Verb: run a seeded rewrite-rule program from its rules text.
pub const TOOL_WORLD_REWRITE: &str = "world.rewrite";
/// Side, in cells, the rewrite door refuses past (the rules format's cap).
pub const MAX_REWRITE_SIDE: u16 = engine_world::rewrite::MAX_SIDE;

/// Most cells one scatter side may name; a cell is one centimetre.
pub const MAX_SCATTER_SIDE: u16 = 4096;
/// Most points one `world.scatter` call answers.
pub const MAX_SCATTER_POINTS: usize = 65_536;
/// Extra seeds `world.wfc` tries after the first one refuses.
pub const MAX_WFC_RETRIES: u32 = 8;
/// Recipe name the scatter seed mixes into: one name, so the same seed keeps
/// the same points.
pub const SCATTER_RECIPE: &str = "world.scatter";

/// A terrain a scatter reads heights and holes from: the same two sources as
/// `world.terrain.sample`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TerrainRef {
    /// Bake seed; pass either this (with an optional `size`) or `path`.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Side to bake when `seed` is given; defaults to [`DEFAULT_SIZE`].
    #[serde(default)]
    pub size: Option<u16>,
    /// Relative TVER path under the project root; pass instead of `seed`.
    #[serde(default)]
    pub path: Option<String>,
}

/// Args for `world.scatter`: a square, a spacing, an optional terrain and an
/// optional JSON path to write under the project root.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ScatterArgs {
    /// Scatter seed; the same seed and spacing always give the same points.
    pub seed: u64,
    /// Square side in 1 cm cells, `1..=`[`MAX_SCATTER_SIDE`].
    pub size: u16,
    /// Smallest gap between two points in cm, `1..=size / 2`.
    pub radius_cm: u16,
    /// Most points to answer, `1..=`[`MAX_SCATTER_POINTS`]; omitted answers
    /// every point the spacing allows.
    #[serde(default)]
    pub count: Option<u32>,
    /// Terrain to read height and hole from; omitted answers neither.
    #[serde(default)]
    pub terrain: Option<TerrainRef>,
    /// Relative JSON path under the project root for the placement
    /// (`target/world/roman-camp-scatter.json`). Omitted writes nothing.
    #[serde(default)]
    pub out: Option<String>,
}

/// One scattered point: where it landed and what the terrain holds there.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ScatterPoint {
    /// Cell as `[x, y]` in 1 cm cells.
    pub at: [u16; 2],
    /// Height at that cell, when the call named a terrain.
    #[serde(default)]
    pub height: Option<u8>,
    /// True when that cell is a hole, when the call named a terrain.
    #[serde(default)]
    pub hole: Option<bool>,
}

/// `world.scatter` result: the points in `(x, y)` order and their digest.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ScatterOutput {
    /// Square side in 1 cm cells.
    pub size: u16,
    /// Smallest gap the call asked for, in cm.
    pub radius_cm: u16,
    /// How many points the answer holds.
    pub point_count: u32,
    /// FNV-1a 64 of the point cells as 16 lowercase hex chars; the same seed,
    /// spacing and count answer the same digest.
    pub digest: String,
    /// Points sorted by `x`, then `y`; one point per cell.
    pub points: Vec<ScatterPoint>,
    /// The relative path written, when the call named one.
    pub out: Option<String>,
}

/// The JSON placement file `world.scatter` writes: the cells and the digest
/// the preview answered.
#[derive(Debug, Clone, Serialize)]
#[serde(crate = "rmcp::serde")]
struct PlacementDoc {
    seed: u64,
    size: u16,
    radius_cm: u16,
    digest: String,
    cells: Vec<[u16; 2]>,
}

/// Args for `world.wfc`: a sized map, one adjacency mask per kind, an optional
/// passable mask and an optional JSON path to write under the project root.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct WfcArgs {
    /// Solve seed; when the bake refuses, later seeds are tried in order.
    pub seed: u64,
    /// Cells across, `1..=engine_world::wfc::MAX_WFC_SIDE` (128).
    pub width: u16,
    /// Cells down, `1..=engine_world::wfc::MAX_WFC_SIDE` (128).
    pub height: u16,
    /// How many kinds the table names, `1..=engine_world::wfc::MAX_KINDS` (16).
    pub kinds: u8,
    /// One `[+x, -x, +z, -z]` row per kind: a 16-bit mask of the kinds that
    /// may sit on that side.
    pub masks: Vec<[u16; DIRECTIONS]>,
    /// Kinds that count as walkable for the connectivity check, as a 16-bit
    /// mask; omitted counts every kind.
    #[serde(default)]
    pub passable: Option<u16>,
    /// Relative JSON path under the project root for the solved map
    /// (`target/world/roman-camp-wfc.json`). Omitted writes nothing.
    #[serde(default)]
    pub out: Option<String>,
}

/// `world.wfc` result: the solved cells and the checks that ran.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct WfcOutput {
    /// Cells across.
    pub width: u16,
    /// Cells down.
    pub height: u16,
    /// The seed that solved it: the requested seed plus the retries taken.
    pub seed: u64,
    /// How many kinds the table named.
    pub kinds: u8,
    /// One lowercase hex digit per kind index, `x` fastest; `width * height`
    /// characters.
    pub cells: String,
    /// FNV-1a 64 of the cells as 16 lowercase hex chars; the same seed, table
    /// and constraints answer the same digest.
    pub digest: String,
    /// True when every passable cell is reachable from every other.
    pub connected: bool,
    /// Extra seeds the solve took after the requested one: `0` means the first
    /// seed solved.
    pub retries: u32,
    /// The relative path written, when the call named one.
    pub out: Option<String>,
}

/// Every kind bit for a `kinds`-wide table (`kinds` is at most 16).
fn all_kinds(kinds: u8) -> u16 {
    1u16.checked_shl(u32::from(kinds))
        .map_or(u16::MAX, |mask| mask.saturating_sub(1))
}

/// The cell a bake-time point falls in: `v` is in cell units on `[0, size)`.
/// A binary search keeps the door integer and casts no float.
fn cell_of(v: f32, size: u16) -> u16 {
    let mut low = 0u16;
    let mut high = size.saturating_sub(1);
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        if f32::from(mid) <= v {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    low
}

/// Refusal for a scatter square outside `1..=`[`MAX_SCATTER_SIDE`].
fn scatter_size_refused(size: u16) -> McpToolError {
    bad_args(format!(
        "size {size} is outside 1..={MAX_SCATTER_SIDE} cells; re-run with a square side in 1 cm \
         cells, for example 64"
    ))
}

/// Refusal for a spacing that cannot fit the square twice.
fn radius_refused(radius_cm: u16, size: u16) -> McpToolError {
    let biggest = size / 2;
    bad_args(format!(
        "radius_cm {radius_cm} does not fit a {size} x {size} cell square twice; re-run with a \
         spacing in 1..={biggest} cm, for example 8"
    ))
}

/// Check the square, the spacing and the count before any sample; answers the
/// most points the call may answer.
fn scatter_want(args: &ScatterArgs) -> Result<usize, McpToolError> {
    if args.size == 0 || args.size > MAX_SCATTER_SIDE {
        return Err(scatter_size_refused(args.size));
    }
    let radius = u32::from(args.radius_cm);
    if radius == 0 || radius * 2 > u32::from(args.size) {
        return Err(radius_refused(args.radius_cm, args.size));
    }
    let want = args.count.map_or(MAX_SCATTER_POINTS, |count| {
        usize::try_from(count).unwrap_or(usize::MAX)
    });
    if want == 0 || want > MAX_SCATTER_POINTS {
        return Err(bad_args(format!(
            "count {want} is outside 1..={MAX_SCATTER_POINTS}; re-run with the most points to \
             answer, for example 512"
        )));
    }
    Ok(want)
}

/// Sample the square, map every point to its cell, sort by `(x, y)`, keep one
/// point per cell and cut the answer to `want` points.
fn scatter_cells(args: &ScatterArgs, want: usize) -> Result<Vec<[u16; 2]>, McpToolError> {
    let side = f32::from(args.size);
    let sampled = poisson_disk(
        args.seed,
        SCATTER_RECIPE,
        side,
        side,
        f32::from(args.radius_cm),
    )
    .map_err(|why| {
        bad_args(format!(
            "{why}: radius_cm {} and size {} do not fit a Poisson-disk grid; re-run with a \
             spacing in 1..={} cm",
            args.radius_cm,
            args.size,
            args.size / 2
        ))
    })?;
    let mut cells: Vec<[u16; 2]> = sampled
        .iter()
        .map(|point| [cell_of(point[0], args.size), cell_of(point[1], args.size)])
        .collect();
    cells.sort_by_key(|at| (at[0], at[1]));
    cells.dedup();
    cells.truncate(want);
    Ok(cells)
}

/// One row per cell: where it is, and what the named terrain holds there.
fn scatter_rows(map: Option<&Terrain>, cells: &[[u16; 2]]) -> Vec<ScatterPoint> {
    let mut points = Vec::with_capacity(cells.len());
    for at in cells {
        let (height, hole) = match map {
            Some(terrain) => {
                let n = usize::from(terrain.size);
                let x = usize::from(at[0]).min(n.saturating_sub(1));
                let y = usize::from(at[1]).min(n.saturating_sub(1));
                let i = y * n + x;
                (Some(terrain.heights[i]), Some(terrain.holes[i]))
            }
            None => (None, None),
        };
        points.push(ScatterPoint {
            at: *at,
            height,
            hole,
        });
    }
    points
}

/// FNV-1a 64 over the point cells as little-endian pairs, as 16 hex chars.
fn scatter_digest(cells: &[[u16; 2]]) -> String {
    let mut plain = Vec::with_capacity(cells.len() * 4);
    for at in cells {
        plain.extend_from_slice(&at[0].to_le_bytes());
        plain.extend_from_slice(&at[1].to_le_bytes());
    }
    format!("{:016x}", terrain_hash(&plain))
}

/// Scatter one square of Poisson-disk points and optionally write the
/// placement, reading height and hole from the named terrain.
fn run_scatter(root: &Path, args: &ScatterArgs) -> Result<ScatterOutput, McpToolError> {
    let want = scatter_want(args)?;
    let cells = scatter_cells(args, want)?;
    let map = match &args.terrain {
        Some(source) => {
            Some(resolve_source(root, source.seed, source.size, source.path.as_deref())?.1)
        }
        None => None,
    };
    let points = scatter_rows(map.as_ref(), &cells);
    let digest = scatter_digest(&cells);
    let mut out = ScatterOutput {
        size: args.size,
        radius_cm: args.radius_cm,
        point_count: u32::try_from(points.len()).unwrap_or(u32::MAX),
        digest: digest.clone(),
        points,
        out: None,
    };
    if let Some(rel) = args.out.as_deref() {
        let doc = PlacementDoc {
            seed: args.seed,
            size: args.size,
            radius_cm: args.radius_cm,
            digest,
            cells,
        };
        let text = rmcp::serde_json::to_string(&doc).map_err(|why| {
            McpToolError::new(
                "write_failed",
                format!(
                    "could not serialize the placement ({why}); re-run with the same seed, size \
                     and radius_cm"
                ),
            )
        })?;
        write_out(root, rel, text.as_bytes())?;
        out.out = Some(rel.to_owned());
    }
    Ok(out)
}

/// Refusal for an adjacency table the crate will not take.
fn mask_refused(error: WfcError) -> McpToolError {
    bad_args(format!(
        "{error}; re-run with one `[+x, -x, +z, -z]` mask per kind, 1..={MAX_KINDS} rows, every \
         bit below `kinds`"
    ))
}

/// Solve the map, trying later seeds while the bake refuses; answers the cells,
/// the seed that worked and how many retries it took.
fn solve_map(args: &WfcArgs, adjacency: &Adjacency) -> Result<(Vec<u8>, u64, u32), McpToolError> {
    let mut seed = args.seed;
    let mut retries = 0u32;
    loop {
        match wfc_bake(args.width, args.height, adjacency, seed) {
            Ok(cells) => return Ok((cells, seed, retries)),
            Err(WfcError::TooLarge { side }) => {
                return Err(bad_args(format!(
                    "wfc side {side} is outside 1..={MAX_WFC_SIDE}; re-run with a smaller width \
                     or height"
                )));
            }
            Err(error) => {
                if retries >= MAX_WFC_RETRIES {
                    return Err(bad_args(format!(
                        "{error} after {retries} retries from seed {}; re-run with another seed or \
                         a table that admits more neighbours",
                        args.seed
                    )));
                }
                retries += 1;
                seed = seed.wrapping_add(1);
            }
        }
    }
}

/// One lowercase hex digit per kind index, `x` fastest.
fn cells_text(cells: &[u8]) -> String {
    let mut text = String::with_capacity(cells.len());
    for kind in cells {
        text.push(char::from_digit(u32::from(*kind), 16).unwrap_or('?'));
    }
    text
}

/// Solve one map, trying later seeds when the bake refuses, then check that the
/// passable kinds form one region and optionally write the map.
fn run_wfc(root: &Path, args: &WfcArgs) -> Result<WfcOutput, McpToolError> {
    if args.width == 0
        || args.height == 0
        || args.width > MAX_WFC_SIDE
        || args.height > MAX_WFC_SIDE
    {
        return Err(bad_args(format!(
            "width {} x height {} is outside 1..={MAX_WFC_SIDE} per side; re-run with a smaller \
             map, for example 16 x 16",
            args.width, args.height
        )));
    }
    let adjacency = Adjacency::new(args.kinds, &args.masks).map_err(mask_refused)?;
    let allowed = all_kinds(args.kinds);
    let passable = args.passable.unwrap_or(allowed);
    if passable == 0 || passable & !allowed != 0 {
        return Err(bad_args(format!(
            "passable {passable} names kinds outside the {allowed} table (bits 0..={}); re-run \
             with a mask of the kinds that may be walked, for example {allowed}",
            args.kinds.saturating_sub(1)
        )));
    }
    let (solved, seed, retries) = solve_map(args, &adjacency)?;
    if !is_connected(&solved, args.width, args.height, passable) {
        return Err(McpToolError::new(
            "wfc_disconnected",
            format!(
                "the solve with {retries} retries leaves the passable kinds {passable} in more \
                 than one region; re-run with a `passable` mask that covers more kinds, for \
                 example {allowed}, or another seed"
            ),
        ));
    }
    let text = cells_text(&solved);
    let digest = format!("{:016x}", terrain_hash(&solved));
    let mut out = WfcOutput {
        width: args.width,
        height: args.height,
        seed,
        kinds: args.kinds,
        cells: text,
        digest: digest.clone(),
        connected: true,
        retries,
        out: None,
    };
    if let Some(rel) = args.out.as_deref() {
        let doc = WfcDoc {
            cells: out.cells.clone(),
            width: args.width,
            height: args.height,
            seed,
            digest,
        };
        let json = rmcp::serde_json::to_string(&doc).map_err(|why| {
            McpToolError::new(
                "write_failed",
                format!(
                    "could not serialize the solved map ({why}); re-run with the same seed and \
                     table"
                ),
            )
        })?;
        write_out(root, rel, json.as_bytes())?;
        out.out = Some(rel.to_owned());
    }
    Ok(out)
}

/// The compact solved map `world.wfc` writes.
#[derive(Debug, Clone, Serialize)]
#[serde(crate = "rmcp::serde")]
struct WfcDoc {
    cells: String,
    width: u16,
    height: u16,
    seed: u64,
    digest: String,
}

/// `world.scatter` handler: the project root the door was built in.
fn scatter_verb(_server: &McpServer, args: &ScatterArgs) -> Result<ScatterOutput, McpToolError> {
    run_scatter(&super::codex::project_root(None), args)
}

/// `world.wfc` handler: the project root the door was built in.
fn wfc_verb(_server: &McpServer, args: &WfcArgs) -> Result<WfcOutput, McpToolError> {
    run_wfc(&super::codex::project_root(None), args)
}

/// Args for `world.rewrite`: the rules text, a seed and an optional path for the
/// grid text under the project root.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RewriteArgs {
    /// Rules text: `size W H` and `alphabet SYMBOLS` first, then `put`, `rect`
    /// and the `one`, `all`, `prl`, `markov` and `sequence` nodes; `#` starts a
    /// comment. The first symbol fills the grid at the start.
    pub rules: String,
    /// Run seed; the same rules and seed always give the same grid and hash.
    pub seed: u64,
    /// Relative text path under the project root for the grid
    /// (`target/world/insula.txt`). Omitted writes nothing.
    #[serde(default)]
    pub out: Option<String>,
}

/// `world.rewrite` result: the grid the rules made, its hash and the steps taken.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RewriteOutput {
    /// Cells across.
    pub width: u16,
    /// Cells down.
    pub height: u16,
    /// The alphabet, fill symbol first: each character is one symbol.
    pub symbols: String,
    /// One line per row of symbol characters, a `\n` after each row; row 0 is
    /// the top row.
    pub grid: String,
    /// Content hash of the grid: `0x` and 16 lowercase hex digits. The same
    /// rules and seed answer the same hash.
    pub hash: String,
    /// Steps the run took, every node together.
    pub steps: u32,
    /// The relative path written, when the call named one.
    pub out: Option<String>,
}

/// Refusal for a rules text the parser will not take; names the 1-based line.
fn rules_refused(error: &RulesError) -> McpToolError {
    bad_args(format!(
        "{error}; re-run with that statement fixed: size takes 1..={MAX_REWRITE_SIDE} cells per \
         side, and every symbol must sit in the alphabet line"
    ))
}

/// Refusal for a run that passes its step or work cap; no partial grid comes back.
fn run_refused(error: RunError) -> McpToolError {
    bad_args(format!(
        "{error}; re-run with a `steps=N` cap on the node that never settles, or rules that stop \
         changing cells"
    ))
}

/// Parse the rules, run them with the seed and optionally write the grid text.
fn run_rewrite(root: &Path, args: &RewriteArgs) -> Result<RewriteOutput, McpToolError> {
    let program = Program::parse(&args.rules).map_err(|error| rules_refused(&error))?;
    let grid = program.run(args.seed).map_err(run_refused)?;
    let mut out = RewriteOutput {
        width: grid.width(),
        height: grid.height(),
        symbols: program.symbols(),
        grid: grid.render(),
        hash: format!("0x{:016x}", grid.hash()),
        steps: grid.steps(),
        out: None,
    };
    if let Some(rel) = args.out.as_deref() {
        write_out(root, rel, out.grid.as_bytes())?;
        out.out = Some(rel.to_owned());
    }
    Ok(out)
}

/// `world.rewrite` handler: the project root the door was built in.
fn rewrite_verb(_server: &McpServer, args: &RewriteArgs) -> Result<RewriteOutput, McpToolError> {
    run_rewrite(&super::codex::project_root(None), args)
}

crate::e2_tool!(
    /// `world.terrain.bake`: bake a seeded TVER map and report its stats.
    TerrainBakeTool,
    name = TOOL_WORLD_TERRAIN_BAKE,
    description = "Bake a seeded terrain (heightmap + hole mask + 3-way splat, TVER v1) and answer its bytes, digest, side, height range and mean, holes, per-layer splat coverage and blocked cells at the default slope. Writes the file only when `out` names a relative path under the project root; the same seed and size always give the same digest.",
    readonly = false,
    args = BakeArgs,
    output = TerrainSummary,
    invoke = bake_verb
);

crate::e2_tool!(
    /// `world.terrain.sample`: height, hole and splat at points.
    TerrainSampleTool,
    name = TOOL_WORLD_TERRAIN_SAMPLE,
    description = "Read up to 4096 points of a terrain: height, hole bit and the three splat weights, in request order. Name either `seed` (with an optional size) or `path`, a TVER file under the project root that world.terrain.bake wrote; a point outside the map refuses and names the legal range. Read-only.",
    readonly = true,
    args = SampleArgs,
    output = SampleOutput,
    invoke = sample_verb
);

crate::e2_tool!(
    /// `world.terrain.paint`: bounded edits over a seeded terrain.
    TerrainPaintTool,
    name = TOOL_WORLD_TERRAIN_PAINT,
    description = "Paint a seeded terrain with 1..=4096 edits in order: a `[x, y, w, h]` rect, a height delta clamped to 0..=255, an optional hole set and optional splat weights that sum to 255. Answers the same stats as world.terrain.bake plus changed_cells, and writes the TVER file only when `out` names a relative path under the project root; the same seed and edits always give the same digest.",
    readonly = false,
    args = PaintArgs,
    output = PaintOutput,
    invoke = paint_verb
);

crate::e2_tool!(
    /// `world.scatter`: seeded Poisson-disk points over a square.
    ScatterTool,
    name = TOOL_WORLD_SCATTER,
    description = "Scatter seeded Poisson-disk points over a square of 1 cm cells: 1..=65536 points sorted by x then y, one per cell, at least `radius_cm` apart. A radius that cannot fit twice across the square refuses. Name `terrain` (seed with an optional size, or a TVER path under the project root) to answer each point's height and hole. Writes a JSON placement (cells, seed, size, radius_cm, digest) only when `out` names a relative path; the same seed, size and spacing always give the same points and digest.",
    readonly = false,
    args = ScatterArgs,
    output = ScatterOutput,
    invoke = scatter_verb
);

crate::e2_tool!(
    /// `world.wfc`: solve a tile map from an adjacency table.
    WfcTool,
    name = TOOL_WORLD_WFC,
    description = "Solve a wave-function-collapse tile map of up to 128 x 128 cells from 1..=16 kinds and one [+x, -x, +z, -z] adjacency mask per kind. Answers one hex digit per cell (x fastest), the digest, the connectivity flag and the retries (later seeds tried after a refusal). A solve that leaves the passable kinds in more than one region refuses with the retry count and one recovery. Writes a compact JSON map only when `out` names a relative path; the same seed and table always give the same cells.",
    readonly = false,
    args = WfcArgs,
    output = WfcOutput,
    invoke = wfc_verb
);

crate::e2_tool!(
    /// `world.rewrite`: run a seeded rewrite-rule program from its rules text.
    RewriteTool,
    name = TOOL_WORLD_REWRITE,
    description = "Run a seeded rewrite-rule grid from its rules text: MarkovJunior-style `IN=OUT` rules in one, all, prl, markov and sequence nodes (the format is in engine_world::rewrite). The rules name `size W H` (1..=128 cells per side) and `alphabet` first. Answers the grid as text, one line per row, its `0x` FNV hash and the steps taken. A bad statement refuses with its line number, and a run that never settles refuses instead of answering a partial grid. Writes the grid text only when `out` names a relative path under the project root; the same rules and seed always give the same grid and hash.",
    readonly = false,
    args = RewriteArgs,
    output = RewriteOutput,
    invoke = rewrite_verb
);

/// The world pack: terrain bake, sample and paint, loaded on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "terrain and layout: world.terrain.bake bakes a seeded TVER heightmap, hole mask and \
         3-way splat and reports its stats; world.terrain.sample answers height, hole and splat \
         at up to 4096 points; world.terrain.paint applies rect height/hole/splat edits and \
         reports what moved; world.scatter lays seeded Poisson-disk points over a square with \
         terrain heights; world.wfc solves a tile map from an adjacency table; world.rewrite \
         runs a seeded rewrite-rule grid from its rules text. Every writer \
         writes only a relative `out` path under the project root",
        false,
    );
    pack.add(VerbSpec::of::<TerrainBakeTool>(
        TOOL_WORLD_TERRAIN_BAKE,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<TerrainSampleTool>(
        TOOL_WORLD_TERRAIN_SAMPLE,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<TerrainPaintTool>(
        TOOL_WORLD_TERRAIN_PAINT,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<TerrainSetTool>(
        TOOL_WORLD_TERRAIN_SET,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<TerrainStatsTool>(
        TOOL_WORLD_TERRAIN_STATS,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<ScatterTool>(
        TOOL_WORLD_SCATTER,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<WfcTool>(
        TOOL_WORLD_WFC,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<RewriteTool>(
        TOOL_WORLD_REWRITE,
        PACK,
        CapabilityCard::Commit,
    )?);
    Ok(pack)
}

/// Verb: cook a declared height grid into this session's terrain mesh.
pub const TOOL_WORLD_TERRAIN_SET: &str = "world.terrain.set";
/// Verb: report this session's cooked terrain mesh stats.
pub const TOOL_WORLD_TERRAIN_STATS: &str = "world.terrain.stats";
/// The name the render lane draws a cooked height grid under; it equals
/// `engine_render::terrain::TERRAIN_DRAW_NAME`, which this crate does not link.
pub const TERRAIN_DRAW_NAME: &str = "terrain";
/// The mesh identity the render lane's draw list carries for a cooked grid
/// (`"TERRAIN1"`); it equals `engine_render::terrain::TERRAIN_MESH_ID`.
pub const TERRAIN_MESH_ID: u64 = 0x5445_5252_4149_4e31;

/// Highest millimetre height a grid may declare; the cook's own cap.
pub const MAX_TERRAIN_HEIGHT_MM: i32 = 1_000_000;
/// Widest sample spacing a grid may declare, in millimetres.
pub const MAX_TERRAIN_SPACING_MM: u32 = 1_000_000;
/// Rows or columns a grid may declare; the cook's own cap.
pub const MAX_TERRAIN_SIDE: u32 = 1024;

/// A declared height grid: integer millimetre heights, row-major.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TerrainSetArgs {
    /// Rows along Z, 2..=1024.
    pub rows: u32,
    /// Columns along X, 2..=1024.
    pub cols: u32,
    /// Distance between neighbouring samples in whole millimetres, 1..=1000000.
    pub spacing_mm: u32,
    /// Heights in millimetres, row-major, exactly `rows * cols` entries.
    pub heights_mm: Vec<i32>,
}

/// What `world.terrain.set` answers: the session revision and the cooked mesh.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TerrainSetOutput {
    /// How many terrain sets this session has made; 1 on the first.
    pub revision: u64,
    /// Vertices the cook made, `rows * cols`.
    pub vertices: u32,
    /// Triangles the cook made, `(rows - 1) * (cols - 1) * 2`.
    pub triangles: u32,
}

/// `world.terrain.stats` takes no arguments: it reads this session's terrain.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "a fieldless door argument DTO keeps the brace form its sibling DTOs use, so its JSON schema stays an object"
)]
pub struct TerrainStatsArgs {}

/// What `world.terrain.stats` answers: the cooked mesh in the render lane's terms.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TerrainStatsOutput {
    /// Session revision of the stored terrain.
    pub revision: u64,
    /// Rows along Z.
    pub rows: u32,
    /// Columns along X.
    pub cols: u32,
    /// Sample spacing in millimetres.
    pub spacing_mm: u32,
    /// Vertices in the mesh.
    pub vertices: u32,
    /// Triangles in the mesh.
    pub triangles: u32,
    /// Bounding sphere in meters, `[x, y, z, radius]`.
    pub bounds: [f32; 4],
    /// Draw name the render lane lists the terrain under.
    pub draw_name: String,
    /// Mesh identity the render lane's draw list carries.
    pub draw_mesh: u64,
}

/// One cooked session terrain: the row `world.terrain.stats` reports.
#[derive(Debug, Clone)]
struct SessionTerrain {
    rows: u32,
    cols: u32,
    spacing_mm: u32,
    vertices: u32,
    triangles: u32,
    bounds: [f32; 4],
}

/// The per-session terrain table: the set count (the revision) plus its mesh,
/// keyed by session.
type TerrainTable = std::collections::BTreeMap<u64, (u64, SessionTerrain)>;

/// The per-session terrain table: the set count (the revision) plus its mesh.
static TERRAINS: std::sync::OnceLock<std::sync::Mutex<TerrainTable>> = std::sync::OnceLock::new();

/// The table, or the poisoned-lock refusal naming the restart.
fn terrains() -> Result<std::sync::MutexGuard<'static, TerrainTable>, McpToolError> {
    TERRAINS
        .get_or_init(std::sync::Mutex::default)
        .lock()
        .map_err(|_| {
            McpToolError::new(
                "lock_poisoned",
                "terrain table is poisoned; restart the door",
            )
        })
}

/// The bounding sphere of a cooked grid, in meters: center of the integer
/// millimetre box, then the farthest vertex from it.
fn cooked_bounds(positions_mm: &[[i32; 3]]) -> [f32; 4] {
    let mut min = [i64::MAX; 3];
    let mut max = [i64::MIN; 3];
    for point in positions_mm {
        for (axis, value) in point.iter().enumerate() {
            let value = i64::from(*value);
            min[axis] = min[axis].min(value);
            max[axis] = max[axis].max(value);
        }
    }
    if positions_mm.is_empty() {
        return [0.0; 4];
    }
    let center_mm = [
        (min[0] + max[0]) / 2,
        (min[1] + max[1]) / 2,
        (min[2] + max[2]) / 2,
    ];
    let mut radius_sq = 0i64;
    for point in positions_mm {
        let offsets = [
            i64::from(point[0]) - center_mm[0],
            i64::from(point[1]) - center_mm[1],
            i64::from(point[2]) - center_mm[2],
        ];
        let distance_sq =
            offsets[0] * offsets[0] + offsets[1] * offsets[1] + offsets[2] * offsets[2];
        radius_sq = radius_sq.max(distance_sq);
    }
    let radius_mm = u64::try_from(radius_sq)
        .unwrap_or(u64::MAX)
        .isqrt()
        .min(1_800_000_000);
    let radius_mm = i32::try_from(radius_mm).unwrap_or(i32::MAX);
    [
        meters_of_mm(center_mm[0]),
        meters_of_mm(center_mm[1]),
        meters_of_mm(center_mm[2]),
        meters_of_mm(i64::from(radius_mm)),
    ]
}

/// One integer millimetre count as meters; a count past the `i32` range clamps,
/// because a grid cap keeps every coordinate far below it.
fn meters_of_mm(mm: i64) -> f32 {
    let whole = i32::try_from(mm).unwrap_or(if mm < 0 { i32::MIN } else { i32::MAX });
    engine_core::checked::f32_from_f64(f64::from(whole) / 1000.0)
}

/// `world.terrain.set`: cook the declared grid and keep it for this session.
fn terrain_set_verb(
    server: &McpServer,
    args: &TerrainSetArgs,
) -> Result<TerrainSetOutput, McpToolError> {
    if args.rows > MAX_TERRAIN_SIDE || args.cols > MAX_TERRAIN_SIDE {
        return Err(McpToolError::new(
            "bad_height_grid",
            format!(
                "{} x {} is past the {MAX_TERRAIN_SIDE} rows and columns a grid may declare; \
                 recovery: split the grid",
                args.rows, args.cols
            ),
        ));
    }
    let grid = engine_assets::collider_cook::HeightGrid {
        rows: args.rows,
        cols: args.cols,
        spacing_mm: args.spacing_mm,
        heights_mm: args.heights_mm.clone(),
    };
    let cooked = engine_assets::collider_cook::cook_height_grid(&grid).map_err(|why| {
        McpToolError::new(
            "bad_height_grid",
            format!(
                "{why}; recovery: declare rows, columns and exactly rows * cols heights, each \
                 within +-{MAX_TERRAIN_HEIGHT_MM} mm and spaced 1..={MAX_TERRAIN_SPACING_MM} mm"
            ),
        )
    })?;
    let vertices = u32::try_from(cooked.mesh.positions_mm.len()).unwrap_or(u32::MAX);
    let triangles = u32::try_from(cooked.mesh.indices.len() / 3).unwrap_or(u32::MAX);
    let row = SessionTerrain {
        rows: args.rows,
        cols: args.cols,
        spacing_mm: args.spacing_mm,
        vertices,
        triangles,
        bounds: cooked_bounds(&cooked.mesh.positions_mm),
    };
    server.dispatch(TOOL_WORLD_TERRAIN_SET, |state| {
        let mut table = terrains()?;
        let entry = table
            .entry(state.session())
            .or_insert_with(|| (0, row.clone()));
        entry.0 += 1;
        entry.1 = row;
        Ok(TerrainSetOutput {
            revision: entry.0,
            vertices,
            triangles,
        })
    })
}

/// `world.terrain.stats`: report this session's terrain in the render lane's terms.
fn terrain_stats_verb(
    server: &McpServer,
    _args: &TerrainStatsArgs,
) -> Result<TerrainStatsOutput, McpToolError> {
    server.dispatch(TOOL_WORLD_TERRAIN_STATS, |state| {
        let table = terrains()?;
        let Some((revision, row)) = table.get(&state.session()) else {
            return Err(McpToolError::new(
                "no_terrain",
                "no terrain is set for this session; recovery: call world.terrain.set with a \
                 height grid first",
            ));
        };
        Ok(TerrainStatsOutput {
            revision: *revision,
            rows: row.rows,
            cols: row.cols,
            spacing_mm: row.spacing_mm,
            vertices: row.vertices,
            triangles: row.triangles,
            bounds: row.bounds,
            draw_name: TERRAIN_DRAW_NAME.to_owned(),
            draw_mesh: TERRAIN_MESH_ID,
        })
    })
}

crate::e2_tool!(
    /// `world.terrain.set`: cook a declared height grid for this session.
    TerrainSetTool,
    name = TOOL_WORLD_TERRAIN_SET,
    description = "Cook a declared height grid into this session's terrain mesh and answer the session revision, vertices and triangles. `heights_mm` is row-major integer millimetres, exactly rows * cols entries; rows and cols are 2..=1024 and `spacing_mm` is 1..=1000000. The mesh is deterministic: the same grid always cooks the same bytes.",
    readonly = false,
    args = TerrainSetArgs,
    output = TerrainSetOutput,
    invoke = terrain_set_verb
);

crate::e2_tool!(
    /// `world.terrain.stats`: report this session's cooked terrain.
    TerrainStatsTool,
    name = TOOL_WORLD_TERRAIN_STATS,
    description = "Report this session's cooked terrain grid, vertex and triangle counts, bounding sphere in meters, and the draw name and mesh identity the render lane draws it under. Refuses before a world.terrain.set. Read-only.",
    readonly = true,
    args = TerrainStatsArgs,
    output = TerrainStatsOutput,
    invoke = terrain_stats_verb
);

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;
    use std::path::PathBuf;

    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    /// A fresh scratch root per test name: no fixed path, no clock.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("e2040-world-pack-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn bake_reports_stats_and_two_calls_agree() {
        let root = scratch("bake");
        let args = BakeArgs {
            seed: 2040,
            size: 32,
            out: None,
        };
        let first = run_bake(&root, &args).expect("bakes");
        let second = run_bake(&root, &args).expect("bakes twice");
        assert_eq!(first, second, "the same seed answers the same row");
        assert_eq!(first.digest.len(), 16);
        assert_eq!(first.size, 32);
        assert!(first.height_min <= first.height_max);
        let coverage: u32 = first.splat_coverage.iter().sum();
        assert!(coverage <= 1000, "coverage is permille: {coverage}");
        assert_eq!(first.out, None);
        assert!(!root.exists(), "no out path writes nothing");
        // A named out path writes the exact bytes it reports.
        let out = "terrain/nested/map.tver";
        let written = run_bake(
            &root,
            &BakeArgs {
                out: Some(out.to_owned()),
                ..args
            },
        )
        .expect("bakes and writes");
        assert_eq!(written.digest, first.digest);
        assert_eq!(written.out.as_deref(), Some(out));
        let bytes = std::fs::read(root.join(out)).expect("file lands");
        assert_eq!(
            u64::try_from(bytes.len()).expect("length fits"),
            written.bytes
        );
        assert_eq!(parse_terrain(&bytes).expect("file parses").size, 32);
        // A side the crate refuses refuses before any work.
        let bad = run_bake(
            &root,
            &BakeArgs {
                seed: 1,
                size: 0,
                out: None,
            },
        )
        .expect_err("size 0 refuses");
        assert_eq!(bad.code, "bad_args");
        assert!(bad.message.contains("1..=256"), "{}", bad.message);
    }

    #[test]
    fn sample_answers_points_in_order_and_refuses_bad_points() {
        let root = scratch("sample");
        let sample = run_sample(
            &root,
            &SampleArgs {
                seed: Some(2040),
                size: Some(16),
                path: None,
                points: vec![[0, 0], [15, 15], [3, 7]],
            },
        )
        .expect("samples");
        assert_eq!(sample.size, 16);
        let baked = bake_terrain(2040, 16).expect("bakes");
        let terrain = parse_terrain(&baked).expect("parses");
        assert_eq!(sample.digest, digest_of(&baked));
        let want = [(0usize, 0usize), (15, 15), (3, 7)];
        for (row, (x, y)) in sample.points.iter().zip(want) {
            let i = y * 16 + x;
            assert_eq!(row.at, [x as u16, y as u16]);
            assert_eq!(row.height, terrain.heights[i]);
            assert_eq!(row.hole, terrain.holes[i]);
            assert_eq!(row.splat, terrain.splat[i]);
        }
        // A written file samples to the same rows.
        run_bake(
            &root,
            &BakeArgs {
                seed: 2040,
                size: 16,
                out: Some("map.tver".to_owned()),
            },
        )
        .expect("writes");
        let from_file = run_sample(
            &root,
            &SampleArgs {
                seed: None,
                size: None,
                path: Some("map.tver".to_owned()),
                points: vec![[0, 0], [15, 15], [3, 7]],
            },
        )
        .expect("samples the file");
        assert_eq!(from_file, sample);
        // A point off the map names the legal range and the recovery.
        let off = run_sample(
            &root,
            &SampleArgs {
                seed: Some(2040),
                size: Some(16),
                path: None,
                points: vec![[16, 0]],
            },
        )
        .expect_err("off-map refuses");
        assert_eq!(off.code, "bad_args");
        assert!(off.message.contains("0..=15"), "{}", off.message);
        assert!(off.message.contains("re-run"), "{}", off.message);
        // Empty and over-long point lists refuse.
        for points in [Vec::new(), vec![[0u16, 0u16]; MAX_POINTS + 1]] {
            let refused = run_sample(
                &root,
                &SampleArgs {
                    seed: Some(1),
                    size: Some(4),
                    path: None,
                    points,
                },
            )
            .expect_err("point count refuses");
            assert_eq!(refused.code, "bad_args");
            assert!(refused.message.contains("1..=4096"), "{}", refused.message);
        }
        // Both sources, and neither, refuse with the one recovery.
        for (seed, path) in [(Some(1u64), Some("map.tver".to_owned())), (None, None)] {
            let refused = run_sample(
                &root,
                &SampleArgs {
                    seed,
                    size: None,
                    path,
                    points: vec![[0, 0]],
                },
            )
            .expect_err("source refuses");
            assert_eq!(refused.code, "bad_args");
            assert!(
                refused.message.contains("never both"),
                "{}",
                refused.message
            );
        }
    }

    #[test]
    fn paint_applies_edits_twice_equal_and_refuses_zero_edits() {
        let root = scratch("paint");
        let args = PaintArgs {
            seed: 2040,
            size: 16,
            edits: vec![
                EditArg {
                    rect: [0, 0, 8, 8],
                    height_delta: 30,
                    hole: Some(true),
                    splat: Some([0, 255, 0]),
                },
                EditArg {
                    rect: [4, 4, 8, 8],
                    height_delta: -10,
                    hole: None,
                    splat: None,
                },
            ],
            out: None,
        };
        let first = run_paint(&root, &args).expect("paints");
        let second = run_paint(&root, &args).expect("paints twice");
        assert_eq!(first, second, "same seed and edits give the same row");
        assert_eq!(first.edits, 2);
        assert!(first.changed_cells > 0);
        assert!(!root.exists(), "no out path writes nothing");
        // The written file is byte-equal across calls.
        let with_out = PaintArgs {
            out: Some("painted/map.tver".to_owned()),
            ..args.clone()
        };
        run_paint(&root, &with_out).expect("paints and writes");
        let once = std::fs::read(root.join("painted/map.tver")).expect("file lands");
        run_paint(&root, &with_out).expect("paints and writes again");
        let twice = std::fs::read(root.join("painted/map.tver")).expect("file lands");
        assert_eq!(once, twice, "same edits write the same bytes");
        let painted = parse_terrain(&once).expect("painted parses");
        assert!(painted.holes[0], "the rect set a hole");
        assert_eq!(painted.splat[0], [0, 255, 0]);
        // Zero edits refuse, and so does an off-map rect.
        let empty = run_paint(
            &root,
            &PaintArgs {
                edits: Vec::new(),
                ..args.clone()
            },
        )
        .expect_err("zero edits refuse");
        assert_eq!(empty.code, "bad_args");
        assert!(empty.message.contains("1..=4096"), "{}", empty.message);
        let off = run_paint(
            &root,
            &PaintArgs {
                edits: vec![EditArg {
                    rect: [15, 15, 2, 2],
                    height_delta: 1,
                    hole: None,
                    splat: None,
                }],
                ..args.clone()
            },
        )
        .expect_err("off-map rect refuses");
        assert_eq!(off.code, "bad_args");
        assert!(
            off.message.contains("outside the 16 x 16 map"),
            "{}",
            off.message
        );
        assert!(off.message.contains("sum to 255"), "{}", off.message);
        // A path that leaves the project root refuses before a write.
        let escaped = run_paint(
            &root,
            &PaintArgs {
                out: Some("../escape.tver".to_owned()),
                ..args
            },
        )
        .expect_err("escaping path refuses");
        assert_eq!(escaped.code, "bad_args");
        assert!(!root.parent().expect("parent").join("escape.tver").exists());
    }

    #[test]
    fn scatter_is_deterministic_and_refuses_a_radius_that_cannot_fit() {
        let root = scratch("scatter");
        let args = ScatterArgs {
            seed: 2040,
            size: 64,
            radius_cm: 8,
            count: Some(32),
            terrain: Some(TerrainRef {
                seed: Some(2040),
                size: Some(16),
                path: None,
            }),
            out: None,
        };
        let first = run_scatter(&root, &args).expect("scatters");
        let second = run_scatter(&root, &args).expect("scatters twice");
        assert_eq!(first, second, "the same seed answers the same points");
        assert_eq!(first.point_count, 32);
        assert_eq!(first.points.len(), 32);
        let mut sorted = first.points.clone();
        sorted.sort_by_key(|point| (point.at[0], point.at[1]));
        assert_eq!(sorted, first.points, "points arrive sorted by (x, y)");
        for point in &first.points {
            assert!(point.at[0] < 64 && point.at[1] < 64, "{:?}", point.at);
            assert!(
                point.height.is_some() && point.hole.is_some(),
                "the named terrain answers height and hole"
            );
        }
        let terrain = parse_terrain(&bake_terrain(2040, 16).expect("bakes")).expect("parses");
        let first_point = &first.points[0];
        let i =
            usize::from(first_point.at[1].min(15)) * 16 + usize::from(first_point.at[0].min(15));
        assert_eq!(first_point.height, Some(terrain.heights[i]));
        assert!(!root.exists(), "no out path writes nothing");
        // The placement JSON is byte-equal twice and carries the digest.
        let with_out = ScatterArgs {
            out: Some("scatter/points.json".to_owned()),
            ..args.clone()
        };
        run_scatter(&root, &with_out).expect("writes the placement");
        let once = std::fs::read(root.join("scatter/points.json")).expect("file lands");
        run_scatter(&root, &with_out).expect("writes the placement again");
        let twice = std::fs::read(root.join("scatter/points.json")).expect("file lands");
        assert_eq!(once, twice, "same points write the same bytes");
        let text = String::from_utf8(once).expect("json is text");
        assert!(text.contains(&first.digest), "{text}");
        // A radius that cannot fit twice across the square refuses.
        let refused = run_scatter(
            &root,
            &ScatterArgs {
                radius_cm: 40,
                ..args.clone()
            },
        )
        .expect_err("radius refuses");
        assert_eq!(refused.code, "bad_args");
        assert!(refused.message.contains("1..=32"), "{}", refused.message);
        assert!(refused.message.contains("re-run"), "{}", refused.message);
        // No terrain source: heights and holes stay absent.
        let bare = run_scatter(
            &root,
            &ScatterArgs {
                terrain: None,
                ..args
            },
        )
        .expect("scatters without a terrain");
        assert!(bare
            .points
            .iter()
            .all(|point| point.height.is_none() && point.hole.is_none()));
        assert_eq!(bare.digest, first.digest, "the terrain never moves a cell");
    }

    #[test]
    fn wfc_solves_twice_equal_and_refuses_a_bad_mask() {
        let root = scratch("wfc");
        let every = all_kinds(3);
        let args = WfcArgs {
            seed: 2040,
            width: 16,
            height: 16,
            kinds: 3,
            masks: vec![[every; DIRECTIONS]; 3],
            passable: None,
            out: None,
        };
        let first = run_wfc(&root, &args).expect("solves");
        let second = run_wfc(&root, &args).expect("solves twice");
        assert_eq!(first, second, "the same seed answers the same cells");
        assert_eq!(first.retries, 0, "a generous table solves its first seed");
        assert_eq!(first.seed, 2040);
        assert!(
            first.connected,
            "every kind is passable, so the map is one region"
        );
        assert_eq!(first.cells.len(), 256);
        assert!(first
            .cells
            .chars()
            .all(|cell| cell.to_digit(16).is_some_and(|kind| kind < 3)));
        assert!(!root.exists(), "no out path writes nothing");
        // The compact map is byte-equal twice and carries the cells.
        let with_out = WfcArgs {
            out: Some("wfc/map.json".to_owned()),
            ..args.clone()
        };
        run_wfc(&root, &with_out).expect("writes the map");
        let once = std::fs::read(root.join("wfc/map.json")).expect("file lands");
        run_wfc(&root, &with_out).expect("writes the map again");
        let twice = std::fs::read(root.join("wfc/map.json")).expect("file lands");
        assert_eq!(once, twice, "same cells write the same bytes");
        let text = String::from_utf8(once).expect("json is text");
        assert!(
            text.contains(&first.cells) && text.contains(&first.digest),
            "{text}"
        );
        // A mask bit above `kinds` refuses with the table's legal range.
        let bad = WfcArgs {
            masks: vec![[every | 0b1000; DIRECTIONS]; 3],
            ..args.clone()
        };
        let refused = run_wfc(&root, &bad).expect_err("mask refuses");
        assert_eq!(refused.code, "bad_args");
        assert!(refused.message.contains("re-run"), "{}", refused.message);
        // A passable mask outside the table refuses too.
        let bad_mask = run_wfc(
            &root,
            &WfcArgs {
                passable: Some(0b1000),
                ..args
            },
        )
        .expect_err("passable refuses");
        assert_eq!(bad_mask.code, "bad_args");
        assert!(bad_mask.message.contains("0..=2"), "{}", bad_mask.message);
    }

    #[test]
    fn wfc_refuses_a_disconnected_solve_naming_the_retries() {
        let root = scratch("wfc-disconnected");
        // A checkerboard table: one kind never touches itself, so a one-kind
        // passable mask splits the map into isolated cells.
        let masks = vec![[0b10; DIRECTIONS], [0b01; DIRECTIONS]];
        let refused = run_wfc(
            &root,
            &WfcArgs {
                seed: 2040,
                width: 8,
                height: 8,
                kinds: 2,
                masks,
                passable: Some(0b01),
                out: None,
            },
        )
        .expect_err("disconnected refuses");
        assert_eq!(refused.code, "wfc_disconnected");
        assert!(refused.message.contains("retries"), "{}", refused.message);
        assert!(refused.message.contains("re-run"), "{}", refused.message);
        assert!(!root.exists(), "a refusal writes nothing");
    }

    /// The maze rule set `content/maps/rewrite/` ships, read at compile time.
    const MAZE_RULES: &str = include_str!("../../../../content/maps/rewrite/maze_streets.rules");
    /// The UW12/UW20 insula floor plan, authored as a rules file.
    const INSULA_RULES: &str = include_str!("../../../../content/maps/rewrite/insula_block.rules");

    /// A rewrite call with the given rules text, seed 2040 and no path.
    fn rewrite_of(rules: &str) -> RewriteArgs {
        RewriteArgs {
            rules: rules.to_owned(),
            seed: 2040,
            out: None,
        }
    }

    #[test]
    fn rewrite_bakes_the_maze_to_its_pinned_hash_twice() {
        let root = scratch("rewrite");
        let args = rewrite_of(MAZE_RULES);
        let first = run_rewrite(&root, &args).expect("bakes the maze");
        let second = run_rewrite(&root, &args).expect("bakes the maze twice");
        assert_eq!(
            first, second,
            "the same rules and seed answer the same grid"
        );
        assert_eq!(first.hash, "0xf51cb0aeb99ba512", "the hash MJ1 pinned");
        assert_eq!((first.width, first.height), (31, 31));
        assert_eq!(first.symbols, "BWGR");
        assert!(first.steps > 0, "the maze takes steps");
        let rows: Vec<&str> = first.grid.lines().collect();
        assert_eq!(rows.len(), 31, "one line per row");
        assert!(rows.iter().all(|row| row.len() == 31), "{rows:?}");
        assert!(!root.exists(), "no out path writes nothing");
        // The door bakes the same grid the engine crate runs.
        let direct = Program::parse(MAZE_RULES)
            .expect("parses")
            .run(2040)
            .expect("settles");
        assert_eq!(first.grid, direct.render());
        // The tool handle answers the same row.
        let served = RewriteTool::invoke(&server(), args).expect("serves through its tool");
        assert_eq!(served.hash, first.hash);
    }

    #[test]
    fn rewrite_refuses_a_bad_symbol_naming_its_line() {
        let root = scratch("rewrite-bad");
        // Line 4 writes `Q`, which the alphabet on line 2 does not name.
        let rules = "size 8 8\nalphabet BW\nrect 0 0 8 8 B\nput 1 1 Q\n";
        let refused = run_rewrite(&root, &rewrite_of(rules)).expect_err("a bad symbol refuses");
        assert_eq!(refused.code, "bad_args");
        assert!(
            refused.message.contains("rules line 4"),
            "{}",
            refused.message
        );
        assert!(refused.message.contains("`Q`"), "{}", refused.message);
        assert!(refused.message.contains("re-run"), "{}", refused.message);
        assert!(!root.exists(), "a refusal writes nothing");
    }

    #[test]
    fn rewrite_refuses_a_grid_over_128_a_side_before_running() {
        let root = scratch("rewrite-side");
        // These rules never settle: a refusal that names the size proves no run started.
        let rules = "size 129 16\nalphabet BW\nput 0 0 B\nmarkov {\n  one { rule B = W }\n  one { rule W = B }\n}\n";
        let refused = run_rewrite(&root, &rewrite_of(rules)).expect_err("129 cells refuse");
        assert_eq!(refused.code, "bad_args");
        assert!(
            refused.message.contains("rules line 1"),
            "{}",
            refused.message
        );
        assert!(refused.message.contains("1..=128"), "{}", refused.message);
        // 128 a side is the largest grid the door takes.
        let edge = run_rewrite(
            &root,
            &rewrite_of("size 128 128\nalphabet BW\nrect 0 0 128 128 W\n"),
        )
        .expect("128 a side bakes");
        assert_eq!((edge.width, edge.height, edge.steps), (128, 128, 1));
    }

    #[test]
    fn rewrite_refuses_a_run_that_never_settles() {
        let root = scratch("rewrite-loop");
        // One cell flips between two symbols forever: no partial grid comes back.
        let rules = "size 4 4\nalphabet BW\nput 0 0 B\nmarkov {\n  one { rule B = W }\n  one { rule W = B }\n}\n";
        let refused = run_rewrite(&root, &rewrite_of(rules)).expect_err("a loop refuses");
        assert_eq!(refused.code, "bad_args");
        assert!(
            refused.message.contains("without settling"),
            "{}",
            refused.message
        );
        assert!(refused.message.contains("re-run"), "{}", refused.message);
    }

    #[test]
    fn rewrite_writes_the_grid_text_it_answers() {
        let root = scratch("rewrite-out");
        let args = RewriteArgs {
            out: Some("rewrite/insula.txt".to_owned()),
            ..rewrite_of(INSULA_RULES)
        };
        let written = run_rewrite(&root, &args).expect("bakes and writes");
        assert_eq!(written.out.as_deref(), Some("rewrite/insula.txt"));
        let bytes = std::fs::read(root.join("rewrite/insula.txt")).expect("file lands");
        assert_eq!(
            bytes,
            written.grid.as_bytes(),
            "the file holds the grid text"
        );
        // A path that leaves the project root refuses before a write.
        let escaped = run_rewrite(
            &root,
            &RewriteArgs {
                out: Some("../escape.txt".to_owned()),
                ..args
            },
        )
        .expect_err("escaping path refuses");
        assert_eq!(escaped.code, "bad_args");
    }

    #[test]
    fn rewrite_bakes_the_insula_floor_plan_from_its_rules_file() {
        let root = scratch("rewrite-insula");
        let args = rewrite_of(INSULA_RULES);
        let plan = run_rewrite(&root, &args).expect("bakes the insula");
        let again = run_rewrite(&root, &args).expect("bakes it twice");
        assert_eq!(plan, again, "the same rules and seed give the same plan");
        assert_eq!((plan.width, plan.height), (24, 18));
        assert_eq!(plan.symbols, "SWRCD");
        // The courtyard is 8 x 6 and no rule reaches it; the street margin stays street.
        assert_eq!(plan.grid.matches('C').count(), 8 * 6);
        let street = "S".repeat(24);
        assert_eq!(plan.grid.lines().next(), Some(street.as_str()));
        // The door bakes the file cell for cell, as the engine crate reads it.
        let direct = Program::parse(INSULA_RULES)
            .expect("parses")
            .run(2040)
            .expect("settles");
        assert_eq!(plan.grid, direct.render());
    }

    #[test]
    fn pack_registers_its_eight_verbs() {
        let pack = pack().expect("registers");
        assert_eq!(pack.name(), PACK);
        assert_eq!(
            pack.verb_names(),
            vec![
                TOOL_WORLD_TERRAIN_BAKE,
                TOOL_WORLD_TERRAIN_SAMPLE,
                TOOL_WORLD_TERRAIN_PAINT,
                TOOL_WORLD_TERRAIN_SET,
                TOOL_WORLD_TERRAIN_STATS,
                TOOL_WORLD_SCATTER,
                TOOL_WORLD_WFC,
                TOOL_WORLD_REWRITE
            ]
        );
        assert!(!pack.default_loaded());
        for verb in pack.verbs() {
            assert_ne!(verb.card(), CapabilityCard::Deny);
            assert!(
                verb.model().annotations.is_some(),
                "{} is annotated",
                verb.name()
            );
        }
        assert_eq!(
            TerrainSampleTool::invoke(
                &server(),
                SampleArgs {
                    seed: Some(7),
                    size: Some(8),
                    path: None,
                    points: vec![[1, 2]],
                },
            )
            .expect("sample serves through its tool")
            .points
            .len(),
            1
        );
    }

    /// A 64 x 64 grid whose heights walk 0..=499 mm: the row the accept names.
    fn sixty_four_grid() -> TerrainSetArgs {
        let rows = 64u32;
        let cols = 64u32;
        let heights_mm: Vec<i32> = (0..rows.saturating_mul(cols))
            .map(|index| i32::try_from(index % 500).unwrap_or(0))
            .collect();
        TerrainSetArgs {
            rows,
            cols,
            spacing_mm: 250,
            heights_mm,
        }
    }

    #[test]
    fn terrain_set_cooks_a_sixty_four_grid_and_stats_reports_its_mesh() {
        let server = McpServer::new(241, Capability::unrestricted(241));
        let set = TerrainSetTool::invoke(&server, sixty_four_grid()).expect("the grid cooks");
        assert_eq!(set.revision, 1, "the first set of this session");
        assert_eq!(set.vertices, 64 * 64);
        assert_eq!(set.triangles, 2 * 63 * 63, "two triangles per quad");
        let stats = TerrainStatsTool::invoke(&server, TerrainStatsArgs::default())
            .expect("stats read back");
        assert_eq!(stats.revision, 1);
        assert_eq!((stats.rows, stats.cols, stats.spacing_mm), (64, 64, 250));
        assert_eq!(stats.triangles, 2 * 63 * 63);
        assert_eq!(stats.vertices, 64 * 64);
        // The render lane's terms: the name and mesh identity its draw list uses.
        assert_eq!(stats.draw_name, TERRAIN_DRAW_NAME);
        assert_eq!(stats.draw_mesh, TERRAIN_MESH_ID);
        // 63 * 250 mm across, half a meter up: the sphere reaches the far corner.
        let [x, y, z, radius] = stats.bounds;
        assert!((7.0..=8.0).contains(&x), "center x {x}");
        assert!((0.0..=0.5).contains(&y), "center y {y}");
        assert!((7.0..=8.0).contains(&z), "center z {z}");
        assert!(radius >= 8.0, "the sphere reaches the far corner: {radius}");
        // A second set is the next revision, same mesh.
        let again = TerrainSetTool::invoke(&server, sixty_four_grid()).expect("sets again");
        assert_eq!(again.revision, 2);
        assert_eq!(again.triangles, 2 * 63 * 63);
    }

    #[test]
    fn terrain_stats_refuses_before_a_set_and_a_bad_grid_is_named() {
        let server = McpServer::new(242, Capability::unrestricted(242));
        let refused = TerrainStatsTool::invoke(&server, TerrainStatsArgs::default())
            .expect_err("nothing is set yet");
        assert_eq!(refused.code, "no_terrain");
        assert!(refused.message.contains("world.terrain.set"));
        let short = TerrainSetTool::invoke(
            &server,
            TerrainSetArgs {
                rows: 3,
                cols: 3,
                spacing_mm: 500,
                heights_mm: vec![0; 8],
            },
        )
        .expect_err("eight heights for nine samples");
        assert_eq!(short.code, "bad_height_grid");
        assert!(short.message.contains("rows * cols"), "{}", short.message);
        let thin = TerrainSetTool::invoke(
            &server,
            TerrainSetArgs {
                rows: 1,
                cols: 3,
                spacing_mm: 500,
                heights_mm: vec![0; 3],
            },
        )
        .expect_err("one row has no quad");
        assert_eq!(thin.code, "bad_height_grid");
        let wide = TerrainSetTool::invoke(
            &server,
            TerrainSetArgs {
                rows: MAX_TERRAIN_SIDE + 1,
                cols: 2,
                spacing_mm: 500,
                heights_mm: vec![0; 2],
            },
        )
        .expect_err("past the side cap");
        assert_eq!(wide.code, "bad_height_grid");
        assert!(wide.message.contains("1024"), "{}", wide.message);
        // A refused set leaves the session exactly as it was.
        let still = TerrainStatsTool::invoke(&server, TerrainStatsArgs::default())
            .expect_err("no terrain yet");
        assert_eq!(still.code, "no_terrain");
    }
}

// WG2: this pack publishes its own row; `packs/mod.rs` keeps no hand list.
#[expect(
    unsafe_code,
    reason = "linkme publishes a pack row through a link section"
)]
#[linkme::distributed_slice(super::PACK_MODULES)]
static PACK_ROW: super::PackModule = super::PackModule {
    order: 21,
    name: PACK,
    default_loaded: false,
    tools: &[
        "world.terrain.bake",
        "world.terrain.sample",
        "world.terrain.paint",
        "world.terrain.set",
        "world.terrain.stats",
        "world.scatter",
        "world.wfc",
        "world.rewrite",
    ],
    build: pack,
};
