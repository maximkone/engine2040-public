//! building (HC2, docs/SPRINT-34-hero-city.md): one lot becomes massing. A podium up to the
//! street wall, then tiers that step in toward the roof (Stone and Deco, the wedding cake), a
//! straight shaft on a podium (Glass), a box with a cornice (Brick); a water tank on short Brick,
//! a spire on tall Deco. Flat-shaded prisms with UVs in bays (3 m) and floors (3.6 m) counted
//! from the ground, and one axis box per tier for cuts, swing anchors and hits. Presentation
//! only, seeded. Units: meters; y is up; x and z are the plan coordinates of the lot.

use super::super::num::{cross, f32_of, sub, u32_of, unit, unit_f32};
use super::{Lot, Style};
use engine_core::hash::GOLDEN_GAMMA;
use engine_core::splitmix64;
use std::f32::consts::TAU;

/// One bay (window column) along a wall, meters: the u texture repeat.
const BAY_M: f32 = 3.0;
/// One floor, meters: the v texture repeat.
const FLOOR_M: f32 = 3.6;
/// The lowest street-wall (podium) height, meters.
const PODIUM_MIN_M: f32 = 25.0;
/// The highest street-wall (podium) height, meters.
const PODIUM_MAX_M: f32 = 45.0;
/// The shortest stepped tier, meters; a shorter remainder joins the tier before it.
const MIN_TIER_M: f32 = 30.0;
/// The tallest stepped tier before the next, meters.
const TIER_MAX_M: f32 = 80.0;
/// The smallest inset per side for a stepped tier, meters.
const INSET_MIN_M: f32 = 3.0;
/// The largest inset per side for a stepped tier, meters.
const INSET_MAX_M: f32 = 6.0;
/// The shortest edge a stepped tier may have, meters: below it the tiers stop.
const MIN_SIDE_M: f32 = 8.0;
/// The Glass podium: one 12 m floor-band, meters.
const GLASS_PODIUM_M: f32 = 12.0;
/// How far the Glass shaft sits in from its podium per side, meters.
const GLASS_INSET_M: f32 = 2.0;
/// The Brick cornice thickness, meters.
const CORNICE_M: f32 = 0.6;
/// How far the Brick cornice sticks out past the wall, meters.
const CORNICE_OUT_M: f32 = 0.5;
/// Brick under this height gets a water tank, meters.
const TANK_MAX_H_M: f32 = 60.0;
/// The water tank's base box height, meters.
const TANK_BASE_M: f32 = 1.0;
/// The water tank's base box half-width, meters.
const TANK_BASE_HALF_M: f32 = 2.0;
/// The water tank drum's radius, meters.
const TANK_RADIUS_M: f32 = 3.0;
/// The water tank drum's height, meters.
const TANK_DRUM_M: f32 = 5.0;
/// The drum's side count.
const TANK_SIDES: usize = 8;
/// How far the roof must reach around the tank's centre, meters.
const TANK_CLEAR_M: f32 = 3.5;
/// Deco over this height gets a spire, meters.
const SPIRE_MIN_H_M: f32 = 200.0;
/// The spire's height as a share of the building's height.
const SPIRE_FRAC: f32 = 0.15;
/// The spire's base half-width, meters.
const SPIRE_HALF_M: f32 = 3.0;
/// How far the roof must reach around the spire's centre, meters.
const SPIRE_CLEAR_M: f32 = 3.0;

/// One mesh part of a building: a prism, a cap or a roof piece, flat-shaded (each face owns its
/// vertices).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Part {
    /// The look family: the facade tile set this part uses.
    pub style: Style,
    /// Vertex positions `[x, y, z]` in world meters.
    pub positions: Vec<[f32; 3]>,
    /// Outward unit normals, one per vertex.
    pub normals: Vec<[f32; 3]>,
    /// Texture coordinates: u in bays, v in floors counted from the ground; caps `[x/3, z/3]`.
    pub uvs: Vec<[f32; 2]>,
    /// Triangle list into the vertex arrays.
    pub indices: Vec<u32>,
}

