use super::*;
use engine_core::vec3::orient2;

// ---- integer helpers ---------------------------------------------------------

/// Integer square root (floor): exact, no libm, no float; `0` for `n <= 0`.
pub(super) fn isqrt(n: i64) -> i64 {
    n.checked_isqrt().unwrap_or(0)
}

/// FNV-1a hex digest of a task id: the deterministic pseudo-capture stamp
/// vision advice cites. A stand-in until live `play.step` captures feed the
/// cross-check; advisory only, never scored.
pub(super) fn capture_hex(id: &str) -> String {
    let mut hash = Fnv64::new();
    hash.write(EVAL_TAG);
    mix_text(&mut hash, id);
    format!("{:016x}", hash.finish())
}

// ---- geometry (fixture-grade 2D oracle, integer cm) --------------------------

/// One grounded footprint in integer cm: center plus half extents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Rect {
    pub(super) min_x: i32,
    pub(super) max_x: i32,
    pub(super) min_z: i32,
    pub(super) max_z: i32,
}

/// Strict overlap: touching edges are clear, matching the spatial crate.
pub(super) fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.min_x < b.max_x && b.min_x < a.max_x && a.min_z < b.max_z && b.min_z < a.max_z
}

/// Strict interior test for segment endpoints.
pub(super) fn inside_strict(p: (i32, i32), r: &Rect) -> bool {
    p.0 > r.min_x && p.0 < r.max_x && p.1 > r.min_z && p.1 < r.max_z
}

/// True when segments ab and cd cross in their strict interiors; touching
/// at endpoints or edges never counts, so grazing sightlines stay clear.
pub(super) fn proper_cross(a: (i32, i32), b: (i32, i32), c: (i32, i32), d: (i32, i32)) -> bool {
    let o1 = orient2(a, b, c);
    let o2 = orient2(a, b, d);
    let o3 = orient2(c, d, a);
    let o4 = orient2(c, d, b);
    ((o1 > 0 && o2 < 0) || (o1 < 0 && o2 > 0)) && ((o3 > 0 && o4 < 0) || (o3 < 0 && o4 > 0))
}

/// True when the open segment from `eye` to `target` enters any blocker
/// interior. Edges of `Rect` hold (min, min) and (max, max) corners.
pub(super) fn seg_blocked(eye: (i32, i32), target: (i32, i32), blockers: &[Rect]) -> bool {
    for r in blockers {
        if inside_strict(eye, r) || inside_strict(target, r) {
            return true;
        }
        let corners = [
            (r.min_x, r.min_z),
            (r.max_x, r.min_z),
            (r.max_x, r.max_z),
            (r.min_x, r.max_z),
        ];
        for i in 0..4 {
            if proper_cross(eye, target, corners[i], corners[(i + 1) % 4]) {
                return true;
            }
        }
    }
    false
}

/// Facing vector for an octant yaw: 0 faces +X, steps run counter-clockwise
/// seen from above (+X, +X+Z, +Z, -X+Z, -X, -X-Z, -Z, +X-Z).
pub(super) fn facing(oct: u8) -> (i32, i32) {
    match oct % 8 {
        0 => (1, 0),
        1 => (1, 1),
        2 => (0, 1),
        3 => (-1, 1),
        4 => (-1, 0),
        5 => (-1, -1),
        6 => (0, -1),
        _ => (1, -1),
    }
}

/// In-view test: target ahead within a 45-degree half cone, integer exact
/// (`dot > 0` and `|cross| <= dot`), so cone edges never wobble on floats.
pub(super) fn in_view(eye: (i32, i32), target: (i32, i32), oct: u8) -> bool {
    let (fx, fz) = facing(oct);
    let dx = i64::from(target.0) - i64::from(eye.0);
    let dz = i64::from(target.1) - i64::from(eye.1);
    if dx == 0 && dz == 0 {
        return true;
    }
    let dot = dx * i64::from(fx) + dz * i64::from(fz);
    if dot <= 0 {
        return false;
    }
    let cross = (dx * i64::from(fz) - dz * i64::from(fx)).abs();
    cross <= dot
}

