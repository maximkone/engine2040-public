//! Intent placement: a model states relations, a deterministic solver
//! returns a transform or the minimal conflicting set.
//!
//! Consumer: the `scene.place` tool and the text-brief content loop. The
//! caller owns the data: input is an [`Intent`] plus a plain actor snapshot,
//! output is one [`TransformOut`] or a [`ConflictSet`]. Units are actor
//! meters in, integer centimeters inside (inherited from checks/relations);
//! the intent itself carries centimeters for [`Target::Point`] and
//! `distance_cm`.
//! Determinism scope: the same intent over the same actor set in any input
//! order gives the identical placement or the identical conflict set.
//! Failure mode: impossible intents return the minimal conflicting subset
//! (shrink by removal in intent order), never a guess.
//!
//! Language: [`Want`] reuses the [`Relation`](super::relation::Relation)
//! vocabulary models already see in briefs. `NextTo` covers "against" and
//! "near" (refined by `distance_cm`); `Facing` covers orientation; `Inside`,
//! `OnTopOf` and `Supports` are evaluated faithfully against relation geometry.
//! Targets are room walls (the union bounds over valid actors), object ids,
//! or explicit XZ points in centimeters. Wall convention: North is the
//! min-z bound, South max-z, West min-x, East max-x.
//!
//! Search: quantized placements over a [`GRID_CM`] cm grid inset into the
//! room bounds, [`YAW_STEPS`] yaws (`k * PI/4`, `0` faces +X as relations do).
//! Candidates scan z ascending, then x ascending, then yaw ascending; the
//! first candidate that satisfies every want and self-verifies wins.
//! Placements rest on the ground plane (`y = sy/2`); a first `OnTopOf`
//! object want rests on the supporter top instead.
//! Self-verify probes one [`Index3`](super::index3::Index3) built over the
//! scene-minus-object rows: a candidate wins when its integer-cm probe reads
//! clear via `overlaps_any` and rests at `min_y` 0. For grounded rooms this
//! coincides with the `overlap` + `floating` verdict on the placed id,
//! at microseconds per candidate instead of a full [`run`](super::run) per
//! candidate. Model rooms as thin wall actors, not solid
//! plates: the overlap check is XZ-based, so any placement inside a solid plate
//! trips verification by construction.
//!
//! Std-only. Never imports scene or editor types.

use super::index::{boxes_of, pos_known, size_known, to_cm, Box3};
use super::index3::{probe_of, Index3};
use super::navcheck::octant;
use super::relation::{Relation, FACING_RANGE_CM, NEXT_TO_CM, SUPPORT_GAP_CM};
use super::Actor;
use engine_core::Fnv64;
use std::collections::BTreeMap;

/// Search grid pitch in centimeters.
pub const GRID_CM: i32 = 10;
/// Yaw candidates per position: `k * PI/4` for `k` in `0..YAW_STEPS`.
pub const YAW_STEPS: i32 = 8;
/// Default wall edge-gap range `(min, max)` in cm for `NextTo` + wall.
pub const DEFAULT_WALL_GAP_CM: (i32, i32) = (0, 50);
/// Default object edge-gap range in cm for `NextTo` + object (relation `NextTo`).
pub const DEFAULT_OBJECT_GAP_CM: (i32, i32) = (0, NEXT_TO_CM);
/// Default point distance range `(min, max)` in cm for `NextTo` + point.
pub const DEFAULT_POINT_DIST_CM: (i32, i32) = (0, 50);

/// Room wall. North is the min-z bound, South max-z, West min-x, East max-x.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Wall {
    North,
    South,
    East,
    West,
}

/// Which actor to place: an id, or a name that must match exactly one actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectRef {
    Id(u32),
    Name(String),
}

/// What a want is measured against: a room wall, an actor id, or an
/// explicit XZ point in integer centimeters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Wall(Wall),
    Object(u32),
    Point { x_cm: i32, z_cm: i32 },
}

/// One relation constraint. `relation` reuses the graph vocabulary;
/// `distance_cm` is an inclusive `(min, max)` in cm and applies to `NextTo`
/// (edge gap for walls/objects, center distance for points) and `Facing`
/// (center distance to the faced object/point, or edge gap to a wall).
/// `None` selects the per-target default. A range with `min < 0` or
/// `max < min` never satisfies. Distance is ignored for `Inside`,
/// `OnTopOf` and `Supports`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Want {
    pub relation: Relation,
    pub target: Target,
    pub distance_cm: Option<(i32, i32)>,
}

/// One placement intent: move `object` so every want holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    pub object: ObjectRef,
    pub wants: Vec<Want>,
}

/// Solved placement. Units are meters, yaw radians.
#[derive(Debug, Clone, PartialEq)]
pub struct TransformOut {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
}

/// One solver-computed repair for a [`ConflictSet`]: relax a single want and
/// take the re-solved placement instead.
///
/// Consumer: `scene.place` readers placing without breaking scenes. The
/// caller owns the data: `dropped` indexes the failed [`Intent`]'s `wants`
/// (`dropped_want` repeats that entry so no lookup is needed), `placement`
/// is the quantized solver output satisfying every other want, `move_cm` is
/// the integer-cm XZ move from the object's current center to `placement`,
/// and `note` is a short stable code like `drop NextTo wall North`.
/// Units are actor meters in `placement`, integer centimeters in `move_cm`.
/// Determinism scope: the same intent over the same actor set in any input
/// order gives the identical hint (first solvable drop in intent order over
/// the frozen [`GRID_CM`] x [`YAW_STEPS`] search). There is no error case:
/// an empty hint list means no single-drop repair exists (or the
/// object/bounds are unusable, so the caller fixes identity first).
/// `Eq` holds for the frozen [`ConflictSet`] shape (every field is `Eq`):
/// the `f32` payload stays out of the `Eq` path — `placement` compares in
/// integer centimeters via [`to_cm`], yaw compares exact (always finite:
/// grid ints over 100, `sy * 0.5` with finite `sy`, `k * PI/4`) — so
/// solver-built hints stay reflexive. The `scene.place` consumer compares
/// conflicts by value beside the refusal.
#[derive(Debug, Clone)]
pub struct RepairHint {
    /// Index into the failed intent's `wants` that was relaxed.
    pub dropped: usize,
    /// The relaxed want, copied so callers need no intent lookup.
    pub dropped_want: Want,
    /// Solver placement satisfying every other want, self-verified clean.
    pub placement: TransformOut,
    /// Integer-cm XZ move from the object's current center to `placement`;
    /// `(0, 0)` when the current position is unknown.
    pub move_cm: [i32; 2],
    /// Short stable repair code for briefs and logs, never a newline.
    pub note: String,
}

impl PartialEq for RepairHint {
    fn eq(&self, other: &Self) -> bool {
        self.dropped == other.dropped
            && self.dropped_want == other.dropped_want
            && self.placement.id == other.placement.id
            && to_cm(self.placement.x) == to_cm(other.placement.x)
            && to_cm(self.placement.y) == to_cm(other.placement.y)
            && to_cm(self.placement.z) == to_cm(other.placement.z)
            && self.placement.yaw == other.placement.yaw
            && self.move_cm == other.move_cm
            && self.note == other.note
    }
}

impl Eq for RepairHint {}

/// Minimal conflicting subset: removing any single want from `unmet` makes
/// the remainder solvable (or the object/bounds are unusable, in which case
/// `unmet` holds every want). Order follows the intent.
///
/// `hints` carries solver-computed repairs beside the refusal: relaxing the
/// hint's `dropped` want and taking its `placement` verifies clean. Empty
/// when no single-drop repair exists or the object/bounds are unusable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictSet {
    pub unmet: Vec<Want>,
    pub reason: String,
    pub hints: Vec<RepairHint>,
}

/// Union XZ bounds over valid actors, in integer centimeters.
#[derive(Debug, Clone, Copy)]
struct Bounds {
    min_x: i32,
    max_x: i32,
    min_z: i32,
    max_z: i32,
}

fn valid_box(a: &Actor) -> bool {
    pos_known(a) && size_known(a) && a.sx > 0.0 && a.sy > 0.0 && a.sz > 0.0
}

fn room_bounds(actors: &[Actor]) -> Option<Bounds> {
    let good: Vec<Actor> = actors.iter().filter(|a| valid_box(a)).cloned().collect();
    if good.is_empty() {
        return None;
    }
    let boxes = boxes_of(&good);
    let mut b = Bounds {
        min_x: i32::MAX,
        max_x: i32::MIN,
        min_z: i32::MAX,
        max_z: i32::MIN,
    };
    for box3 in &boxes {
        b.min_x = b.min_x.min(box3.min_x);
        b.max_x = b.max_x.max(box3.max_x);
        b.min_z = b.min_z.min(box3.min_z);
        b.max_z = b.max_z.max(box3.max_z);
    }
    Some(b)
}

/// Resolved object geometry plus its grounded rest height in meters.
struct Placed {
    id: u32,
    sx: f32,
    sy: f32,
    sz: f32,
    y_m: f32,
}

/// Typed refusal from [`resolve_object`]: which lookup failed and why.
/// Renders exactly the legacy `reason` strings via [`Display`], so conflict
/// reports keep their bytes while callers can match on the variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaceError {
    /// No actor matches the intent's object reference (`id {id}` or `name '{name}'`).
    UnknownObject(String),
    /// A reference matches more than one actor.
    AmbiguousObject {
        /// The reference label (`id {id}` or `name '{name}'`).
        label: String,
        /// How many actors matched.
        matches: usize,
    },
    /// The matched actor has no usable size.
    UnknownSize(u32),
}

impl std::fmt::Display for PlaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownObject(label) => write!(f, "unknown-object {label}"),
            Self::AmbiguousObject { label, matches } => {
                write!(f, "ambiguous-object {label}: {matches} matches")
            }
            Self::UnknownSize(id) => write!(f, "unknown-size id {id}"),
        }
    }
}

