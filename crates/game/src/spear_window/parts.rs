//! parts: the spear's own meshes, wood and iron. The shaft is a slim round tube, 2.8 cm across;
//! the iron is a flat leaf head about 30 cm long and 5.6 cm at its widest, set in a round socket,
//! and a pointed shoe on the butt. Both are baked in world space from the tip and the unit
//! direction of the flight, so a spear thrown, held, stuck or fallen always points along its own
//! axis: the tip is exactly the tip and the butt is exactly [`SPEAR_LEN_M`] behind it.
//!
//! Presentation only. Caller: `spear_window::SpearScene::push`. Units: meters, Y up. Faces wind
//! counter-clockwise seen from outside (the renderer culls back faces) and carry the true normals
//! of the shape, so the leaf, the socket and the shoe light as the solids they are.

use super::{add, basis, idx, join, norm as unit, num, scale, sub, tube, Mesh};
use crate::spear_look::SPEAR_LEN_M;
use engine_core::vec3::cross3;
use std::f32::consts::TAU;

/// Radius of the wooden shaft, meters.
pub(super) const SHAFT_R_M: f32 = 0.014;
/// Length of the iron head from its tip to the end of its socket, meters.
pub(super) const HEAD_LEN_M: f32 = 0.30;
/// Length of the iron shoe on the butt, meters.
pub(super) const SHOE_LEN_M: f32 = 0.12;
/// How far the shaft goes into the head's socket and the shoe, meters.
const SEAT_M: f32 = 0.05;
/// Sides of a turned iron ring.
const SIDES: usize = 8;

/// One ring of a turned shape: its distance along the axis from the shape's start, its half
/// width and its half thickness, meters.
type Ring = (f32, f32, f32);

/// The head from its tip back: a flat leaf, widest a third of the way down, then a round socket.
const HEAD: [Ring; 10] = [
    (0.000, 0.000, 0.000),
    (0.025, 0.012, 0.0035),
    (0.060, 0.022, 0.0065),
    (0.105, 0.028, 0.0085),
    (0.150, 0.026, 0.0090),
    (0.195, 0.018, 0.0095),
    (0.228, 0.012, 0.0100),
    (0.235, 0.019, 0.0190),
    (0.270, 0.0175, 0.0175),
    (0.300, 0.0165, 0.0165),
];

/// The butt shoe from the very end of the spear forward: a blunt point, then a sleeve.
const SHOE: [Ring; 5] = [
    (0.000, 0.000, 0.000),
    (0.006, 0.012, 0.012),
    (0.020, 0.0165, 0.0165),
    (0.100, 0.0165, 0.0165),
    (0.120, 0.0145, 0.0145),
];

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// How fast `pick` changes along the axis at ring `i`, from its neighbors.
fn slope(rings: &[Ring], i: usize, pick: fn(&Ring) -> f32) -> f32 {
    let lo = rings[i.saturating_sub(1)];
    let hi = rings[(i + 1).min(rings.len() - 1)];
    let run = hi.0 - lo.0;
    if run.abs() < 1.0e-6 {
        0.0
    } else {
        (pick(&hi) - pick(&lo)) / run
    }
}

/// `rings` turned about the line from `origin` along the unit `axis`, wider along `e1` than
/// along `e2`, with outward normals and outward winding whichever way `e1 x e2` faces.
fn lathe(origin: [f32; 3], axis: [f32; 3], (e1, e2): ([f32; 3], [f32; 3]), rings: &[Ring]) -> Mesh {
    let mut positions = Vec::with_capacity(rings.len() * SIDES);
    let mut normals = Vec::with_capacity(rings.len() * SIDES);
    for (i, &(at, a, b)) in rings.iter().enumerate() {
        let (da, db) = (slope(rings, i, |r| r.1), slope(rings, i, |r| r.2));
        let centre = add(origin, scale(axis, at));
        for k in 0..SIDES {
            let (s, c) = (num(k) / num(SIDES) * TAU).sin_cos();
            let radial = add(scale(e1, a * c), scale(e2, b * s));
            positions.push(add(centre, radial));
            let around = add(scale(e1, -a * s), scale(e2, b * c));
            let along = add(axis, add(scale(e1, da * c), scale(e2, db * s)));
            let out = cross3(around, along);
            normals.push(if a + b <= 1.0e-6 {
                scale(axis, -1.0)
            } else if dot(out, radial) < 0.0 {
                unit(scale(out, -1.0))
            } else {
                unit(out)
            });
        }
    }
    let flip = dot(cross3(e1, e2), axis) < 0.0;
    let mut indices = Vec::with_capacity((rings.len() - 1) * SIDES * 6);
    for ring in 0..rings.len() - 1 {
        for k in 0..SIDES {
            let (a, b) = (ring * SIDES + k, ring * SIDES + (k + 1) % SIDES);
            let (c, d) = (a + SIDES, b + SIDES);
            let quad = if flip {
                [a, c, b, b, c, d]
            } else {
                [a, b, c, b, d, c]
            };
            indices.extend(quad.map(idx));
        }
    }
    (positions, normals, indices)
}

/// The wooden part of a spear: the shaft from the shoe to the socket.
pub(super) fn shaft_mesh(tip: [f32; 3], dir: [f32; 3]) -> Mesh {
    let tail = sub(tip, scale(dir, SPEAR_LEN_M));
    let from = add(tail, scale(dir, SHOE_LEN_M - SEAT_M));
    let to = sub(tip, scale(dir, HEAD_LEN_M - SEAT_M));
    tube(&[from, to], &|_| SHAFT_R_M, 0.0)
}

