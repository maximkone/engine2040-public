//! isovist: integer-cm visibility polygon plus closed-ring area.
//! Owner: spatial crate. Consumer: sight readability gate (next slice).
//! Units: integer centimetres in, square centimetres out.
//! Determinism: integer math only (`i64`/`i128`), sorted walls/endpoints and
//! angles; same inputs give the same order twice. No float, no clock.
//! Errors: none; empty walls or a coordinate past `MAX_COORD_CM` give an
//! empty polygon, short rings return 0.
//! Crossings split at integer-cm points before the sweep; collinear
//! overlaps stay as-is. Fixtures beyond the X crossing are non-crossing.
//! Proof: `parity_*` tests compare the ring with a brute-force segment test.
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

/// Largest coordinate magnitude, in centimetres, the sweep accepts (2^24 cm,
/// 167 km). Inside it every cross product and every hit-point numerator fits
/// the integer width used for it, so the sweep cannot overflow. Outside it
/// [`isovist`] reports an empty ring and [`split_crossings`] returns its
/// input unchanged.
pub const MAX_COORD_CM: i32 = 1 << 24;

fn in_domain(p: [i32; 2]) -> bool {
    p[0].unsigned_abs() <= MAX_COORD_CM.unsigned_abs()
        && p[1].unsigned_abs() <= MAX_COORD_CM.unsigned_abs()
}

fn walls_in_domain(walls: &[WallSeg]) -> bool {
    walls
        .iter()
        .all(|w| in_domain([w.ax_cm, w.az_cm]) && in_domain([w.bx_cm, w.bz_cm]))
}

