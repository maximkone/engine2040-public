//! rope: how the spear grapple's rope is drawn. The physics world hands over a coarse verlet chain
//! (8 to 32 nodes) that jitters a little from tick to tick; drawn as it comes, it is a thin
//! angular zigzag. Here the nodes are relaxed and run through a centripetal Catmull-Rom curve,
//! sampled at a ring spacing the tier sets, and swept into one lit tube whose three-lobe cross
//! section turns along the rope: the lay of a three-strand hemp line. A taut rope is the straight
//! line between its ends, a slack one sags through its nodes, a snagged one runs to its corner
//! and turns round it on a small fillet.
//!
//! Presentation only: the nodes are read, never written back, so the sim and the golden cannot
//! move. Caller: `spear_window::SpearScene::push`. Units: meters, Y up. Determinism: pure floats,
//! the same nodes give the same mesh. Budget: the vertex count of a rope mesh is fixed per tier
//! (a short rope repeats its last ring), so the renderer rewrites the uploaded buffer in place
//! every frame instead of allocating a new one as the rope grows and shrinks.

use super::{add, idx, length, norm as unit, num, scale, sub, Mesh};
use crate::lock_on::whole;
use engine_core::vec3::cross3;
use engine_physfx::Tier;
use std::f32::consts::TAU;

/// Radius of the rope's core, meters: a line about 3.2 cm thick, with the strands standing a
/// little proud of it.
pub(super) const ROPE_R_M: f32 = 0.016;
/// How far a strand lobe stands out of the core, as a share of its radius.
const LOBE: f32 = 0.22;
/// Strands round the rope.
const LOBES: f32 = 3.0;
/// Length of rope for one full turn of the strands, meters.
pub(super) const LAY_M: f32 = 0.18;
/// Fewest rings per turn of the lay that still read as a helix; with fewer the strands alias
/// into a zigzag and the rope is drawn round.
const MIN_RINGS_PER_LAY: f32 = 4.0;
/// Passes of the 1-2-1 filter a hanging rope's nodes take before the curve goes through them.
const SLACK_PASSES: usize = 2;
/// Passes for a rope lying on the sand, whose nodes have no ends to hold them in line.
const LOOSE_PASSES: usize = 4;
/// Samples per node span when the spline is laid down, before it is cut into rings.
const SPAN_STEPS: usize = 8;
/// Samples on the fillet round a snag corner.
const FILLET_STEPS: usize = 6;
/// Radius of the fillet round a snag corner, meters.
const WRAP_FILLET_M: f32 = 0.08;
/// The lowest an interior ring sits, meters: the rope lies on the sand, not in it.
const FLOOR_M: f32 = 0.015;

/// What a rope looks like on one tier: how many rings it is cut into and how many sides each has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct RopeStyle {
    /// Distance between rings along the rope, meters.
    pub(super) ring_m: f32,
    /// Rings in every mesh of this style. Fixed, so the topology never changes.
    pub(super) rings: usize,
    /// Sides of one ring.
    pub(super) sides: usize,
    /// Turn the strand lobes along the rope; off, the rope is a round tube.
    pub(super) twist: bool,
}

impl RopeStyle {
    /// The rope of one tier: the weak tiers draw a round tube with few rings, the desktop tiers
    /// the twisted strands.
    pub(super) const fn for_tier(tier: Tier) -> Self {
        match tier {
            Tier::Potato => Self {
                ring_m: 0.30,
                rings: 35,
                sides: 5,
                twist: false,
            },
            Tier::Mobile => Self {
                ring_m: 0.20,
                rings: 51,
                sides: 6,
                twist: false,
            },
            Tier::Web => Self {
                ring_m: 0.12,
                rings: 85,
                sides: 8,
                twist: false,
            },
            Tier::Comfort => Self {
                ring_m: 0.025,
                rings: 401,
                sides: 12,
                twist: true,
            },
            Tier::High => Self {
                ring_m: 0.018,
                rings: 557,
                sides: 15,
                twist: true,
            },
        }
    }

    /// Vertices of every rope mesh of this style.
    pub(super) const fn vertices(&self) -> usize {
        self.rings * self.sides
    }
}

