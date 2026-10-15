// Attribution: donor erincatto/box2d-lite (MIT, verified, abandoned):
// warm-started sequential-impulse island-solver idea
// (accumulated normal impulses re-applied next step, contacts solved
// island by island in a fixed order). License verified MIT; the donor is
// abandoned upstream. Fresh write below against the engine-physics
// `Solver`/`broadphase` API; no donor code is copied. The simulation hash
// stays on its existing stable path: this file only times and compares.

use engine_physics::broadphase;
use engine_physics::solver::{Config, Solver};
use engine_physics::Body;
use std::time::Instant;

/// 256 bodies as 8 far-apart stacks of 32: 8 contact islands, no shared
/// bodies, so the island-split path must answer the legacy loop bit-for-bit.
const STACKS: usize = 8;
const PER_STACK: usize = 32;
const BODIES: usize = STACKS * PER_STACK;
/// Steps per timed run at 60 Hz.
const STEPS: usize = 120;
const DT: f32 = 1.0 / 60.0;
/// Repetitions per path; the minimum wins to cut machine noise.
const REPS: usize = 3;

/// Deterministic 256-body drop: columns 6 m apart, boxes 1.02 m apart in y.
fn fixture() -> Vec<Body> {
    let mut out = Vec::with_capacity(BODIES);
    for side in 0..STACKS {
        let x = (side as f32 - (STACKS as f32 - 1.0) / 2.0) * 6.0;
        for i in 0..PER_STACK {
            out.push(Body {
                x,
                y: 0.5 + i as f32 * 1.02,
                vx: 0.0,
                vy: 0.0,
                hx: 0.5,
                hy: 0.5,
            });
        }
    }
    out
}

fn solver(split: bool) -> Solver {
    let mut cfg = Config::default();
    cfg.island_split = split;
    Solver::with_config(cfg)
}

/// Run `STEPS` ticks from the fixture; answer the final state hash.
fn run_steps(split: bool) -> (Vec<Body>, u64) {
    let mut s = solver(split);
    let mut b = fixture();
    for _ in 0..STEPS {
        s.step(&mut b, DT);
    }
    let h = s.state_hash(&b);
    (b, h)
}

/// Best-of-`REPS` wall time for one full run; never part of the tick.
fn best_secs(split: bool) -> (f64, u64) {
    let mut best = f64::MAX;
    let mut hash = 0_u64;
    for _ in 0..REPS {
        let start = Instant::now();
        let (_, h) = run_steps(split);
        let secs = start.elapsed().as_secs_f64();
        hash = h;
        if secs < best {
            best = secs;
        }
    }
    (best, hash)
}

#[test]
fn broadphase_sees_the_stacks() {
    let b = fixture();
    assert_eq!(b.len(), BODIES, "fixture must hold 256 bodies");
    let tuples: Vec<(f32, f32, f32, f32)> = b.iter().map(|m| (m.x, m.y, m.hx, m.hy)).collect();
    let pairs = broadphase::candidate_pairs(&tuples, 2.0);
    assert!(!pairs.is_empty(), "stacked cells must yield candidates");
    assert_eq!(
        pairs,
        broadphase::candidate_pairs(&tuples, 2.0),
        "broadphase must repeat exactly"
    );
}

#[test]
fn island_split_matches_legacy_hash_256() {
    let (legacy, legacy_hash) = run_steps(false);
    let (split, split_hash) = run_steps(true);
    assert_eq!(
        legacy_hash, split_hash,
        "island-split must answer the legacy loop bit-for-bit"
    );
    // Same seed + steps = same hash on a second pass of each path.
    assert_eq!(run_steps(false).1, legacy_hash, "legacy hash x2");
    assert_eq!(run_steps(true).1, split_hash, "split hash x2");
    for bodies in [&legacy, &split] {
        for m in bodies {
            assert!(m.x.is_finite() && m.y.is_finite(), "no NaN escape: {m:?}");
            assert!(m.y >= m.hy - 0.05, "no tunneling: {m:?}");
        }
    }
}

#[test]
fn seq_impulse_bench_256_before_after() {
    let (before_secs, before_hash) = best_secs(false);
    let (after_secs, after_hash) = best_secs(true);
    assert_eq!(
        before_hash, after_hash,
        "timed runs must agree on the state hash"
    );
    assert_ne!(before_hash, 0, "hash must observe real state");
    let steps_f = STEPS as f64;
    let before_ms = before_secs * 1e3 / steps_f;
    let after_ms = after_secs * 1e3 / steps_f;
    let gain_pct = (before_ms - after_ms) / before_ms * 100.0;
    if after_ms <= before_ms {
        println!(
            "passed seq_impulse_bench_256_before_after: legacy {before_ms:.3} ms/step \
             before->after island-split {after_ms:.3} ms/step; gain {gain_pct:.1}% \
             over {STEPS} steps x {BODIES} bodies; hash {before_hash:016x}"
        );
    } else {
        println!(
            "NOOP seq_impulse_bench_256_before_after: no speedup: legacy {before_ms:.3} ms/step \
             before->after island-split {after_ms:.3} ms/step; gain {gain_pct:.1}% \
             over {STEPS} steps x {BODIES} bodies; hash {before_hash:016x}"
        );
    }
}

#[test]
fn nan_body_steps_without_panic_and_hash_stable() {
    fn with_nan() -> Vec<Body> {
        let mut b = fixture();
        b.push(Body {
            x: f32::NAN,
            y: f32::NAN,
            vx: 0.0,
            vy: 0.0,
            hx: 0.5,
            hy: 0.5,
        });
        b
    }
    fn run_once() -> u64 {
        let mut s = Solver::new();
        let mut b = with_nan();
        for _ in 0..STEPS {
            s.step(&mut b, DT);
        }
        s.state_hash(&b)
    }
    assert_eq!(
        run_once(),
        run_once(),
        "NaN body must step deterministically, never panic"
    );
}