impl Part {
    /// An empty part in `style`.
    fn new(style: Style) -> Self {
        Self {
            style,
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// Appends a flat triangle with normal `n`; its corners wind so the normal is the face side.
    fn tri(&mut self, corners: [[f32; 3]; 3], uvs: [[f32; 2]; 3], n: [f32; 3]) {
        let base = u32_of(self.positions.len());
        for (p, uv) in corners.into_iter().zip(uvs) {
            self.positions.push(p);
            self.normals.push(n);
            self.uvs.push(uv);
        }
        self.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }

    /// Appends a flat quad `corners` (in order A, B, C, D: a wall from A-B up to C-D) with
    /// normal `n`, as two triangles A-C-B and A-D-C.
    fn quad(&mut self, corners: [[f32; 3]; 4], uvs: [[f32; 2]; 4], n: [f32; 3]) {
        let base = u32_of(self.positions.len());
        for (p, uv) in corners.into_iter().zip(uvs) {
            self.positions.push(p);
            self.normals.push(n);
            self.uvs.push(uv);
        }
        self.indices
            .extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
    }
}

/// One building: its mesh parts and one axis box per tier.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Building {
    /// The mesh parts: one per tier, plus Brick's cornice and tank and Deco's spire.
    pub parts: Vec<Part>,
    /// One axis box `(min, max)` per tier, in world meters.
    pub boxes: Vec<([f32; 3], [f32; 3])>,
}

/// One stacked section of a lot's massing: a prism of footprint between two heights.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Tier {
    /// Footprint corners `[x, z]`, counter-clockwise seen from above; meters.
    pub footprint: Vec<[f32; 2]>,
    /// Bottom height above the ground, meters.
    pub base_m: f32,
    /// Top height above the ground, meters.
    pub top_m: f32,
}

/// A seeded word stream: each draw advances a SplitMix64 state by the golden gamma.
struct Seq(u64);

impl Seq {
    /// The next 64-bit word.
    fn word(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(GOLDEN_GAMMA);
        splitmix64(self.0)
    }

    /// A seeded value in `lo..hi`.
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * unit_f32(self.word())
    }
}

/// Each edge of `poly` as `(start, end)`, closing back to the first corner.
fn edges(poly: &[[f32; 2]]) -> impl Iterator<Item = ([f32; 2], [f32; 2])> + '_ {
    poly.iter()
        .zip(poly.iter().cycle().skip(1))
        .map(|(a, b)| (*a, *b))
}

/// The length of the edge `a` to `b`, meters.
fn edge_len(a: [f32; 2], b: [f32; 2]) -> f32 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

/// Twice the signed plan area of `poly`: positive when counter-clockwise.
fn twice_area(poly: &[[f32; 2]]) -> f32 {
    edges(poly).map(|(a, b)| a[0] * b[1] - b[0] * a[1]).sum()
}

/// `poly` counter-clockwise seen from above (reversed when it is clockwise).
fn ccw(poly: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut out = poly.to_vec();
    if twice_area(poly) < 0.0 {
        out.reverse();
    }
    out
}

/// The unit normal of the edge `a` to `b` pointing into a counter-clockwise polygon, or None
/// for a zero-length edge.
fn inward(a: [f32; 2], b: [f32; 2]) -> Option<[f32; 2]> {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let len = dx.hypot(dz);
    (len > 1e-6).then(|| [-dz / len, dx / len])
}

/// The mean of the corners of `poly` (the origin for none).
fn mean(poly: &[[f32; 2]]) -> [f32; 2] {
    if poly.is_empty() {
        return [0.0; 2];
    }
    let s = poly
        .iter()
        .fold([0.0_f32; 2], |acc, p| [acc[0] + p[0], acc[1] + p[1]]);
    let k = 1.0 / f32_of(poly.len());
    [s[0] * k, s[1] * k]
}

/// The distance from the mean of `poly` to its nearest edge line, meters: how wide a roof
/// piece can be centred on it.
fn clearance(poly: &[[f32; 2]]) -> f32 {
    let m = mean(poly);
    edges(poly)
        .filter_map(|(a, b)| inward(a, b).map(|n| n[0] * (m[0] - a[0]) + n[1] * (m[1] - a[1])))
        .fold(f32::INFINITY, f32::min)
}

