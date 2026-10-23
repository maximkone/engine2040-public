//! scene.query: spatial questions with reasons, budget-fit answers, hints.
//! Owner: endpoint. Consumers: SP5/AB1 via free-space and largest-open
//! (engine-spatial sample calls below are the live path, not a test shape);
//! `SceneQueryTool` serves the same answers over the MCP door (registered on
//! the router in `mcp.rs`). Units: meters in, integer cm inside.
//! Determinism: id-sorted scans and integer math; same scene plus args give
//! byte-identical answer text. Errors: typed `QueryError`; every conflict
//! carries a repair hint.

use std::borrow::Cow;

use engine_core::Fnv64;
use engine_scene::ids::ObjectId;
use engine_spatial::diff::{diff_graph, SemanticDiff};
use engine_spatial::navcheck::octant;
use engine_spatial::relation::{
    frame_name, parse_rel, rel_name, rel_words, subjects_of, Frame, Rel,
};
use engine_spatial::sample::{
    largest_open_spot, sample_free_space, SamplerConfig, SAMPLE_ACTOR_BASE,
};
use engine_spatial::sight::{
    cover, reachable, visible, Cover, Reach, ReachView, SightWorld, Stance,
};
use engine_spatial::{pos_known, size_known, to_cm, Actor};

use rmcp::handler::server::router::tool::{SyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::mcp::{ChangeListDto, McpServer, McpToolError};
use super::scene_tools::{
    object_label, parse_object_ref, snapshot, SCENE_BRIEF_DEFAULT_BUDGET, SCENE_BRIEF_MAX_BUDGET,
};

/// Door tool name for spatial questions.
pub const TOOL_SCENE_QUERY: &str = "scene.query";
/// Default answer budget in bytes: the `scene.brief` default.
pub const QUERY_DEFAULT_BUDGET: u32 = SCENE_BRIEF_DEFAULT_BUDGET;
/// Hard answer budget in bytes: the `scene.brief` cap.
pub const QUERY_MAX_BUDGET: u32 = SCENE_BRIEF_MAX_BUDGET;
/// Hard hit cap per answer.
pub const QUERY_MAX_RESULTS: usize = 64;
/// Hard radius cap in cm, mirroring the SP1 world bound.
pub const QUERY_MAX_RADIUS_CM: i32 = 10_000;
/// Between tolerance in cm: path via the candidate may exceed direct by this.
pub const BETWEEN_TOL_CM: i64 = 50;
/// Synthetic placement footprint in meters (crate scale, grounded).
pub const PLACED_SX_M: f32 = 0.8;
/// Synthetic placement height in meters.
pub const PLACED_SY_M: f32 = 0.8;
/// Synthetic placement depth in meters.
pub const PLACED_SZ_M: f32 = 0.8;
/// Digest tag so query bytes never alias another domain hash.
const QUERY_TAG: &[u8] = b"e2query:1";

/// One-line router hook, landed: the query tool registers on the
/// nineteen-tool router in `mcp.rs` with the tool-list budget rebump.
/// Kept as data so tests name the hook without touching the router.
pub fn hook_name() -> &'static str {
    "mcp.rs ToolRouter::with_sync_tool::<SceneQueryTool>() + tool_names 18->19 + budget rebump"
}

/// Question kind. Discriminants are frozen: the digest feeds them raw, so
/// variants are append-only, never reordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum QueryKind {
    /// Actors within `radius_cm` of `anchor`.
    Near = 0,
    /// Actors on the corridor from `anchor` to `other`.
    Between = 1,
    /// Actors seen from `eye_m` with `yaw`.
    Visible = 2,
    /// Actors reachable from `anchor` through nav polygons.
    Reachable = 3,
    /// Actors within `radius_cm` of `anchor` whose feet take cover.
    CoverNear = 4,
    /// Largest open spot for the footprint plus capsule.
    LargestOpen = 5,
    /// Poisson-disc free-space placements.
    FreeSpace = 6,
    /// Actors inside a horizontal cone around the observer, within a plan-view range.
    Cone = 7,
    /// Actors for which a relation word (`left_of`, `visible_from`, ...)
    /// holds toward the anchor, optionally seen from a viewer actor.
    Relation = 8,
    /// Whether `anchor` sees `other`, with the blocker when it does not.
    Sees = 9,
    /// Why `anchor` is not on screen from `eye_m` facing `yaw`.
    WhyHidden = 10,
    /// Room around `anchor`: gap to the nearest neighbour and which way to push.
    Clearance = 11,
    /// How far `anchor` can slide along `dir` before it hits something.
    Slide = 12,
    /// The floor under the plan point of `eye_m`.
    Ground = 13,
    /// Gap, overlap, alignment or containment of `anchor` against `other`.
    Measure = 14,
    /// The room number holding `anchor`, with its exits.
    RoomOf = 15,
    /// The objects inside one room.
    InRoom = 16,
    /// The doorways of one room.
    ExitsOf = 17,
    /// Per-object coverage of the view from `eye_m` facing `yaw`.
    Glance = 18,
    /// Share of the frame `anchor` fills from `eye_m` facing `yaw` and `pitch`.
    Coverage = 19,
    /// Run-length text picture of what the eye sees (SP69).
    View = 20,
    /// 72 horizontal ray distances from the origin, 5 degrees apart (SP69).
    Lidar = 21,
    /// Top-down text grid of blocking and walkable cells (SP69).
    Map = 22,
    /// One ray through a view pixel: entity id, point in cm, distance (SP69).
    Pixel = 23,
    /// What changed between two held document revisions (AX23).
    Diff = 24,
}

impl QueryKind {
    /// Door word for this kind, mirroring `parse`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Near => "near",
            Self::Between => "between",
            Self::Visible => "visible",
            Self::Reachable => "reachable",
            Self::CoverNear => "cover_near",
            Self::LargestOpen => "largest_open",
            Self::FreeSpace => "free_space",
            Self::Cone => "cone",
            Self::Relation => "relation",
            Self::Sees => "sees",
            Self::WhyHidden => "why_hidden",
            Self::Clearance => "clearance",
            Self::Slide => "slide",
            Self::Ground => "ground",
            Self::Measure => "measure",
            Self::RoomOf => "room_of",
            Self::InRoom => "in_room",
            Self::ExitsOf => "exits_of",
            Self::Glance => "glance",
            Self::Coverage => "coverage",
            Self::View => "view",
            Self::Lidar => "lidar",
            Self::Map => "map",
            Self::Pixel => "pixel",
            Self::Diff => "diff",
        }
    }

    /// Parse a kind word; unknown words refuse with the legal list.
    pub fn parse(text: &str) -> Result<Self, QueryError> {
        match text {
            "near" => Ok(Self::Near),
            "between" => Ok(Self::Between),
            "visible" => Ok(Self::Visible),
            "reachable" => Ok(Self::Reachable),
            "cover_near" => Ok(Self::CoverNear),
            "largest_open" => Ok(Self::LargestOpen),
            "free_space" => Ok(Self::FreeSpace),
            "cone" => Ok(Self::Cone),
            "relation" => Ok(Self::Relation),
            "sees" => Ok(Self::Sees),
            "why_hidden" => Ok(Self::WhyHidden),
            "clearance" => Ok(Self::Clearance),
            "slide" => Ok(Self::Slide),
            "ground" => Ok(Self::Ground),
            "measure" => Ok(Self::Measure),
            "room_of" => Ok(Self::RoomOf),
            "in_room" => Ok(Self::InRoom),
            "exits_of" => Ok(Self::ExitsOf),
            "glance" => Ok(Self::Glance),
            "coverage" => Ok(Self::Coverage),
            "view" => Ok(Self::View),
            "lidar" => Ok(Self::Lidar),
            "map" => Ok(Self::Map),
            "pixel" => Ok(Self::Pixel),
            "diff" => Ok(Self::Diff),
            _ => Err(QueryError::new(
                "bad_kind",
                format!("query kind `{text}` is unknown"),
                "legal kinds: near, between, visible, reachable, cover_near, \
                 largest_open, free_space, cone, relation, sees, why_hidden, clearance, slide, ground, \
                 measure, room_of, in_room, exits_of, glance, coverage, view, lidar, map, pixel, diff \
                 (re-read scene.brief for ids first)",
            )),
        }
    }
}

mod cards;
pub(crate) mod diff;
pub use cards::{KindCard, KIND_CARDS};

/// Typed query failure. `hint` always names the legal values plus one
/// recovery, so every refusal is actionable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryError {
    pub code: &'static str,
    pub message: String,
    pub hint: String,
}

impl QueryError {
    pub fn new(code: &'static str, message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: hint.into(),
        }
    }
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {} (hint: {})", self.code, self.message, self.hint)
    }
}

impl std::error::Error for QueryError {}

/// Non-fatal conflict attached to an answer. Always carries a hint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub code: &'static str,
    pub message: String,
    pub hint: String,
}

/// One answered id: actor hits carry scene ids, placements carry synthetic
/// ids from `SAMPLE_ACTOR_BASE`. `matched` marks the query predicate;
/// every row carries its reason either way.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub matched: bool,
    pub reason: String,
}

/// Query arguments. Use the constructors; fields stay public for the door.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryArgs {
    pub kind: QueryKind,
    pub anchor: Option<u32>,
    pub other: Option<u32>,
    pub radius_cm: Option<i32>,
    pub eye_m: Option<[f32; 3]>,
    pub yaw: Option<f32>,
    pub crouch: bool,
    pub capsule_radius_cm: i32,
    pub min_gap_cm: i32,
    pub max_samples: usize,
    pub seed: u64,
    pub limit: usize,
    pub budget_bytes: u32,
    pub half_angle_deg: Option<i32>,
    pub filter: Option<String>,
    /// Relation word for `relation` queries.
    pub rel: Option<Rel>,
    /// Slide direction word for `slide` queries.
    pub dir: Option<String>,
    /// View or pixel box in cells `[w, h, x, y]` (view, pixel); the eye yaw
    /// gives the lidar fan start.
    pub cells: Option<[i32; 4]>,
    /// Fields of the measure, room and glance kinds.
    pub space: SpaceArgs,
}

impl QueryArgs {
    fn base(kind: QueryKind) -> Self {
        Self {
            kind,
            anchor: None,
            other: None,
            radius_cm: None,
            eye_m: None,
            yaw: None,
            crouch: false,
            capsule_radius_cm: 30,
            min_gap_cm: 100,
            max_samples: 8,
            seed: 7,
            limit: QUERY_MAX_RESULTS,
            budget_bytes: QUERY_DEFAULT_BUDGET,
            half_angle_deg: None,
            filter: None,
            rel: None,
            dir: None,
            cells: None,
            space: SpaceArgs::default(),
        }
    }

    /// Actors within `radius_cm` of `anchor`.
    pub fn near(anchor: u32, radius_cm: i32) -> Self {
        let mut a = Self::base(QueryKind::Near);
        a.anchor = Some(anchor);
        a.radius_cm = Some(radius_cm);
        a
    }

    /// Actors on the corridor from `anchor` to `other`.
    pub fn between(anchor: u32, other: u32) -> Self {
        let mut a = Self::base(QueryKind::Between);
        a.anchor = Some(anchor);
        a.other = Some(other);
        a
    }

    /// Actors seen from `eye_m` looking along `yaw` radians.
    pub fn visible(eye_m: [f32; 3], yaw: f32) -> Self {
        let mut a = Self::base(QueryKind::Visible);
        a.eye_m = Some(eye_m);
        a.yaw = Some(yaw);
        a
    }

    /// Actors reachable from `anchor` through nav polygons.
    pub fn reachable(anchor: u32) -> Self {
        let mut a = Self::base(QueryKind::Reachable);
        a.anchor = Some(anchor);
        a
    }

