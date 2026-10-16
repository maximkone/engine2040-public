//! Instruction-count budget for the three hot simulation loops: the arena
//! match, the physics step and the ECS spine tick.
//!
//! Callgrind counts retired instructions, which repeat exactly from run to run
//! and machine to machine, so a 2% budget holds in CI where wall clock does not.
//! Gungraun keeps the previous run of every benchmark and fails the run when
//! `Ir` grows past `IR_BUDGET_PCT` over it.
//!
//! Run (Linux with Valgrind and `gungraun-runner` installed):
//! `cargo bench --locked -p engine-perf --bench instructions`.
//! Enforcement leg: this budget is enforced ONLY by the Linux leg of
//! `node tools/proof.mjs --perf` (`irGateVerdict` there), whose receipt line
//! `[PASS] instruction counts inside the 2% Ir budget (...)` (or `[FAIL] ...`)
//! is the proof. A bare `cargo bench` run, a Windows run, or a
//! `cargo test --all-targets` launch (this binary exits 0 without measuring,
//! see `main` below) proves nothing about the budget: cite the `--perf`
//! receipt line wherever the 2% is claimed.
//! Inputs come from `setup`, which Callgrind does not count; the measured
//! region is the call under test. Wall-clock timing stays in the ignored
//! timing tests.
//!
//! `cargo test --all-targets` also launches bench binaries, bare and on
//! machines that may lack Valgrind or the runner: the entry point does nothing
//! without arguments, so only `cargo bench` and the runner's re-launches measure.

#[cfg(target_os = "linux")]
mod harness {
    use engine_ecs::archetype::{Position, Velocity, World};
    use engine_ecs::schedule::{Phase, Schedule};
    use engine_ecs::PhaseArena;
    use engine_perf::workloads::{ArenaTick, PhysicsStep, DT_S};
    use gungraun::{
        library_benchmark, library_benchmark_group, main, Callgrind, EventKind,
        LibraryBenchmarkConfig,
    };
    use std::hint::black_box;

    /// Allowed growth of the instruction count over the stored baseline, percent.
    const IR_BUDGET_PCT: f64 = 2.0;
    /// Spine ticks per measured run.
    const SPINE_TICKS: u32 = 60;
    /// Scratch headroom above the live entity count (census plus spawned probes).
    const SPINE_HEADROOM_ENTITIES: usize = 8;
    /// Checkpoint bytes per live entity: id, mask, position, velocity.
    const SPINE_BYTES_PER_ENTITY: usize = 21;
    /// Checkpoint byte headroom above the exact staging size.
    const SPINE_HEADROOM_BYTES: usize = 64;

    /// Archetype world, integrate schedule and presized scratch for the spine tick.
    struct SpineFixture {
        world: World,
        schedule: Schedule<World>,
        arena: PhaseArena,
    }

    /// Build the production tick path: `entities` position-and-velocity bodies
    /// on a fixed grid, one integrate system, scratch presized so ticks never grow.
    fn spine_fixture(entities: usize) -> SpineFixture {
        let mut world = World::new();
        world.reserve(entities + SPINE_HEADROOM_ENTITIES);
        for i in 0..entities {
            let f = i as f32;
            world.spawn(
                Some(Position { x: f * 0.25, y: -f }),
                Some(Velocity { x: 0.5, y: -0.25 }),
            );
        }
        let mut schedule: Schedule<World> = Schedule::new();
        schedule.add_system(Phase::Update, "integrate", |world, ctx| {
            world.integrate(ctx.dt);
        });
        let slots = entities + SPINE_HEADROOM_ENTITIES;
        let bytes = entities * SPINE_BYTES_PER_ENTITY + SPINE_HEADROOM_BYTES;
        let mut arena = PhaseArena::with_capacity(slots, bytes);
        arena.reserve(slots, bytes);
        SpineFixture {
            world,
            schedule,
            arena,
        }
    }

    // Canonical arena match: 16 AI-filled fighters, brains, combat and the crate
    // prop. (The benchmark macro rejects doc comments, so these notes are plain.)
    #[library_benchmark]
    #[bench::fighters_16(ArenaTick::canonical())]
    fn arena_step(workload: ArenaTick) -> u64 {
        black_box(workload.run_hash())
    }

    // Canonical physics run: the seeded 256-body cloud, state hash per tick.
    #[library_benchmark]
    #[bench::bodies_256(PhysicsStep::canonical())]
    fn physics_step(workload: PhysicsStep) -> u64 {
        black_box(workload.run_hash())
    }

    // Scheduled arena ticks over the archetype world. The fixture is returned
    // with the checkpoint so its drop falls outside the measured region.
    #[library_benchmark]
    #[bench::entities_1000(args = (1_000), setup = spine_fixture)]
    fn ecs_spine(mut fixture: SpineFixture) -> (u64, SpineFixture) {
        let mut checkpoint = 0;
        for _ in 0..SPINE_TICKS {
            checkpoint = fixture
                .schedule
                .run_arena_tick(&mut fixture.world, &mut fixture.arena, DT_S)
                .expect("the fixture presizes scratch for every entity");
        }
        (black_box(checkpoint), fixture)
    }

    library_benchmark_group!(
        name = simulation,
        benchmarks = [arena_step, physics_step, ecs_spine]
    );

    main!(
        config = LibraryBenchmarkConfig::default()
            .tool(Callgrind::default().soft_limits([(EventKind::Ir, IR_BUDGET_PCT)])),
        library_benchmark_groups = simulation
    );

    /// Hand control to the generated Gungraun entry point.
    pub fn run() {
        main();
    }
}

fn main() {
    // `cargo test --all-targets` launches bench binaries bare while
    // `cargo nextest run --all-targets` probes them with `--list --format terse`.
    // Both probes must exit 0 without touching the Gungraun runner, which may
    // not be installed; only `cargo bench` and the runner's re-launches measure.
    if std::env::args().any(|arg| arg == "--list") {
        return;
    }
    #[cfg(target_os = "linux")]
    if std::env::args().len() > 1 {
        harness::run();
    }
}
