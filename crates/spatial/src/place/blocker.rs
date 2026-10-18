//! SPR6: say why a placement failed.
//!
//! Consumer: the `scene.place` door prints [`ConflictSet::blocks`] beside the
//! repair hints, so a weak model reads which object (or wall) stands in the way,
//! by how many centimeters, and the nearest pose that misses. The caller owns
//! the actor snapshot; this module only reads the solver working set.
//! Units: integer centimeters. Determinism: candidates scan z, x, yaw ascending
//! and ties keep the first, overlapped objects are visited by ascending id, so
//! the same intent over the same actors in any order gives identical blocks.
//! Budget: one grid pass per unmet want plus one for the occupied-spot pass,
//! on the failure path only, never the tick.
//!
//! Two shapes of failure, told apart by one question: is there any pose that
//! meets every unmet want, ignoring what stands there?
//!
//! - Yes, and every such pose overlaps something: the wants point at a taken
//!   spot. Each unmet want names the dominant object at the cheapest taken pose
//!   ([`Blocker::Object`], with its overlap and the shortest push that clears
//!   it, both in plan view) and `miss_cm` says how far the nearest clean pose
//!   that keeps the other wants misses this one.
//! - No: the wants contradict each other. Each unmet want names its own target
//!   (wall, object or point) and the cm the nearest clean pose misses it by.
//!
//! The minimal subset, the repair hints and the audit digest are computed
//! before this runs and are never read back from it.

use super::*;
use crate::intmath::isqrt;

/// Miss reported for a want that no pose can ever meet (missing target, the
/// object itself): one cm, the smallest honest figure.
const NEVER_CM: i64 = 1;

/// Shortest axis push that clears one overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Escape {
    /// Door word of the direction: `-x`, `+x`, `-z`, `+z`, `up` or `down`.
    pub dir: &'static str,
    /// Centimeters to move along `dir` until the boxes only touch.
    pub cm: i32,
}

/// Shortest push that moves `mover` out of `fixed` along one axis: the four
/// plan directions, plus `up` and `down` when `vertical`. Ties keep the first
/// word in the order `-x +x -z +z up down`. Boxes that do not overlap read 0.
pub fn escape(mover: &Box3, fixed: &Box3, vertical: bool) -> Escape {
    let candidates = [
        ("-x", mover.max_x - fixed.min_x),
        ("+x", fixed.max_x - mover.min_x),
        ("-z", mover.max_z - fixed.min_z),
        ("+z", fixed.max_z - mover.min_z),
        ("up", fixed.max_y - mover.min_y),
        ("down", mover.max_y - fixed.min_y),
    ];
    let take = if vertical { 6 } else { 4 };
    let (dir, cm) = candidates
        .iter()
        .take(take)
        .copied()
        .min_by_key(|(_, cm)| *cm)
        .unwrap_or(("+x", 0));
    Escape { dir, cm: cm.max(0) }
}

/// How deep two boxes overlap: the shallowest intersection extent over the
/// plan axes (and the vertical axis when `vertical`). Apart reads 0.
pub fn overlap_depth_cm(a: &Box3, b: &Box3, vertical: bool) -> i32 {
    let ex = a.max_x.min(b.max_x) - a.min_x.max(b.min_x);
    let ez = a.max_z.min(b.max_z) - a.min_z.max(b.min_z);
    let mut depth = ex.min(ez);
    if vertical {
        depth = depth.min(a.max_y.min(b.max_y) - a.min_y.max(b.min_y));
    }
    depth.max(0)
}

/// What stands in one unmet want's way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocker {
    /// The wants point at a taken spot: this object overlaps the cheapest pose
    /// that meets every unmet want by `overlap_cm`, and moving it `fix` clears it.
    Object {
        id: u32,
        overlap_cm: i32,
        fix: Escape,
    },
    /// The wants contradict: this want measures against the wall and misses it.
    Wall(Wall),
    /// The wants contradict: this want measures against the object and misses it.
    Target(u32),
    /// The wants contradict: this want measures against the point and misses it.
    Point { x_cm: i32, z_cm: i32 },
}