/// Corner `i` of an inset polygon: where the offset line of edge `i - 1` meets that of edge `i`.
/// `lines` holds each edge's inward normal and its offset line constant.
fn corner(p: [f32; 2], lines: &[([f32; 2], f32)], i: usize, d: f32) -> [f32; 2] {
    let n = lines.len();
    let (n1, c1) = lines[(i + n - 1) % n];
    let (n2, c2) = lines[i];
    let det = n1[0] * n2[1] - n1[1] * n2[0];
    if det.abs() < 1e-6 {
        return [p[0] + n2[0] * d, p[1] + n2[1] * d];
    }
    [
        (c1 * n2[1] - n1[1] * c2) / det,
        (n1[0] * c2 - c1 * n2[0]) / det,
    ]
}

/// `poly` with every edge moved `d` meters inward (outward when negative), a convex polygon's
/// offset. None when a side ends shorter than `min_side` meters or the shape inverts (the
/// offset ran past the middle, as a thin wedge does).
fn inset(poly: &[[f32; 2]], d: f32, min_side: f32) -> Option<Vec<[f32; 2]>> {
    if poly.len() < 3 {
        return None;
    }
    let lines = edges(poly)
        .map(|(a, b)| inward(a, b).map(|n| (n, n[0] * a[0] + n[1] * a[1] + d)))
        .collect::<Option<Vec<_>>>()?;
    let out: Vec<[f32; 2]> = poly
        .iter()
        .enumerate()
        .map(|(i, p)| corner(*p, &lines, i, d))
        .collect();
    let finite = out.iter().all(|p| p[0].is_finite() && p[1].is_finite());
    let sides_ok = edges(&out).all(|(a, b)| edge_len(a, b) >= min_side);
    if finite && sides_ok && twice_area(&out) > 0.0 {
        Some(out)
    } else {
        None
    }
}

/// The stacked sections of `lot`, bottom first; each footprint lies inside the one below.
/// Brick is one box (its cornice and tank are roof pieces, not tiers); Glass is a podium and a
/// shaft; Stone and Deco step in until the footprint gets too small, then rise as a shaft.
pub(super) fn tiers(lot: &Lot) -> Vec<Tier> {
    let poly = ccw(&lot.footprint);
    let h = lot.height_m;
    if poly.len() < 3 || twice_area(&poly) <= 0.0 || h <= 0.0 || h.is_nan() {
        return Vec::new();
    }
    let mut seq = Seq(lot.seed);
    match lot.style {
        Style::Brick => brick_tiers(poly, h),
        Style::Glass => glass_tiers(poly, h),
        Style::Stone | Style::Deco => stepped_tiers(poly, h, &mut seq),
    }
}

/// Brick: one box up to the cornice's underside (the cornice takes the top 0.6 m).
fn brick_tiers(poly: Vec<[f32; 2]>, h: f32) -> Vec<Tier> {
    let top_m = if h > CORNICE_M { h - CORNICE_M } else { h };
    vec![Tier {
        footprint: poly,
        base_m: 0.0,
        top_m,
    }]
}

/// Glass: a 12 m podium, then a straight shaft inset 2 m per side to the roof.
fn glass_tiers(poly: Vec<[f32; 2]>, h: f32) -> Vec<Tier> {
    let podium_top = h.min(GLASS_PODIUM_M);
    let mut out = vec![Tier {
        footprint: poly.clone(),
        base_m: 0.0,
        top_m: podium_top,
    }];
    if h > podium_top {
        let shaft = inset(&poly, GLASS_INSET_M, 0.0).unwrap_or_else(|| poly.clone());
        out.push(Tier {
            footprint: shaft,
            base_m: podium_top,
            top_m: h,
        });
    }
    out
}

/// Stone and Deco: a podium up to the street wall, then tiers each inset 3 to 6 m per side
/// and 30 to 80 m tall, until a side would fall under 8 m; the last tier then rises to the roof.
fn stepped_tiers(poly: Vec<[f32; 2]>, h: f32, seq: &mut Seq) -> Vec<Tier> {
    let drawn = seq.range(PODIUM_MIN_M, PODIUM_MAX_M);
    let podium_top = if h - drawn < MIN_TIER_M {
        h
    } else {
        drawn.min(h)
    };
    let mut cur = poly.clone();
    let mut out = vec![Tier {
        footprint: poly,
        base_m: 0.0,
        top_m: podium_top,
    }];
    let mut y = podium_top;
    while h - y >= MIN_TIER_M {
        let Some(next) = inset(&cur, seq.range(INSET_MIN_M, INSET_MAX_M), MIN_SIDE_M) else {
            break;
        };
        let top = (y + seq.range(MIN_TIER_M, TIER_MAX_M)).min(h);
        let top = if h - top < MIN_TIER_M { h } else { top };
        out.push(Tier {
            footprint: next.clone(),
            base_m: y,
            top_m: top,
        });
        cur = next;
        y = top;
    }
    if let Some(last) = out.last_mut() {
        last.top_m = h;
    }
    out
}

