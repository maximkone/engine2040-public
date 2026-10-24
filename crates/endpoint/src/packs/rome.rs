//! `rome` pack: Rome door verbs over the dig grid.
//!
//! Consumer: an MCP client staging a Roman camp. `rome.camp` lays a marching camp from
//! `content/rome/camp.toml` on a flat square site: one ditch run per side, dug to the depth
//! that table gives for the named threat, and it answers the works list the perimeter is
//! built from (`engine_world::works`). `rome.undo` puts the flat site back, byte for byte.
//! Each call answers a capture (the grid digest before and after, the earth it moved) plus a
//! receipt, and it is one undoable change.
//!
//! Data owner: the session's slot in this module (the door document holds props, never
//! terrain), keyed by `state.session` like the world pack's terrain table. Units: metres at
//! the args edge, millimetres inside; earth in cell-millimetres, positive dug and negative
//! put back. Determinism: integer math only, fixed side order (south, east, north, west), no
//! clock, so the same site and threat dig the same grid twice. Errors: `bad_threat`,
//! `bad_site`, `rome_table`, `nothing_to_undo`, `lock_poisoned`.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

use engine_core::Fnv64;
use engine_rome::{Camp, Ditch, Threat};
use engine_world::dig::{HeightGrid, RectMm, CELL_MM};
use engine_world::works::{CampPlan, CampWorks, WorkKind};

use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::{PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::tool_capabilities::CapabilityCard;

/// Tool: lay a marching camp on a flat site.
pub const TOOL_ROME_CAMP: &str = "rome.camp";
/// Tool: put the flat site back under the newest camp.
pub const TOOL_ROME_UNDO: &str = "rome.undo";

/// Pack name.
pub const PACK: &str = "rome";

/// Smallest site the verb accepts, in metres of perimeter.
const MIN_SITE_M: u32 = 20;
/// Largest site the verb accepts, in metres of perimeter: a 200 m side stays a few MB.
const MAX_SITE_M: u32 = 800;

/// Gates every camp opens, one build job each (the march's two).
const GATES: u32 = 2;

/// The four sides a camp's ditch runs along, in dig order.
const SIDES: [&str; 4] = ["south", "east", "north", "west"];

/// The threat spellings the args accept, with the table threat each one names.
const THREATS: [(&str, Threat); 3] = [
    ("one_night", Threat::OneNight),
    ("light", Threat::Light),
    ("serious", Threat::Serious),
];

/// The threat spellings, comma separated, for refusal messages.
const THREAT_LIST: &str = "one_night, light, serious";

/// The threat an arg spelling names, if any.
fn threat_of(name: &str) -> Option<Threat> {
    THREATS
        .iter()
        .find(|(text, _)| *text == name)
        .map(|(_, threat)| *threat)
}

/// The arg spelling a threat answers to.
fn threat_name(threat: Threat) -> &'static str {
    THREATS[threat.index()].0
}

/// The works-list name of one kind.
fn work_name(kind: WorkKind) -> &'static str {
    match kind {
        WorkKind::Ditch => "ditch",
        WorkKind::Rampart => "rampart",
        WorkKind::StakeLine => "stake_line",
        WorkKind::Gate => "gate",
    }
}

/// The session's dig grid: the newest camp's grid, the flat site under it and the
/// camp's own output, so the same idempotency key replays instead of digging again.
#[derive(Default)]
struct RomeSlot {
    grid: Option<HeightGrid>,
    flat: Option<HeightGrid>,
    earth_cell_mm: i64,
    applied: Option<Applied>,
}

/// One landed camp: the key and args that dug it, plus the answer to replay.
struct Applied {
    key: u64,
    site: u32,
    threat: String,
    output: RomeCampOutput,
}

/// The per-session camps, keyed by the door session id.
static SLOTS: OnceLock<Mutex<BTreeMap<u64, RomeSlot>>> = OnceLock::new();

/// The camp table, or the poisoned-lock refusal naming the restart.
fn slots() -> Result<MutexGuard<'static, BTreeMap<u64, RomeSlot>>, McpToolError> {
    SLOTS.get_or_init(Mutex::default).lock().map_err(|_| {
        McpToolError::new(
            "lock_poisoned",
            "the Rome camp table is poisoned; restart the door",
        )
    })
}

// ---------------------------------------------------------------------------
// rome.camp
// ---------------------------------------------------------------------------

