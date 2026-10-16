//! physics_solver: deterministic tick-hash plus island and sleep pin.
//! Owner: engine-physics solver. Units: meters, seconds, hash u64.
//! Determinism: same bodies plus same steps give same hash.
//! Errors: none; asserts fail loudly on drift or missed sleep.

use engine_physics::solver::{Config, Solver};
use engine_physics::Body;

const DT: f32 = 1.0 / 60.0;
const SETTLE: usize = 900;
const EXTRA: usize = 120;

/// Three-high stack at x 0, 1.02 m pitch.
fn stack3() -> Vec<Body> {
    vec![
        Body::new(0.0, 0.5, 0.0, 0.0, 0.5, 0.5),
        Body::new(0.0, 1.52, 0.0, 0.0, 0.5, 0.5),
        Body::new(0.0, 2.54, 0.0, 0.0, 0.5, 0.5),
    ]
}

/// Two far-apart 3-high stacks: two islands, no shared bodies.
fn two_stacks() -> Vec<Body> {
    vec![
        Body::new(-5.0, 0.5, 0.0, 0.0, 0.5, 0.5),
        Body::new(-5.0, 1.52, 0.0, 0.0, 0.5, 0.5),
        Body::new(-5.0, 2.54, 0.0, 0.0, 0.5, 0.5),
        Body::new(5.0, 0.5, 0.0, 0.0, 0.5, 0.5),
        Body::new(5.0, 1.52, 0.0, 0.0, 0.5, 0.5),
        Body::new(5.0, 2.54, 0.0, 0.0, 0.5, 0.5),
    ]
}

fn settle(mut bodies: Vec<Body>, steps: usize) -> (Vec<Body>, Solver) {
    let mut solver = Solver::new();
    for _ in 0..steps {
        solver.step(&mut bodies, DT);
    }
    (bodies, solver)
}

#[test]
fn tick_hash_stable_across_runs() {
    let (bodies_a, solver_a) = settle(stack3(), SETTLE);
    let (bodies_b, solver_b) = settle(stack3(), SETTLE);
    assert_eq!(solver_a.sleeping_count(), 3, "stack must sleep");
    assert_eq!(
        solver_a.state_hash(&bodies_a),
        solver_b.state_hash(&bodies_b),
        "same bodies plus same steps give same hash"
    );
    let pinned = solver_a.state_hash(&bodies_a);
    let (mut bodies_a, mut solver_a) = (bodies_a, solver_a);
    for _ in 0..EXTRA {
        solver_a.step(&mut bodies_a, DT);
    }
    assert_eq!(
        solver_a.state_hash(&bodies_a),
        pinned,
        "sleeping stack holds its hash"
    );
    assert!(bodies_a[1].y > bodies_a[0].y, "stack order kept");
    assert!(bodies_a[2].y > bodies_a[1].y, "stack order kept");
    for body in &bodies_a {
        assert!(
            body.x.abs() < 0.05 && body.y >= body.hy - 0.01,
            "no tunnel or explosion: {body:?}"
        );
        assert!(
            body.x.is_finite()
                && body.y.is_finite()
                && body.vx.is_finite()
                && body.vy.is_finite(),
            "no NaN escape: {body:?}"
        );
    }
}

#[test]
fn sleep_wake_transitions() {
    let mut solver = Solver::new();
    let mut bodies = two_stacks();
    assert_eq!(solver.sleeping_count(), 0, "fresh solver stays awake");
    for idx in 0..bodies.len() {
        assert!(!solver.is_sleeping(idx), "fresh body stays awake");
    }
    for _ in 0..SETTLE {
        solver.step(&mut bodies, DT);
    }
    assert_eq!(solver.sleeping_count(), 6, "both islands must sleep");
    assert!(
        solver.sleeping_mask().iter().all(|wake| *wake),
        "sleep mask all true"
    );
    let pinned = solver.state_hash(&bodies);
    bodies[2].vy = 3.0;
    solver.step(&mut bodies, DT);
    assert!(!solver.is_sleeping(2), "kick wakes its island");
    assert!(
        solver.state_hash(&bodies) != pinned,
        "kicked state must move the hash"
    );
    solver.wake(0);
    assert!(!solver.is_sleeping(0), "wake clears one sleeper");
    solver.wake_all();
    assert_eq!(solver.sleeping_count(), 0, "wake_all clears sleep");
}

#[test]
fn island_split_matches_legacy_hash() {
    assert!(!Config::default().island_split, "split path defaults off");
    fn run(split: bool) -> (Vec<Body>, Solver) {
        let mut cfg = Config::default();
        cfg.island_split = split;
        let mut solver = Solver::with_config(cfg);
        let mut bodies = two_stacks();
        for _ in 0..240 {
            solver.step(&mut bodies, DT);
        }
        (bodies, solver)
    }
    let (plain_bodies, plain) = run(false);
    let (split_bodies, split) = run(true);
    assert_eq!(
        plain.state_hash(&plain_bodies),
        split.state_hash(&split_bodies),
        "split path must match legacy hash"
    );
    for bodies in [&plain_bodies, &split_bodies] {
        for (low, high) in [
            (0_usize, 1_usize),
            (1_usize, 2_usize),
            (3_usize, 4_usize),
            (4_usize, 5_usize),
        ] {
            assert!(bodies[high].y > bodies[low].y, "stack order kept");
        }
    }
}
