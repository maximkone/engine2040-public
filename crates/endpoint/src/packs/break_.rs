//! `break` pack: destruction an AI can author and test.
//!
//! Four read-only verbs over the destruction and combat crates:
//! `structure.analyze` (load paths, weakest link, predicted collapse of a
//! seeded frame building), `break.fracture` (seeded Voronoi fracture of a
//! siege wall at a hit point, with its break events), `hit.simulate` (one
//! blade state against every severable joint over many clean hits) and
//! `wound.table.get` (the shipped wound table and its digest). All
//! four take `CapabilityCard::Observe`: they compute and return numbers and
//! digests, never touching the document. Units: meters for geometry, Q16
//! (`1.0 == 65536`) for energy, loads and odds, counts for samples. Errors:
//! `bad_args` with the legal range named. Determinism: pure integer or
//! fixed-order math in the called crates, so one request gives one answer.
//!
//! DTO structs carry no doc comments on purpose: every doc line inflates the
//! discovery schemas the door budgets.

use super::bad_args as bad;
use super::{PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::tool_capabilities::CapabilityCard;
use engine_combat::limbloss::WoundTable;
use engine_combat::AnatomyTable;
use engine_destruction::bake::BREAKING_THRESHOLD;
use engine_destruction::{
    analyze, building, fracture_at, BondedCollection, BreakEvent, BuildingSpec, FractureSpec,
    PropMesh, MAX_FRACTURE_CELLS,
};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

/// Pack name (the Rust keyword forces the module file name `break_`).
pub const PACK: &str = "break";
/// Verb: load paths and the weakest link of a seeded frame building.
pub const TOOL_STRUCTURE_ANALYZE: &str = "structure.analyze";
/// Verb: runtime Voronoi fracture of a wall at a hit point.
pub const TOOL_BREAK_FRACTURE: &str = "break.fracture";
/// Verb: a blade against every severable joint over many clean hits.
pub const TOOL_HIT_SIMULATE: &str = "hit.simulate";
/// Verb: the shipped V1 wound table and its digest.
pub const TOOL_WOUND_TABLE_GET: &str = "wound.table.get";

/// Most fallen element ids echoed by `structure.analyze`.
pub const MAX_ECHOED_IDS: usize = 64;
/// Most piece boxes echoed by `break.fracture`.
pub const MAX_ECHOED_PIECES: usize = 64;

// ---------------------------------------------------------------------------
// structure.analyze
// ---------------------------------------------------------------------------

/// Frame building to analyze; every field defaults to the proof building.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde", default)]
#[schemars(crate = "rmcp::schemars")]
pub struct StructureAnalyzeArgs {
    pub bays_x: u32,
    pub bays_z: u32,
    pub stories: u32,
    pub bay_m: f32,
    pub story_m: f32,
    pub knockouts: u32,
    pub seed: u64,
    pub capacity_percent: u32,
}

impl Default for StructureAnalyzeArgs {
    fn default() -> Self {
        let spec = BuildingSpec::three_story(0);
        Self {
            bays_x: spec.bays_x,
            bays_z: spec.bays_z,
            stories: spec.stories,
            bay_m: spec.bay_m,
            story_m: spec.story_m,
            knockouts: spec.knockouts,
            seed: 0,
            capacity_percent: 125,
        }
    }
}

/// Counts, the weakest link, what falls with it and the digests.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct StructureAnalyzeOutput {
    pub elements: u32,
    pub bonds: u32,
    pub anchors: u32,
    pub standing: u32,
    pub weakest: Option<u32>,
    pub weakest_kind: Option<String>,
    pub weakest_load_q16: u64,
    pub max_load_ratio_q16: u64,
    pub falls_count: u32,
    pub falls_mass_q16: u64,
    pub falls: Vec<u32>,
    pub report_digest: String,
    pub load_digest: String,
}

