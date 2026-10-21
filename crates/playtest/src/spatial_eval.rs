//! Spatial gold eval: 100 place/query/fix tasks with gold answers.
//!
//! Consumer: the proof harness resolves [`SPATIAL_GATE_NAME`] and calls
//! [`proof_gate`]; later authoring stages read the gold task shapes as input. Data owner:
//! this module owns tasks, scores and reports; `engine-spatial` owns truth
//! (its integer-cm oracles below are the fixture-grade reference the live
//! `scene.query`/`sample`/`solve` path must agree with, never a second
//! solver). Units: integer centimeters, octant yaws, FNV-1a digests.
//! Determinism: same fixture files twice give the identical report; no wall
//! clock, thread timing or hash-map order enters a score. Errors are typed
//! [`SpatialError`]; vision notes stay advisory and never gate a score.

use std::fmt;

use engine_core::Fnv64;
use engine_spatial::fuzz;

use crate::eval::mix_text;
use crate::parse::{kv_expect_keys, kv_get, split_kv, ParseError};

// ---- constants -------------------------------------------------------------

/// Fixture format version this runner accepts.
pub const SPEC_VERSION: u16 = 1;
/// Tasks shipped in `fixtures/spatial-gold/tasks.tsv`.
pub const TASK_COUNT: usize = 100;
/// Place tasks among the hundred.
pub const PLACE_COUNT: usize = 30;
/// Query tasks among the hundred.
pub const QUERY_COUNT: usize = 40;
/// Fix tasks among the hundred.
pub const FIX_COUNT: usize = 30;
/// Between-corridor tolerance in cm, mirroring the live query path.
pub const BETWEEN_TOL_CM: i64 = 50;
/// Largest-open scan pitch in cm, mirroring the live sampler.
pub const OPEN_SCAN_CM: i32 = 25;
/// Repair search pitch in cm.
pub const REPAIR_STEP_CM: i32 = 10;
/// Repair search radius in cm.
pub const REPAIR_RADIUS_CM: i32 = 400;
/// Most ids one gold answer may carry.
pub const MAX_IDS: usize = 64;
/// Largest accepted fixture file in bytes.
pub const MAX_TSV_BYTES: usize = 64 * 1024;
/// Coordinate bound in cm for every fixture value.
pub const MAX_COORD_CM: i32 = 100_000;
/// Radius bound in cm, mirroring the spatial world bound.
pub const MAX_RADIUS_CM: i32 = 10_000;
/// Report format version stamped on every eval report.
pub const REPORT_VERSION: u16 = 1;
/// Name under which proof invokes this gate. The orchestrator wires
/// `tools/proof.mjs` to `engine_playtest::spatial_eval::proof_gate` under
/// this key; the key lives here (playtest side) so the gate keeps its name
/// even before the proof crate grows the dependency edge.
pub const SPATIAL_GATE_NAME: &str = "sp12-spatial-gold-eval";
/// Digest tag so eval bytes never alias another domain hash.
const EVAL_TAG: &[u8] = b"e2sp12:1";

// ---- errors ----------------------------------------------------------------

/// What stopped a spatial-eval step. Every message names the legal values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpatialError {
    /// Fixture directory unreadable.
    DirUnreadable(String),
    /// One fixture file refused; carries the file name and reason.
    BadFixture {
        /// File name of the offending fixture.
        file: String,
        /// What was wrong with it.
        reason: String,
    },
    /// Unknown task id; the report lists the legal ids.
    BadTask {
        /// Requested id.
        id: String,
    },
    /// Answer count mismatches the task count.
    AnswerCount {
        /// Tasks scored.
        want: usize,
        /// Answers given.
        got: usize,
    },
}

impl SpatialError {
    /// Machine code for this failure.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::DirUnreadable(_) => "dir_unreadable",
            Self::BadFixture { .. } => "bad_fixture",
            Self::BadTask { .. } => "bad_task",
            Self::AnswerCount { .. } => "answer_count",
        }
    }
}

impl fmt::Display for SpatialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DirUnreadable(reason) => write!(f, "spatial-gold dir unreadable: {reason}"),
            Self::BadFixture { file, reason } => {
                write!(f, "bad spatial-gold fixture `{file}`: {reason}")
            }
            Self::BadTask { id } => write!(
                f,
                "bad_task: task `{id}` is unknown; run the sp12-001..sp12-100 ids"
            ),
            Self::AnswerCount { want, got } => write!(
                f,
                "answer_count: {got} answers for {want} tasks; answer every task in order"
            ),
        }
    }
}

impl std::error::Error for SpatialError {}

mod world;
use world::*;
#[cfg(test)]
mod known_good;
pub mod llm;
/// SPR4 live recorder: one real model over the eval-room door tools.
#[cfg(feature = "live")]
pub mod llm_live;
mod tsv;
pub use tsv::*;

// ---- task model --------------------------------------------------------------

/// Task family: place proposes, query asks, fix repairs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind {
    Place,
    Query,
    Fix,
}

impl TaskKind {
    /// Stable lowercase label, also the report key.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Place => "place",
            Self::Query => "query",
            Self::Fix => "fix",
        }
    }

    /// Parse a kind word; unknown words refuse with the legal list.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        match text {
            "place" => Ok(Self::Place),
            "query" => Ok(Self::Query),
            "fix" => Ok(Self::Fix),
            _ => Err(ParseError::Unknown {
                what: format!("kind `{text}` is unknown; legal kinds: place, query, fix"),
            }),
        }
    }
}

/// Query word: the seven `scene.query` questions the door serves. Discriminants
/// are frozen: fixtures persist them as words, so variants are append-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryWord {
    Near,
    Between,
    Visible,
    Reachable,
    CoverNear,
    LargestOpen,
    FreeSpace,
}

impl QueryWord {
    /// Door word and live scene-query API of this question, in one table.
    const fn words(self) -> (&'static str, &'static str) {
        match self {
            Self::Near => ("near", "scene_query::run_query near"),
            Self::Between => ("between", "scene_query::run_query between"),
            Self::Visible => ("visible", "scene_query::run_query visible"),
            Self::Reachable => ("reachable", "scene_query::run_query reachable"),
            Self::CoverNear => ("cover_near", "scene_query::run_query cover_near"),
            Self::LargestOpen => ("largest_open", "sample::largest_open_spot"),
            Self::FreeSpace => ("free_space", "sample::sample_free_space"),
        }
    }

    /// Door word for this question, mirroring `scene.query`.
    pub const fn as_str(self) -> &'static str {
        self.words().0
    }

    /// Parse a query word; unknown words refuse with the legal list.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        match text {
            "near" => Ok(Self::Near),
            "between" => Ok(Self::Between),
            "visible" => Ok(Self::Visible),
            "reachable" => Ok(Self::Reachable),
            "cover_near" => Ok(Self::CoverNear),
            "largest_open" => Ok(Self::LargestOpen),
            "free_space" => Ok(Self::FreeSpace),
            _ => Err(ParseError::Unknown {
                what: format!(
                    "query `{text}` is unknown; legal words: near, between, visible, \
                     reachable, cover_near, largest_open, free_space"
                ),
            }),
        }
    }

    /// Live public API this word exercises, for the mapping table.
    pub const fn live_api(self) -> &'static str {
        self.words().1
    }
}

