//! navcheck: nav checks over a pure-data bake view. Integer math only, no nav import.

use super::index::{build_rects, pos_known, to_cm};
use super::{Actor, Finding};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Edge of one nav tile, in centimeters.
pub const TILE_CM: i32 = 100;
/// Frame eaten on both sides of a door together, in centimeters.
pub const DOOR_FRAME_CM: i32 = 20; // frame eaten on both sides together
/// Actors slower than this hold no heading (centimeters).
pub const FLOW_MIN_SPEED_CM: i32 = 20; // actors slower than this hold no heading
/// Half-extent of the access-front probe, in centimeters.
pub const PROBE_CM: i32 = 45; // accessFront probe half-extent
/// Single-file clear minimum of a door, in centimeters (red below).
pub const DOOR_SINGLE_CM: i32 = 90; // single-file clear minimum (red below)
/// Two-way clear minimum of a door, in centimeters (yellow below).
pub const DOOR_DOUBLE_CM: i32 = 150; // two-way clear minimum (yellow below)

/// One walkable nav tile.
#[derive(Debug, Clone, Default)]
pub struct Tile {
    pub walk: bool,
}

/// Door clearance data. Width in cm, two-way flag baked per door actor id.
#[derive(Debug, Clone)]
pub struct Door {
    pub width_cm: i32,
    pub two_way: bool,
}

/// Bake output: tiles + off-mesh actor links + flow field + doors + BFS root.
#[derive(Debug, Clone, Default)]
pub struct NavView {
    pub tiles: BTreeMap<(i32, i32), Tile>,
    pub links: Vec<(u32, u32)>,
    pub flow: BTreeMap<(i32, i32), (i8, i8)>,
    pub doors: BTreeMap<u32, Door>,
    pub spawn_id: u32,
}

fn tile_of(x_cm: i32, z_cm: i32) -> (i32, i32) {
    (x_cm.div_euclid(TILE_CM), z_cm.div_euclid(TILE_CM))
}

/// Octant facing from yaw radians. Mult+round are IEEE-exact; no libm call.
/// 0=+X, 2=+Z, 4=-X, 6=-Z. Shared with the relation graph's `Facing`.
pub fn octant(yaw: f32) -> (i32, i32) {
    const DIRS: [(i32, i32); 8] = [
        (1, 0),
        (1, 1),
        (0, 1),
        (-1, 1),
        (-1, 0),
        (-1, -1),
        (0, -1),
        (1, -1),
    ];
    if !yaw.is_finite() {
        return (1, 0);
    }
    let k = (yaw * 1.2732395_f32).round() as i32;
    DIRS[k.rem_euclid(8) as usize]
}

/// Walkable tiles reachable from the spawn actor tile (4-dir + actor links).
fn reachable(nav: &NavView, actors: &[Actor]) -> BTreeSet<(i32, i32)> {
    let mut out = BTreeSet::new();
    if nav.tiles.is_empty() {
        return out;
    }
    let id_tile: BTreeMap<u32, (i32, i32)> = actors
        .iter()
        .filter(|a| pos_known(a))
        .map(|a| (a.id, tile_of(to_cm(a.x), to_cm(a.z))))
        .collect();
    let start = match id_tile.get(&nav.spawn_id) {
        Some(t) => *t,
        None => return out,
    };
    if !nav.tiles.get(&start).map(|t| t.walk).unwrap_or(false) {
        return out;
    }
    // Actor-id links become tile edges.
    let mut edges: BTreeMap<(i32, i32), Vec<(i32, i32)>> = BTreeMap::new();
    for (a, b) in &nav.links {
        if let (Some(ta), Some(tb)) = (id_tile.get(a), id_tile.get(b)) {
            edges.entry(*ta).or_default().push(*tb);
            edges.entry(*tb).or_default().push(*ta);
        }
    }
    let mut q = VecDeque::from([start]);
    out.insert(start);
    while let Some(t) = q.pop_front() {
        let mut next = Vec::new();
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let n = (t.0 + dx, t.1 + dz);
            if nav.tiles.get(&n).map(|x| x.walk).unwrap_or(false) && !out.contains(&n) {
                next.push(n);
            }
        }
        if let Some(extra) = edges.get(&t) {
            for n in extra {
                if nav.tiles.get(n).map(|x| x.walk).unwrap_or(false) && !out.contains(n) {
                    next.push(*n);
                }
            }
        }
        for n in next {
            out.insert(n);
            q.push_back(n);
        }
    }
    out
}

fn red(check: &'static str, a: u32) -> Finding {
    Finding {
        check,
        a,
        b: u32::MAX,
        pct: 0.0,
        severity: "red",
    }
}

fn yellow(check: &'static str, a: u32, b: u32) -> Finding {
    Finding {
        check,
        a,
        b,
        pct: 0.0,
        severity: "yellow",
    }
}