/// How a rope lies: what decides how its nodes are drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Lay {
    /// Pulled straight between its ends.
    Taut,
    /// Hanging in its sag.
    Slack,
    /// Let go and fallen: no end holds it.
    Loose,
    /// Snagged on a corner (meters): out to the corner and back.
    Wrapped([f32; 3]),
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    add(a, scale(sub(b, a), t))
}

/// Length of a polyline, meters.
pub(super) fn path_len(points: &[[f32; 3]]) -> f32 {
    points
        .windows(2)
        .map(|pair| length(sub(pair[1], pair[0])))
        .sum()
}

/// Rings a rope of `len_m` is cut into: one per ring spacing, at least the two ends, at most
/// what the mesh holds (a rope longer than its reach spreads its rings).
fn rings_for(len_m: f32, style: &RopeStyle) -> usize {
    let cuts = usize::try_from(whole((len_m / style.ring_m).ceil())).unwrap_or(0);
    (cuts + 1).clamp(2, style.rings)
}

/// `n` points evenly spaced along the polyline, the first and last exactly its ends.
fn resample(poly: &[[f32; 3]], n: usize) -> Vec<[f32; 3]> {
    let (Some(&first), Some(&last)) = (poly.first(), poly.last()) else {
        return Vec::new();
    };
    let mut reach = Vec::with_capacity(poly.len());
    let mut sum = 0.0;
    reach.push(sum);
    for pair in poly.windows(2) {
        sum += length(sub(pair[1], pair[0]));
        reach.push(sum);
    }
    if n < 2 || sum < 1.0e-6 {
        return vec![first, last];
    }
    let mut out = Vec::with_capacity(n);
    let mut seg = 0;
    for i in 0..n {
        let at = sum * num(i) / num(n - 1);
        while seg + 2 < poly.len() && reach[seg + 1] < at {
            seg += 1;
        }
        let span = reach[seg + 1] - reach[seg];
        let t = if span > 1.0e-9 {
            ((at - reach[seg]) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        out.push(lerp(poly[seg], poly[seg + 1], t));
    }
    out[0] = first;
    let end = out.len() - 1;
    out[end] = last;
    out
}

/// One point of the centripetal Catmull-Rom span from `p[1]` to `p[2]`, `t` in `0..=1` (the
/// Barry-Goldman form). The centripetal knots keep an unevenly spaced chain from looping or
/// cusping.
fn catmull(p: [[f32; 3]; 4], t: f32) -> [f32; 3] {
    let knot = |a: [f32; 3], b: [f32; 3]| length(sub(b, a)).sqrt().max(1.0e-3);
    let t1 = knot(p[0], p[1]);
    let t2 = t1 + knot(p[1], p[2]);
    let t3 = t2 + knot(p[2], p[3]);
    let u = t1 + (t2 - t1) * t;
    let mix = |a: [f32; 3], b: [f32; 3], ta: f32, tb: f32| lerp(a, b, (u - ta) / (tb - ta));
    let a1 = mix(p[0], p[1], 0.0, t1);
    let a2 = mix(p[1], p[2], t1, t2);
    let a3 = mix(p[2], p[3], t2, t3);
    let b1 = mix(a1, a2, 0.0, t2);
    let b2 = mix(a2, a3, t1, t3);
    mix(b1, b2, t1, t2)
}

/// The spline through `nodes`, laid down as a dense polyline (it passes through every node).
fn dense(nodes: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let n = nodes.len();
    if n < 2 {
        return nodes.to_vec();
    }
    let mut ext = Vec::with_capacity(n + 2);
    ext.push(sub(scale(nodes[0], 2.0), nodes[1]));
    ext.extend_from_slice(nodes);
    ext.push(sub(scale(nodes[n - 1], 2.0), nodes[n - 2]));
    let mut out = Vec::with_capacity((n - 1) * SPAN_STEPS + 1);
    for span in 0..n - 1 {
        let four = [ext[span], ext[span + 1], ext[span + 2], ext[span + 3]];
        for step in 0..SPAN_STEPS {
            out.push(catmull(four, num(step) / num(SPAN_STEPS)));
        }
    }
    out.push(nodes[n - 1]);
    out
}

/// The 1-2-1 filter over the interior nodes, ends fixed: it takes the tick-to-tick zigzag out of
/// the chain and moves a hanging rope's sag by less than a millimeter.
fn relax(points: &mut [[f32; 3]]) {
    let last = points.len().saturating_sub(1);
    if last < 2 {
        return;
    }
    let mut before = points[0];
    for i in 1..last {
        let here = points[i];
        let mixed = add(
            add(scale(before, 0.25), scale(here, 0.5)),
            scale(points[i + 1], 0.25),
        );
        points[i] = mixed;
        before = here;
    }
}

/// The straight line between the ends, in rings.
fn straight(a: [f32; 3], b: [f32; 3], style: &RopeStyle) -> Vec<[f32; 3]> {
    let n = rings_for(length(sub(b, a)), style);
    let mut out: Vec<[f32; 3]> = (0..n).map(|i| lerp(a, b, num(i) / num(n - 1))).collect();
    out[n - 1] = b;
    out
}

/// A rope hanging or lying: filtered nodes, the spline through them, cut into rings and kept
/// off the sand.
fn spline(nodes: &[[f32; 3]], passes: usize, style: &RopeStyle) -> Vec<[f32; 3]> {
    let (Some(&a), Some(&b)) = (nodes.first(), nodes.last()) else {
        return Vec::new();
    };
    if nodes.len() < 3 {
        return straight(a, b, style);
    }
    let mut points = nodes.to_vec();
    for _ in 0..passes {
        relax(&mut points);
    }
    let poly = dense(&points);
    let mut out = resample(&poly, rings_for(path_len(&poly), style));
    let last = out.len().saturating_sub(1);
    for p in out.iter_mut().take(last).skip(1) {
        p[1] = p[1].max(FLOOR_M);
    }
    out
}

/// A rope snagged on `corner`: straight out to it and straight back, rounded by a small fillet
/// so the line turns round the corner instead of kinking.
fn wrapped(a: [f32; 3], corner: [f32; 3], b: [f32; 3], style: &RopeStyle) -> Vec<[f32; 3]> {
    let (leg_in, leg_out) = (sub(corner, a), sub(b, corner));
    let (in_len, out_len) = (length(leg_in), length(leg_out));
    if in_len < 1.0e-3 || out_len < 1.0e-3 {
        return straight(a, b, style);
    }
    let r = WRAP_FILLET_M.min(in_len * 0.5).min(out_len * 0.5);
    let enter = sub(corner, scale(leg_in, r / in_len));
    let leave = add(corner, scale(leg_out, r / out_len));
    let mut poly = vec![a, enter];
    for k in 1..FILLET_STEPS {
        let t = num(k) / num(FILLET_STEPS);
        poly.push(lerp(lerp(enter, corner, t), lerp(corner, leave, t), t));
    }
    poly.push(leave);
    poly.push(b);
    resample(&poly, rings_for(path_len(&poly), style))
}

/// The points the rope tube is swept along, from its nodes (`first` the palm, `last` the tie at
/// the far end, both exactly the ends of the result). Empty when a node is not finite.
pub(super) fn curve(nodes: &[[f32; 3]], lay: Lay, style: &RopeStyle) -> Vec<[f32; 3]> {
    let (Some(&a), Some(&b)) = (nodes.first(), nodes.last()) else {
        return Vec::new();
    };
    if !nodes.iter().flatten().all(|c| c.is_finite()) {
        return Vec::new();
    }
    match lay {
        Lay::Taut => straight(a, b, style),
        Lay::Slack => spline(nodes, SLACK_PASSES, style),
        Lay::Loose => spline(nodes, LOOSE_PASSES, style),
        Lay::Wrapped(corner) if corner.iter().all(|c| c.is_finite()) => {
            wrapped(a, corner, b, style)
        }
        Lay::Wrapped(_) => straight(a, b, style),
    }
}

/// A unit vector at right angles to the unit `t`.
fn perp(t: [f32; 3]) -> [f32; 3] {
    let helper = if t[1].abs() < 0.95 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    unit(cross3(t, helper))
}

/// `u` carried along to the tangent `t` without turning about it (parallel transport), so the
/// strands do not twist where the rope bends.
fn carry(u: [f32; 3], t: [f32; 3]) -> [f32; 3] {
    let flat = sub(u, scale(t, dot(u, t)));
    if length(flat) < 1.0e-4 {
        perp(t)
    } else {
        unit(flat)
    }
}

/// Tangent of the curve at ring `i`: the chord between its neighbors.
fn tangent(curve: &[[f32; 3]], i: usize) -> [f32; 3] {
    let last = curve.len().saturating_sub(1);
    unit(sub(curve[(i + 1).min(last)], curve[i.saturating_sub(1)]))
}

/// The rope tube swept along `curve`: `style.sides` per ring, always `style.rings` rings (a short
/// curve closes its far end with a ring collapsed to the end point and repeats that, so the mesh
/// keeps its shape). With twist the cross section
/// is three lobes that turn once per [`LAY_M`] of rope, its normals the true ones of that surface,
/// so the strands light up as a helix. `None` when there are fewer than two points.
pub(super) fn mesh(curve: &[[f32; 3]], style: &RopeStyle) -> Option<Mesh> {
    let n = curve.len();
    if n < 2 {
        return None;
    }
    let sides = style.sides;
    let rings = style.rings.max(n);
    let spacing = path_len(curve) / num(n - 1);
    let twist = style.twist && spacing * MIN_RINGS_PER_LAY <= LAY_M;
    let turns_per_m = TAU / LAY_M;
    let angles: Vec<(f32, f32, f32)> = (0..sides)
        .map(|k| {
            let phi = num(k) / num(sides) * TAU;
            (phi, phi.cos(), phi.sin())
        })
        .collect();
    let mut positions = Vec::with_capacity(style.vertices());
    let mut normals = Vec::with_capacity(style.vertices());
    let mut u = perp(tangent(curve, 0));
    let mut along_m = 0.0;
    for i in 0..n {
        let t = tangent(curve, i);
        u = carry(u, t);
        let v = cross3(t, u);
        if i > 0 {
            along_m += length(sub(curve[i], curve[i - 1]));
        }
        for &(phi, c, s) in &angles {
            let radial = add(scale(u, c), scale(v, s));
            if twist {
                let (sin_p, cos_p) = (LOBES * phi - along_m * turns_per_m).sin_cos();
                let r = ROPE_R_M * (1.0 + LOBE * cos_p);
                let dr_dphi = -ROPE_R_M * LOBE * LOBES * sin_p;
                let dr_ds = ROPE_R_M * LOBE * sin_p * turns_per_m;
                let around = add(scale(u, -s), scale(v, c));
                let lean = sub(scale(radial, r), scale(around, dr_dphi));
                positions.push(add(curve[i], scale(radial, r)));
                normals.push(unit(sub(lean, scale(t, r * dr_ds))));
            } else {
                positions.push(add(curve[i], scale(radial, ROPE_R_M)));
                normals.push(radial);
            }
        }
    }
    if rings > n {
        // The far end is closed: one ring collapsed to the end point, facing along the rope.
        let along = tangent(curve, n - 1);
        positions.extend(std::iter::repeat_n(curve[n - 1], sides));
        normals.extend(std::iter::repeat_n(along, sides));
    }
    for _ in n + 1..rings {
        let from = positions.len() - sides;
        positions.extend_from_within(from..);
        normals.extend_from_within(from..);
    }
    let mut indices = Vec::with_capacity((rings - 1) * sides * 6);
    for ring in 0..rings - 1 {
        for k in 0..sides {
            let (a, b) = (ring * sides + k, ring * sides + (k + 1) % sides);
            let (c, d) = (a + sides, b + sides);
            indices.extend([idx(a), idx(b), idx(c), idx(b), idx(d), idx(c)]);
        }
    }
    Some((positions, normals, indices))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_physfx::{PhysFx, TetherState};

    /// Rope length every style's mesh holds at full ring density, meters; a longer rope spreads
    /// its rings.
    const REACH_M: f32 = 10.0;

    fn style() -> RopeStyle {
        RopeStyle::for_tier(Tier::Comfort)
    }

    /// A chain sagging between two points 3 m apart, `nodes` long, with `jitter` meters of
    /// alternating error on every interior node (the tick-to-tick noise of a verlet chain).
    fn sagging(nodes: usize, jitter: f32) -> Vec<[f32; 3]> {
        (0..nodes)
            .map(|i| {
                let t = num(i) / num(nodes - 1);
                let ends = i == 0 || i == nodes - 1;
                let wobble = if ends {
                    0.0
                } else if i % 2 == 0 {
                    jitter
                } else {
                    -jitter
                };
                [t * 3.0, 1.4 - 0.9 * 4.0 * t * (1.0 - t) + wobble, 0.2 * t]
            })
            .collect()
    }

    /// The biggest turn between two neighboring segments, degrees.
    fn sharpest_deg(points: &[[f32; 3]]) -> f32 {
        points
            .windows(3)
            .map(|w| {
                let (a, b) = (unit(sub(w[1], w[0])), unit(sub(w[2], w[1])));
                dot(a, b).clamp(-1.0, 1.0).acos().to_degrees()
            })
            .fold(0.0, f32::max)
    }

    /// Smallest distance between two segments of the curve that are at least `gap` segments
    /// apart (so neighbors on the same bend do not count), by sampling the segments.
    fn nearest_apart(points: &[[f32; 3]], gap: usize) -> f32 {
        let mut best = f32::MAX;
        for i in 0..points.len() - 1 {
            for j in i + gap..points.len() - 1 {
                for (p, q) in [(points[i], points[j]), (points[i + 1], points[j + 1])] {
                    best = best.min(length(sub(p, q)));
                }
            }
        }
        best
    }

    #[test]
    fn rope_styles_cover_the_reach_and_the_potato_is_cheapest() {
        let mut last = 0;
        for tier in Tier::ALL {
            let s = RopeStyle::for_tier(tier);
            assert!(
                num(s.rings - 1) * s.ring_m >= REACH_M - 1.0e-3,
                "{tier:?} reaches {} m",
                num(s.rings - 1) * s.ring_m
            );
            assert!(
                s.vertices() > last,
                "{tier:?} costs more than the tier under"
            );
            last = s.vertices();
            assert_eq!(s.twist, matches!(tier, Tier::Comfort | Tier::High));
        }
        assert!(RopeStyle::for_tier(Tier::Potato).vertices() < 200);
    }

    #[test]
    fn the_curve_starts_and_ends_exactly_on_the_nodes_whatever_the_lay() {
        let nodes = sagging(20, 0.02);
        let (a, b) = (nodes[0], nodes[19]);
        for lay in [
            Lay::Taut,
            Lay::Slack,
            Lay::Loose,
            Lay::Wrapped([1.5, 0.6, 2.0]),
        ] {
            let c = curve(&nodes, lay, &style());
            assert!(c.len() >= 2, "{lay:?}");
            assert_eq!(c[0], a, "{lay:?} starts in the palm");
            assert_eq!(c[c.len() - 1], b, "{lay:?} ends on the tie");
        }
        assert!(curve(&[], Lay::Slack, &style()).is_empty());
        let mut bad = nodes;
        bad[7][1] = f32::NAN;
        assert!(curve(&bad, Lay::Slack, &style()).is_empty());
    }

    #[test]
    fn node_jitter_does_not_draw_as_a_zigzag() {
        let nodes = sagging(20, 0.02);
        assert!(
            sharpest_deg(&nodes) > 25.0,
            "the raw chain zigzags: {}",
            sharpest_deg(&nodes)
        );
        for tier in Tier::ALL {
            let c = curve(&nodes, Lay::Slack, &RopeStyle::for_tier(tier));
            let kink = sharpest_deg(&c);
            let limit = match tier {
                Tier::Potato => 40.0,
                Tier::Mobile | Tier::Web => 25.0,
                _ => 6.0,
            };
            assert!(kink < limit, "{tier:?}: {kink} degrees in one ring");
        }
    }

    #[test]
    fn a_hanging_rope_does_not_cross_itself_and_stays_inside_its_slack() {
        let nodes = sagging(20, 0.02);
        let c = curve(&nodes, Lay::Slack, &style());
        let gap = nearest_apart(&c, 6);
        assert!(gap > 0.1, "{gap} m between two stretches of the rope");
        let chord = length(sub(nodes[19], nodes[0]));
        let rope = path_len(&nodes);
        let drawn = path_len(&c);
        assert!(
            drawn >= chord,
            "{drawn} m is shorter than its chord {chord}"
        );
        assert!(
            drawn <= rope * 1.03,
            "{drawn} m is longer than the rope {rope}"
        );
    }

    #[test]
    fn a_real_verlet_rope_draws_smooth_within_its_length_and_straight_when_taut() {
        let mut fx = PhysFx::new(Tier::Comfort);
        let (a, b) = ([0.0, 1.3, 0.0], [3.4, 1.1, 0.5]);
        let rest = 4.2;
        fx.attach_tether(1, a, b, rest).expect("a rope");
        for tick in 0..90 {
            let sway = (num(tick) * 0.4).sin() * 0.02;
            fx.move_tether(1, a, [b[0], b[1], b[2] + sway])
                .expect("ends");
            fx.step();
        }
        let info = fx.tether_info(1).expect("the rope");
        assert_eq!(info.state, TetherState::Slack);
        let nodes = fx.tether_nodes(1, 1.0).expect("alpha").expect("nodes");
        let c = curve(&nodes, Lay::Slack, &style());
        let chord = length(sub(*nodes.last().expect("end"), nodes[0]));
        let drawn = path_len(&c);
        assert!(drawn > chord + 0.2, "it sags: {drawn} vs chord {chord}");
        assert!(drawn <= rest * 1.03, "{drawn} m of a {rest} m rope");
        assert!(sharpest_deg(&c) < 6.0, "{} degrees", sharpest_deg(&c));
        assert!(nearest_apart(&c, 6) > 0.1);
        // Pulled tight: the straight line, every ring on it, evenly spaced.
        let tight = [chord + 0.5, 1.1, 0.5];
        let line = curve(&[a, tight], Lay::Taut, &style());
        let step = length(sub(line[1], line[0]));
        for pair in line.windows(2) {
            assert!((length(sub(pair[1], pair[0])) - step).abs() < 1.0e-4);
        }
        assert!(sharpest_deg(&line) < 0.1);
        assert!((path_len(&line) - length(sub(tight, a))).abs() < 1.0e-3);
    }

    #[test]
    fn a_snagged_rope_runs_to_its_corner_and_turns_round_it() {
        let (hand, anchor, corner) = ([0.0, 1.0, 0.0], [0.0, 1.0, 10.0], [5.0, 1.0, 5.0]);
        let c = curve(&[hand, anchor], Lay::Wrapped(corner), &style());
        assert_eq!((c[0], c[c.len() - 1]), (hand, anchor));
        let near = c
            .iter()
            .map(|p| length(sub(*p, corner)))
            .fold(f32::MAX, f32::min);
        assert!(near < WRAP_FILLET_M, "{near} m from the corner");
        // Both legs are straight: no ring strays off the two lines.
        let off_leg = |p: [f32; 3], from: [f32; 3], to: [f32; 3]| {
            let d = unit(sub(to, from));
            let rel = sub(p, from);
            length(sub(rel, scale(d, dot(rel, d))))
        };
        for p in &c {
            let leg = off_leg(*p, hand, corner).min(off_leg(*p, corner, anchor));
            assert!(leg < 0.05, "{p:?} is {leg} m off the legs");
        }
        // A corner on the line itself leaves the rope straight.
        let flat = curve(&[hand, anchor], Lay::Wrapped([0.0, 1.0, 5.0]), &style());
        assert!(sharpest_deg(&flat) < 0.1);
        // A corner that is not a number draws the plain line.
        let bad = curve(&[hand, anchor], Lay::Wrapped([f32::NAN; 3]), &style());
        assert!(sharpest_deg(&bad) < 0.1);
    }

    /// Every triangle of `shape` winds counter-clockwise seen from outside (its face normal
    /// agrees with its vertex normals), as the renderer's back-face culling needs.
    fn outward_share(shape: &Mesh) -> f32 {
        let (positions, normals, indices) = shape;
        let (mut good, mut seen) = (0, 0);
        for tri in indices.chunks(3) {
            let at = |i: u32| positions[usize::try_from(i).expect("index")];
            let (p0, p1, p2) = (at(tri[0]), at(tri[1]), at(tri[2]));
            let face = cross3(sub(p1, p0), sub(p2, p0));
            if length(face) < 1.0e-9 {
                continue;
            }
            let mut mean = [0.0; 3];
            for &i in tri {
                mean = add(mean, normals[usize::try_from(i).expect("index")]);
            }
            seen += 1;
            good += i32::from(dot(face, mean) > 0.0);
        }
        num(usize::try_from(good).expect("count")) / num(usize::try_from(seen).expect("count"))
    }

    #[test]
    fn the_rope_mesh_has_a_fixed_shape_and_valid_lit_faces() {
        for tier in Tier::ALL {
            let s = RopeStyle::for_tier(tier);
            let short = curve(&sagging(s.sides + 5, 0.0), Lay::Slack, &s);
            let long = curve(&[[0.0, 1.0, 0.0], [8.0, 1.5, 0.0]], Lay::Taut, &s);
            assert!(short.len() < long.len(), "{tier:?}");
            let (a, b) = (
                mesh(&short, &s).expect("a short rope"),
                mesh(&long, &s).expect("a long rope"),
            );
            assert_eq!(a.0.len(), s.vertices(), "{tier:?}");
            assert_eq!(a.0.len(), b.0.len(), "{tier:?} keeps its vertex count");
            for (shape, ring) in [(&a, &short), (&b, &long)] {
                let end = *ring.last().expect("an end");
                let cap = &shape.0[ring.len() * s.sides..(ring.len() + 1) * s.sides];
                assert!(cap.iter().all(|p| *p == end), "{tier:?} closes its far end");
            }
            assert_eq!(a.2, b.2, "{tier:?} keeps its indices");
            assert!(a
                .2
                .iter()
                .all(|i| usize::try_from(*i).expect("i") < a.0.len()));
            assert!(a.0.iter().chain(&a.1).flatten().all(|c| c.is_finite()));
            assert!(a.1.iter().all(|n| (length(*n) - 1.0).abs() < 1.0e-3));
            assert!(
                outward_share(&b) > 0.99,
                "{tier:?}: {} of the faces wind outward",
                outward_share(&b)
            );
        }
        assert!(mesh(&[[0.0; 3]], &style()).is_none());
    }

    #[test]
    fn the_strands_stand_out_and_turn_along_the_rope_on_the_desktop_tiers() {
        let (a, b) = ([0.0, 1.0, 0.0], [2.0, 1.0, 0.0]);
        let s = style();
        let line = curve(&[a, b], Lay::Taut, &s);
        let (positions, _, _) = mesh(&line, &s).expect("a rope");
        let ring = |i: usize| &positions[i * s.sides..(i + 1) * s.sides];
        let radius = |p: &[f32; 3], i: usize| length(sub(*p, line[i]));
        let (mut lo, mut hi) = (f32::MAX, 0.0_f32);
        for i in 0..line.len() {
            for p in ring(i) {
                lo = lo.min(radius(p, i));
                hi = hi.max(radius(p, i));
            }
        }
        assert!((lo - ROPE_R_M * (1.0 - LOBE)).abs() < 2.0e-3, "valley {lo}");
        assert!((hi - ROPE_R_M * (1.0 + LOBE)).abs() < 2.0e-3, "crest {hi}");
        // The direction of the farthest point of a ring turns from ring to ring.
        let crest = |i: usize| {
            let far = ring(i)
                .iter()
                .max_by(|p, q| radius(p, i).total_cmp(&radius(q, i)))
                .expect("a ring");
            sub(*far, line[i])
        };
        let turn = dot(unit(crest(0)), unit(crest(3))).clamp(-1.0, 1.0).acos();
        assert!(turn.to_degrees() > 8.0, "{} degrees", turn.to_degrees());
        // The weak tiers draw a plain round tube.
        let potato = RopeStyle::for_tier(Tier::Potato);
        let flat = curve(&[a, b], Lay::Taut, &potato);
        let (round, _, _) = mesh(&flat, &potato).expect("a rope");
        for (i, p) in round.iter().enumerate().take(flat.len() * potato.sides) {
            let r = length(sub(*p, flat[i / potato.sides]));
            assert!((r - ROPE_R_M).abs() < 1.0e-4, "{r}");
        }
    }
}