/// Why one unmet want of a [`ConflictSet`] could not hold.
///
/// `want` indexes [`ConflictSet::unmet`]. `miss_cm` is how far the nearest
/// clean pose that keeps every other unmet want misses this one (always at
/// least 1). `near_miss` is that pose's center, `[x_cm, z_cm]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WantBlock {
    pub want: usize,
    pub blocker: Blocker,
    pub miss_cm: i32,
    pub near_miss: [i32; 2],
}

fn clamp_cm(v: i64) -> i32 {
    i32::try_from(v).unwrap_or(i32::MAX)
}

/// Smallest `r` with `r * r >= n`.
fn ceil_sqrt(n: i64) -> i64 {
    let r = isqrt(n);
    if r * r == n {
        r
    } else {
        r + 1
    }
}

/// Distance outside an inclusive range, 0 inside.
fn range_miss(value: i64, range: (i64, i64)) -> i64 {
    if value < range.0 {
        range.0 - value
    } else if value > range.1 {
        value - range.1
    } else {
        0
    }
}

/// Squared-distance version of [`range_miss`]; mirrors `dist_in`.
fn dist_miss(d2: i64, range: (i64, i64)) -> i64 {
    if d2 < range.0 * range.0 {
        range.0 - isqrt(d2)
    } else if d2 > range.1 * range.1 {
        ceil_sqrt(d2) - range.1
    } else {
        0
    }
}

/// How invalid a range is, in cm, so a never-valid want reads a positive figure.
fn bad_range_cm(range: Option<(i32, i32)>) -> i64 {
    match range {
        Some((lo, _)) if lo < 0 => i64::from(lo).abs(),
        Some((lo, hi)) => (i64::from(lo) - i64::from(hi)).max(NEVER_CM),
        None => NEVER_CM,
    }
}

/// Facing refinement from a center distance range; `None` always holds.
fn center_miss(cx: i32, cz: i32, tx: i32, tz: i32, range: Option<(i32, i32)>) -> i64 {
    match range {
        None => 0,
        Some(_) => match valid_range(range) {
            Some(valid) => dist_miss(center2(cx, cz, tx, tz), valid),
            None => bad_range_cm(range),
        },
    }
}

/// Mirrors `face_holds`: one cm for a wrong or undefined heading, plus the cm
/// past the facing range.
fn face_miss(cx: i32, cz: i32, yaw_idx: i32, tx: i32, tz: i32) -> i64 {
    let d2 = center2(cx, cz, tx, tz);
    let mut miss = 0;
    if d2 == 0 {
        miss += 1;
    } else {
        let (fx, fz) = octant(yaw_of(yaw_idx));
        let dx = i64::from(tx) - i64::from(cx);
        let dz = i64::from(tz) - i64::from(cz);
        if i64::from(fx) * dx + i64::from(fz) * dz <= 0 {
            miss += 1;
        }
    }
    let range = i64::from(FACING_RANGE_CM);
    if d2 > range * range {
        miss += ceil_sqrt(d2) - range;
    }
    miss
}

/// Strict-overlap shortfall on one axis: 0 when `[lo, hi)` overlaps
/// `[o_lo, o_hi)`, else the cm that closes the gap plus one.
fn overlap_miss(lo: i32, hi: i32, o_lo: i32, o_hi: i32) -> i64 {
    if lo < o_hi && o_lo < hi {
        0
    } else {
        i64::from((lo - o_hi).max(o_lo - hi)) + 1
    }
}

/// Rest-gap shortfall: the support gap must sit in `0..=SUPPORT_GAP_CM`.
fn rest_miss(gap: i64) -> i64 {
    range_miss(gap, (0, i64::from(SUPPORT_GAP_CM)))
}

fn box_volume(b: &Box3) -> i64 {
    (i64::from(b.max_x) - i64::from(b.min_x)).max(0)
        * (i64::from(b.max_y) - i64::from(b.min_y)).max(0)
        * (i64::from(b.max_z) - i64::from(b.min_z)).max(0)
}

