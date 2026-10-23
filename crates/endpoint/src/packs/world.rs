//! `world` pack: the door bakes, samples and paints the TVER terrain that
//! `engine-world` owns.
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
//! Units: texels for sides, rects and points, heights in texel steps
//! `0..=255`, hundredths for the mean, permille for splat coverage. File IO
//! only at the `out`/`path` edge and only under the project root: a relative
//! path with no `..`, never a fixed path. Determinism: the same seed and the
//! same edits give the same bytes and digest. Errors: [`McpToolError`] with
//! `bad_args` (the legal range plus one recovery), `read_failed` or
//! `write_failed` at the file edge.

use std::path::{Component, Path};

use engine_world::terrain::{
    bake_terrain, paint_terrain, parse_terrain, terrain_hash, terrain_stats, write_terrain, Edit,
    Terrain,
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
                "could not write terrain `{rel}` under the project root ({why}); re-run with a \
                 writable relative path such as target/terrain/tver.bin"
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

/// Resolve the map a sample reads: a seed bake or a written TVER file.
fn sampled_terrain(root: &Path, args: &SampleArgs) -> Result<(Vec<u8>, Terrain), McpToolError> {
    match (args.seed, args.path.as_deref()) {
        (Some(seed), None) => {
            let size = args.size.unwrap_or(DEFAULT_SIZE);
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

/// The world pack: terrain bake, sample and paint, loaded on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "terrain: world.terrain.bake bakes a seeded TVER heightmap, hole mask and 3-way splat \
         and reports its stats; world.terrain.sample answers height, hole and splat at up to \
         4096 points; world.terrain.paint applies rect height/hole/splat edits and reports what \
         moved. Both writers write only an `out` path under the project root",
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
    Ok(pack)
}

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
    fn pack_registers_its_three_verbs() {
        let pack = pack().expect("registers");
        assert_eq!(pack.name(), PACK);
        assert_eq!(
            pack.verb_names(),
            vec![
                TOOL_WORLD_TERRAIN_BAKE,
                TOOL_WORLD_TERRAIN_SAMPLE,
                TOOL_WORLD_TERRAIN_PAINT
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
}