fn structure_analyze(
    _server: &McpServer,
    args: &StructureAnalyzeArgs,
) -> Result<StructureAnalyzeOutput, McpToolError> {
    let pct = args.capacity_percent;
    if pct != 0 && !(100..=1000).contains(&pct) {
        return Err(bad(format!(
            "capacity_percent {pct} must be 0 (load rule off) or 100..=1000 (percent of the most loaded element's ratio)"
        )));
    }
    let spec = BuildingSpec {
        bays_x: args.bays_x,
        bays_z: args.bays_z,
        stories: args.stories,
        bay_m: args.bay_m,
        story_m: args.story_m,
        knockouts: args.knockouts,
        ..BuildingSpec::three_story(args.seed)
    };
    let built = building(&spec)
        .map_err(|e| bad(format!("{e}; bays 1..=16, stories 1..=8, knockouts 0..=8")))?;
    let mut field = BondedCollection::new(&built.desc).map_err(|e| bad(e.to_string()))?;
    if pct != 0 {
        let capacity = field.max_load_ratio_q16().saturating_mul(u64::from(pct)) / 100;
        field.set_capacity_q16(capacity);
    }
    let report = analyze(&field);
    let weakest_kind = report
        .weakest
        .and_then(|w| built.kinds.get(usize::try_from(w).unwrap_or(usize::MAX)))
        .map(|kind| kind.name().to_owned());
    Ok(StructureAnalyzeOutput {
        elements: report.chunk_count,
        bonds: report.bond_count,
        anchors: report.anchor_count,
        standing: report.supported_count,
        weakest: report.weakest,
        weakest_kind,
        weakest_load_q16: report.weakest_load_q16,
        max_load_ratio_q16: report.max_load_ratio_q16,
        falls_count: u32::try_from(report.predicted_collapse.len()).unwrap_or(u32::MAX),
        falls_mass_q16: report.predicted_mass_q16,
        falls: report
            .predicted_collapse
            .iter()
            .copied()
            .take(MAX_ECHOED_IDS)
            .collect(),
        report_digest: format!("{:016x}", engine_destruction::report_digest(&report)),
        load_digest: format!("{:016x}", report.load_digest),
    })
}

crate::e2_tool!(
    /// `structure.analyze`: load paths and weakest link of a seeded frame building.
    StructureAnalyzeTool,
    name = TOOL_STRUCTURE_ANALYZE,
    description = "Analyze a seeded frame building (bays, stories, knockouts, seed, capacity_percent): weakest link, what falls if it goes, load and report digests. Read-only.",
    readonly = true,
    args = StructureAnalyzeArgs,
    output = StructureAnalyzeOutput,
    invoke = structure_analyze
);

// ---------------------------------------------------------------------------
// break.fracture
// ---------------------------------------------------------------------------

/// Wall, hit point and fracture knobs; every field has a default.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde", default)]
#[schemars(crate = "rmcp::schemars")]
pub struct BreakFractureArgs {
    pub size_m: [f32; 3],
    pub hit: [f32; 3],
    pub cells: u32,
    pub radius_m: f32,
    pub seed: u64,
}

impl Default for BreakFractureArgs {
    fn default() -> Self {
        Self {
            size_m: [4.0, 3.0, 0.4],
            hit: [0.0, 1.5, 0.2],
            cells: 16,
            radius_m: 0.8,
            seed: 0,
        }
    }
}

/// One fracture piece: box corners and triangle count.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PieceOut {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub triangles: u32,
}

/// Piece count, struck piece, event counts, digest and piece boxes.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BreakFractureOutput {
    pub chunks: u32,
    pub hit_chunk: u32,
    pub digest: String,
    pub bond_events: u32,
    pub freed_events: u32,
    pub pieces: Vec<PieceOut>,
}