/// The probe's footprint at one candidate pose, in integer cm.
struct Foot {
    cx: i32,
    cz: i32,
    hx: i32,
    hz: i32,
    half_y: i32,
    y_cm: i32,
}

impl Foot {
    fn at(cx: i32, cz: i32, obj: &Placed) -> Self {
        Self {
            cx,
            cz,
            hx: to_cm(obj.sx).abs() / 2,
            hz: to_cm(obj.sz).abs() / 2,
            half_y: to_cm(obj.sy).abs() / 2,
            y_cm: to_cm(obj.y_m),
        }
    }

    fn x(&self) -> (i32, i32) {
        (self.cx - self.hx, self.cx + self.hx)
    }

    fn z(&self) -> (i32, i32) {
        (self.cz - self.hz, self.cz + self.hz)
    }

    fn y(&self) -> (i32, i32) {
        (self.y_cm - self.half_y, self.y_cm + self.half_y)
    }

    fn probe(&self) -> Box3 {
        probe_of(self.cx, self.y_cm, self.cz, self.hx, self.half_y, self.hz)
    }
}

/// Gap from the probe's `wall` face to the room bound, mirroring `satisfies`.
fn wall_gap(w: Wall, foot: &Foot, ctx: &Ctx<'_>) -> i64 {
    let b = &ctx.bounds;
    let (min_x, max_x) = foot.x();
    let (min_z, max_z) = foot.z();
    match w {
        Wall::North => i64::from(min_z) - i64::from(b.min_z),
        Wall::South => i64::from(b.max_z) - i64::from(max_z),
        Wall::West => i64::from(min_x) - i64::from(b.min_x),
        Wall::East => i64::from(b.max_x) - i64::from(max_x),
    }
}

/// How far the probe at `(cx, cz, yaw_idx)` misses `want`, in cm: 0 exactly when
/// `satisfies` is true. Sums the parts of a want that can fail separately.
fn want_miss(want: &Want, cx: i32, cz: i32, yaw_idx: i32, obj: &Placed, ctx: &Ctx<'_>) -> i64 {
    let foot = Foot::at(cx, cz, obj);
    match (want.relation, want.target) {
        (Relation::NextTo, Target::Wall(w)) => {
            match valid_range_or(want.distance_cm, DEFAULT_WALL_GAP_CM) {
                Some(range) => range_miss(wall_gap(w, &foot, ctx), range),
                None => bad_range_cm(want.distance_cm),
            }
        }
        (Relation::NextTo, Target::Object(id)) => {
            let Some(range) = valid_range_or(want.distance_cm, DEFAULT_OBJECT_GAP_CM) else {
                return bad_range_cm(want.distance_cm);
            };
            let Some(box3) = ctx.boxes.get(&id) else {
                return NEVER_CM;
            };
            if box3.id == obj.id {
                return NEVER_CM;
            }
            let (min_x, max_x) = foot.x();
            let (min_z, max_z) = foot.z();
            let (p_min_y, p_max_y) = foot.y();
            let level = if p_min_y < box3.max_y && box3.min_y < p_max_y {
                0
            } else {
                i64::from((p_min_y - box3.max_y).max(box3.min_y - p_max_y)) + 1
            };
            level + dist_miss(edge_gap2(min_x, max_x, min_z, max_z, box3), range)
        }
        (Relation::NextTo, Target::Point { x_cm, z_cm }) => {
            match valid_range_or(want.distance_cm, DEFAULT_POINT_DIST_CM) {
                Some(range) => dist_miss(center2(cx, cz, x_cm, z_cm), range),
                None => bad_range_cm(want.distance_cm),
            }
        }
        (Relation::Facing, Target::Wall(w)) => {
            let (fx, fz) = octant(yaw_of(yaw_idx));
            let toward = match w {
                Wall::North => fz < 0,
                Wall::South => fz > 0,
                Wall::West => fx < 0,
                Wall::East => fx > 0,
            };
            let reach = match want.distance_cm {
                None => 0,
                Some(_) => match valid_range(want.distance_cm) {
                    Some(range) => range_miss(wall_gap(w, &foot, ctx), range),
                    None => bad_range_cm(want.distance_cm),
                },
            };
            i64::from(!toward) + reach
        }
        (Relation::Facing, Target::Object(id)) => {
            let Some(target) = ctx.by_id.get(&id) else {
                return NEVER_CM;
            };
            if target.id == obj.id || !pos_known(target) {
                return NEVER_CM;
            }
            let (tx, tz) = (to_cm(target.x), to_cm(target.z));
            face_miss(cx, cz, yaw_idx, tx, tz) + center_miss(cx, cz, tx, tz, want.distance_cm)
        }
        (Relation::Facing, Target::Point { x_cm, z_cm }) => {
            face_miss(cx, cz, yaw_idx, x_cm, z_cm)
                + center_miss(cx, cz, x_cm, z_cm, want.distance_cm)
        }
        _ => stack_miss(want, &foot, obj, ctx),
    }
}

