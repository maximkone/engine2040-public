//! quick_start: first fight in 4 steps (~60 s tutorial, est_seconds 60).
//!
//! Run: `cargo run --example quick_start`
//! Steps: 4 (1 spawn arena, 2 spawn fighters, 3 run ticks, 4 print hash).
//! Budget: est_seconds 60 of a 900 s tutorial budget; headless, no window.
//! Determinism: fixed seed 2040 plus fixed dt; same seed gives the same hash.
//!
//! One-file minimal-app shape adapted from bevyengine/bevy
//! (MIT/Apache-2.0) examples/app/empty_defaults.rs:5.

use engine_core::{run_ticks, Rng, Tick};
use engine_ecs::{Position, Velocity, World};

/// Fixed match seed: every run replays the same fight and hash.
const SEED: u64 = 2040;
/// Fixed bout length in ticks (1 s at 60 Hz).
const TICKS: u64 = 60;
/// Fixed step in seconds; never wall clock.
const DT: f32 = 1.0 / 60.0;

fn main() {
    // STEP 1: spawn the arena world plus the seeded stream.
    let mut world = World::new();
    let mut rng = Rng::new(SEED);

    // STEP 2: spawn two fighters on fixed marks with a seeded jitter.
    for side in [-5.0_f32, 5.0_f32] {
        let jitter = (rng.next_u64() % 100) as f32 / 100.0 - 0.5;
        world.spawn(
            Position { x: side, y: jitter },
            Velocity {
                x: -side.signum() * 2.0,
                y: 0.0,
            },
        );
    }

    // STEP 3: run the fixed bout; each tick integrates and hashes the state.
    let hashes = run_ticks(TICKS, SEED, |_: Tick, _: &mut Rng| {
        world.integrate(DT);
        world.state_hash()
    });

    // STEP 4: print the stable receipt; reruns must print the same hash.
    println!(
        "quick_start final_hash={:016x} ticks={TICKS} fighters={}",
        hashes.last().copied().unwrap_or(0),
        world.count()
    );
}