/// Task parameters: every value is an integer (cm, ids, counts, seeds), so
/// fixtures never pass through a float.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Params {
    /// Proposed footprint center plus half extents in cm.
    Place {
        x_cm: i32,
        z_cm: i32,
        hx_cm: i32,
        hz_cm: i32,
    },
    /// Anchor actor plus radius in cm.
    Near { anchor: u32, radius_cm: i32 },
    /// Corridor endpoints as actor ids.
    Between { a: u32, b: u32 },
    /// Eye in cm plus octant yaw.
    Visible { ex_cm: i32, ez_cm: i32, yaw_oct: u8 },
    /// Reachability anchor as actor id.
    Reachable { anchor: u32 },
    /// Cover seeker, threat eye in cm, stance and radius in cm.
    CoverNear {
        anchor: u32,
        tx_cm: i32,
        tz_cm: i32,
        crouch: bool,
        radius_cm: i32,
    },
    /// Capsule clearance in cm.
    LargestOpen { capsule_cm: i32 },
    /// Capsule, gap, count and seed.
    FreeSpace {
        capsule_cm: i32,
        gap_cm: i32,
        max_n: usize,
        seed: u64,
    },
}

/// Gold answer: exact ids, verdicts and hints for geometric tasks;
/// property bounds for sampler tasks (mirroring the live acceptance bar:
/// 0 overlaps, 0 floating, 100% reachable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gold {
    /// Placement verdict plus ascending blocker ids.
    Verdict { clean: bool, conflicts: Vec<u32> },
    /// Exact expected id set, ascending.
    Ids(Vec<u32>),
    /// Repair vector plus the fixed point, which verifies clean.
    Repair {
        dx_cm: i32,
        dz_cm: i32,
        x_cm: i32,
        z_cm: i32,
    },
    /// Sampler property: emptiness plus the minimum clearance in cm every
    /// returned sample must carry (`expect_empty` tasks must answer empty).
    Samples {
        expect_empty: bool,
        min_clear_cm: i32,
    },
}

/// Vision cross-check note: advisory only, never scored. `capture` cites
/// the deterministic pseudo-capture stamp; the live feed arrives from
/// `play.step` captures once the orchestrator wires that edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vision {
    /// Advisory note; always starts with `advisory:`.
    pub note: String,
    /// Pseudo-capture digest in hex.
    pub capture: String,
}

/// One gold task: stable id, family, scene, MCP door, integer params, the
/// gold answer and the advisory vision note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    /// Stable id (`sp12-001`..`sp12-100`).
    pub id: String,
    /// Task family.
    pub kind: TaskKind,
    /// Scene name (`room_pillar`, `room_open`, `room_tight`).
    pub scene: String,
    /// MCP door this task runs through (`scene.query near`, `scene.place`,
    /// `tx.preview`).
    pub tool: String,
    /// Integer parameters.
    pub params: Params,
    /// Gold answer.
    pub gold: Gold,
    /// Advisory vision cross-check.
    pub vision: Vision,
}

impl Task {
    /// Query word for query tasks; `None` for place and fix rows.
    pub fn word(&self) -> Option<QueryWord> {
        match &self.params {
            Params::Place { .. } => None,
            Params::Near { .. } => Some(QueryWord::Near),
            Params::Between { .. } => Some(QueryWord::Between),
            Params::Visible { .. } => Some(QueryWord::Visible),
            Params::Reachable { .. } => Some(QueryWord::Reachable),
            Params::CoverNear { .. } => Some(QueryWord::CoverNear),
            Params::LargestOpen { .. } => Some(QueryWord::LargestOpen),
            Params::FreeSpace { .. } => Some(QueryWord::FreeSpace),
        }
    }
}

// ---- generator (the frozen hundred) --------------------------------------------

fn vision_for(id: &str, task: &Task) -> Vision {
    let note = match &task.params {
        Params::Visible { .. } => "advisory:confirm-occluder-in-capture",
        Params::CoverNear { .. } => "advisory:confirm-threat-sightline",
        Params::Place { .. } | Params::FreeSpace { .. } | Params::LargestOpen { .. } => {
            "advisory:confirm-rest-contact"
        }
        Params::Near { .. } | Params::Between { .. } | Params::Reachable { .. } => "advisory:agree",
    };
    Vision {
        note: note.to_owned(),
        capture: capture_hex(id),
    }
}

/// Recompute the oracle gold for one task from first principles. Returns
/// `None` only when the scene name is unknown. Place, query and sampler
/// rows share this single source: the generator and the runtime verifier
/// below both call it, so fixture gold can never drift from the oracle.
pub fn expected_gold(task: &Task) -> Option<Gold> {
    let scene = scene_of(&task.scene)?;
    match &task.params {
        Params::Place {
            x_cm,
            z_cm,
            hx_cm,
            hz_cm,
        } => {
            let (clean, conflicts) = place_verdict(scene, *x_cm, *z_cm, *hx_cm, *hz_cm);
            Some(Gold::Verdict { clean, conflicts })
        }
        Params::Near { anchor, radius_cm } => Some(Gold::Ids(near_ids(scene, *anchor, *radius_cm))),
        Params::Between { a, b } => Some(Gold::Ids(between_ids(scene, *a, *b))),
        Params::Visible {
            ex_cm,
            ez_cm,
            yaw_oct,
        } => Some(Gold::Ids(matched_ids(&visible_rows(
            scene,
            (*ex_cm, *ez_cm),
            *yaw_oct,
        )))),
        Params::Reachable { anchor } => {
            Some(Gold::Ids(matched_ids(&reachable_rows(scene, *anchor))))
        }
        Params::CoverNear {
            anchor,
            tx_cm,
            tz_cm,
            crouch,
            radius_cm,
        } => Some(Gold::Ids(cover_matched(&cover_rows(
            scene,
            *anchor,
            (*tx_cm, *tz_cm),
            *crouch,
            *radius_cm,
        )))),
        Params::LargestOpen { capsule_cm } | Params::FreeSpace { capsule_cm, .. } => {
            match largest_open(scene, 40, 40, *capsule_cm) {
                Some(_) => Some(Gold::Samples {
                    expect_empty: false,
                    min_clear_cm: *capsule_cm,
                }),
                None => Some(Gold::Samples {
                    expect_empty: true,
                    min_clear_cm: 0,
                }),
            }
        }
    }
}

/// Recompute the repair gold for one fix task: the deterministic hint the
/// oracle scan finds, or the zero vector at the proposal when nothing in
/// range verifies clean.
fn expected_repair(task: &Task) -> Option<Gold> {
    let scene = scene_of(&task.scene)?;
    let Params::Place {
        x_cm,
        z_cm,
        hx_cm,
        hz_cm,
    } = &task.params
    else {
        return None;
    };
    Some(match repair_hint(scene, *x_cm, *z_cm, *hx_cm, *hz_cm) {
        Some((dx, dz, fx, fz)) => Gold::Repair {
            dx_cm: dx,
            dz_cm: dz,
            x_cm: fx,
            z_cm: fz,
        },
        None => Gold::Repair {
            dx_cm: 0,
            dz_cm: 0,
            x_cm: *x_cm,
            z_cm: *z_cm,
        },
    })
}

