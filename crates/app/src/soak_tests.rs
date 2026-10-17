//! RC35 runtime soak: a 10-minute siege through the app, twice, identical.
//!
//! Consumer: the app frame loop (`App::run_frame`) driving an archetype
//! World through `Schedule::run_arena_tick`, as a game's MatchStep lane would.
//! Data owner: the siege world below; the app only orders lanes and extracts.
//! Load: 1,000 agents plus 64 siege slots, one slot killed and respawned per
//! tick (the 64-slot churn), 36,000 frames (10 minutes at 60 Hz).
//! Determinism: the replay chain folds each tick's world digest with the
//! extracted tick and frame; wall time is read for the p99 report only.

use std::cell::Cell;
use std::rc::Rc;

use engine_core::{hash_bytes, TICK_HZ};
use engine_ecs::archetype::{Entity, Position, Velocity, World};
use engine_ecs::schedule::{Phase, Schedule};
use engine_ecs::PhaseArena;

use crate::app::{App, SystemKind};

/// Agents standing in the siege world.
const AGENTS: usize = 1_000;
/// Siege slots: one is killed and respawned each tick.
const SLOTS: usize = 64;
/// Ten minutes of 60 Hz frames.
const FRAMES: u64 = 600 * TICK_HZ;
/// P6 budget: p99 of one whole app frame (siege tick plus extraction).
const FRAME_P99_BUDGET_NS: u128 = 2_000_000;

/// What one soak run reports.
struct SoakReport {
    /// Fold of every tick digest with its extracted tick and frame.
    chain: u64,
    /// Frames the app ran.
    frames: u64,
    /// p99 of one frame in nanoseconds, nearest rank.
    frame_p99_ns: u128,
}

fn siege_pos(i: u64) -> Option<Position> {
    let f = (i % 1_000) as f32;
    Some(Position { x: f * 0.25, y: -f })
}

fn siege_vel() -> Option<Velocity> {
    Some(Velocity { x: 0.5, y: -0.25 })
}

/// Run the 10-minute siege through an `App` and report chain and timing.
fn siege_soak() -> SoakReport {
    let live = AGENTS + SLOTS;
    let mut world = World::new();
    world.reserve(live * 2);
    for i in 0..AGENTS as u64 {
        world.spawn(siege_pos(i), siege_vel());
    }
    let mut slots: Vec<Entity> = (0..SLOTS as u64)
        .map(|i| world.spawn(siege_pos(i + 7), siege_vel()))
        .collect();
    // Checkpoint bytes per live P+V entity are 21; headroom for the churn.
    let scratch_slots = live + 8;
    let scratch_bytes = scratch_slots * 21 + 64;
    let mut arena = PhaseArena::with_capacity(scratch_slots, scratch_bytes);
    arena.reserve(scratch_slots, scratch_bytes);
    let mut schedule: Schedule<World> = Schedule::new();
    schedule.add_system(Phase::Update, "integrate", |world: &mut World, ctx| {
        world.integrate(ctx.dt);
    });

    let digest = Rc::new(Cell::new(0u64));
    let chain = Rc::new(Cell::new(0u64));
    let (digest_out, digest_in) = (Rc::clone(&digest), Rc::clone(&digest));
    let chain_in = Rc::clone(&chain);
    let mut app = App::builder()
        .register(
            "siege_step",
            SystemKind::MatchStep,
            Phase::Update,
            move |ctx| {
                let victim = (ctx.tick % SLOTS as u64) as usize;
                assert!(world.kill(slots[victim]), "siege slot {victim} was live");
                slots[victim] = world.spawn(siege_pos(ctx.tick), siege_vel());
                let checkpoint = schedule
                    .run_arena_tick(&mut world, &mut arena, 0.016)
                    .expect("siege tick fits the pre-size");
                digest_out.set(checkpoint);
            },
        )
        .expect("siege_step registers")
        .register(
            "siege_extract",
            SystemKind::Extract,
            Phase::PostUpdate,
            move |ctx| {
                let mut bytes = [0u8; 24];
                bytes[..8].copy_from_slice(&chain_in.get().to_le_bytes());
                bytes[8..16].copy_from_slice(&digest_in.get().to_le_bytes());
                bytes[16..].copy_from_slice(&ctx.frame.to_le_bytes());
                chain_in.set(hash_bytes(&bytes));
            },
        )
        .expect("siege_extract registers")
        .startup()
        .expect("app boots");

    let mut times = Vec::with_capacity(FRAMES as usize);
    for _ in 0..FRAMES {
        let start = engine_clock::now();
        let receipt = app.run_frame(1.0 / 60.0).expect("frame runs");
        times.push(engine_clock::since(start).as_nanos());
        assert_eq!(receipt.extraction.frame, receipt.frame, "stale extraction");
    }
    let frames = app.frame();
    assert_eq!(app.last_extraction().map(|x| x.frame), Some(frames));
    times.sort_unstable();
    // Nearest rank: the smallest sample with at least 99% of samples at or below it.
    let rank = (times.len() * 99).div_ceil(100);
    SoakReport {
        chain: chain.get(),
        frames,
        frame_p99_ns: times[rank - 1],
    }
}

#[test]
fn ten_minute_64_slot_siege_replays_exactly_inside_the_frame_budget() {
    let first = siege_soak();
    let second = siege_soak();
    eprintln!(
        "RC35 siege: {} frames, frame p99 {}us then {}us (budget {}us), chain {:016x}",
        first.frames,
        first.frame_p99_ns / 1_000,
        second.frame_p99_ns / 1_000,
        FRAME_P99_BUDGET_NS / 1_000,
        first.chain,
    );
    assert_eq!(first.frames, FRAMES);
    assert_ne!(first.chain, 0, "siege never extracted");
    assert_eq!(first.chain, second.chain, "siege replay drifted");
    assert!(
        first.frame_p99_ns <= FRAME_P99_BUDGET_NS,
        "run 1 p99 {}ns",
        first.frame_p99_ns
    );
    assert!(
        second.frame_p99_ns <= FRAME_P99_BUDGET_NS,
        "run 2 p99 {}ns",
        second.frame_p99_ns
    );
}