impl std::error::Error for PlaceError {}

impl PlaceError {
    /// The conflict `reason` string for [`ConflictSet`]: exactly the legacy
    /// bytes via [`Display`].
    fn into_reason(self) -> String {
        self.to_string()
    }
}

fn resolve_object(intent: &Intent, actors: &[Actor]) -> Result<Placed, PlaceError> {
    let found: Vec<&Actor> = match &intent.object {
        ObjectRef::Id(id) => actors.iter().filter(|a| a.id == *id).collect(),
        ObjectRef::Name(name) => actors.iter().filter(|a| &a.name == name).collect(),
    };
    if found.is_empty() {
        let label = match &intent.object {
            ObjectRef::Id(id) => format!("id {id}"),
            ObjectRef::Name(name) => format!("name '{name}'"),
        };
        return Err(PlaceError::UnknownObject(label));
    }
    if found.len() > 1 {
        let label = match &intent.object {
            ObjectRef::Id(id) => format!("id {id}"),
            ObjectRef::Name(name) => format!("name '{name}'"),
        };
        return Err(PlaceError::AmbiguousObject {
            label,
            matches: found.len(),
        });
    }
    let a = found[0];
    if !size_known(a) || a.sx <= 0.0 || a.sy <= 0.0 || a.sz <= 0.0 {
        return Err(PlaceError::UnknownSize(a.id));
    }
    Ok(Placed {
        id: a.id,
        sx: a.sx,
        sy: a.sy,
        sz: a.sz,
        y_m: a.sy * 0.5,
    })
}

/// Solver working set: frozen maps plus bounds, all order-invariant.
struct Ctx<'a> {
    bounds: Bounds,
    boxes: BTreeMap<u32, Box3>,
    by_id: BTreeMap<u32, &'a Actor>,
}

fn context<'a>(actors: &'a [Actor], bounds: Bounds) -> Ctx<'a> {
    let good: Vec<Actor> = actors.iter().filter(|a| valid_box(a)).cloned().collect();
    let boxes: BTreeMap<u32, Box3> = boxes_of(&good).into_iter().map(|b| (b.id, b)).collect();
    let by_id: BTreeMap<u32, &'a Actor> = actors.iter().map(|a| (a.id, a)).collect();
    Ctx {
        bounds,
        boxes,
        by_id,
    }
}

fn valid_range(range: Option<(i32, i32)>) -> Option<(i64, i64)> {
    range
        .map(|(lo, hi)| (i64::from(lo), i64::from(hi)))
        .filter(|(lo, hi)| *lo >= 0 && *hi >= *lo)
}

/// Inclusive range check over a squared center distance.
fn dist_in(center2: i64, range: (i64, i64)) -> bool {
    range.0 * range.0 <= center2 && center2 <= range.1 * range.1
}

fn center2(cx: i32, cz: i32, tx: i32, tz: i32) -> i64 {
    let dx = i64::from(cx) - i64::from(tx);
    let dz = i64::from(cz) - i64::from(tz);
    dx * dx + dz * dz
}

fn edge_gap2(min_x: i32, max_x: i32, min_z: i32, max_z: i32, box3: &Box3) -> i64 {
    let gx = (i64::from(min_x.max(box3.min_x)) - i64::from(max_x.min(box3.max_x))).max(0);
    let gz = (i64::from(min_z.max(box3.min_z)) - i64::from(max_z.min(box3.max_z))).max(0);
    gx * gx + gz * gz
}

fn yaw_of(idx: i32) -> f32 {
    idx as f32 * core::f32::consts::FRAC_PI_4
}

/// True when the probe at `(cx, cz)` with `yaw_idx` satisfies `want`.
/// All geometry runs on integer centimeters.
fn satisfies(want: &Want, cx: i32, cz: i32, yaw_idx: i32, obj: &Placed, ctx: &Ctx<'_>) -> bool {
    let hx = to_cm(obj.sx).abs() / 2;
    let hz = to_cm(obj.sz).abs() / 2;
    let (min_x, max_x) = (cx - hx, cx + hx);
    let (min_z, max_z) = (cz - hz, cz + hz);
    let y_cm = to_cm(obj.y_m);
    let half_y = to_cm(obj.sy).abs() / 2;
    let (p_min_y, p_max_y) = (y_cm - half_y, y_cm + half_y);
    let b = &ctx.bounds;
    match (want.relation, want.target) {
        (Relation::NextTo, Target::Wall(w)) => {
            let Some(range) = valid_range_or(want.distance_cm, DEFAULT_WALL_GAP_CM) else {
                return false;
            };
            let gap = match w {
                Wall::North => i64::from(min_z) - i64::from(b.min_z),
                Wall::South => i64::from(b.max_z) - i64::from(max_z),
                Wall::West => i64::from(min_x) - i64::from(b.min_x),
                Wall::East => i64::from(b.max_x) - i64::from(max_x),
            };
            range.0 <= gap && gap <= range.1
        }
        (Relation::NextTo, Target::Object(id)) => {
            let Some(range) = valid_range_or(want.distance_cm, DEFAULT_OBJECT_GAP_CM) else {
                return false;
            };
            let Some(box3) = ctx.boxes.get(&id) else {
                return false;
            };
            if box3.id == obj.id {
                return false;
            }
            if !(p_min_y < box3.max_y && box3.min_y < p_max_y) {
                return false;
            }
            dist_in(edge_gap2(min_x, max_x, min_z, max_z, box3), range)
        }
        (Relation::NextTo, Target::Point { x_cm, z_cm }) => {
            let Some(range) = valid_range_or(want.distance_cm, DEFAULT_POINT_DIST_CM) else {
                return false;
            };
            dist_in(center2(cx, cz, x_cm, z_cm), range)
        }
        (Relation::Facing, Target::Wall(w)) => {
            let (fx, fz) = octant(yaw_of(yaw_idx));
            let toward = match w {
                Wall::North => fz < 0,
                Wall::South => fz > 0,
                Wall::West => fx < 0,
                Wall::East => fx > 0,
            };
            if !toward {
                return false;
            }
            match want.distance_cm {
                None => true,
                Some(_) => {
                    let Some(range) = valid_range(want.distance_cm) else {
                        return false;
                    };
                    let gap = match w {
                        Wall::North => i64::from(min_z) - i64::from(b.min_z),
                        Wall::South => i64::from(b.max_z) - i64::from(max_z),
                        Wall::West => i64::from(min_x) - i64::from(b.min_x),
                        Wall::East => i64::from(b.max_x) - i64::from(max_x),
                    };
                    range.0 <= gap && gap <= range.1
                }
            }
        }
        (Relation::Facing, Target::Object(id)) => {
            let Some(target) = ctx.by_id.get(&id) else {
                return false;
            };
            if target.id == obj.id || !pos_known(target) {
                return false;
            }
            face_holds(cx, cz, yaw_idx, to_cm(target.x), to_cm(target.z))
                && dist_holds(cx, cz, to_cm(target.x), to_cm(target.z), want.distance_cm)
        }
        (Relation::Facing, Target::Point { x_cm, z_cm }) => {
            face_holds(cx, cz, yaw_idx, x_cm, z_cm)
                && dist_holds(cx, cz, x_cm, z_cm, want.distance_cm)
        }
        (Relation::Inside, Target::Object(id)) => {
            let Some(box3) = ctx.boxes.get(&id) else {
                return false;
            };
            if box3.id == obj.id {
                return false;
            }
            contains_probe(box3, min_x, p_min_y, min_z, max_x, p_max_y, max_z)
        }
        (Relation::Inside, _) => false,
        (Relation::OnTopOf, Target::Object(id)) => {
            let Some(box3) = ctx.boxes.get(&id) else {
                return false;
            };
            if box3.id == obj.id {
                return false;
            }
            let hit = min_x < box3.max_x
                && box3.min_x < max_x
                && min_z < box3.max_z
                && box3.min_z < max_z;
            if !hit {
                return false;
            }
            let gap = i64::from(support_gap_cm(box3.max_y, p_min_y));
            0 <= gap && gap <= i64::from(SUPPORT_GAP_CM)
        }
        (Relation::OnTopOf, _) => false,
        (Relation::Supports, Target::Object(id)) => {
            let Some(target) = ctx.by_id.get(&id) else {
                return false;
            };
            if target.id == obj.id || !pos_known(target) || !size_known(target) {
                return false;
            }
            let thx = to_cm(target.sx).abs() / 2;
            let thz = to_cm(target.sz).abs() / 2;
            let (tcx, tcz) = (to_cm(target.x), to_cm(target.z));
            let hit =
                tcx - thx < max_x && min_x < tcx + thx && tcz - thz < max_z && min_z < tcz + thz;
            if !hit {
                return false;
            }
            let t_min_y = to_cm(target.y) - to_cm(target.sy).abs() / 2;
            let gap = i64::from(support_gap_cm(p_max_y, t_min_y));
            0 <= gap && gap <= i64::from(SUPPORT_GAP_CM)
        }
        (Relation::Supports, _) => false,
    }
}

fn valid_range_or(want: Option<(i32, i32)>, fallback: (i32, i32)) -> Option<(i64, i64)> {
    match want {
        Some(_) => valid_range(want),
        None => Some((i64::from(fallback.0), i64::from(fallback.1))),
    }
}

/// Relation facing: strictly ahead in the yaw-snapped half-plane and in range.
fn face_holds(cx: i32, cz: i32, yaw_idx: i32, tx: i32, tz: i32) -> bool {
    if cx == tx && cz == tz {
        return false;
    }
    let (fx, fz) = octant(yaw_of(yaw_idx));
    let dx = i64::from(tx) - i64::from(cx);
    let dz = i64::from(tz) - i64::from(cz);
    if i64::from(fx) * dx + i64::from(fz) * dz <= 0 {
        return false;
    }
    let range = i64::from(FACING_RANGE_CM);
    dx * dx + dz * dz <= range * range
}

