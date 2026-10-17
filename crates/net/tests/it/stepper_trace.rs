//! Lockstep tick_step keypoint trace: 30 authoritative ticks, hash per tick;
//! plus the 600-tick step-count receipt (ponytail findings 1 and 4): the
//! server and the predicting client each hold one live sim stepped once per
//! tick, so 600 ticks cost 600 steps and every checkpoint is byte-equal to
//! the arena prefix oracle.
//!
//! Port-fresh tribute to cBournhonesque/lightyear (Apache-2.0),
//! crates/tests/src/stepper.rs:879 (tick_step):
//! https://github.com/cBournhonesque/lightyear
//! Licence verified as Apache-2.0 via LICENSE-APACHE raw text. No code copied.

use engine_arena::{run, ArenaConfig, Checkpoint, FighterTape};
use engine_input::fighter::Buttons;
use engine_input::{FighterId, FighterInput};
use engine_net::{Client, Server};

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

/// The incremental compare names exactly the ticks the old full-prefix
/// compare named: a lost input keeps its tick in the list on the next state
/// (no resimulation yet), and resimulation then converges to server truth.
#[test]
fn mispredicts_name_the_lost_tick_until_resimulation_converges() {
    let config = ArenaConfig {
        seed: 7,
        ticks: 30,
        fighters: 2,
        half_extent_m: 20.0,
        table_version: 1,
    };
    // A wish plus an attack the arena brain fill will not reproduce.
    let distinct = |tick: u32| FighterInput {
        tick,
        fighter: FighterId::new(0).expect("slot 0"),
        mx: 100,
        mz: 100,
        facing: 9_000,
        buttons: Buttons(engine_input::fighter::ATTACK),
    };
    let mut server = Server::new(config).expect("server opens");
    let mut client = Client::new(config, FighterId::new(0).expect("slot 0")).expect("client opens");
    // Tick 0 matches: the state names nothing.
    let (c0, d0) = server.tick(&[distinct(0)]).expect("tick 0");
    client.predict(distinct(0)).expect("predict 0");
    assert_eq!(
        client.on_server_state(c0, &d0).expect("apply"),
        Vec::<u32>::new()
    );
    client.resimulate();
    // Tick 1's input is lost: the server AI-fills it, the client predicted it.
    let (c1, d1) = server.tick(&[]).expect("tick 1 ai-filled");
    client.predict(distinct(1)).expect("predict 1");
    assert_eq!(
        client.on_server_state(c1, &d1).expect("apply"),
        vec![1],
        "the lost tick is named"
    );
    // Without resimulation the next state names the same divergence again,
    // exactly like the old full-prefix compare.
    let (c2, d2) = server.tick(&[distinct(2)]).expect("tick 2");
    client.predict(distinct(2)).expect("predict 2");
    assert_eq!(
        client.on_server_state(c2, &d2).expect("apply"),
        vec![1, 2],
        "the divergent prefix stays named"
    );
    // Resimulation converges the history onto server truth.
    client.resimulate();
    assert_eq!(client.checkpoints()[..3], server.checkpoints()[..3]);
}
/// The finding-1 cost proof (finding 4): 600 ticks cost exactly 600 sim
/// steps on the authority and 600 on the predictor — a from-zero replay per
/// tick would show far more — and every per-tick checkpoint is byte-equal to
/// the arena prefix oracle. The same inputs give the same checkpoints twice.
#[test]
fn six_hundred_ticks_cost_six_hundred_steps_and_match_the_oracle() {
    const LONG_TICKS: u32 = 600;
    let config = ArenaConfig {
        seed: 2040,
        ticks: LONG_TICKS,
        fighters: 2,
        half_extent_m: 20.0,
        table_version: 1,
    };
    let tape = FighterTape((0..LONG_TICKS).map(input).collect());
    let oracle = run(&config, &tape);
    assert_eq!(oracle.len(), LONG_TICKS as usize, "the oracle runs");

    // Authority: one step per authored tick, byte-equal to the oracle.
    let mut server = Server::new(config).expect("server opens");
    let mut last_delta = None;
    for tick in 0..LONG_TICKS {
        let (point, delta) = server.tick(&[input(tick)]).expect("tick authors");
        assert_eq!(point, oracle[tick as usize], "server checkpoint {tick}");
        last_delta = Some(delta);
    }
    assert_eq!(
        server.steps(),
        u64::from(LONG_TICKS),
        "one sim step per authored tick"
    );
    assert_eq!(
        server.checkpoints(),
        oracle.as_slice(),
        "authority matches the prefix oracle"
    );

    // Predictor: one step per predicted tick, byte-equal to the oracle.
    let mut client = Client::new(config, FighterId::new(0).expect("slot 0")).expect("client opens");
    for tick in 0..LONG_TICKS {
        let point = client.predict(input(tick)).expect("predict");
        assert_eq!(point, oracle[tick as usize], "client checkpoint {tick}");
    }
    assert_eq!(
        client.steps(),
        u64::from(LONG_TICKS),
        "one sim step per predicted tick"
    );
    assert_eq!(
        client.checkpoints(),
        oracle.as_slice(),
        "prediction matches the prefix oracle"
    );

    // The client converges onto the authority tape: the last state names no
    // mispredict and the resimulation keeps the oracle checkpoints. The state
    // pass costs one authority step per received tick (no from-zero replay),
    // so the lockstep session totals two steps per tick.
    let (tip, delta) = (oracle[LONG_TICKS as usize - 1], last_delta.expect("delta"));
    assert_eq!(
        client.on_server_state(tip, &delta).expect("state applies"),
        Vec::<u32>::new(),
        "lockstep prediction never mispredicts"
    );
    assert_eq!(
        client.steps(),
        2 * u64::from(LONG_TICKS),
        "one prediction step plus one authority step per tick"
    );
    assert_eq!(client.resimulate(), oracle, "resimulation holds the oracle");
    assert_eq!(
        client.steps(),
        2 * u64::from(LONG_TICKS),
        "the authority prefix never re-steps"
    );

    // Same inputs twice: same checkpoints, same step count.
    let mut again = Server::new(config).expect("server opens");
    for tick in 0..LONG_TICKS {
        again.tick(&[input(tick)]).expect("tick authors");
    }
    assert_eq!(again.steps(), u64::from(LONG_TICKS));
    assert_eq!(again.checkpoints(), oracle.as_slice(), "digest x2");
}
