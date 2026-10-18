//! SPF4 + SPX11: surface gaps, the push that opens one, and one pairwise
//! measure for weak models that must not do arithmetic in their heads.
//!
//! Consumer: the `scene.query` kinds `clearance` and `measure` (lane A wires
//! them; see `docs/plan/spatial-entry-points.md`). Units: meters in (actors),
//! integer cm inside. Determinism: boxes ordered by id, ties to the lower id,
//! integer math only. Budget: O(actors) per clearance, O(1) per pair.
//!
//! Sign convention: a gap is positive when the boxes are apart, zero when
//! they touch, negative (minus the shallowest squeeze) when interiors
//! overlap. Push directions point from the neighbour toward the anchor.

use super::index::{actor_box, Box3};
use super::intmath::isqrt;
use super::Actor;

/// Walking flow bar in cm: a clearance under it reads `narrow`.
pub const FLOW_BAR_CM: i32 = 90;
/// Resting contact band in cm: a neighbour the anchor rests on is ignored.
const REST_CM: i32 = 5;

/// How far to trust a gap figure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapKind {
    /// Touching or overlapping: the figure is the true squeeze.
    Exact,
    /// Apart: box-to-box distance bounds the real clearance from above.
    UpperBound,
    /// Apart by less than the flow bar the caller asked for.
    Narrow,
}

impl GapKind {
    /// Door word.
    pub fn word(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::UpperBound => "upper_bound",
            Self::Narrow => "narrow",
        }
    }
}

/// Signed surface gap between two boxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceGap {
    /// Signed gap in cm (see the module sign convention).
    pub gap_cm: i32,
    /// Axis (0 x, 1 y, 2 z) that carries the gap or the shallowest squeeze.
    pub axis: usize,
    /// Per-axis separation: positive gap, or minus the overlap depth.
    pub axis_gap: [i32; 3],
    /// Unit push direction in permille per axis, moving `a` away from `b`.
    pub push_pm: [i32; 3],
}

fn lo_hi(b: &Box3) -> ([i32; 3], [i32; 3]) {
    ([b.min_x, b.min_y, b.min_z], [b.max_x, b.max_y, b.max_z])
}

/// Surface gap of `a` against `b`. Apart on any axis: the Euclidean distance
/// between the boxes (floored), pushed along the per-axis gaps scaled to a
/// unit vector. Overlapping on all axes: minus the shallowest squeeze, pushed
/// along that axis toward the side `a`'s center sits on (ties go to +).
pub fn surface_gap(a: &Box3, b: &Box3) -> SurfaceGap {
    let (alo, ahi) = lo_hi(a);
    let (blo, bhi) = lo_hi(b);
    let mut axis_gap = [0i32; 3];
    for k in 0..3 {
        axis_gap[k] = alo[k].max(blo[k]) - ahi[k].min(bhi[k]);
    }
    let sign = |k: usize| {
        // Side of `a`'s center relative to `b`'s, doubled so halves stay integer.
        let d = (alo[k] + ahi[k]) - (blo[k] + bhi[k]);
        if d < 0 {
            -1
        } else {
            1
        }
    };
    if axis_gap.iter().any(|g| *g > 0) {
        let sq: i64 = axis_gap.iter().map(|g| i64::from((*g).max(0)).pow(2)).sum();
        let dist = isqrt(sq);
        let mut push_pm = [0i32; 3];
        for k in 0..3 {
            if axis_gap[k] > 0 {
                let pm = i64::from(axis_gap[k]) * 1000 / dist.max(1);
                push_pm[k] = sign(k) * i32::try_from(pm).unwrap_or(1000);
            }
        }
        let axis = (0..3).max_by_key(|k| (axis_gap[*k], 2 - *k)).unwrap_or(0);
        return SurfaceGap {
            gap_cm: i32::try_from(dist).unwrap_or(i32::MAX),
            axis,
            axis_gap,
            push_pm,
        };
    }
    // Interiors overlap (or faces touch on every axis): the axis with the
    // largest (closest to zero) gap is the shallowest squeeze.
    let axis = (0..3).max_by_key(|k| (axis_gap[*k], 2 - *k)).unwrap_or(0);
    let mut push_pm = [0i32; 3];
    push_pm[axis] = 1000 * sign(axis);
    SurfaceGap {
        gap_cm: axis_gap[axis],
        axis,
        axis_gap,
        push_pm,
    }
}

/// Clearance of one actor against its nearest neighbour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clearance {
    /// Nearest neighbour's actor id (lower id on ties).
    pub neighbour: u32,
    /// The gap itself; see [`SurfaceGap`].
    pub gap: SurfaceGap,
    /// `exact`, `upper_bound`, or `narrow` when apart by less than `need_cm`.
    pub kind: GapKind,
    /// The gap meets `need_cm`.
    pub holds: bool,
}

