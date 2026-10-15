//! isovist: integer-cm visibility polygon plus closed-ring area.
//! Owner: spatial crate. Consumer: sight readability gate (next slice).
//! Units: integer centimetres in, square centimetres out.
//! Determinism: integer math only (`i64`/`i128`), sorted walls/endpoints and
//! angles; same inputs give the same order twice. No float, no clock.
//! Errors: none; empty walls give empty polygon, short rings return 0.
//! Crossings split at integer-cm points before the sweep; collinear
//! overlaps stay as-is. Fixtures beyond the X crossing are non-crossing.
//! Book: [found2 | 9.1 Polygon Clipping] (clip keeps the positive side).

use std::cmp::Ordering;

/// One wall segment in integer centimetres on the XZ plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WallSeg {
    /// Start X in centimetres.
    pub ax_cm: i32,
    /// Start Z in centimetres.
    pub az_cm: i32,
    /// End X in centimetres.
    pub bx_cm: i32,
    /// End Z in centimetres.
    pub bz_cm: i32,
}

/// Closed-ring area of a polygon in square centimetres.
///
/// Shoelace over `i64`; returns 0 for fewer than 3 points. A 10 m square
/// (1_000 cm side) reports 1_000_000 cm2; a 10_000 cm side reports
/// 100_000_000 cm2. Sign-independent: clockwise and counter-clockwise
/// rings report the same area, twice identically.
pub fn area_cm2(poly: &[[i32; 2]]) -> i64 {
    if poly.len() < 3 {
        return 0;
    }
    let mut acc: i64 = 0;
    for (i, point) in poly.iter().enumerate() {
        let next = poly[(i + 1) % poly.len()];
        acc += i64::from(point[0]) * i64::from(next[1]) - i64::from(next[0]) * i64::from(point[1]);
    }
    acc.abs() / 2
}