/// Squared edge gap from a point to a rect; 0 when touching or inside.
pub(super) fn point_gap2(p: (i32, i32), r: &Rect) -> i64 {
    let gx = if p.0 < r.min_x {
        i64::from(r.min_x) - i64::from(p.0)
    } else if p.0 > r.max_x {
        i64::from(p.0) - i64::from(r.max_x)
    } else {
        0
    };
    let gz = if p.1 < r.min_z {
        i64::from(r.min_z) - i64::from(p.1)
    } else if p.1 > r.max_z {
        i64::from(p.1) - i64::from(r.max_z)
    } else {
        0
    };
    gx * gx + gz * gz
}

/// Smallest edge gap from `r` to any rect; negative when overlapping,
/// zero when touching.
pub(super) fn edge_gap(r: &Rect, rects: &[Rect]) -> i32 {
    let mut best = i32::MAX;
    for b in rects {
        let gx = if r.min_x >= b.max_x {
            r.min_x - b.max_x
        } else if b.min_x >= r.max_x {
            b.min_x - r.max_x
        } else {
            -1
        };
        let gz = if r.min_z >= b.max_z {
            r.min_z - b.max_z
        } else if b.min_z >= r.max_z {
            b.min_z - r.max_z
        } else {
            -1
        };
        let gap = if gx < 0 && gz < 0 {
            -1
        } else if gx < 0 {
            gz
        } else if gz < 0 {
            gx
        } else {
            gx.min(gz)
        };
        best = best.min(gap);
    }
    if best == i32::MAX {
        MAX_RADIUS_CM
    } else {
        best
    }
}

// ---- scenes ------------------------------------------------------------------

/// One eval actor: grounded footprint in integer cm plus its nav polygon
/// (`None` rides off-mesh, mirroring the live reach view).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Actor {
    pub(super) id: u32,
    pub(super) x_cm: i32,
    pub(super) z_cm: i32,
    pub(super) hx_cm: i32,
    pub(super) hz_cm: i32,
    pub(super) poly: Option<u32>,
}

impl Actor {
    pub(super) const fn rect(&self) -> Rect {
        Rect {
            min_x: self.x_cm - self.hx_cm,
            max_x: self.x_cm + self.hx_cm,
            min_z: self.z_cm - self.hz_cm,
            max_z: self.z_cm + self.hz_cm,
        }
    }
}

/// Scene under test: bounds plus actors plus the nav adjacency the
/// reachable oracle walks. Id-sorted by construction.
pub(super) struct Scene {
    pub(super) name: &'static str,
    pub(super) min_cm: i32,
    pub(super) max_cm: i32,
    pub(super) actors: &'static [Actor],
    pub(super) adj: &'static [(u32, u32)],
}

pub(super) const fn wall(id: u32, x: i32, z: i32, hx: i32, hz: i32) -> Actor {
    Actor {
        id,
        x_cm: x,
        z_cm: z,
        hx_cm: hx,
        hz_cm: hz,
        poly: None,
    }
}

pub(super) const fn prop(id: u32, x: i32, z: i32, poly: Option<u32>) -> Actor {
    Actor {
        id,
        x_cm: x,
        z_cm: z,
        hx_cm: 40,
        hz_cm: 40,
        poly,
    }
}

/// Walled 10 m room with a pillar occluder and crates on the west/east
/// axis plus one north, one far west, edge bands and one off-mesh actor;
/// coordinates mirror the live query gold scene decimeter for decimeter.
pub(super) const PILLAR_ACTORS: [Actor; 13] = [
    wall(0, 0, -500, 500, 10),
    wall(1, 0, 500, 500, 10),
    wall(2, -500, 0, 10, 500),
    wall(3, 500, 0, 10, 500),
    prop(10, 200, 0, Some(0)),
    prop(20, -400, 0, Some(0)),
    prop(21, 0, 0, Some(0)),
    prop(22, 400, 0, Some(9)),
    prop(23, 0, 400, Some(0)),
    prop(24, -900, 0, Some(0)),
    prop(25, 200, 200, Some(0)),
    prop(26, -200, 500, Some(0)),
    prop(27, 300, 400, None),
];

pub(super) const OPEN_ACTORS: [Actor; 8] = [
    wall(0, 0, -500, 500, 10),
    wall(1, 0, 500, 500, 10),
    wall(2, -500, 0, 10, 500),
    wall(3, 500, 0, 10, 500),
    prop(30, -300, -300, Some(0)),
    prop(31, 0, 0, Some(1)),
    prop(32, 300, 300, Some(2)),
    prop(33, 400, -400, Some(9)),
];