/// [`want_miss`] for the stacking wants: `Inside`, `OnTopOf` and `Supports`.
/// Anything else, and a non-object target, can never hold.
fn stack_miss(want: &Want, foot: &Foot, obj: &Placed, ctx: &Ctx<'_>) -> i64 {
    let Target::Object(id) = want.target else {
        return NEVER_CM;
    };
    let (min_x, max_x) = foot.x();
    let (min_z, max_z) = foot.z();
    let (p_min_y, p_max_y) = foot.y();
    match want.relation {
        Relation::Inside => {
            let Some(outer) = ctx.boxes.get(&id) else {
                return NEVER_CM;
            };
            if outer.id == obj.id {
                return NEVER_CM;
            }
            let protrusion = [
                outer.min_x - min_x,
                max_x - outer.max_x,
                outer.min_y - p_min_y,
                p_max_y - outer.max_y,
                outer.min_z - min_z,
                max_z - outer.max_z,
            ]
            .into_iter()
            .max()
            .unwrap_or(0);
            if protrusion > 0 {
                return i64::from(protrusion);
            }
            // Contained but not strictly smaller: one cm of growth in the room.
            i64::from(box_volume(outer) <= box_volume(&foot.probe()))
        }
        Relation::OnTopOf => {
            let Some(box3) = ctx.boxes.get(&id) else {
                return NEVER_CM;
            };
            if box3.id == obj.id {
                return NEVER_CM;
            }
            overlap_miss(min_x, max_x, box3.min_x, box3.max_x)
                + overlap_miss(min_z, max_z, box3.min_z, box3.max_z)
                + rest_miss(i64::from(support_gap_cm(box3.max_y, p_min_y)))
        }
        Relation::Supports => {
            let Some(target) = ctx.by_id.get(&id) else {
                return NEVER_CM;
            };
            if target.id == obj.id || !pos_known(target) || !size_known(target) {
                return NEVER_CM;
            }
            let thx = to_cm(target.sx).abs() / 2;
            let thz = to_cm(target.sz).abs() / 2;
            let (tcx, tcz) = (to_cm(target.x), to_cm(target.z));
            let t_min_y = to_cm(target.y) - to_cm(target.sy).abs() / 2;
            overlap_miss(min_x, max_x, tcx - thx, tcx + thx)
                + overlap_miss(min_z, max_z, tcz - thz, tcz + thz)
                + rest_miss(i64::from(support_gap_cm(p_max_y, t_min_y)))
        }
        Relation::NextTo | Relation::Facing => NEVER_CM,
    }
}

/// What a want measures against, as a blocker.
fn own_target(target: Target) -> Blocker {
    match target {
        Target::Wall(w) => Blocker::Wall(w),
        Target::Object(id) => Blocker::Target(id),
        Target::Point { x_cm, z_cm } => Blocker::Point { x_cm, z_cm },
    }
}

/// Grid scan inputs shared by both passes.
struct Scan<'a, 'c> {
    obj: &'a Placed,
    ctx: &'a Ctx<'c>,
    index: &'a Index3,
    grid: &'a Grid,
    yaws: i32,
}

