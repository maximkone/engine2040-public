//! `level_paint` pack: `level.paint` lays a kit across a room in one call.
//!
//! The gesture comes from Godot's GridMap paint and LDtk auto-layer rules: a
//! caller names a palette kit and the cells it wants, and one gated call turns
//! that into one batch of `Op::Insert` inside one undo step. Before this verb,
//! 24 cells were 24 preview-and-apply pairs; here they are one.
//!
//! Cells come from three sources, all read in one call: `cells` (explicit
//! poses), `region` (a `w` by `h` block of grid steps) and, over that region,
//! `rules` — an authored adjacency table the shared wave-function-collapse
//! solver (`engine_world::wfc`) collapses instead of the caller clicking.
//! `scatter` adds seeded Poisson-disk points of a second kit over the region,
//! or over the level square when no region is given, so points alone can be
//! painted without filling cells first. A call with no source writes nothing
//! and refuses.
//!
//! Units: meters for positions and grid steps, radians for yaw, cells for
//! counts. The palette is `crate::build_level::KINDS`, so a painted kit is the
//! same named body `level.build` places and lands with the same authored scale.
//! Determinism: ids derive from `(kit, seed, cell)` bit patterns, never a
//! random mint, so the same call twice gives the same ids and the same document
//! digest; `seed` is a required field, so a solve or a scatter can never
//! arrive unseeded.
//! Errors: `McpToolError` with `no_kit` (names the palette), `off_grid` and
//! `rule_unsatisfiable` (name the cell), `bad_cell` (a pose that is not finite,
//! or one submitted twice), `bad_grid`, `too_many_cells` (names the count),
//! `stale_revision` and `bad_args`; the command layer adds `invalid_batch`,
//! `duplicate_key`, `apply_in_flight` and `spatial_rejected` when two staged
//! bodies intersect, naming the pair. A refusal writes nothing.

use engine_editor::capability::{self, approve};
use engine_editor::commands::{self, Batch, Op, ProvenanceStamp};
use engine_scene::ids::ObjectId;
use engine_scene::transform::Transform;
use engine_world::scatter::poisson_disk;
use engine_world::wfc::{bake_constrained, Adjacency, Constraints, Rect, WfcError, MAX_KINDS};

use super::{bad_args, PackDef, PackError, VerbSpec};
use crate::build_level::{kind_words, KINDS};
use crate::mcp::{batch_error, cap_error, granted_ops, McpServer, McpToolError, ServerState};
use crate::play::hex_bytes;
use crate::spatial_glue::quat_of_yaw;
use crate::tool_capabilities::CapabilityCard;
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

/// The verb name.
pub const TOOL_LEVEL_PAINT: &str = "level.paint";
/// The pack name.
pub const PACK: &str = "level_paint";
/// Most cells one call may insert: the transaction batch cap, so one paint is
/// always one undo step and never a partial write.
pub const MAX_PAINT_CELLS: usize = 256;
/// Default grid step in meters, the width of the `wall` kit.
pub const DEFAULT_STEP_M: f32 = 2.0;
/// Default level half-extent in meters: the golden arena edge.
pub const DEFAULT_HALF_EXTENT_M: f32 = 20.0;

/// One painted cell: a pose on the grounded plane.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CellDto {
    /// Cell centre x, meters.
    pub x: f32,
    /// Cell centre z, meters.
    pub z: f32,
    /// Facing yaw, radians.
    pub yaw: f32,
}

/// A block of grid cells: the computed case, `w` by `h` steps.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RegionDto {
    /// First column, in grid steps from the level origin.
    pub cx: i16,
    /// First row, in grid steps from the level origin.
    pub cz: i16,
    /// Columns.
    pub w: u16,
    /// Rows.
    pub h: u16,
    /// Grid step in meters; zero takes [`DEFAULT_STEP_M`].
    pub step_m: f32,
}

impl RegionDto {
    /// The step this region paints on, with the default filled in.
    fn step(&self) -> f32 {
        if self.step_m == 0.0 {
            DEFAULT_STEP_M
        } else {
            self.step_m
        }
    }

    /// Centre of one grid cell, meters.
    fn centre(&self, col: u16, row: u16) -> (f32, f32) {
        let step = self.step();
        (
            f32::from(self.cx) + f32::from(col) * step,
            f32::from(self.cz) + f32::from(row) * step,
        )
    }
}

/// One authored pin: this cell must hold this kind.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PinDto {
    /// Cell column, grid steps.
    pub x: u16,
    /// Cell row, grid steps.
    pub z: u16,
    /// Kind index into [`RulesDto::kinds`].
    pub kind: u8,
}

/// One authored forbidden block: this rect may never hold this kind.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BlockDto {
    /// First column, included.
    pub x0: u16,
    /// First row, included.
    pub z0: u16,
    /// Last column, included.
    pub x1: u16,
    /// Last row, included.
    pub z1: u16,
    /// Kind index into [`RulesDto::kinds`].
    pub kind: u8,
}