pub(super) const TIGHT_ACTORS: [Actor; 9] = [
    wall(0, 0, -500, 500, 10),
    wall(1, 0, 500, 500, 10),
    wall(2, -500, 0, 10, 500),
    wall(3, 500, 0, 10, 500),
    prop(40, -300, 0, Some(0)),
    prop(41, -100, 0, Some(0)),
    prop(42, 100, 0, Some(0)),
    prop(43, 300, 0, Some(0)),
    prop(44, 0, -300, Some(0)),
];

/// Poly adjacency the reachable oracle walks: the island poly stays
/// disconnected, the open room chains 0-1-2.
pub(super) const NO_ADJ: [(u32, u32); 0] = [];
pub(super) const OPEN_ADJ: [(u32, u32); 2] = [(0, 1), (1, 2)];

pub(super) const SCENES: [Scene; 3] = [
    Scene {
        name: "room_pillar",
        min_cm: -500,
        max_cm: 500,
        actors: &PILLAR_ACTORS,
        adj: &NO_ADJ,
    },
    Scene {
        name: "room_open",
        min_cm: -500,
        max_cm: 500,
        actors: &OPEN_ACTORS,
        adj: &OPEN_ADJ,
    },
    Scene {
        name: "room_tight",
        min_cm: -500,
        max_cm: 500,
        actors: &TIGHT_ACTORS,
        adj: &NO_ADJ,
    },
];

pub(super) fn scene_of(name: &str) -> Option<&'static Scene> {
    SCENES.iter().find(|s| s.name == name)
}

pub(super) fn actor_of(scene: &Scene, id: u32) -> Option<&Actor> {
    scene.actors.iter().find(|a| a.id == id)
}

// ---- oracles -------------------------------------------------------------------

pub(super) fn dist_cm(a: (i32, i32), b: (i32, i32)) -> i64 {
    let dx = i64::from(a.0) - i64::from(b.0);
    let dz = i64::from(a.1) - i64::from(b.1);
    isqrt(dx * dx + dz * dz)
}

/// Near oracle: ids within `radius_cm` of the anchor as an ascending set,
/// mirroring the `scene.query near` membership (ranking lives in the live
/// answer reasons, not in gold).
pub(super) fn near_ids(scene: &Scene, anchor: u32, radius_cm: i32) -> Vec<u32> {
    let Some(base) = actor_of(scene, anchor) else {
        return Vec::new();
    };
    let origin = (base.x_cm, base.z_cm);
    let mut ids: Vec<u32> = Vec::new();
    for a in scene.actors {
        if a.id == anchor || a.poly.is_none() {
            continue;
        }
        if dist_cm(origin, (a.x_cm, a.z_cm)) <= i64::from(radius_cm) {
            ids.push(a.id);
        }
    }
    ids.sort_unstable();
    ids
}

/// Between oracle: ids on the floor-plan corridor from `a` to `b`,
/// mirroring the `scene.query between` tolerance.
pub(super) fn between_ids(scene: &Scene, a: u32, b: u32) -> Vec<u32> {
    let (Some(first), Some(second)) = (actor_of(scene, a), actor_of(scene, b)) else {
        return Vec::new();
    };
    if a == b {
        return Vec::new();
    }
    let pa = (first.x_cm, first.z_cm);
    let pb = (second.x_cm, second.z_cm);
    let direct = dist_cm(pa, pb);
    let mut ids: Vec<u32> = Vec::new();
    for c in scene.actors {
        if c.id == a || c.id == b || c.poly.is_none() {
            continue;
        }
        let pc = (c.x_cm, c.z_cm);
        if dist_cm(pa, pc) + dist_cm(pc, pb) <= direct + BETWEEN_TOL_CM {
            ids.push(c.id);
        }
    }
    ids.sort_unstable();
    ids
}

/// One visibility row: id plus the closed reason vocabulary the live
/// `scene.query visible` answers share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Seen {
    pub(super) id: u32,
    pub(super) reason: &'static str,
}

/// Visible oracle: 2D cone plus segment occlusion over actor rects.
/// Fixture-grade (the live sight path owns 3D truth); exact in integers.
pub(super) fn visible_rows(scene: &Scene, eye: (i32, i32), oct: u8) -> Vec<Seen> {
    let rects: Vec<(u32, Rect)> = scene.actors.iter().map(|a| (a.id, a.rect())).collect();
    let mut rows = Vec::new();
    for a in scene.actors {
        if a.poly.is_none() {
            continue;
        }
        let target = (a.x_cm, a.z_cm);
        let blockers: Vec<Rect> = rects
            .iter()
            .filter(|(id, _)| *id != a.id)
            .map(|(_, r)| *r)
            .collect();
        let reason = if !in_view(eye, target, oct) {
            "blocked:out_of_view"
        } else if seg_blocked(eye, target, &blockers) {
            "blocked:occluded"
        } else {
            "visible"
        };
        rows.push(Seen { id: a.id, reason });
    }
    rows.sort_by_key(|s| s.id);
    rows
}