struct Near {
    miss: i64,
    x: i32,
    z: i32,
}

/// Nearest pose to meeting want `i` while every other unmet want holds and the
/// pose verifies clean (`strict`), else the nearest pose to want `i` alone.
/// Misses are measured at the rest the finished placement would take.
fn nearest(scan: &Scan<'_, '_>, wants: &[Want], i: usize, strict: bool) -> Option<Near> {
    let others: Vec<Want> = wants
        .iter()
        .enumerate()
        .filter(|(j, _)| *j != i)
        .map(|(_, w)| *w)
        .collect();
    let (kept, rest_cm) = resting(&others, scan.obj, scan.ctx)?;
    let (full, _) = resting(wants, scan.obj, scan.ctx)?;
    let mut best: Option<Near> = None;
    let mut cz = scan.grid.z_lo;
    while cz <= scan.grid.z_hi {
        let mut cx = scan.grid.x_lo;
        while cx <= scan.grid.x_hi {
            for yaw_idx in 0..scan.yaws {
                if strict && !wants_hold(&others, cx, cz, yaw_idx, &kept, scan.ctx) {
                    continue;
                }
                let miss = want_miss(&wants[i], cx, cz, yaw_idx, &full, scan.ctx);
                if best.as_ref().is_some_and(|b| miss >= b.miss) {
                    continue;
                }
                if strict {
                    let out = transform_of(&kept, cx, cz, yaw_idx);
                    if !verifies(scan.index, &kept, &out, rest_cm) {
                        continue;
                    }
                }
                best = Some(Near { miss, x: cx, z: cz });
                if miss <= 1 {
                    return best;
                }
            }
            cx += GRID_CM;
        }
        cz += GRID_CM;
    }
    best
}

struct Taken {
    score: i64,
    blocker: Blocker,
}

/// The cheapest pose that meets every unmet want but overlaps something, and
/// the dominant object there. `None` when no pose meets all the wants (they
/// contradict) or when no meeting pose overlaps an object.
fn taken_spot(scan: &Scan<'_, '_>, wants: &[Want]) -> Option<Taken> {
    let (kept, _) = resting(wants, scan.obj, scan.ctx)?;
    let hx = to_cm(kept.sx).abs() / 2;
    let hy = to_cm(kept.sy).abs() / 2;
    let hz = to_cm(kept.sz).abs() / 2;
    let y_cm = to_cm(kept.y_m);
    let mut best: Option<Taken> = None;
    let mut cz = scan.grid.z_lo;
    while cz <= scan.grid.z_hi {
        let mut cx = scan.grid.x_lo;
        while cx <= scan.grid.x_hi {
            let meets = (0..scan.yaws).any(|yaw| wants_hold(wants, cx, cz, yaw, &kept, scan.ctx));
            if meets {
                let probe = probe_of(cx, y_cm, cz, hx, hy, hz);
                if let Some(taken) = weigh(scan, &probe) {
                    if best.as_ref().is_none_or(|b| taken.score < b.score) {
                        best = Some(taken);
                    }
                }
            }
            cx += GRID_CM;
        }
        cz += GRID_CM;
    }
    best
}

/// Total push needed to clear everything the probe overlaps, and the object
/// with the longest push (lowest id on ties). `None` when the probe is clear.
fn weigh(scan: &Scan<'_, '_>, probe: &Box3) -> Option<Taken> {
    let mut ids = scan
        .index
        .query_box(
            [probe.min_x, probe.min_y, probe.min_z],
            [probe.max_x, probe.max_y, probe.max_z],
        )
        .unwrap_or_default();
    ids.sort_unstable();
    ids.dedup();
    let mut score = 0i64;
    let mut dominant: Option<(i32, Blocker)> = None;
    for id in ids {
        let Some(other) = scan.ctx.boxes.get(&id) else {
            continue;
        };
        let fix = escape(other, probe, false);
        score += i64::from(fix.cm);
        if dominant.is_none_or(|(cm, _)| fix.cm > cm) {
            dominant = Some((
                fix.cm,
                Blocker::Object {
                    id,
                    overlap_cm: overlap_depth_cm(probe, other, false),
                    fix,
                },
            ));
        }
    }
    dominant.map(|(_, blocker)| Taken { score, blocker })
}