/// Verify loaded fixture gold against the oracle, task by task. The proof
/// gate runs this before scoring, so a hand-edited TSV that drifts from
/// first principles fails loudly instead of scoring fiction.
pub fn verify_gold(tasks: &[Task]) -> Result<(), SpatialError> {
    for task in tasks {
        let recomputed = match task.kind {
            TaskKind::Fix => expected_repair(task),
            TaskKind::Place | TaskKind::Query => expected_gold(task),
        };
        if recomputed.as_ref() != Some(&task.gold) {
            return Err(SpatialError::BadFixture {
                file: "tasks.tsv".to_owned(),
                reason: format!(
                    "gold for `{}` drifts from the oracle; regenerate the fixture",
                    task.id
                ),
            });
        }
    }
    Ok(())
}

fn place_task(n: usize, scene: &Scene, x_cm: i32, z_cm: i32) -> Task {
    let id = format!("sp12-{n:03}");
    let params = Params::Place {
        x_cm,
        z_cm,
        hx_cm: 40,
        hz_cm: 40,
    };
    let mut task = Task {
        id: id.clone(),
        kind: TaskKind::Place,
        scene: scene.name.to_owned(),
        tool: "scene.place".to_owned(),
        params,
        gold: Gold::Ids(Vec::new()),
        vision: Vision {
            note: String::new(),
            capture: String::new(),
        },
    };
    // The scene name above comes from the scene table itself, so the
    // oracle always resolves it; anything else is a code bug, not input.
    task.gold = expected_gold(&task).expect("place gold resolves its own scene");
    task.vision = vision_for(&id, &task);
    task
}

fn fix_task(n: usize, scene: &Scene, x_cm: i32, z_cm: i32) -> Task {
    let id = format!("sp12-{n:03}");
    let params = Params::Place {
        x_cm,
        z_cm,
        hx_cm: 40,
        hz_cm: 40,
    };
    let mut task = Task {
        id: id.clone(),
        kind: TaskKind::Fix,
        scene: scene.name.to_owned(),
        tool: "tx.preview".to_owned(),
        params,
        gold: Gold::Ids(Vec::new()),
        vision: Vision {
            note: String::new(),
            capture: String::new(),
        },
    };
    task.gold = expected_repair(&task).expect("fix gold resolves its own scene");
    task.vision = vision_for(&id, &task);
    task
}

fn query_task(n: usize, scene: &Scene, params: Params) -> Task {
    let id = format!("sp12-{n:03}");
    let word = match &params {
        Params::Near { .. } => QueryWord::Near,
        Params::Between { .. } => QueryWord::Between,
        Params::Visible { .. } => QueryWord::Visible,
        Params::Reachable { .. } => QueryWord::Reachable,
        Params::CoverNear { .. } => QueryWord::CoverNear,
        Params::LargestOpen { .. } => QueryWord::LargestOpen,
        Params::FreeSpace { .. } => QueryWord::FreeSpace,
        Params::Place { .. } => QueryWord::Near,
    };
    let mut task = Task {
        id: id.clone(),
        kind: TaskKind::Query,
        scene: scene.name.to_owned(),
        tool: format!("scene.query {}", word.as_str()),
        params,
        gold: Gold::Ids(Vec::new()),
        vision: Vision {
            note: String::new(),
            capture: String::new(),
        },
    };
    // Same proven-scene expect as the place and fix builders: the name
    // comes from the scene table, so the oracle always resolves it.
    task.gold = expected_gold(&task).expect("query gold resolves its own scene");
    task.vision = vision_for(&id, &task);
    task
}

fn matched_ids(rows: &[Seen]) -> Vec<u32> {
    rows.iter()
        .filter(|s| s.reason == "visible" || s.reason == "reachable")
        .map(|s| s.id)
        .collect()
}

fn cover_matched(rows: &[Seen]) -> Vec<u32> {
    rows.iter()
        .filter(|s| s.reason != "cover:none")
        .map(|s| s.id)
        .collect()
}

/// Build the frozen hundred: 30 place, 40 query, 30 fix. Deterministic in
/// code alone; the checked-in TSV must match this builder byte for byte.
/// The release tooling calls this to regenerate the gold set; the proof
/// gate never does (it loads and verifies the checked-in file instead).
pub fn generate_tasks() -> Vec<Task> {
    let rooms = gold_rooms();
    let mut tasks: Vec<Task> = Vec::with_capacity(TASK_COUNT);
    push_place_tasks(&rooms, &mut tasks);
    push_query_tasks(&rooms, &mut tasks);
    push_fix_tasks(&rooms, &mut tasks);
    tasks
}

/// The three gold rooms in rotation order: pillar, open, tight.
fn gold_rooms() -> [&'static Scene; 3] {
    [
        scene_of("room_pillar").expect("pillar scene exists"),
        scene_of("room_open").expect("open scene exists"),
        scene_of("room_tight").expect("tight scene exists"),
    ]
}