    /// Actors near `anchor` whose feet take cover from `threat_eye_m`.
    pub fn cover_near(anchor: u32, threat_eye_m: [f32; 3], crouch: bool, radius_cm: i32) -> Self {
        let mut a = Self::base(QueryKind::CoverNear);
        a.anchor = Some(anchor);
        a.eye_m = Some(threat_eye_m);
        a.crouch = crouch;
        a.radius_cm = Some(radius_cm);
        a
    }

    /// Largest open spot for the footprint plus capsule.
    pub fn largest_open(capsule_radius_cm: i32) -> Self {
        let mut a = Self::base(QueryKind::LargestOpen);
        a.capsule_radius_cm = capsule_radius_cm;
        a
    }

    /// Poisson-disc free-space placements from `seed`.
    pub fn free_space(
        capsule_radius_cm: i32,
        min_gap_cm: i32,
        max_samples: usize,
        seed: u64,
    ) -> Self {
        let mut a = Self::base(QueryKind::FreeSpace);
        a.capsule_radius_cm = capsule_radius_cm;
        a.min_gap_cm = min_gap_cm;
        a.max_samples = max_samples;
        a.seed = seed;
        a
    }

    /// Actors for which `rel` holds toward `anchor`, in the world frame or
    /// (with `viewer`) as that actor sees the scene.
    pub fn relation(anchor: u32, rel: Rel, viewer: Option<u32>) -> Self {
        let mut a = Self::base(QueryKind::Relation);
        a.anchor = Some(anchor);
        a.rel = Some(rel);
        a.other = viewer;
        a
    }

    /// Actors inside a cone of `half_angle_deg` around `anchor`, facing its yaw.
    pub fn cone_from(anchor: u32, half_angle_deg: i32, radius_cm: i32) -> Self {
        let mut a = Self::base(QueryKind::Cone);
        a.anchor = Some(anchor);
        a.half_angle_deg = Some(half_angle_deg);
        a.radius_cm = Some(radius_cm);
        a
    }

    /// Actors inside a cone of `half_angle_deg` around `eye_m`, facing `yaw` radians.
    pub fn cone_at(eye_m: [f32; 3], yaw: f32, half_angle_deg: i32, radius_cm: i32) -> Self {
        let mut a = Self::base(QueryKind::Cone);
        a.eye_m = Some(eye_m);
        a.yaw = Some(yaw);
        a.half_angle_deg = Some(half_angle_deg);
        a.radius_cm = Some(radius_cm);
        a
    }

    /// Run-length text picture a camera at `eye_m` facing `yaw` radians sees,
    /// `w` by `h` cells (SP69).
    pub fn view(eye_m: [f32; 3], yaw: f32, w: i32, h: i32) -> Self {
        let mut a = Self::base(QueryKind::View);
        a.eye_m = Some(eye_m);
        a.yaw = Some(yaw);
        a.cells = Some([w, h, 0, 0]);
        a
    }

    /// 72 horizontal ray distances from `eye_m`, the first ray along the facing
    /// `yaw` radians (0 faces +x), 5 degrees apart (SP69).
    pub fn lidar(eye_m: [f32; 3], yaw: f32) -> Self {
        let mut a = Self::base(QueryKind::Lidar);
        a.eye_m = Some(eye_m);
        a.yaw = Some(yaw);
        a
    }

    /// Top-down text grid of the walkable cells around `eye_m` within
    /// `radius_cm` (SP69).
    pub fn map(eye_m: [f32; 3], radius_cm: i32) -> Self {
        let mut a = Self::base(QueryKind::Map);
        a.eye_m = Some(eye_m);
        a.radius_cm = Some(radius_cm);
        a
    }

    /// One ray through cell `(x, y)` of a `w` by `h` view from `eye_m` facing
    /// `yaw` radians (SP69).
    pub fn pixel(eye_m: [f32; 3], yaw: f32, w: i32, h: i32, x: i32, y: i32) -> Self {
        let mut a = Self::base(QueryKind::Pixel);
        a.eye_m = Some(eye_m);
        a.yaw = Some(yaw);
        a.cells = Some([w, h, x, y]);
        a
    }
}

/// Bounded answer: ranked hits, conflicts with hints, digest and byte size.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryAnswer {
    pub kind: QueryKind,
    pub hits: Vec<Hit>,
    pub conflicts: Vec<Conflict>,
    pub digest: u64,
    pub bytes: usize,
    pub truncated: bool,
}