/// Optional center-distance refinement; `None` always holds.
fn dist_holds(cx: i32, cz: i32, tx: i32, tz: i32, distance_cm: Option<(i32, i32)>) -> bool {
    match distance_cm {
        None => true,
        Some(_) => match valid_range(distance_cm) {
            Some(range) => dist_in(center2(cx, cz, tx, tz), range),
            None => false,
        },
    }
}

/// Bounds containment with strict volume, mirroring relation `contains_box`.
fn contains_probe(
    outer: &Box3,
    min_x: i32,
    min_y: i32,
    min_z: i32,
    max_x: i32,
    max_y: i32,
    max_z: i32,
) -> bool {
    let vol = |lo_x: i32, lo_y: i32, lo_z: i32, hi_x: i32, hi_y: i32, hi_z: i32| -> i64 {
        (i64::from(hi_x) - i64::from(lo_x)).max(0)
            * (i64::from(hi_y) - i64::from(lo_y)).max(0)
            * (i64::from(hi_z) - i64::from(lo_z)).max(0)
    };
    let v_outer = vol(
        outer.min_x,
        outer.min_y,
        outer.min_z,
        outer.max_x,
        outer.max_y,
        outer.max_z,
    );
    let v_probe = vol(min_x, min_y, min_z, max_x, max_y, max_z);
    v_outer > v_probe
        && outer.min_x <= min_x
        && max_x <= outer.max_x
        && outer.min_y <= min_y
        && max_y <= outer.max_y
        && outer.min_z <= min_z
        && max_z <= outer.max_z
}

/// Frozen candidate grid: z ascending, x ascending, yaw ascending, streamed
/// without materializing. Dense rooms span millions of cells while the scan
/// usually exits in the first rows, so a `Vec` would be pure waste per solve.
struct Grid {
    x_lo: i32,
    x_hi: i32,
    z_lo: i32,
    z_hi: i32,
}

fn grid_of(obj: &Placed, bounds: &Bounds) -> Option<Grid> {
    let hx = to_cm(obj.sx).abs() / 2;
    let hz = to_cm(obj.sz).abs() / 2;
    let (x_lo, x_hi) = (bounds.min_x + hx, bounds.max_x - hx);
    let (z_lo, z_hi) = (bounds.min_z + hz, bounds.max_z - hz);
    if x_lo > x_hi || z_lo > z_hi {
        return None;
    }
    Some(Grid {
        x_lo,
        x_hi,
        z_lo,
        z_hi,
    })
}

fn wants_hold(wants: &[Want], cx: i32, cz: i32, yaw_idx: i32, obj: &Placed, ctx: &Ctx<'_>) -> bool {
    wants
        .iter()
        .all(|w| satisfies(w, cx, cz, yaw_idx, obj, ctx))
}

/// Rest epsilon in integer cm. Mirrors the `support`/`floating` convention
/// in `checks.rs` (`FLOAT_EPS_CM`): a `min_y` at or under it rests.
const FLOAT_EPS_CM: i32 = 1;

/// Signed plane gap in integer cm between a support top and a placed bottom.
/// `0` means touching, `1..=SUPPORT_GAP_CM` means resting contact within
/// `SUPPORT_GAP_CM`, negative means overlap, past `SUPPORT_GAP_CM` means floating.
pub(crate) fn support_gap_cm(support_top_cm: i32, placed_min_cm: i32) -> i32 {
    placed_min_cm - support_top_cm
}

/// Support surface kind: floor vs raised tops (wide tops vs small steps).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SupportKind {
    Floor,
    ShelfTop,
    Tread,
}

/// One support surface, integer cm. `supporter` is `None` for the floor.
/// `gap_cm` reuses `support_gap_cm`; `rests` is floor at ground epsilon else
/// gap within `SUPPORT_GAP_CM` plus XZ overlap; `clear` probes the rest pose
/// against the caller's `Index3`. Sorted by `(top_cm, supporter)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupportSurface {
    pub supporter: Option<u32>,
    pub kind: SupportKind,
    pub top_cm: i32,
    pub min_x: i32,
    pub max_x: i32,
    pub min_z: i32,
    pub max_z: i32,
    pub gap_cm: i32,
    pub rests: bool,
    pub clear: bool,
}

/// List floor plus every raised top for one object's XZ footprint.
/// Same actors any order give the same vec; empty for unknown objects.
/// Budget: O(n) surfaces plus one probe each; room-scale, never the tick.
pub fn enumerate_support_surfaces(
    actors: &[Actor],
    object_id: u32,
    index: &Index3,
) -> Vec<SupportSurface> {
    let Some(me) = actors.iter().find(|a| a.id == object_id) else {
        return Vec::new();
    };
    if !valid_box(me) {
        return Vec::new();
    }
    let hx = to_cm(me.sx).abs() / 2;
    let hy = to_cm(me.sy).abs() / 2;
    let hz = to_cm(me.sz).abs() / 2;
    let (cx, cz) = (to_cm(me.x), to_cm(me.z));
    let min_y = to_cm(me.y) - hy;
    let (ox0, ox1) = (cx - hx, cx + hx);
    let (oz0, oz1) = (cz - hz, cz + hz);
    let live = super::index3::live_boxes(actors);
    let (mut bx0, mut bx1, mut bz0, mut bz1) = (ox0, ox1, oz0, oz1);
    let mut seeded = false;
    for b in &live {
        if b.id == object_id {
            continue;
        }
        if !seeded {
            (bx0, bx1, bz0, bz1) = (b.min_x, b.max_x, b.min_z, b.max_z);
            seeded = true;
        } else {
            bx0 = bx0.min(b.min_x);
            bx1 = bx1.max(b.max_x);
            bz0 = bz0.min(b.min_z);
            bz1 = bz1.max(b.max_z);
        }
    }
    let mut out = Vec::new();
    out.push(SupportSurface {
        supporter: None,
        kind: SupportKind::Floor,
        top_cm: 0,
        min_x: bx0,
        max_x: bx1,
        min_z: bz0,
        max_z: bz1,
        gap_cm: support_gap_cm(0, min_y),
        rests: min_y <= FLOAT_EPS_CM,
        clear: !index.overlaps_any(&probe_of(cx, hy, cz, hx, hy, hz), object_id),
    });
    for b in &live {
        if b.id == object_id {
            continue;
        }
        let kind =
            if b.max_x - b.min_x <= 120 && b.max_z - b.min_z <= 120 && b.max_y - b.min_y <= 120 {
                SupportKind::Tread
            } else {
                SupportKind::ShelfTop
            };
        let gap = support_gap_cm(b.max_y, min_y);
        let hit = ox0 < b.max_x && b.min_x < ox1 && oz0 < b.max_z && b.min_z < oz1;
        out.push(SupportSurface {
            supporter: Some(b.id),
            kind,
            top_cm: b.max_y,
            min_x: b.min_x,
            max_x: b.max_x,
            min_z: b.min_z,
            max_z: b.max_z,
            gap_cm: gap,
            rests: (0..=SUPPORT_GAP_CM).contains(&gap) && hit,
            clear: !index.overlaps_any(&probe_of(cx, b.max_y + hy, cz, hx, hy, hz), object_id),
        });
    }
    out.sort_by_key(|s| (s.top_cm, s.supporter));
    out
}

/// Stacked rest center for the first `OnTopOf` object want, in integer-cm
/// center `y`. `None` keeps the grounded rest; self or unknown supporters
/// stay `None` so they keep failing as conflicts.
fn rest_for(wants: &[Want], obj: &Placed, ctx: &Ctx<'_>) -> Option<i32> {
    for want in wants {
        if want.relation != Relation::OnTopOf {
            continue;
        }
        let Target::Object(id) = want.target else {
            continue;
        };
        if id == obj.id {
            return None;
        }
        let box3 = ctx.boxes.get(&id)?;
        let half_y = to_cm(obj.sy).abs() / 2;
        return Some(box3.max_y + half_y);
    }
    None
}

/// Self-verify behind the 3D index: the candidate probe must clear
/// every indexed volume (strict 3D, the solver clearance contract) and rest on
/// the ground plane, or on the stacked support top when `rest_cm` holds the
/// integer-cm center from `rest_for`. Unknown or degenerate room rows index
/// nothing, like the relation isolates, so they never block a placement.
fn verifies(index: &Index3, obj: &Placed, out: &TransformOut, rest_cm: Option<i32>) -> bool {
    let hx = to_cm(obj.sx).abs() / 2;
    let hy = to_cm(obj.sy).abs() / 2;
    let hz = to_cm(obj.sz).abs() / 2;
    let probe = probe_of(to_cm(out.x), to_cm(out.y), to_cm(out.z), hx, hy, hz);
    if index.overlaps_any(&probe, obj.id) {
        return false;
    }
    match rest_cm {
        None => to_cm(out.y) - hy <= FLOAT_EPS_CM,
        Some(rest) => (to_cm(out.y) - rest).abs() <= FLOAT_EPS_CM,
    }
}

fn transform_of(obj: &Placed, cx: i32, cz: i32, yaw_idx: i32) -> TransformOut {
    TransformOut {
        id: obj.id,
        x: cx as f32 / 100.0,
        y: obj.y_m,
        z: cz as f32 / 100.0,
        yaw: yaw_of(yaw_idx),
    }
}

fn search(
    wants: &[Want],
    obj: &Placed,
    ctx: &Ctx<'_>,
    index: &Index3,
    grid: Option<&Grid>,
) -> Option<TransformOut> {
    let grid = grid?;
    let rest_cm = rest_for(wants, obj, ctx);
    let stacked_y_m = match rest_cm {
        None => obj.y_m,
        Some(rest) => {
            let Ok(rest_i16) = i16::try_from(rest) else {
                return None;
            };
            f32::from(rest_i16) / 100.0
        }
    };
    let stacked = Placed {
        id: obj.id,
        sx: obj.sx,
        sy: obj.sy,
        sz: obj.sz,
        y_m: stacked_y_m,
    };
    let mut cz = grid.z_lo;
    while cz <= grid.z_hi {
        let mut cx = grid.x_lo;
        while cx <= grid.x_hi {
            for yaw_idx in 0..YAW_STEPS {
                if !wants_hold(wants, cx, cz, yaw_idx, &stacked, ctx) {
                    continue;
                }
                let out = transform_of(&stacked, cx, cz, yaw_idx);
                if verifies(index, &stacked, &out, rest_cm) {
                    return Some(out);
                }
            }
            cx += GRID_CM;
        }
        cz += GRID_CM;
    }
    None
}