/// Place rows sp12-001..030: indexed sweep points rotate rooms, so gold
/// mixes clean placements with conflicts that name their blockers.
fn push_place_tasks(rooms: &[&'static Scene; 3], tasks: &mut Vec<Task>) {
    for i in 0..PLACE_COUNT {
        let scene = rooms[i % 3];
        let x_cm = -450 + ((i * 137) % 900) as i32;
        let z_cm = -450 + ((i * 251) % 900) as i32;
        tasks.push(place_task(i + 1, scene, x_cm, z_cm));
    }
}

/// Query rows sp12-031..070: every query word covered. Gold always comes
/// from the oracle via query_task, never hand-written.
fn push_query_tasks(rooms: &[&'static Scene; 3], tasks: &mut Vec<Task>) {
    let [pillar, open, tight] = *rooms;
    let mut n = PLACE_COUNT;
    let mut push = |scene: &Scene, params: Params| {
        n += 1;
        tasks.push(query_task(n, scene, params));
    };
    // Near x10 over the pillar room.
    for (anchor, radius) in [21, 20, 22, 21, 20, 22, 21, 20, 21, 22]
        .into_iter()
        .zip([150, 250, 450, 500, 2000, 300, 450, 450, 120, 900])
    {
        push(
            pillar,
            Params::Near {
                anchor,
                radius_cm: radius,
            },
        );
    }
    // Between x8: seven pillar corridors plus one open-room corridor.
    // Every pair carries a non-empty gold set, so the seeded relation bug
    // (which blinds `between`) drops exactly these eight cases.
    for (scene, a, b) in [
        (pillar, 20, 22),
        (pillar, 21, 22),
        (pillar, 24, 22),
        (pillar, 23, 22),
        (pillar, 24, 21),
        (pillar, 22, 23),
        (pillar, 20, 22),
        (open, 30, 32),
    ] {
        push(scene, Params::Between { a, b });
    }
    // Visible x8: facing the room, turned away, and cone-edge bands.
    for (ex, ez, oct) in [
        (-300, 0, 0),
        (-300, 0, 4),
        (0, -300, 2),
        (0, 300, 6),
        (200, -300, 2),
        (-300, 200, 0),
        (400, 400, 5),
        (0, 0, 0),
    ] {
        push(
            pillar,
            Params::Visible {
                ex_cm: ex,
                ez_cm: ez,
                yaw_oct: oct,
            },
        );
    }
    push_area_queries(pillar, open, tight, &mut push);
}

/// Reachable, cover and sampler query rows: the second half of the query
/// block, numbered by the same `push` closure so ids stay contiguous.
fn push_area_queries(
    pillar: &Scene,
    open: &Scene,
    tight: &Scene,
    push: &mut impl FnMut(&Scene, Params),
) {
    // Reachable x6: home poly, island poly, off-mesh actor, open chain.
    for (scene, anchor) in [
        (pillar, 20),
        (pillar, 22),
        (pillar, 27),
        (open, 30),
        (open, 33),
        (tight, 40),
    ] {
        push(scene, Params::Reachable { anchor });
    }
    // Cover-near x4: stand/crouch pairs so crouch sets only ever grow.
    for (scene, anchor, tx, tz, crouch) in [
        (pillar, 20, 1000, 0, false),
        (pillar, 20, 1000, 0, true),
        (open, 30, 1000, 1000, false),
        (pillar, 21, -1000, 0, true),
    ] {
        push(
            scene,
            Params::CoverNear {
                anchor,
                tx_cm: tx,
                tz_cm: tz,
                crouch,
                radius_cm: 3000,
            },
        );
    }
    // Largest-open x2: roomy capsule plus an impossible one. Sampler gold
    // is property-shaped on purpose (see Gold::Samples docs).
    push(pillar, Params::LargestOpen { capsule_cm: 30 });
    push(pillar, Params::LargestOpen { capsule_cm: 9000 });
    // Free-space x2: open room property plus an impossible capsule.
    push(
        pillar,
        Params::FreeSpace {
            capsule_cm: 30,
            gap_cm: 100,
            max_n: 8,
            seed: 7,
        },
    );
    push(
        tight,
        Params::FreeSpace {
            capsule_cm: 9500,
            gap_cm: 100,
            max_n: 8,
            seed: 7,
        },
    );
}

/// Fix rows sp12-071..100: conflicted proposals (an in-room actor center
/// nudged inside its own footprint, so a repair always sits inside the
/// search radius) with repair hints that verify clean.
fn push_fix_tasks(rooms: &[&'static Scene; 3], tasks: &mut Vec<Task>) {
    for i in 0..FIX_COUNT {
        let scene = rooms[i % 3];
        let inside: Vec<&Actor> = scene
            .actors
            .iter()
            .filter(|a| a.x_cm.abs() <= 400 && a.z_cm.abs() <= 400)
            .collect();
        let actor = inside[(i + 4) % inside.len()];
        let x_cm = actor.x_cm + 20 + ((i % 3) as i32) * 10;
        let z_cm = actor.z_cm - 10;
        tasks.push(fix_task(
            PLACE_COUNT + QUERY_COUNT + i + 1,
            scene,
            x_cm,
            z_cm,
        ));
    }
}

// ---- fixtures ------------------------------------------------------------------

/// Directory holding the spatial-gold fixtures, resolved from the crate
/// location so the suite runs from any working directory.
pub fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/spatial-gold")
}

/// Load every gold task from `fixtures/spatial-gold/tasks.tsv`.
pub fn load_tasks() -> Result<Vec<Task>, SpatialError> {
    let path = fixture_dir().join("tasks.tsv");
    let bytes = std::fs::read(&path).map_err(|error| SpatialError::BadFixture {
        file: "tasks.tsv".to_owned(),
        reason: error.to_string(),
    })?;
    if bytes.len() > MAX_TSV_BYTES {
        return Err(SpatialError::BadFixture {
            file: "tasks.tsv".to_owned(),
            reason: format!("file over {MAX_TSV_BYTES} bytes"),
        });
    }
    let text = String::from_utf8(bytes).map_err(|error| SpatialError::BadFixture {
        file: "tasks.tsv".to_owned(),
        reason: error.to_string(),
    })?;
    parse_tsv(&text).map_err(|error| SpatialError::BadFixture {
        file: "tasks.tsv".to_owned(),
        reason: error.to_string(),
    })
}

/// One task by id, for callers that address tasks by name.
pub fn task_by_id(tasks: &[Task], id: &str) -> Result<Task, SpatialError> {
    tasks
        .iter()
        .find(|task| task.id == id)
        .cloned()
        .ok_or_else(|| SpatialError::BadTask { id: id.to_owned() })
}

// ---- agents ----------------------------------------------------------------------

/// A fresh agent answer: verdicts and id sets for geometric tasks, points
/// for sampler tasks, vectors for repairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentAnswer {
    /// Placement verdict plus blocker ids.
    Verdict { clean: bool, conflicts: Vec<u32> },
    /// Answered id set, ascending.
    Ids(Vec<u32>),
    /// Sample points in cm.
    Samples(Vec<(i32, i32)>),
    /// Repair vector plus the fixed point in cm.
    Repair {
        dx_cm: i32,
        dz_cm: i32,
        x_cm: i32,
        z_cm: i32,
    },
}

/// A fresh agent working through MCP: one answer per task, in order.
/// Models the eval loop (brief, then `scene.query` / `scene.place` /
/// `tx.preview` calls); providers stay out so the score never waits on a
/// network.
pub trait SpatialAgent {
    /// Answer one task; the harness scores it against gold.
    fn answer(&self, task: &Task) -> AgentAnswer;
}

/// The competent fresh agent: reads the brief, runs the door tools and
/// answers from gold (geometric rows) or from valid placements (sampler
/// rows, which score on properties like the live acceptance bar).
pub struct GoldAgent;

impl SpatialAgent for GoldAgent {
    fn answer(&self, task: &Task) -> AgentAnswer {
        match &task.gold {
            Gold::Verdict { clean, conflicts } => AgentAnswer::Verdict {
                clean: *clean,
                conflicts: conflicts.clone(),
            },
            Gold::Ids(ids) => AgentAnswer::Ids(ids.clone()),
            Gold::Repair {
                dx_cm,
                dz_cm,
                x_cm,
                z_cm,
            } => AgentAnswer::Repair {
                dx_cm: *dx_cm,
                dz_cm: *dz_cm,
                x_cm: *x_cm,
                z_cm: *z_cm,
            },
            Gold::Samples { expect_empty, .. } => {
                if *expect_empty {
                    return AgentAnswer::Samples(Vec::new());
                }
                let Some(scene) = scene_of(&task.scene) else {
                    return AgentAnswer::Samples(Vec::new());
                };
                let capsule = match &task.params {
                    Params::LargestOpen { capsule_cm } => *capsule_cm,
                    Params::FreeSpace { capsule_cm, .. } => *capsule_cm,
                    _ => return AgentAnswer::Samples(Vec::new()),
                };
                match largest_open(scene, 40, 40, capsule) {
                    Some((x, z, _)) => AgentAnswer::Samples(vec![(x, z)]),
                    None => AgentAnswer::Samples(Vec::new()),
                }
            }
        }
    }
}

/// Seeded relation bug: blinds one query word (the relation under test
/// answers empty) and flips every other verdict it touches. The acceptance
/// bug blinds `between`, so exactly the corridor tasks drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeededBug {
    /// Query word this bug blinds.
    pub blind: QueryWord,
}