/// Args for `rome.camp`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RomeCampArgs {
    /// Perimeter of the flat square site in metres, the four runs end to end, `20..=800`.
    pub site: u32,
    /// Threat the camp is dug for: `one_night`, `light` or `serious`.
    pub threat: String,
    /// Idempotency key: the same key with the same site and threat replays the stored receipt.
    pub key: u64,
}

/// One ditch run along one side of the site.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RomeDitchRun {
    /// Side it runs along: `south`, `east`, `north` or `west`.
    pub side: String,
    /// Length of the run, millimetres.
    pub length_mm: u32,
    /// Ditch width at the top from `camp.toml`, millimetres.
    pub width_mm: u32,
    /// Ditch depth from `camp.toml`, millimetres.
    pub depth_mm: u32,
    /// Grid cells the run lowered.
    pub cells: u32,
    /// Earth it took out, cell-millimetres.
    pub earth_cell_mm: i64,
    /// The table `ref` the width came from.
    pub source: String,
}

/// One build job the camp perimeter needs.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RomeWork {
    /// Work kind: `ditch`, `rampart`, `stake_line` or `gate`.
    pub kind: String,
    /// Work steps the job needs.
    pub total: u32,
    /// Work steps already done.
    pub done: u32,
}

/// What the call changed on the grid: digests before and after plus the earth moved.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RomeCapture {
    /// Grid columns.
    pub cols: u32,
    /// Grid rows.
    pub rows: u32,
    /// Whole-grid digest before the call (16 hex digits).
    pub grid_before: String,
    /// Whole-grid digest after the call (16 hex digits).
    pub grid_after: String,
    /// Cells the call lowered or raised.
    pub cells: u32,
    /// Earth moved, cell-millimetres: positive dug, negative put back.
    pub earth_cell_mm: i64,
}

/// The seal on one call: session, verb, key and the digest of the change.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RomeReceipt {
    /// Door session the grid belongs to.
    pub session: u64,
    /// The verb that sealed it.
    pub verb: String,
    /// Idempotency key it answered.
    pub key: u64,
    /// Document revision after the call.
    pub revision: u64,
    /// Digest of the before and after grid digests (16 hex digits).
    pub digest: String,
}

/// Result of `rome.camp`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RomeCampOutput {
    /// Site side, metres.
    pub site_m: u32,
    /// Threat it was dug for.
    pub threat: String,
    /// Site perimeter, millimetres: the four runs end to end.
    pub perimeter_mm: u32,
    /// The four ditch runs, south, east, north then west.
    pub runs: Vec<RomeDitchRun>,
    /// The works list the perimeter is built from.
    pub works: Vec<RomeWork>,
    /// Rampart width from `camp.toml`, millimetres.
    pub rampart_width_mm: u32,
    /// Rampart height from `camp.toml`, millimetres.
    pub rampart_height_mm: u32,
    /// The via principalis from `camp.toml`, millimetres.
    pub via_principalis_mm: u32,
    /// What the call changed.
    pub capture: RomeCapture,
    /// The seal on the call.
    pub receipt: RomeReceipt,
    /// True when the same key and args replayed a stored answer instead of digging.
    pub replayed: bool,
}

/// Digest of one change: the before and after grid digests under a version tag.
fn change_digest(verb: &str, before: u64, after: u64) -> String {
    let mut hasher = Fnv64::new();
    hasher.write(b"engine-endpoint/rome/1");
    hasher.write(verb.as_bytes());
    hasher.write_u64(before);
    hasher.write_u64(after);
    format!("{:016x}", hasher.finish())
}

/// The rect one side's ditch band covers: the band hugs the site edge and is as wide as
/// the table says, so the four runs close a square perimeter.
fn run_rect(side: usize, side_mm: i32, width_mm: i32) -> RectMm {
    let width = width_mm.clamp(1, side_mm.max(1));
    let inner = side_mm.saturating_sub(width);
    match side {
        0 => RectMm::new(0, 0, side_mm, width),
        1 => RectMm::new(inner, 0, side_mm, side_mm),
        2 => RectMm::new(0, inner, side_mm, side_mm),
        _ => RectMm::new(0, 0, width, side_mm),
    }
}