/// Reachable oracle: breadth-first walk over the scene poly adjacency.
/// Off-mesh actors read `blocked:off_mesh`, unreachable polys read
/// `blocked:unreachable`, mirroring the live reach reasons.
pub(super) fn reachable_rows(scene: &Scene, anchor: u32) -> Vec<Seen> {
    let mut shut: Vec<u32> = Vec::new();
    let mut open: Vec<u32> = Vec::new();
    if let Some(poly) = actor_of(scene, anchor).and_then(|a| a.poly) {
        open.push(poly);
    }
    while let Some(poly) = open.pop() {
        if shut.contains(&poly) {
            continue;
        }
        shut.push(poly);
        for (x, y) in scene.adj {
            if *x == poly && !shut.contains(y) && !open.contains(y) {
                open.push(*y);
            }
            if *y == poly && !shut.contains(x) && !open.contains(x) {
                open.push(*x);
            }
        }
    }
    shut.sort_unstable();
    let mut seen = Vec::new();
    for a in scene.actors {
        if a.id == anchor {
            continue;
        }
        match a.poly {
            None => seen.push(Seen {
                id: a.id,
                reason: "blocked:off_mesh",
            }),
            Some(poly) if shut.contains(&poly) => seen.push(Seen {
                id: a.id,
                reason: "reachable",
            }),
            Some(_) => seen.push(Seen {
                id: a.id,
                reason: "blocked:unreachable",
            }),
        }
    }
    seen.sort_by_key(|s| s.id);
    seen
}

/// Cover oracle: the feet-to-threat segment decides. Standing cover reads
/// full exactly when a wall, pillar or crate blocks the segment; crouching
/// additionally grants half within 150 cm of any rect edge, so crouch sets
/// only ever grow. Fixture-grade; the live sight-and-reach path owns 3D truth.
pub(super) fn cover_rows(
    scene: &Scene,
    anchor: u32,
    threat: (i32, i32),
    crouch: bool,
    radius_cm: i32,
) -> Vec<Seen> {
    let Some(base) = actor_of(scene, anchor) else {
        return Vec::new();
    };
    let origin = (base.x_cm, base.z_cm);
    let rects: Vec<(u32, Rect)> = scene.actors.iter().map(|a| (a.id, a.rect())).collect();
    let mut rows = Vec::new();
    for a in scene.actors {
        if a.id == anchor || a.poly.is_none() {
            continue;
        }
        if dist_cm(origin, (a.x_cm, a.z_cm)) > i64::from(radius_cm) {
            continue;
        }
        let feet = (a.x_cm, a.z_cm);
        let blockers: Vec<Rect> = rects
            .iter()
            .filter(|(id, _)| *id != a.id)
            .map(|(_, r)| *r)
            .collect();
        let blocked = seg_blocked(feet, threat, &blockers);
        let near_edge = rects
            .iter()
            .filter(|(id, _)| *id != a.id)
            .any(|(_, r)| point_gap2(feet, r) <= 150 * 150);
        let reason = if blocked {
            "cover:full"
        } else if crouch && near_edge {
            "cover:half"
        } else {
            "cover:none"
        };
        rows.push(Seen { id: a.id, reason });
    }
    rows.sort_by_key(|s| s.id);
    rows
}

/// Candidate footprint clears every scene rect once expanded by the
/// capsule, and sits inside the room bounds.
pub(super) fn fits(scene: &Scene, x: i32, z: i32, hx: i32, hz: i32, capsule_cm: i32) -> bool {
    if x - hx - capsule_cm < scene.min_cm
        || x + hx + capsule_cm > scene.max_cm
        || z - hz - capsule_cm < scene.min_cm
        || z + hz + capsule_cm > scene.max_cm
    {
        return false;
    }
    let padded = Rect {
        min_x: x - hx - capsule_cm,
        max_x: x + hx + capsule_cm,
        min_z: z - hz - capsule_cm,
        max_z: z + hz + capsule_cm,
    };
    !scene.actors.iter().any(|a| overlaps(&padded, &a.rect()))
}

