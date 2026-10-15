//! headless example: 120 ticks, prints final hash.
//! `cargo run --example headless`
use engine_core::{run_ticks, Rng, Tick};
use engine_ecs::{Position, Velocity, World};

fn main() {
    let seed = 2040u64;
    let mut world = World::new();
    let mut rng = Rng::new(seed);
    for _ in 0..10 {
        world.spawn(
            Position {
                x: (rng.next_u64() % 50) as f32,
                y: 0.0,
            },
            Velocity { x: 2.0, y: 1.0 },
        );
    }
    let hashes = run_ticks(120, seed, |_: Tick, _r: &mut Rng| {
        world.integrate(1.0 / 60.0);
        world.state_hash()
    });
    println!(
        "example final_hash={:016x} entities={}",
        hashes.last().unwrap(),
        world.count()
    );
}
