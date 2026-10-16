//! headless_ticks bench: vectorized env throughput plus per-env hashes.
//!
//! Fixture: seed 2040 plus index, 600 ticks, 2 fighters, 20 m extent, the
//! same push-both tape every env carries in `vec_parity`. Prints
//! `N envs x M ticks = K ticks/s` for 1/8/32 envs plus the two-run per-env
//! hash table; the two runs must agree or the bench fails.

use std::time::Instant;

use engine_arena::{vec_step, ArenaConfig, BatchEnv, FighterId, FighterTape};
use engine_input::fighter::Buttons;

const TICKS: u32 = 600;
const FIGHTERS: u8 = 2;
const HALF_M: f32 = 20.0;

fn env(seed: u64) -> BatchEnv {
    let config = ArenaConfig {
        seed,
        ticks: TICKS,
        fighters: FIGHTERS,
        half_extent_m: HALF_M,
        table_version: engine_combat::tables::TABLES_VERSION_V1,
    };
    let mut inputs = Vec::new();
    for tick in 0..TICKS {
        for (fighter, mx, buttons) in [(0u8, 60i8, u8::from(tick % 4 == 0)), (1u8, -60i8, 0u8)] {
            inputs.push(engine_arena::FighterInput {
                tick,
                fighter: FighterId::new(fighter).expect("bench slots 0..2 are legal"),
                mx,
                mz: 0,
                facing: 0,
                buttons: Buttons(buttons),
            });
        }
    }
    inputs.sort_by_key(|input| (input.tick, input.fighter.0));
    BatchEnv {
        config,
        tape: FighterTape(inputs),
    }
}

fn run_once(count: usize) -> (Vec<u64>, f64) {
    let envs: Vec<BatchEnv> = (0..count).map(|index| env(2040 + index as u64)).collect();
    let start = Instant::now();
    let snaps = vec_step(&envs);
    let elapsed = start.elapsed().as_secs_f64();
    let mut hashes = Vec::with_capacity(snaps.len());
    for snap in &snaps {
        let shot = snap.as_ref().expect("bench fixture envs must simulate");
        hashes.push(shot.digest);
    }
    (hashes, elapsed)
}

fn main() {
    for count in [1usize, 8, 32] {
        let (first, first_s) = run_once(count);
        let (second, second_s) = run_once(count);
        assert_eq!(first, second, "per-env hashes must replay identically");
        assert!(
            first.iter().all(|hash| *hash != 0),
            "600 ticks must move every digest"
        );
        let total = count as f64 * f64::from(TICKS);
        let rate = total / first_s.max(f64::EPSILON);
        println!(
            "{count} envs x {TICKS} ticks = {rate:.1} ticks/s ({total:.0} ticks in {first_s:.3}s)"
        );
        println!("second run: {second_s:.3}s (replay check)");
        for (index, hash) in first.iter().enumerate() {
            println!("env[{index}] seed={} hash={hash:016x}", 2040 + index as u64);
        }
        let _ = second;
    }
}
