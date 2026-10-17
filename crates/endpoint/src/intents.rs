//! SP11 intent macros over SP5 placements in one undoable batch.
//!
//! Owner: endpoint. Consumers: `apply_brief` authors whole briefs through
//! the SP5 solver plus one significant ED7 journal entry, so one undo
//! reverts the batch; `scene.query` reads the staged scene. Units: meters
//! in, integer cm inside the solver. Determinism: same ops plus seed give
//! the same placements (scatter draws a wrapping LCG; circle trig collapses
//! onto the 10 cm solver grid). Errors: typed `IntentError` naming the
//! legal values.

use std::collections::{BTreeMap, BTreeSet};

use crate::canon::valid_name;
use crate::spatial_glue::{quat_of_yaw, yaw_of_quat};
use engine_editor::commands::{self, Batch, CommandMeta, Document, Journal, Op, ProvenanceStamp};
use engine_scene::ids::ObjectId;
use engine_scene::transform::Transform;
use engine_spatial::{Actor, Intent, ObjectRef, Relation, Target, Wall, Want};

/// Most macro ops one brief may hold.
pub const MAX_INTENT_OPS: usize = 32;
/// Most new solved objects one brief may place (bootstrap walls excluded).
pub const MAX_NEW_OBJECTS: usize = 24;
/// Actor-name bound, mirroring the spatial grammar check.
pub const NAME_MAX: usize = crate::canon::NAME_MAX;
/// First wall actor id; wall ids never collide with prop ids.
pub const WALL_BASE: u32 = 900_000;
/// Default room half-extent in meters when no bootstrap sets one.
pub const DEFAULT_HALF_M: f32 = 5.0;
/// Room half-extent bounds in meters.
pub const MIN_HALF_M: f32 = 2.0;
/// Room half-extent bounds in meters.
pub const MAX_HALF_M: f32 = 10.0;
/// Along-wall lane spacing in meters between neighbours.
pub const WALL_LANE_DX_M: f32 = 1.2;
/// Along-wall inset in meters from the wall inner face.
pub const WALL_LANE_INSET_M: f32 = 1.0;

/// Creatable kinds with their grounded sizes in meters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentKind {
    Player,
    Enemy,
    Camera,
    Crate,
    Brazier,
    Pillar,
    Torch,
    Banner,
    Bench,
    Altar,
    Stand,
    Post,
}

impl IntentKind {
    /// Legal kind words, in door order.
    pub const KINDS: &[&str] = &[
        "player", "enemy", "camera", "crate", "brazier", "pillar", "torch", "banner", "bench",
        "altar", "stand", "post",
    ];

    /// Door word for this kind, mirroring `parse`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Player => "player",
            Self::Enemy => "enemy",
            Self::Camera => "camera",
            Self::Crate => "crate",
            Self::Brazier => "brazier",
            Self::Pillar => "pillar",
            Self::Torch => "torch",
            Self::Banner => "banner",
            Self::Bench => "bench",
            Self::Altar => "altar",
            Self::Stand => "stand",
            Self::Post => "post",
        }
    }

    /// Parse a kind word; unknown words refuse with the legal list.
    pub fn parse(text: &str) -> Result<Self, IntentError> {
        match text {
            "player" => Ok(Self::Player),
            "enemy" => Ok(Self::Enemy),
            "camera" => Ok(Self::Camera),
            "crate" => Ok(Self::Crate),
            "brazier" => Ok(Self::Brazier),
            "pillar" => Ok(Self::Pillar),
            "torch" => Ok(Self::Torch),
            "banner" => Ok(Self::Banner),
            "bench" => Ok(Self::Bench),
            "altar" => Ok(Self::Altar),
            "stand" => Ok(Self::Stand),
            "post" => Ok(Self::Post),
            _ => Err(IntentError::UnknownKind { kind: text.into() }),
        }
    }

    /// Grounded `(sx, sy, sz)` size in meters.
    pub fn size(&self) -> (f32, f32, f32) {
        match self {
            Self::Player | Self::Enemy => (0.6, 1.7, 0.6),
            Self::Camera => (0.4, 1.5, 0.4),
            Self::Crate => (0.8, 0.8, 0.8),
            Self::Brazier => (0.6, 1.0, 0.6),
            Self::Pillar => (0.5, 3.0, 0.5),
            Self::Torch => (0.3, 1.6, 0.3),
            Self::Banner => (0.9, 2.0, 0.1),
            Self::Bench => (1.8, 0.5, 0.5),
            Self::Altar => (1.5, 1.0, 1.0),
            Self::Stand => (1.2, 0.8, 1.2),
            Self::Post => (0.3, 2.2, 0.3),
        }
    }
}

/// `scene.bootstrap` args: four thin grounded walls forming a square room.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BootstrapArgs {
    /// Room half-extent in meters, `MIN_HALF_M..=MAX_HALF_M`.
    pub half_extent_m: f32,
}

/// `create.player` / `create.enemy` / `create.camera` args: one named actor.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateArgs {
    /// Unique actor name (spatial grammar: `[a-z0-9_]`, no leading digit).
    pub name: String,
    /// Solver wants; `Object` targets name new actors in the same brief.
    pub wants: Vec<NamedWant>,
}

impl CreateArgs {
    /// One actor with no wants (first free solver cell).
    pub fn named(name: &str) -> Self {
        Self {
            name: name.into(),
            wants: Vec::new(),
        }
    }
}

/// `arrange` patterns: every item becomes a point want, solved in order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ArrangePattern {
    /// Centered rows-by-cols lattice; `rows * cols` must equal the headcount.
    Grid {
        rows: u32,
        cols: u32,
        spacing_m: f32,
    },
    /// Ring around the room center.
    Circle { radius_m: f32 },
    /// Seeded jittered lattice (deterministic per seed).
    Scatter,
    /// Lane along one wall inner face, centered, `WALL_LANE_DX_M` apart.
    AlongWall { wall: Wall },
}

/// `arrange` args: many actors of one kind in one pattern.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrangeArgs {
    /// Shared kind for every item.
    pub kind: IntentKind,
    /// One unique name per item.
    pub names: Vec<String>,
    /// Layout pattern.
    pub pattern: ArrangePattern,
}

/// One `furnish` row: a kind, a name and its wants.
#[derive(Debug, Clone, PartialEq)]
pub struct FurnishItem {
    /// Prop kind.
    pub kind: IntentKind,
    /// Unique actor name.
    pub name: String,
    /// Solver wants; `Object` targets name new actors in the same brief.
    pub wants: Vec<NamedWant>,
}

/// One relation want with a brief-local target: walls, cm points, or the
/// name of another new actor in the same brief (resolved at compile time).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedWant {
    /// SP3 relation word.
    pub relation: Relation,
    /// Brief-local target.
    pub target: NamedTarget,
    /// Inclusive `(lo, hi)` cm range, or the per-target default.
    pub distance_cm: Option<(i32, i32)>,
}

/// Brief-local want target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamedTarget {
    /// Room wall.
    Wall(Wall),
    /// Explicit XZ point in integer centimeters.
    Point { x_cm: i32, z_cm: i32 },
    /// Another new actor in the same brief, by name.
    Object(String),
}