/// One [`WantBlock`] per unmet want of a minimal conflict, in conflict order.
/// `wants` is the conflict itself; `grid` is `None` when the object does not
/// fit the room at all (it has no blocks then).
pub(super) fn block_report(
    wants: &[Want],
    obj: &Placed,
    ctx: &Ctx<'_>,
    index: &Index3,
    grid: Option<&Grid>,
) -> Vec<WantBlock> {
    // An object wider than the room has no pose at all, so every trial fails,
    // the shrink drops every want and there is nothing to blame.
    let Some(grid) = grid else {
        return Vec::new();
    };
    let yaws = if wants.iter().any(|w| w.relation == Relation::Facing) {
        YAW_STEPS
    } else {
        1
    };
    let scan = Scan {
        obj,
        ctx,
        index,
        grid,
        yaws,
    };
    let taken = taken_spot(&scan, wants);
    (0..wants.len())
        .map(|i| {
            let near = nearest(&scan, wants, i, true).or_else(|| nearest(&scan, wants, i, false));
            let (miss, near_miss) = near
                .as_ref()
                .map_or((NEVER_CM, [0, 0]), |n| (n.miss, [n.x, n.z]));
            let blocker = taken
                .as_ref()
                .map_or_else(|| own_target(wants[i].target), |t| t.blocker);
            WantBlock {
                want: i,
                blocker,
                miss_cm: clamp_cm(miss.max(NEVER_CM)),
                near_miss,
            }
        })
        .collect()
}

#[cfg(test)]
mod conflict_blockers {
    use super::*;
    use engine_core::Rng;

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

    /// Four thin walls (outer faces at +-510 cm), a gate near the north wall
    /// and the brazier we place.
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

    fn near(relation: Relation, target: Target, range: (i32, i32)) -> Want {
        Want {
            relation,
            target,
            distance_cm: Some(range),
        }
    }

    /// Seeds 0..10 pin the brazier against both the north and south walls;
    /// seeds 10..20 ask for an edge gap to the gate that is both near and far.
    fn seeded(seed: usize) -> (Intent, i32) {
        if seed < 10 {
            let hi_n = 30 + (seed as i32 % 3) * 10;
            let hi_s = 30 + ((seed as i32 + 1) % 3) * 10;
            let wants = vec![
                near(Relation::NextTo, Target::Wall(Wall::North), (0, hi_n)),
                near(Relation::NextTo, Target::Wall(Wall::South), (0, hi_s)),
            ];
            // Walls are 20 cm thick, so a clean pose sits at gap 20 or more; the
            // other want pins the gap, and the room is 1020 cm deep.
            let expected = 960 - hi_n - hi_s;
            (
                Intent {
                    object: ObjectRef::Id(5),
                    wants,
                },
                expected,
            )
        } else {
            let s = seed - 10;
            let near_hi = 40 + (s as i32 % 3) * 10;
            let far_lo = 300 + (s as i32 % 2) * 20;
            let wants = vec![
                near(Relation::NextTo, Target::Object(4), (0, near_hi)),
                near(Relation::NextTo, Target::Object(4), (far_lo, far_lo + 100)),
            ];
            (
                Intent {
                    object: ObjectRef::Id(5),
                    wants,
                },
                far_lo - near_hi,
            )
        }
    }