/// Run requested phase-2 checks. Pushes in (check, a) order; caller sorts.
pub fn run_nav(actors: &[Actor], nav: &NavView, wanted: &BTreeSet<&str>) -> Vec<Finding> {
    let mut out = Vec::new();
    let ids: BTreeMap<u32, &Actor> = actors.iter().map(|a| (a.id, a)).collect();

    if wanted.contains("navConnect") {
        let reach = reachable(nav, actors);
        if !reach.is_empty() {
            let mut physical: Vec<u32> = actors
                .iter()
                .filter(|a| a.physical && pos_known(a))
                .filter(|a| {
                    let t = tile_of(to_cm(a.x), to_cm(a.z));
                    !reach.contains(&t)
                })
                .map(|a| a.id)
                .collect();
            physical.sort_unstable();
            for id in physical {
                out.push(red("navConnect", id));
            }
        }
    }

    if wanted.contains("flowPath") {
        let mut movers: Vec<u32> = Vec::new();
        for a in actors {
            if !pos_known(a) || !a.vx.is_finite() || !a.vz.is_finite() {
                continue;
            }
            let vx = i64::from(to_cm(a.vx));
            let vz = i64::from(to_cm(a.vz));
            let v2 = vx * vx + vz * vz;
            if v2 <= i64::from(FLOW_MIN_SPEED_CM) * i64::from(FLOW_MIN_SPEED_CM) {
                continue;
            }
            let t = tile_of(to_cm(a.x), to_cm(a.z));
            let (fx, fz) = match nav.flow.get(&t) {
                Some(d) => (i64::from(d.0), i64::from(d.1)),
                None => continue,
            };
            let f2 = fx * fx + fz * fz;
            if f2 <= 0 {
                continue;
            }
            let dot = vx * fx + vz * fz;
            // cos < 0.707  <=>  dot < 0  or  2*dot^2 < v2*f2
            if dot < 0 || 2 * dot * dot < v2 * f2 {
                movers.push(a.id);
            }
        }
        movers.sort_unstable();
        for id in movers {
            out.push(yellow("flowPath", id, u32::MAX));
        }
    }

    if wanted.contains("accessFront") {
        let rects = build_rects(actors);
        let by_id: BTreeMap<u32, (i32, i32, i32, i32)> = rects
            .iter()
            .map(|r| (r.id, (r.min_x, r.max_x, r.min_z, r.max_z)))
            .collect();
        // The floor of the layout is what the actors step onto, so it never
        // blocks a front probe, and it never needs a free front of its own.
        let plates: BTreeSet<u32> = actors
            .iter()
            .filter(|a| super::checks::is_ground_plate(a))
            .map(|a| a.id)
            .collect();
        let mut blocked: Vec<u32> = Vec::new();
        for a in actors {
            if !pos_known(a) || !a.yaw.is_finite() {
                continue;
            }
            if plates.contains(&a.id) {
                continue;
            }
            let (dx, dz) = octant(a.yaw);
            let px = to_cm(a.x) + dx * PROBE_CM;
            let pz = to_cm(a.z) + dz * PROBE_CM;
            let (x0, x1, z0, z1) = (px - PROBE_CM, px + PROBE_CM, pz - PROBE_CM, pz + PROBE_CM);
            for (id, (ox0, ox1, oz0, oz1)) in &by_id {
                if *id == a.id {
                    continue;
                }
                if plates.contains(id) {
                    continue;
                }
                if x0 < *ox1 && *ox0 < x1 && z0 < *oz1 && *oz0 < z1 {
                    blocked.push(a.id);
                    break;
                }
            }
        }
        blocked.sort_unstable();
        for id in blocked {
            out.push(yellow("accessFront", id, u32::MAX));
        }
    }

    if wanted.contains("doorClear") {
        let mut keys: Vec<u32> = nav.doors.keys().copied().collect();
        keys.sort_unstable();
        for id in keys {
            if !ids.contains_key(&id) {
                continue; // stale bake entry, skip loud-free (bake owns freshness)
            }
            let d = &nav.doors[&id];
            let clear = d.width_cm - DOOR_FRAME_CM;
            if !d.two_way && clear < DOOR_SINGLE_CM {
                out.push(red("doorClear", id));
            } else if d.two_way && clear < DOOR_DOUBLE_CM {
                out.push(Finding {
                    check: "doorClear",
                    a: id,
                    b: u32::MAX,
                    pct: 0.0,
                    severity: "yellow",
                });
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Actor;
    use std::collections::{BTreeMap, BTreeSet};

    fn actor(id: u32, x: f32, z: f32) -> Actor {
        Actor {
            id,
            name: format!("a{id}"),
            x,
            y: 0.5,
            z,
            sx: 1.0,
            sy: 1.0,
            sz: 1.0,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    fn empty_view() -> NavView {
        NavView {
            tiles: BTreeMap::new(),
            links: Vec::new(),
            flow: BTreeMap::new(),
            doors: BTreeMap::new(),
            spawn_id: 0,
        }
    }

    fn wanted(one: &str) -> BTreeSet<&str> {
        BTreeSet::from([one])
    }

    #[test]
    fn doorclear_two_way_narrow_is_yellow() {
        let mut nav = empty_view();
        nav.doors.insert(
            7,
            Door {
                width_cm: 160,
                two_way: true,
            },
        );
        let actors = vec![actor(7, 0.0, 0.0)];
        let out = run_nav(&actors, &nav, &wanted("doorClear"));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].check, "doorClear");
        assert_eq!(out[0].a, 7);
        assert_eq!(out[0].severity, "yellow");
    }

    #[test]
    fn doorclear_two_way_wide_is_clean() {
        let mut nav = empty_view();
        nav.doors.insert(
            7,
            Door {
                width_cm: 200,
                two_way: true,
            },
        );
        let actors = vec![actor(7, 0.0, 0.0)];
        let out = run_nav(&actors, &nav, &wanted("doorClear"));
        assert!(out.is_empty());
    }

    #[test]
    fn doorclear_stale_entry_skipped_and_single_edge() {
        let mut nav = empty_view();
        nav.doors.insert(
            99,
            Door {
                width_cm: 50,
                two_way: false,
            },
        );
        let actors = vec![actor(7, 0.0, 0.0)];
        let out = run_nav(&actors, &nav, &wanted("doorClear"));
        assert!(out.is_empty());

        let mut edge = empty_view();
        edge.doors.insert(
            7,
            Door {
                width_cm: 110,
                two_way: false,
            },
        );
        let clean = run_nav(&actors, &edge, &wanted("doorClear"));
        assert!(clean.is_empty());

        edge.doors.insert(
            7,
            Door {
                width_cm: 109,
                two_way: false,
            },
        );
        let red = run_nav(&actors, &edge, &wanted("doorClear"));
        assert_eq!(red.len(), 1);
        assert_eq!(red[0].severity, "red");
    }

    #[test]
    fn navconnect_empty_view_returns_empty() {
        let actors = vec![actor(0, 0.0, 0.0), actor(1, 2.0, 0.0)];
        let out = run_nav(&actors, &empty_view(), &wanted("navConnect"));
        assert!(out.is_empty());
    }

    #[test]
    fn flowpath_ignores_slow_movers() {
        let mut nav = empty_view();
        nav.tiles.insert((0, 0), Tile { walk: true });
        nav.flow.insert((0, 0), (1, 0));
        let mut slow = actor(0, 0.0, 0.0);
        slow.vx = 0.1;
        let out = run_nav(std::slice::from_ref(&slow), &nav, &wanted("flowPath"));
        assert!(out.is_empty());

        let mut fast = actor(0, 0.0, 0.0);
        fast.vx = -2.0;
        let out2 = run_nav(std::slice::from_ref(&fast), &nav, &wanted("flowPath"));
        assert_eq!(out2.len(), 1);
        assert_eq!(out2[0].severity, "yellow");
    }

    #[test]
    fn accessfront_ground_plate_never_blocks_but_a_bench_does() {
        // The floor of the layout is what the actor steps onto, so it never
        // blocks the front probe. A bench is not a floor and still blocks.
        let mut floor = actor(1, 0.0, 0.0);
        floor.sx = 30.0;
        floor.sy = 0.2;
        floor.sz = 30.0;
        floor.y = 0.1;
        // The walker stands a metre to the side of the slab, so the slab is
        // in front of the walker and the walker is not in front of the slab.
        let walker = actor(2, -1.0, 0.0);
        let out = run_nav(
            &[floor, walker.clone()],
            &empty_view(),
            &wanted("accessFront"),
        );
        assert!(
            out.is_empty(),
            "the floor must not block the front: {out:?}"
        );

        let mut bench = actor(1, 0.0, 0.0);
        bench.sy = 0.2;
        bench.y = 0.1;
        let out = run_nav(&[bench, walker], &empty_view(), &wanted("accessFront"));
        assert_eq!(out.len(), 1, "a bench must block the front: {out:?}");
        assert_eq!(out[0].check, "accessFront");
        assert_eq!(out[0].a, 2);
        assert_eq!(out[0].severity, "yellow");
    }

    #[test]
    fn octant_compass_and_nan_fallback() {
        assert_eq!(octant(0.0), (1, 0));
        assert_eq!(octant(std::f32::consts::FRAC_PI_2), (0, 1));
        assert_eq!(octant(std::f32::consts::PI), (-1, 0));
        assert_eq!(octant(-std::f32::consts::FRAC_PI_2), (0, -1));
        assert_eq!(octant(f32::NAN), (1, 0));
        assert_eq!(octant(f32::INFINITY), (1, 0));
    }
}