/// One connect rule: these two cells must share a path of `open` kinds.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LinkDto {
    /// Path start column.
    pub from_x: u16,
    /// Path start row.
    pub from_z: u16,
    /// Path end column.
    pub to_x: u16,
    /// Path end row.
    pub to_z: u16,
}

/// An authored adjacency table plus the rules one solve must satisfy.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RulesDto {
    /// Kit name per tile kind, in kind order; every name must be in the palette.
    pub kinds: Vec<String>,
    /// One `[+x, -x, +z, -z]` neighbour-mask row per kind; bit `k` allows kind `k`.
    pub masks: Vec<[u16; 4]>,
    /// Cells that must hold one kind.
    pub pinned: Vec<PinDto>,
    /// Blocks that may never hold one kind.
    pub forbidden: Vec<BlockDto>,
    /// Kind bits a connect path may step on; zero refuses every link.
    pub open: u16,
    /// Cell pairs that must share a 4-way path of `open` kinds.
    pub connect: Vec<LinkDto>,
}

/// Seeded Poisson-disk scatter of a second kit over a region, or over the
/// level square when the call gives no region.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ScatterDto {
    /// Kit the points wear; must be in the palette.
    pub kit: String,
    /// Recipe text mixed into the seed, so two recipes spread differently.
    pub recipe: String,
    /// Minimum point spacing, meters.
    pub min_dist_m: f32,
}

/// Door args for `level.paint`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PaintArgs {
    /// Kit every explicit and filled cell wears; one of the palette names.
    pub kit: String,
    /// Seed that names the ids, the solve and the scatter.
    pub seed: u64,
    /// Must equal the document revision, else nothing runs.
    pub expected_revision: u64,
    /// Retry identity for the landed batch.
    pub idempotency_key: u64,
    /// Cells to paint, in any order; the batch sorts them.
    #[serde(default)]
    pub cells: Vec<CellDto>,
    /// Grid block to fill; with `rules` it is the solve's map.
    #[serde(default)]
    pub region: Option<RegionDto>,
    /// Adjacency rules to solve over the region instead of filling it.
    #[serde(default)]
    pub rules: Option<RulesDto>,
    /// Seeded scatter over the region, or over the level square.
    #[serde(default)]
    pub scatter: Option<ScatterDto>,
    /// Most cells this call may insert; zero takes [`MAX_PAINT_CELLS`].
    #[serde(default)]
    pub max_cells: u16,
    /// Level half-extent in meters; a cell outside it refuses `off_grid`.
    #[serde(default)]
    pub half_extent_m: f32,
}

impl PaintArgs {
    /// The cap this call honors: the caller's, never above [`MAX_PAINT_CELLS`].
    fn cap(&self) -> usize {
        if self.max_cells == 0 {
            MAX_PAINT_CELLS
        } else {
            usize::from(self.max_cells).min(MAX_PAINT_CELLS)
        }
    }

    /// The level half-extent in meters, with the default filled in.
    fn extent(&self) -> f32 {
        if self.half_extent_m == 0.0 {
            DEFAULT_HALF_EXTENT_M
        } else {
            self.half_extent_m
        }
    }
}

/// One landed cell with its deterministic id.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PaintedDto {
    /// Live document id (UUID text), derived from `(kit, seed, cell)`.
    pub id: String,
    /// Kit this cell wears.
    pub kit: String,
    /// Centre x, meters.
    pub x: f32,
    /// Feet height, meters: the grounded plane is zero.
    pub y: f32,
    /// Centre z, meters.
    pub z: f32,
    /// Facing yaw, radians.
    pub yaw: f32,
}

/// Door output for `level.paint`: what landed in one batch. Undo rides
/// `tx.undo` with `key`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PaintOutput {
    /// Revision after the call.
    pub revision: u64,
    /// Cells this batch inserted.
    pub cells: usize,
    /// Ops in the landed batch: one `Op::Insert` per cell.
    pub ops: usize,
    /// Landed cells in batch order, so `placed[0]` is the first op.
    pub placed: Vec<PaintedDto>,
    /// First landed id; empty when nothing landed.
    pub first_id: String,
    /// Last landed id, for a reader that only wants the end of the run.
    pub last_id: String,
    /// Document digest (hex) after the batch.
    pub digest: String,
    /// Effect lines the command layer reported for the batch.
    pub effects: Vec<String>,
    /// Landed batch key.
    pub key: u64,
    /// Undo identity for `tx.undo`.
    pub undo_token: u64,
    /// D19 verdict report: the checks that judged this paint.
    #[serde(default)]
    pub verify: String,
}

