//! Schedule arena tick wiring: the production tick path plus its refusal case.

use engine_ecs::archetype::{Position, Velocity, World};
use engine_ecs::schedule::{Phase, Schedule};
use engine_ecs::{ArenaError, PhaseArena};

/// Deterministic P+V world on a fixed grid.
fn seeded_world(slots: usize) -> World {
    let mut world = World::new();
    for i in 0..slots {
        let f = i as f32;
        world.spawn(
            Some(Position { x: f * 0.25, y: -f }),
            Some(Velocity { x: 0.5, y: -0.25 }),
        );
    }
    world
}

/// Schedule that integrates on update, the same work the direct path calls.
fn integrate_schedule() -> Schedule<World> {
    let mut schedule = Schedule::new();
    schedule.add_system(Phase::Update, "integrate", |world: &mut World, ctx| {
        world.integrate(ctx.dt);
    });
    schedule
}

/// One scripted run: stage one kill plus a recycled spawn, then step.
/// The arena path flushes and checkpoints through scratch; the direct path
/// uses the plain flush plus hash. Both must agree every tick.
fn run_path(arena_path: bool) -> Vec<u64> {
    let mut world = seeded_world(32);
    world.reserve(64);
    let mut arena = PhaseArena::with_capacity(64, 2048);
    arena.reserve(64, 2048);
    let mut schedule = integrate_schedule();
    let mut out = Vec::with_capacity(16);
    for step in 0..16 {
        // Deterministic victim from the live set in index order.
        world
            .collect_live_into(&mut arena)
            .expect("census fits the pre-size");
        let live = arena.entity_len();
        assert!(live > 0, "census must see live entities");
        let victim = arena.entities()[step % live];
        assert!(world.kill(victim), "victim must stage");
        let f = step as f32;
        world.spawn(
            Some(Position { x: f, y: -f }),
            Some(Velocity { x: 0.5, y: 0.25 }),
        );
        let hash = if arena_path {
            schedule
                .run_arena_tick(&mut world, &mut arena, 0.016)
                .expect("tick fits the pre-size")
        } else {
            world.advance();
            schedule.run(&mut world, 0.016);
            world.flush();
            world.state_hash()
        };
        // The arena checkpoint equals the direct hash by construction.
        assert_eq!(
            hash,
            world.state_hash(),
            "checkpoint differs on step {step}"
        );
        out.push(hash);
    }
    out
}

#[test]
fn schedule_tick_matches_direct_path() {
    let direct = run_path(false);
    let arena = run_path(true);
    assert_eq!(direct, arena, "schedule tick moved a checkpoint");
    assert_eq!(run_path(true), arena, "schedule tick is not deterministic");
    assert!(
        arena.windows(2).any(|w| w[0] != w[1]),
        "script never moved the checkpoint"
    );
}

#[test]
fn short_presize_refuses_without_applying_kills() {
    let mut world = seeded_world(4);
    world.reserve(8);
    let mut census = PhaseArena::with_capacity(8, 256);
    world.collect_live_into(&mut census).expect("census fits");
    assert!(world.kill(census.entities()[0]));
    assert_eq!(world.pending(), 1);
    let mut tiny = PhaseArena::new();
    let mut schedule = integrate_schedule();
    assert!(
        matches!(
            schedule.run_arena_tick(&mut world, &mut tiny, 0.016),
            Err(ArenaError::Overflow { .. })
        ),
        "short scratch must refuse"
    );
    assert_eq!(world.pending(), 1, "refused tick must keep staged kills");
    tiny.reserve(8, 256);
    world.reserve(8);
    let hash = schedule
        .run_arena_tick(&mut world, &mut tiny, 0.016)
        .expect("reserved tick runs");
    assert_eq!(hash, world.state_hash());
    assert_eq!(world.pending(), 0);
}

/// RC35 (P6): agents in the 1,000-agent soak world.
const SOAK_AGENTS: usize = 1_000;
/// Ticks sampled per soak run (10 seconds at 60 Hz) and warmup before them.
const SOAK_TICKS: usize = 600;
const SOAK_WARMUP: usize = 60;
/// P6 budget: p99 of one full arena tick over 1,000 agents.
const P6_P99_BUDGET_NS: u128 = 2_000_000;

/// One 1,000-agent soak run: the arena tick digests and the p99 tick time.
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "timing test reads the wall clock; it never enters the tick"
)]
fn soak_run() -> (Vec<u64>, u128) {
    let mut world = World::new();
    world.reserve(SOAK_AGENTS + 8);
    for i in 0..SOAK_AGENTS {
        let f = i as f32;
        world.spawn(
            Some(Position { x: f * 0.25, y: -f }),
            Some(Velocity { x: 0.5, y: -0.25 }),
        );
    }
    // Checkpoint bytes per live P+V entity are 21; headroom for the census.
    let slots = SOAK_AGENTS + 8;
    let bytes = SOAK_AGENTS * 21 + 64;
    let mut arena = PhaseArena::with_capacity(slots, bytes);
    arena.reserve(slots, bytes);
    let mut schedule = integrate_schedule();
    let mut digests = Vec::with_capacity(SOAK_WARMUP + SOAK_TICKS);
    let mut times = Vec::with_capacity(SOAK_TICKS);
    for tick in 0..SOAK_WARMUP + SOAK_TICKS {
        let start = std::time::Instant::now();
        let digest = schedule
            .run_arena_tick(&mut world, &mut arena, 0.016)
            .expect("soak tick fits the pre-size");
        let spent = start.elapsed().as_nanos();
        digests.push(digest);
        if tick >= SOAK_WARMUP {
            times.push(spent);
        }
    }
    times.sort_unstable();
    // Nearest rank: the smallest sample with at least 99% of samples at or below it.
    let rank = (times.len() * 99).div_ceil(100);
    (digests, times[rank - 1])
}

#[test]
fn thousand_agent_tick_p99_under_2ms_and_replays() {
    let (first, first_p99) = soak_run();
    let (second, second_p99) = soak_run();
    eprintln!(
        "RC35 1000 agents: p99 {}us then {}us per arena tick (budget {}us), {} ticks each",
        first_p99 / 1_000,
        second_p99 / 1_000,
        P6_P99_BUDGET_NS / 1_000,
        first.len(),
    );
    assert_eq!(first, second, "1,000-agent replay drifted between runs");
    assert!(
        first.windows(2).any(|w| w[0] != w[1]),
        "soak never moved the checkpoint"
    );
    assert!(first_p99 <= P6_P99_BUDGET_NS, "run 1 p99 {first_p99}ns");
    assert!(second_p99 <= P6_P99_BUDGET_NS, "run 2 p99 {second_p99}ns");
}