/// The axis box of `poly` between heights `y0` and `y1` (world meters).
fn axis_box(poly: &[[f32; 2]], y0: f32, y1: f32) -> ([f32; 3], [f32; 3]) {
    let mut lo = [f32::INFINITY; 2];
    let mut hi = [f32::NEG_INFINITY; 2];
    for p in poly {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    ([lo[0], y0, lo[1]], [hi[0], y1, hi[1]])
}

/// Fan-caps a ring of world points: up (+y, a top cap) or down (-y, a bottom cap). `ring` runs
/// counter-clockwise seen from above; the cap uses `[x/3, z/3]`.
fn cap(part: &mut Part, ring: &[[f32; 3]], up: bool) {
    let Some((&r0, rest)) = ring.split_first() else {
        return;
    };
    let n = if up {
        [0.0, 1.0, 0.0]
    } else {
        [0.0, -1.0, 0.0]
    };
    for pair in rest.windows(2) {
        if let [p, q] = pair {
            let corners = if up { [r0, *q, *p] } else { [r0, *p, *q] };
            let uvs = corners.map(|c| [c[0] / BAY_M, c[2] / BAY_M]);
            part.tri(corners, uvs, n);
        }
    }
}

/// A prism of `poly` between `base_m` and `top_m` meters above the ground: outward walls, a top
/// cap, and a bottom cap only when `cap_bottom` (a piece hanging over open air).
fn prism(
    poly: &[[f32; 2]],
    base_m: f32,
    top_m: f32,
    ground_y: f32,
    style: Style,
    cap_bottom: bool,
) -> Part {
    let mut part = Part::new(style);
    let (y0, y1) = (ground_y + base_m, ground_y + top_m);
    let (v0, v1) = (base_m / FLOOR_M, top_m / FLOOR_M);
    for (a, b) in edges(poly) {
        let n = unit([b[1] - a[1], 0.0, -(b[0] - a[0])]);
        let u1 = edge_len(a, b) / BAY_M;
        let corners = [
            [a[0], y0, a[1]],
            [b[0], y0, b[1]],
            [b[0], y1, b[1]],
            [a[0], y1, a[1]],
        ];
        part.quad(corners, [[0.0, v0], [u1, v0], [u1, v1], [0.0, v1]], n);
    }
    let top: Vec<[f32; 3]> = poly.iter().map(|p| [p[0], y1, p[1]]).collect();
    cap(&mut part, &top, true);
    if cap_bottom {
        let bottom: Vec<[f32; 3]> = poly.iter().map(|p| [p[0], y0, p[1]]).collect();
        cap(&mut part, &bottom, false);
    }
    part
}

/// The 4 m square base box and the 8-sided, 3 m radius drum of a water tank centred on `c`.
fn water_tank(c: [f32; 2], roof_m: f32, ground_y: f32) -> Vec<Part> {
    let h = TANK_BASE_HALF_M;
    let base: Vec<[f32; 2]> = [[-h, -h], [h, -h], [h, h], [-h, h]]
        .iter()
        .map(|o| [c[0] + o[0], c[1] + o[1]])
        .collect();
    let drum: Vec<[f32; 2]> = (0..TANK_SIDES)
        .map(|i| {
            let a = TAU * f32_of(i) / f32_of(TANK_SIDES);
            [
                c[0] + TANK_RADIUS_M * a.cos(),
                c[1] + TANK_RADIUS_M * a.sin(),
            ]
        })
        .collect();
    let drum_base = roof_m + TANK_BASE_M;
    vec![
        prism(&base, roof_m, drum_base, ground_y, Style::Brick, false),
        prism(
            &drum,
            drum_base,
            drum_base + TANK_DRUM_M,
            ground_y,
            Style::Brick,
            false,
        ),
    ]
}

/// A 4-sided pyramid spire on the roof centred on `c`: a 6 m square base and a height of 15% of
/// the building's height.
fn spire(c: [f32; 2], roof_m: f32, ground_y: f32) -> Part {
    let h = SPIRE_HALF_M;
    let y = ground_y + roof_m;
    let ring = [[-h, -h], [h, -h], [h, h], [-h, h]].map(|o| [c[0] + o[0], y, c[1] + o[1]]);
    let apex = [c[0], y + SPIRE_FRAC * roof_m, c[1]];
    let mut part = Part::new(Style::Deco);
    let (v0, v1) = (roof_m / FLOOR_M, (roof_m + SPIRE_FRAC * roof_m) / FLOOR_M);
    for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
        let n = unit(cross(sub(apex, *a), sub(*b, *a)));
        let u1 = ((b[0] - a[0]).hypot(b[2] - a[2])) / BAY_M;
        part.tri([*a, apex, *b], [[0.0, v0], [0.5, v1], [u1, v0]], n);
    }
    part
}