/// Nearest-surface clearance of `anchor` against every other physical actor,
/// asked against a `need_cm` walking width. A neighbour the anchor rests on
/// (its top within a few cm of the anchor's bottom and overlapping in plan)
/// is the floor, not an obstacle, and is skipped. `None` for an unknown or
/// degenerate anchor, or when nothing else is in the scene.
pub fn clearance(actors: &[Actor], anchor: u32, need_cm: i32) -> Option<Clearance> {
    let me = actor_box(actors.iter().find(|a| a.id == anchor)?)?;
    let mut best: Option<(i32, u32, SurfaceGap)> = None;
    let mut ordered: Vec<&Actor> = actors
        .iter()
        .filter(|a| a.id != anchor && a.physical)
        .collect();
    ordered.sort_by_key(|a| a.id);
    for other in ordered {
        let Some(b) = actor_box(other) else { continue };
        let plan =
            me.min_x < b.max_x && b.min_x < me.max_x && me.min_z < b.max_z && b.min_z < me.max_z;
        if plan && (me.min_y - b.max_y).abs() <= REST_CM {
            continue;
        }
        let g = surface_gap(&me, &b);
        if best.is_none_or(|(bg, _, _)| g.gap_cm < bg) {
            best = Some((g.gap_cm, b.id, g));
        }
    }
    let (gap_cm, neighbour, gap) = best?;
    let kind = if gap_cm <= 0 {
        GapKind::Exact
    } else if gap_cm < need_cm {
        GapKind::Narrow
    } else {
        GapKind::UpperBound
    };
    Some(Clearance {
        neighbour,
        gap,
        kind,
        holds: gap_cm >= need_cm,
    })
}

/// Which pairwise question to answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum What {
    /// Signed gap against a minimum clearance.
    Gap,
    /// Interiors must not overlap deeper than the tolerance.
    Overlap,
    /// Reference points line up on the chosen axes.
    Align,
    /// `a` sits inside `b` with a margin.
    Contain,
}

/// Reference point for alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    Center,
    Min,
    Max,
}

/// Caller knobs for [`measure`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeasureArgs {
    pub what: What,
    /// Touching within this many cm is contact; also the align tolerance.
    pub tolerance_cm: i32,
    /// Axes the question reads, as a mask: bit 0 x, bit 1 y, bit 2 z.
    pub axes: u8,
    /// Reference point for `Align`.
    pub reference: Anchor,
    /// Required gap for `Gap`, required margin for `Contain`.
    pub min_clearance_cm: i32,
}

impl Default for MeasureArgs {
    fn default() -> Self {
        Self {
            what: What::Gap,
            tolerance_cm: 1,
            axes: 0b111,
            reference: Anchor::Center,
            min_clearance_cm: 0,
        }
    }
}

/// Relation word of a pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairRelation {
    Apart,
    Contact,
    Overlap,
}

impl PairRelation {
    /// Door word.
    pub fn word(self) -> &'static str {
        match self {
            Self::Apart => "apart",
            Self::Contact => "contact",
            Self::Overlap => "overlap",
        }
    }
}

/// Answer to one pairwise question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Measure {
    /// Signed surface gap in cm.
    pub gap_cm: i32,
    /// Per-axis gap (positive) or overlap depth (negative), cm.
    pub axis_gap: [i32; 3],
    pub relation: PairRelation,
    /// Overlap volume over the smaller box's volume, permille (0 when apart).
    pub penetration_pm: u32,
    /// The asked condition holds.
    pub passed: bool,
    /// How many cm short of passing (0 when it passes).
    pub shortfall_cm: i32,
}

fn volume(b: &Box3) -> i128 {
    i128::from(b.max_x - b.min_x) * i128::from(b.max_y - b.min_y) * i128::from(b.max_z - b.min_z)
}