/// One intent macro: the agent spells about ten of these instead of dozens
/// of raw placement calls.
#[derive(Debug, Clone, PartialEq)]
pub enum IntentOp {
    /// `scene.bootstrap`: the walled room every later want solves against.
    Bootstrap(BootstrapArgs),
    /// `create.player`: one named fighter start.
    CreatePlayer(CreateArgs),
    /// `create.enemy`: one named opponent start.
    CreateEnemy(CreateArgs),
    /// `create.camera`: one named camera prop.
    CreateCamera(CreateArgs),
    /// `arrange`: one kind in a grid, circle, scatter or wall lane.
    Arrange(ArrangeArgs),
    /// `furnish`: mixed props with per-item wants.
    Furnish(Vec<FurnishItem>),
}

/// What stopped a brief. Every message names the legal values; failures
/// before apply write nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentError {
    /// Brief holds no macro.
    EmptyBrief,
    /// Brief holds more than `MAX_INTENT_OPS` macros.
    TooManyOps { count: usize },
    /// Unknown prop kind.
    UnknownKind { kind: String },
    /// Two new actors share one name (or one collides with the staged scene).
    DuplicateName { name: String },
    /// A name breaks the spatial grammar.
    BadName { name: String },
    /// A pattern is inconsistent or does not fit the room.
    BadPattern { reason: String },
    /// Brief places more than `MAX_NEW_OBJECTS` solved objects.
    TooManyObjects { count: usize },
    /// Two bootstraps in one brief (walls would overlap).
    DoubleBootstrap,
    /// A want names a new actor the brief never declares.
    UnknownName { name: String },
    /// A want range has `lo < 0` or `hi < lo`.
    BadRange { source: String },
    /// The solver admits no placement; carries the minimal conflict.
    Unplaceable {
        name: String,
        reason: String,
        conflicts: Vec<String>,
    },
    /// Solved placements trip a spatial check; writes nothing.
    CheckFailed { findings: Vec<String> },
    /// The single apply was refused; the document is untouched.
    ApplyFailed { reason: String },
    /// The compiled brief disagrees with itself; writes nothing.
    Internal { reason: String },
}

impl IntentError {
    /// Machine code for this failure.
    pub fn code(&self) -> &'static str {
        match self {
            Self::EmptyBrief => "empty_brief",
            Self::TooManyOps { .. } => "too_many_ops",
            Self::UnknownKind { .. } => "unknown_kind",
            Self::DuplicateName { .. } => "duplicate_name",
            Self::BadName { .. } => "bad_name",
            Self::BadPattern { .. } => "bad_pattern",
            Self::TooManyObjects { .. } => "too_many_objects",
            Self::DoubleBootstrap => "double_bootstrap",
            Self::UnknownName { .. } => "unknown_name",
            Self::BadRange { .. } => "bad_range",
            Self::Unplaceable { .. } => "unplaceable",
            Self::CheckFailed { .. } => "check_failed",
            Self::ApplyFailed { .. } => "apply_failed",
            Self::Internal { .. } => "internal",
        }
    }
}

impl std::fmt::Display for IntentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyBrief => write!(
                f,
                "empty_brief: the brief holds no macro; submit scene.bootstrap, \
                 create.player/enemy/camera, arrange or furnish"
            ),
            Self::TooManyOps { count } => write!(
                f,
                "too_many_ops: brief holds {count} macros; at most {MAX_INTENT_OPS}"
            ),
            Self::UnknownKind { kind } => write!(
                f,
                "unknown_kind `{kind}`: submit one of {}",
                IntentKind::KINDS.join(", ")
            ),
            Self::DuplicateName { name } => write!(
                f,
                "duplicate_name `{name}`: every new actor name must be unique in \
                 one brief and miss the staged scene"
            ),
            Self::BadName { name } => write!(
                f,
                "bad_name `{name}`: lowercase letters, digits and `_`, no \
                 leading digit, at most {NAME_MAX} chars"
            ),
            Self::BadPattern { reason } => write!(f, "bad_pattern: {reason}"),
            Self::TooManyObjects { count } => write!(
                f,
                "too_many_objects: brief places {count} objects; at most {MAX_NEW_OBJECTS}"
            ),
            Self::DoubleBootstrap => write!(
                f,
                "double_bootstrap: one brief bootstraps at most once (walls \
                 would overlap); drop the second scene.bootstrap"
            ),
            Self::UnknownName { name } => write!(
                f,
                "unknown_name `{name}`: an object want names a new actor the \
                 brief never declares; declare it with create/arrange/furnish first"
            ),
            Self::BadRange { source } => write!(
                f,
                "bad_range `{source}`: need 0 <= lo <= hi in `[<lo>,<hi>]`"
            ),
            Self::Unplaceable {
                name,
                reason,
                conflicts,
            } => write!(
                f,
                "unplaceable `{name}`: {reason}; conflicts: [{}]",
                conflicts.join(", ")
            ),
            Self::CheckFailed { findings } => write!(
                f,
                "check_failed: solved placements trip {} finding(s): {}; \
                 loosen a want range and rebuild",
                findings.len(),
                findings.join("; ")
            ),
            Self::ApplyFailed { reason } => write!(f, "apply_failed: {reason}"),
            Self::Internal { reason } => write!(
                f,
                "internal `{reason}`: the compiled brief disagrees with itself; \
                 rebuild from scene.bootstrap and retry"
            ),
        }
    }
}

impl std::error::Error for IntentError {}

fn relation_word(relation: Relation) -> &'static str {
    match relation {
        Relation::Supports => "supports",
        Relation::OnTopOf => "on_top_of",
        Relation::Inside => "inside",
        Relation::NextTo => "next_to",
        Relation::Facing => "facing",
    }
}

fn wall_word(wall: Wall) -> &'static str {
    match wall {
        Wall::North => "north",
        Wall::South => "south",
        Wall::East => "east",
        Wall::West => "west",
    }
}

/// One want rendered in the `scene.place` grammar (conflicts echo these).
fn want_text(want: &Want) -> String {
    let target = match want.target {
        Target::Wall(wall) => format!("wall:{}", wall_word(wall)),
        Target::Object(id) => format!("object:{id}"),
        Target::Point { x_cm, z_cm } => format!("point:{x_cm},{z_cm}"),
    };
    match want.distance_cm {
        None => format!("{} {target}", relation_word(want.relation)),
        Some((lo, hi)) => format!("{} {target} [{lo},{hi}]", relation_word(want.relation)),
    }
}