/// The roof pieces on the last tier: Brick's cornice and water tank (a tank under 60 m, on a roof
/// wide enough) and Deco's spire (over 200 m, on a roof wide enough).
fn roof_parts(lot: &Lot, top: &Tier, ground_y: f32, parts: &mut Vec<Part>) {
    let cornice = lot.style == Style::Brick && lot.height_m > CORNICE_M;
    let roof_m = if cornice {
        top.top_m + CORNICE_M
    } else {
        top.top_m
    };
    match lot.style {
        Style::Brick => {
            if cornice {
                if let Some(out) = inset(&top.footprint, -CORNICE_OUT_M, 0.0) {
                    parts.push(prism(&out, top.top_m, roof_m, ground_y, Style::Brick, true));
                }
            }
            if lot.height_m < TANK_MAX_H_M && clearance(&top.footprint) >= TANK_CLEAR_M {
                parts.extend(water_tank(mean(&top.footprint), roof_m, ground_y));
            }
        }
        Style::Deco => {
            if lot.height_m > SPIRE_MIN_H_M && clearance(&top.footprint) >= SPIRE_CLEAR_M {
                parts.push(spire(mean(&top.footprint), roof_m, ground_y));
            }
        }
        Style::Stone | Style::Glass => {}
    }
}

/// The massing of `lot` on the ground at `ground_y` (world meters): one part and one box per
/// tier, plus the roof pieces. Empty for a lot without area or height.
pub(super) fn massing(lot: &Lot, ground_y: f32) -> Building {
    let stack = tiers(lot);
    let mut parts = Vec::with_capacity(stack.len() + 2);
    let mut boxes = Vec::with_capacity(stack.len());
    for tier in &stack {
        parts.push(prism(
            &tier.footprint,
            tier.base_m,
            tier.top_m,
            ground_y,
            lot.style,
            false,
        ));
        boxes.push(axis_box(
            &tier.footprint,
            ground_y + tier.base_m,
            ground_y + tier.top_m,
        ));
    }
    if let Some(top) = stack.last() {
        roof_parts(lot, top, ground_y, &mut parts);
    }
    Building { parts, boxes }
}

#[cfg(test)]
mod tests {
    use super::super::super::num::{dot, idx};
    use super::*;

    /// A counter-clockwise rectangle `w` by `d` meters at the origin.
    fn rect_lot(w: f32, d: f32, height_m: f32, style: Style, seed: u64) -> Lot {
        Lot {
            footprint: vec![[0.0, 0.0], [w, 0.0], [w, d], [0.0, d]],
            height_m,
            style,
            seed,
        }
    }

    /// A counter-clockwise wedge (a triangle cut by the diagonal) at the origin.
    fn wedge_lot(height_m: f32, style: Style, seed: u64) -> Lot {
        Lot {
            footprint: vec![[0.0, 0.0], [30.0, 0.0], [0.0, 40.0]],
            height_m,
            style,
            seed,
        }
    }

    /// Rectangles of three sizes and a wedge, every style, heights 10 to 380 m in 5 m steps,
    /// four seeds each.
    fn sample_lots() -> Vec<Lot> {
        let mut lots = Vec::new();
        for style in Style::ALL {
            for seed in 0..4u64 {
                for step in 0..75usize {
                    let height = 10.0 + 5.0 * f32_of(step);
                    lots.push(rect_lot(12.0, 12.0, height, style, seed));
                    lots.push(rect_lot(40.0, 40.0, height, style, seed));
                    lots.push(rect_lot(12.0, 40.0, height, style, seed));
                    lots.push(wedge_lot(height, style, seed));
                }
            }
        }
        lots
    }