/// The iron of a spear: the leaf head with its socket, and the shoe on the butt. `roll` turns the
/// flat of the blade about the axis.
pub(super) fn head_mesh(tip: [f32; 3], dir: [f32; 3], roll: f32) -> Mesh {
    let frame = basis(dir, roll);
    let tail = sub(tip, scale(dir, SPEAR_LEN_M));
    join(vec![
        lathe(tip, scale(dir, -1.0), frame, &HEAD),
        lathe(tail, dir, frame, &SHOE),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spear_window::to_mesh;

    fn length(v: [f32; 3]) -> f32 {
        dot(v, v).sqrt()
    }

    /// Where `p` sits along the spear and how far off its axis, meters.
    fn along_and_off(p: [f32; 3], tip: [f32; 3], dir: [f32; 3]) -> (f32, f32) {
        let rel = sub(p, tip);
        let along = dot(rel, dir);
        (along, length(sub(rel, scale(dir, along))))
    }

    fn all_parts(tip: [f32; 3], dir: [f32; 3], roll: f32) -> Vec<Mesh> {
        vec![shaft_mesh(tip, dir), head_mesh(tip, dir, roll)]
    }

    #[test]
    fn the_spear_has_its_length_and_slim_proportions_along_its_flight() {
        for (tip, flight) in [
            ([1.0, 1.3, -2.0], [0.5, 0.1, 0.8]),
            ([0.0, 0.4, 0.0], [0.0, -0.9, 0.2]),
            ([4.0, 2.5, 1.0], [-0.7, 0.3, -0.2]),
            ([0.0, 1.0, 0.0], [0.0, 1.0, 0.0]),
        ] {
            let dir = unit(flight);
            let (mut far, mut near, mut wide) = (0.0_f32, f32::MAX, 0.0_f32);
            for (positions, _, _) in all_parts(tip, dir, 0.7) {
                for p in positions {
                    let (along, off) = along_and_off(p, tip, dir);
                    far = far.max(-along);
                    near = near.min(-along);
                    wide = wide.max(off);
                    assert!(off < 0.031, "{p:?} is {off} m off the axis");
                }
            }
            assert!(near.abs() < 1.0e-5, "the tip is the nearest point: {near}");
            assert!(
                (far - SPEAR_LEN_M).abs() < 1.0e-4,
                "the butt is {far} m behind the tip"
            );
            assert!(wide > 0.025, "the head is wider than the shaft: {wide}");
        }
        // The blade is slender: a leaf at least four and a half times as long as it is wide.
        let blade_max = HEAD.iter().map(|r| r.1).fold(0.0, f32::max);
        assert!(HEAD_LEN_M / (2.0 * blade_max) > 4.5, "a leaf, not a cone");
    }

    // About a sixth of the spear is iron.
    const _: () = assert!(HEAD_LEN_M > 0.25 && HEAD_LEN_M < 0.4 && SPEAR_LEN_M > 5.0 * HEAD_LEN_M);

    #[test]
    fn the_tip_is_the_flights_point_and_the_head_stays_flat_across_the_roll() {
        let (tip, dir) = ([1.0, 1.3, -2.0], unit([0.5, 0.1, 0.8]));
        let (positions, _, _) = head_mesh(tip, dir, 0.0);
        assert!(positions.contains(&tip));
        // The blade is wide along one axis and thin along the other: rolling turns it.
        let flat = |roll: f32| {
            let (positions, _, _) = head_mesh(tip, dir, roll);
            let widest = positions
                .iter()
                .max_by(|p, q| {
                    along_and_off(**p, tip, dir)
                        .1
                        .total_cmp(&along_and_off(**q, tip, dir).1)
                })
                .expect("a vertex");
            unit(sub(
                *widest,
                add(tip, scale(dir, dot(sub(*widest, tip), dir))),
            ))
        };
        let (still, turned) = (flat(0.0), flat(std::f32::consts::FRAC_PI_2));
        assert!(
            dot(still, turned).abs() < 0.2,
            "a quarter roll turns the blade a quarter"
        );
    }

    #[test]
    fn every_spear_part_is_a_valid_mesh_that_faces_outward() {
        let (tip, dir) = ([1.0, 1.3, -2.0], unit([0.5, 0.1, 0.8]));
        for (name, shape) in [
            ("shaft", shaft_mesh(tip, dir)),
            ("head", head_mesh(tip, dir, 0.7)),
            ("head up", head_mesh([0.0, 2.0, 0.0], [0.0, 1.0, 0.0], 0.0)),
            (
                "head down",
                head_mesh([0.0, 2.0, 0.0], [0.0, -1.0, 0.0], 0.3),
            ),
        ] {
            let (positions, normals, indices) = &shape;
            let mut outward = 0;
            let mut seen = 0;
            for tri in indices.chunks(3) {
                let at = |i: u32| positions[usize::try_from(i).expect("index")];
                let (p0, p1, p2) = (at(tri[0]), at(tri[1]), at(tri[2]));
                let face = cross3(sub(p1, p0), sub(p2, p0));
                if length(face) < 1.0e-9 {
                    continue;
                }
                let mean = tri.iter().fold([0.0; 3], |sum, &i| {
                    add(sum, normals[usize::try_from(i).expect("index")])
                });
                seen += 1;
                outward += i32::from(dot(face, mean) > 0.0);
            }
            assert!(seen >= 16, "{name}: {seen} faces");
            assert_eq!(outward, seen, "{name}: every face winds outward");
            assert!(normals.iter().all(|n| (length(*n) - 1.0).abs() < 1.0e-3));
            to_mesh(shape).unwrap_or_else(|err| panic!("{name}: {err}"));
        }
    }
}