/// Sorted-id snapshot of every node with a finite world position. Sizes ride
/// the node scale (sanitized to positive finite, else a unit cube); the wire
/// order never moves, so the same document always maps to the same actors.
fn snapshot(doc: &Document) -> (Vec<Actor>, Vec<ObjectId>) {
    let mut order: Vec<ObjectId> = Vec::new();
    let mut stack = doc.hierarchy.roots();
    while let Some(id) = stack.pop() {
        order.push(id);
        stack.extend(doc.hierarchy.children(id));
    }
    order.sort_by_key(|id| id.to_string());
    let mut actors = Vec::new();
    let mut ids = Vec::new();
    for id in order {
        let Some(world) = doc.hierarchy.world(id) else {
            continue;
        };
        if !world.position.iter().all(|lane| lane.is_finite()) {
            continue;
        }
        let yaw = if world.rotation.iter().all(|lane| lane.is_finite()) {
            yaw_of_quat(world.rotation)
        } else {
            0.0
        };
        let (sx, sy, sz) = doc
            .hierarchy
            .local(id)
            .map(|pose| (pose.scale[0], pose.scale[1], pose.scale[2]))
            .unwrap_or((1.0, 1.0, 1.0));
        let sane = |v: f32| v.is_finite() && v > 0.0;
        let (sx, sy, sz) = (
            if sane(sx) { sx } else { 1.0 },
            if sane(sy) { sy } else { 1.0 },
            if sane(sz) { sz } else { 1.0 },
        );
        let index = u32::try_from(actors.len()).unwrap_or(u32::MAX);
        if index == u32::MAX {
            break;
        }
        ids.push(id);
        actors.push(Actor {
            id: index,
            name: format!("prop_{index:04}"),
            x: world.position[0],
            y: world.position[1],
            z: world.position[2],
            sx,
            sy,
            sz,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw,
        });
    }
    (actors, ids)
}

/// One solved placement in meters, yaw radians.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    /// Actor name from the brief.
    pub name: String,
    /// Prop kind from the brief.
    pub kind: IntentKind,
    /// Solver actor id.
    pub id: u32,
    /// Position meters.
    pub x: f32,
    /// Grounded height meters.
    pub y: f32,
    /// Position meters.
    pub z: f32,
    /// Facing yaw radians.
    pub yaw: f32,
}

/// Pure compile result: placements plus the staged scene. Writes nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct Compiled {
    /// Placements in brief order.
    pub placements: Vec<Placement>,
    /// Existing plus walls plus solved actors (checks run over this).
    pub actors: Vec<Actor>,
    /// New actor ids in brief order (walls first, then solved).
    pub new_ids: Vec<u32>,
    /// Raw placement ops this brief replaces (one batch replaces them all).
    pub raw_ops: usize,
}

struct Spec {
    name: String,
    kind: IntentKind,
    wants: Vec<Want>,
}

fn resolve_want(want: &NamedWant, index_of: &BTreeMap<String, u32>) -> Result<Want, IntentError> {
    let target = match &want.target {
        NamedTarget::Wall(wall) => Target::Wall(*wall),
        NamedTarget::Point { x_cm, z_cm } => Target::Point {
            x_cm: *x_cm,
            z_cm: *z_cm,
        },
        NamedTarget::Object(name) => match index_of.get(name).copied() {
            Some(id) => Target::Object(id),
            None => return Err(IntentError::UnknownName { name: name.clone() }),
        },
    };
    if let Some((lo, hi)) = want.distance_cm {
        if lo < 0 || hi < lo {
            return Err(IntentError::BadRange {
                source: format!(
                    "{} {} [{lo},{hi}]",
                    relation_word(want.relation),
                    match &want.target {
                        NamedTarget::Wall(wall) => format!("wall:{}", wall_word(*wall)),
                        NamedTarget::Point { x_cm, z_cm } => format!("point:{x_cm},{z_cm}"),
                        NamedTarget::Object(name) => format!("object:{name}"),
                    }
                ),
            });
        }
    }
    Ok(Want {
        relation: want.relation,
        target,
        distance_cm: want.distance_cm,
    })
}

fn check_name(name: &str, taken: &BTreeSet<String>) -> Result<(), IntentError> {
    if !valid_name(name) {
        return Err(IntentError::BadName { name: name.into() });
    }
    if taken.contains(name) {
        return Err(IntentError::DuplicateName { name: name.into() });
    }
    Ok(())
}

/// Wrapping LCG step; the high 31 bits name a float in `[0, 1)`.
fn lcg_unit(state: &mut u64) -> f32 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    ((*state >> 33) as f32) / 2_147_483_600.0
}

/// Pattern points in cm, deterministic per seed. Every point stays a meter
/// inside the room so the solver grid covers it.
fn arrange_points(
    pattern: &ArrangePattern,
    count: usize,
    half: f32,
    seed: u64,
) -> Result<Vec<(i32, i32)>, IntentError> {
    let bad = |reason: String| IntentError::BadPattern { reason };
    let room = 2.0 * half - 2.0;
    let to_cm = |m: f32| (m * 100.0).round() as i32;
    match *pattern {
        ArrangePattern::Grid {
            rows,
            cols,
            spacing_m,
        } => {
            if rows == 0 || cols == 0 || rows > 8 || cols > 8 {
                return Err(bad(format!(
                    "grid needs 1..=8 rows and cols, got {rows}x{cols}"
                )));
            }
            if rows as usize * cols as usize != count {
                return Err(bad(format!(
                    "grid {rows}x{cols} holds {} cells for {count} names; \
                     rows * cols must equal the headcount",
                    rows as usize * cols as usize
                )));
            }
            if !(0.5..=4.0).contains(&spacing_m) {
                return Err(bad(format!(
                    "grid spacing {spacing_m} m is outside 0.5..=4.0 m"
                )));
            }
            let span_x = (cols as f32 - 1.0) * spacing_m;
            let span_z = (rows as f32 - 1.0) * spacing_m;
            if span_x > room || span_z > room {
                return Err(bad(format!(
                    "grid spans {span_x:.1}x{span_z:.1} m, the room fits \
                     {room:.1}x{room:.1} m; shrink the grid or grow the bootstrap"
                )));
            }
            let mut out = Vec::with_capacity(count);
            for i in 0..count {
                let gx = (i as u32 % cols) as f32;
                let gz = (i as u32 / cols) as f32;
                out.push((
                    to_cm((gx - (cols as f32 - 1.0) / 2.0) * spacing_m),
                    to_cm((gz - (rows as f32 - 1.0) / 2.0) * spacing_m),
                ));
            }
            Ok(out)
        }
        ArrangePattern::Circle { radius_m } => {
            if radius_m > half - 1.0 || radius_m < 0.5 {
                return Err(bad(format!(
                    "circle radius {radius_m} m needs 0.5..={:.1} m in this room",
                    half - 1.0
                )));
            }
            let mut out = Vec::with_capacity(count);
            for i in 0..count {
                let angle = 2.0 * std::f32::consts::PI * i as f32 / count as f32;
                out.push((to_cm(radius_m * angle.cos()), to_cm(radius_m * angle.sin())));
            }
            Ok(out)
        }
        ArrangePattern::Scatter => {
            let cols = count.isqrt().max(1);
            let rows = count.div_ceil(cols);
            let cell = room / cols.max(rows) as f32;
            let mut rng = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut out = Vec::with_capacity(count);
            for i in 0..count {
                let gx = (i % cols) as f32;
                let gz = (i / cols) as f32;
                let jx = (lcg_unit(&mut rng) - 0.5) * cell / 2.0;
                let jz = (lcg_unit(&mut rng) - 0.5) * cell / 2.0;
                out.push((
                    to_cm(-half + 1.0 + (gx + 0.5) * cell + jx),
                    to_cm(-half + 1.0 + (gz + 0.5) * cell + jz),
                ));
            }
            Ok(out)
        }
        ArrangePattern::AlongWall { wall } => {
            if count == 0 || count > 12 {
                return Err(bad(format!("wall lanes hold 1..=12 names, got {count}")));
            }
            let span = (count as f32 - 1.0) * WALL_LANE_DX_M;
            if span > room {
                return Err(bad(format!(
                    "lane of {count} spans {span:.1} m, the wall fits \
                     {room:.1} m; shorten the lane or grow the bootstrap"
                )));
            }
            let face = half - WALL_LANE_INSET_M;
            let mut out = Vec::with_capacity(count);
            for i in 0..count {
                let along = (i as f32 - (count as f32 - 1.0) / 2.0) * WALL_LANE_DX_M;
                let (x, z) = match wall {
                    Wall::North => (along, -face),
                    Wall::South => (along, face),
                    Wall::West => (-face, along),
                    Wall::East => (face, along),
                };
                out.push((to_cm(x), to_cm(z)));
            }
            Ok(out)
        }
    }
}