    /// Whether `p` lies inside the counter-clockwise polygon `poly`, to 1 cm.
    fn inside(poly: &[[f32; 2]], p: [f32; 2]) -> bool {
        for (a, b) in edges(poly) {
            let Some(n) = inward(a, b) else {
                return false;
            };
            if n[0] * (p[0] - a[0]) + n[1] * (p[1] - a[1]) < -0.01 {
                return false;
            }
        }
        true
    }

    #[test]
    /// Each tier's corners sit inside the tier below, the tiers stack without gaps, and the
    /// stepped sides keep 8 m or more.
    fn tiers_sit_inside_the_one_below() {
        for lot in sample_lots() {
            let stack = tiers(&lot);
            assert!(!stack.is_empty(), "no tiers for {lot:?}");
            for tier in &stack {
                assert!(tier.top_m > tier.base_m, "empty tier in {lot:?}");
            }
            for pair in stack.windows(2) {
                if let [below, above] = pair {
                    for p in &above.footprint {
                        assert!(inside(&below.footprint, *p), "corner outside in {lot:?}");
                    }
                    assert!((above.base_m - below.top_m).abs() < 1e-4);
                    if matches!(lot.style, Style::Stone | Style::Deco) {
                        for (a, b) in edges(&above.footprint) {
                            assert!(edge_len(a, b) >= MIN_SIDE_M - 1e-3, "short side in {lot:?}");
                        }
                    }
                }
            }
            if lot.style != Style::Brick {
                let top = stack.last().map_or(0.0, |t| t.top_m);
                assert!(
                    (top - lot.height_m).abs() < 1e-3,
                    "roof not at height for {lot:?}"
                );
            }
        }
    }

    #[test]
    /// Walls face out of the lot, every triangle's winding agrees with its stored normal, and
    /// top caps face up.
    fn shells_face_out_and_caps_face_up() {
        for lot in sample_lots() {
            let building = massing(&lot, 0.0);
            let centre = mean(&lot.footprint);
            let rect = lot.footprint.len() == 4;
            for part in &building.parts {
                for tri in part.indices.chunks(3) {
                    let &[i, j, k] = tri else {
                        continue;
                    };
                    let p0 = part.positions[idx(i)];
                    let p1 = part.positions[idx(j)];
                    let p2 = part.positions[idx(k)];
                    let g = cross(sub(p1, p0), sub(p2, p0));
                    let n = part.normals[idx(i)];
                    assert!(dot(g, n) > 0.0, "wound against its normal in {lot:?}");
                    if n[1].abs() < 0.5 && rect {
                        let mx = (p0[0] + p1[0] + p2[0]) / 3.0 - centre[0];
                        let mz = (p0[2] + p1[2] + p2[2]) / 3.0 - centre[1];
                        assert!(n[0] * mx + n[2] * mz > 0.0, "wall faces in {lot:?}");
                    }
                }
            }
        }
    }

    #[test]
    /// A 3 m by 3.6 m wall spans one bay in u and one floor in v; caps take `[x/3, z/3]`.
    fn wall_uv_is_one_bay_by_one_floor() {
        let square = vec![[0.0, 0.0], [3.0, 0.0], [3.0, 3.0], [0.0, 3.0]];
        let part = prism(&square, 0.0, 3.6, 0.0, Style::Brick, false);
        let walls: Vec<[f32; 2]> = part
            .normals
            .iter()
            .zip(&part.uvs)
            .filter(|(n, _)| n[1].abs() < 1e-6)
            .map(|(_, uv)| *uv)
            .collect();
        let u_min = walls.iter().map(|t| t[0]).fold(f32::INFINITY, f32::min);
        let u_max = walls.iter().map(|t| t[0]).fold(f32::NEG_INFINITY, f32::max);
        let v_min = walls.iter().map(|t| t[1]).fold(f32::INFINITY, f32::min);
        let v_max = walls.iter().map(|t| t[1]).fold(f32::NEG_INFINITY, f32::max);
        assert!(
            ((u_max - u_min) - 1.0).abs() < 1e-4,
            "u span {}",
            u_max - u_min
        );
        assert!(
            ((v_max - v_min) - 1.0).abs() < 1e-4,
            "v span {}",
            v_max - v_min
        );
        for ((p, n), uv) in part.positions.iter().zip(&part.normals).zip(&part.uvs) {
            if n[1] > 0.5 {
                assert!((uv[0] - p[0] / BAY_M).abs() < 1e-4);
                assert!((uv[1] - p[2] / BAY_M).abs() < 1e-4);
            }
        }
    }