/// Largest-open oracle: grid scan for the footprint center with maximum
/// clearance. Ties break by (z, x) ascending. Mirrors the live
/// `largest_open_spot` scan contract in 2D.
pub(super) fn largest_open(
    scene: &Scene,
    hx: i32,
    hz: i32,
    capsule_cm: i32,
) -> Option<(i32, i32, i32)> {
    let lo_x = scene.min_cm + hx + capsule_cm;
    let hi_x = scene.max_cm - hx - capsule_cm;
    let lo_z = scene.min_cm + hz + capsule_cm;
    let hi_z = scene.max_cm - hz - capsule_cm;
    if lo_x > hi_x || lo_z > hi_z {
        return None;
    }
    let rects: Vec<Rect> = scene.actors.iter().map(Actor::rect).collect();
    let mut best: Option<(i32, i32, i32)> = None;
    let mut z = lo_z;
    while z <= hi_z {
        let mut x = lo_x;
        while x <= hi_x {
            let cell = Rect {
                min_x: x - hx,
                max_x: x + hx,
                min_z: z - hz,
                max_z: z + hz,
            };
            if !rects
                .iter()
                .any(|b| overlaps(&expanded(&cell, capsule_cm), b))
            {
                let gap = edge_gap(&cell, &rects);
                let wins = match best {
                    None => true,
                    Some((g, bx, bz)) => gap > g || (gap == g && (z, x) < (bz, bx)),
                };
                if wins {
                    best = Some((gap, x, z));
                }
            }
            x += OPEN_SCAN_CM;
        }
        z += OPEN_SCAN_CM;
    }
    best.map(|(gap, x, z)| (x, z, gap))
}

/// Expand a rect by `pad` cm on every side.
pub(super) fn expanded(r: &Rect, pad: i32) -> Rect {
    Rect {
        min_x: r.min_x - pad,
        max_x: r.max_x + pad,
        min_z: r.min_z - pad,
        max_z: r.max_z + pad,
    }
}

/// Placement verdict: overlapping scene ids in ascending order, or clean.
/// Proposals outside the room bounds refuse with an empty conflict set and
/// `clean = false`, so bounds escapes never read as placements.
pub(super) fn place_verdict(scene: &Scene, x: i32, z: i32, hx: i32, hz: i32) -> (bool, Vec<u32>) {
    if x - hx < scene.min_cm
        || x + hx > scene.max_cm
        || z - hz < scene.min_cm
        || z + hz > scene.max_cm
    {
        return (false, Vec::new());
    }
    let cell = Rect {
        min_x: x - hx,
        max_x: x + hx,
        min_z: z - hz,
        max_z: z + hz,
    };
    let mut hits: Vec<u32> = scene
        .actors
        .iter()
        .filter(|a| overlaps(&cell, &a.rect()))
        .map(|a| a.id)
        .collect();
    hits.sort_unstable();
    (hits.is_empty(), hits)
}

/// Repair oracle: first clean cell in the fixed scan order, minimizing
/// (distance-squared, dz, dx). Deterministic; the gold hint verifies clean
/// when applied, mirroring the live repair-hint contract in 2D.
pub(super) fn repair_hint(
    scene: &Scene,
    x: i32,
    z: i32,
    hx: i32,
    hz: i32,
) -> Option<(i32, i32, i32, i32)> {
    let mut best: Option<(i64, i32, i32, i32, i32)> = None;
    let mut dz = -REPAIR_RADIUS_CM;
    while dz <= REPAIR_RADIUS_CM {
        let mut dx = -REPAIR_RADIUS_CM;
        while dx <= REPAIR_RADIUS_CM {
            let (fx, fz) = (x + dx, z + dz);
            let (clean, _) = place_verdict(scene, fx, fz, hx, hz);
            if clean {
                let d2 = i64::from(dx) * i64::from(dx) + i64::from(dz) * i64::from(dz);
                let wins = match best {
                    None => true,
                    Some((bd2, bdz, bdx, _, _)) => (d2, dz, dx) < (bd2, bdz, bdx),
                };
                if wins {
                    best = Some((d2, dz, dx, fx, fz));
                }
            }
            dx += REPAIR_STEP_CM;
        }
        dz += REPAIR_STEP_CM;
    }
    best.map(|(_, dz, dx, fx, fz)| (dx, dz, fx, fz))
}