/// Closed box shell of a wall: base on the ground, centered in x and z.
fn wall_shell(size: [f32; 3]) -> PropMesh {
    let (x, y, z) = (size[0] * 0.5, size[1], size[2] * 0.5);
    PropMesh {
        positions: vec![
            [-x, 0.0, -z],
            [x, 0.0, -z],
            [x, y, -z],
            [-x, y, -z],
            [-x, 0.0, z],
            [x, 0.0, z],
            [x, y, z],
            [-x, y, z],
        ],
        indices: vec![
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 4, 7, 0, 7, 3, 1, 2, 6, 1, 6, 5, 0, 1, 5, 0, 5,
            4, 3, 7, 6, 3, 6, 2,
        ],
    }
}

fn break_fracture(
    _server: &McpServer,
    args: &BreakFractureArgs,
) -> Result<BreakFractureOutput, McpToolError> {
    for (axis, len) in args.size_m.iter().enumerate() {
        if !len.is_finite() || !(0.05..=100.0).contains(len) {
            return Err(bad(format!(
                "size_m[{axis}] {len} must be 0.05..=100 meters"
            )));
        }
    }
    let spec = FractureSpec {
        cells: args.cells,
        radius_m: args.radius_m,
        seed: args.seed,
    };
    let fracture = fracture_at(&wall_shell(args.size_m), args.hit, &spec).map_err(|e| {
        bad(format!(
            "{e}; cells 2..={MAX_FRACTURE_CELLS}, radius_m 0..=1000, hit inside the wall box (x +-size/2, y 0..size, z +-size/2)"
        ))
    })?;
    let events = fracture
        .break_events(1, 1.0, BREAKING_THRESHOLD)
        .map_err(|e| bad(e.to_string()))?;
    let freed = events
        .iter()
        .filter(|e| matches!(e, BreakEvent::ChunkFreed { .. }))
        .count();
    Ok(BreakFractureOutput {
        chunks: u32::try_from(fracture.chunks.len()).unwrap_or(u32::MAX),
        hit_chunk: fracture.hit_chunk,
        digest: format!("{:016x}", fracture.digest()),
        bond_events: u32::try_from(events.len() - freed).unwrap_or(u32::MAX),
        freed_events: u32::try_from(freed).unwrap_or(u32::MAX),
        pieces: fracture
            .chunks
            .iter()
            .take(MAX_ECHOED_PIECES)
            .map(|c| PieceOut {
                min: c.min,
                max: c.max,
                triangles: u32::try_from(c.tri_count()).unwrap_or(u32::MAX),
            })
            .collect(),
    })
}

crate::e2_tool!(
    /// `break.fracture`: seeded Voronoi fracture of a wall at a hit point.
    BreakFractureTool,
    name = TOOL_BREAK_FRACTURE,
    description = "Fracture a wall (size_m, hit, cells 2..=64, radius_m, seed) with seeded Voronoi cells at the hit: piece boxes, break event counts, digest. Same request, same bytes. Read-only.",
    readonly = true,
    args = BreakFractureArgs,
    output = BreakFractureOutput,
    invoke = break_fracture
);

// ---------------------------------------------------------------------------
// hit.simulate
// ---------------------------------------------------------------------------

/// Blade state, worn armor bits and sample count; every field has a default.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde", default)]
#[schemars(crate = "rmcp::schemars")]
pub struct HitSimulateArgs {
    pub energy_q16: u32,
    pub sharpness_q16: u32,
    pub wear_q16: u32,
    pub worn: u8,
    pub samples: u32,
    pub seed: u64,
}

impl Default for HitSimulateArgs {
    fn default() -> Self {
        Self {
            energy_q16: 7 << 16,
            sharpness_q16: 90_000,
            wear_q16: 0,
            worn: 0,
            samples: 1_000,
            seed: 0,
        }
    }
}

/// One joint: protection, sever odds, severed count and bleed-out ticks.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct HitRowOut {
    pub joint: String,
    pub protection_q16: u64,
    pub odds_q16: u32,
    pub severed: u32,
    pub per_thousand: u32,
    pub bleed_out_ticks: u32,
}