/// Short stable relation code for hint notes, in the frozen vocabulary.
fn rel_code(rel: Relation) -> &'static str {
    match rel {
        Relation::NextTo => "NextTo",
        Relation::Facing => "Facing",
        Relation::Inside => "Inside",
        Relation::OnTopOf => "OnTopOf",
        Relation::Supports => "Supports",
    }
}

/// Short stable target code for hint notes, in integer centimeters.
fn target_code(target: &Target) -> String {
    match *target {
        Target::Wall(w) => match w {
            Wall::North => "wall North".to_string(),
            Wall::South => "wall South".to_string(),
            Wall::West => "wall West".to_string(),
            Wall::East => "wall East".to_string(),
        },
        Target::Object(id) => format!("object {id}"),
        Target::Point { x_cm, z_cm } => format!("point ({x_cm},{z_cm})"),
    }
}

/// Current object center in integer centimeters, or `(0, 0)` when the
/// snapshot holds no known position for `id`.
fn current_center_cm(actors: &[Actor], id: u32) -> (i32, i32) {
    actors
        .iter()
        .find(|a| a.id == id)
        .filter(|a| pos_known(a))
        .map(|a| (to_cm(a.x), to_cm(a.z)))
        .unwrap_or((0, 0))
}

/// Solver-computed repair hints for one minimal conflict, beside the frozen
/// solver: the first index in `conflict` (intent order) whose removal lets
/// the full intent re-solve gives one hint; anything else gives none. Pure:
/// runs the same frozen grid search over the same snapshot, never the tick.
fn repair_hints(
    intent: &Intent,
    conflict: &[usize],
    obj: &Placed,
    ctx: &Ctx<'_>,
    index: &Index3,
    grid: Option<&Grid>,
    actors: &[Actor],
) -> Vec<RepairHint> {
    let (cur_x, cur_z) = current_center_cm(actors, obj.id);
    for &d in conflict {
        let trial: Vec<Want> = intent
            .wants
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != d)
            .map(|(_, w)| *w)
            .collect();
        if let Some(out) = search(&trial, obj, ctx, index, grid) {
            let move_cm = [to_cm(out.x) - cur_x, to_cm(out.z) - cur_z];
            let want = intent.wants[d];
            let note = format!(
                "drop {} {} (move [{},{}] cm)",
                rel_code(want.relation),
                target_code(&want.target),
                move_cm[0],
                move_cm[1]
            );
            return vec![RepairHint {
                dropped: d,
                dropped_want: want,
                placement: out,
                move_cm,
                note,
            }];
        }
    }
    Vec::new()
}

/// SP38 audit verdict for one [`ConflictSet`] beneath the SP17 hints.
/// `Minimal` means the geometric shape held; `HonestFull` means an
/// identity or bounds refusal carried the full want set with empty hints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictAudit {
    /// Subset of the intent in intent order with a minimal-conflict reason.
    Minimal,
    /// Full carry-over for identity or bounds faults with honestly empty hints.
    HonestFull,
}

/// Integer-only subset check beneath the SP17 hints.
///
/// Consumer: the `scene.place` door and the playtest gold harness read the
/// audit beside the refusal. Units are intent indices and want counts; no
/// meters cross the boundary. Determinism: the same intent and conflict in
/// any actor order give the same verdict. Errors: `None` on any shape miss;
/// the caller keeps the solver refusal. Never mutates the graph and never
/// touches placement scalars: only `Want` equality, index order and note bytes.
pub fn audit_subset(intent: &Intent, conflict: &ConflictSet) -> Option<ConflictAudit> {
    if conflict.unmet.is_empty() || conflict.reason.is_empty() {
        return None;
    }
    // Subset in intent order without allocating: each unmet want maps to a
    // strictly later intent index.
    let mut prev: Option<usize> = None;
    for want in &conflict.unmet {
        let start = prev.map_or(0, |p| p + 1);
        let mut found: Option<usize> = None;
        let mut i = start;
        while i < intent.wants.len() {
            if intent.wants[i] == *want {
                found = Some(i);
                break;
            }
            i += 1;
        }
        let pos = found?;
        prev = Some(pos);
    }
    let full = conflict.unmet.len() == intent.wants.len();
    let is_minimal = conflict.reason.contains("minimal conflict");
    if is_minimal {
        if conflict.hints.len() > 1 {
            return None;
        }
        if let Some(hint) = conflict.hints.first() {
            if hint.dropped >= intent.wants.len() {
                return None;
            }
            if hint.dropped_want != intent.wants[hint.dropped] {
                return None;
            }
            if hint.note.is_empty() || hint.note.contains('\n') {
                return None;
            }
            if !hint.note.starts_with("drop ") {
                return None;
            }
        }
        return Some(ConflictAudit::Minimal);
    }
    let identity = conflict.reason.contains("unknown-object")
        || conflict.reason.contains("ambiguous-object")
        || conflict.reason.contains("unknown-size")
        || conflict.reason.contains("no-bounds");
    if identity && full && conflict.hints.is_empty() {
        return Some(ConflictAudit::HonestFull);
    }
    None
}

/// Integer-only identity check for two [`ConflictSet`]s.
///
/// Consumer and determinism match [`audit_subset`]. Compares the integer
/// shape only (`unmet`, `reason`, hint indices, wants, moves and notes) and
/// never the placement scalars, so shuffled actor input must still read
/// identical. Never mutates either set.
pub fn audit_identical(a: &ConflictSet, b: &ConflictSet) -> bool {
    if a.unmet.len() != b.unmet.len() || a.reason != b.reason {
        return false;
    }
    let mut i = 0;
    while i < a.unmet.len() {
        if a.unmet[i] != b.unmet[i] {
            return false;
        }
        i += 1;
    }
    if a.hints.len() != b.hints.len() {
        return false;
    }
    let mut j = 0;
    while j < a.hints.len() {
        let x = &a.hints[j];
        let y = &b.hints[j];
        if x.dropped != y.dropped
            || x.dropped_want != y.dropped_want
            || x.move_cm != y.move_cm
            || x.note != y.note
        {
            return false;
        }
        j += 1;
    }
    true
}

/// Canonical audit digest, perm-stable integer-only; consumer harness; spike `05b20def` x2.
pub fn canonical_audit_digest(conflicts: &[ConflictSet]) -> u32 {
    fn want(h: &mut Fnv64, w: &Want) {
        h.write(w.relation.name().as_bytes());
        match w.target {
            Target::Wall(x) => {
                h.write_u8(0);
                h.write_u8(match x {
                    Wall::North => 0,
                    Wall::South => 1,
                    Wall::East => 2,
                    Wall::West => 3,
                });
            }
            Target::Object(id) => {
                h.write_u8(1);
                h.write_u32(id);
            }
            Target::Point { x_cm, z_cm } => {
                h.write_u8(2);
                h.write_i32(x_cm);
                h.write_i32(z_cm);
            }
        }
        match w.distance_cm {
            None => h.write_u8(0),
            Some((a, b)) => {
                h.write_u8(1);
                h.write_i32(a);
                h.write_i32(b);
            }
        }
    }
    let mut h = Fnv64::new();
    h.write(b"e2placeaudit1");
    h.write_u64(u64::try_from(conflicts.len()).unwrap_or(u64::MAX));
    for c in conflicts {
        h.write_u64(u64::try_from(c.unmet.len()).unwrap_or(u64::MAX));
        for w in &c.unmet {
            want(&mut h, w);
        }
        h.write_u64(u64::try_from(c.hints.len()).unwrap_or(u64::MAX));
        for x in &c.hints {
            h.write_u64(u64::try_from(x.dropped).unwrap_or(u64::MAX));
            want(&mut h, &x.dropped_want);
            h.write_i32(x.move_cm[0]);
            h.write_i32(x.move_cm[1]);
        }
    }
    u32::try_from(h.finish() & 0xffff_ffff).unwrap_or(u32::MAX)
}

