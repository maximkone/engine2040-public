//! island_split pin: off == on hash on settled + kicked stacks.
//!
//! Two far-apart 3-high stacks = two islands, no shared bodies. The
//! island-split path must answer the legacy loop bit-for-bit, settled
//! and after a kick.

use engine_physics::solver::{Config, Solver};
use engine_physics::Body;

const DT: f32 = 1.0 / 60.0;
const SETTLE_STEPS: usize = 900;
const KICK_STEPS: usize = 240;

/// Two far-apart stacks (x = -5 / +5), 3 boxes each, 1.02 m pitch.
fn two_stacks() -> Vec<Body> {
    let mut v = Vec::with_capacity(6);
    for side in [-5.0f32, 5.0] {
        for i in 0..3 {
            v.push(Body::new(side, 0.5 + i as f32 * 1.02, 0.0, 0.0, 0.5, 0.5));
        }
    }
    v
}

fn run_settled(split: bool) -> (Vec<Body>, Solver) {
    let mut cfg = Config::default();
    cfg.island_split = split;
    let mut s = Solver::with_config(cfg);
    let mut b = two_stacks();
    for _ in 0..SETTLE_STEPS {
        s.step(&mut b, DT);
    }
    (b, s)
}

fn run_kicked(split: bool) -> (Vec<Body>, Solver) {
    let (mut b, mut s) = run_settled(split);
    // Kick the top box of each stack; the wake backstop (v2 > sleep_v2)
    // wakes only the kicked islands.
    b[2].vy = 3.0;
    b[5].vy = 3.0;
    for _ in 0..KICK_STEPS {
        s.step(&mut b, DT);
    }
    (b, s)
}

#[test]
fn island_split_settled_stacks_match_hash() {
    assert!(!Config::default().island_split, "flag defaults off");
    let (b0, s0) = run_settled(false);
    let (b1, s1) = run_settled(true);
    assert_eq!(
        s0.state_hash(&b0),
        s1.state_hash(&b1),
        "settled stacks: split path must match legacy hash"
    );
    // Per-stack order kept on both runs (2 -> 3 is the far jump).
    for b in [&b0, &b1] {
        for (lo, hi) in [(0, 1), (1, 2), (3, 4), (4, 5)] {
            assert!(b[hi].y > b[lo].y, "stack order kept: {b:?}");
        }
    }
}

#[test]
fn island_split_kicked_stacks_match_hash() {
    let (b0, s0) = run_kicked(false);
    let (b1, s1) = run_kicked(true);
    assert_eq!(
        s0.state_hash(&b0),
        s1.state_hash(&b1),
        "kicked stacks: split path must match legacy hash"
    );
    for b in [&b0, &b1] {
        for m in b {
            assert!(
                m.x.is_finite() && m.y.is_finite() && m.vx.is_finite() && m.vy.is_finite(),
                "no NaN escape: {m:?}"
            );
        }
    }
}