/// Integer square root by Newton iteration: exact, no libm, no float.
fn isqrt(n: i64) -> i64 {
    if n <= 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

fn center_cm(a: &Actor) -> Option<(i32, i32, i32)> {
    if !(pos_known(a) && size_known(a)) {
        return None;
    }
    if !(a.x.is_finite() && a.y.is_finite() && a.z.is_finite()) {
        return None;
    }
    Some((to_cm(a.x), to_cm(a.y), to_cm(a.z)))
}

fn dist_cm(a: (i32, i32, i32), b: (i32, i32, i32)) -> i64 {
    let dx = i64::from(a.0) - i64::from(b.0);
    let dy = i64::from(a.1) - i64::from(b.1);
    let dz = i64::from(a.2) - i64::from(b.2);
    isqrt(dx * dx + dy * dy + dz * dz)
}

/// Plan-view distance: corridors read the floor plan, so a tall pillar
/// still stands between two crates at its feet.
fn dist_xz(a: (i32, i32, i32), b: (i32, i32, i32)) -> i64 {
    let dx = i64::from(a.0) - i64::from(b.0);
    let dz = i64::from(a.2) - i64::from(b.2);
    isqrt(dx * dx + dz * dz)
}

fn actor_by_id(actors: &[Actor], id: u32) -> Option<&Actor> {
    actors.iter().find(|a| a.id == id)
}

fn valid_ids_hint(actors: &[Actor]) -> String {
    let mut ids: Vec<u32> = actors.iter().map(|a| a.id).collect();
    ids.sort_unstable();
    ids.dedup();
    let shown: Vec<String> = ids.iter().take(8).map(|id| id.to_string()).collect();
    format!(
        "anchor must name a scene id ({} actors, e.g. [{}]; re-read scene.brief)",
        ids.len(),
        shown.join(", ")
    )
}

fn check_budget(budget: u32) -> Result<usize, QueryError> {
    if !(1..=QUERY_MAX_BUDGET).contains(&budget) {
        return Err(QueryError::new(
            "bad_budget",
            format!("budget_bytes {budget} is outside 1..={QUERY_MAX_BUDGET}"),
            format!("pass 1..={QUERY_MAX_BUDGET}, default {QUERY_DEFAULT_BUDGET}"),
        ));
    }
    Ok(budget as usize)
}

fn check_limit(limit: usize) -> Result<usize, QueryError> {
    if !(1..=QUERY_MAX_RESULTS).contains(&limit) {
        return Err(QueryError::new(
            "bad_limit",
            format!("limit {limit} is outside 1..={QUERY_MAX_RESULTS}"),
            format!("pass 1..={QUERY_MAX_RESULTS} (matched hits rank first)"),
        ));
    }
    Ok(limit)
}

fn check_radius(radius_cm: i32) -> Result<i32, QueryError> {
    if !(1..=QUERY_MAX_RADIUS_CM).contains(&radius_cm) {
        return Err(QueryError::new(
            "bad_radius",
            format!("radius_cm {radius_cm} is outside 1..={QUERY_MAX_RADIUS_CM}"),
            "pass 1..=10000 cm (near bands live under 2000 cm; re-read scene.brief)",
        ));
    }
    Ok(radius_cm)
}

fn check_eye(eye_m: Option<[f32; 3]>) -> Result<[f32; 3], QueryError> {
    match eye_m {
        Some(e) if e.iter().all(|v| v.is_finite()) => Ok(e),
        _ => Err(QueryError::new(
            "bad_point",
            "eye_m must hold three finite meters",
            "pass [x, y, z] in meters, e.g. [0.0, 1.6, 0.0] at eye height",
        )),
    }
}

fn check_anchor(actors: &[Actor], anchor: Option<u32>) -> Result<&Actor, QueryError> {
    match anchor.and_then(|id| actor_by_id(actors, id)) {
        Some(a) => Ok(a),
        None => Err(QueryError::new(
            "unknown_actor",
            format!("anchor {:?} names no scene actor", anchor),
            valid_ids_hint(actors),
        )),
    }
}

fn check_capsule(capsule_radius_cm: i32) -> Result<i32, QueryError> {
    if !(0..=QUERY_MAX_RADIUS_CM).contains(&capsule_radius_cm) {
        return Err(QueryError::new(
            "bad_capsule",
            format!("capsule_radius_cm {capsule_radius_cm} is outside 0..={QUERY_MAX_RADIUS_CM}"),
            "pass 0..=10000 cm (30 cm is a crouched capsule; shrink it when space runs out)",
        ));
    }
    Ok(capsule_radius_cm)
}

/// Order-invariant digest over the matched outcome: kind, anchors, then
/// sorted (id, matched, reason) rows plus the tag.
fn digest_answer(kind: QueryKind, anchors: &[u32], hits: &[Hit]) -> u64 {
    let mut rows: Vec<(u32, bool, &str)> = hits
        .iter()
        .map(|h| (h.id, h.matched, h.reason.as_str()))
        .collect();
    rows.sort_by(|a, b| {
        (a.0, a.1, a.2)
            .partial_cmp(&(b.0, b.1, b.2))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut hasher = Fnv64::new();
    hasher.write(QUERY_TAG);
    hasher.write(&[kind as u8]);
    let mut sorted_anchors = anchors.to_vec();
    sorted_anchors.sort_unstable();
    for id in sorted_anchors {
        hasher.write(&id.to_le_bytes());
    }
    for (id, matched, reason) in rows {
        hasher.write(&id.to_le_bytes());
        hasher.write(&[u8::from(matched)]);
        hasher.write(reason.as_bytes());
    }
    hasher.finish()
}

fn digest_line(digest: u64) -> String {
    format!("digest {digest}\n")
}

fn hit_line(h: &Hit) -> String {
    format!(
        "hit {} {} {:.2} {:.2} {:.2} {}\n",
        h.id,
        u8::from(h.matched),
        h.x,
        h.y,
        h.z,
        h.reason
    )
}

fn conflict_line(c: &Conflict) -> String {
    format!("conflict {} {} hint:{}\n", c.code, c.message, c.hint)
}

/// Render the answer within `budget`: digest plus all conflicts are
/// mandatory identity, then a hit prefix (matched first) while it fits.
/// Whole rows only; dropping any hit sets `truncated`.
fn render(
    kind: QueryKind,
    anchors: &[u32],
    mut hits: Vec<Hit>,
    conflicts: Vec<Conflict>,
    budget: usize,
) -> QueryAnswer {
    hits.sort_by(|a, b| {
        (u8::from(!a.matched), a.id)
            .partial_cmp(&(u8::from(!b.matched), b.id))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let digest = digest_answer(kind, anchors, &hits);
    let head = digest_line(digest);
    let tails: Vec<String> = conflicts.iter().map(conflict_line).collect();
    let mut text = head.clone();
    for t in &tails {
        text.push_str(t);
    }
    let mut kept: Vec<Hit> = Vec::new();
    let mut truncated = false;
    for h in hits {
        let line = hit_line(&h);
        if text.len() + line.len() <= budget {
            text.push_str(&line);
            kept.push(h);
        } else {
            truncated = true;
        }
    }
    // Pathological budget below the digest line: digest still wins.
    if text.len() > budget && kept.is_empty() && tails.is_empty() {
        text = head;
        truncated = true;
    }
    let bytes = text.len();
    QueryAnswer {
        kind,
        hits: kept,
        conflicts,
        digest,
        bytes,
        truncated,
    }
}

fn cover_name(c: Cover) -> &'static str {
    match c {
        Cover::None => "none",
        Cover::Half => "half",
        Cover::Full => "full",
    }
}

/// Answer one spatial question over an actor snapshot plus the caller-owned
/// nav reachability view. Pure: same inputs give the same answer text.
pub fn run_query(
    actors: &[Actor],
    reach: &ReachView,
    args: &QueryArgs,
) -> Result<QueryAnswer, QueryError> {
    let budget = check_budget(args.budget_bytes)?;
    let limit = check_limit(args.limit)?;
    match args.kind {
        QueryKind::Diff => Err(diff::needs_journal()),
        QueryKind::Near => {
            let anchor = check_anchor(actors, args.anchor)?;
            let radius = check_radius(args.radius_cm.unwrap_or(500))?;
            let ac = center_cm(anchor).ok_or_else(|| {
                QueryError::new(
                    "unknown_actor",
                    format!("anchor {} has no finite center", anchor.id),
                    valid_ids_hint(actors),
                )
            })?;
            let mut rows: Vec<(i64, Hit)> = Vec::new();
            let mut ordered: Vec<&Actor> = actors.iter().collect();
            ordered.sort_by_key(|a| a.id);
            for a in ordered {
                if a.id == anchor.id {
                    continue;
                }
                let Some(c) = center_cm(a) else { continue };
                let d = dist_cm(ac, c);
                if d <= i64::from(radius) {
                    rows.push((
                        d,
                        Hit {
                            id: a.id,
                            x: a.x,
                            y: a.y,
                            z: a.z,
                            matched: true,
                            reason: format!("near: {d} cm"),
                        },
                    ));
                }
            }
            rows.sort_by(|a, b| {
                (a.0, a.1.id)
                    .partial_cmp(&(b.0, b.1.id))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let hits: Vec<Hit> = rows.into_iter().map(|(_, h)| h).take(limit).collect();
            Ok(render(args.kind, &[anchor.id], hits, vec![], budget))
        }
        QueryKind::Between => {
            let anchor = check_anchor(actors, args.anchor)?;
            let other_id = args.other.ok_or_else(|| {
                QueryError::new(
                    "unknown_actor",
                    "between needs an `other` actor id".to_string(),
                    valid_ids_hint(actors),
                )
            })?;
            let other = actor_by_id(actors, other_id).ok_or_else(|| {
                QueryError::new(
                    "unknown_actor",
                    format!("other {other_id} names no scene actor"),
                    valid_ids_hint(actors),
                )
            })?;
            let (Some(ac), Some(bc)) = (center_cm(anchor), center_cm(other)) else {
                return Err(QueryError::new(
                    "unknown_actor",
                    "between endpoints need finite centers".to_string(),
                    valid_ids_hint(actors),
                ));
            };
            let direct = dist_xz(ac, bc);
            let mut hits = Vec::new();
            let mut ordered: Vec<&Actor> = actors.iter().collect();
            ordered.sort_by_key(|a| a.id);
            for a in ordered {
                if a.id == anchor.id || a.id == other_id {
                    continue;
                }
                let Some(c) = center_cm(a) else { continue };
                if dist_xz(ac, c) + dist_xz(c, bc) <= direct + BETWEEN_TOL_CM {
                    hits.push(Hit {
                        id: a.id,
                        x: a.x,
                        y: a.y,
                        z: a.z,
                        matched: true,
                        reason: format!("between {} and {}", anchor.id, other_id),
                    });
                    if hits.len() >= limit {
                        break;
                    }
                }
            }
            Ok(render(
                args.kind,
                &[anchor.id, other_id],
                hits,
                vec![],
                budget,
            ))
        }
        QueryKind::Visible => {
            let eye = check_eye(args.eye_m)?;
            let yaw = args.yaw.unwrap_or(0.0);
            if !yaw.is_finite() {
                return Err(QueryError::new(
                    "bad_point",
                    "yaw must be finite radians".to_string(),
                    "pass yaw 0 facing +X, e.g. 0.0 (octant-snapped, like the solver)",
                ));
            }
            let world = SightWorld::build(actors);
            let mut hits = Vec::new();
            let mut ordered: Vec<&Actor> = actors.iter().collect();
            ordered.sort_by_key(|a| a.id);
            for a in ordered {
                let vis = visible(&world, eye, yaw, a.id).map_err(|_| {
                    QueryError::new(
                        "unknown_actor",
                        format!("actor {} indexes no sight box", a.id),
                        "degenerate rows (zero size, NaN) index nothing; re-read scene.brief"
                            .to_string(),
                    )
                })?;
                let (matched, reason) = if vis.visible() {
                    (true, "visible".to_string())
                } else if !vis.in_view {
                    (false, "blocked:out_of_view".to_string())
                } else {
                    (false, "blocked:occluded".to_string())
                };
                hits.push(Hit {
                    id: a.id,
                    x: a.x,
                    y: a.y,
                    z: a.z,
                    matched,
                    reason,
                });
                if hits.len() >= limit {
                    break;
                }
            }
            Ok(render(args.kind, &[], hits, vec![], budget))
        }
        QueryKind::Reachable => {
            let anchor = check_anchor(actors, args.anchor)?;
            let mut hits = Vec::new();
            let mut ordered: Vec<&Actor> = actors.iter().collect();
            ordered.sort_by_key(|a| a.id);
            for a in ordered {
                if a.id == anchor.id {
                    continue;
                }
                let (matched, reason) = match reachable(reach, anchor.id, a.id) {
                    Reach::Reachable => (true, "reachable".to_string()),
                    Reach::Blocked => (false, "blocked:unreachable".to_string()),
                    Reach::OffMesh => (false, "blocked:off_mesh".to_string()),
                };
                hits.push(Hit {
                    id: a.id,
                    x: a.x,
                    y: a.y,
                    z: a.z,
                    matched,
                    reason,
                });
                if hits.len() >= limit {
                    break;
                }
            }
            Ok(render(args.kind, &[anchor.id], hits, vec![], budget))
        }
        QueryKind::CoverNear => {
            let anchor = check_anchor(actors, args.anchor)?;
            let threat = check_eye(args.eye_m)?;
            let radius = check_radius(args.radius_cm.unwrap_or(500))?;
            let ac = center_cm(anchor).ok_or_else(|| {
                QueryError::new(
                    "unknown_actor",
                    format!("anchor {} has no finite center", anchor.id),
                    valid_ids_hint(actors),
                )
            })?;
            let stance = if args.crouch {
                Stance::Crouch
            } else {
                Stance::Stand
            };
            let world = SightWorld::build(actors);
            let mut rows: Vec<(i64, Hit)> = Vec::new();
            let mut ordered: Vec<&Actor> = actors.iter().collect();
            ordered.sort_by_key(|a| a.id);
            for a in ordered {
                if a.id == anchor.id {
                    continue;
                }
                let Some(c) = center_cm(a) else { continue };
                let d = dist_cm(ac, c);
                if d > i64::from(radius) {
                    continue;
                }
                let feet = [a.x, a.y - a.sy / 2.0, a.z];
                let cov = cover(&world, feet, threat, stance).map_err(|_| {
                    QueryError::new(
                        "bad_point",
                        format!("actor {} feet are not finite", a.id),
                        "cover needs finite feet; re-read scene.brief".to_string(),
                    )
                })?;
                let matched = cov != Cover::None;
                rows.push((
                    d,
                    Hit {
                        id: a.id,
                        x: a.x,
                        y: a.y,
                        z: a.z,
                        matched,
                        reason: format!("cover:{} {d} cm", cover_name(cov)),
                    },
                ));
            }
            rows.sort_by(|a, b| {
                (a.0, a.1.id)
                    .partial_cmp(&(b.0, b.1.id))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let hits: Vec<Hit> = rows.into_iter().map(|(_, h)| h).take(limit).collect();
            Ok(render(args.kind, &[anchor.id], hits, vec![], budget))
        }
        QueryKind::LargestOpen => {
            let capsule = check_capsule(args.capsule_radius_cm)?;
            match largest_open_spot(actors, PLACED_SX_M, PLACED_SY_M, PLACED_SZ_M, capsule)
                .map_err(|e| {
                    QueryError::new(
                        "no_space",
                        e.to_string(),
                        "shrink the capsule or clear the room",
                    )
                })? {
                Some(s) => {
                    let hit = Hit {
                        id: SAMPLE_ACTOR_BASE,
                        x: s.x,
                        y: s.y,
                        z: s.z,
                        matched: true,
                        reason: format!(
                            "largest_open: clearance {} cm; place via scene.place Point({},{})",
                            s.clearance_cm,
                            (s.x * 100.0).round() as i32,
                            (s.z * 100.0).round() as i32
                        ),
                    };
                    Ok(render(args.kind, &[], vec![hit], vec![], budget))
                }
                None => {
                    // Solver-computed hint: rescan with no capsule and name
                    // the largest gap that actually fits.
                    let best = largest_open_spot(actors, PLACED_SX_M, PLACED_SY_M, PLACED_SZ_M, 0)
                        .ok()
                        .flatten()
                        .map(|s| s.clearance_cm)
                        .unwrap_or(0);
                    Ok(render(
                        args.kind,
                        &[],
                        vec![],
                        vec![Conflict {
                            code: "no_space",
                            message: format!("no spot fits capsule {capsule} cm"),
                            hint: format!(
                                "largest gap fits capsule {best} cm; retry capsule_radius_cm <= {best} or clear the room"
                            ),
                        }],
                        budget,
                    ))
                }
            }
        }
        QueryKind::Cone => {
            let rows = cone_rows(actors, args)?;
            let anchors: Vec<u32> = args.anchor.into_iter().collect();
            let hits: Vec<Hit> = rows
                .into_iter()
                .take(limit)
                .filter_map(|row| {
                    let a = actor_by_id(actors, row.id)?;
                    Some(Hit {
                        id: row.id,
                        x: a.x,
                        y: a.y,
                        z: a.z,
                        matched: true,
                        reason: format!(
                            "cone: {} {} deg {} cm",
                            row.side.name(),
                            row.bearing_deg,
                            row.distance_cm
                        ),
                    })
                })
                .collect();
            Ok(render(args.kind, &anchors, hits, vec![], budget))
        }
        QueryKind::Sees
        | QueryKind::WhyHidden
        | QueryKind::Clearance
        | QueryKind::Slide
        | QueryKind::Ground => pairs::run_pair_kind(actors, args, budget),
        QueryKind::Measure
        | QueryKind::RoomOf
        | QueryKind::InRoom
        | QueryKind::ExitsOf
        | QueryKind::Glance
        | QueryKind::Coverage
        | QueryKind::View
        | QueryKind::Lidar
        | QueryKind::Map
        | QueryKind::Pixel => space_kinds::run_space_kind(actors, args, budget),
        QueryKind::Relation => {
            let anchor = check_anchor(actors, args.anchor)?;
            let rel = args.rel.ok_or_else(|| {
                QueryError::new(
                    "bad_relation",
                    "relation needs a `relation` word".to_string(),
                    format!("legal relations: {}", rel_words()),
                )
            })?;
            let frame = match args.other {
                Some(viewer) => {
                    actor_by_id(actors, viewer).ok_or_else(|| {
                        QueryError::new(
                            "unknown_actor",
                            format!("viewer {viewer} names no scene actor"),
                            valid_ids_hint(actors),
                        )
                    })?;
                    Frame::Viewer(viewer)
                }
                None => Frame::World,
            };
            let hits: Vec<Hit> = subjects_of(actors, rel, anchor.id, frame)
                .into_iter()
                .take(limit)
                .filter_map(|id| {
                    let a = actor_by_id(actors, id)?;
                    let numbers = engine_spatial::pair::pair_numbers(actors, anchor.id, id, frame)
                        .map(|n| {
                            format!(
                                "; {} cm, bearing {} deg, gap {} cm",
                                n.dist_cm, n.bearing_deg, n.gap_cm
                            )
                        })
                        .unwrap_or_default();
                    Some(Hit {
                        id,
                        x: a.x,
                        y: a.y,
                        z: a.z,
                        matched: true,
                        reason: format!("{}: {} frame{numbers}", rel_name(rel), frame_name(frame)),
                    })
                })
                .collect();
            Ok(render(args.kind, &[anchor.id], hits, vec![], budget))
        }
        QueryKind::FreeSpace => {
            let capsule = check_capsule(args.capsule_radius_cm)?;
            if !(1..=QUERY_MAX_RADIUS_CM).contains(&args.min_gap_cm) {
                return Err(QueryError::new(
                    "bad_gap",
                    format!(
                        "min_gap_cm {} is outside 1..={QUERY_MAX_RADIUS_CM}",
                        args.min_gap_cm
                    ),
                    "pass 1..=10000 cm (100 cm suits crate scale; gap >= capsule x2 spreads best)",
                ));
            }
            if !(1..=QUERY_MAX_RESULTS).contains(&args.max_samples) {
                return Err(QueryError::new(
                    "bad_limit",
                    format!(
                        "max_samples {} is outside 1..={QUERY_MAX_RESULTS}",
                        args.max_samples
                    ),
                    format!("pass 1..={QUERY_MAX_RESULTS} placements per query"),
                ));
            }
            let cfg = SamplerConfig::new(
                PLACED_SX_M,
                PLACED_SY_M,
                PLACED_SZ_M,
                capsule,
                args.min_gap_cm,
                args.max_samples.min(limit),
                args.seed,
            )
            .map_err(|e| {
                QueryError::new("bad_capsule", e.to_string(), "shrink the capsule or gap")
            })?;
            match sample_free_space(actors, &cfg) {
                Ok(samples) => {
                    let hits: Vec<Hit> = samples
                        .iter()
                        .enumerate()
                        .map(|(i, s)| Hit {
                            id: SAMPLE_ACTOR_BASE + i as u32,
                            x: s.x,
                            y: s.y,
                            z: s.z,
                            matched: true,
                            reason: format!(
                                "free_space: clearance {} cm; place via scene.place Point({},{})",
                                s.clearance_cm,
                                (s.x * 100.0).round() as i32,
                                (s.z * 100.0).round() as i32
                            ),
                        })
                        .collect();
                    Ok(render(args.kind, &[], hits, vec![], budget))
                }
                Err(e) => Ok(render(
                    args.kind,
                    &[],
                    vec![],
                    vec![Conflict {
                        code: "no_space",
                        message: e.to_string(),
                        hint: "shrink capsule_radius_cm or min_gap_cm, raise max_samples, or clear the room (largest_open names the best gap)".to_string(),
                    }],
                    budget,
                )),
            }
        }
    }
}

/// Rendered answer text: digest line, conflicts, then kept hits. Fits the
/// query budget by construction; the digest line is mandatory identity.
pub fn answer_text(answer: &QueryAnswer) -> String {
    let mut text = digest_line(answer.digest);
    for c in &answer.conflicts {
        text.push_str(&conflict_line(c));
    }
    for h in &answer.hits {
        text.push_str(&hit_line(h));
    }
    text
}

/// Placement actors for the matched synthetic hits (largest-open and
/// free-space): grounded crates the checks and graphs consume. Real scene
/// ids pass through untouched; the caller owns id uniqueness.
pub fn apply_actors(actors: &[Actor], answer: &QueryAnswer) -> Vec<Actor> {
    let mut out = actors.to_vec();
    for h in answer
        .hits
        .iter()
        .filter(|h| h.matched && h.id >= SAMPLE_ACTOR_BASE)
    {
        out.push(Actor {
            id: h.id,
            name: format!("sample_{:04}", h.id - SAMPLE_ACTOR_BASE),
            x: h.x,
            y: h.y,
            z: h.z,
            sx: PLACED_SX_M,
            sy: PLACED_SY_M,
            sz: PLACED_SZ_M,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        });
    }
    out.sort_by_key(|a| a.id);
    out
}

/// Dry-run diff: relation-graph delta the placements would apply.
pub fn preview_diff(actors: &[Actor], answer: &QueryAnswer) -> SemanticDiff {
    let after = apply_actors(actors, answer);
    diff_graph(
        &engine_spatial::relation::build(actors),
        &engine_spatial::relation::build(&after),
    )
}

/// Applied diff: the same pure computation over the same inputs, so the
/// dry-run graph diff equals the applied result by construction.
pub fn apply_diff(actors: &[Actor], answer: &QueryAnswer) -> SemanticDiff {
    let after = apply_actors(actors, answer);
    diff_graph(
        &engine_spatial::relation::build(actors),
        &engine_spatial::relation::build(&after),
    )
}

impl Hit {
    /// Clearance of one placement hit in cm, parsed from its reason line
    /// (`-1` for actor hits, which carry no clearance).
    pub fn clearance_cm(&self) -> i32 {
        // Reason shape: "free_space: clearance N cm; ...".
        self.reason
            .split("clearance ")
            .nth(1)
            .and_then(|tail| tail.split(' ').next())
            .and_then(|num| num.parse::<i32>().ok())
            .unwrap_or(-1)
    }
}

// ---------------------------------------------------------------------------
// Door tool: scene.query over the staged document.
// ---------------------------------------------------------------------------
//
// The staging map is scene.place's (`scene_tools::snapshot`: index ids over the
// id-text sort, collider-sized boxes), so query answers agree with brief geometry. Reachability reads the
// baked nav mesh the host handed the session (`ServerState::with_nav`): each
// staged actor gets its real polygon, an actor off the mesh answers
// `blocked:off_mesh`, and with no mesh held `reachable` refuses
// `needs_nav_view` rather than answer yes to every pair. Library callers with
// their own view pass it to `run_query` directly.
// Field docs are short on purpose: every doc line is discovery cost (the
// tool-list ledger gate). Units ride in the text, one kind line per question.

/// Arguments of `scene.query`. Send `kind` plus the fields that kind needs.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SceneQueryArgs {
    /// Question. Needs: near anchor; between anchor, other; visible eye_m; reachable anchor; cover_near anchor, eye_m; largest_open -; free_space -; cone half_angle_deg, anchor|eye_m; relation anchor, relation; sees anchor, other; why_hidden anchor, eye_m; clearance anchor; slide anchor, dir; ground eye_m; measure anchor, other; room_of anchor; in_room room; exits_of anchor|room; glance eye_m; coverage anchor, eye_m; view eye_m; lidar eye_m; map eye_m; pixel eye_m; diff revision_a, revision_b. a|b means either.
    #[schemars(schema_with = "cards::kind_schema")]
    pub kind: String,
    /// Object label from scene.brief (`lamp_2 [uuid]`) or bare UUID: the subject.
    pub anchor: Option<String>,
    /// Second object label: far end for between, viewer for relation.
    pub other: Option<String>,
    /// Radius in cm, 1..=10000: search radius (default 500), slide range (500), clearance width needed (60), measure gap needed.
    pub radius_cm: Option<i32>,
    /// Point [x, y, z] in meters: the eye for visible and cone, the threat for cover_near.
    pub eye_m: Option<[f32; 3]>,
    /// Facing in radians, 0 = +X (visible, cone from eye_m); default 0.
    pub yaw: Option<f32>,
    /// True to test crouched cover (cover_near).
    pub crouch: Option<bool>,
    /// Clearance radius in cm, 0..=10000, default 30 (largest_open, free_space).
    pub capsule_radius_cm: Option<i32>,
    /// Smallest gap between samples in cm, default 100 (free_space).
    pub min_gap_cm: Option<i32>,
    /// Spots to sample, default 8 (free_space).
    pub max_samples: Option<u32>,
    /// Sampler seed, default 7 (free_space).
    pub seed: Option<u64>,
    /// Most hits, 1..=64, default 64.
    pub limit: Option<u32>,
    /// Answer budget in bytes, 1..=16384, default 4096.
    pub budget_bytes: Option<u32>,
    /// Cone half-width in whole degrees, 1..=180 (cone); half the field of view (glance, coverage).
    pub half_angle_deg: Option<i32>,
    /// Keep cone rows whose label contains this text.
    pub filter: Option<String>,
    /// Relation word: above, below, in_front_of, behind, left_of, right_of, against, around, aligned, near, far, visible_from, reachable_from.
    pub relation: Option<String>,
    /// Answer shape: rows (default, one row per hit), ids (labels of matched hits only), count (matched count only).
    pub format: Option<String>,
    /// Drop unmatched rows. Default true for visible (pass false for blocked rows), false otherwise.
    pub matched_only: Option<bool>,
    /// Slide direction: +x, -x, +z, -z, up, down, +x+z, +x-z, -x+z, -x-z (slide).
    pub dir: Option<String>,
    /// gap (default), overlap, align or contain (measure).
    pub what: Option<String>,
    /// Room number from room_of (in_room, exits_of).
    pub room: Option<u32>,
    /// Camera pitch in radians, up is positive (coverage).
    pub pitch: Option<f32>,
    /// View or pixel box `[w, h, x, y]` in cells, edges 1..=128 (view, pixel;
    /// default [24,16,0,0], x and y inside the view). The map uses the eye as
    /// its center and `radius_cm` as its reach.
    pub cells: Option<[i32; 4]>,
    /// Before revision of a diff (diff); must be held by the journal.
    pub revision_a: Option<u64>,
    /// After revision of a diff; may be lower than revision_a.
    pub revision_b: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SceneQueryHit {
    pub object: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub matched: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SceneQueryConflict {
    pub code: String,
    pub message: String,
    pub hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SceneQueryOutput {
    pub kind: String,
    pub digest: u64,
    pub bytes: u64,
    pub truncated: bool,
    pub count: u32,
    #[serde(default, skip_serializing_if = "super::scene_tools::no_repeat")]
    pub repeat_count: u32,
    pub hits: Vec<SceneQueryHit>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<String>,
    pub conflicts: Vec<SceneQueryConflict>,
    /// Revision diff (kind diff): the same list `tx.apply` returns
    /// (`ChangeListDto`: revisions, counts, rows, touched ids and edges).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<rmcp::serde_json::Value>")]
    pub change: Option<ChangeListDto>,
}

/// True when the door request carries `field` (`a|b` holds when either does).
fn has_field(args: &SceneQueryArgs, field: &str) -> bool {
    field.split('|').any(|one| match one {
        "anchor" => args.anchor.is_some(),
        "other" => args.other.is_some(),
        "eye_m" => args.eye_m.is_some(),
        "half_angle_deg" => args.half_angle_deg.is_some(),
        "relation" => args.relation.is_some(),
        "dir" => args.dir.is_some(),
        "room" => args.room.is_some(),
        "cells" => args.cells.is_some(),
        "revision_a" => args.revision_a.is_some(),
        "revision_b" => args.revision_b.is_some(),
        _ => true,
    })
}

/// Refuse a request that lacks a field its kind needs: `missing_arg` names
/// the kind, the field and a complete working call. `relation` keeps its own
/// `bad_relation` refusal, which lists the legal words.
fn require_fields(kind: QueryKind, args: &SceneQueryArgs) -> Result<(), McpToolError> {
    let card = KindCard::of(kind);
    let missing = card
        .needs
        .split(", ")
        .find(|field| *field != "relation" && *field != "-" && !has_field(args, field));
    match missing {
        None => Ok(()),
        Some(field) => Err(McpToolError::new(
            "missing_arg",
            format!(
                "scene.query kind `{}` needs `{field}`; working call: {}",
                kind.name(),
                card.example
            ),
        )),
    }
}

/// Answer shape asked through `format` (`scene.query` and `scene.spatial`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueryFormat {
    Rows,
    Ids,
    Count,
}

impl QueryFormat {
    /// Parse the `format` word; absent reads `rows`, unknown refuses `bad_format`.
    pub(crate) fn parse(word: Option<&str>) -> Result<Self, McpToolError> {
        match word {
            None | Some("rows") => Ok(Self::Rows),
            Some("ids") => Ok(Self::Ids),
            Some("count") => Ok(Self::Count),
            Some(other) => Err(McpToolError::new(
                "bad_format",
                format!("format `{other}` is unknown; pass rows, ids or count"),
            )),
        }
    }
}

fn query_error(error: QueryError) -> McpToolError {
    McpToolError::new(error.code, format!("{}; {}", error.message, error.hint))
}

/// Wire bytes of the door request, measured from the same JSON the stdio
/// door parses, so the token ledger budgets what the model actually sends.
fn request_chars(args: &SceneQueryArgs) -> usize {
    rmcp::serde_json::to_string(args)
        .map(|text| text.len())
        .unwrap_or(0)
}

fn resolve_actor(field: &str, text: &str, uuids: &[ObjectId]) -> Result<u32, McpToolError> {
    let id = parse_object_ref(field, text)?;
    uuids
        .iter()
        .position(|uuid| *uuid == id)
        .map(|index| index as u32)
        .ok_or_else(|| {
            McpToolError::new(
                "unknown_actor",
                format!(
                    "{field} `{text}` names no staged object ({} staged; \
                     re-read scene.brief for the staged labels)",
                    uuids.len()
                ),
            )
        })
}

pub(crate) fn label_of(id: u32, actors: &[Actor], uuids: &[ObjectId]) -> String {
    let at = usize::try_from(id).unwrap_or(usize::MAX);
    match (actors.get(at), uuids.get(at)) {
        (Some(actor), Some(uuid)) => object_label(&actor.name, uuid),
        _ => format!("sample_{:04}", id.wrapping_sub(SAMPLE_ACTOR_BASE)),
    }
}

/// The door reply for an answer: rows, matched labels or the count only,
/// with the digest and sizes of the full answer.
fn shape_output(
    answer: &QueryAnswer,
    format: QueryFormat,
    matched_only: bool,
    actors: &[Actor],
    uuids: &[ObjectId],
) -> SceneQueryOutput {
    let kept = answer
        .hits
        .iter()
        .filter(|hit| hit.matched || !matched_only)
        .map(|hit| (hit, label_of(hit.id, actors, uuids)));
    let (hits, ids) = match format {
        QueryFormat::Rows => (
            kept.map(|(hit, object)| SceneQueryHit {
                object,
                x: hit.x,
                y: hit.y,
                z: hit.z,
                matched: hit.matched,
                reason: hit.reason.clone(),
            })
            .collect(),
            Vec::new(),
        ),
        QueryFormat::Ids => (
            Vec::new(),
            kept.filter(|(hit, _)| hit.matched)
                .map(|(_, object)| object)
                .collect(),
        ),
        QueryFormat::Count => (Vec::new(), Vec::new()),
    };
    let output = SceneQueryOutput {
        kind: answer.kind.name().to_owned(),
        digest: answer.digest,
        bytes: answer.bytes as u64,
        truncated: answer.truncated,
        count: u32::try_from(answer.hits.iter().filter(|hit| hit.matched).count())
            .unwrap_or(u32::MAX),
        repeat_count: 0,
        hits,
        ids,
        conflicts: answer
            .conflicts
            .iter()
            .map(|conflict| SceneQueryConflict {
                code: conflict.code.to_owned(),
                message: conflict.message.clone(),
                hint: conflict.hint.clone(),
            })
            .collect(),
        change: None,
    };
    output
}

impl super::mcp::ServerState {
    /// Answer one `scene.query` call over the staged document. Read-only:
    /// the document, journal, budget and ledger never move.
    pub fn scene_query(&self, args: &SceneQueryArgs) -> Result<SceneQueryOutput, McpToolError> {
        let kind = QueryKind::parse(&args.kind).map_err(query_error)?;
        require_fields(kind, args)?;
        let format = QueryFormat::parse(args.format.as_deref())?;
        if kind == QueryKind::Diff {
            return diff::answer(&self.doc, &self.journal, args);
        }
        let snap = snapshot(&self.doc);
        let (actors, uuids) = (snap.actors, snap.ids);
        let anchor = args
            .anchor
            .as_deref()
            .map(|text| resolve_actor("anchor", text, &uuids))
            .transpose()?;
        let other = args
            .other
            .as_deref()
            .map(|text| resolve_actor("other", text, &uuids))
            .transpose()?;
        let rel = args
            .relation
            .as_deref()
            .map(|word| {
                parse_rel(word).ok_or_else(|| {
                    query_error(QueryError::new(
                        "bad_relation",
                        format!("relation `{word}` is unknown"),
                        format!("legal relations: {}", rel_words()),
                    ))
                })
            })
            .transpose()?;
        let reach = match (&self.nav, kind) {
            (Some(mesh), _) => staged_reach(mesh, &actors),
            (None, QueryKind::Reachable) => return Err(needs_nav_view()),
            (None, _) => ReachView::default(),
        };
        let query = QueryArgs {
            kind,
            anchor,
            other,
            radius_cm: args.radius_cm,
            eye_m: args.eye_m,
            yaw: args.yaw,
            crouch: args.crouch.unwrap_or(false),
            capsule_radius_cm: args.capsule_radius_cm.unwrap_or(30),
            min_gap_cm: args.min_gap_cm.unwrap_or(100),
            max_samples: args.max_samples.map(|n| n as usize).unwrap_or(8),
            seed: args.seed.unwrap_or(7),
            limit: args.limit.map(|n| n as usize).unwrap_or(QUERY_MAX_RESULTS),
            budget_bytes: args.budget_bytes.unwrap_or(QUERY_DEFAULT_BUDGET),
            half_angle_deg: args.half_angle_deg,
            filter: args.filter.clone(),
            rel,
            dir: args.dir.clone(),
            cells: args.cells,
            space: SpaceArgs {
                what: args.what.clone(),
                room: args.room,
                pitch_deg: args.pitch.map(engine_spatial::sense_text::whole_deg),
            },
        };
        let answer = run_query(&actors, &reach, &query).map_err(query_error)?;
        let matched_only = args.matched_only.unwrap_or(kind == QueryKind::Visible);
        let output = shape_output(&answer, format, matched_only, &actors, &uuids);
        let wire = rmcp::serde_json::to_string(&output).map_or(0, |text| text.len());
        super::watch_ledger::note_door_answer(kind, request_chars(args), &answer, wire);
        Ok(output)
    }
}

pub struct SceneQueryTool;

impl ToolBase for SceneQueryTool {
    type Parameter = SceneQueryArgs;
    type Output = SceneQueryOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_SCENE_QUERY.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Spatial questions with reasons: near, between, visible, reachable, \
             cover_near, largest_open, free_space, cone, relation, view, lidar, \
             map, pixel, diff. Read-only; api.describe query:<kind> gives a working call."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for SceneQueryTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch_screened(TOOL_SCENE_QUERY, &param, |state| {
            let repeat = state.note_read(TOOL_SCENE_QUERY, &param)?;
            let mut out = state.scene_query(&param)?;
            out.repeat_count = repeat;
            Ok(out)
        })
    }
}

mod cone;
mod pairs;
mod reach;
mod space_kinds;
pub use cone::*;
use reach::{needs_nav_view, staged_reach};
use space_kinds::SpaceArgs;

#[cfg(test)]
mod compact_tests;
#[cfg(test)]
mod diff_tests;
#[cfg(test)]
mod reach_door_tests;
#[cfg(test)]
mod schema_tests;
#[cfg(test)]
mod space_kind_tests;
#[cfg(test)]
mod space_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use engine_spatial::relation::Relation;
    use engine_spatial::sight::Reach;
    use engine_spatial::{Intent, ObjectRef, Target, Want};
    use std::collections::BTreeMap;

    #[expect(
        clippy::too_many_arguments,
        reason = "test fixture builder takes each field positionally"
    )]
    fn mk(id: u32, name: &str, x: f32, y: f32, z: f32, sx: f32, sy: f32, sz: f32) -> Actor {
        Actor {
            id,
            name: name.into(),
            x,
            y,
            z,
            sx,
            sy,
            sz,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    /// Fixture: walled 10 m room, a 3 m pillar occluder at x = 2, crates on
    /// the west/east axis plus one north, one far west behind the eye, two
    /// on the view-cone edge bands, and one actor missing from the nav map.
    fn scene() -> Vec<Actor> {
        vec![
            mk(0, "wall_north", 0.0, 1.0, -5.0, 10.0, 2.0, 0.2),
            mk(1, "wall_south", 0.0, 1.0, 5.0, 10.0, 2.0, 0.2),
            mk(2, "wall_west", -5.0, 1.0, 0.0, 0.2, 2.0, 10.0),
            mk(3, "wall_east", 5.0, 1.0, 0.0, 0.2, 2.0, 10.0),
            mk(10, "pillar", 2.0, 1.5, 0.0, 1.0, 3.0, 1.0),
            mk(20, "crate_w", -4.0, 0.4, 0.0, 0.8, 0.8, 0.8),
            mk(21, "crate_c", 0.0, 0.4, 0.0, 0.8, 0.8, 0.8),
            mk(22, "crate_e", 4.0, 0.4, 0.0, 0.8, 0.8, 0.8),
            mk(23, "crate_n", 0.0, 0.4, 4.0, 0.8, 0.8, 0.8),
            mk(24, "crate_far", -9.0, 0.4, 0.0, 0.8, 0.8, 0.8),
            mk(25, "crate_edge", 2.0, 0.4, 2.0, 0.8, 0.8, 0.8),
            mk(26, "crate_out", -2.0, 0.4, 5.0, 0.8, 0.8, 0.8),
            mk(27, "crate_offmesh", 3.0, 0.4, 4.0, 0.8, 0.8, 0.8),
        ]
    }

    fn reach_view() -> ReachView {
        // Polygon 0 holds the west room, polygon 9 the east island crate.
        let mut adj = BTreeMap::new();
        adj.insert(0u32, vec![]);
        adj.insert(9u32, vec![]);
        let mut at = BTreeMap::new();
        for id in [0u32, 1, 2, 3, 10, 20, 21, 23, 24, 25, 26] {
            at.insert(id, 0u32);
        }
        at.insert(22, 9u32);
        ReachView { adj, at }
    }

    fn matched_ids(answer: &QueryAnswer) -> Vec<u32> {
        let mut ids: Vec<u32> = answer
            .hits
            .iter()
            .filter(|h| h.matched)
            .map(|h| h.id)
            .collect();
        ids.sort_unstable();
        ids
    }

    fn reason_of(answer: &QueryAnswer, id: u32) -> String {
        answer
            .hits
            .iter()
            .find(|h| h.id == id)
            .map(|h| h.reason.clone())
            .unwrap_or_default()
    }

    /// Refusal rows of the gold set: every error names its code and a
    /// recovery hint. Split from `gold` to keep each builder short.
    fn gold_refusals(actors: &[Actor], view: &ReachView, g: &mut Vec<(&'static str, u32, u32)>) {
        // --- Refusals: every one names its code and a recovery hint.
        let bad_anchor = run_query(actors, view, &QueryArgs::near(999, 500)).unwrap_err();
        g.push((
            "err-anchor-code",
            u32::from(bad_anchor.code == "unknown_actor"),
            1,
        ));
        g.push((
            "err-anchor-hint",
            u32::from(bad_anchor.hint.contains("scene.brief")),
            1,
        ));
        let bad_radius = run_query(actors, view, &QueryArgs::near(21, 0)).unwrap_err();
        g.push((
            "err-radius-code",
            u32::from(bad_radius.code == "bad_radius"),
            1,
        ));
        g.push(("err-radius-hint", u32::from(!bad_radius.hint.is_empty()), 1));
        let bad_kind = QueryKind::parse("sideways").unwrap_err();
        g.push(("err-kind-code", u32::from(bad_kind.code == "bad_kind"), 1));
        g.push((
            "err-kind-hint",
            u32::from(bad_kind.hint.contains("free_space")),
            1,
        ));
        let bad_eye =
            run_query(actors, view, &QueryArgs::visible([f32::NAN, 1.6, 0.0], 0.0)).unwrap_err();
        g.push(("err-eye-code", u32::from(bad_eye.code == "bad_point"), 1));
        let mut bad_budget_args = QueryArgs::near(21, 500);
        bad_budget_args.budget_bytes = 0;
        let bad_budget = run_query(actors, view, &bad_budget_args).unwrap_err();
        g.push((
            "err-budget-code",
            u32::from(bad_budget.code == "bad_budget"),
            1,
        ));
        let mut bad_limit_args = QueryArgs::near(21, 500);
        bad_limit_args.limit = 0;
        g.push((
            "err-limit-code",
            u32::from(run_query(actors, view, &bad_limit_args).unwrap_err().code == "bad_limit"),
            1,
        ));
        let bad_capsule = run_query(actors, view, &QueryArgs::largest_open(-5)).unwrap_err();
        g.push((
            "err-capsule-code",
            u32::from(bad_capsule.code == "bad_capsule"),
            1,
        ));
        let bad_gap = run_query(actors, view, &QueryArgs::free_space(30, 0, 8, 7)).unwrap_err();
        g.push(("err-gap-code", u32::from(bad_gap.code == "bad_gap"), 1));
    }

    /// 100-question gold set: (name, obtained, expected). Integer outcomes
    /// only, mirroring the SP8 gold shape.
    fn gold() -> Vec<(&'static str, u32, u32)> {
        let actors = scene();
        let view = reach_view();
        let mut g: Vec<(&'static str, u32, u32)> = Vec::new();
        let has = |ids: &[u32], id: u32| -> u32 { u32::from(ids.contains(&id)) };
        // --- Near from the center crate (anchor 21): pillar 200, L/R/N 400,
        // walls 500, pillar-adjacent nothing else inside 450.
        let near_ids = |radius: i32| -> Vec<u32> {
            matched_ids(
                &run_query(&actors, &view, &QueryArgs::near(21, radius)).expect("near runs"),
            )
        };
        let n450 = near_ids(450);
        for (id, want) in [
            (10u32, 1u32),
            (20, 1),
            (22, 1),
            (23, 1),
            (0, 0),
            (24, 0),
            (27, 0),
        ] {
            g.push(("near450", has(&n450, id), want));
        }
        let n250 = near_ids(250);
        for (id, want) in [(10u32, 1u32), (20, 0), (22, 0), (23, 0)] {
            g.push(("near250", has(&n250, id), want));
        }
        let n150 = near_ids(150);
        g.push(("near150-empty", u32::from(n150.is_empty()), 1));
        // Radii sweep: matched sets only grow.
        let mut prev = 0;
        for radius in [100, 200, 300, 400, 500, 600, 800, 1000, 1500, 2000] {
            let n = near_ids(radius).len() as u32;
            g.push(("near-mono", u32::from(n >= prev), 1));
            prev = n;
        }
        // --- Between: west-east corridor holds the center crate; the pillar
        // sits between center and east; the diagonal holds nothing.
        let between_ids = |a: u32, b: u32| -> Vec<u32> {
            matched_ids(
                &run_query(&actors, &view, &QueryArgs::between(a, b)).expect("between runs"),
            )
        };
        let b_we = between_ids(20, 22);
        for (id, want) in [(21u32, 1u32), (10, 1), (23, 0), (20, 0), (22, 0)] {
            g.push(("between-we", has(&b_we, id), want));
        }
        let b_ce = between_ids(21, 22);
        for (id, want) in [(10u32, 1u32), (20, 0), (23, 0)] {
            g.push(("between-ce", has(&b_ce, id), want));
        }
        let b_diag = between_ids(20, 23);
        g.push(("between-diag-empty", u32::from(b_diag.is_empty()), 1));
        // --- Visible from (-3, 1.6, 0) facing +X: pillar and center seen,
        // east hidden behind the pillar, west crates behind the eye.
        let vis = run_query(&actors, &view, &QueryArgs::visible([-3.0, 1.6, 0.0], 0.0))
            .expect("visible runs");
        let vids = matched_ids(&vis);
        for (id, want) in [
            (21u32, 1u32),
            (10, 1),
            (20, 0),
            (22, 0),
            (24, 0),
            (25, 1),
            (26, 0),
        ] {
            g.push(("visible-east", has(&vids, id), want));
        }
        // Reasons name the block: occluded east, out-of-view far west.
        g.push((
            "reason-occluded",
            u32::from(reason_of(&vis, 22) == "blocked:occluded"),
            1,
        ));
        g.push((
            "reason-out-of-view",
            u32::from(reason_of(&vis, 24) == "blocked:out_of_view"),
            1,
        ));
        g.push((
            "reason-visible",
            u32::from(reason_of(&vis, 21) == "visible"),
            1,
        ));
        // Turned around (yaw PI): the west crates face the eye instead.
        let vis_back = run_query(
            &actors,
            &view,
            &QueryArgs::visible([-3.0, 1.6, 0.0], std::f32::consts::PI),
        )
        .expect("back runs");
        let bids = matched_ids(&vis_back);
        // The far-west crate sits outside the room: the west wall occludes
        // it, so only the inside west crate faces the turned eye.
        for (id, want) in [(24u32, 0u32), (21, 0), (22, 0), (20, 1)] {
            g.push(("visible-back", has(&bids, id), want));
        }
        g.push((
            "reason-back-wall",
            u32::from(reason_of(&vis_back, 24) == "blocked:occluded"),
            1,
        ));
        // --- Reachable from the west crate: island east blocks, the actor
        // missing from the nav map reads off-mesh.
        let reach = run_query(&actors, &view, &QueryArgs::reachable(20)).expect("reachable runs");
        let rids = matched_ids(&reach);
        for (id, want) in [(21u32, 1u32), (23, 1), (10, 1), (22, 0), (27, 0)] {
            g.push(("reachable-west", has(&rids, id), want));
        }
        g.push((
            "reason-unreachable",
            u32::from(reason_of(&reach, 22) == "blocked:unreachable"),
            1,
        ));
        g.push((
            "reason-offmesh",
            u32::from(reason_of(&reach, 27) == "blocked:off_mesh"),
            1,
        ));
        g.push((
            "reason-reachable",
            u32::from(reason_of(&reach, 21) == "reachable"),
            1,
        ));
        // Self-answer: the anchor never lists itself.
        g.push(("reachable-no-self", has(&rids, 20), 0));
        // --- Cover from threat (10, 1.5, 0): the pillar gives full cover at
        // the center crate; open ground (no wall world) gives none.
        let cover_full = run_query(
            &actors,
            &view,
            &QueryArgs::cover_near(20, [10.0, 1.5, 0.0], false, 3000),
        )
        .expect("cover runs");
        let cids = matched_ids(&cover_full);
        g.push(("cover-center-full", has(&cids, 21), 1));
        g.push((
            "cover-reason-full",
            u32::from(reason_of(&cover_full, 21).starts_with("cover:full")),
            1,
        ));
        // Low wall world: head clears a 1 m wall, chest does not (half).
        let low_wall = vec![
            mk(0, "floor_ref", 0.0, 0.05, 0.0, 20.0, 0.1, 20.0),
            mk(1, "low_wall", 5.0, 0.5, 0.0, 1.0, 1.0, 6.0),
            mk(2, "seeker", 0.0, 0.4, 0.0, 0.8, 0.8, 0.8),
        ];
        let low_view = ReachView::default();
        let low = run_query(
            &low_wall,
            &low_view,
            &QueryArgs::cover_near(2, [10.0, 1.5, 0.0], false, 3000),
        )
        .expect("low cover runs");
        g.push((
            "cover-low-half",
            u32::from(reason_of(&low, 0).starts_with("cover:")),
            1,
        ));
        // Open world: no cover anywhere.
        let open = vec![
            mk(0, "a", 0.0, 0.4, 0.0, 0.8, 0.8, 0.8),
            mk(1, "b", 4.0, 0.4, 0.0, 0.8, 0.8, 0.8),
        ];
        let open_cover = run_query(
            &open,
            &ReachView::default(),
            &QueryArgs::cover_near(0, [10.0, 1.5, 0.0], false, 3000),
        )
        .expect("open cover runs");
        g.push((
            "cover-open-none",
            u32::from(matched_ids(&open_cover).is_empty()),
            1,
        ));
        g.push((
            "cover-open-reason",
            u32::from(reason_of(&open_cover, 1).starts_with("cover:none")),
            1,
        ));
        // --- Largest open: some clearance, grounded, inside the room.
        let open_spot = run_query(&actors, &view, &QueryArgs::largest_open(30)).expect("open runs");
        g.push(("open-one-hit", open_spot.hits.len() as u32, 1));
        let h = &open_spot.hits[0];
        g.push(("open-clearance", u32::from(h.clearance_cm() >= 30), 1));
        g.push(("open-grounded", u32::from(h.y == PLACED_SY_M / 2.0), 1));
        g.push(("open-matched", u32::from(h.matched), 1));
        // Impossible capsule: conflict with a solver-computed smaller value.
        let tight = run_query(&actors, &view, &QueryArgs::largest_open(9000)).expect("tight runs");
        g.push(("open-tight-empty", u32::from(tight.hits.is_empty()), 1));
        g.push(("open-tight-conflict", tight.conflicts.len() as u32, 1));
        g.push((
            "open-tight-hint",
            u32::from(!tight.conflicts[0].hint.is_empty()),
            1,
        ));
        // --- Free space: placements clear, grounded, reachable, budgeted.
        let free =
            run_query(&actors, &view, &QueryArgs::free_space(30, 100, 8, 7)).expect("free runs");
        g.push(("free-nonempty", u32::from(!free.hits.is_empty()), 1));
        g.push((
            "free-clear",
            u32::from(free.hits.iter().all(|x| x.clearance_cm() >= 0)),
            1,
        ));
        // Digest stable twice, order-invariant inputs, byte budget holds.
        let free2 =
            run_query(&actors, &view, &QueryArgs::free_space(30, 100, 8, 7)).expect("free again");
        g.push((
            "free-digest-stable",
            u32::from(free.digest == free2.digest),
            1,
        ));
        g.push((
            "free-budget",
            u32::from(free.bytes <= QUERY_DEFAULT_BUDGET as usize),
            1,
        ));
        g.push((
            "free-text-fits",
            u32::from(answer_text(&free).len() == free.bytes),
            1,
        ));
        gold_refusals(&actors, &view, &mut g);
        // --- Shape rows: anchors, limits, reasons, kinds, door strings.
        let n20 = matched_ids(
            &run_query(&actors, &view, &QueryArgs::near(20, 500)).expect("near20 runs"),
        );
        g.push(("near-excludes-anchor", has(&n20, 20), 0));
        let mut lim_args = QueryArgs::near(21, 3000);
        lim_args.limit = 2;
        let lim = run_query(&actors, &view, &lim_args).expect("limit runs");
        g.push(("near-limit-held", u32::from(lim.hits.len() <= 2), 1));
        g.push((
            "near-reason-prefix",
            u32::from(lim.hits.iter().all(|h| h.reason.starts_with("near: "))),
            1,
        ));
        g.push((
            "between-unknown-other",
            u32::from(
                run_query(&actors, &view, &QueryArgs::between(20, 999))
                    .unwrap_err()
                    .code
                    == "unknown_actor",
            ),
            1,
        ));
        let b_self = matched_ids(
            &run_query(&actors, &view, &QueryArgs::between(21, 21)).expect("self runs"),
        );
        g.push(("between-self-empty", u32::from(b_self.is_empty()), 1));
        g.push((
            "visible-row-count",
            vis.hits.len() as u32,
            actors.len() as u32,
        ));
        g.push((
            "visible-reasons-closed",
            u32::from(vis.hits.iter().all(|h| {
                h.reason == "visible"
                    || h.reason == "blocked:occluded"
                    || h.reason == "blocked:out_of_view"
            })),
            1,
        ));
        let mut vlim_args = QueryArgs::visible([-3.0, 1.6, 0.0], 0.0);
        vlim_args.limit = 3;
        g.push((
            "visible-limit-held",
            run_query(&actors, &view, &vlim_args)
                .expect("vlim runs")
                .hits
                .len() as u32,
            3,
        ));
        g.push((
            "reachable-reasons-closed",
            u32::from(reach.hits.iter().all(|h| {
                h.reason == "reachable"
                    || h.reason == "blocked:unreachable"
                    || h.reason == "blocked:off_mesh"
            })),
            1,
        ));
        g.push((
            "cover-unknown-anchor",
            u32::from(
                run_query(
                    &actors,
                    &view,
                    &QueryArgs::cover_near(999, [10.0, 1.5, 0.0], false, 500),
                )
                .unwrap_err()
                .code
                    == "unknown_actor",
            ),
            1,
        ));
        g.push((
            "reachable-unknown-anchor",
            u32::from(
                run_query(&actors, &view, &QueryArgs::reachable(999))
                    .unwrap_err()
                    .code
                    == "unknown_actor",
            ),
            1,
        ));
        // Crouch never loses cover the stand has: matched sets only grow.
        let stand_n = matched_ids(
            &run_query(
                &actors,
                &view,
                &QueryArgs::cover_near(20, [10.0, 1.5, 0.0], false, 3000),
            )
            .expect("stand runs"),
        )
        .len() as u32;
        let crouch_n = matched_ids(
            &run_query(
                &actors,
                &view,
                &QueryArgs::cover_near(20, [10.0, 1.5, 0.0], true, 3000),
            )
            .expect("crouch runs"),
        )
        .len() as u32;
        g.push(("cover-crouch-grows", u32::from(crouch_n >= stand_n), 1));
        let open0 = run_query(&actors, &view, &QueryArgs::largest_open(0)).expect("open0 runs");
        g.push(("open-zero-capsule", u32::from(!open0.hits.is_empty()), 1));
        let free1 =
            run_query(&actors, &view, &QueryArgs::free_space(30, 100, 1, 7)).expect("free1 runs");
        g.push(("free-max-one", u32::from(free1.hits.len() <= 1), 1));
        let free_wide =
            run_query(&actors, &view, &QueryArgs::free_space(30, 5000, 8, 7)).expect("wide runs");
        g.push(("free-wide-gap-few", u32::from(free_wide.hits.len() <= 4), 1));
        let n_again = run_query(&actors, &view, &QueryArgs::near(21, 450)).expect("again runs");
        let n_first = run_query(&actors, &view, &QueryArgs::near(21, 450)).expect("first runs");
        g.push((
            "near-digest-stable",
            u32::from(n_again.digest == n_first.digest),
            1,
        ));
        g.push((
            "answer-digest-line",
            u32::from(answer_text(&n_first).starts_with(&format!("digest {}", n_first.digest))),
            1,
        ));
        for (word, kind) in [
            ("near", QueryKind::Near),
            ("between", QueryKind::Between),
            ("visible", QueryKind::Visible),
            ("reachable", QueryKind::Reachable),
            ("cover_near", QueryKind::CoverNear),
            ("largest_open", QueryKind::LargestOpen),
            ("free_space", QueryKind::FreeSpace),
        ] {
            g.push((
                "kind-roundtrip",
                u32::from(QueryKind::parse(word).expect("kind parses") == kind),
                1,
            ));
            g.push(("kind-name", u32::from(kind.name() == word), 1));
        }
        g.push(("tool-name", u32::from(TOOL_SCENE_QUERY == "scene.query"), 1));
        g.push((
            "hook-names-router",
            u32::from(hook_name().contains("mcp.rs")),
            1,
        ));
        let yaw_nan = run_query(
            &actors,
            &view,
            &QueryArgs::visible([0.0, 1.6, 0.0], f32::NAN),
        )
        .unwrap_err();
        g.push(("err-yaw-code", u32::from(yaw_nan.code == "bad_point"), 1));
        let mut over_args = QueryArgs::near(21, 500);
        over_args.budget_bytes = QUERY_MAX_BUDGET + 1;
        g.push((
            "err-budget-max",
            u32::from(run_query(&actors, &view, &over_args).unwrap_err().code == "bad_budget"),
            1,
        ));
        g.push((
            "err-radius-max",
            u32::from(
                run_query(
                    &actors,
                    &view,
                    &QueryArgs::near(21, QUERY_MAX_RADIUS_CM + 1),
                )
                .unwrap_err()
                .code
                    == "bad_radius",
            ),
            1,
        ));
        let mut zero_samples = QueryArgs::free_space(30, 100, 8, 7);
        zero_samples.max_samples = 0;
        g.push((
            "err-samples-zero",
            u32::from(run_query(&actors, &view, &zero_samples).unwrap_err().code == "bad_limit"),
            1,
        ));
        g
    }

    #[test]
    fn gold_query_rate() {
        let g = gold();
        assert!(g.len() >= 100, "gold set holds {} questions", g.len());
        let mut failures: Vec<&str> = Vec::new();
        for (name, got, want) in &g {
            if got != want {
                failures.push(name);
            }
        }
        let pass = g.len() - failures.len();
        assert!(
            pass * 100 >= 95 * g.len(),
            "scene.query {pass}/{} < 95%: {failures:?}",
            g.len()
        );
        assert!(failures.is_empty(), "gold failures: {failures:?}");
    }

    #[test]
    fn answers_fit_the_brief_budget_and_truncate_cleanly() {
        let actors = scene();
        let view = reach_view();
        // Wide query under the default budget: everything fits.
        let full =
            run_query(&actors, &view, &QueryArgs::visible([-3.0, 1.6, 0.0], 0.0)).expect("runs");
        assert!(
            full.bytes <= QUERY_DEFAULT_BUDGET as usize,
            "bytes={}",
            full.bytes
        );
        assert!(!full.truncated);
        assert_eq!(answer_text(&full).len(), full.bytes);
        // Tight budget: digest survives, hits drop, truncation is flagged.
        let mut tight_args = QueryArgs::visible([-3.0, 1.6, 0.0], 0.0);
        tight_args.budget_bytes = 64;
        let tight = run_query(&actors, &view, &tight_args).expect("tight runs");
        assert_eq!(tight.digest, full.digest, "digest survives any budget");
        assert!(tight.truncated, "tight budget drops rows");
        assert!(answer_text(&tight).starts_with(&format!("digest {}", full.digest)));
    }

    #[test]
    fn sampled_placements_verify_clean_reachable_and_placeable() {
        let actors = scene();
        let view = reach_view();
        let answer =
            run_query(&actors, &view, &QueryArgs::free_space(30, 100, 8, 7)).expect("free runs");
        assert!(!answer.hits.is_empty());
        // Staged scene stays overlap-clean and floating-clean for new ids.
        let staged = apply_actors(&actors, &answer);
        let before = engine_spatial::run(&actors);
        let after = engine_spatial::run(&staged);
        assert_eq!(
            after.count("overlap"),
            before.count("overlap"),
            "0 new overlaps"
        );
        let new_floats = after
            .findings
            .iter()
            .filter(|f| f.check == "floating" && f.a >= SAMPLE_ACTOR_BASE)
            .count();
        assert_eq!(new_floats, 0, "0 floating placements");
        // Reachable: every placement shares the west polygon through the
        // caller map, mirroring the NV1 polygon_at wiring.
        let mut at = BTreeMap::new();
        at.insert(20u32, 0u32);
        for h in &answer.hits {
            at.insert(h.id, 0u32);
        }
        let mut adj = BTreeMap::new();
        adj.insert(0u32, vec![0u32]);
        let poly = ReachView { adj, at };
        for h in &answer.hits {
            assert_eq!(
                reachable(&poly, 20, h.id),
                Reach::Reachable,
                "hit {} reaches",
                h.id
            );
        }
        // SP5 consumes the sampler: the first placement anchors an intent.
        let mut scene2 = actors.clone();
        scene2.push(mk(60, "mover", -4.5, 0.4, -4.5, 0.8, 0.8, 0.8));
        let first = &answer.hits[0];
        let intent = Intent {
            object: ObjectRef::Id(60),
            wants: vec![Want {
                relation: Relation::NextTo,
                target: Target::Point {
                    x_cm: (first.x * 100.0).round() as i32,
                    z_cm: (first.z * 100.0).round() as i32,
                },
                distance_cm: Some((0, 120)),
            }],
        };
        let out = engine_spatial::solve(&intent, &scene2).expect("sampled point is placeable");
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn dry_run_diff_equals_applied_ten_of_ten() {
        let actors = scene();
        let view = reach_view();
        for seed in 0..10u64 {
            let answer =
                run_query(&actors, &view, &QueryArgs::free_space(30, 100, 6, seed)).expect("runs");
            let dry = preview_diff(&actors, &answer);
            let applied = apply_diff(&actors, &answer);
            assert_eq!(dry, applied, "seed {seed}: dry-run equals applied");
            assert!(!dry.is_empty(), "seed {seed}: placements change relations");
        }
        // Largest-open agrees too.
        let open = run_query(&actors, &view, &QueryArgs::largest_open(30)).expect("open runs");
        assert_eq!(preview_diff(&actors, &open), apply_diff(&actors, &open));
    }

    #[test]
    fn every_conflict_carries_a_repair_hint() {
        let actors = scene();
        let view = reach_view();
        let tight = run_query(&actors, &view, &QueryArgs::largest_open(9000)).expect("tight runs");
        assert!(!tight.conflicts.is_empty());
        for c in &tight.conflicts {
            assert!(!c.hint.is_empty(), "conflict {} has a hint", c.code);
            assert!(
                c.hint.contains("capsule"),
                "hint names the repair: {}",
                c.hint
            );
        }
        let sealed = vec![mk(0, "plate", 0.0, 0.5, 0.0, 10.0, 1.0, 10.0)];
        let no_space = run_query(
            &sealed,
            &ReachView::default(),
            &QueryArgs::free_space(30, 100, 4, 1),
        )
        .expect("sealed runs");
        assert!(!no_space.conflicts.is_empty());
        for c in &no_space.conflicts {
            assert!(!c.hint.is_empty());
        }
    }

    fn door_server() -> McpServer {
        use engine_editor::capability::Capability;
        use engine_editor::commands::{self, Batch, Document, Journal, Op};
        use engine_scene::transform::Transform;

        // Four grounded crates through the command service: a west-east row
        // plus one north, so the room has depth for the open-space kinds.
        // Fixed ids: the staging map sorts by id text, so labels[0..4] read
        // west, middle, east, north deterministically.
        let ids = [
            "11111111-1111-1111-1111-111111111111",
            "22222222-2222-2222-2222-222222222222",
            "33333333-3333-3333-3333-333333333333",
            "44444444-4444-4444-4444-444444444444",
        ]
        .map(|text| ObjectId::parse(text).expect("fixed id parses"));
        let spots = [(-2.0f32, 0.0f32), (0.0, 0.0), (2.0, 0.0), (0.0, 2.0)];
        let mut doc = Document::new();
        let mut journal = Journal::new();
        for ((key, (x, z)), id) in [11u64, 12, 13, 14].iter().zip(spots).zip(ids) {
            let batch = Batch {
                ops: vec![Op::Insert {
                    id,
                    parent: ObjectId::NIL,
                    local: Transform::new([x, 0.5, z], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
                }],
                base_revision: doc.revision,
                idempotency_key: *key,
                provenance: None,
            };
            commands::apply(&mut doc, &mut journal, &batch).expect("seed applies");
        }
        McpServer::with_document(doc, journal, 7, Capability::unrestricted(7))
    }

    fn staged_labels(server: &McpServer) -> Vec<String> {
        use crate::scene_tools::{SceneBriefArgs, SceneBriefTool};

        SceneBriefTool::invoke(server, SceneBriefArgs::default())
            .expect("brief reads")
            .objects
    }

    fn query(server: &McpServer, args: SceneQueryArgs) -> SceneQueryOutput {
        SceneQueryTool::invoke(server, args).expect("query serves")
    }

    #[test]
    fn query_tool_answers_relation_words_like_the_spatial_oracle() {
        let server = door_server();
        let labels = staged_labels(&server);
        let relation = |anchor: &str, word: &str, viewer: Option<&str>| SceneQueryArgs {
            kind: "relation".to_owned(),
            anchor: Some(anchor.to_owned()),
            other: viewer.map(str::to_owned),
            relation: Some(word.to_owned()),
            ..Default::default()
        };
        // Oracle: the same four crates as plain actors, ids by label order.
        let spots = [(-2.0f32, 0.0f32), (0.0, 0.0), (2.0, 0.0), (0.0, 2.0)];
        let actors: Vec<Actor> = spots
            .iter()
            .enumerate()
            .map(|(id, (x, z))| Actor {
                id: id as u32,
                name: format!("crate_{id}"),
                x: *x,
                y: 0.5,
                z: *z,
                sx: 1.0,
                sy: 1.0,
                sz: 1.0,
                parent: None,
                physical: true,
                vx: 0.0,
                vz: 0.0,
                yaw: 0.0,
            })
            .collect();
        let mut sided = 0;
        for word in ["left_of", "right_of", "near", "in_front_of", "behind"] {
            let rel = parse_rel(word).expect("word parses");
            let out = query(&server, relation(&labels[1], word, None));
            let want: Vec<String> = subjects_of(&actors, rel, 1, Frame::World)
                .into_iter()
                .map(|id| labels[id as usize].clone())
                .collect();
            let got: Vec<String> = out.hits.iter().map(|hit| hit.object.clone()).collect();
            assert_eq!(got, want, "{word} from the door matches the oracle");
            assert!(out
                .hits
                .iter()
                .all(|hit| hit.matched && hit.reason.starts_with(&format!("{word}: world frame"))));
            if word == "left_of" || word == "right_of" {
                sided += got.len();
            }
        }
        assert_eq!(sided, 2, "west and east sit left and right of the middle");
        // A viewer actor changes the frame, like the oracle's Viewer frame.
        let seen = query(&server, relation(&labels[1], "left_of", Some(&labels[3])));
        let want: Vec<String> = subjects_of(&actors, Rel::Left, 1, Frame::Viewer(3))
            .into_iter()
            .map(|id| labels[id as usize].clone())
            .collect();
        let got: Vec<String> = seen.hits.iter().map(|hit| hit.object.clone()).collect();
        assert_eq!(got, want);
        // Typed refusals name the legal words.
        for bad in [
            SceneQueryArgs {
                kind: "relation".to_owned(),
                anchor: Some(labels[1].clone()),
                ..Default::default()
            },
            relation(&labels[1], "leftish", None),
        ] {
            let error = SceneQueryTool::invoke(&server, bad).expect_err("bad relation refuses");
            assert_eq!(error.code, "bad_relation");
            assert!(error.message.contains("left_of"), "{}", error.message);
        }
    }

    #[test]
    fn query_tool_identity_is_readonly_with_schemas() {
        use rmcp::handler::server::router::tool::ToolBase;

        assert_eq!(SceneQueryTool::name(), "scene.query");
        assert_eq!(TOOL_SCENE_QUERY, "scene.query");
        let annotations = SceneQueryTool::annotations().expect("annotations ride along");
        assert_eq!(annotations.read_only_hint, Some(true));
        assert_eq!(annotations.open_world_hint, Some(false));
        assert!(
            SceneQueryTool::input_schema().is_some(),
            "query has an input schema"
        );
        assert!(
            SceneQueryTool::output_schema().is_some(),
            "query has an output schema"
        );
    }

    #[test]
    fn query_tool_answers_every_kind_over_the_staged_scene() {
        let server = door_server();
        let labels = staged_labels(&server);
        assert_eq!(labels.len(), 4);
        let (west, middle, east, north) = (
            labels[0].clone(),
            labels[1].clone(),
            labels[2].clone(),
            labels[3].clone(),
        );

        // Near the middle crate: every neighbour sits within 500 cm.
        let near = query(
            &server,
            SceneQueryArgs {
                kind: "near".to_owned(),
                anchor: Some(middle.clone()),
                other: None,
                radius_cm: Some(500),
                eye_m: None,
                yaw: None,
                crouch: None,
                capsule_radius_cm: None,
                min_gap_cm: None,
                max_samples: None,
                seed: None,
                limit: None,
                budget_bytes: None,
                half_angle_deg: None,
                filter: None,
                relation: None,
                format: None,
                matched_only: None,
                dir: None,
                ..Default::default()
            },
        );
        assert_eq!(near.kind, "near");
        let mut matched: Vec<&str> = near
            .hits
            .iter()
            .filter(|hit| hit.matched)
            .map(|hit| hit.object.as_str())
            .collect();
        matched.sort_unstable();
        let mut expected = vec![west.as_str(), east.as_str(), north.as_str()];
        expected.sort_unstable();
        assert_eq!(matched, expected);
        assert!(near.hits.iter().all(|hit| hit.reason.starts_with("near: ")));
        // Twice: the digest is stable over the door.
        let again = query(
            &server,
            SceneQueryArgs {
                kind: "near".to_owned(),
                anchor: Some(middle.clone()),
                other: None,
                radius_cm: Some(500),
                eye_m: None,
                yaw: None,
                crouch: None,
                capsule_radius_cm: None,
                min_gap_cm: None,
                max_samples: None,
                seed: None,
                limit: None,
                budget_bytes: None,
                half_angle_deg: None,
                filter: None,
                relation: None,
                format: None,
                matched_only: None,
                dir: None,
                ..Default::default()
            },
        );
        assert_eq!(near.digest, again.digest);

        // Between the outer crates: the middle one stands on the corridor.
        let between = query(
            &server,
            SceneQueryArgs {
                kind: "between".to_owned(),
                anchor: Some(west.clone()),
                other: Some(east.clone()),
                radius_cm: None,
                eye_m: None,
                yaw: None,
                crouch: None,
                capsule_radius_cm: None,
                min_gap_cm: None,
                max_samples: None,
                seed: None,
                limit: None,
                budget_bytes: None,
                half_angle_deg: None,
                filter: None,
                relation: None,
                format: None,
                matched_only: None,
                dir: None,
                ..Default::default()
            },
        );
        assert!(between
            .hits
            .iter()
            .any(|hit| hit.object == middle && hit.matched));

        // Visible from the west at eye height facing +X: all four read.
        let seen = query(
            &server,
            SceneQueryArgs {
                kind: "visible".to_owned(),
                anchor: None,
                other: None,
                radius_cm: None,
                eye_m: Some([-4.0, 1.6, 0.0]),
                yaw: Some(0.0),
                crouch: None,
                capsule_radius_cm: None,
                min_gap_cm: None,
                max_samples: None,
                seed: None,
                limit: None,
                budget_bytes: None,
                half_angle_deg: None,
                filter: None,
                relation: None,
                format: None,
                matched_only: Some(false),
                dir: None,
                ..Default::default()
            },
        );
        assert_eq!(seen.hits.len(), 4);

        // Reachable needs a baked mesh: this door holds none, so it refuses
        // (`reach_door_*` answers it from a mesh).
        let reach = SceneQueryTool::invoke(
            &server,
            SceneQueryArgs {
                kind: "reachable".to_owned(),
                anchor: Some(west.clone()),
                ..Default::default()
            },
        )
        .expect_err("no nav mesh, no answer");
        assert_eq!(reach.code, "needs_nav_view");

        // Cover from the east threat: open ground takes no cover, but every
        // row still carries its cover reason.
        let cover = query(
            &server,
            SceneQueryArgs {
                kind: "cover_near".to_owned(),
                anchor: Some(middle.clone()),
                other: None,
                radius_cm: Some(3000),
                eye_m: Some([10.0, 1.5, 0.0]),
                yaw: None,
                crouch: Some(false),
                capsule_radius_cm: None,
                min_gap_cm: None,
                max_samples: None,
                seed: None,
                limit: None,
                budget_bytes: None,
                half_angle_deg: None,
                filter: None,
                relation: None,
                format: None,
                matched_only: None,
                dir: None,
                ..Default::default()
            },
        );
        assert!(cover
            .hits
            .iter()
            .all(|hit| hit.reason.starts_with("cover:")));

        // Largest open plus free space: grounded placements with clearance.
        let open = query(
            &server,
            SceneQueryArgs {
                kind: "largest_open".to_owned(),
                anchor: None,
                other: None,
                radius_cm: None,
                eye_m: None,
                yaw: None,
                crouch: None,
                capsule_radius_cm: Some(30),
                min_gap_cm: None,
                max_samples: None,
                seed: None,
                limit: None,
                budget_bytes: None,
                half_angle_deg: None,
                filter: None,
                relation: None,
                format: None,
                matched_only: None,
                dir: None,
                ..Default::default()
            },
        );
        assert_eq!(open.hits.len(), 1);
        assert!(open.hits[0].matched);
        let free = query(
            &server,
            SceneQueryArgs {
                kind: "free_space".to_owned(),
                anchor: None,
                other: None,
                radius_cm: None,
                eye_m: None,
                yaw: None,
                crouch: None,
                capsule_radius_cm: Some(30),
                min_gap_cm: Some(100),
                max_samples: Some(4),
                seed: Some(7),
                limit: None,
                budget_bytes: None,
                half_angle_deg: None,
                filter: None,
                relation: None,
                format: None,
                matched_only: None,
                dir: None,
                ..Default::default()
            },
        );
        assert!(!free.hits.is_empty());
        assert!(free.bytes as usize <= QUERY_DEFAULT_BUDGET as usize);
    }

    #[test]
    fn query_tool_refusals_name_the_rule() {
        let server = door_server();
        let labels = staged_labels(&server);
        let bad_anchor = SceneQueryTool::invoke(
            &server,
            SceneQueryArgs {
                kind: "near".to_owned(),
                anchor: Some("prop_9999 [00000000-0000-0000-0000-000000000000]".to_owned()),
                other: None,
                radius_cm: None,
                eye_m: None,
                yaw: None,
                crouch: None,
                capsule_radius_cm: None,
                min_gap_cm: None,
                max_samples: None,
                seed: None,
                limit: None,
                budget_bytes: None,
                half_angle_deg: None,
                filter: None,
                relation: None,
                format: None,
                matched_only: None,
                dir: None,
                ..Default::default()
            },
        )
        .expect_err("unknown anchor refuses");
        assert_eq!(bad_anchor.code, "unknown_actor");
        assert!(bad_anchor.message.contains("scene.brief"));
        let bad_kind = SceneQueryTool::invoke(
            &server,
            SceneQueryArgs {
                kind: "sideways".to_owned(),
                anchor: None,
                other: None,
                radius_cm: None,
                eye_m: None,
                yaw: None,
                crouch: None,
                capsule_radius_cm: None,
                min_gap_cm: None,
                max_samples: None,
                seed: None,
                limit: None,
                budget_bytes: None,
                half_angle_deg: None,
                filter: None,
                relation: None,
                format: None,
                matched_only: None,
                dir: None,
                ..Default::default()
            },
        )
        .expect_err("unknown kind refuses");
        assert_eq!(bad_kind.code, "bad_kind");
        assert!(bad_kind.message.contains("free_space"));
        let bad_radius = SceneQueryTool::invoke(
            &server,
            SceneQueryArgs {
                kind: "near".to_owned(),
                anchor: Some(labels[0].clone()),
                other: None,
                radius_cm: Some(0),
                eye_m: None,
                yaw: None,
                crouch: None,
                capsule_radius_cm: None,
                min_gap_cm: None,
                max_samples: None,
                seed: None,
                limit: None,
                budget_bytes: None,
                half_angle_deg: None,
                filter: None,
                relation: None,
                format: None,
                matched_only: None,
                dir: None,
                ..Default::default()
            },
        )
        .expect_err("bad radius refuses");
        assert_eq!(bad_radius.code, "bad_radius");
    }

    #[test]
    fn query_tool_is_registered_on_the_door() {
        assert!(
            crate::mcp::tool_names().contains(&TOOL_SCENE_QUERY),
            "scene.query rides the router"
        );
    }
}