    #[test]
    fn twenty_seeded_conflicts_each_name_a_blocker_and_a_miss() {
        for seed in 0..20 {
            let actors = room();
            let (intent, expected) = seeded(seed);
            let err = super::super::solve(&intent, &actors).expect_err("seeded intent conflicts");
            assert_eq!(
                err.unmet, intent.wants,
                "seed {seed}: minimal subset keeps both"
            );
            assert!(
                err.reason.contains("minimal conflict") && !err.reason.contains("gate"),
                "seed {seed}: one fixed sentence, counts only: {}",
                err.reason
            );
            assert_eq!(
                err.blocks.len(),
                err.unmet.len(),
                "seed {seed}: one per unmet want"
            );
            for (i, block) in err.blocks.iter().enumerate() {
                assert_eq!(block.want, i, "seed {seed}");
                assert_eq!(block.miss_cm, expected, "seed {seed} want {i}: miss in cm");
                match (seed < 10, block.blocker) {
                    (true, Blocker::Wall(w)) => {
                        let Target::Wall(asked) = intent.wants[i].target else {
                            panic!("seed {seed}: wall want");
                        };
                        assert_eq!(w, asked, "seed {seed} want {i}: names its wall");
                    }
                    (false, Blocker::Target(id)) => {
                        assert_eq!(id, 4, "seed {seed}: names the gate");
                    }
                    (_, other) => panic!("seed {seed} want {i}: unexpected blocker {other:?}"),
                }
                // The near-miss pose keeps every other want and is clean.
                let obj = Placed {
                    id: 5,
                    sx: 0.6,
                    sy: 1.0,
                    sz: 0.6,
                    y_m: 0.5,
                };
                let bounds = room_bounds(&actors).expect("room has bounds");
                let ctx = context(&actors, bounds);
                for (j, w) in intent.wants.iter().enumerate() {
                    let holds = satisfies(w, block.near_miss[0], block.near_miss[1], 0, &obj, &ctx);
                    assert_eq!(
                        holds,
                        j != i,
                        "seed {seed} want {i}: near-miss keeps the others"
                    );
                }
            }
            let again = super::super::solve(&intent, &actors).expect_err("repeat conflicts");
            assert_eq!(err, again, "seed {seed}: x2 identical");
            let mut rev = actors.clone();
            rev.reverse();
            let shuffled = super::super::solve(&intent, &rev).expect_err("shuffled conflicts");
            assert_eq!(
                err.blocks, shuffled.blocks,
                "seed {seed}: input order never moves blocks"
            );
        }
    }

    #[test]
    fn a_taken_spot_names_the_object_the_overlap_and_the_push() {
        let mut actors = room();
        // A crate parked exactly where the want points.
        actors.push(mk(6, "crate", 0.0, 0.5, -4.0, 1.0, 1.0, 1.0));
        let intent = Intent {
            object: ObjectRef::Id(5),
            wants: vec![near(
                Relation::NextTo,
                Target::Point {
                    x_cm: 0,
                    z_cm: -400,
                },
                (0, 0),
            )],
        };
        let err = super::super::solve(&intent, &actors).expect_err("taken spot conflicts");
        assert_eq!(err.blocks.len(), 1);
        let block = err.blocks[0];
        // 60 cm brazier centered inside a 100 cm crate: it is inside by 60 cm,
        // and the crate must travel 80 cm to leave the brazier's footprint.
        assert_eq!(
            block.blocker,
            Blocker::Object {
                id: 6,
                overlap_cm: 60,
                fix: Escape { dir: "-x", cm: 80 },
            }
        );
        // Nearest clean pose is a crate half-width plus a brazier half-width away.
        assert_eq!(block.miss_cm, 80);
        let (dx, dz) = (block.near_miss[0], block.near_miss[1] + 400);
        assert_eq!(
            dx * dx + dz * dz,
            80 * 80,
            "near-miss sits 80 cm from the point"
        );
        let again = super::super::solve(&intent, &actors).expect_err("repeat conflicts");
        assert_eq!(err, again);
    }

    #[test]
    fn identity_refusals_carry_no_blocks() {
        let intent = Intent {
            object: ObjectRef::Name("missing".to_string()),
            wants: vec![near(Relation::NextTo, Target::Wall(Wall::North), (0, 30))],
        };
        let err = super::super::solve(&intent, &room()).expect_err("unknown object");
        assert!(err.blocks.is_empty());
    }