/// Thin grounded walls forming a `half` square. Corners touch at most, never
/// overlap, mirroring the SP5 room convention.
fn wall_actors(half: f32) -> Vec<Actor> {
    let wall = |id: u32, name: &str, x: f32, z: f32, sx: f32, sz: f32| Actor {
        id,
        name: name.into(),
        x,
        y: 1.0,
        z,
        sx,
        sy: 2.0,
        sz,
        parent: None,
        physical: true,
        vx: 0.0,
        vz: 0.0,
        yaw: 0.0,
    };
    let span = 2.0 * half - 0.2;
    vec![
        wall(WALL_BASE, "wall_north", 0.0, -half, span, 0.2),
        wall(WALL_BASE + 1, "wall_south", 0.0, half, span, 0.2),
        wall(WALL_BASE + 2, "wall_west", -half, 0.0, 0.2, 2.0 * half),
        wall(WALL_BASE + 3, "wall_east", half, 0.0, 0.2, 2.0 * half),
    ]
}

/// Compile macros to SP5 placements over `existing` actors. Pure: same ops
/// plus seed give the same placements; writes nothing.
pub fn compile(ops: &[IntentOp], existing: &[Actor], seed: u64) -> Result<Compiled, IntentError> {
    if ops.is_empty() {
        return Err(IntentError::EmptyBrief);
    }
    if ops.len() > MAX_INTENT_OPS {
        return Err(IntentError::TooManyOps { count: ops.len() });
    }
    // Pass 1: names, ids and kinds. New ids continue past the staged scene
    // so brief ids never collide with document ids.
    let mut taken: BTreeSet<String> = existing.iter().map(|a| a.name.clone()).collect();
    let mut next_id = existing
        .iter()
        .map(|a| a.id)
        .filter(|id| *id < WALL_BASE)
        .max()
        .map_or(0, |max| max.saturating_add(1));
    let mut half: Option<f32> = None;
    // (name, kind) in brief order.
    let mut declared: Vec<(String, IntentKind)> = Vec::new();
    for op in ops {
        match op {
            IntentOp::Bootstrap(args) => {
                if half.is_some() {
                    return Err(IntentError::DoubleBootstrap);
                }
                if !(MIN_HALF_M..=MAX_HALF_M).contains(&args.half_extent_m) {
                    return Err(IntentError::BadPattern {
                        reason: format!(
                            "bootstrap half {} m is outside {MIN_HALF_M}..={MAX_HALF_M} m",
                            args.half_extent_m
                        ),
                    });
                }
                half = Some(args.half_extent_m);
            }
            IntentOp::CreatePlayer(args)
            | IntentOp::CreateEnemy(args)
            | IntentOp::CreateCamera(args) => {
                check_name(&args.name, &taken)?;
                taken.insert(args.name.clone());
                let kind = match op {
                    IntentOp::CreatePlayer(_) => IntentKind::Player,
                    IntentOp::CreateEnemy(_) => IntentKind::Enemy,
                    _ => IntentKind::Camera,
                };
                declared.push((args.name.clone(), kind));
            }
            IntentOp::Arrange(args) => {
                if args.names.is_empty() {
                    return Err(IntentError::BadPattern {
                        reason: "arrange needs at least one name".to_string(),
                    });
                }
                for name in &args.names {
                    check_name(name, &taken)?;
                    taken.insert(name.clone());
                    declared.push((name.clone(), args.kind));
                }
            }
            IntentOp::Furnish(items) => {
                if items.is_empty() {
                    return Err(IntentError::BadPattern {
                        reason: "furnish needs at least one item".to_string(),
                    });
                }
                for item in items {
                    check_name(&item.name, &taken)?;
                    taken.insert(item.name.clone());
                    declared.push((item.name.clone(), item.kind));
                }
            }
        }
    }
    if declared.len() > MAX_NEW_OBJECTS {
        return Err(IntentError::TooManyObjects {
            count: declared.len(),
        });
    }
    // Fresh ids in brief order.
    let mut index_of: BTreeMap<String, u32> = BTreeMap::new();
    for (name, _) in &declared {
        index_of.insert(name.clone(), next_id);
        next_id = next_id.saturating_add(1);
    }
    let bootstrapped = half.is_some();
    let half = half.unwrap_or_else(|| {
        let mut extent = DEFAULT_HALF_M;
        for actor in existing {
            extent = extent
                .max(actor.x.abs() + actor.sx.abs() / 2.0)
                .max(actor.z.abs() + actor.sz.abs() / 2.0);
        }
        extent.clamp(MIN_HALF_M, MAX_HALF_M)
    });
    // Pass 2: wants. Object targets resolve against the brief-local map, so
    // declaration order never matters.
    let mut specs: Vec<Spec> = Vec::with_capacity(declared.len());
    for op in ops {
        match op {
            IntentOp::Bootstrap(_) => {}
            IntentOp::CreatePlayer(args)
            | IntentOp::CreateEnemy(args)
            | IntentOp::CreateCamera(args) => {
                let mut wants = Vec::with_capacity(args.wants.len());
                for want in &args.wants {
                    wants.push(resolve_want(want, &index_of)?);
                }
                let kind = match op {
                    IntentOp::CreatePlayer(_) => IntentKind::Player,
                    IntentOp::CreateEnemy(_) => IntentKind::Enemy,
                    _ => IntentKind::Camera,
                };
                specs.push(Spec {
                    name: args.name.clone(),
                    kind,
                    wants,
                });
            }
            IntentOp::Arrange(args) => {
                let points = arrange_points(&args.pattern, args.names.len(), half, seed)?;
                for (name, (x_cm, z_cm)) in args.names.iter().zip(points.iter().copied()) {
                    specs.push(Spec {
                        name: name.clone(),
                        kind: args.kind,
                        wants: vec![Want {
                            relation: Relation::NextTo,
                            target: Target::Point { x_cm, z_cm },
                            distance_cm: None,
                        }],
                    });
                }
            }
            IntentOp::Furnish(items) => {
                for item in items {
                    let mut wants = Vec::with_capacity(item.wants.len());
                    for want in &item.wants {
                        wants.push(resolve_want(want, &index_of)?);
                    }
                    specs.push(Spec {
                        name: item.name.clone(),
                        kind: item.kind,
                        wants,
                    });
                }
            }
        }
    }
    // Stage: walls plus deterministic slots, then solve in brief order.
    let mut actors: Vec<Actor> = existing.to_vec();
    let mut new_ids: Vec<u32> = Vec::new();
    if bootstrapped {
        for wall in wall_actors(half) {
            new_ids.push(wall.id);
            actors.push(wall);
        }
    }
    for (slot, spec) in specs.iter().enumerate() {
        let id = index_of[spec.name.as_str()];
        let (sx, sy, sz) = spec.kind.size();
        actors.push(Actor {
            id,
            name: spec.name.clone(),
            x: -4.0 + (slot % 6) as f32 * 1.6,
            y: sy * 0.5,
            z: 3.2 - (slot / 6) as f32 * 1.6,
            sx,
            sy,
            sz,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        });
        new_ids.push(id);
    }
    let touched: BTreeSet<u32> = new_ids.iter().copied().collect();
    let mut placements = Vec::with_capacity(specs.len());
    for spec in &specs {
        let id = index_of[spec.name.as_str()];
        let intent = Intent {
            object: ObjectRef::Id(id),
            wants: spec.wants.clone(),
        };
        match engine_spatial::solve(&intent, &actors) {
            Ok(out) => {
                let Some(first) = out.into_iter().next() else {
                    return Err(IntentError::Unplaceable {
                        name: spec.name.clone(),
                        reason: "solver returned no placement and no conflict".to_string(),
                        conflicts: spec.wants.iter().map(want_text).collect(),
                    });
                };
                if let Some(slot) = actors.iter().position(|a| a.id == id) {
                    actors[slot].x = first.x;
                    actors[slot].y = first.y;
                    actors[slot].z = first.z;
                    actors[slot].yaw = first.yaw;
                }
                placements.push(Placement {
                    name: spec.name.clone(),
                    kind: spec.kind,
                    id,
                    x: first.x,
                    y: first.y,
                    z: first.z,
                    yaw: first.yaw,
                });
            }
            Err(conflict) => {
                return Err(IntentError::Unplaceable {
                    name: spec.name.clone(),
                    reason: conflict.reason,
                    conflicts: conflict.unmet.iter().map(want_text).collect(),
                });
            }
        }
    }
    //Solved placements must verify clean: any finding touching a new actor
    // refuses the brief before anything is written.
    let report = engine_spatial::run(&actors);
    let failed: Vec<String> = report
        .findings
        .iter()
        .filter(|finding| {
            touched.contains(&finding.a) || (finding.b != u32::MAX && touched.contains(&finding.b))
        })
        .map(engine_spatial::Finding::row)
        .collect();
    if !failed.is_empty() {
        return Err(IntentError::CheckFailed { findings: failed });
    }
    let raw_ops = new_ids.len();
    Ok(Compiled {
        placements,
        actors,
        new_ids,
        raw_ops,
    })
}