/// Samples run, digest and one row per severable joint.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct HitSimulateOutput {
    pub samples: u32,
    pub digest: String,
    pub rows: Vec<HitRowOut>,
}

/// Joint name for a canonical joint byte; the byte itself when unknown.
fn joint_name(joint: u8) -> String {
    engine_combat::JointId::from_u8(joint).map_or_else(|| joint.to_string(), |j| format!("{j:?}"))
}

fn hit_simulate(
    _server: &McpServer,
    args: &HitSimulateArgs,
) -> Result<HitSimulateOutput, McpToolError> {
    let spec = engine_combat::hitsim::HitSpec {
        energy_q16: args.energy_q16,
        sharpness_q16: args.sharpness_q16,
        wear_q16: args.wear_q16,
        worn: args.worn,
        samples: args.samples,
        seed: args.seed,
    };
    let report =
        engine_combat::hitsim::simulate(&AnatomyTable::rome_kit(), &WoundTable::v1(), &spec)
            .map_err(|e| bad(e.to_string()))?;
    let rows = report
        .rows
        .iter()
        .map(|row| HitRowOut {
            joint: joint_name(row.joint),
            protection_q16: row.protection_q16,
            odds_q16: row.odds_q16,
            severed: row.severed,
            per_thousand: u32::try_from(
                u64::from(row.severed) * 1000 / u64::from(report.samples.max(1)),
            )
            .unwrap_or(u32::MAX),
            bleed_out_ticks: row.bleed_out_ticks,
        })
        .collect();
    Ok(HitSimulateOutput {
        samples: report.samples,
        digest: format!("{:016x}", report.digest),
        rows,
    })
}

crate::e2_tool!(
    /// `hit.simulate`: one blade state against every severable joint, many clean hits.
    HitSimulateTool,
    name = TOOL_HIT_SIMULATE,
    description = "Simulate clean blade hits (energy_q16, sharpness_q16, wear_q16, worn armor bits, samples 1..=100000, seed) on every severable joint of the Rome kit: sever odds, severed counts per thousand, bleed-out ticks. Read-only.",
    readonly = true,
    args = HitSimulateArgs,
    output = HitSimulateOutput,
    invoke = hit_simulate
);

// ---------------------------------------------------------------------------
// wound.table.get
// ---------------------------------------------------------------------------

/// No arguments: the shipped table is the only one.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "serde reads the wire shape `{}`; a unit struct reads null"
)]
pub struct WoundTableGetArgs {}

/// One joint: stump class, sever shock and the wound and disable thresholds.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct WoundJointOut {
    pub joint: String,
    pub stump_class: String,
    pub sever_shock_q16: u32,
    pub wound_at_q16: u32,
    pub disable_at_q16: u32,
}

/// The V1 wound table fields in canonical order, and its digest.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct WoundTableGetOutput {
    pub version: u32,
    pub blood_full_q16: u32,
    pub bleed_out_ticks: [u32; 3],
    pub joints: Vec<WoundJointOut>,
    pub shock_decay_q16: u32,
    pub bleed_shock_q16: [u32; 3],
    pub surrender_shock_q16: u32,
    pub surrender_blood_q16: u32,
    pub faint_blood_q16: u32,
    pub digest: String,
}