    #[test]
    /// Every building stays under 400 triangles across the sampled lots.
    fn triangle_budget_holds_over_many_lots() {
        for lot in sample_lots() {
            let tris: usize = massing(&lot, 0.0)
                .parts
                .iter()
                .map(|p| p.indices.len() / 3)
                .sum();
            assert!(tris < 400, "{tris} triangles for {lot:?}");
        }
    }

    #[test]
    /// The same lot builds the same bytes; a different seed builds a different podium.
    fn same_lot_same_bytes() {
        for lot in sample_lots() {
            assert_eq!(massing(&lot, 0.0), massing(&lot, 0.0));
        }
        let a = massing(&rect_lot(40.0, 40.0, 200.0, Style::Stone, 1), 0.0);
        let b = massing(&rect_lot(40.0, 40.0, 200.0, Style::Stone, 2), 0.0);
        assert_ne!(a, b);
    }

    #[test]
    /// A wedge lot builds for every style, with tiers and boxes.
    fn wedge_lot_builds() {
        for style in Style::ALL {
            let b = massing(&wedge_lot(120.0, style, 9), 0.0);
            assert!(!b.parts.is_empty(), "no parts for {style:?}");
            assert!(!b.boxes.is_empty(), "no boxes for {style:?}");
        }
    }

    #[test]
    /// A 10 m lot is one prism with no tiers, whatever its style.
    fn ten_metre_lot_is_one_prism() {
        for style in Style::ALL {
            let lot = rect_lot(20.0, 30.0, 10.0, style, 5);
            assert_eq!(tiers(&lot).len(), 1, "{style:?}");
            assert_eq!(massing(&lot, 0.0).boxes.len(), 1, "{style:?}");
        }
    }

    #[test]
    /// Brick under 60 m gets a water tank; a tall Brick has only its box and cornice.
    fn brick_gets_a_tank_under_sixty_metres() {
        let short = massing(&rect_lot(20.0, 30.0, 40.0, Style::Brick, 1), 0.0);
        assert_eq!(short.parts.len(), 4, "box, cornice, tank base, drum");
        let top = short
            .parts
            .iter()
            .flat_map(|p| p.positions.iter())
            .map(|p| p[1])
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((top - 46.0).abs() < 1e-3, "tank top {top}");
        let tall = massing(&rect_lot(20.0, 30.0, 80.0, Style::Brick, 1), 0.0);
        assert_eq!(tall.parts.len(), 2, "box and cornice only");
    }

    #[test]
    /// Deco over 200 m gets a spire 15% of its height above the roof; under 200 m none.
    fn deco_gets_a_spire_over_two_hundred_metres() {
        let tall = massing(&rect_lot(40.0, 40.0, 250.0, Style::Deco, 1), 0.0);
        let top = tall
            .parts
            .iter()
            .flat_map(|p| p.positions.iter())
            .map(|p| p[1])
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((top - 287.5).abs() < 1e-3, "spire top {top}");
        let low = massing(&rect_lot(40.0, 40.0, 150.0, Style::Deco, 1), 0.0);
        let low_top = low
            .parts
            .iter()
            .flat_map(|p| p.positions.iter())
            .map(|p| p[1])
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            (low_top - 150.0).abs() < 1e-3,
            "no spire under 200 m: {low_top}"
        );
    }

    #[test]
    /// The Glass shaft sits 2 m in from its 12 m podium on every side.
    fn glass_shaft_is_inset_two_metres() {
        let stack = tiers(&rect_lot(30.0, 20.0, 100.0, Style::Glass, 3));
        assert_eq!(stack.len(), 2);
        let (min_x, max_x) = stack[1]
            .footprint
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| {
                (lo.min(p[0]), hi.max(p[0]))
            });
        assert!((min_x - 2.0).abs() < 1e-4 && (max_x - 28.0).abs() < 1e-4);
        assert!((stack[0].top_m - 12.0).abs() < 1e-4);
    }
}