/// Receipt for one applied brief: a single journal entry.
#[derive(Debug, Clone, PartialEq)]
pub struct IntentReceipt {
    /// Document revision after the apply.
    pub revision: u64,
    /// The batch key this receipt answers (also the undo token).
    pub key: u64,
    /// One line per insert, in apply order.
    pub effects: Vec<String>,
    /// Brief names with their minted ids, in apply order.
    pub ids: Vec<(String, ObjectId)>,
    /// Solved placements in brief order.
    pub placements: Vec<Placement>,
    /// Raw placement ops this batch replaces.
    pub raw_ops: usize,
}

fn apply_failed(error: commands::BatchError) -> IntentError {
    match error {
        commands::BatchError::StaleRevision { expected, got } => IntentError::ApplyFailed {
            reason: format!(
                "document is at revision {expected}, batch expects {got}; \
                 re-read the scene, then build again"
            ),
        },
        commands::BatchError::DuplicateKey { key } => IntentError::ApplyFailed {
            reason: format!(
                "idempotency key {key} was already used with a different payload; \
                 submit a fresh key"
            ),
        },
        commands::BatchError::UndoneKey { key } => IntentError::ApplyFailed {
            reason: format!(
                "idempotency key {key} belongs to an undone batch; \
                 submit a fresh key to rebuild"
            ),
        },
        other => IntentError::ApplyFailed {
            reason: format!("batch refused: {other:?}; fix the brief and rebuild"),
        },
    }
}

/// Insert parts for one compiled id: walls read the staged actors, props
/// read the solved placements. Missing entries refuse with typed
/// `Internal` so MCP callers never see a panic.
fn insert_parts(
    compiled: &Compiled,
    by_id: &BTreeMap<u32, &Placement>,
    id: u32,
) -> Result<([f32; 3], [f32; 3], String, f32), IntentError> {
    if id >= WALL_BASE {
        let Some(wall) = compiled.actors.iter().find(|a| a.id == id) else {
            return Err(IntentError::Internal {
                reason: format!("wall actor {id} missing from compiled stage"),
            });
        };
        Ok((
            [wall.x, wall.y, wall.z],
            [wall.sx, wall.sy, wall.sz],
            wall.name.clone(),
            0.0,
        ))
    } else {
        let Some(placed) = by_id.get(&id) else {
            return Err(IntentError::Internal {
                reason: format!("placement {id} missing from compiled stage"),
            });
        };
        let (sx, sy, sz) = placed.kind.size();
        Ok((
            [placed.x, placed.y, placed.z],
            [sx, sy, sz],
            placed.name.clone(),
            placed.yaw,
        ))
    }
}