/// The seeded relation bug the acceptance gate drops exactly.
pub const RELATION_BUG: SeededBug = SeededBug {
    blind: QueryWord::Between,
};

/// A fresh agent running the seeded buggy build: gold everywhere except
/// the blinded word, which answers empty.
pub struct SeededBugAgent {
    /// Bug under test.
    pub bug: SeededBug,
}

impl SpatialAgent for SeededBugAgent {
    fn answer(&self, task: &Task) -> AgentAnswer {
        if task.word() == Some(self.bug.blind) {
            return match &task.gold {
                Gold::Ids(_) | Gold::Samples { .. } => AgentAnswer::Ids(Vec::new()),
                Gold::Verdict { .. } => AgentAnswer::Verdict {
                    clean: false,
                    conflicts: Vec::new(),
                },
                Gold::Repair { x_cm, z_cm, .. } => AgentAnswer::Repair {
                    dx_cm: 0,
                    dz_cm: 0,
                    x_cm: *x_cm,
                    z_cm: *z_cm,
                },
            };
        }
        GoldAgent.answer(task)
    }
}

impl SeededBug {
    /// True when this bug corrupts the task (it names the blinded word).
    pub const fn affects(&self, word: Option<QueryWord>) -> bool {
        match (word, self.blind) {
            (Some(a), b) => a as u8 == b as u8,
            (None, _) => false,
        }
    }
}

// ---- scoring -----------------------------------------------------------------------

/// Score one answer against gold. Sampler rows check properties (in-bounds,
/// capsule clearance, gap, count bound); every other row needs exact gold.
fn score_answer(task: &Task, answer: &AgentAnswer) -> bool {
    match (&task.gold, answer) {
        (
            Gold::Verdict { clean, conflicts },
            AgentAnswer::Verdict {
                clean: got,
                conflicts: got_ids,
            },
        ) => clean == got && conflicts == got_ids,
        (Gold::Ids(want), AgentAnswer::Ids(got)) => want == got,
        (
            Gold::Repair {
                dx_cm,
                dz_cm,
                x_cm,
                z_cm,
            },
            AgentAnswer::Repair {
                dx_cm: a,
                dz_cm: b,
                x_cm: c,
                z_cm: d,
            },
        ) => dx_cm == a && dz_cm == b && x_cm == c && z_cm == d,
        (
            Gold::Samples {
                expect_empty,
                min_clear_cm,
            },
            AgentAnswer::Samples(points),
        ) => {
            let Some(scene) = scene_of(&task.scene) else {
                return false;
            };
            let (capsule, gap, max_n) = match &task.params {
                Params::LargestOpen { capsule_cm } => (*capsule_cm, 1, 1),
                Params::FreeSpace {
                    capsule_cm,
                    gap_cm,
                    max_n,
                    ..
                } => (*capsule_cm, *gap_cm, *max_n),
                _ => return false,
            };
            if *expect_empty {
                return points.is_empty();
            }
            if points.is_empty() || points.len() > max_n {
                return false;
            }
            let rects: Vec<Rect> = scene.actors.iter().map(Actor::rect).collect();
            for (x, z) in points {
                let cell = Rect {
                    min_x: x - 40,
                    max_x: x + 40,
                    min_z: z - 40,
                    max_z: z + 40,
                };
                if !fits(scene, *x, *z, 40, 40, capsule) {
                    return false;
                }
                if edge_gap(&cell, &rects) < *min_clear_cm {
                    return false;
                }
            }
            for (i, a) in points.iter().enumerate() {
                for b in &points[i + 1..] {
                    if dist_cm(*a, *b) < i64::from(gap) {
                        return false;
                    }
                }
            }
            true
        }
        _ => false,
    }
}

/// Score aligned answers (same order as tasks) into a report.
pub fn score(tasks: &[Task], answers: &[AgentAnswer]) -> Result<EvalReport, SpatialError> {
    if answers.len() != tasks.len() {
        return Err(SpatialError::AnswerCount {
            want: tasks.len(),
            got: answers.len(),
        });
    }
    let mut passed = 0;
    let mut failed: Vec<String> = Vec::new();
    let mut place = (0, 0);
    let mut query = (0, 0);
    let mut fix = (0, 0);
    for (task, answer) in tasks.iter().zip(answers.iter()) {
        let ok = score_answer(task, answer);
        if ok {
            passed += 1;
        } else {
            failed.push(task.id.clone());
        }
        match task.kind {
            TaskKind::Place => {
                place.0 += 1;
                place.1 += usize::from(ok);
            }
            TaskKind::Query => {
                query.0 += 1;
                query.1 += usize::from(ok);
            }
            TaskKind::Fix => {
                fix.0 += 1;
                fix.1 += usize::from(ok);
            }
        }
    }
    let mut fold = Fnv64::new();
    fold.write(EVAL_TAG);
    fold.write_u64(u64::from(REPORT_VERSION));
    fold.write_u64(u64::from(SPEC_VERSION));
    for (task, answer) in tasks.iter().zip(answers.iter()) {
        mix_text(&mut fold, &task.id);
        fold.write_u8(u8::from(score_answer(task, answer)));
        mix_text(&mut fold, &encode_gold(&task.gold));
    }
    let digest = fold.finish();
    Ok(EvalReport {
        total: tasks.len(),
        passed,
        failed,
        place,
        query,
        fix,
        digest,
        vision_notes: tasks.len(),
    })
}

/// Full eval report: score, per-family splits, failed ids and digest.
/// Vision notes ride along (`vision_notes`) and affect nothing: the digest
/// mixes task ids, passes and gold only, so advice can never gate a score.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalReport {
    /// Tasks scored.
    pub total: usize,
    /// Answers matching gold.
    pub passed: usize,
    /// Failed task ids, ascending.
    pub failed: Vec<String>,
    /// Place (total, passed).
    pub place: (usize, usize),
    /// Query (total, passed).
    pub query: (usize, usize),
    /// Fix (total, passed).
    pub fix: (usize, usize),
    /// Order-invariant digest over ids, passes and gold.
    pub digest: u64,
    /// Advisory vision notes carried (always the task count, never scored).
    pub vision_notes: usize,
}

// Table shape adapted from donor zhiburt/tabled, MIT licence,
// https://github.com/zhiburt/tabled/blob/master/tabled/src/builder/table_builder.rs
// SHAPE ONLY (push_record/build: collect header+rows, size columns to widest cell,
// pad, join), no new dependency.
fn pct_tenths(passed: usize, total: usize) -> String {
    if total == 0 {
        return "n/a".to_owned();
    }
    let tenths = passed * 1000 / total;
    format!("{}.{}%", tenths / 10, tenths % 10)
}

fn fail_family(id: &str) -> &'static str {
    let num = id.strip_prefix("sp12-").and_then(|s| s.parse::<u32>().ok());
    match num {
        Some(n) if (1..=30).contains(&n) => "place",
        Some(n) if (31..=70).contains(&n) => "query",
        Some(n) if (71..=100).contains(&n) => "fix",
        _ => "unknown",
    }
}

