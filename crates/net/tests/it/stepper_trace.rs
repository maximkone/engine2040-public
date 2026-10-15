//! Lockstep tick_step keypoint trace: 30 authoritative ticks, hash per tick.
//!
//! Port-fresh tribute to cBournhonesque/lightyear (Apache-2.0),
//! crates/tests/src/stepper.rs:879 (tick_step):
//! https://github.com/cBournhonesque/lightyear
//! Licence verified as Apache-2.0 via LICENSE-APACHE raw text. No code copied.

use engine_arena::{ArenaConfig, Checkpoint};
use engine_input::fighter::Buttons;
use engine_input::{FighterId, FighterInput};
use engine_net::Server;

const TICKS: u32 = 30;

fn input(tick: u32) -> FighterInput {
    FighterInput {
        tick,
        fighter: FighterId::new(0).expect("slot 0"),
        mx: if tick % 20 < 10 { 100 } else { -100 },
        mz: 0,
        facing: 0,
        buttons: Buttons::NONE,
    }
}

fn trace() -> Vec<Checkpoint> {
    let config = ArenaConfig {
        seed: 7,
        ticks: TICKS,
        fighters: 2,
        half_extent_m: 20.0,
        // engine_combat::tables::TABLES_VERSION_V1 (value 1); not reachable from engine-net tests.
        table_version: 1,
    };
    let mut server = Server::new(config).expect("server opens");
    let mut out = Vec::with_capacity(TICKS as usize);
    for tick in 0..TICKS {
        let (point, _) = server.tick(&[input(tick)]).expect("tick authors");
        assert_eq!(point.tick, tick, "tick steps in order");
        assert_ne!(point.hash, 0, "per-tick hash is keyed");
        out.push(point);
    }
    out
}

#[test]
fn lockstep_tick_step_30_keypoints() {
    let first = trace();
    let second = trace();
    assert_eq!(
        (first.len(), second.len()),
        (TICKS as usize, TICKS as usize)
    );
    for (tick, (a, b)) in first.iter().zip(&second).enumerate() {
        assert_eq!(a.hash, b.hash, "per-tick hash repeats at tick {tick}");
    }
    for key in [0, 15, 29] {
        assert_eq!(first[key], second[key], "keypoint {key} repeats");
    }
}