/// Build one brief against the live document in a single significant batch:
/// validate (dry run, no side effects), then atomic apply. One undo reverts
/// the whole brief.
pub fn apply_brief(
    doc: &mut Document,
    journal: &mut Journal,
    ops: &[IntentOp],
    seed: u64,
    key: u64,
) -> Result<IntentReceipt, IntentError> {
    let (existing, _) = snapshot(doc);
    let compiled = compile(ops, &existing, seed)?;
    let by_id: BTreeMap<u32, &Placement> = compiled.placements.iter().map(|p| (p.id, p)).collect();
    // Inserts in actor order (walls first, then brief order): deterministic.
    let mut ordered: Vec<u32> = compiled.new_ids.clone();
    ordered.sort_unstable();
    let mut inserts = Vec::with_capacity(ordered.len());
    let mut receipt_ids = Vec::with_capacity(ordered.len());
    for id in ordered {
        let fresh = ObjectId::new();
        let (pos, scale, name, yaw) = insert_parts(&compiled, &by_id, id)?;
        inserts.push(Op::Insert {
            id: fresh,
            parent: ObjectId::NIL,
            local: Transform::new(pos, quat_of_yaw(yaw), scale),
        });
        receipt_ids.push((name, fresh));
    }
    let batch = Batch {
        ops: inserts,
        base_revision: doc.revision,
        idempotency_key: key,
        provenance: Some(ProvenanceStamp::script("intent")),
    };
    commands::validate(doc, &batch).map_err(apply_failed)?;
    let meta = CommandMeta::significant(format!("intent batch ({} new)", compiled.raw_ops));
    let applied = commands::apply_named(doc, journal, &batch, &meta).map_err(apply_failed)?;
    Ok(IntentReceipt {
        revision: applied.revision,
        key: applied.key,
        effects: applied.effects,
        ids: receipt_ids,
        placements: compiled.placements,
        raw_ops: compiled.raw_ops,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> (Document, Journal) {
        (Document::new(), Journal::new())
    }

    fn bootstrap(half: f32) -> IntentOp {
        IntentOp::Bootstrap(BootstrapArgs {
            half_extent_m: half,
        })
    }

    fn player(name: &str, wants: Vec<NamedWant>) -> IntentOp {
        IntentOp::CreatePlayer(CreateArgs {
            name: name.into(),
            wants,
        })
    }

    fn enemy(name: &str, wants: Vec<NamedWant>) -> IntentOp {
        IntentOp::CreateEnemy(CreateArgs {
            name: name.into(),
            wants,
        })
    }

    fn camera(name: &str, wants: Vec<NamedWant>) -> IntentOp {
        IntentOp::CreateCamera(CreateArgs {
            name: name.into(),
            wants,
        })
    }

    fn wall_next(wall: Wall) -> NamedWant {
        NamedWant {
            relation: Relation::NextTo,
            target: NamedTarget::Wall(wall),
            distance_cm: None,
        }
    }

    fn wall_range(wall: Wall, lo: i32, hi: i32) -> NamedWant {
        NamedWant {
            relation: Relation::NextTo,
            target: NamedTarget::Wall(wall),
            distance_cm: Some((lo, hi)),
        }
    }

    fn arrange(kind: IntentKind, names: &[&str], pattern: ArrangePattern) -> IntentOp {
        IntentOp::Arrange(ArrangeArgs {
            kind,
            names: names.iter().map(|n| (*n).into()).collect(),
            pattern,
        })
    }

    fn furnish(items: Vec<(&str, IntentKind, Vec<NamedWant>)>) -> IntentOp {
        IntentOp::Furnish(
            items
                .into_iter()
                .map(|(name, kind, wants)| FurnishItem {
                    kind,
                    name: name.into(),
                    wants,
                })
                .collect(),
        )
    }

    struct Case {
        name: &'static str,
        ops: Vec<IntentOp>,
        seed: u64,
        expect_new: usize,
    }

    /// Twenty briefs spanning every macro: bootstrap, the three creates,
    /// all four arrange patterns on every wall, furnish mixes and full
    /// combos on three room sizes.
    fn cases() -> Vec<Case> {
        vec![
            Case {
                name: "bootstrap_only",
                ops: vec![bootstrap(5.0)],
                seed: 1,
                expect_new: 4,
            },
            Case {
                name: "player",
                ops: vec![bootstrap(5.0), player("hero", vec![wall_next(Wall::North)])],
                seed: 2,
                expect_new: 5,
            },
            Case {
                name: "enemy",
                ops: vec![bootstrap(5.0), enemy("brute", vec![wall_next(Wall::South)])],
                seed: 3,
                expect_new: 5,
            },
            Case {
                name: "camera",
                ops: vec![bootstrap(5.0), camera("cam", vec![wall_next(Wall::East)])],
                seed: 4,
                expect_new: 5,
            },
            Case {
                name: "duel",
                ops: vec![
                    bootstrap(5.0),
                    player("hero", vec![wall_next(Wall::North)]),
                    enemy("brute", vec![wall_range(Wall::South, 100, 300)]),
                ],
                seed: 5,
                expect_new: 6,
            },
            Case {
                name: "grid_crates",
                ops: vec![
                    bootstrap(5.0),
                    arrange(
                        IntentKind::Crate,
                        &["c1", "c2", "c3", "c4"],
                        ArrangePattern::Grid {
                            rows: 2,
                            cols: 2,
                            spacing_m: 1.5,
                        },
                    ),
                ],
                seed: 6,
                expect_new: 8,
            },
            Case {
                name: "grid_torches",
                ops: vec![
                    bootstrap(5.0),
                    arrange(
                        IntentKind::Torch,
                        &["t1", "t2", "t3", "t4", "t5", "t6"],
                        ArrangePattern::Grid {
                            rows: 3,
                            cols: 2,
                            spacing_m: 1.2,
                        },
                    ),
                ],
                seed: 7,
                expect_new: 10,
            },
            Case {
                name: "circle_posts",
                ops: vec![
                    bootstrap(5.0),
                    arrange(
                        IntentKind::Post,
                        &["p1", "p2", "p3", "p4", "p5", "p6"],
                        ArrangePattern::Circle { radius_m: 2.0 },
                    ),
                ],
                seed: 8,
                expect_new: 10,
            },
            Case {
                name: "circle_pillars",
                ops: vec![
                    bootstrap(5.0),
                    arrange(
                        IntentKind::Pillar,
                        &["a1", "a2", "a3", "a4"],
                        ArrangePattern::Circle { radius_m: 2.5 },
                    ),
                ],
                seed: 9,
                expect_new: 8,
            },
            Case {
                name: "scatter_crates",
                ops: vec![
                    bootstrap(5.0),
                    arrange(
                        IntentKind::Crate,
                        &["s1", "s2", "s3", "s4", "s5"],
                        ArrangePattern::Scatter,
                    ),
                ],
                seed: 10,
                expect_new: 9,
            },
            Case {
                name: "scatter_torches",
                ops: vec![
                    bootstrap(5.0),
                    arrange(
                        IntentKind::Torch,
                        &["u1", "u2", "u3", "u4", "u5", "u6", "u7", "u8"],
                        ArrangePattern::Scatter,
                    ),
                ],
                seed: 11,
                expect_new: 12,
            },
            Case {
                name: "wall_north_crates",
                ops: vec![
                    bootstrap(5.0),
                    arrange(
                        IntentKind::Crate,
                        &["w1", "w2", "w3", "w4"],
                        ArrangePattern::AlongWall { wall: Wall::North },
                    ),
                ],
                seed: 12,
                expect_new: 8,
            },
            Case {
                name: "wall_east_braziers",
                ops: vec![
                    bootstrap(5.0),
                    arrange(
                        IntentKind::Brazier,
                        &["b1", "b2", "b3"],
                        ArrangePattern::AlongWall { wall: Wall::East },
                    ),
                ],
                seed: 13,
                expect_new: 7,
            },
            Case {
                name: "wall_south_banners",
                ops: vec![
                    bootstrap(5.0),
                    arrange(
                        IntentKind::Banner,
                        &["g1", "g2", "g3", "g4"],
                        ArrangePattern::AlongWall { wall: Wall::South },
                    ),
                ],
                seed: 14,
                expect_new: 8,
            },
            Case {
                name: "wall_west_posts",
                ops: vec![
                    bootstrap(5.0),
                    arrange(
                        IntentKind::Post,
                        &["q1", "q2", "q3"],
                        ArrangePattern::AlongWall { wall: Wall::West },
                    ),
                ],
                seed: 15,
                expect_new: 7,
            },
            Case {
                name: "furnish_corner",
                ops: vec![
                    bootstrap(5.0),
                    furnish(vec![
                        ("c1", IntentKind::Crate, vec![wall_next(Wall::North)]),
                        (
                            "b1",
                            IntentKind::Brazier,
                            vec![NamedWant {
                                relation: Relation::NextTo,
                                target: NamedTarget::Object("c1".into()),
                                distance_cm: None,
                            }],
                        ),
                        (
                            "s1",
                            IntentKind::Bench,
                            vec![NamedWant {
                                relation: Relation::Facing,
                                target: NamedTarget::Wall(Wall::South),
                                distance_cm: None,
                            }],
                        ),
                    ]),
                ],
                seed: 16,
                expect_new: 7,
            },
            Case {
                name: "furnish_hall",
                ops: vec![
                    bootstrap(5.0),
                    furnish(vec![
                        ("altar", IntentKind::Altar, vec![]),
                        (
                            "bench_w",
                            IntentKind::Bench,
                            vec![NamedWant {
                                relation: Relation::NextTo,
                                target: NamedTarget::Object("altar".into()),
                                distance_cm: None,
                            }],
                        ),
                        (
                            "bench_e",
                            IntentKind::Bench,
                            vec![NamedWant {
                                relation: Relation::NextTo,
                                target: NamedTarget::Object("altar".into()),
                                distance_cm: None,
                            }],
                        ),
                        ("torch_n", IntentKind::Torch, vec![wall_next(Wall::North)]),
                        ("torch_s", IntentKind::Torch, vec![wall_next(Wall::South)]),
                    ]),
                ],
                seed: 17,
                expect_new: 9,
            },
            Case {
                name: "grand_hall",
                ops: vec![
                    bootstrap(8.0),
                    player("hero", vec![wall_next(Wall::North)]),
                    enemy("brute", vec![wall_next(Wall::South)]),
                    camera("cam", vec![wall_next(Wall::East)]),
                    arrange(
                        IntentKind::Crate,
                        &["k1", "k2", "k3", "k4", "k5", "k6", "k7", "k8", "k9"],
                        ArrangePattern::Grid {
                            rows: 3,
                            cols: 3,
                            spacing_m: 1.5,
                        },
                    ),
                    furnish(vec![
                        ("fire", IntentKind::Brazier, vec![wall_next(Wall::West)]),
                        ("flag", IntentKind::Banner, vec![wall_next(Wall::North)]),
                    ]),
                ],
                seed: 18,
                expect_new: 18,
            },
            Case {
                name: "small_room",
                ops: vec![
                    bootstrap(3.0),
                    player("hero", vec![wall_next(Wall::North)]),
                    furnish(vec![(
                        "lamp",
                        IntentKind::Torch,
                        vec![NamedWant {
                            relation: Relation::NextTo,
                            target: NamedTarget::Object("hero".into()),
                            distance_cm: None,
                        }],
                    )]),
                ],
                seed: 19,
                expect_new: 6,
            },
            Case {
                name: "furnish_point",
                ops: vec![furnish(vec![
                    (
                        "cache",
                        IntentKind::Crate,
                        vec![NamedWant {
                            relation: Relation::NextTo,
                            target: NamedTarget::Point { x_cm: 0, z_cm: 200 },
                            distance_cm: None,
                        }],
                    ),
                    (
                        "stash",
                        IntentKind::Crate,
                        vec![NamedWant {
                            relation: Relation::NextTo,
                            target: NamedTarget::Point {
                                x_cm: -200,
                                z_cm: 200,
                            },
                            distance_cm: None,
                        }],
                    ),
                ])],
                seed: 20,
                expect_new: 2,
            },
        ]
    }

    fn staged_actors(doc: &Document) -> Vec<Actor> {
        snapshot(doc).0
    }

    #[test]
    fn twenty_briefs_build_with_fewer_calls_clean_checks_and_one_undo() {
        let mut total_raw = 0usize;
        let mut total_intent = 0usize;
        for case in cases() {
            let (mut doc, mut journal) = empty();
            // Furnish-point builds against a lived scene (no bootstrap).
            if case.name == "furnish_point" {
                for (i, x) in [-3.0f32, 3.0].iter().enumerate() {
                    let batch = Batch {
                        ops: vec![Op::Insert {
                            id: ObjectId::new(),
                            parent: ObjectId::NIL,
                            local: Transform::new(
                                [*x, 0.4, 0.0],
                                [0.0, 0.0, 0.0, 1.0],
                                [0.8, 0.8, 0.8],
                            ),
                        }],
                        base_revision: doc.revision,
                        idempotency_key: 900 + i as u64,
                        provenance: None,
                    };
                    commands::apply(&mut doc, &mut journal, &batch).expect("seed applies");
                }
            }
            let pre_doc = doc.clone();
            let pre_past = journal.past_len();
            let receipt = apply_brief(
                &mut doc,
                &mut journal,
                &case.ops,
                case.seed,
                500 + case.seed,
            )
            .unwrap_or_else(|error| panic!("brief {} builds: {error}", case.name));
            // One batch replaces every raw op; walls carry no placements.
            assert_eq!(receipt.raw_ops, case.expect_new, "brief {}", case.name);
            let bootstrapped = case
                .ops
                .iter()
                .any(|op| matches!(op, IntentOp::Bootstrap(_)));
            assert_eq!(
                receipt.placements.len(),
                case.expect_new - usize::from(bootstrapped) * 4,
                "brief {}",
                case.name
            );
            // At least 50% fewer calls: 1 batch against N raw ops.
            assert!(
                (receipt.raw_ops - 1) * 100 >= 50 * receipt.raw_ops,
                "brief {}: 1 batch vs {} raw ops",
                case.name,
                receipt.raw_ops
            );
            total_raw += receipt.raw_ops;
            total_intent += 1;
            // Same checks clean over the staged scene.
            let report = engine_spatial::run(&staged_actors(&doc));
            assert_eq!(
                report.verdict(),
                "clean",
                "brief {}: {:?}",
                case.name,
                report.findings.iter().map(engine_spatial::Finding::row).collect::<Vec<_>>()
            );
            // Exactly one journal entry: one undo reverts the brief.
            assert_eq!(journal.past_len(), pre_past + 1, "brief {}", case.name);
            let undone = commands::undo(&mut doc, &mut journal).expect("brief undoes");
            assert_eq!(undone.revision, pre_doc.revision + 2, "brief {}", case.name);
            // Undo seals a new revision by design, so the revert proof
            // compares staged actors, not the revision counter.
            assert_eq!(
                staged_actors(&doc),
                staged_actors(&pre_doc),
                "brief {} reverts exactly",
                case.name
            );
            assert_eq!(journal.past_len(), pre_past, "brief {}", case.name);
        }
        // Aggregate: one batch per brief against every raw op.
        assert!(
            (total_raw - total_intent) * 100 >= 50 * total_raw,
            "aggregate: {total_intent} batches vs {total_raw} raw ops"
        );
    }

    #[test]
    fn same_brief_same_seed_places_identically_twice() {
        for case in cases() {
            if case.name == "furnish_point" {
                continue;
            }
            let first = compile(&case.ops, &[], case.seed)
                .unwrap_or_else(|error| panic!("brief {} compiles: {error}", case.name));
            let second = compile(&case.ops, &[], case.seed).expect("recompiles");
            assert_eq!(first, second, "brief {} is deterministic", case.name);
        }
        // Scatter moves with the seed (the LCG actually feeds it).
        let scatter = || {
            compile(&cases()[9].ops, &[], cases()[9].seed)
                .expect("scatter compiles")
                .placements
        };
        let other = compile(&cases()[9].ops, &[], 999).expect("other seed compiles");
        assert_ne!(scatter(), other.placements, "seeds move scatter");
    }

    #[test]
    fn kinds_parse_and_name_roundtrip() {
        for word in IntentKind::KINDS {
            assert_eq!(IntentKind::parse(word).expect("kind parses").name(), *word);
        }
        assert_eq!(
            IntentKind::parse("trebuchet")
                .expect_err("unknown kind refuses")
                .code(),
            "unknown_kind"
        );
        assert!(IntentError::UnknownKind { kind: "x".into() }
            .to_string()
            .contains("player"));
    }

    #[test]
    fn refusals_name_the_rule_and_write_nothing() {
        let (mut doc, mut journal) = empty();
        let pre = doc.clone();
        // Empty brief.
        assert_eq!(
            apply_brief(&mut doc, &mut journal, &[], 1, 1)
                .expect_err("empty refuses")
                .code(),
            "empty_brief"
        );
        // Duplicate names across macros.
        let dup = vec![
            bootstrap(5.0),
            player("hero", vec![]),
            enemy("hero", vec![]),
        ];
        assert_eq!(
            apply_brief(&mut doc, &mut journal, &dup, 1, 2)
                .expect_err("dup refuses")
                .code(),
            "duplicate_name"
        );
        // Bad grammar.
        let bad = vec![bootstrap(5.0), player("Hero-1", vec![])];
        assert_eq!(
            apply_brief(&mut doc, &mut journal, &bad, 1, 3)
                .expect_err("bad name refuses")
                .code(),
            "bad_name"
        );
        // Second bootstrap.
        let twice = vec![bootstrap(5.0), bootstrap(5.0)];
        assert_eq!(
            apply_brief(&mut doc, &mut journal, &twice, 1, 4)
                .expect_err("double bootstrap refuses")
                .code(),
            "double_bootstrap"
        );
        // Object want naming nobody.
        let ghost = vec![
            bootstrap(5.0),
            furnish(vec![(
                "lonely",
                IntentKind::Crate,
                vec![NamedWant {
                    relation: Relation::NextTo,
                    target: NamedTarget::Object("ghost".into()),
                    distance_cm: None,
                }],
            )]),
        ];
        assert_eq!(
            apply_brief(&mut doc, &mut journal, &ghost, 1, 5)
                .expect_err("ghost refuses")
                .code(),
            "unknown_name"
        );
        // Inverted range.
        let range = vec![
            bootstrap(5.0),
            player("hero", vec![wall_range(Wall::North, 300, 100)]),
        ];
        assert_eq!(
            apply_brief(&mut doc, &mut journal, &range, 1, 6)
                .expect_err("range refuses")
                .code(),
            "bad_range"
        );
        // Impossible intent: near both walls at once (each alone solves).
        let stuck = vec![
            bootstrap(5.0),
            player(
                "hero",
                vec![
                    wall_range(Wall::North, 0, 100),
                    wall_range(Wall::South, 0, 100),
                ],
            ),
        ];
        let conflict =
            apply_brief(&mut doc, &mut journal, &stuck, 1, 7).expect_err("impossible refuses");
        assert_eq!(conflict.code(), "unplaceable");
        let text = conflict.to_string();
        assert!(text.contains("wall:north"), "got: {text}");
        // Grid headcount mismatch.
        let grid = vec![
            bootstrap(5.0),
            arrange(
                IntentKind::Crate,
                &["a", "b", "c"],
                ArrangePattern::Grid {
                    rows: 2,
                    cols: 2,
                    spacing_m: 1.5,
                },
            ),
        ];
        assert_eq!(
            apply_brief(&mut doc, &mut journal, &grid, 1, 8)
                .expect_err("grid refuses")
                .code(),
            "bad_pattern"
        );
        // Nothing ever wrote.
        assert_eq!(doc, pre, "refusals write nothing");
        assert_eq!(journal.past_len(), 0, "refusals journal nothing");
    }

    #[test]
    fn duplicate_key_replays_or_refuses_loudly() {
        let (mut doc, mut journal) = empty();
        // Seed the room first: the rebuild then compiles cleanly against the
        // lived scene, so the refused second apply isolates the key gate.
        apply_brief(&mut doc, &mut journal, &[bootstrap(5.0)], 1, 100).expect("room builds");
        let ops = vec![furnish(vec![("box1", IntentKind::Crate, vec![])])];
        let first = apply_brief(&mut doc, &mut journal, &ops, 1, 4242).expect("builds");
        assert_eq!(first.key, 4242);
        // Same key with fresh random ids is a key conflict, refused loudly.
        let replay =
            apply_brief(&mut doc, &mut journal, &ops, 1, 4242).expect_err("key reuse refuses");
        assert_eq!(replay.code(), "apply_failed");
        let text = replay.to_string();
        assert!(text.contains("fresh key"), "got: {text}");
        assert_eq!(
            journal.past_len(),
            2,
            "the refused rebuild journals nothing"
        );
        let _ = first;
    }

    #[test]
    fn corrupt_stage_refuses_with_typed_internal() {
        let empty_compiled = Compiled {
            placements: Vec::new(),
            actors: Vec::new(),
            new_ids: Vec::new(),
            raw_ops: 0,
        };
        let by_id: BTreeMap<u32, &Placement> = BTreeMap::new();
        // Wall id with no staged wall: the old `expect` panicked here.
        let missing_wall =
            insert_parts(&empty_compiled, &by_id, WALL_BASE).expect_err("wall refuses");
        assert_eq!(missing_wall.code(), "internal");
        assert!(
            missing_wall.to_string().contains("rebuild"),
            "got: {missing_wall}"
        );
        // Prop id with no solved placement: the old index panicked here.
        let missing_prop = insert_parts(&empty_compiled, &by_id, 7).expect_err("prop refuses");
        assert_eq!(missing_prop.code(), "internal");
        assert!(
            missing_prop.to_string().contains("placement 7"),
            "got: {missing_prop}"
        );
    }

    #[test]
    fn staged_inserts_cover_walls_and_props_without_internal() {
        let (mut doc, mut journal) = empty();
        let ops = vec![bootstrap(5.0), player("hero", vec![wall_next(Wall::North)])];
        let receipt = apply_brief(&mut doc, &mut journal, &ops, 11, 311).expect("builds");
        // Four walls plus one solved prop, all through the typed insert path.
        assert_eq!(receipt.raw_ops, 5);
        assert_eq!(receipt.placements.len(), 1);
        let compiled = compile(&ops, &[], 11).expect("compiles");
        let by_id: BTreeMap<u32, &Placement> =
            compiled.placements.iter().map(|p| (p.id, p)).collect();
        for id in &compiled.new_ids {
            insert_parts(&compiled, &by_id, *id).expect("staged id resolves");
        }
    }
}