/// Dig one run per side, in `SIDES` order, and report the runs with the cells and the
/// earth the four took.
fn dig_runs(
    grid: &mut HeightGrid,
    side_mm: i32,
    ditch: Ditch<'_>,
) -> (Vec<RomeDitchRun>, u32, i64) {
    let depth = i32::try_from(ditch.depth.mm).unwrap_or(i32::MAX);
    let width = i32::try_from(ditch.width.mm).unwrap_or(i32::MAX);
    let mut runs = Vec::with_capacity(SIDES.len());
    let mut earth_cell_mm = 0i64;
    let mut cells = 0u32;
    for (index, side) in SIDES.iter().enumerate() {
        let moved = grid.dig(run_rect(index, side_mm, width), -depth);
        let run_cells = u32::try_from(moved.cells).unwrap_or(u32::MAX);
        earth_cell_mm = earth_cell_mm.saturating_add(moved.amount_cell_mm);
        cells = cells.saturating_add(run_cells);
        runs.push(RomeDitchRun {
            side: (*side).to_owned(),
            length_mm: u32::try_from(side_mm).unwrap_or(u32::MAX),
            width_mm: ditch.width.mm,
            depth_mm: ditch.depth.mm,
            cells: run_cells,
            earth_cell_mm: moved.amount_cell_mm,
            source: ditch.width.source.to_owned(),
        });
    }
    (runs, cells, earth_cell_mm)
}

/// The works list one perimeter is built from, in plan order.
fn works_rows(perimeter_mm: u32) -> Vec<RomeWork> {
    let perimeter = i32::try_from(perimeter_mm).unwrap_or(i32::MAX);
    CampWorks::new(CampPlan {
        perimeter_mm: perimeter,
        gates: GATES,
    })
    .jobs()
    .iter()
    .map(|job| RomeWork {
        kind: work_name(job.kind).to_owned(),
        total: job.total,
        done: job.done,
    })
    .collect()
}

/// Dig the four runs and lay the plan's jobs. Pure over the grids it returns: the dug
/// grid, the flat site it was dug on, and the earth it moved.
fn lay_camp(
    camp: &Camp<'_>,
    threat: Threat,
    ditch: Ditch<'_>,
    args: &RomeCampArgs,
    session: u64,
    revision: u64,
) -> Result<(RomeCampOutput, HeightGrid, HeightGrid, i64), McpToolError> {
    let perimeter_mm = u32::try_from(u64::from(args.site) * 1000)
        .map_err(|_| McpToolError::new("bad_site", format!("site {} m is too long", args.site)))?;
    let side_mm = i32::try_from(perimeter_mm / 4).unwrap_or(i32::MAX);
    let cols = usize::try_from(side_mm / CELL_MM).unwrap_or(0);
    let flat = HeightGrid::new(cols, cols, 0);
    let mut grid = flat.clone();
    let before = flat.state_hash();
    let (runs, cells, earth_cell_mm) = dig_runs(&mut grid, side_mm, ditch);
    let works = works_rows(perimeter_mm);
    let after = grid.state_hash();
    let output = RomeCampOutput {
        site_m: args.site,
        threat: threat_name(threat).to_owned(),
        perimeter_mm,
        runs,
        works,
        rampart_width_mm: camp.rampart.width.mm,
        rampart_height_mm: camp.rampart.height.mm,
        via_principalis_mm: camp.streets.via_principalis.mm,
        capture: RomeCapture {
            cols: u32::try_from(cols).unwrap_or(u32::MAX),
            rows: u32::try_from(cols).unwrap_or(u32::MAX),
            grid_before: format!("{before:016x}"),
            grid_after: format!("{after:016x}"),
            cells,
            earth_cell_mm,
        },
        receipt: RomeReceipt {
            session,
            verb: TOOL_ROME_CAMP.to_owned(),
            key: args.key,
            revision,
            digest: change_digest(TOOL_ROME_CAMP, before, after),
        },
        replayed: false,
    };
    Ok((output, flat, grid, earth_cell_mm))
}

fn rome_camp(server: &McpServer, args: &RomeCampArgs) -> Result<RomeCampOutput, McpToolError> {
    let threat = threat_of(&args.threat).ok_or_else(|| {
        McpToolError::new(
            "bad_threat",
            format!(
                "threat `{}` is not one of {THREAT_LIST}; re-run with one of those",
                args.threat
            ),
        )
    })?;
    if !(MIN_SITE_M..=MAX_SITE_M).contains(&args.site) {
        return Err(McpToolError::new(
            "bad_site",
            format!(
                "site {} m is outside {MIN_SITE_M}..={MAX_SITE_M} m; re-run with a perimeter in that range",
                args.site
            ),
        ));
    }
    let camp = Camp::shipped().map_err(|error| {
        McpToolError::new(
            "rome_table",
            format!("content/rome/camp.toml did not load ({error}); re-run once it loads"),
        )
    })?;
    let ditch = camp.ditch(threat);
    server.dispatch(TOOL_ROME_CAMP, |state| {
        let session = state.session;
        let revision = state.doc.revision;
        let mut table = slots()?;
        let slot = table.entry(session).or_default();
        if let Some(applied) = &slot.applied {
            if applied.key == args.key && applied.site == args.site && applied.threat == args.threat
            {
                let mut replayed = applied.output.clone();
                replayed.replayed = true;
                return Ok(replayed);
            }
        }
        let (output, flat, dug, earth_cell_mm) =
            lay_camp(&camp, threat, ditch, args, session, revision)?;
        slot.grid = Some(dug);
        slot.flat = Some(flat);
        slot.earth_cell_mm = earth_cell_mm;
        slot.applied = Some(Applied {
            key: args.key,
            site: args.site,
            threat: args.threat.clone(),
            output: output.clone(),
        });
        Ok(output)
    })
}

