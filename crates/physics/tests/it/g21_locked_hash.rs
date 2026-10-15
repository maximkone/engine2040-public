//! G-21: locked-step hash test. Same seed + 1000 fixed-dt steps = same hash chain.

use engine_physics::solver::Solver;
use engine_physics::Body;

/// Fixed step. One rate only. Never varies.
const DT: f32 = 1.0 / 60.0;
/// Locked step count for this test.
const STEPS: usize = 1000;

/// Same seed every call. Plain math from the index. No randomness.
fn seed_bodies() -> Vec<Body> {
    (0..8)
        .map(|i| {
            let k = i as f32;
            Body::new(
                (k * 0.37) % 4.0 - 2.0,
                2.0 + k * 1.1,
                (k * 0.13) % 2.0 - 1.0,
                0.0,
                0.5,
                0.5,
            )
        })
        .collect()
}

/// Run 1000 fixed steps. Fold each step hash into one chain value.
fn run_chain() -> (u64, u64) {
    let mut solver = Solver::new();
    let mut bodies = seed_bodies();
    // Simple FNV-style fold. Same inputs give same output.
    let mut chain: u64 = 0xcbf29ce484222325;
    for _ in 0..STEPS {
        solver.step(&mut bodies, DT);
        chain ^= solver.state_hash(&bodies);
        chain = chain.wrapping_mul(0x100000001b3);
    }
    (chain, solver.state_hash(&bodies))
}

#[test]
fn locked_step_1000_same_seed_same_chain() {
    let (chain_a, final_a) = run_chain();
    let (chain_b, final_b) = run_chain();
    assert_eq!(
        chain_a, chain_b,
        "same seed + 1000 fixed steps = same chain"
    );
    assert_eq!(
        final_a, final_b,
        "same seed + 1000 fixed steps = same final hash"
    );
    assert_ne!(chain_a, 0, "chain must move");
}