fn wound_table_get(
    _server: &McpServer,
    _args: &WoundTableGetArgs,
) -> Result<WoundTableGetOutput, McpToolError> {
    let table = WoundTable::v1();
    let joints = (0..engine_combat::JOINT_TOTAL)
        .map(|i| WoundJointOut {
            joint: joint_name(u8::try_from(i).unwrap_or(u8::MAX)),
            stump_class: format!("{:?}", table.stump_class[i]),
            sever_shock_q16: table.sever_shock_q16[i],
            wound_at_q16: table.wound_at_q16[i],
            disable_at_q16: table.disable_at_q16[i],
        })
        .collect();
    Ok(WoundTableGetOutput {
        version: table.version,
        blood_full_q16: table.blood_full_q16,
        bleed_out_ticks: table.bleed_out_ticks,
        joints,
        shock_decay_q16: table.shock_decay_q16,
        bleed_shock_q16: table.bleed_shock_q16,
        surrender_shock_q16: table.surrender_shock_q16,
        surrender_blood_q16: table.surrender_blood_q16,
        faint_blood_q16: table.faint_blood_q16,
        digest: format!("{:016x}", table.digest()),
    })
}

crate::e2_tool!(
    /// `wound.table.get`: the shipped V1 wound table and its digest.
    WoundTableGetTool,
    name = TOOL_WOUND_TABLE_GET,
    description = "Read the shipped V1 wound table: blood, bleed-out ticks per class, per-joint stump class, sever shock and wound and disable thresholds, surrender lines, and its digest. Read-only.",
    readonly = true,
    args = WoundTableGetArgs,
    output = WoundTableGetOutput,
    invoke = wound_table_get
);