/// One pairwise measure of `a` against `b` (see [`What`]). Exact integers,
/// so a weak model reads `shortfall_cm` instead of subtracting.
pub fn measure(a: &Box3, b: &Box3, args: &MeasureArgs) -> Measure {
    let g = surface_gap(a, b);
    let tol = args.tolerance_cm.max(0);
    let relation = if g.gap_cm.abs() <= tol {
        PairRelation::Contact
    } else if g.gap_cm < 0 {
        PairRelation::Overlap
    } else {
        PairRelation::Apart
    };
    let penetration_pm = if g.axis_gap.iter().all(|v| *v < 0) {
        let cut: i128 = g.axis_gap.iter().map(|v| i128::from(-*v)).product();
        let small = volume(a).min(volume(b)).max(1);
        u32::try_from(cut * 1000 / small).unwrap_or(1000)
    } else {
        0
    };
    let on = |k: usize| args.axes >> k & 1 == 1;
    let (alo, ahi) = lo_hi(a);
    let (blo, bhi) = lo_hi(b);
    let shortfall_cm = match args.what {
        What::Gap => (args.min_clearance_cm - g.gap_cm).max(0),
        What::Overlap => (-g.gap_cm - tol).max(0),
        What::Align => (0..3)
            .filter(|k| on(*k))
            .map(|k| {
                let r = |lo: i32, hi: i32| match args.reference {
                    Anchor::Center => lo + hi,
                    Anchor::Min => lo * 2,
                    Anchor::Max => hi * 2,
                };
                ((r(alo[k], ahi[k]) - r(blo[k], bhi[k])).abs() + 1) / 2
            })
            .map(|d| (d - tol).max(0))
            .max()
            .unwrap_or(0),
        What::Contain => (0..3)
            .filter(|k| on(*k))
            .map(|k| {
                let low = (blo[k] + args.min_clearance_cm) - alo[k];
                let high = ahi[k] - (bhi[k] - args.min_clearance_cm);
                low.max(high).max(0)
            })
            .max()
            .unwrap_or(0),
    };
    Measure {
        gap_cm: g.gap_cm,
        axis_gap: g.axis_gap,
        relation,
        penetration_pm,
        passed: shortfall_cm == 0,
        shortfall_cm,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::Rng;

    fn bx(id: u32, lo: [i32; 3], hi: [i32; 3]) -> Box3 {
        Box3 {
            id,
            min_x: lo[0],
            max_x: hi[0],
            min_y: lo[1],
            max_y: hi[1],
            min_z: lo[2],
            max_z: hi[2],
        }
    }

    /// Voxel brute force: the box is the set of unit cells it covers.
    fn share_cell(a: &Box3, b: &Box3, shift: [i32; 3]) -> bool {
        let (alo, ahi) = lo_hi(a);
        let (blo, bhi) = lo_hi(b);
        (0..3).all(|k| alo[k] + shift[k] < bhi[k] && blo[k] < ahi[k] + shift[k])
    }

    /// Min squared distance over lattice points of both boxes.
    fn lattice_d2(a: &Box3, b: &Box3) -> i64 {
        let (alo, ahi) = lo_hi(a);
        let (blo, bhi) = lo_hi(b);
        let mut best = i64::MAX;
        for ax in alo[0]..=ahi[0] {
            for ay in alo[1]..=ahi[1] {
                for az in alo[2]..=ahi[2] {
                    for bx_ in blo[0]..=bhi[0] {
                        for by in blo[1]..=bhi[1] {
                            for bz in blo[2]..=bhi[2] {
                                let d = |p: i32, q: i32| i64::from(p - q).pow(2);
                                best = best.min(d(ax, bx_) + d(ay, by) + d(az, bz));
                            }
                        }
                    }
                }
            }
        }
        best
    }

    /// Shallowest squeeze by counting: per axis, how many unit cells both
    /// intervals cover; the smallest count is the penetration depth.
    fn shallowest_by_count(a: &Box3, b: &Box3) -> i32 {
        let (alo, ahi) = lo_hi(a);
        let (blo, bhi) = lo_hi(b);
        (0..3)
            .map(|k| {
                (0..64)
                    .filter(|c| alo[k] <= *c && *c < ahi[k] && blo[k] <= *c && *c < bhi[k])
                    .count()
            })
            .min()
            .map(|n| i32::try_from(n).unwrap())
            .unwrap()
    }

    #[test]
    fn surface_gap_pins_the_four_forge_cases() {
        let a = bx(1, [0, 0, 0], [100, 100, 100]);
        // Apart 30 cm on x, anchor on the +x side: gap 30, push +x.
        let near = bx(2, [-130, 0, 0], [-30, 100, 100]);
        let g = surface_gap(&a, &near);
        assert_eq!((g.gap_cm, g.axis, g.push_pm), (30, 0, [1000, 0, 0]));
        // Touching: zero, exact.
        let touch = bx(2, [-100, 0, 0], [0, 100, 100]);
        assert_eq!(surface_gap(&a, &touch).gap_cm, 0);
        // 12 cm overlap on the shallowest axis: -12, pushed toward a's side.
        let over = bx(2, [-88, -50, -50], [12, 150, 150]);
        let g = surface_gap(&a, &over);
        assert_eq!((g.gap_cm, g.axis, g.push_pm), (-12, 0, [1000, 0, 0]));
        // Corner 30 / 40 apart: 50, push split 600 / 800 permille.
        let corner = bx(2, [-130, 0, -140], [-30, 100, -40]);
        let g = surface_gap(&a, &corner);
        assert_eq!((g.gap_cm, g.push_pm), (50, [600, 0, 800]));
    }

    #[test]
    fn moving_the_anchor_by_the_push_makes_the_clearance_hold() {
        let mut a = Actor {
            id: 1,
            name: "a".into(),
            x: 0.5,
            y: 0.5,
            z: 0.5,
            sx: 1.0,
            sy: 1.0,
            sz: 1.0,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        };
        let mut wall = a.clone();
        wall.id = 2;
        wall.x = -0.8; // 30 cm face gap on x (wall spans -1.3..-0.3)
        wall.y = 0.5;
        let scene = |a: &Actor, w: &Actor| vec![a.clone(), w.clone()];
        let c = clearance(&scene(&a, &wall), 1, 90).unwrap();
        assert_eq!(
            (c.neighbour, c.gap.gap_cm, c.kind, c.holds),
            (2, 30, GapKind::Narrow, false)
        );
        // Move by push x (need - gap) = 60 cm along +x.
        a.x += 0.60;
        let c = clearance(&scene(&a, &wall), 1, 90).unwrap();
        assert!(c.holds && c.gap.gap_cm == 90, "{c:?}");
        assert_eq!(c.kind, GapKind::UpperBound);
        // A floor slab the anchor rests on is not an obstacle.
        let mut floor = wall.clone();
        floor.id = 3;
        floor.x = 0.0;
        floor.y = -0.05;
        floor.sx = 20.0;
        floor.sy = 0.1;
        floor.sz = 20.0;
        let with_floor = vec![a.clone(), wall.clone(), floor];
        assert_eq!(clearance(&with_floor, 1, 90).unwrap().neighbour, 2);
        assert_eq!(clearance(&[a], 1, 90), None);
    }

    #[test]
    fn measure_pair_matches_brute_force_on_40_seeded_pairs() {
        let mut rng = Rng::new(0x6a9);
        for case in 0..40 {
            let mut r = |m: u32| (rng.next_u32() % m) as i32;
            let size = |r: &mut dyn FnMut(u32) -> i32| [1 + r(5), 1 + r(5), 1 + r(5)];
            let (sa, sb) = (size(&mut r), size(&mut r));
            let (pa, pb) = ([r(8), r(8), r(8)], [r(8), r(8), r(8)]);
            let a = bx(1, pa, [pa[0] + sa[0], pa[1] + sa[1], pa[2] + sa[2]]);
            let b = bx(2, pb, [pb[0] + sb[0], pb[1] + sb[1], pb[2] + sb[2]]);
            let want = if share_cell(&a, &b, [0; 3]) {
                -shallowest_by_count(&a, &b)
            } else {
                i32::try_from(isqrt(lattice_d2(&a, &b))).unwrap()
            };
            let args = MeasureArgs {
                min_clearance_cm: 3,
                tolerance_cm: 0,
                ..MeasureArgs::default()
            };
            let m = measure(&a, &b, &args);
            assert_eq!(m.gap_cm, want, "case {case}: {a:?} vs {b:?}");
            assert_eq!(m.passed, want >= 3, "case {case}");
            assert_eq!(m.shortfall_cm, (3 - want).max(0), "case {case}");
            assert_eq!(m, measure(&a, &b, &args), "replay x2");
            assert_eq!(
                m.penetration_pm > 0,
                share_cell(&a, &b, [0; 3]),
                "case {case}"
            );
        }
    }

    #[test]
    fn contact_align_and_contain_read_the_poking_crate() {
        let room = bx(1, [0, 0, 0], [400, 300, 400]);
        let inside = bx(2, [50, 0, 50], [150, 100, 150]);
        let poking = bx(3, [350, 0, 50], [450, 100, 150]);
        let contain = MeasureArgs {
            what: What::Contain,
            ..MeasureArgs::default()
        };
        assert!(measure(&inside, &room, &contain).passed);
        let m = measure(&poking, &room, &contain);
        assert!(!m.passed);
        assert_eq!(m.shortfall_cm, 50, "pokes 50 cm out of the +x wall");
        // Touching within tolerance is contact.
        let next = bx(4, [150, 0, 50], [250, 100, 150]);
        let near = MeasureArgs {
            tolerance_cm: 2,
            ..MeasureArgs::default()
        };
        assert_eq!(
            measure(&inside, &next, &near).relation,
            PairRelation::Contact
        );
        // Flush on min z: aligned on z only.
        let align = MeasureArgs {
            what: What::Align,
            axes: 0b100,
            reference: Anchor::Min,
            tolerance_cm: 0,
            ..MeasureArgs::default()
        };
        assert!(measure(&inside, &next, &align).passed);
        let off = bx(5, [150, 0, 80], [250, 100, 180]);
        assert_eq!(measure(&inside, &off, &align).shortfall_cm, 30);
    }
}