/// One cell the plan wants, with its palette entry resolved once.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Planned {
    /// Palette name.
    kit: &'static str,
    /// Authored scale, meters.
    scale: [f32; 3],
    /// Centre x, meters.
    x: f32,
    /// Centre z, meters.
    z: f32,
    /// Facing yaw, radians.
    yaw: f32,
}

/// Palette row of one kit name: the catalog name and its authored scale.
type PaletteEntry = (&'static str, [f32; 3]);
/// One solved region: the collapsed grid (row-major, `x` fastest, one kind
/// index per cell) and the palette row each kind maps to.
type Solved = (Vec<u8>, Vec<PaletteEntry>);

/// Palette entry of a kit name, or `None` when the catalog does not hold it.
/// The catalog carries its scale as a tuple; the door works in `[f32; 3]`.
fn palette(name: &str) -> Option<(&'static str, [f32; 3])> {
    KINDS
        .iter()
        .find(|entry| entry.0 == name)
        .map(|entry| (entry.0, [entry.1 .0, entry.1 .1, entry.1 .2]))
}

/// Refusal for a kit name the palette does not hold.
fn no_kit(name: &str) -> McpToolError {
    McpToolError::new(
        "no_kit",
        format!(
            "kit `{name}` is not in the palette; paint one of {}: \
             re-run with a listed kit",
            kind_words()
        ),
    )
}

/// Refusal for a cell the level bounds do not hold.
fn off_grid(x: f32, z: f32, extent: f32) -> McpToolError {
    McpToolError::new(
        "off_grid",
        format!(
            "cell ({x}, {z}) is outside the level at +/-{extent} m; \
             move it inside or raise half_extent_m"
        ),
    )
}

/// The door's refusal for one solver error: a rule that cannot hold names its
/// cell, so the author edits that cell instead of hunting the table.
fn grid_refusal(error: &WfcError) -> McpToolError {
    match error {
        WfcError::UnsatisfiableCell { x, z }
        | WfcError::CellOutOfBounds { x, z }
        | WfcError::Contradiction { x, z }
        | WfcError::BacktrackCap { x, z, .. } => McpToolError::new(
            "rule_unsatisfiable",
            format!(
                "the rules leave cell ({x}, {z}) no kind that fits; loosen the pin or \
                 forbid that names it, or re-run with another seed"
            ),
        ),
        WfcError::KindOutOfRange { x, z, kind } => McpToolError::new(
            "bad_grid",
            format!(
                "a rule names kind {kind} at cell ({x}, {z}), past the kinds in the \
                 table; keep every rule kind below the kind count"
            ),
        ),
        other => McpToolError::new(
            "bad_grid",
            format!(
                "the adjacency table or rules were refused ({other}); submit \
                 1..={MAX_KINDS} kinds with one [+x,-x,+z,-z] mask row each, and a map \
                 no wider than the solver side bound"
            ),
        ),
    }
}

/// A deterministic id from `(kit, seed, cell)`: two hashes of the labelled cell
/// become 32 hex chars, which `ObjectId::parse` reads as a simple UUID. Ids are
/// never minted random here, so the same paint gives the same ids twice.
fn cell_id(kit: &str, seed: u64, x: f32, z: f32, yaw: f32) -> Result<ObjectId, McpToolError> {
    let label = format!(
        "level.paint\0{kit}\0{seed}\0{:x}\0{:x}\0{:x}",
        x.to_bits(),
        z.to_bits(),
        yaw.to_bits()
    );
    let head = engine_core::hash_bytes(label.as_bytes());
    let tail = engine_core::hash_bytes(format!("{label}\0end").as_bytes());
    let text = format!("{head:016x}{tail:016x}");
    ObjectId::parse(&text).map_err(|_| {
        McpToolError::new(
            "internal",
            format!("the minted id `{text}` is not 32 hex chars; report this paint"),
        )
    })
}

/// Collapse the region from an authored table, and answer the palette name each
/// tile kind maps to. Kind names resolve against the palette before the solve,
/// so a rule can only name a kit the door actually holds.
fn solve(region: &RegionDto, seed: u64, rules: &RulesDto) -> Result<Solved, McpToolError> {
    if rules.kinds.is_empty() {
        return Err(bad_args(
            "rules need at least one kind: name the kits to collapse".to_owned(),
        ));
    }
    if rules.kinds.len() > MAX_KINDS {
        return Err(bad_args(format!(
            "rules name {} kinds, past the {} the solver holds; merge the palette",
            rules.kinds.len(),
            MAX_KINDS
        )));
    }
    if rules.masks.len() != rules.kinds.len() {
        return Err(bad_args(format!(
            "rules carry {} mask rows for {} kinds; submit one [+x,-x,+z,-z] row per kind",
            rules.masks.len(),
            rules.kinds.len()
        )));
    }
    let mut entries = Vec::with_capacity(rules.kinds.len());
    for name in &rules.kinds {
        entries.push(palette(name).ok_or_else(|| no_kit(name))?);
    }
    let kinds = u8::try_from(rules.kinds.len())
        .map_err(|_| bad_args("the kind list is past one byte; merge the palette".to_owned()))?;
    let adjacency = Adjacency::new(kinds, &rules.masks).map_err(|error| grid_refusal(&error))?;
    let constraints = Constraints {
        pinned: rules
            .pinned
            .iter()
            .map(|pin| (pin.x, pin.z, pin.kind))
            .collect(),
        forbidden: rules
            .forbidden
            .iter()
            .map(|block| {
                (
                    Rect::new(block.x0, block.z0, block.x1, block.z1),
                    block.kind,
                )
            })
            .collect(),
        connect: rules
            .connect
            .iter()
            .map(|link| ((link.from_x, link.from_z), (link.to_x, link.to_z)))
            .collect(),
        open: rules.open,
    };
    let grid = bake_constrained(region.w, region.h, &adjacency, seed, &constraints)
        .map_err(|error| grid_refusal(&error))?;
    Ok((grid, entries))
}

/// The cells the caller typed, each pose checked against the level bounds.
fn typed_cells(
    args: &PaintArgs,
    entry: PaletteEntry,
    extent: f32,
) -> Result<Vec<Planned>, McpToolError> {
    let mut out: Vec<Planned> = Vec::new();
    for cell in &args.cells {
        if !cell.x.is_finite() || !cell.z.is_finite() || !cell.yaw.is_finite() {
            return Err(McpToolError::new(
                "bad_cell",
                format!(
                    "cell ({}, {}) is not a finite pose; submit meters and radians",
                    cell.x, cell.z
                ),
            ));
        }
        if cell.x.abs() > extent || cell.z.abs() > extent {
            return Err(off_grid(cell.x, cell.z, extent));
        }
        out.push(Planned {
            kit: entry.0,
            scale: entry.1,
            x: cell.x,
            z: cell.z,
            yaw: cell.yaw,
        });
    }
    Ok(out)
}

/// The cells of a region: the called kit everywhere, or, when `rules` name a
/// table, what that table collapsed. The grid is row-major with `x` fastest, so
/// a tile index is `row * w + col`.
fn region_cells(
    args: &PaintArgs,
    entry: PaletteEntry,
    extent: f32,
) -> Result<Vec<Planned>, McpToolError> {
    let Some(region) = &args.region else {
        if args.rules.is_some() {
            return Err(bad_args(
                "rules collapse a region: submit region with w and h".to_owned(),
            ));
        }
        return Ok(Vec::new());
    };
    if region.w == 0 || region.h == 0 {
        return Err(bad_args(format!(
            "region needs w and h of at least one cell, got {} x {}",
            region.w, region.h
        )));
    }
    if region.step() <= 0.0 || !region.step().is_finite() {
        return Err(bad_args(
            "region step_m must be a finite step above zero meters".to_owned(),
        ));
    }
    let solved = match &args.rules {
        None => None,
        Some(rules) => Some(solve(region, args.seed, rules)?),
    };
    let mut out: Vec<Planned> = Vec::new();
    for row in 0..region.h {
        for col in 0..region.w {
            let (x, z) = region.centre(col, row);
            if x.abs() > extent || z.abs() > extent {
                return Err(off_grid(x, z, extent));
            }
            let (kit, scale) = match &solved {
                None => entry,
                Some((grid, entries)) => {
                    let index = usize::from(row) * usize::from(region.w) + usize::from(col);
                    entries[usize::from(grid[index])]
                }
            };
            out.push(Planned {
                kit,
                scale,
                x,
                z,
                yaw: 0.0,
            });
        }
    }
    Ok(out)
}

/// The scatter's seeded points: over the region frame when one is given, else
/// over the level square, so points alone can be painted without a fill first.
fn scatter_cells(args: &PaintArgs, extent: f32) -> Result<Vec<Planned>, McpToolError> {
    let Some(scatter) = &args.scatter else {
        return Ok(Vec::new());
    };
    let entry = palette(&scatter.kit).ok_or_else(|| no_kit(&scatter.kit))?;
    let (origin, spread) = match &args.region {
        Some(region) => {
            let step = region.step();
            (
                (f32::from(region.cx), f32::from(region.cz)),
                (f32::from(region.w) * step, f32::from(region.h) * step),
            )
        }
        None => ((-extent, -extent), (2.0 * extent, 2.0 * extent)),
    };
    let points = poisson_disk(
        args.seed,
        &scatter.recipe,
        spread.0,
        spread.1,
        scatter.min_dist_m,
    )
    .map_err(|error| {
        bad_args(format!(
            "scatter recipe `{}` at {} m spacing refused over {} x {} m: {error}; \
             lower min_dist_m",
            scatter.recipe, scatter.min_dist_m, spread.0, spread.1
        ))
    })?;
    let mut out: Vec<Planned> = Vec::new();
    for point in points {
        let x = origin.0 + point[0];
        let z = origin.1 + point[1];
        if x.abs() > extent || z.abs() > extent {
            return Err(off_grid(x, z, extent));
        }
        out.push(Planned {
            kit: entry.0,
            scale: entry.1,
            x,
            z,
            yaw: 0.0,
        });
    }
    Ok(out)
}

/// Expand one call into the cells it wants, in sorted order, or refuse.
///
/// Sources read in order: explicit `cells`, then the region (filled with the
/// called kit, or collapsed from `rules`), then `scatter` over the region.
fn plan(args: &PaintArgs) -> Result<Vec<Planned>, McpToolError> {
    let entry = palette(&args.kit).ok_or_else(|| no_kit(&args.kit))?;
    let extent = args.extent();
    let mut out = typed_cells(args, entry, extent)?;
    out.extend(region_cells(args, entry, extent)?);
    out.extend(scatter_cells(args, extent)?);
    if out.is_empty() {
        return Err(bad_args(
            "level.paint needs work: submit cells, a region, a region with rules, or a scatter"
                .to_owned(),
        ));
    }
    out.sort_by(|left, right| {
        left.x
            .total_cmp(&right.x)
            .then(left.z.total_cmp(&right.z))
            .then_with(|| left.kit.cmp(right.kit))
    });
    // Sorted, so a repeated cell is adjacent: one id derives from one cell, and
    // the command layer would refuse the batch for a duplicate node name.
    if let Some(cell) = out.windows(2).find(|pair| pair[0] == pair[1]) {
        return Err(McpToolError::new(
            "bad_cell",
            format!(
                "cell ({}, {}) with kit `{}` is submitted twice; paint each cell once, or \
                 move one of the pair apart",
                cell[0].x, cell[0].z, cell[0].kit
            ),
        ));
    }
    let cap = args.cap();
    if out.len() > cap {
        return Err(McpToolError::new(
            "too_many_cells",
            format!(
                "the call expands to {} cells, past the {} this door inserts in one undo \
                 step; paint a smaller region or raise max_cells up to {MAX_PAINT_CELLS}",
                out.len(),
                cap
            ),
        ));
    }
    Ok(out)
}

/// Land the planned cells as one gated batch and answer what landed.
fn paint_on_state(state: &mut ServerState, args: &PaintArgs) -> Result<PaintOutput, McpToolError> {
    if args.expected_revision != state.doc.revision {
        return Err(McpToolError::new(
            "stale_revision",
            format!(
                "document is at revision {}, intent expects {}; re-read scene.brief, \
                 then paint again",
                state.doc.revision, args.expected_revision
            ),
        ));
    }
    let planned = plan(args)?;
    let mut ops = Vec::with_capacity(planned.len());
    let mut placed = Vec::with_capacity(planned.len());
    for cell in &planned {
        let id = cell_id(cell.kit, args.seed, cell.x, cell.z, cell.yaw)?;
        ops.push(Op::Insert {
            id,
            parent: ObjectId::NIL,
            local: Transform::new([cell.x, 0.0, cell.z], quat_of_yaw(cell.yaw), cell.scale),
        });
        placed.push(PaintedDto {
            id: id.to_string(),
            kit: cell.kit.to_owned(),
            x: cell.x,
            y: 0.0,
            z: cell.z,
            yaw: cell.yaw,
        });
    }
    let cells = planned.len();
    let batch = Batch {
        ops,
        base_revision: state.doc.revision,
        idempotency_key: args.idempotency_key,
        provenance: Some(ProvenanceStamp::script(TOOL_LEVEL_PAINT)),
    };
    let _preview = commands::validate(&state.doc, &batch).map_err(|error| batch_error(&error))?;
    let approval = approve(
        state.session,
        &batch,
        state.doc.revision,
        state.doc.revision,
    );
    let granted = granted_ops(&state.cap);
    let applied = capability::apply_gated(
        &mut state.doc,
        &mut state.journal,
        &batch,
        &mut state.cap,
        &approval,
        &mut state.ledger,
    )
    .map_err(|error| cap_error(&error, &granted, &state.doc))?;
    // A landed paint is a watch event: move the session cache, as a build does.
    state.refresh_watch();
    let checks = vec![
        crate::verify_tasks::check(
            "level.painted",
            TOOL_LEVEL_PAINT,
            cells > 0,
            format!("{cells} cells"),
        ),
        crate::verify_tasks::check(
            "level.digest",
            TOOL_LEVEL_PAINT,
            applied.after_digest.iter().any(|byte| *byte != 0),
            format!("revision {}", applied.revision),
        ),
    ];
    let first_id = placed
        .first()
        .map_or_else(String::new, |row| row.id.clone());
    let last_id = placed.last().map_or_else(String::new, |row| row.id.clone());
    Ok(PaintOutput {
        revision: applied.revision,
        cells,
        ops: cells,
        placed,
        first_id,
        last_id,
        digest: hex_bytes(&applied.after_digest),
        effects: applied.effects.clone(),
        key: applied.key,
        undo_token: applied.key,
        verify: crate::verify_tasks::VerifyBlock::of(TOOL_LEVEL_PAINT, &checks).wire_report(),
    })
}

crate::e2_tool!(
    /// `level.paint`: lay a kit across the cells of one room in one undo step.
    PaintTool,
    name = TOOL_LEVEL_PAINT,
    description = "Paint a room in one gated call: name a kit from the level palette and the \
                   cells to fill, and the call expands them into one batch of inserts inside one \
                   undo step. Cells come from `cells` (poses in meters), a `region` block of grid \
                   steps, or a `rules` adjacency table collapsed over the region by the wave-\
                   function solver; `scatter` adds seeded Poisson-disk points of a kit, over \
                   the region when the call gives one and over the level square when it does \
                   not. \
                   Answers the revision, the cell and op counts, the landed ids first to last, the \
                   document digest and the undo key. Ids derive from `(kit, seed, cell)`, so the \
                   same call twice gives the same digest. Refuses `no_kit` with the palette, \
                   `off_grid` and `rule_unsatisfiable` with the cell, `too_many_cells` with the \
                   count, and writes nothing on any refusal.",
    readonly = false,
    args = PaintArgs,
    output = PaintOutput,
    invoke = paint_verb
);

/// Handler: audit the call through the shared pack dispatch seam, then paint.
/// `level.paint` is not a manifest name, so it rides `dispatch` like every
/// other pack verb; no kit string reaches the document unvalidated.
fn paint_verb(server: &McpServer, args: &PaintArgs) -> Result<PaintOutput, McpToolError> {
    server.dispatch(TOOL_LEVEL_PAINT, |state| paint_on_state(state, args))
}

/// The level-paint pack: one verb, loaded on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "kit painting: level.paint expands a kit over explicit cells, a grid region, an \
         authored adjacency solve and seeded scatter into one batch of inserts in one undo step",
        false,
    );
    pack.add(VerbSpec::of::<PaintTool>(
        TOOL_LEVEL_PAINT,
        PACK,
        CapabilityCard::Commit,
    )?);
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::{SceneReadArgs, SceneReadTool, TxUndoArgs, TxUndoTool};
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;

    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    /// A 6 x 4 block on the default 2 m step: the 24-cell wall run of the row.
    fn wall_run() -> RegionDto {
        RegionDto {
            cx: -6,
            cz: -4,
            w: 6,
            h: 4,
            step_m: 0.0,
        }
    }

    fn walls() -> PaintArgs {
        PaintArgs {
            kit: "wall".to_owned(),
            seed: 2040,
            expected_revision: 0,
            idempotency_key: 700,
            region: Some(wall_run()),
            ..Default::default()
        }
    }

    #[test]
    fn twenty_four_cells_land_in_one_call() {
        let one = server();
        let out = PaintTool::invoke(&one, walls()).expect("a wall run paints");
        assert_eq!(out.cells, 24, "a 6 x 4 block is 24 cells");
        assert_eq!(out.ops, 24, "one insert per cell");
        assert_eq!(out.revision, 1, "one batch is one revision step");
        assert_eq!(out.placed.len(), 24);
        assert_eq!(out.first_id, out.placed[0].id);
        assert_eq!(out.last_id, out.placed[23].id);
        assert_eq!(out.key, out.undo_token);
        assert_eq!(out.placed[0].kit, "wall", "the kit did not reach the row");
        let page = SceneReadTool::invoke(
            &one,
            SceneReadArgs {
                limit: Some(40),
                cursor: None,
            },
        )
        .expect("the scene reads");
        assert_eq!(page.total, 24, "one call wrote 24 live nodes");
    }

    #[test]
    fn one_undo_takes_all_twenty_four_off() {
        let one = server();
        let out = PaintTool::invoke(&one, walls()).expect("a wall run paints");
        let undone = TxUndoTool::invoke(
            &one,
            TxUndoArgs {
                undo_token: Some(out.undo_token),
            },
        )
        .expect("the batch undoes");
        assert_eq!(undone.revision, 2, "undo is its own revision step");
        let back = SceneReadTool::invoke(&one, SceneReadArgs::default()).expect("scene reads");
        assert_eq!(back.total, 0, "one undo removed all 24 cells");
    }

    #[test]
    fn the_same_seed_gives_the_same_digest_twice() {
        let args = PaintArgs {
            kit: "crate".to_owned(),
            seed: 21,
            idempotency_key: 702,
            cells: vec![
                CellDto {
                    x: 1.0,
                    z: 2.0,
                    yaw: 0.0,
                },
                CellDto {
                    x: -3.0,
                    z: 4.5,
                    yaw: 1.5,
                },
            ],
            ..walls()
        };
        let first = PaintTool::invoke(&server(), args.clone()).expect("first paint");
        let second = PaintTool::invoke(&server(), args).expect("second paint");
        assert_eq!(first.digest, second.digest, "the document digest moved");
        assert_eq!(first.placed, second.placed, "the ids moved");
        assert_eq!(first.digest.len(), 64, "not a hex document digest");
    }

    #[test]
    fn typed_order_never_reaches_the_document() {
        let base = PaintArgs {
            kit: "post".to_owned(),
            seed: 22,
            idempotency_key: 703,
            region: None,
            ..walls()
        };
        let one = PaintArgs {
            cells: vec![
                CellDto {
                    x: 2.0,
                    z: 2.0,
                    yaw: 0.0,
                },
                CellDto {
                    x: 0.0,
                    z: 0.0,
                    yaw: 0.0,
                },
            ],
            ..base.clone()
        };
        let flipped = PaintArgs {
            cells: vec![
                CellDto {
                    x: 0.0,
                    z: 0.0,
                    yaw: 0.0,
                },
                CellDto {
                    x: 2.0,
                    z: 2.0,
                    yaw: 0.0,
                },
            ],
            ..base
        };
        let first = PaintTool::invoke(&server(), one).expect("first order");
        let second = PaintTool::invoke(&server(), flipped).expect("second order");
        assert_eq!(
            first.digest, second.digest,
            "typed order reached the document"
        );
        assert_eq!(first.cells, 2, "two posts, not the region too");
        assert_eq!(first.placed, second.placed, "the batch order moved");
    }

    #[test]
    fn refusals_name_their_fix_and_write_nothing() {
        let one = server();
        let unknown = PaintTool::invoke(
            &one,
            PaintArgs {
                kit: "laser".to_owned(),
                ..walls()
            },
        )
        .expect_err("an unknown kit refuses");
        assert_eq!(unknown.code, "no_kit");
        assert!(
            unknown.message.contains("barrel"),
            "the palette is not listed: {}",
            unknown.message
        );

        let edge = PaintTool::invoke(
            &one,
            PaintArgs {
                cells: vec![CellDto {
                    x: 90.0,
                    z: 0.0,
                    yaw: 0.0,
                }],
                region: None,
                ..walls()
            },
        )
        .expect_err("a cell past the edge refuses");
        assert_eq!(edge.code, "off_grid");
        assert!(edge.message.contains("90"), "the cell is not named");

        let twice = PaintTool::invoke(
            &one,
            PaintArgs {
                cells: vec![
                    CellDto {
                        x: 5.0,
                        z: 5.0,
                        yaw: 0.0,
                    },
                    CellDto {
                        x: 5.0,
                        z: 5.0,
                        yaw: 0.0,
                    },
                ],
                region: None,
                ..walls()
            },
        )
        .expect_err("one cell submitted twice refuses");
        assert_eq!(twice.code, "bad_cell");
        assert!(
            twice.message.contains("twice"),
            "the repeat is not explained: {}",
            twice.message
        );

        let many = PaintTool::invoke(
            &one,
            PaintArgs {
                region: Some(RegionDto {
                    cx: 0,
                    cz: 0,
                    w: 20,
                    h: 20,
                    step_m: 1.0,
                }),
                max_cells: 100,
                ..walls()
            },
        )
        .expect_err("a block past the cap refuses");
        assert_eq!(many.code, "too_many_cells");
        assert!(many.message.contains("400"), "the count is not named");

        let idle = PaintTool::invoke(
            &one,
            PaintArgs {
                region: None,
                ..walls()
            },
        )
        .expect_err("no source refuses");
        assert_eq!(idle.code, "bad_args");

        let stale = PaintTool::invoke(
            &one,
            PaintArgs {
                expected_revision: 9,
                cells: vec![CellDto {
                    x: 0.0,
                    z: 0.0,
                    yaw: 0.0,
                }],
                region: None,
                ..walls()
            },
        )
        .expect_err("a stale revision refuses");
        assert_eq!(stale.code, "stale_revision");
        let page = SceneReadTool::invoke(&one, SceneReadArgs::default()).expect("scene reads");
        assert_eq!(page.total, 0, "refusals wrote nothing");
    }

    /// A two-kind table that always fits: every neighbour is allowed. Both
    /// bodies are under 1 m wide, so any pair 2 m apart stages without
    /// overlap and the door's spatial gate stays out of the solve's way.
    fn two_kinds() -> RulesDto {
        RulesDto {
            kinds: vec!["crate".to_owned(), "post".to_owned()],
            masks: vec![[0b11, 0b11, 0b11, 0b11], [0b11, 0b11, 0b11, 0b11]],
            ..Default::default()
        }
    }

    #[test]
    fn a_rule_that_cannot_hold_names_the_cell() {
        let one = server();
        let refused = PaintTool::invoke(
            &one,
            PaintArgs {
                region: Some(RegionDto {
                    cx: 0,
                    cz: 0,
                    w: 2,
                    h: 2,
                    step_m: 1.0,
                }),
                rules: Some(RulesDto {
                    pinned: vec![
                        PinDto {
                            x: 0,
                            z: 0,
                            kind: 0,
                        },
                        PinDto {
                            x: 0,
                            z: 0,
                            kind: 1,
                        },
                    ],
                    ..two_kinds()
                }),
                ..walls()
            },
        )
        .expect_err("two different pins on one cell cannot both hold");
        assert_eq!(refused.code, "rule_unsatisfiable");
        assert!(
            refused.message.contains("(0, 0)"),
            "the cell is not named: {}",
            refused.message
        );
        let page = SceneReadTool::invoke(&one, SceneReadArgs::default()).expect("scene reads");
        assert_eq!(page.total, 0, "a refused solve wrote nothing");
    }

    #[test]
    fn a_solved_region_paints_the_kinds_it_collapsed() {
        let args = PaintArgs {
            kit: "crate".to_owned(),
            seed: 2040,
            expected_revision: 0,
            region: Some(RegionDto {
                cx: -2,
                cz: -2,
                w: 3,
                h: 3,
                step_m: 2.0,
            }),
            rules: Some(RulesDto {
                pinned: vec![PinDto {
                    x: 1,
                    z: 1,
                    kind: 0,
                }],
                open: 0b10,
                connect: vec![LinkDto {
                    from_x: 0,
                    from_z: 0,
                    to_x: 2,
                    to_z: 0,
                }],
                ..two_kinds()
            }),
            ..walls()
        };
        let one = server();
        let out = PaintTool::invoke(&one, args.clone()).expect("a solved region paints");
        assert_eq!(out.cells, 9, "a 3 x 3 solve fills nine cells");
        assert!(
            out.placed.iter().any(|row| row.kit == "crate"),
            "the pin did not land a crate"
        );
        assert!(
            out.placed.iter().any(|row| row.kit == "post"),
            "the connect corridor did not land posts"
        );
        let replay = PaintTool::invoke(
            &server(),
            PaintArgs {
                idempotency_key: 711,
                ..args
            },
        )
        .expect("the same solve replays on a fresh session");
        assert_eq!(out.digest, replay.digest, "the solve moved between runs");
        assert_eq!(out.placed, replay.placed, "the solved ids moved");
    }

    #[test]
    fn scatter_lays_seeded_points_over_the_level_square() {
        let args = PaintArgs {
            kit: "barrel".to_owned(),
            seed: 2040,
            expected_revision: 0,
            idempotency_key: 712,
            region: None,
            scatter: Some(ScatterDto {
                kit: "barrel".to_owned(),
                recipe: "forest edge".to_owned(),
                min_dist_m: 3.0,
            }),
            ..walls()
        };
        let out = PaintTool::invoke(&server(), args.clone())
            .expect("a seeded scatter paints the level square");
        assert!(
            out.cells > 4,
            "the square frame is 40 x 40 m at 3 m spacing: {} barrels",
            out.cells
        );
        assert!(
            out.placed.iter().all(|row| row.kit == "barrel"),
            "a source other than the scatter landed"
        );
        for (index, left) in out.placed.iter().enumerate() {
            for right in out.placed[index + 1..].iter() {
                let apart = ((left.x - right.x).powi(2) + (left.z - right.z).powi(2)).sqrt();
                assert!(apart >= 2.999, "two barrels landed {apart} m apart");
            }
        }
        let replay = PaintTool::invoke(
            &server(),
            PaintArgs {
                idempotency_key: 713,
                ..args
            },
        )
        .expect("the same seed replays");
        assert_eq!(out.digest, replay.digest, "the scatter moved between runs");
        assert_eq!(out.placed, replay.placed, "the scattered ids moved");
    }

    /// A minted id, or the test's own failure: the mint is 32 hex chars.
    fn mint(kit: &str, seed: u64, x: f32, z: f32, yaw: f32) -> ObjectId {
        cell_id(kit, seed, x, z, yaw).expect("a 32-hex minted id")
    }

    #[test]
    fn the_cell_id_is_stable_and_never_a_random_mint() {
        let first = mint("wall", 2040, 1.0, -2.0, 0.0);
        assert_eq!(first, mint("wall", 2040, 1.0, -2.0, 0.0));
        assert_ne!(
            first,
            mint("wall", 2040, 1.0, -2.0, 1.0),
            "yaw did not reach the id"
        );
        assert_ne!(
            first,
            mint("crate", 2040, 1.0, -2.0, 0.0),
            "the kit did not reach the id"
        );
        let text = first.to_string();
        assert_eq!(text.matches('-').count(), 4, "not UUID text: {text}");
    }
}