crate::e2_tool!(
    /// `rome.camp`: lay a marching camp from camp.toml on a flat site.
    RomeCampTool,
    name = TOOL_ROME_CAMP,
    description = "Lay a marching camp from content/rome/camp.toml on a flat square site: one ditch run per side (south, east, north, west) dug to the depth the named threat gives, plus the works list (ditch, rampart, stake line, two gates) the perimeter is built from. Answers a capture of the grid before and after and a receipt. One undoable change; the same key with the same site and threat replays it.",
    readonly = false,
    args = RomeCampArgs,
    output = RomeCampOutput,
    invoke = rome_camp
);

// ---------------------------------------------------------------------------
// rome.undo
// ---------------------------------------------------------------------------

/// Args for `rome.undo`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RomeUndoArgs {
    /// Idempotency key of the undo itself.
    pub key: u64,
}

/// Result of `rome.undo`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RomeUndoOutput {
    /// Site side the camp stood on, metres (0 when nothing stood).
    pub site_m: u32,
    /// What the undo changed.
    pub capture: RomeCapture,
    /// The seal on the call.
    pub receipt: RomeReceipt,
}

fn rome_undo(server: &McpServer, args: &RomeUndoArgs) -> Result<RomeUndoOutput, McpToolError> {
    server.dispatch(TOOL_ROME_UNDO, |state| {
        let session = state.session;
        let revision = state.doc.revision;
        let mut table = slots()?;
        let slot = table.entry(session).or_default();
        let Some(flat) = slot.flat.take() else {
            return Err(McpToolError::new(
                "nothing_to_undo",
                "no camp stands on this session; call rome.camp first".to_owned(),
            ));
        };
        let dug = slot.grid.as_ref().map_or(0, HeightGrid::state_hash);
        let site_m = slot.applied.as_ref().map_or(0, |applied| applied.site);
        let earth_cell_mm = std::mem::take(&mut slot.earth_cell_mm);
        slot.grid = Some(flat.clone());
        slot.applied = None;
        let restored = flat.state_hash();
        let cells = u32::try_from(flat.cols().saturating_mul(flat.rows())).unwrap_or(u32::MAX);
        Ok(RomeUndoOutput {
            site_m,
            capture: RomeCapture {
                cols: u32::try_from(flat.cols()).unwrap_or(u32::MAX),
                rows: u32::try_from(flat.rows()).unwrap_or(u32::MAX),
                grid_before: format!("{dug:016x}"),
                grid_after: format!("{restored:016x}"),
                // The whole site comes back: every cell is written by the restore.
                cells,
                earth_cell_mm: earth_cell_mm.saturating_neg(),
            },
            receipt: RomeReceipt {
                session,
                verb: TOOL_ROME_UNDO.to_owned(),
                key: args.key,
                revision,
                digest: change_digest(TOOL_ROME_UNDO, dug, restored),
            },
        })
    })
}

crate::e2_tool!(
    /// `rome.undo`: put the flat site back under the newest camp.
    RomeUndoTool,
    name = TOOL_ROME_UNDO,
    description = "Put the flat site back under the newest rome.camp on this session, byte for byte, and answer the capture of that restore plus a receipt. With no camp standing it refuses nothing_to_undo.",
    readonly = false,
    args = RomeUndoArgs,
    output = RomeUndoOutput,
    invoke = rome_undo
);

// ---------------------------------------------------------------------------
// Reading the grid back
// ---------------------------------------------------------------------------