/// The break pack: four read-only verbs, loaded on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "destruction: load-path analysis and weakest link, runtime Voronoi fracture with break \
         events, and hit simulation over the anatomy and wound tables",
        false,
    );
    pack.add(VerbSpec::of::<StructureAnalyzeTool>(
        TOOL_STRUCTURE_ANALYZE,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<BreakFractureTool>(
        TOOL_BREAK_FRACTURE,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<HitSimulateTool>(
        TOOL_HIT_SIMULATE,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<WoundTableGetTool>(
        TOOL_WOUND_TABLE_GET,
        PACK,
        CapabilityCard::Observe,
    )?);
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;

    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    #[test]
    fn structure_analyze_names_the_weakest_link_and_replays() {
        let server = server();
        let a = StructureAnalyzeTool::invoke(&server, StructureAnalyzeArgs::default())
            .expect("analyzes");
        let b = StructureAnalyzeTool::invoke(&server, StructureAnalyzeArgs::default())
            .expect("analyzes");
        assert_eq!(a, b);
        assert_eq!(a.elements, 427);
        assert_eq!(a.anchors, 42);
        assert!(a.weakest.is_some());
        assert!(a.weakest_kind.is_some());
        assert_eq!(a.standing, a.elements, "every element stands at rest");
        assert_eq!(a.report_digest.len(), 16);
        let other = StructureAnalyzeTool::invoke(
            &server,
            StructureAnalyzeArgs {
                seed: 2,
                ..StructureAnalyzeArgs::default()
            },
        )
        .expect("analyzes");
        assert_ne!(a.load_digest, other.load_digest);
    }

    #[test]
    fn structure_analyze_refuses_bad_specs_with_the_legal_range() {
        let server = server();
        for bad_args in [
            StructureAnalyzeArgs {
                capacity_percent: 50,
                ..StructureAnalyzeArgs::default()
            },
            StructureAnalyzeArgs {
                bays_x: 0,
                ..StructureAnalyzeArgs::default()
            },
            StructureAnalyzeArgs {
                stories: 99,
                ..StructureAnalyzeArgs::default()
            },
        ] {
            let error = StructureAnalyzeTool::invoke(&server, bad_args).expect_err("refuses");
            assert_eq!(error.code, "bad_args");
            assert!(error.message.contains("..="), "{}", error.message);
        }
    }

    #[test]
    fn break_fracture_is_deterministic_capped_and_refuses_bad_input() {
        let server = server();
        let a =
            BreakFractureTool::invoke(&server, BreakFractureArgs::default()).expect("fractures");
        let b =
            BreakFractureTool::invoke(&server, BreakFractureArgs::default()).expect("fractures");
        assert_eq!(a, b);
        assert!(a.chunks >= 2 && a.chunks <= 16);
        assert!(a.freed_events >= 1 && a.bond_events >= 1);
        assert_eq!(a.pieces.len(), usize::try_from(a.chunks).expect("small"));
        let moved = BreakFractureTool::invoke(
            &server,
            BreakFractureArgs {
                hit: [-1.0, 2.0, 0.2],
                ..BreakFractureArgs::default()
            },
        )
        .expect("fractures");
        assert_ne!(a.digest, moved.digest);
        for bad_args in [
            BreakFractureArgs {
                cells: 1,
                ..BreakFractureArgs::default()
            },
            BreakFractureArgs {
                cells: 65,
                ..BreakFractureArgs::default()
            },
            BreakFractureArgs {
                hit: [50.0, 0.0, 0.0],
                ..BreakFractureArgs::default()
            },
            BreakFractureArgs {
                size_m: [0.0, 3.0, 0.4],
                ..BreakFractureArgs::default()
            },
        ] {
            let error = BreakFractureTool::invoke(&server, bad_args).expect_err("refuses");
            assert_eq!(error.code, "bad_args");
        }
    }

    #[test]
    fn hit_simulate_reports_odds_counts_and_bleed_out() {
        let server = server();
        let a = HitSimulateTool::invoke(&server, HitSimulateArgs::default()).expect("simulates");
        let b = HitSimulateTool::invoke(&server, HitSimulateArgs::default()).expect("simulates");
        assert_eq!(a, b);
        assert_eq!(a.rows.len(), 15);
        assert!(a
            .rows
            .iter()
            .any(|r| r.joint == "Neck" && r.bleed_out_ticks == 1_200));
        let armored = HitSimulateTool::invoke(
            &server,
            HitSimulateArgs {
                worn: 0x1F,
                ..HitSimulateArgs::default()
            },
        )
        .expect("simulates");
        let bare_cut: u32 = a.rows.iter().map(|r| r.severed).sum();
        let worn_cut: u32 = armored.rows.iter().map(|r| r.severed).sum();
        assert!(
            worn_cut <= bare_cut,
            "armor never adds severs: {worn_cut} vs {bare_cut}"
        );
        let error = HitSimulateTool::invoke(
            &server,
            HitSimulateArgs {
                samples: 0,
                ..HitSimulateArgs::default()
            },
        )
        .expect_err("refuses");
        assert_eq!(error.code, "bad_args");
    }

    #[test]
    fn wound_table_get_returns_v1_and_its_digest() {
        let server = server();
        let a = WoundTableGetTool::invoke(&server, WoundTableGetArgs {}).expect("reads");
        let b = WoundTableGetTool::invoke(&server, WoundTableGetArgs {}).expect("reads");
        assert_eq!(a, b);
        assert_eq!(a.bleed_out_ticks, [18_000, 5_400, 1_200]);
        assert_eq!(a.digest, format!("{:016x}", WoundTable::v1().digest()));
        assert_eq!(a.version, WoundTable::v1().version);
        assert_eq!(a.joints.len(), engine_combat::JOINT_TOTAL);
        let neck = a
            .joints
            .iter()
            .find(|j| j.joint == "Neck")
            .expect("neck row");
        assert_eq!(neck.stump_class, "Arterial");
        assert_eq!(neck.sever_shock_q16, 80 << 16);
        assert!(neck.wound_at_q16 < neck.disable_at_q16);
    }

    #[test]
    fn the_pack_registers_four_observe_verbs() {
        let pack = pack().expect("pack builds");
        assert_eq!(
            pack.verb_names(),
            vec![
                TOOL_STRUCTURE_ANALYZE,
                TOOL_BREAK_FRACTURE,
                TOOL_HIT_SIMULATE,
                TOOL_WOUND_TABLE_GET
            ]
        );
        for verb in pack.verbs() {
            assert_eq!(verb.card(), CapabilityCard::Observe);
        }
    }
}