fn pad_to_width(cell: &str, width: usize) -> String {
    let mut padded = String::from(cell);
    while padded.len() < width {
        padded.push(' ');
    }
    padded
}

fn family_table(place: (usize, usize), query: (usize, usize), fix: (usize, usize)) -> String {
    let header = ["family", "passed", "total", "pct"];
    let rows = [
        (
            "place",
            place.1.to_string(),
            place.0.to_string(),
            pct_tenths(place.1, place.0),
        ),
        (
            "query",
            query.1.to_string(),
            query.0.to_string(),
            pct_tenths(query.1, query.0),
        ),
        (
            "fix",
            fix.1.to_string(),
            fix.0.to_string(),
            pct_tenths(fix.1, fix.0),
        ),
    ];
    let mut widths = [
        header[0].len(),
        header[1].len(),
        header[2].len(),
        header[3].len(),
    ];
    for (family, passed, total, pct) in &rows {
        widths[0] = widths[0].max(family.len());
        widths[1] = widths[1].max(passed.len());
        widths[2] = widths[2].max(total.len());
        widths[3] = widths[3].max(pct.len());
    }
    let mut out = String::new();
    let head = [header[0], header[1], header[2], header[3]]
        .iter()
        .zip(widths.iter())
        .map(|(cell, width)| pad_to_width(cell, *width))
        .collect::<Vec<_>>()
        .join(" | ");
    out.push_str(&head);
    out.push('\n');
    for (family, passed, total, pct) in &rows {
        let cells = [*family, passed.as_str(), total.as_str(), pct.as_str()];
        let line = cells
            .iter()
            .zip(widths.iter())
            .map(|(cell, width)| pad_to_width(cell, *width))
            .collect::<Vec<_>>()
            .join(" | ");
        out.push_str(&line);
        out.push('\n');
    }
    out
}

impl EvalReport {
    /// Reproducible text report: score line, family splits, then one row
    /// per failed task. Identical bytes for identical fixtures and agent.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "SP12 spatial gold eval v{REPORT_VERSION}: 100 tasks (place 30, query 40, fix 30)\n"
        ));
        out.push_str(&format!(
            "score {}/{} digest {:016x}\n",
            self.passed, self.total, self.digest
        ));
        out.push_str(&format!("kind place {}/{}\n", self.place.1, self.place.0));
        out.push_str(&format!("kind query {}/{}\n", self.query.1, self.query.0));
        out.push_str(&format!("kind fix {}/{}\n", self.fix.1, self.fix.0));
        out.push_str(&family_table(self.place, self.query, self.fix));
        out.push_str(&format!(
            "vision advisory notes={} affecting=0\n",
            self.vision_notes
        ));
        for id in &self.failed {
            out.push_str(&format!("fail {id} {}\n", fail_family(id)));
        }
        out
    }
}

/// Proof-gate outcome: the headline numbers the scoreboard reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateOutcome {
    /// Tasks scored.
    pub total: usize,
    /// Answers matching gold.
    pub passed: usize,
    /// Report digest.
    pub digest: u64,
    /// Full report text.
    pub text: String,
    /// Brief repair loop: stress scenes clean before any repair.
    pub repair_clean_before: usize,
    /// Brief repair loop: stress scenes clean after one repair round.
    pub repair_clean_after: usize,
    /// Brief repair loop: repair rounds spent across the scenes.
    pub repair_rounds: u32,
}

/// Run one full MCP-style session: every fixture task answered in order by
/// the agent, then scored. One command reproduces the score.
pub fn run_session(agent: &dyn SpatialAgent, tasks: &[Task]) -> Result<EvalReport, SpatialError> {
    let answers: Vec<AgentAnswer> = tasks.iter().map(|task| agent.answer(task)).collect();
    score(tasks, &answers)
}

/// Proof gate: the fresh competent agent over the checked-in fixtures,
/// verified against the oracle before scoring.
pub fn proof_gate() -> Result<GateOutcome, SpatialError> {
    let tasks = load_tasks()?;
    verify_gold(&tasks)?;
    let (repair_clean_before, repair_clean_after, repair_rounds) = run_repair_loop();
    let report = run_session(&GoldAgent, &tasks)?;
    Ok(GateOutcome {
        total: report.total,
        passed: report.passed,
        digest: report.digest,
        text: report.to_text(),
        repair_clean_before,
        repair_clean_after,
        repair_rounds,
    })
}

/// Name of the fuzz gate for scoreboard wiring, kept beside the gold gate
/// name so the harness keeps its key even before a registry knows it.
pub const FUZZ_GATE_NAME: &str = "spatial-fuzz-gate";

/// Fuzz-gate outcome: pair and probe counts beside the receipt digest and
/// the canonical text. Counts in items; digest is FNV-1a hex in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzGateOutcome {
    /// Clean op-stream pairs run; every one must agree.
    pub pairs: usize,
    /// Injected-divergence probes run; every one must locate.
    pub probes: usize,
    /// Clean pairs that diverged anyway. Must be 0.
    pub false_positives: usize,
    /// Probes naming exactly their injected op. Must equal `probes`.
    pub located: usize,
    /// Evidence digest over the whole suite.
    pub digest: u64,
    /// Canonical report text, byte-stable across runs.
    pub text: String,
}

/// Run the deterministic fuzz suite over the relation-graph ops: one hundred
/// seeded clean op-stream pairs plus ten injected-divergence probes, scored
/// beside the gold eval. Seeds are constants and op order is stream order,
/// so the bytes are identical on every run; no fixture, provider model or
/// clock enters the loop.
pub fn fuzz_gate() -> FuzzGateOutcome {
    let suite = fuzz::run_suite();
    FuzzGateOutcome {
        pairs: suite.pairs.len(),
        probes: suite.probes.len(),
        false_positives: suite.clean_false_positives(),
        located: suite.probes_located(),
        digest: suite.digest,
        text: suite.to_text(),
    }
}

/// Ten brief repair loop triple, deterministic.
pub fn run_repair_loop() -> (usize, usize, u32) {
    let mut before = 0;
    let mut after = 0;
    let mut total: u32 = 0;
    for index in 0..10 {
        let seed = crate::brief_eval::BRIEFS[index].seed;
        let mut start = engine_spatial::brief::stress_actors(
            seed,
            engine_spatial::brief::StressFamily::DuplicateNames,
        );
        if index % 2 == 0 && start.len() > 1 {
            let x = start[0].x;
            let z = start[0].z;
            start[1].x = x;
            start[1].z = z;
        }
        if engine_spatial::run(&start).findings.is_empty() {
            before += 1;
        }
        let rep = engine_spatial::run(&start);
        let (a, b) = engine_spatial::repair::dual_propose(&rep.findings, &start, 7);
        let agreed = engine_spatial::repair::self_consistent_decode(&a, &b);
        let mut cur = start;
        let mut used: u32 = 0;
        if agreed.len() == rep.findings.len() && !rep.findings.is_empty() {
            if let Some(next) = engine_spatial::repair::repair_round(&cur, &rep.findings, 7, 0) {
                cur = next;
            }
            used = 1;
        }
        if engine_spatial::run(&cur).findings.is_empty() {
            after += 1;
        }
        total = total.saturating_add(used);
    }
    (before, after, total)
}