fn cross(ax: i64, az: i64, bx: i64, bz: i64) -> i64 {
    ax * bz - az * bx
}
fn ray_hit(origin: [i32; 2], dir: [i64; 2], w: WallSeg) -> Option<(i64, i64, [i32; 2])> {
    let ex = i64::from(w.bx_cm) - i64::from(w.ax_cm);
    let ez = i64::from(w.bz_cm) - i64::from(w.az_cm);
    let rx = i64::from(w.ax_cm) - i64::from(origin[0]);
    let rz = i64::from(w.az_cm) - i64::from(origin[1]);
    let det = cross(ex, ez, dir[0], dir[1]);
    if det == 0 {
        return None;
    }
    let t_num = cross(ex, ez, rx, rz);
    let u_num = cross(dir[0], dir[1], rx, rz);
    let inside = if det > 0 {
        t_num > 0 && u_num >= 0 && u_num <= det
    } else {
        t_num < 0 && u_num <= 0 && u_num >= det
    };
    if !inside {
        return None;
    }
    let (tn, dn) = (t_num.abs(), det.abs());
    let px = (i64::from(origin[0]) * dn + dir[0] * tn) / dn;
    let pz = (i64::from(origin[1]) * dn + dir[1] * tn) / dn;
    let (px32, pz32) = (i32::try_from(px).ok()?, i32::try_from(pz).ok()?);
    Some((tn, dn, [px32, pz32]))
}
fn closest_hit(origin: [i32; 2], dir: [i64; 2], walls: &[WallSeg]) -> Option<[i32; 2]> {
    let mut best: Option<(i64, i64, [i32; 2])> = None;
    for w in walls {
        if let Some((tn, dn, pt)) = ray_hit(origin, dir, *w) {
            let better = match best {
                None => true,
                Some((bn, bd, _)) => {
                    i128::from(tn) * i128::from(bd) < i128::from(bn) * i128::from(dn)
                }
            };
            if better {
                best = Some((tn, dn, pt));
            }
        }
    }
    best.map(|(_, _, pt)| pt)
}
fn angle_cmp(origin: [i32; 2], a: [i32; 2], b: [i32; 2]) -> Ordering {
    let ax = i64::from(a[0]) - i64::from(origin[0]);
    let az = i64::from(a[1]) - i64::from(origin[1]);
    let bx = i64::from(b[0]) - i64::from(origin[0]);
    let bz = i64::from(b[1]) - i64::from(origin[1]);
    let ha = i32::from(!(az > 0 || (az == 0 && ax > 0)));
    let hb = i32::from(!(bz > 0 || (bz == 0 && bx > 0)));
    if ha != hb {
        return ha.cmp(&hb);
    }
    let cr = cross(ax, az, bx, bz);
    if cr != 0 {
        return if cr > 0 {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }
    let da = ax * ax + az * az;
    let db = bx * bx + bz * bz;
    da.cmp(&db).then_with(|| a.cmp(&b))
}
fn simplify_collinear(poly: Vec<[i32; 2]>) -> Vec<[i32; 2]> {
    let n = poly.len();
    if n < 3 {
        return poly;
    }
    let mut kept: Vec<[i32; 2]> = Vec::with_capacity(n);
    for (idx, curr) in poly.iter().enumerate() {
        let prev = poly[(idx + n - 1) % n];
        let next = poly[(idx + 1) % n];
        let inx = i64::from(curr[0]) - i64::from(prev[0]);
        let inz = i64::from(curr[1]) - i64::from(prev[1]);
        let outx = i64::from(next[0]) - i64::from(curr[0]);
        let outz = i64::from(next[1]) - i64::from(curr[1]);
        if !(cross(inx, inz, outx, outz) == 0 && inx * outx + inz * outz > 0) {
            kept.push(*curr);
        }
    }
    kept
}
/// Split proper crossings so the sweep sees only meeting endpoints.
///
/// Caller: [`isovist`] (non-test path) canonicalizes the output again.
/// Collects every interior crossing point per wall (integer cm, rounded
/// to nearest), sorts each wall's points along the wall, and emits the
/// consecutive pieces canonicalized (`a <= b`), sorted and deduped.
/// Proper crossing only: shared endpoints and collinear overlaps stay
/// as-is. Integer math only (`i64`/`i128`); same input gives the same
/// output twice. No float, no clock.
pub fn split_crossings(walls: &[WallSeg]) -> Vec<WallSeg> {
    let n = walls.len();
    if n < 2 {
        return walls.to_vec();
    }
    let mut pts: Vec<Vec<[i32; 2]>> = Vec::with_capacity(n);
    for w in walls {
        pts.push(vec![[w.ax_cm, w.az_cm], [w.bx_cm, w.bz_cm]]);
    }
    for i in 0..n {
        let a = walls[i];
        let p1 = [i64::from(a.ax_cm), i64::from(a.az_cm)];
        let r = [i64::from(a.bx_cm) - p1[0], i64::from(a.bz_cm) - p1[1]];
        for j in (i + 1)..n {
            let b = walls[j];
            let p3 = [i64::from(b.ax_cm), i64::from(b.az_cm)];
            let s = [i64::from(b.bx_cm) - p3[0], i64::from(b.bz_cm) - p3[1]];
            let d1 = cross(s[0], s[1], p1[0] - p3[0], p1[1] - p3[1]);
            let p2 = [i64::from(a.bx_cm), i64::from(a.bz_cm)];
            let d2 = cross(s[0], s[1], p2[0] - p3[0], p2[1] - p3[1]);
            let d3 = cross(r[0], r[1], p3[0] - p1[0], p3[1] - p1[1]);
            let p4 = [i64::from(b.bx_cm), i64::from(b.bz_cm)];
            let d4 = cross(r[0], r[1], p4[0] - p1[0], p4[1] - p1[1]);
            let straddle_a = (d1 > 0 && d2 < 0) || (d1 < 0 && d2 > 0);
            let straddle_b = (d3 > 0 && d4 < 0) || (d3 < 0 && d4 > 0);
            if !(straddle_a && straddle_b) {
                continue;
            }
            let mut denom = cross(r[0], r[1], s[0], s[1]);
            let mut t_num = cross(p3[0] - p1[0], p3[1] - p1[1], s[0], s[1]);
            if denom == 0 {
                continue;
            }
            if denom < 0 {
                denom = -denom;
                t_num = -t_num;
            }
            let round_div = |off: i128, d: i128| -> Option<i64> {
                let q = if off >= 0 {
                    (off + d / 2) / d
                } else {
                    (off - d / 2) / d
                };
                i64::try_from(q).ok()
            };
            let denom128 = i128::from(denom);
            let Some(dx) = round_div(i128::from(r[0]) * i128::from(t_num), denom128) else {
                continue;
            };
            let Some(dz) = round_div(i128::from(r[1]) * i128::from(t_num), denom128) else {
                continue;
            };
            let px = p1[0] + dx;
            let pz = p1[1] + dz;
            let (Some(x), Some(z)) = (i32::try_from(px).ok(), i32::try_from(pz).ok()) else {
                continue;
            };
            pts[i].push([x, z]);
            pts[j].push([x, z]);
        }
    }
    let mut out: Vec<WallSeg> = Vec::new();
    for (idx, w) in walls.iter().enumerate() {
        let mut list = std::mem::take(&mut pts[idx]);
        list.sort_unstable();
        list.dedup();
        let ax = i64::from(w.ax_cm);
        let az = i64::from(w.az_cm);
        let rx = i64::from(w.bx_cm) - ax;
        let rz = i64::from(w.bz_cm) - az;
        list.sort_by_key(|p| {
            let dx = i64::from(p[0]) - ax;
            let dz = i64::from(p[1]) - az;
            dx * rx + dz * rz
        });
        list.dedup();
        for pair in list.windows(2) {
            let (a_pt, b_pt) = (pair[0], pair[1]);
            if a_pt == b_pt {
                continue;
            }
            let (mut a_pt, mut b_pt) = (a_pt, b_pt);
            if (a_pt[0], a_pt[1]) > (b_pt[0], b_pt[1]) {
                std::mem::swap(&mut a_pt, &mut b_pt);
            }
            out.push(WallSeg {
                ax_cm: a_pt[0],
                az_cm: a_pt[1],
                bx_cm: b_pt[0],
                bz_cm: b_pt[1],
            });
        }
    }
    out.sort_by_key(|w| (w.ax_cm, w.az_cm, w.bx_cm, w.bz_cm));
    out.dedup();
    out
}
/// Integer rays cast per distinct endpoint: exact plus two `K = 1024`
/// offsets for shadow edges.
///
/// Caller: [`isovist`] (non-test path) sizes its hit buffer from
/// [`isovist_ray_budget`]. Units: rays; determinism: same count gives
/// the same budget twice. No float, no clock. Book: [found2 | 9.1 Polygon Clipping].
pub const RAYS_PER_ENDPOINT: usize = 3;

/// Upper bound on rays cast for `wall_count` walls: three per endpoint,
/// two endpoints per wall, saturating. The sweep never casts more; an eye
/// on an endpoint casts fewer (that ray is skipped). Pure, no alloc,
/// integer only. Determinism: same count gives the same budget twice.
#[must_use]
pub fn isovist_ray_budget(wall_count: usize) -> usize {
    wall_count
        .saturating_mul(2)
        .saturating_mul(RAYS_PER_ENDPOINT)
}
/// Visibility polygon from an integer-cm origin over wall segments.
///
/// Splits crossings, then canonicalizes and sorts walls, casts three
/// integer rays per endpoint
/// (exact plus two `K = 1024` offsets for shadow edges), keeps the closest
/// hit per ray, sorts by angle, dedups, and drops straight-edge dust.
/// Same inputs give the same order twice.
pub fn isovist(origin: [i32; 2], walls: &[WallSeg]) -> Vec<[i32; 2]> {
    if walls.is_empty() {
        return Vec::new();
    }
    let mut sorted: Vec<WallSeg> = split_crossings(walls);
    for w in sorted.iter_mut() {
        if (w.ax_cm, w.az_cm) > (w.bx_cm, w.bz_cm) {
            (w.ax_cm, w.bx_cm) = (w.bx_cm, w.ax_cm);
            (w.az_cm, w.bz_cm) = (w.bz_cm, w.az_cm);
        }
    }
    sorted.sort_by_key(|w| (w.ax_cm, w.az_cm, w.bx_cm, w.bz_cm));
    sorted.dedup();
    let mut ends: Vec<[i32; 2]> = Vec::with_capacity(sorted.len() * 2);
    for w in &sorted {
        ends.push([w.ax_cm, w.az_cm]);
        ends.push([w.bx_cm, w.bz_cm]);
    }
    ends.sort_unstable();
    ends.dedup();
    const K: i64 = 1024;
    let mut hits: Vec<[i32; 2]> = Vec::with_capacity(isovist_ray_budget(sorted.len()));
    for e in &ends {
        let dx = i64::from(e[0]) - i64::from(origin[0]);
        let dz = i64::from(e[1]) - i64::from(origin[1]);
        if dx == 0 && dz == 0 {
            continue;
        }
        let dirs = [
            [dx, dz],
            [dx * K - dz, dz * K + dx],
            [dx * K + dz, dz * K - dx],
        ];
        for d in dirs {
            if let Some(pt) = closest_hit(origin, d, &sorted) {
                if pt != origin {
                    hits.push(pt);
                }
            }
        }
    }
    if hits.is_empty() {
        return Vec::new();
    }
    hits.sort_by(|a, b| angle_cmp(origin, *a, *b));
    hits.dedup();
    simplify_collinear(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn seg(ax: i32, az: i32, bx: i32, bz: i32) -> WallSeg {
        WallSeg {
            ax_cm: ax,
            az_cm: az,
            bx_cm: bx,
            bz_cm: bz,
        }
    }
    fn room_walls() -> Vec<WallSeg> {
        vec![
            seg(0, 0, 1000, 0),
            seg(1000, 0, 1000, 1000),
            seg(1000, 1000, 0, 1000),
            seg(0, 1000, 0, 0),
        ]
    }
    #[test]
    fn sight_isovist_square_centre_is_four_corners() {
        let walls = room_walls();
        let origin = [500, 500];
        let poly = isovist(origin, &walls);
        assert_eq!(poly, isovist(origin, &walls));
        assert_eq!(poly.len(), 4);
        assert_eq!(poly, vec![[1000, 1000], [0, 1000], [0, 0], [1000, 0]]);
        assert_eq!(area_cm2(&poly), 1_000_000);
    }
    #[test]
    fn sight_isovist_pillar_adds_shadow_vertices() {
        let origin = [500, 500];
        let empty = isovist(origin, &room_walls());
        assert_eq!(empty.len(), 4);
        let mut full = room_walls();
        full.extend([
            seg(600, 450, 700, 450),
            seg(700, 450, 700, 550),
            seg(700, 550, 600, 550),
            seg(600, 550, 600, 450),
        ]);
        let poly = isovist(origin, &full);
        assert_eq!(poly, isovist(origin, &full));
        assert!(poly.len() > empty.len());
        assert!(poly.contains(&[600, 450]));
        assert!(poly.contains(&[600, 550]));
        assert!(poly.iter().copied().filter(|p| p[0] == 1000).count() >= 4);
        assert_eq!(area_cm2(&empty), 1_000_000);
        let a_full = area_cm2(&poly);
        assert!(a_full < 1_000_000 && a_full > 0);
    }

    #[test]
    fn sight_isovist_closed_room_area_is_room() {
        let room_10m = [[0, 0], [1000, 0], [1000, 1000], [0, 1000]];
        assert_eq!(area_cm2(&room_10m), 1_000_000);
        let room_big = [[0, 0], [10000, 0], [10000, 10000], [0, 10000]];
        assert_eq!(area_cm2(&room_big), 100_000_000);
        let mut reversed = room_10m.to_vec();
        reversed.reverse();
        assert_eq!(area_cm2(&reversed), 1_000_000);
        assert_eq!(area_cm2(&room_10m), area_cm2(&room_10m));
    }

    #[test]
    fn sight_isovist_short_poly_returns_zero_and_replays() {
        let empty: [[i32; 2]; 0] = [];
        assert_eq!(area_cm2(&empty), 0);
        assert_eq!(area_cm2(&[[3, 4]]), 0);
        assert_eq!(area_cm2(&[[0, 0], [1000, 0]]), 0);
        let tri = [[0, 0], [400, 0], [0, 300]];
        assert_eq!(area_cm2(&tri), 60_000);
        let mut flipped = tri.to_vec();
        flipped.reverse();
        assert_eq!(area_cm2(&flipped), 60_000);
        assert_eq!(area_cm2(&tri), area_cm2(&tri));
    }

    #[test]
    fn sight_isovist_crossing_split_is_stable() {
        // GS30 slice: an X crossing splits at (500, 500) into four
        // segments; the split set replays and the sweep sees the same
        // polygon with or without pre-split input.
        let crossing = vec![seg(0, 0, 1000, 1000), seg(0, 1000, 1000, 0)];
        let split = split_crossings(&crossing);
        assert_eq!(split, split_crossings(&crossing));
        assert_eq!(split.len(), 4);
        assert!(split
            .iter()
            .all(|w| (w.ax_cm, w.az_cm) <= (w.bx_cm, w.bz_cm)));
        let mut pts: Vec<[i32; 2]> = Vec::new();
        for w in &split {
            pts.push([w.ax_cm, w.az_cm]);
            pts.push([w.bx_cm, w.bz_cm]);
        }
        assert!(pts.contains(&[500, 500]));
        let origin = [500, 100];
        assert_eq!(isovist(origin, &crossing), isovist(origin, &split));
    }

    #[test]
    fn sight_isovist_open_ground_reports_zero() {
        // Open ground (no walls) has no bounded ring: empty polygon, 0 area.
        let origin = [500, 500];
        let empty: Vec<WallSeg> = Vec::new();
        let poly = isovist(origin, &empty);
        assert!(poly.is_empty(), "open ground must report no polygon");
        assert_eq!(poly, isovist(origin, &empty));
        assert_eq!(area_cm2(&poly), 0);
    }

    #[test]
    fn sight_isovist_origin_on_endpoint_is_skipped_and_stable() {
        // Edge semantics lock: the eye may sit exactly on a wall endpoint.
        // The zero-length ray from the eye to itself is skipped, hits at the
        // eye are dropped, so the ring never contains the eye and still
        // replays twice identically.
        let walls = room_walls();
        let origin = [0, 0];
        let poly = isovist(origin, &walls);
        assert_eq!(poly, isovist(origin, &walls));
        assert!(!poly.contains(&origin), "ring must not contain the eye");
        assert!(area_cm2(&poly) > 0, "corner eye still sees room area");
        let lone = vec![seg(0, 0, 1000, 0)];
        let single = isovist(origin, &lone);
        assert_eq!(single, isovist(origin, &lone));
        assert!(!single.contains(&origin));
    }

    #[test]
    fn sight_isovist_ray_budget_is_three_per_endpoint() {
        // Budget lock: three integer rays per endpoint, two endpoints per
        // wall, saturating. Same count gives the same budget twice.
        assert_eq!(isovist_ray_budget(0), 0);
        assert_eq!(isovist_ray_budget(1), 6);
        assert_eq!(isovist_ray_budget(2), 12);
        assert_eq!(isovist_ray_budget(204), 1224);
        assert_eq!(isovist_ray_budget(204), isovist_ray_budget(204));
        assert_eq!(RAYS_PER_ENDPOINT, 3);
        assert_eq!(isovist_ray_budget(usize::MAX), usize::MAX);
    }

    #[test]
    fn sight_isovist_arena_fixture_fits_ray_budget() {
        // Arena fixture (204 walls) never casts past its ray budget; the
        // sweep replays twice identically. Open ground budgets zero.
        assert_eq!(isovist_ray_budget(0), 0);
        let walls = arena_scale_walls();
        assert_eq!(walls.len(), 204);
        let budget = isovist_ray_budget(walls.len());
        assert_eq!(budget, 1224);
        let origin = [1000, 1000];
        let first = isovist(origin, &walls);
        assert_eq!(first, isovist(origin, &walls));
        assert!(!first.is_empty());
        // Distinct endpoints after split bound the fan; hits never exceed
        // three per distinct endpoint.
        let mut ends: Vec<[i32; 2]> = Vec::with_capacity(walls.len() * 2);
        for w in &walls {
            ends.push([w.ax_cm, w.az_cm]);
            ends.push([w.bx_cm, w.bz_cm]);
        }
        ends.sort_unstable();
        ends.dedup();
        assert!(first.len() <= ends.len() * RAYS_PER_ENDPOINT);
        assert!(first.len() <= budget);
    }

    fn arena_scale_walls() -> Vec<WallSeg> {
        // Deterministic arena scale: a 20 m outer room plus fifty 40 cm
        // pillars (4 walls each) for 204 walls with no crossings.
        let mut walls = room_walls();
        for row in 0..5 {
            for col in 0..10 {
                let x0 = 200 + col * 160;
                let z0 = 200 + row * 160;
                let x1 = x0 + 40;
                let z1 = z0 + 40;
                walls.push(seg(x0, z0, x1, z0));
                walls.push(seg(x1, z0, x1, z1));
                walls.push(seg(x1, z1, x0, z1));
                walls.push(seg(x0, z1, x0, z0));
            }
        }
        walls
    }

    /// Acceptance probe: the 3-rays-per-endpoint integer fan holds arena
    /// scale under 2 ms p95. Setup (walls) is built once outside the sweep
    /// clock, so the p95 measures sweep only; heap-call counting needs the
    /// engine-core test-util manifest row and is not asserted here.
    #[test]
    #[ignore = "timing: isovist budget, run under --perf in release"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn sight_isovist_arena_scale_budget_p95_and_replay() {
        let walls = arena_scale_walls();
        assert!(walls.len() >= 200, "budget needs 200+ walls");
        let origin = [1000, 1000];
        let first = isovist(origin, &walls);
        assert!(!first.is_empty(), "arena sweep must return a ring");
        assert_eq!(first, isovist(origin, &walls));
        let mut micros: Vec<u128> = Vec::with_capacity(21);
        for _ in 0..21 {
            let start = std::time::Instant::now();
            let poly = isovist(origin, &walls);
            let elapsed = start.elapsed().as_micros();
            assert_eq!(poly, first, "sweep drifted mid-budget");
            micros.push(elapsed);
        }
        micros.sort_unstable();
        let p95 = micros[(micros.len() * 95 / 100).min(micros.len() - 1)];
        println!(
            "isovist budget: walls={} p95_us={p95} runs={}",
            walls.len(),
            micros.len()
        );
        assert_eq!(walls.len(), 204, "arena fixture drifted");
        assert!(
            p95 <= 2_000,
            "isovist p95 {p95} us over 2 ms on {} walls",
            walls.len()
        );
    }
}