/// Closed-ring area of a polygon in square centimetres.
///
/// Shoelace over `i128`; returns 0 for fewer than 3 points. A 10 m square
/// (1_000 cm side) reports 1_000_000 cm2; a 10_000 cm side reports
/// 100_000_000 cm2. Sign-independent: clockwise and counter-clockwise
/// rings report the same area, twice identically. An area past `i64::MAX`
/// (only possible for rings wider than [`MAX_COORD_CM`]) saturates.
pub fn area_cm2(poly: &[[i32; 2]]) -> i64 {
    if poly.len() < 3 {
        return 0;
    }
    let mut acc: i128 = 0;
    for (i, point) in poly.iter().enumerate() {
        let next = poly[(i + 1) % poly.len()];
        acc +=
            i128::from(point[0]) * i128::from(next[1]) - i128::from(next[0]) * i128::from(point[1]);
    }
    i64::try_from(acc.abs() / 2).unwrap_or(i64::MAX)
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
    // Numerators reach 2^87 inside the domain: widen before the divide.
    let (tn128, dn128) = (i128::from(tn), i128::from(dn));
    let px = (i128::from(origin[0]) * dn128 + i128::from(dir[0]) * tn128) / dn128;
    let pz = (i128::from(origin[1]) * dn128 + i128::from(dir[1]) * tn128) / dn128;
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
/// Counter-clockwise order of two ray directions from +X, exact in integers.
/// Equal directions compare equal whatever their length.
fn dir_cmp(a: [i64; 2], b: [i64; 2]) -> Ordering {
    let ha = i32::from(!(a[1] > 0 || (a[1] == 0 && a[0] > 0)));
    let hb = i32::from(!(b[1] > 0 || (b[1] == 0 && b[0] > 0)));
    if ha != hb {
        return ha.cmp(&hb);
    }
    // Offset rays reach 2^36 per axis, so the cross product needs 128 bits.
    let turn = i128::from(a[0]) * i128::from(b[1]) - i128::from(a[1]) * i128::from(b[0]);
    0.cmp(&turn)
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
    if n < 2 || !walls_in_domain(walls) {
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
/// hit per ray, sorts the hits by their ray's direction, dedups, and drops
/// straight-edge dust.
/// Same inputs give the same order twice, whatever order or direction the
/// walls come in. Every vertex sits on a wall to the centimetre, and a
/// shadow step lies within 1/1024 rad of the true shadow edge.
///
/// Rooms must be closed: a ray that escapes through a gap adds no vertex.
/// Empty walls, or any coordinate beyond [`MAX_COORD_CM`], give an empty ring.
pub fn isovist(origin: [i32; 2], walls: &[WallSeg]) -> Vec<[i32; 2]> {
    if walls.is_empty() || !in_domain(origin) || !walls_in_domain(walls) {
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
    // Each hit keeps the direction of the ray that found it. The ring is
    // ordered by those exact directions, never by the rounded hit points: a
    // far hit rounded onto an end's own bearing would otherwise swap places
    // with the near end it was cast beside.
    let mut rays: Vec<([i64; 2], [i32; 2])> = Vec::with_capacity(isovist_ray_budget(sorted.len()));
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
                    rays.push((d, pt));
                }
            }
        }
    }
    if rays.is_empty() {
        return Vec::new();
    }
    rays.sort_by(|a, b| dir_cmp(a.0, b.0).then_with(|| a.1.cmp(&b.1)));
    let mut hits: Vec<[i32; 2]> = rays.into_iter().map(|(_, pt)| pt).collect();
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

    // Brute-force parity. The oracle never touches the sweep: a point is
    // seen when the straight line from the eye to it touches no wall, and
    // the ring must contain exactly those points. Two thin bands are
    // skipped because the fan is only exact there by design: within 4 cm of
    // a wall (centimetre rounding) and within 0.002 rad of a wall end
    // (the near-radial shadow step).

    /// Fixed-seed generator so the random rooms replay exactly.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0 >> 33
        }
        fn range(&mut self, lo: i32, hi: i32) -> i32 {
            let span = u64::try_from(hi - lo + 1).expect("hi >= lo");
            lo + i32::try_from(self.next() % span).expect("span fits i32")
        }
    }

    fn v128(p: [i32; 2]) -> [i128; 2] {
        [i128::from(p[0]), i128::from(p[1])]
    }
    fn sub(a: [i128; 2], b: [i128; 2]) -> [i128; 2] {
        [a[0] - b[0], a[1] - b[1]]
    }
    fn cross128(a: [i128; 2], b: [i128; 2]) -> i128 {
        a[0] * b[1] - a[1] * b[0]
    }
    fn orient(a: [i128; 2], b: [i128; 2], c: [i128; 2]) -> i128 {
        cross128(sub(b, a), sub(c, a))
    }
    fn in_box(a: [i128; 2], b: [i128; 2], c: [i128; 2]) -> bool {
        c[0] >= a[0].min(b[0])
            && c[0] <= a[0].max(b[0])
            && c[1] >= a[1].min(b[1])
            && c[1] <= a[1].max(b[1])
    }
    /// Segments `p1p2` and `p3p4` share at least one point.
    fn segments_touch(p1: [i128; 2], p2: [i128; 2], p3: [i128; 2], p4: [i128; 2]) -> bool {
        let d1 = orient(p3, p4, p1);
        let d2 = orient(p3, p4, p2);
        let d3 = orient(p1, p2, p3);
        let d4 = orient(p1, p2, p4);
        if ((d1 > 0 && d2 < 0) || (d1 < 0 && d2 > 0)) && ((d3 > 0 && d4 < 0) || (d3 < 0 && d4 > 0))
        {
            return true;
        }
        (d1 == 0 && in_box(p3, p4, p1))
            || (d2 == 0 && in_box(p3, p4, p2))
            || (d3 == 0 && in_box(p1, p2, p3))
            || (d4 == 0 && in_box(p1, p2, p4))
    }
    fn sight_blocked(eye: [i32; 2], p: [i32; 2], walls: &[WallSeg]) -> bool {
        walls.iter().any(|w| {
            segments_touch(
                v128(eye),
                v128(p),
                v128([w.ax_cm, w.az_cm]),
                v128([w.bx_cm, w.bz_cm]),
            )
        })
    }
    /// Even-odd test: casts a ray toward +X and counts edge crossings.
    fn ring_contains(ring: &[[i32; 2]], p: [i32; 2]) -> bool {
        let (px, pz) = (i128::from(p[0]), i128::from(p[1]));
        let mut inside = false;
        for (i, corner) in ring.iter().enumerate() {
            let a = v128(*corner);
            let b = v128(ring[(i + 1) % ring.len()]);
            if (a[1] > pz) != (b[1] > pz) {
                let lhs = (px - a[0]) * (b[1] - a[1]);
                let rhs = (b[0] - a[0]) * (pz - a[1]);
                let crossing_is_right = if b[1] > a[1] { lhs < rhs } else { lhs > rhs };
                if crossing_is_right {
                    inside = !inside;
                }
            }
        }
        inside
    }
    /// True when `p` sits within `band` cm of wall `w`.
    fn near_wall(p: [i32; 2], w: &WallSeg, band: i128) -> bool {
        let a = v128([w.ax_cm, w.az_cm]);
        let b = v128([w.bx_cm, w.bz_cm]);
        let e = sub(b, a);
        let q = sub(v128(p), a);
        let len2 = e[0] * e[0] + e[1] * e[1];
        let along = q[0] * e[0] + q[1] * e[1];
        if len2 == 0 || along <= 0 {
            return q[0] * q[0] + q[1] * q[1] <= band * band;
        }
        if along >= len2 {
            let r = sub(v128(p), b);
            return r[0] * r[0] + r[1] * r[1] <= band * band;
        }
        let side = cross128(e, q);
        side * side <= band * band * len2
    }
    /// True when the eye-to-`p` direction is within 0.002 rad of the
    /// direction to any wall end.
    fn in_end_sliver(eye: [i32; 2], p: [i32; 2], walls: &[WallSeg]) -> bool {
        let d = sub(v128(p), v128(eye));
        let dd = d[0] * d[0] + d[1] * d[1];
        walls.iter().any(|w| {
            [[w.ax_cm, w.az_cm], [w.bx_cm, w.bz_cm]].iter().any(|end| {
                let v = sub(v128(*end), v128(eye));
                let vv = v[0] * v[0] + v[1] * v[1];
                let side = cross128(d, v);
                vv != 0 && d[0] * v[0] + d[1] * v[1] > 0 && side * side * 250_000 <= dd * vv
            })
        })
    }

    struct Parity {
        checked: usize,
        sampled: usize,
        mismatches: Vec<[i32; 2]>,
    }

    /// Compare the ring with the brute-force oracle on a grid over
    /// `lo..=hi`. Also locks replay x2 and that every vertex sits on a wall.
    fn parity(eye: [i32; 2], walls: &[WallSeg], lo: [i32; 2], hi: [i32; 2]) -> Parity {
        let ring = isovist(eye, walls);
        assert_eq!(ring, isovist(eye, walls), "ring replays x2");
        assert!(ring.len() >= 3, "a closed room gives a ring: {ring:?}");
        let pieces = split_crossings(walls);
        for v in &ring {
            assert!(
                pieces.iter().any(|w| near_wall(*v, w, 2)),
                "vertex {v:?} floats off every wall"
            );
        }
        let mut out = Parity {
            checked: 0,
            sampled: 0,
            mismatches: Vec::new(),
        };
        let step = 37_usize;
        for z in (lo[1] + 13..=hi[1]).step_by(step) {
            for x in (lo[0] + 13..=hi[0]).step_by(step) {
                let p = [x, z];
                out.sampled += 1;
                if p == eye
                    || pieces.iter().any(|w| near_wall(p, w, 4))
                    || in_end_sliver(eye, p, &pieces)
                {
                    continue;
                }
                out.checked += 1;
                if !sight_blocked(eye, p, walls) != ring_contains(&ring, p) {
                    out.mismatches.push(p);
                }
            }
        }
        out
    }

    fn assert_parity(label: &str, eye: [i32; 2], walls: &[WallSeg], lo: [i32; 2], hi: [i32; 2]) {
        let got = parity(eye, walls, lo, hi);
        assert!(
            got.mismatches.is_empty(),
            "{label} eye {eye:?}: ring and brute force disagree at {:?} ({} of {} checked)",
            &got.mismatches[..got.mismatches.len().min(6)],
            got.mismatches.len(),
            got.checked
        );
        assert!(
            got.checked * 10 >= got.sampled * 8,
            "{label} eye {eye:?}: only {} of {} samples were checked",
            got.checked,
            got.sampled
        );
    }

    fn ring_walls(points: &[[i32; 2]]) -> Vec<WallSeg> {
        let mut walls = Vec::with_capacity(points.len());
        for (i, a) in points.iter().enumerate() {
            let b = points[(i + 1) % points.len()];
            walls.push(seg(a[0], a[1], b[0], b[1]));
        }
        walls
    }

    fn pillar(x0: i32, z0: i32, x1: i32, z1: i32) -> Vec<WallSeg> {
        ring_walls(&[[x0, z0], [x1, z0], [x1, z1], [x0, z1]])
    }

    #[test]
    fn parity_hand_built_rooms_match_brute_force() {
        let mut pillar_room = room_walls();
        pillar_room.extend(pillar(600, 450, 700, 550));
        let l_room = ring_walls(&[
            [0, 0],
            [1200, 0],
            [1200, 500],
            [500, 500],
            [500, 1200],
            [0, 1200],
        ]);
        let mut busy_room = ring_walls(&[[0, 0], [1400, 0], [1400, 900], [0, 900]]);
        busy_room.extend(pillar(300, 300, 380, 380));
        busy_room.extend(pillar(900, 500, 1000, 560));
        busy_room.push(seg(600, 100, 600, 600));
        let mut x_room = room_walls();
        x_room.push(seg(100, 100, 701, 503));
        x_room.push(seg(100, 600, 700, 150));
        // Each room is probed from several eyes over its box plus a margin.
        let probe = |label: &str, walls: &[WallSeg], eyes: &[[i32; 2]], size: [i32; 2]| {
            for eye in eyes {
                assert_parity(label, *eye, walls, [-60, -60], [size[0] + 60, size[1] + 60]);
            }
        };
        let eyes = [[500, 500], [150, 150], [850, 850], [650, 300]];
        probe("pillar", &pillar_room, &eyes, [1000, 1000]);
        let eyes = [[250, 250], [1000, 250], [250, 1000], [100, 1100]];
        probe("ell", &l_room, &eyes, [1200, 1200]);
        let eyes = [[150, 150], [700, 700], [1250, 800], [700, 650]];
        probe("busy", &busy_room, &eyes, [1400, 900]);
        let eyes = [[50, 50], [500, 900], [900, 300], [400, 330]];
        probe("cross", &x_room, &eyes, [1000, 1000]);
    }

    /// A closed 1000 x 1000 room with a few random walls, and an eye that
    /// is not within 8 cm of any of them.
    fn random_room(seed: u64) -> (Vec<WallSeg>, [i32; 2]) {
        let mut rng = Lcg(seed);
        let mut walls = room_walls();
        let extra = 2 + usize::try_from(rng.next() % 7).expect("small");
        for _ in 0..extra {
            let a = [rng.range(60, 940), rng.range(60, 940)];
            let mut b = [
                (a[0] + rng.range(-300, 300)).clamp(20, 980),
                (a[1] + rng.range(-300, 300)).clamp(20, 980),
            ];
            if rng.next().is_multiple_of(3) {
                b[1] = a[1];
            }
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            if dx * dx + dz * dz >= 400 {
                walls.push(seg(a[0], a[1], b[0], b[1]));
            }
        }
        let pieces = split_crossings(&walls);
        loop {
            let eye = [rng.range(40, 960), rng.range(40, 960)];
            if !pieces.iter().any(|w| near_wall(eye, w, 8)) {
                return (walls, eye);
            }
        }
    }

    #[test]
    fn isovist_far_hit_rounded_onto_an_end_bearing_stays_before_that_end() {
        // The ray just above the free wall's top end meets the left wall at
        // z 300.7, which rounds onto the end's own bearing (the end is at
        // (600, 600), the eye at (700, 650)). Ordering the rounded hits by
        // bearing then put the near end first and folded the ring over
        // itself; ordering by the casting ray keeps the far hit first.
        let mut walls = ring_walls(&[[0, 0], [1400, 0], [1400, 900], [0, 900]]);
        walls.push(seg(600, 100, 600, 600));
        let eye = [700, 650];
        let ring = isovist(eye, &walls);
        assert_eq!(ring, isovist(eye, &walls));
        let far = ring.iter().position(|p| *p == [0, 300]).expect("far hit");
        let near = ring
            .iter()
            .position(|p| *p == [600, 600])
            .expect("near end");
        assert_eq!(near, far + 1, "far hit comes first: {ring:?}");
        assert_eq!(
            ring,
            vec![
                [1400, 900],
                [0, 900],
                [0, 300],
                [600, 600],
                [600, 100],
                [582, 0],
                [1400, 0]
            ]
        );
    }

    #[test]
    fn parity_random_rooms_match_brute_force() {
        for seed in 1..=24_u64 {
            let (walls, eye) = random_room(seed);
            assert_parity(
                &format!("seed {seed}"),
                eye,
                &walls,
                [-40, -40],
                [1040, 1040],
            );
        }
    }

    #[test]
    fn isovist_ignores_wall_order_direction_and_junk() {
        for seed in [3_u64, 9, 17] {
            let (walls, eye) = random_room(seed);
            let want = isovist(eye, &walls);
            assert!(want.len() >= 4);
            let mut turned: Vec<WallSeg> = walls
                .iter()
                .rev()
                .map(|w| seg(w.bx_cm, w.bz_cm, w.ax_cm, w.az_cm))
                .collect();
            assert_eq!(isovist(eye, &turned), want, "seed {seed}: reversed walls");
            turned.rotate_left(3);
            assert_eq!(isovist(eye, &turned), want, "seed {seed}: rotated walls");
            // A repeated wall and a zero-length wall add no sight line.
            turned.push(turned[2]);
            turned.push(seg(333, 777, 333, 777));
            assert_eq!(
                isovist(eye, &turned),
                want,
                "seed {seed}: duplicate and dot"
            );
        }
    }

    #[test]
    fn isovist_far_coordinates_stay_exact() {
        // The widest legal room: every product here overflowed `i64` before
        // the hit point was widened.
        let m = MAX_COORD_CM;
        let room = ring_walls(&[[-m, -m], [m, -m], [m, m], [-m, m]]);
        let ring = isovist([0, 0], &room);
        assert_eq!(ring, isovist([0, 0], &room));
        assert_eq!(ring, vec![[m, m], [-m, m], [-m, -m], [m, -m]]);
        assert_eq!(area_cm2(&ring), 1_i64 << 50);
        let mut with_pillar = room.clone();
        with_pillar.extend(pillar(m / 2, -100, m / 2 + 100, 100));
        let shaded = isovist([0, 0], &with_pillar);
        assert_eq!(shaded, isovist([0, 0], &with_pillar));
        assert!(shaded.len() > 4, "the pillar casts a shadow: {shaded:?}");
        assert!(shaded.contains(&[m / 2, -100]) && shaded.contains(&[m / 2, 100]));
        assert!(area_cm2(&shaded) < 1_i64 << 50);
        // A cross at the far corner splits on an integer point inside the box.
        let split = split_crossings(&[seg(-m, -m, m, m), seg(-m, m, m, -m)]);
        assert_eq!(split.len(), 4);
        assert!(split
            .iter()
            .all(|w| w.ax_cm.abs() <= m && w.bz_cm.abs() <= m));
    }

    #[test]
    fn isovist_outside_the_domain_is_empty_not_a_panic() {
        let m = MAX_COORD_CM;
        let room = ring_walls(&[[-m, -m], [m, -m], [m, m], [-m, m]]);
        assert!(!isovist([0, 0], &room).is_empty());
        // One cm past the bound, an extreme eye and an extreme wall.
        assert!(isovist([m + 1, 0], &room).is_empty());
        assert!(isovist([0, i32::MIN], &room).is_empty());
        let wide = ring_walls(&[[i32::MIN, i32::MIN], [i32::MAX, i32::MIN], [0, i32::MAX]]);
        assert!(isovist([0, 0], &wide).is_empty());
        assert_eq!(isovist([0, 0], &wide), isovist([0, 0], &wide));
        // `split_crossings` hands out-of-domain walls back untouched.
        let cross = [
            seg(i32::MIN, i32::MIN, i32::MAX, i32::MAX),
            seg(i32::MIN, i32::MAX, i32::MAX, i32::MIN),
        ];
        assert_eq!(split_crossings(&cross), cross.to_vec());
        // The widest ring a caller can build saturates instead of wrapping.
        let huge = [
            [i32::MIN, i32::MIN],
            [i32::MAX, i32::MIN],
            [i32::MAX, i32::MAX],
            [i32::MIN, i32::MAX],
        ];
        assert_eq!(area_cm2(&huge), i64::MAX);
        assert_eq!(area_cm2(&huge), area_cm2(&huge));
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
