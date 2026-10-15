//! Broadphase pin: known layout answers one exact pair set, and the
//! broadphase query path leaves the solver step hash untouched.
//!
//! Owner: physics `broadphase` (uniform grid + SAP sweep + BVH walk).
//! Units: meters, seconds. Deterministic: same bodies give same pairs
//! and same hash; no wall clock or hash-seed order on the path.

use engine_physics::broadphase::{bvh_pairs, candidate_pairs, sweep_pairs};
use engine_physics::solver::{Config, Solver};
use engine_physics::Body;

const DT: f32 = 1.0 / 60.0;
const CELL: f32 = 2.0;

/// Four boxes: 0-1 touch near the origin, 2 far away, 3 far in y.
/// Only (0, 1) shares both a grid cell and a true AABB overlap.
fn layout() -> Vec<Body> {
    vec![
        Body::new(0.0, 0.0, 0.0, 0.0, 0.5, 0.5),
        Body::new(0.4, 0.0, 0.0, 0.0, 0.5, 0.5),
        Body::new(50.0, 50.0, 0.0, 0.0, 0.5, 0.5),
        Body::new(0.0, 50.0, 0.0, 0.0, 0.5, 0.5),
    ]
}

fn grid_of(bodies: &[Body]) -> Vec<(f32, f32, f32, f32)> {
    bodies.iter().map(|b| (b.x, b.y, b.hx, b.hy)).collect()
}

fn x_bounds(bodies: &[Body]) -> (Vec<f32>, Vec<f32>) {
    let mut mins = Vec::with_capacity(bodies.len());
    let mut maxs = Vec::with_capacity(bodies.len());
    for b in bodies {
        mins.push(b.x - b.hx);
        maxs.push(b.x + b.hx);
    }
    (mins, maxs)
}

/// Strict AABB overlap oracle: the same `<` gate the solver uses.
fn brute_pairs(bodies: &[Body]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for i in 0..bodies.len() {
        for j in (i + 1)..bodies.len() {
            if (bodies[i].x - bodies[j].x).abs() < bodies[i].hx + bodies[j].hx
                && (bodies[i].y - bodies[j].y).abs() < bodies[i].hy + bodies[j].hy
            {
                out.push((i, j));
            }
        }
    }
    out
}

fn solver_no_gravity() -> Solver {
    let mut cfg = Config::default();
    cfg.gravity = 0.0;
    Solver::with_config(cfg)
}

/// Step fixture: an overlapping pair plus two far bodies, high off the
/// ground so the grounded infinite-mass rule stays out. Body 0 drifts
/// so the movers-only swept query sees the pair.
fn step_bodies() -> Vec<Body> {
    vec![
        Body::new(0.0, 5.0, 1.0, 0.0, 0.5, 0.5),
        Body::new(0.9, 5.0, 0.0, 0.0, 0.5, 0.5),
        Body::new(50.0, 50.0, 0.0, 0.0, 0.5, 0.5),
        Body::new(0.0, 50.0, 0.0, 0.0, 0.5, 0.5),
    ]
}

#[test]
fn known_layout_answers_exact_pair_set() {
    let bodies = layout();
    let grid = grid_of(&bodies);
    assert_eq!(candidate_pairs(&grid, CELL), vec![(0, 1)]);
    assert_eq!(brute_pairs(&bodies), vec![(0, 1)]);
    assert_eq!(candidate_pairs(&grid, CELL), candidate_pairs(&grid, CELL));
    for (a, b) in candidate_pairs(&grid, CELL) {
        assert!(a < 2 && b < 2, "far body leaked: ({a}, {b})");
    }
}

#[test]
fn sweep_and_bvh_match_exact_x_overlaps() {
    let bodies = layout();
    let (mins, maxs) = x_bounds(&bodies);
    let want = vec![(0, 1), (0, 3), (1, 3)];
    assert_eq!(sweep_pairs(&mins, &maxs), want);
    assert_eq!(bvh_pairs(&mins, &maxs), want);
    assert_eq!(sweep_pairs(&mins, &maxs), bvh_pairs(&mins, &maxs));
    let moving = step_bodies();
    assert_eq!(brute_pairs(&moving), vec![(0, 1)], "strict oracle pins the pair");
    assert!(!brute_pairs(&moving).iter().any(|(a, b)| *a >= 2 || *b >= 2), "far body leaked");
}

#[test]
fn broadphase_path_leaves_step_hash_untouched() {
    fn reference() -> (Vec<Body>, u64) {
        let mut solver = solver_no_gravity();
        let mut bodies = step_bodies();
        for _ in 0..120 {
            solver.step(&mut bodies, DT);
        }
        let hash = solver.state_hash(&bodies);
        (bodies, hash)
    }
    fn probed() -> (Vec<Body>, u64) {
        let mut solver = solver_no_gravity();
        let mut bodies = step_bodies();
        for _ in 0..120 {
            let grid = grid_of(&bodies);
            let candidates = candidate_pairs(&grid, CELL);
            let (mins, maxs) = x_bounds(&bodies);
            let sweep = sweep_pairs(&mins, &maxs);
            let bvh = bvh_pairs(&mins, &maxs);
            assert_eq!(sweep, bvh, "sweep and BVH must agree each tick");
            let brute = brute_pairs(&bodies);
            for pair in &brute {
                assert!(candidates.contains(pair), "grid dropped true pair {pair:?}: {candidates:?}");
                assert!(sweep.contains(pair), "sweep dropped true pair {pair:?}");
            }
            solver.step(&mut bodies, DT);
        }
        let hash = solver.state_hash(&bodies);
        (bodies, hash)
    }
    let (plain_bodies, plain_hash) = reference();
    let (probe_bodies, probe_hash) = probed();
    assert_eq!(plain_hash, probe_hash, "broadphase queries must not move the step hash");
    for (i, (plain, probe)) in plain_bodies.iter().zip(probe_bodies.iter()).enumerate() {
        assert_eq!((plain.x.to_bits(), plain.y.to_bits(), plain.vx.to_bits(), plain.vy.to_bits()), (probe.x.to_bits(), probe.y.to_bits(), probe.vx.to_bits(), probe.vy.to_bits()), "body {i} drifted under the broadphase path");
    }
    let (_, again) = reference();
    assert_eq!(plain_hash, again, "step hash must repeat");
}