/// The session's dig grid: `None` before any camp. For the door's own tests and a read
/// verb later; it copies the grid, so a caller can never write the session's own bytes.
#[cfg(test)]
pub(crate) fn session_grid(session: u64) -> Result<Option<HeightGrid>, McpToolError> {
    let table = slots()?;
    Ok(table.get(&session).and_then(|slot| slot.grid.clone()))
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// The rome pack: two commit verbs, loaded on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "Rome: lay a marching camp from camp.toml on the dig grid with its works list, and undo it",
        false,
    );
    pack.add(VerbSpec::of::<RomeCampTool>(
        TOOL_ROME_CAMP,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<RomeUndoTool>(
        TOOL_ROME_UNDO,
        PACK,
        CapabilityCard::Commit,
    )?);
    Ok(pack)
}

// The pack publishes its own row; `packs/mod.rs` keeps no hand list.
#[expect(
    unsafe_code,
    reason = "linkme publishes a pack row through a link section"
)]
#[linkme::distributed_slice(super::PACK_MODULES)]
static PACK_ROW: super::PackModule = super::PackModule {
    order: 27,
    name: PACK,
    default_loaded: false,
    tools: &[TOOL_ROME_CAMP, TOOL_ROME_UNDO],
    build: pack,
};

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;

    /// `camp.toml`: the light ditch and the rampart every camp raises.
    const LIGHT_WIDTH_MM: u32 = 2664;
    const LIGHT_DEPTH_MM: u32 = 2072;
    const RAMPART_WIDTH_MM: u32 = 2368;
    const RAMPART_HEIGHT_MM: u32 = 1776;
    const VIA_PRINCIPALIS_MM: u32 = 17_760;

    /// Session ids distinct per test: the camp table is process-wide, like the world
    /// pack's terrain table, so two tests must never share one session id.
    const CAMP_SESSION: u64 = 9101;
    const UNDO_SESSION: u64 = 9102;
    const THREAT_SESSION: u64 = 9103;
    const REPLAY_SESSION: u64 = 9104;
    const REPLAY_TWIN: u64 = 9105;

    fn server(session: u64) -> McpServer {
        McpServer::new(session, Capability::unrestricted(session))
    }

    fn camp_args(site: u32, threat: &str, key: u64) -> RomeCampArgs {
        RomeCampArgs {
            site,
            threat: threat.to_owned(),
            key,
        }
    }

    /// A flat site: the grid every undug metre starts on. `site_m` is the perimeter, so a
    /// 200 m site is four 50 m sides.
    fn flat_site(site_m: u32) -> HeightGrid {
        let side_mm = i32::try_from(site_m * 1000 / 4).expect("a site side fits i32 millimetres");
        let cells = usize::try_from(side_mm / CELL_MM).expect("cells are positive");
        HeightGrid::new(cells, cells, 0)
    }

    #[test]
    fn a_flat_200_m_site_dug_for_light_returns_four_ditch_runs_from_the_table() {
        let door = server(CAMP_SESSION);
        let out = RomeCampTool::invoke(&door, camp_args(200, "light", 1)).expect("the camp digs");
        // Four runs, one per side, each a quarter of the 200 m perimeter.
        assert_eq!(out.runs.len(), 4);
        let sides: Vec<&str> = out.runs.iter().map(|run| run.side.as_str()).collect();
        assert_eq!(sides, ["south", "east", "north", "west"]);
        for run in &out.runs {
            assert_eq!(run.length_mm, 50_000, "{} is a quarter of 200 m", run.side);
            assert_eq!(run.width_mm, LIGHT_WIDTH_MM, "camp.toml light width");
            assert_eq!(run.depth_mm, LIGHT_DEPTH_MM, "camp.toml light depth");
            assert!(run.cells > 0, "{} dug cells", run.side);
            assert!(run.earth_cell_mm > 0, "{} took earth out", run.side);
            assert!(run.source.contains("rome.md"), "the table ref rides along");
        }
        assert_eq!(out.perimeter_mm, 200_000, "four 50 m runs end to end");
        assert!(out.capture.earth_cell_mm > 0);
        assert_ne!(
            out.capture.grid_before, out.capture.grid_after,
            "the grid moved"
        );
        assert_eq!(out.capture.cols, 100, "50 m of 500 mm cells");
        assert!(!out.replayed);
        assert_eq!(out.receipt.key, 1);
        assert_eq!(out.receipt.verb, TOOL_ROME_CAMP);
        assert_eq!(out.receipt.session, CAMP_SESSION);
    }

    #[test]
    fn a_camp_returns_the_works_list_from_camp_and_the_camp_table() {
        let door = server(CAMP_SESSION);
        let out = RomeCampTool::invoke(&door, camp_args(200, "light", 2)).expect("the camp digs");
        let kinds: Vec<&str> = out.works.iter().map(|work| work.kind.as_str()).collect();
        assert_eq!(
            kinds,
            ["ditch", "rampart", "stake_line", "gate", "gate"],
            "three perimeter works and the two gates"
        );
        assert!(
            out.works.iter().all(|work| work.done == 0),
            "nothing worked"
        );
        assert_eq!(out.works[0].total, 1000, "200 m of ditch at 200 mm a step");
        // The rest of the same table rides along, so a client quotes it, not a copy.
        assert_eq!(out.rampart_width_mm, RAMPART_WIDTH_MM);
        assert_eq!(out.rampart_height_mm, RAMPART_HEIGHT_MM);
        assert_eq!(out.via_principalis_mm, VIA_PRINCIPALIS_MM);
    }

    #[test]
    fn undo_restores_the_grid_byte_equal() {
        let door = server(UNDO_SESSION);
        let out = RomeCampTool::invoke(&door, camp_args(200, "light", 1)).expect("the camp digs");
        assert_ne!(
            session_grid(UNDO_SESSION).expect("the table reads"),
            Some(flat_site(200)),
            "the camp moved the ground"
        );
        let undone = RomeUndoTool::invoke(&door, RomeUndoArgs { key: 2 }).expect("the undo lands");
        // Byte equal: every cell of the site is back at its flat height.
        let restored = session_grid(UNDO_SESSION)
            .expect("the table reads")
            .expect("the grid stays");
        assert_eq!(
            restored,
            flat_site(200),
            "undo restores the grid byte equal"
        );
        assert_eq!(
            undone.capture.grid_after,
            format!("{:016x}", flat_site(200).state_hash())
        );
        assert_eq!(undone.capture.grid_before, out.capture.grid_after);
        assert_eq!(undone.capture.earth_cell_mm, -out.capture.earth_cell_mm);
        assert_eq!(undone.site_m, 200);
        // A second undo has nothing under it and refuses typed.
        let refused = RomeUndoTool::invoke(&door, RomeUndoArgs { key: 3 })
            .expect_err("one camp is one undoable change");
        assert_eq!(refused.code, "nothing_to_undo");
        assert!(refused.message.contains("rome.camp"), "{}", refused.message);
    }

    #[test]
    fn a_bad_threat_or_site_is_a_typed_refusal() {
        let door = server(THREAT_SESSION);
        let refused = RomeCampTool::invoke(&door, camp_args(200, "heavy", 1))
            .expect_err("an unknown threat digs nothing");
        assert_eq!(refused.code, "bad_threat");
        assert!(
            refused.message.contains("one_night") && refused.message.contains("serious"),
            "the refusal names the legal values: {}",
            refused.message
        );
        assert!(
            refused.message.contains("re-run"),
            "weak models get a recovery word: {}",
            refused.message
        );
        let small = RomeCampTool::invoke(&door, camp_args(1, "light", 2))
            .expect_err("a one-metre site is no camp");
        assert_eq!(small.code, "bad_site");
        assert!(small.message.contains("re-run"), "{}", small.message);
        assert_eq!(
            session_grid(THREAT_SESSION).expect("the table reads"),
            None,
            "a refusal digs nothing"
        );
    }

    #[test]
    fn the_same_key_replays_and_two_sites_dig_the_same_grid() {
        let first = server(REPLAY_SESSION);
        let second = server(REPLAY_TWIN);
        let dug = RomeCampTool::invoke(&first, camp_args(200, "serious", 7)).expect("digs");
        let twin = RomeCampTool::invoke(&second, camp_args(200, "serious", 7)).expect("digs");
        assert_eq!(
            dug.capture.grid_after, twin.capture.grid_after,
            "the same site and threat dig the same grid on another session"
        );
        let replay = RomeCampTool::invoke(&first, camp_args(200, "serious", 7)).expect("replays");
        assert!(
            replay.replayed,
            "the same key answers again, it does not dig"
        );
        assert_eq!(replay.capture.grid_after, dug.capture.grid_after);
        assert_eq!(replay.receipt.digest, dug.receipt.digest);
        // A new key digs again from the flat site, so the answer is the same grid.
        let again = RomeCampTool::invoke(&first, camp_args(200, "serious", 8)).expect("digs");
        assert!(!again.replayed);
        assert_eq!(again.capture.grid_after, dug.capture.grid_after);
    }
}