/// Solve one placement intent over an actor snapshot.
///
/// Returns the first quantized placement (10 cm grid, 8 yaws, z/x/yaw
/// order) that satisfies every want and passes the `overlap`/`floating`
/// self-verification, or the minimal conflicting subset on failure.
pub fn solve(intent: &Intent, actors: &[Actor]) -> Result<Vec<TransformOut>, ConflictSet> {
    let unmet_all = || intent.wants.clone();
    let obj = match resolve_object(intent, actors) {
        Ok(obj) => obj,
        Err(error) => {
            return Err(ConflictSet {
                unmet: unmet_all(),
                reason: error.into_reason(),
                hints: Vec::default(), // empty vec never allocates; conflict-only path outside the fixed tick.
            });
        }
    };
    let Some(bounds) = room_bounds(actors) else {
        return Err(ConflictSet {
            unmet: unmet_all(),
            reason: "no-bounds: no valid actor anchors the room".to_string(),
            hints: Vec::default(), // empty vec never allocates; conflict-only path outside the fixed tick.
        });
    };
    let ctx = context(actors, bounds);
    // One index build per solve over the scene-minus-object rows: every
    // candidate verify below (first hit and each conflict trial) probes it.
    let rest: Vec<Actor> = actors.iter().filter(|a| a.id != obj.id).cloned().collect();
    let index = Index3::build(&rest);
    let grid = grid_of(&obj, &bounds);
    #[cfg(debug_assertions)]
    {
        let surfaces = enumerate_support_surfaces(actors, obj.id, &index);
        for w in &intent.wants {
            if w.relation != Relation::OnTopOf {
                continue;
            }
            let Target::Object(wid) = w.target else {
                continue;
            };
            if wid == obj.id {
                continue;
            }
            if let Some(s) = surfaces.iter().find(|s| s.supporter == Some(wid)) {
                if let Some(rest_cm) = rest_for(&intent.wants, &obj, &ctx) {
                    debug_assert_eq!(s.top_cm + to_cm(obj.sy).abs() / 2, rest_cm);
                }
            }
            break;
        }
    }
    if let Some(out) = search(&intent.wants, &obj, &ctx, &index, grid.as_ref()) {
        return Ok(vec![out]);
    }
    // Minimal conflict: shrink by removal in intent order. Removing want `i`
    // sticks only while the remainder stays unsatisfiable.
    let mut conflict: Vec<usize> = (0..intent.wants.len()).collect();
    for i in 0..intent.wants.len() {
        if !conflict.contains(&i) {
            continue;
        }
        let trial: Vec<Want> = conflict
            .iter()
            .filter(|&&j| j != i)
            .map(|&j| intent.wants[j])
            .collect();
        if search(&trial, &obj, &ctx, &index, grid.as_ref()).is_none() {
            conflict.retain(|&j| j != i);
        }
    }
    let unmet: Vec<Want> = conflict.iter().map(|&j| intent.wants[j]).collect();
    let reason = format!(
        "no placement on {GRID_CM} cm grid x {YAW_STEPS} yaws satisfies {} want(s); minimal conflict covers {} want(s)",
        intent.wants.len(),
        unmet.len(),
    );
    let hints = repair_hints(intent, &conflict, &obj, &ctx, &index, grid.as_ref(), actors);
    Err(ConflictSet {
        unmet,
        reason,
        hints,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relation::Relation;

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

    /// Walled room, 10.2 m square outer edge: thin walls plus a gate slab
    /// near the north wall and a brazier starter. All pairs XZ-disjoint
    /// (corners touch at most), everything grounded: `run()` is clean.
    fn room() -> Vec<Actor> {
        vec![
            mk(0, "wall_north", 0.0, 1.0, -5.0, 9.8, 2.0, 0.2),
            mk(1, "wall_south", 0.0, 1.0, 5.0, 9.8, 2.0, 0.2),
            mk(2, "wall_west", -5.0, 1.0, 0.0, 0.2, 2.0, 10.0),
            mk(3, "wall_east", 5.0, 1.0, 0.0, 0.2, 2.0, 10.0),
            mk(4, "gate", 3.0, 0.5, -4.5, 1.0, 1.0, 0.5),
            mk(5, "brazier", 0.0, 0.5, 2.0, 0.6, 1.0, 0.6),
        ]
    }

    fn placed_scene(actors: &[Actor], out: &TransformOut) -> Vec<Actor> {
        actors
            .iter()
            .map(|a| {
                if a.id == out.id {
                    Actor {
                        x: out.x,
                        y: out.y,
                        z: out.z,
                        yaw: out.yaw,
                        ..a.clone()
                    }
                } else {
                    a.clone()
                }
            })
            .collect()
    }

    /// Board example: "brazier against the north wall, 2 m from the gate".
    fn board_intent() -> Intent {
        Intent {
            object: ObjectRef::Name("brazier".to_string()),
            wants: vec![
                Want {
                    relation: Relation::NextTo,
                    target: Target::Wall(Wall::North),
                    distance_cm: Some((0, 60)),
                },
                Want {
                    relation: Relation::NextTo,
                    target: Target::Object(4),
                    distance_cm: Some((150, 250)),
                },
            ],
        }
    }

    #[test]
    fn room_fixture_is_sp1_clean() {
        let report = super::super::run(&room());
        assert!(
            report.findings.is_empty(),
            "fixture must be clean, got {:?}",
            report.findings
        );
    }

    #[test]
    fn yaw_steps_match_octant_dirs() {
        let dirs = [
            (1, 0),
            (1, 1),
            (0, 1),
            (-1, 1),
            (-1, 0),
            (-1, -1),
            (0, -1),
            (1, -1),
        ];
        for (idx, want) in dirs.iter().enumerate() {
            assert_eq!(octant(yaw_of(idx as i32)), *want, "yaw step {idx}");
        }
    }

    #[test]
    fn board_example_solves_and_passes_sp1() {
        let actors = room();
        let intent = board_intent();
        let outs = solve(&intent, &actors).expect("board intent must solve");
        assert_eq!(outs.len(), 1);
        let out = &outs[0];
        assert_eq!(out.id, 5);
        // Against the north wall: edge gap within (0, 60) cm of min-z bound.
        let min_z = to_cm(out.z) - 30;
        let gap_north = min_z - (-510);
        assert!(
            (0..=60).contains(&gap_north),
            "north gap {gap_north} cm out of range"
        );
        // About 2 m edge gap from the gate box (x 250..350, z -475..-425).
        let (px0, px1) = (to_cm(out.x) - 30, to_cm(out.x) + 30);
        let (pz0, pz1) = (to_cm(out.z) - 30, to_cm(out.z) + 30);
        let ex = (250 - px1).max(px0 - 350).max(0);
        let ez = (-475 - pz1).max(pz0 - (-425)).max(0);
        let gap2 = i64::from(ex) * i64::from(ex) + i64::from(ez) * i64::from(ez);
        assert!(
            (150 * 150..=250 * 250).contains(&gap2),
            "gate gap^2 {gap2} out of range"
        );
        // Grounded and solved placements run clean.
        assert_eq!(out.y, 0.5);
        let report = super::super::run(&placed_scene(&actors, out));
        assert!(
            report.findings.is_empty(),
            "solved scene must run clean, got {:?}",
            report.findings
        );
    }

    #[test]
    fn same_intent_twice_identical() {
        let actors = room();
        let intent = board_intent();
        let a = solve(&intent, &actors).expect("first solve");
        let b = solve(&intent, &actors).expect("second solve");
        assert_eq!(a, b);
        // Input order never moves the answer either.
        let mut rev = actors.clone();
        rev.reverse();
        let c = solve(&intent, &rev).expect("shuffled solve");
        assert_eq!(a, c);
    }

    #[test]
    fn two_walls_tiny_gap_returns_minimal_conflict() {
        let actors = room();
        let against = |wall| Want {
            relation: Relation::NextTo,
            target: Target::Wall(wall),
            distance_cm: Some((0, 50)),
        };
        let intent = Intent {
            object: ObjectRef::Id(5),
            wants: vec![against(Wall::North), against(Wall::South)],
        };
        let err = solve(&intent, &actors).expect_err("both walls must fail");
        assert_eq!(err.unmet.len(), 2, "conflict keeps both wants");
        assert_eq!(err.unmet, intent.wants, "conflict follows intent order");
        assert!(err.reason.contains("minimal conflict"));
        // Minimal: each want alone solves.
        for want in &intent.wants {
            let single = Intent {
                object: ObjectRef::Id(5),
                wants: vec![*want],
            };
            let outs = solve(&single, &actors).expect("single wall must solve");
            let report = super::super::run(&placed_scene(&actors, &outs[0]));
            assert!(report.findings.is_empty());
        }
    }

    #[test]
    fn point_and_facing_solve_clean() {
        let actors = room();
        let intent = Intent {
            object: ObjectRef::Id(5),
            wants: vec![
                Want {
                    relation: Relation::NextTo,
                    target: Target::Point {
                        x_cm: 100,
                        z_cm: -400,
                    },
                    distance_cm: Some((0, 50)),
                },
                Want {
                    relation: Relation::Facing,
                    target: Target::Object(4),
                    distance_cm: None,
                },
            ],
        };
        let outs = solve(&intent, &actors).expect("point+facing must solve");
        let out = &outs[0];
        let dx = i64::from(to_cm(out.x)) - 100;
        let dz = i64::from(to_cm(out.z)) + 400;
        assert!(dx * dx + dz * dz <= 50 * 50, "must sit near the point");
        let report = super::super::run(&placed_scene(&actors, out));
        assert!(
            report.findings.is_empty(),
            "solved scene must run clean, got {:?}",
            report.findings
        );
    }

    #[test]
    fn unknown_object_returns_full_conflict() {
        let actors = room();
        let intent = Intent {
            object: ObjectRef::Name("missing_prop".to_string()),
            wants: vec![Want {
                relation: Relation::NextTo,
                target: Target::Wall(Wall::North),
                distance_cm: None,
            }],
        };
        let err = solve(&intent, &actors).expect_err("unknown object must fail");
        assert_eq!(err.unmet, intent.wants);
        assert!(err.reason.contains("unknown-object"));
    }

    #[test]
    fn ambiguous_name_returns_full_conflict() {
        let mut actors = room();
        actors.push(mk(6, "brazier", -2.0, 0.5, 2.0, 0.6, 1.0, 0.6));
        let intent = Intent {
            object: ObjectRef::Name("brazier".to_string()),
            wants: vec![],
        };
        let err = solve(&intent, &actors).expect_err("dupe name must fail");
        assert!(err.reason.contains("ambiguous-object"));
    }

    #[test]
    fn place_error_names_the_fault_with_source() {
        use std::error::Error;
        let missing = PlaceError::UnknownObject("id 9".to_string());
        assert_eq!(missing.to_string(), "unknown-object id 9");
        let dupe = PlaceError::AmbiguousObject {
            label: "name 'brazier'".to_string(),
            matches: 2,
        };
        assert_eq!(
            dupe.to_string(),
            "ambiguous-object name 'brazier': 2 matches"
        );
        let size = PlaceError::UnknownSize(5);
        assert_eq!(size.to_string(), "unknown-size id 5");
        assert!(missing.source().is_none());
    }

    #[test]
    fn invalid_distance_never_satisfies() {
        let actors = room();
        let intent = Intent {
            object: ObjectRef::Id(5),
            wants: vec![Want {
                relation: Relation::NextTo,
                target: Target::Wall(Wall::North),
                distance_cm: Some((60, 10)),
            }],
        };
        let err = solve(&intent, &actors).expect_err("inverted range must fail");
        assert_eq!(err.unmet.len(), 1);
    }

    #[test]
    fn unknown_target_object_is_a_conflict() {
        let actors = room();
        let intent = Intent {
            object: ObjectRef::Id(5),
            wants: vec![Want {
                relation: Relation::NextTo,
                target: Target::Object(999),
                distance_cm: None,
            }],
        };
        let err = solve(&intent, &actors).expect_err("missing target must fail");
        assert_eq!(err.unmet.len(), 1);
    }

    #[test]
    fn solved_placement_reads_clear_on_index3() {
        // Index3 consumer contract from the solver side: every
        // solution must read clear on the scene-minus-object index through
        // the shared `probe_of` conversion, and stay byte-stable twice.
        use super::super::index3::{probe_actor, Index3};
        let actors = room();
        let intent = board_intent();
        let outs = solve(&intent, &actors).expect("board intent must solve");
        assert_eq!(outs.len(), 1);
        let rest: Vec<Actor> = actors
            .iter()
            .filter(|a| a.id != outs[0].id)
            .cloned()
            .collect();
        let index = Index3::build(&rest);
        let moved = mk(
            outs[0].id, "brazier", outs[0].x, outs[0].y, outs[0].z, 0.6, 1.0, 0.6,
        );
        let probe = probe_actor(&moved).expect("valid probe");
        assert!(!index.overlaps_any(&probe, outs[0].id));
        // A crowded room keeps the same contract: the winner still verifies.
        let mut dense = room();
        for i in 0..120 {
            let x = -4.0 + (i % 12) as f32 * 0.7;
            let z = -3.0 + (i / 12) as f32 * 0.7;
            dense.push(mk(
                100 + i,
                &format!("prop_{i:03}"),
                x,
                0.5,
                z,
                0.4,
                1.0,
                0.4,
            ));
        }
        let dense_intent = Intent {
            object: ObjectRef::Id(5),
            wants: vec![Want {
                relation: Relation::NextTo,
                target: Target::Wall(Wall::South),
                distance_cm: Some((0, 50)),
            }],
        };
        let dense_out = solve(&dense_intent, &dense).expect("dense room must solve");
        let dense_rest: Vec<Actor> = dense
            .iter()
            .filter(|a| a.id != dense_out[0].id)
            .cloned()
            .collect();
        let dense_index = Index3::build(&dense_rest);
        let dense_moved = mk(
            dense_out[0].id,
            "brazier",
            dense_out[0].x,
            dense_out[0].y,
            dense_out[0].z,
            0.6,
            1.0,
            0.6,
        );
        let dense_probe = probe_actor(&dense_moved).expect("valid probe");
        assert!(!dense_index.overlaps_any(&dense_probe, dense_out[0].id));
        assert_eq!(
            dense_out,
            solve(&dense_intent, &dense).expect("dense solve repeats")
        );
        let report = super::super::run(&placed_scene(&dense, &dense_out[0]));
        assert!(
            !report.findings.iter().any(|f| (f.check == "overlap"
                && (f.a == dense_out[0].id || f.b == dense_out[0].id))
                || (f.check == "floating" && f.a == dense_out[0].id)),
            "dense solution must verify clean, got {:?}",
            report.findings
        );
    }

    #[test]
    fn empty_wants_take_first_clean_cell() {
        let actors = room();
        let intent = Intent {
            object: ObjectRef::Id(5),
            wants: vec![],
        };
        let a = solve(&intent, &actors).expect("empty wants must solve");
        let b = solve(&intent, &actors).expect("empty wants must solve twice");
        assert_eq!(a, b);
        let report = super::super::run(&placed_scene(&actors, &a[0]));
        assert!(report.findings.is_empty());
    }

    /// One seeded over-constrained intent per index: seeds 0..10 pin the
    /// brazier against both the north and south walls (a 10 m room never
    /// satisfies both), seeds 10..20 ask for an edge gap to the gate that is
    /// both near and far (disjoint ranges never satisfy together). Every
    /// seed is a joint failure whose single-drop remainder solves.
    fn seeded_conflict(seed: usize) -> Intent {
        if seed < 10 {
            let hi_n = 30 + (seed as i32 % 3) * 10;
            let hi_s = 30 + ((seed as i32 + 1) % 3) * 10;
            Intent {
                object: ObjectRef::Id(5),
                wants: vec![
                    Want {
                        relation: Relation::NextTo,
                        target: Target::Wall(Wall::North),
                        distance_cm: Some((0, hi_n)),
                    },
                    Want {
                        relation: Relation::NextTo,
                        target: Target::Wall(Wall::South),
                        distance_cm: Some((0, hi_s)),
                    },
                ],
            }
        } else {
            let s = seed - 10;
            let near_hi = 40 + (s as i32 % 3) * 10;
            let far_lo = 300 + (s as i32 % 2) * 20;
            Intent {
                object: ObjectRef::Id(5),
                wants: vec![
                    Want {
                        relation: Relation::NextTo,
                        target: Target::Object(4),
                        distance_cm: Some((0, near_hi)),
                    },
                    Want {
                        relation: Relation::NextTo,
                        target: Target::Object(4),
                        distance_cm: Some((far_lo, far_lo + 100)),
                    },
                ],
            }
        }
    }

    #[test]
    fn hints_repair_twenty_seeded_conflicts() {
        for seed in 0..20 {
            let actors = room();
            let intent = seeded_conflict(seed);
            let err = solve(&intent, &actors).expect_err("seeded intent must conflict");
            assert_eq!(err.unmet.len(), 2, "seed {seed}: joint failure keeps both");
            assert!(err.reason.contains("minimal conflict"), "seed {seed}");
            assert_eq!(err.hints.len(), 1, "seed {seed}: exactly one repair hint");
            let hint = &err.hints[0];
            // First solvable drop in intent order: both singles solve, so the
            // hint always relaxes want 0.
            assert_eq!(hint.dropped, 0, "seed {seed}");
            assert_eq!(hint.dropped_want, intent.wants[0], "seed {seed}");
            assert!(!hint.note.contains('\n'), "seed {seed}: one-line note");
            assert!(
                hint.note.starts_with("drop NextTo "),
                "seed {seed}: stable code, got {}",
                hint.note
            );
            // Same inputs twice give the identical conflict plus hint.
            let again = solve(&intent, &actors).expect_err("repeat must conflict");
            assert_eq!(err, again, "seed {seed}: conflicts byte-stable");
            // The hinted remainder re-solves to exactly the hinted placement.
            let mut rest = intent.wants.clone();
            rest.remove(hint.dropped);
            let single = Intent {
                object: ObjectRef::Id(5),
                wants: rest,
            };
            let outs = solve(&single, &actors).expect("hint remainder must solve");
            assert_eq!(outs.len(), 1, "seed {seed}");
            assert_eq!(outs[0], hint.placement, "seed {seed}");
            // The hinted move is the integer-cm delta from the live center.
            assert_eq!(
                hint.move_cm,
                [
                    to_cm(hint.placement.x) - to_cm(0.0),
                    to_cm(hint.placement.z) - to_cm(2.0)
                ],
                "seed {seed}: brazier starts at (0.0, 2.0)"
            );
            // Applying the hint verifies clean on the full offline catalog.
            let placed = placed_scene(&actors, &hint.placement);
            let report = super::super::run(&placed);
            assert!(
                report.findings.is_empty(),
                "seed {seed}: hinted scene must run clean, got {:?}",
                report.findings
            );
            // The hint-applied dry-run diff equals the applied diff.
            let before = super::super::relation::build(&actors);
            let dry = super::super::diff::dry_run_diff(&before, &placed);
            let applied =
                super::super::diff::diff_graph(&before, &super::super::relation::build(&placed));
            assert_eq!(dry, applied, "seed {seed}: dry-run == applied");
        }
    }

    /// Baseline tape: 60 seeded over-constrained wall intents over the
    /// frozen room, read-only on the solver. Seeds 0..23 pin the brazier
    /// against the north and south walls (a 10 m room never satisfies both,
    /// so the full pair is already minimal); seeds 23..60 add one
    /// satisfiable east/west near want the shrink must drop. Every seed runs
    /// through the real [`solve`] entry point the `scene.place` door and the
    /// playtest gold harness consume.
    fn audit_seed(seed: usize) -> Intent {
        let gap = |wall: Wall, hi: i32| Want {
            relation: Relation::NextTo,
            target: Target::Wall(wall),
            distance_cm: Some((0, hi)),
        };
        if seed < 23 {
            let hi_n = 20 + (seed as i32 % 4) * 10;
            let hi_s = 20 + ((seed as i32 + 2) % 4) * 10;
            Intent {
                object: ObjectRef::Id(5),
                wants: vec![gap(Wall::North, hi_n), gap(Wall::South, hi_s)],
            }
        } else {
            let s = seed - 23;
            let hi_n = 20 + (s as i32 % 4) * 10;
            let hi_s = 20 + ((s as i32 + 2) % 4) * 10;
            let side = if s.is_multiple_of(2) {
                Wall::East
            } else {
                Wall::West
            };
            Intent {
                object: ObjectRef::Id(5),
                wants: vec![
                    gap(Wall::North, hi_n),
                    gap(Wall::South, hi_s),
                    gap(side, 50),
                ],
            }
        }
    }

    /// Subset-minimality oracle for one tape seed: the shrunk conflict stays
    /// unsolvable, every single-drop remainder solves, the conflict follows
    /// intent order, and the hinted repair re-solves exactly. Returns true
    /// when the full want set was already minimal (no shrink needed).
    fn assert_audit_minimal(intent: &Intent, actors: &[Actor], seed: usize) -> bool {
        let err = solve(intent, actors).expect_err("tape seed must conflict");
        assert!(
            err.reason.contains("minimal conflict"),
            "seed {seed}: {}",
            err.reason
        );
        assert!(!err.unmet.is_empty(), "seed {seed}: conflict never empty");
        // Conflict follows intent order over a subset of the intent wants.
        let mut idx = Vec::new();
        for want in &err.unmet {
            match intent.wants.iter().position(|w| w == want) {
                Some(p) => idx.push(p),
                None => panic!("seed {seed}: conflict want not in intent"),
            }
        }
        let mut ordered = idx.clone();
        ordered.sort_unstable();
        ordered.dedup();
        assert_eq!(idx, ordered, "seed {seed}: conflict follows intent order");
        // Irreducible: the conflict itself fails, each single-drop solves.
        let conflict = Intent {
            object: intent.object.clone(),
            wants: err.unmet.clone(),
        };
        assert!(
            solve(&conflict, actors).is_err(),
            "seed {seed}: conflict must stay unsolvable"
        );
        for drop in 0..err.unmet.len() {
            let mut rest = err.unmet.clone();
            rest.remove(drop);
            let trial = Intent {
                object: intent.object.clone(),
                wants: rest,
            };
            assert!(
                solve(&trial, actors).is_ok(),
                "seed {seed}: drop {drop} must solve"
            );
        }
        // Hinted repair re-solves to exactly the hinted placement.
        assert_eq!(err.hints.len(), 1, "seed {seed}: exactly one repair hint");
        let hint = &err.hints[0];
        let mut repaired = intent.wants.clone();
        repaired.remove(hint.dropped);
        let outs = solve(
            &Intent {
                object: intent.object.clone(),
                wants: repaired,
            },
            actors,
        )
        .expect("seed: hint remainder must solve");
        assert_eq!(outs.len(), 1, "seed {seed}");
        assert_eq!(outs[0], hint.placement, "seed {seed}");
        // Same inputs twice give the identical conflict set (digest x2).
        let again = solve(intent, actors).expect_err("repeat must conflict");
        assert_eq!(err, again, "seed {seed}: conflicts byte-stable");
        err.unmet.len() == intent.wants.len()
    }

    #[test]
    fn conflict_audit_baseline() {
        // 60 seeded wall intents: 23 already minimal, 37 shrink 3 -> 2.
        let mut already_minimal = 0;
        for seed in 0..60 {
            let actors = room();
            let intent = audit_seed(seed);
            if assert_audit_minimal(&intent, &actors, seed) {
                already_minimal += 1;
            }
        }
        assert_eq!(
            already_minimal, 23,
            "tape shape: 23 two-wall seeds need no shrink, 37 three-wall seeds shrink 3 -> 2"
        );
        // Permutation stability over the first 20 seeds: reversed actor input
        // gives the identical conflict set.
        for seed in 0..20 {
            let actors = room();
            let intent = audit_seed(seed);
            let fwd = solve(&intent, &actors).expect_err("seed must conflict");
            let mut rev = actors.clone();
            rev.reverse();
            let bwd = solve(&intent, &rev).expect_err("reversed must conflict");
            assert_eq!(fwd, bwd, "seed {seed}: permutation-identical");
        }
    }

    #[test]
    fn sp38_audit_hunks_cover_sixty_seeds_and_twenty_perms() {
        // Production audit beside SP17 hints: 60/60 subset-minimal shape plus
        // 20/20 permutation-identical conflicts, integer-only and read-only.
        for seed in 0..60 {
            let actors = room();
            let intent = audit_seed(seed);
            let err = solve(&intent, &actors).expect_err("tape seed must conflict");
            assert_eq!(
                super::audit_subset(&intent, &err),
                Some(super::ConflictAudit::Minimal),
                "seed {seed}: subset-minimal"
            );
            let again = solve(&intent, &actors).expect_err("repeat must conflict");
            assert!(
                super::audit_identical(&err, &again),
                "seed {seed}: byte-stable"
            );
        }
        for seed in 0..20 {
            let actors = room();
            let intent = audit_seed(seed);
            let fwd = solve(&intent, &actors).expect_err("seed must conflict");
            let mut rev = actors.clone();
            rev.reverse();
            let bwd = solve(&intent, &rev).expect_err("reversed must conflict");
            assert!(
                super::audit_identical(&fwd, &bwd),
                "seed {seed}: permutation-identical"
            );
        }
        // Negative: empty conflict never audits.
        let actors = room();
        let intent = audit_seed(0);
        let mut bad = solve(&intent, &actors).expect_err("seed must conflict");
        bad.unmet.clear();
        assert_eq!(super::audit_subset(&intent, &bad), None);
        // Negative: swapped order on a three-wall seed never audits.
        let probe = audit_seed(30);
        let perr = solve(&probe, &actors).expect_err("seed 30 must conflict");
        let mut bad_order = perr.clone();
        bad_order.unmet.swap(0, 1);
        assert_eq!(super::audit_subset(&probe, &bad_order), None);
        // Identity shape audits honest without geometry.
        let missing = Intent {
            object: ObjectRef::Name("missing_prop".to_string()),
            wants: vec![Want {
                relation: Relation::NextTo,
                target: Target::Wall(Wall::North),
                distance_cm: None,
            }],
        };
        let lost = solve(&missing, &actors).expect_err("unknown object must fail");
        assert_eq!(
            super::audit_subset(&missing, &lost),
            Some(super::ConflictAudit::HonestFull)
        );
        assert!(super::audit_identical(&lost, &lost));
    }

    #[test]
    fn conflict_audit_baseline_failures() {
        // Invalid input never satisfies, yet the audit still classifies it
        // honestly: one inverted range is its own minimal conflict, and the
        // empty remainder solves (first clean cell).
        let actors = room();
        let bad = Intent {
            object: ObjectRef::Id(5),
            wants: vec![Want {
                relation: Relation::NextTo,
                target: Target::Wall(Wall::North),
                distance_cm: Some((60, 10)),
            }],
        };
        let err = solve(&bad, &actors).expect_err("inverted range must fail");
        assert_eq!(err.unmet, bad.wants);
        let empty = Intent {
            object: bad.object.clone(),
            wants: Vec::new(),
        };
        assert!(solve(&empty, &actors).is_ok(), "empty remainder solves");
        let again = solve(&bad, &actors).expect_err("repeat must fail");
        assert_eq!(err, again);
        // Identity failures are not geometric: the full want set carries over
        // with honestly empty hints, and the audit skips minimality there.
        let missing = Intent {
            object: ObjectRef::Name("missing_prop".to_string()),
            wants: vec![Want {
                relation: Relation::NextTo,
                target: Target::Wall(Wall::North),
                distance_cm: None,
            }],
        };
        let lost = solve(&missing, &actors).expect_err("unknown object must fail");
        assert_eq!(lost.unmet, missing.wants);
        assert!(lost.hints.is_empty());
        assert!(lost.reason.contains("unknown-object"));
    }

    #[test]
    fn unknown_object_carries_no_hint() {
        // Identity failures have no geometry to repair: the caller fixes the
        // reference first, so hints stay honestly empty.
        let actors = room();
        let intent = Intent {
            object: ObjectRef::Name("missing_prop".to_string()),
            wants: vec![Want {
                relation: Relation::NextTo,
                target: Target::Wall(Wall::North),
                distance_cm: None,
            }],
        };
        let err = solve(&intent, &actors).expect_err("unknown object must fail");
        assert_eq!(err.unmet, intent.wants);
        assert!(err.hints.is_empty());
    }

    #[test]
    fn support_gap_touching_is_zero() {
        let gap = super::support_gap_cm(20, 20);
        assert_eq!(gap, 0);
        let limit = i64::from(super::super::relation::SUPPORT_GAP_CM);
        assert!(0 <= i64::from(gap) && i64::from(gap) <= limit);
    }

    #[test]
    fn support_gap_rest_at_limit() {
        let limit = super::super::relation::SUPPORT_GAP_CM;
        let gap = super::support_gap_cm(20, 20 + limit);
        assert_eq!(gap, limit);
        assert!(0 <= i64::from(gap) && i64::from(gap) <= i64::from(limit));
    }

    #[test]
    fn support_gap_past_limit_floats() {
        let limit = super::super::relation::SUPPORT_GAP_CM;
        let gap = super::support_gap_cm(20, 20 + limit + 1);
        assert_eq!(gap, limit + 1);
        assert!(i64::from(gap) > i64::from(limit));
    }

    #[test]
    fn support_gap_negative_is_overlap() {
        let gap = super::support_gap_cm(20, 20 - 1);
        assert_eq!(gap, -1);
        assert!(i64::from(gap) < 0);
    }

    #[test]
    fn solved_placement_on_top_rests_on_the_shelf() {
        let mut actors = room();
        actors.push(mk(6, "shelf", 0.0, 0.5, 0.0, 2.0, 1.0, 1.0));
        actors.push(mk(7, "crate", -2.0, 0.2, 2.0, 0.4, 0.4, 0.4));
        let intent = Intent {
            object: ObjectRef::Id(7),
            wants: vec![Want {
                relation: Relation::OnTopOf,
                target: Target::Object(6),
                distance_cm: None,
            }],
        };
        let outs = solve(&intent, &actors).expect("on-top must rest on the shelf");
        assert_eq!(outs.len(), 1);
        let out = &outs[0];
        assert_eq!(out.id, 7);
        assert!(
            (out.y - 1.2).abs() < 0.01,
            "rest y {} must be shelf top 1.0 plus half 0.2",
            out.y
        );
        // XZ must overlap the shelf footprint (x -100..100, z -50..50).
        let hx = to_cm(0.4).abs() / 2;
        let hz = to_cm(0.4).abs() / 2;
        let (min_x, max_x) = (to_cm(out.x) - hx, to_cm(out.x) + hx);
        let (min_z, max_z) = (to_cm(out.z) - hz, to_cm(out.z) + hz);
        assert!(
            min_x < 100 && -100 < max_x && min_z < 50 && -50 < max_z,
            "crate XZ [{min_x},{max_x}] x [{min_z},{max_z}] must overlap the shelf"
        );
        // The raised probe must read clear on the scene-minus-object index.
        use super::super::index3::{probe_actor, Index3};
        let rest: Vec<Actor> = actors.iter().filter(|a| a.id != out.id).cloned().collect();
        let index = Index3::build(&rest);
        let moved = mk(out.id, "crate", out.x, out.y, out.z, 0.4, 0.4, 0.4);
        let probe = probe_actor(&moved).expect("valid probe");
        assert!(!index.overlaps_any(&probe, out.id));
        // Self and unknown supporters stay conflicts, never guesses.
        let self_intent = Intent {
            object: ObjectRef::Id(7),
            wants: vec![Want {
                relation: Relation::OnTopOf,
                target: Target::Object(7),
                distance_cm: None,
            }],
        };
        assert!(solve(&self_intent, &actors).is_err());
        let unknown_intent = Intent {
            object: ObjectRef::Id(7),
            wants: vec![Want {
                relation: Relation::OnTopOf,
                target: Target::Object(99),
                distance_cm: None,
            }],
        };
        assert!(solve(&unknown_intent, &actors).is_err());
    }

    /// SP16 support gate for one stacked solve: rest height, 3D-index
    /// clearance, 0 support findings for the placed id, both relation
    /// edges, bytes x2 and permutation stability.
    fn check_supported(actors: &[Actor], supporter: u32, prop: u32, top_y: f32) {
        use super::super::index3::{probe_actor, Index3};
        let intent = Intent {
            object: ObjectRef::Id(prop),
            wants: vec![Want {
                relation: Relation::OnTopOf,
                target: Target::Object(supporter),
                distance_cm: None,
            }],
        };
        let outs = solve(&intent, actors).expect("support must solve");
        assert_eq!(outs.len(), 1);
        let out = &outs[0];
        assert_eq!(out.id, prop);
        assert!(
            (out.y - top_y).abs() < 0.01,
            "supporter {supporter} rest y {} != {top_y}",
            out.y
        );
        let rest: Vec<Actor> = actors.iter().filter(|a| a.id != out.id).cloned().collect();
        let index = Index3::build(&rest);
        let moved = mk(prop, "prop", out.x, out.y, out.z, 0.4, 0.4, 0.4);
        let probe = probe_actor(&moved).expect("valid probe");
        assert!(!index.overlaps_any(&probe, out.id));
        let placed = placed_scene(actors, out);
        let report = super::super::run(&placed);
        assert!(
            !report
                .findings
                .iter()
                .any(|f| f.check == "support" && f.a == out.id),
            "supporter {supporter} findings: {:?}",
            report.findings
        );
        let graph = super::super::relation::build(&placed);
        assert!(graph.edges.contains(&(supporter, Relation::Supports, prop)));
        assert!(graph.edges.contains(&(prop, Relation::OnTopOf, supporter)));
        assert_eq!(outs, solve(&intent, actors).expect("bytes x2"));
        let mut rev = actors.to_vec();
        rev.reverse();
        assert_eq!(outs, solve(&intent, &rev).expect("perm stable"));
    }

    #[test]
    fn ludus_siege_wall_place_on_shelves_and_stairs() {
        // SP16: two-floor ludus shelf/stair plus siege-wall top place with
        // 0 support findings for the placed id; flat goldens untouched.
        // XZ-only overlap still fires for stacked contact by design, so the
        // gate here is support-only plus 3D-index clearance and the
        // Supports/OnTopOf edges. Flat goldens stay green in
        // board_example_solves_and_passes_sp1 (out.y 0.5, run clean).
        let ludus = vec![
            mk(0, "slab_ground", 0.0, 0.1, 0.0, 10.0, 0.2, 10.0),
            mk(1, "slab_first", 0.0, 3.1, -2.0, 6.0, 0.2, 6.0),
            mk(2, "step_0", 0.0, 0.5, 3.5, 1.0, 1.0, 1.0),
            mk(3, "step_1", 0.0, 1.5, 2.5, 1.0, 1.0, 1.0),
            mk(4, "step_2", 0.0, 2.5, 1.5, 1.0, 1.0, 1.0),
            mk(5, "balcony", 3.0, 2.6, 3.0, 2.0, 0.2, 2.0),
            mk(6, "walker", 3.0, 1.1, 3.0, 0.6, 1.8, 0.6),
            mk(7, "guard", -1.0, 4.1, -2.0, 0.6, 1.8, 0.6),
            mk(8, "crate", -3.0, 0.7, 3.0, 1.0, 1.0, 1.0),
            mk(9, "prop", -2.0, 0.2, 2.0, 0.4, 0.4, 0.4),
        ];
        check_supported(&ludus, 1, 9, 3.4);
        check_supported(&ludus, 3, 9, 2.2);
        let siege = vec![
            mk(10, "wall_a", -6.0, 3.0, 0.0, 4.0, 6.0, 1.0),
            mk(11, "wall_b", -2.0, 3.0, 0.0, 4.0, 6.0, 1.0),
            mk(12, "wall_c", 2.0, 3.0, 0.0, 4.0, 6.0, 1.0),
            mk(13, "wall_d", 6.0, 3.0, 0.0, 4.0, 6.0, 1.0),
            mk(14, "gate_pier", 0.0, 2.0, 0.0, 1.0, 4.0, 1.2),
            mk(15, "ladder", -2.0, 1.5, 1.2, 0.6, 3.0, 0.4),
            mk(16, "tent", -2.0, 1.0, 12.0, 3.0, 2.0, 3.0),
            mk(17, "prop", 5.0, 0.2, 5.0, 0.4, 0.4, 0.4),
        ];
        check_supported(&siege, 11, 17, 6.2);
    }

    #[test]
    fn conflict_set_keeps_eq() {
        // Compile-time proof for the frozen shape: hints ride along but
        // the float payload stays out of the `Eq` path.
        fn is_eq<T: Eq>() {}
        is_eq::<ConflictSet>();
        is_eq::<RepairHint>();
    }

    #[test]
    fn enumeration_lists_floor_shelf_tread_with_clearance() {
        let shelf = mk(6, "shelf", 0.0, 0.5, 0.0, 2.0, 1.0, 1.0);
        let tread = mk(7, "tread", 3.0, 0.3, 0.0, 1.0, 1.0, 1.0);
        let prop = mk(9, "prop", -3.0, 0.2, 0.0, 0.4, 0.4, 0.4);
        let actors = vec![shelf.clone(), tread.clone(), prop.clone()];
        let rest: Vec<Actor> = actors.iter().filter(|a| a.id != 9).cloned().collect();
        let index = super::super::index3::Index3::build(&rest);
        let surfaces = super::enumerate_support_surfaces(&actors, 9, &index);
        assert!(surfaces.len() >= 3, "got {surfaces:?}");
        let floor = surfaces
            .iter()
            .find(|s| s.kind == super::SupportKind::Floor)
            .expect("floor");
        assert_eq!(floor.top_cm, 0);
        assert!(floor.clear && floor.rests, "got {floor:?}");
        let shelf_s = surfaces
            .iter()
            .find(|s| s.supporter == Some(6))
            .expect("shelf");
        assert_eq!(
            (shelf_s.kind, shelf_s.top_cm),
            (super::SupportKind::ShelfTop, 100)
        );
        assert!(shelf_s.clear && !shelf_s.rests, "got {shelf_s:?}");
        let tread_s = surfaces
            .iter()
            .find(|s| s.supporter == Some(7))
            .expect("tread");
        assert_eq!(
            (tread_s.kind, tread_s.top_cm),
            (super::SupportKind::Tread, 80)
        );
        assert!(tread_s.clear && !tread_s.rests, "got {tread_s:?}");
        let mut stacked = prop.clone();
        stacked.y = 1.2;
        stacked.x = 0.0;
        let placed = vec![shelf, tread, stacked];
        let report = super::super::run(&placed);
        assert!(!report
            .findings
            .iter()
            .any(|f| f.check == "support" && f.a == 9));
        let rest2: Vec<Actor> = placed.iter().filter(|a| a.id != 9).cloned().collect();
        let idx2 = super::super::index3::Index3::build(&rest2);
        let surfaces2 = super::enumerate_support_surfaces(&placed, 9, &idx2);
        let shelf2 = surfaces2
            .iter()
            .find(|s| s.supporter == Some(6))
            .expect("shelf2");
        assert!(shelf2.rests && shelf2.clear, "got {shelf2:?}");
        assert_eq!(
            surfaces2,
            super::enumerate_support_surfaces(&placed, 9, &idx2)
        );
    }

    #[test]
    fn canonical_audit_digest_repeats_x2() {
        let mut fwd_all = Vec::new();
        let mut rev_all = Vec::new();
        for seed in 0..60 {
            let actors = room();
            let intent = audit_seed(seed);
            let err = solve(&intent, &actors).expect_err("tape must conflict");
            let got = super::audit_subset(&intent, &err);
            assert_eq!(got, Some(super::ConflictAudit::Minimal), "seed {seed}");
            fwd_all.push(err);
            let mut rev = actors.clone();
            rev.reverse();
            let perr = solve(&intent, &rev).expect_err("perm must conflict");
            rev_all.push(perr);
            if seed < 20 {
                let a = &fwd_all[seed];
                let b = &rev_all[seed];
                assert!(super::audit_identical(a, b), "seed {seed}");
            }
        }
        let d0 = super::canonical_audit_digest(&fwd_all);
        assert_eq!(d0, super::canonical_audit_digest(&fwd_all), "digest x2");
        assert_ne!(d0, 0);
        assert_eq!(d0, super::canonical_audit_digest(&rev_all), "perm digest");
    }
}