    /// `want_miss` is zero exactly where `satisfies` is true, for every kind of want.
    #[test]
    fn miss_is_zero_exactly_where_the_want_holds() {
        let mut actors = room();
        actors.push(mk(6, "shelf", 2.0, 0.5, 2.0, 2.0, 1.0, 1.0));
        actors.push(mk(7, "beam", -2.0, 2.5, 1.0, 2.0, 1.0, 1.0));
        let bounds = room_bounds(&actors).expect("room has bounds");
        let ctx = context(&actors, bounds);
        let obj = Placed {
            id: 5,
            sx: 0.6,
            sy: 1.0,
            sz: 0.6,
            y_m: 0.5,
        };
        let targets = [
            Target::Wall(Wall::North),
            Target::Wall(Wall::East),
            Target::Object(4),
            Target::Object(6),
            Target::Object(7),
            Target::Object(5),
            Target::Object(99),
            Target::Point {
                x_cm: 100,
                z_cm: -50,
            },
        ];
        let relations = [
            Relation::NextTo,
            Relation::Facing,
            Relation::Inside,
            Relation::OnTopOf,
            Relation::Supports,
        ];
        let ranges = [
            None,
            Some((0, 50)),
            Some((120, 400)),
            Some((60, 10)),
            Some((-5, 30)),
        ];
        let mut rng = Rng::new(0x5b6);
        let mut zero = 0;
        let mut total = 0;
        for relation in relations {
            for target in targets {
                for range in ranges {
                    let want = Want {
                        relation,
                        target,
                        distance_cm: range,
                    };
                    for _ in 0..40 {
                        let cx = ((rng.next_u32() % 90) as i32 - 45) * 10;
                        let cz = ((rng.next_u32() % 90) as i32 - 45) * 10;
                        let yaw = (rng.next_u32() % 8) as i32;
                        let holds = satisfies(&want, cx, cz, yaw, &obj, &ctx);
                        let miss = want_miss(&want, cx, cz, yaw, &obj, &ctx);
                        assert_eq!(
                            holds,
                            miss == 0,
                            "{want:?} at ({cx},{cz}) yaw {yaw}: miss {miss}"
                        );
                        assert!(miss >= 0, "{want:?}: negative miss");
                        zero += usize::from(holds);
                        total += 1;
                    }
                }
            }
        }
        assert!(
            zero > 20 && zero < total,
            "sample reaches both sides: {zero}/{total}"
        );
    }

    #[test]
    fn escape_takes_the_shortest_axis_and_breaks_ties_in_word_order() {
        let wall = Box3 {
            id: 1,
            min_x: -10,
            max_x: 10,
            min_y: 0,
            max_y: 200,
            min_z: -500,
            max_z: 500,
        };
        let crate_box = Box3 {
            id: 2,
            min_x: -50,
            max_x: 50,
            min_y: 0,
            max_y: 100,
            min_z: 0,
            max_z: 100,
        };
        // Crate straddles the wall: 60 cm out toward -x, 60 toward +x; ties go to -x.
        assert_eq!(
            escape(&crate_box, &wall, true),
            Escape { dir: "-x", cm: 60 }
        );
        assert_eq!(overlap_depth_cm(&crate_box, &wall, true), 20);
        let right = Box3 {
            min_x: -5,
            max_x: 95,
            ..crate_box
        };
        assert_eq!(escape(&right, &wall, false), Escape { dir: "+x", cm: 15 });
        // Apart reads zero.
        let far = Box3 {
            min_x: 100,
            max_x: 200,
            ..crate_box
        };
        assert_eq!(escape(&far, &wall, false).cm, 0);
        assert_eq!(overlap_depth_cm(&far, &wall, false), 0);
    }

    #[test]
    fn audit_digest_ignores_blocks() {
        let mut conflicts = Vec::new();
        for seed in 0..20 {
            let (intent, _) = seeded(seed);
            conflicts
                .push(super::super::solve(&intent, &room()).expect_err("seeded intent conflicts"));
        }
        let with = super::super::canonical_audit_digest(&conflicts);
        for c in &mut conflicts {
            c.blocks.clear();
        }
        assert_eq!(with, super::super::canonical_audit_digest(&conflicts));
        assert_ne!(with, 0);
    }
}