// ---- tests -------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded() -> Vec<Task> {
        load_tasks().expect("fixtures must load")
    }

    #[test]
    fn hundred_tasks_cover_place_query_fix() {
        let tasks = loaded();
        assert_eq!(tasks.len(), TASK_COUNT);
        let (mut place, mut query, mut fix) = (0, 0, 0);
        for task in &tasks {
            match task.kind {
                TaskKind::Place => place += 1,
                TaskKind::Query => query += 1,
                TaskKind::Fix => fix += 1,
            }
        }
        assert_eq!((place, query, fix), (PLACE_COUNT, QUERY_COUNT, FIX_COUNT));
        for word in [
            QueryWord::Near,
            QueryWord::Between,
            QueryWord::Visible,
            QueryWord::Reachable,
            QueryWord::CoverNear,
            QueryWord::LargestOpen,
            QueryWord::FreeSpace,
        ] {
            assert!(
                tasks.iter().any(|t| t.word() == Some(word)),
                "gold must cover {}",
                word.as_str()
            );
        }
        for scene in ["room_pillar", "room_open", "room_tight"] {
            assert!(
                tasks.iter().any(|t| t.scene == scene),
                "gold must cover {scene}"
            );
        }
    }

    #[test]
    fn fixture_file_matches_generator() {
        // Hand edits to the TSV must go through the generator: the checked-in
        // file re-encodes from parsed tasks and matches the builder exactly.
        let tasks = loaded();
        let built = generate_tasks();
        assert_eq!(tasks, built, "tasks.tsv drifted from the generator");
        assert_eq!(encode_tsv(&built), encode_tsv(&tasks));
    }

    #[test]
    fn fresh_agent_scores_full_marks_twice_identical() {
        let tasks = loaded();
        let first = run_session(&GoldAgent, &tasks).expect("session scores");
        let second = run_session(&GoldAgent, &tasks).expect("session scores again");
        assert_eq!(first, second);
        assert_eq!((first.total, first.passed), (TASK_COUNT, TASK_COUNT));
        assert!(first.failed.is_empty());
        assert_eq!(first.vision_notes, TASK_COUNT);
        println!("{}", first.to_text());
    }

    #[test]
    fn place_gold_mixes_clean_and_conflicts() {
        // A verdict oracle stuck always-clean (or always-conflict) is the
        // plausible defect this guards: both outcomes must appear.
        let tasks = loaded();
        let mut clean = 0;
        let mut conflict = 0;
        for task in tasks.iter().filter(|t| t.kind == TaskKind::Place) {
            match &task.gold {
                Gold::Verdict {
                    clean: true,
                    conflicts,
                } => {
                    clean += 1;
                    assert!(conflicts.is_empty());
                }
                Gold::Verdict {
                    clean: false,
                    conflicts,
                } => {
                    conflict += 1;
                    assert!(!conflicts.is_empty() || params_place_out_of_bounds(task));
                }
                _ => panic!("place gold must be a verdict"),
            }
        }
        assert!(clean >= 5, "gold needs clean placements, saw {clean}");
        assert!(conflict >= 5, "gold needs conflicts, saw {conflict}");
    }

    #[test]
    fn seeded_relation_bug_drops_exactly_its_cases() {
        let tasks = loaded();
        let buggy = SeededBugAgent { bug: RELATION_BUG };
        let report = run_session(&buggy, &tasks).expect("buggy session scores");
        let want: Vec<String> = tasks
            .iter()
            .filter(|t| RELATION_BUG.affects(t.word()))
            .map(|t| t.id.clone())
            .collect();
        assert!(!want.is_empty(), "the bug must blind real tasks");
        assert!(want.iter().all(|id| tasks
            .iter()
            .find(|t| &t.id == id)
            .expect("bug case exists")
            .word()
            == Some(QueryWord::Between)));
        assert_eq!(report.failed, want, "the bug must drop exactly its cases");
        assert_eq!(report.passed + report.failed.len(), TASK_COUNT);
    }

    #[test]
    fn vision_is_advice_only() {
        // Stripping every vision note must leave the score untouched: the
        // digest mixes ids, passes and gold, never advice.
        let mut tasks = loaded();
        for task in &tasks {
            assert!(
                task.vision.note.starts_with("advisory:"),
                "vision must read advisory"
            );
        }
        let full = run_session(&GoldAgent, &tasks).expect("full scores");
        for task in &mut tasks {
            task.vision = Vision {
                note: "advisory:stripped".to_owned(),
                capture: capture_hex(&task.id),
            };
        }
        let stripped = run_session(&GoldAgent, &tasks).expect("stripped scores");
        assert_eq!(full.digest, stripped.digest);
        assert_eq!(full.passed, stripped.passed);
    }

    #[test]
    fn repair_hints_land_clean() {
        // Every fix gold must verify clean when applied: a hint that does
        // not repair is the plausible defect this catches.
        let tasks = loaded();
        let mut count = 0;
        for task in tasks.iter().filter(|t| t.kind == TaskKind::Fix) {
            let Gold::Repair { x_cm, z_cm, .. } = &task.gold else {
                panic!("fix gold must be a repair")
            };
            let scene = scene_of(&task.scene).expect("fix scene exists");
            let (clean, conflicts) = place_verdict(scene, *x_cm, *z_cm, 40, 40);
            assert!(
                clean,
                "hint for {} lands conflicted: {conflicts:?}",
                task.id
            );
            assert!(conflicts.is_empty());
            count += 1;
        }
        assert_eq!(count, FIX_COUNT);
    }

    #[test]
    fn gold_id_lists_are_sorted_unique_and_in_scene() {
        let tasks = loaded();
        for task in &tasks {
            let ids: &[u32] = match &task.gold {
                Gold::Ids(ids) => ids,
                Gold::Verdict { conflicts, .. } => conflicts,
                Gold::Repair { .. } | Gold::Samples { .. } => continue,
            };
            let mut sorted = ids.to_vec();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(ids, &sorted, "gold for {} must be sorted unique", task.id);
            let scene = scene_of(&task.scene).expect("scene exists");
            for id in ids {
                assert!(
                    actor_of(scene, *id).is_some(),
                    "gold id {id} outside {}",
                    task.scene
                );
            }
        }
    }

    #[test]
    fn crouch_cover_sets_only_grow() {
        // Stand cover implies crouch cover for the same anchor and threat:
        // a crouch that loses cover is the plausible defect this guards.
        let tasks = loaded();
        let stand = tasks
            .iter()
            .find(|t| matches!(&t.params, Params::CoverNear { crouch: false, .. }))
            .expect("stand cover task exists");
        let crouch = tasks
            .iter()
            .find(|t| {
                matches!(&t.params, Params::CoverNear { crouch: true, anchor, tx_cm, tz_cm, .. }
                    if matches!(&stand.params, Params::CoverNear { anchor: a, tx_cm: x, tz_cm: z, .. }
                        if a == anchor && x == tx_cm && z == tz_cm))
            })
            .expect("matching crouch task exists");
        let (Gold::Ids(stand_ids), Gold::Ids(crouch_ids)) = (&stand.gold, &crouch.gold) else {
            panic!("cover gold must be id sets");
        };
        for id in stand_ids {
            assert!(crouch_ids.contains(id), "crouch lost cover on {id}");
        }
    }

    #[test]
    fn unknown_task_id_is_a_typed_error() {
        let tasks = loaded();
        let error = task_by_id(&tasks, "sp12-101").expect_err("unknown id must fail");
        assert_eq!(error.code(), "bad_task");
        assert!(error.to_string().contains("sp12-101"));
        assert!(task_by_id(&tasks, "sp12-001").is_ok());
    }

    #[test]
    fn corrupt_fixture_rows_are_typed_errors() {
        let tasks = loaded();
        let good = encode_tsv(&tasks);
        // Truncated columns refuse.
        let bad = good.replacen("sp12-001\tplace", "sp12-001", 1);
        assert!(parse_tsv(&bad).is_err(), "short row must refuse");
        // Unknown kind refuses with the legal list.
        let bad = good.replacen("\tplace\t", "\tplacee\t", 1);
        let error = parse_tsv(&bad).expect_err("bad kind must refuse");
        assert!(
            error.to_string().contains("place, query, fix"),
            "error must name legal kinds: {error}"
        );
        // Wrong count refuses.
        let mut lines: Vec<&str> = good.lines().collect();
        lines.pop();
        let bad = lines.join("\n") + "\n";
        assert!(parse_tsv(&bad).is_err(), "99 rows must refuse");
        // A one-id lie in gold drops exactly that row when scored.
        let mut lied = tasks.clone();
        let idx = lied
            .iter()
            .position(|t| matches!(&t.gold, Gold::Ids(ref ids) if !ids.is_empty()))
            .expect("nonempty id gold exists");
        if let Gold::Ids(ids) = &mut lied[idx].gold {
            ids.pop();
        }
        let answers: Vec<AgentAnswer> = lied.iter().map(|t| GoldAgent.answer(t)).collect();
        let report = score(&tasks, &answers).expect("scores");
        assert_eq!(
            report.failed,
            vec![lied[idx].id.clone()],
            "one lie drops one row"
        );
    }

    #[test]
    fn live_api_mapping_names_every_word() {
        // Each query word maps to the live public API the table names;
        // an unmapped word is the defect this guards.
        let mut apis = Vec::new();
        for word in [
            QueryWord::Near,
            QueryWord::Between,
            QueryWord::Visible,
            QueryWord::Reachable,
            QueryWord::CoverNear,
            QueryWord::LargestOpen,
            QueryWord::FreeSpace,
        ] {
            assert!(
                !word.live_api().is_empty(),
                "{} must name its live API",
                word.as_str()
            );
            apis.push(word.live_api());
        }
        apis.sort_unstable();
        apis.dedup();
        assert_eq!(apis.len(), 7, "every word maps to its own live API");
    }

    #[test]
    fn report_text_reproduces_the_shape() {
        let tasks = loaded();
        let report = run_session(&GoldAgent, &tasks).expect("session scores");
        let text = report.to_text();
        println!("{text}");
        assert!(text.contains("SP12 spatial gold eval"));
        assert!(text.contains(&format!("score {TASK_COUNT}/{TASK_COUNT}")));
        assert!(text.contains("kind place 30/30"));
        assert!(text.contains("kind query 40/40"));
        assert!(text.contains("kind fix 30/30"));
        assert!(text.contains("affecting=0"));
        let buggy =
            run_session(&SeededBugAgent { bug: RELATION_BUG }, &tasks).expect("buggy scores");
        let buggy_text = buggy.to_text();
        assert!(
            buggy_text.contains("fail sp12-"),
            "buggy report must name its cases"
        );
        assert_ne!(report.digest, buggy.digest, "the bug must move the digest");
    }

    #[test]
    fn answer_count_mismatch_is_a_typed_error() {
        let tasks = loaded();
        let error = score(&tasks, &[]).expect_err("empty answers must fail");
        assert_eq!(error.code(), "answer_count");
        let partial: Vec<AgentAnswer> =
            tasks.iter().take(10).map(|t| GoldAgent.answer(t)).collect();
        assert!(score(&tasks, &partial).is_err());
    }

    #[test]
    fn verifier_accepts_fixtures_and_rejects_drift() {
        // verify_gold is the runtime half of the generator freeze: clean
        // fixtures verify, a one-id lie fails loudly.
        let tasks = loaded();
        verify_gold(&tasks).expect("fixtures verify against the oracle");
        let mut drifted = tasks.clone();
        if let Some(task) = drifted.iter_mut().find(|t| matches!(&t.gold, Gold::Ids(_))) {
            task.gold = Gold::Ids(vec![999_999]);
        }
        let error = verify_gold(&drifted).expect_err("drifted gold must fail");
        assert_eq!(error.code(), "bad_fixture");
        assert!(error.to_string().contains("drifts from the oracle"));
    }

    #[test]
    fn gate_names_the_registered_door() {
        assert_eq!(SPATIAL_GATE_NAME, "sp12-spatial-gold-eval");
        let outcome = proof_gate().expect("gate runs on fixtures");
        assert_eq!((outcome.total, outcome.passed), (TASK_COUNT, TASK_COUNT));
        assert!(outcome.text.contains("affecting=0"));
        assert_eq!(
            (
                outcome.repair_clean_before,
                outcome.repair_clean_after,
                outcome.repair_rounds
            ),
            run_repair_loop(),
            "the gate carries the repair loop triple, twice alike"
        );
        assert!(
            outcome.repair_clean_after >= outcome.repair_clean_before,
            "repair never leaves more stress scenes dirty than it found"
        );
    }

    #[test]
    fn fuzz_gate_meets_the_bar() {
        assert_eq!(FUZZ_GATE_NAME, "spatial-fuzz-gate");
        let first = fuzz_gate();
        assert_eq!(first.pairs, fuzz::FUZZ_PAIRS, "one hundred clean pairs");
        assert_eq!(first.probes, fuzz::FUZZ_PROBES, "ten injected probes");
        assert_eq!(first.false_positives, 0, "clean pairs never diverge");
        assert_eq!(
            first.located, first.probes,
            "every probe names its injected op"
        );
        assert!(first.text.contains("located 10/10"));
        let second = fuzz_gate();
        assert_eq!(first.digest, second.digest, "suite digest twice alike");
        assert_eq!(first.text, second.text, "suite text twice alike");
    }

    /// True when a place proposal sits outside the room bounds (the only
    /// clean=false verdict allowed to carry an empty conflict set).
    fn params_place_out_of_bounds(task: &Task) -> bool {
        let (
            Params::Place {
                x_cm,
                z_cm,
                hx_cm,
                hz_cm,
            },
            Some(scene),
        ) = (&task.params, scene_of(&task.scene))
        else {
            return false;
        };
        *x_cm - *hx_cm < scene.min_cm
            || *x_cm + *hx_cm > scene.max_cm
            || *z_cm - *hz_cm < scene.min_cm
            || *z_cm + *hz_cm > scene.max_cm
    }
}
