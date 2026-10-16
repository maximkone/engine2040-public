//! perf: stress harness -> perf.csv. Zero deps, deterministic.
//! run: cargo run --release -p engine-perf > perf.csv (gates: sim-1k p99<2ms, 10k p99<=16.6ms, hash x2).
//! profile: cargo run --release -p engine-perf --features profile -- --capture [frames]
//!   enables the profiler and prints the frame-capture report (perf report input).
//! vram: cargo run -p engine-perf -- --vram-plan prints the Dawn budget line
//!   for the 6 GiB accept reading (presentation only, never hashed).
//! heap-peak: cargo run -p engine-perf -- --heap-peak prints the 8-fighter
//!   peak-heap csv plus the 10 percent gate verdict (presentation only).

use engine_core::hash::FNV64_PRIME;
use engine_core::{run_ticks, Rng, Tick};
use engine_ecs::archetype::{Position, Velocity, World};
use engine_ecs::schedule::{Phase, Schedule};
use engine_ecs::PhaseArena;
use engine_perf::alloc::{allows, heap_peak_csv, parse_heap_peak_baseline, peak_of};
use engine_perf::profiler::{self, CAPTURE_FRAMES_DEFAULT, CAPTURE_FRAMES_MAX};
use engine_perf::vram::{self, VramInfo};
use engine_perf::workloads::ScaledArenaTick;
use engine_physics::{Body, Space};
use engine_render::NullRenderer;
use std::time::Instant;

/// Scratch headroom above the live count: census plus checkpoint fit with
/// room for a few spawned probes.
const ARENA_HEADROOM_ENTITIES: usize = 8;
/// Checkpoint bytes per live P+V entity (id, mask, position, velocity).
const ARENA_BYTES_PER_ENTITY: usize = 21;
/// Checkpoint byte headroom above the exact `entities * 21` staging.
const ARENA_HEADROOM_BYTES: usize = 64;
/// Perf csv header: the `cpu_workload_rate` column is ticks per CPU second
/// of the headless loop (`1000 / mean_ms`), not frames per second.
const CSV_HEADER: &str =
    "scenario,seed,ticks,agents,sprites,mean_ms,p50_ms,p99_ms,max_ms,cpu_workload_rate,final_hash";

/// Builds one perf csv row with `rate` as ticks per CPU second of the
/// headless loop (`1000 / mean_ms`), not frames per second.
fn csv_row(
    name: &str,
    agents: usize,
    sprites: usize,
    stats: (f64, f64, f64, f64),
    hash: u64,
) -> String {
    let (mean, p50, p99, max) = stats;
    let rate = 1000.0 / mean.max(1e-9);
    format!("{name},2040,600,{agents},{sprites},{mean:.4},{p50:.4},{p99:.4},{max:.4},{rate:.1},{hash:016x}")
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

fn stats(mut dts: Vec<f64>) -> (f64, f64, f64, f64) {
    dts.sort_by(f64::total_cmp);
    let mean = dts.iter().sum::<f64>() / dts.len().max(1) as f64;
    (
        mean,
        percentile(&dts, 50.0),
        percentile(&dts, 99.0),
        dts.last().copied().unwrap_or(0.0),
    )
}

fn seed_world(n: usize, seed: u64, dynamic: bool) -> (World, Space) {
    let mut world = World::new();
    world.reserve(n + ARENA_HEADROOM_ENTITIES);
    let mut space = Space::new();
    let mut rng = Rng::new(seed);
    for _ in 0..n {
        let px = (rng.next_u64() % 100) as f32;
        let py = (rng.next_u64() % 100) as f32;
        let (vx, vy) = if dynamic { (1.0, 0.5) } else { (0.0, 0.0) };
        world.spawn(
            Some(Position { x: px, y: py }),
            Some(Velocity { x: vx, y: vy }),
        );
        space.add(Body {
            x: px,
            y: py + 10.0,
            vx,
            vy,
            hx: 0.5,
            hy: 0.5,
        });
    }
    (world, space)
}

/// Presized arena tick state for one harness scenario: the integrate
/// schedule plus scratch sized for `entities` live P+V bodies.
fn arena_tick_state(entities: usize) -> (Schedule<World>, PhaseArena) {
    let mut schedule: Schedule<World> = Schedule::new();
    schedule.add_system(Phase::Update, "integrate", |world, ctx| {
        world.integrate(ctx.dt);
    });
    let slots = entities + ARENA_HEADROOM_ENTITIES;
    let bytes = entities * ARENA_BYTES_PER_ENTITY + ARENA_HEADROOM_BYTES;
    let mut arena = PhaseArena::with_capacity(slots, bytes);
    arena.reserve(slots, bytes);
    (schedule, arena)
}

/// Dawn budget line for the 6 GiB accept reading (budget 6 GiB, usage
/// 2 GiB, requested 1 GiB). Pure integer math through [`vram::plan`];
/// same inputs give the same line. Presentation only, never hashed.
fn vram_plan_text() -> String {
    let info = VramInfo {
        budget_bytes: 6 * vram::GIB,
        current_bytes: 2 * vram::GIB,
        requested_bytes: vram::GIB,
    };
    format!("{}", vram::plan(info))
}

/// Peak live-heap bytes over two 600-tick 8-fighter headless runs.
///
/// Runs the 8-fighter canonical match twice and takes the max `alloc_bytes`
/// via [`peak_of`]. The struct literal equals
/// [`ScaledArenaTick::canonical_for`] for 8 (seed 1, 600 ticks, 20 m);
/// the match is deterministic, so the same build gives the same peak twice.
/// Presentation gate only, never enters the simulation hash.
/// Caller: `main --heap-peak` plus the `heap_peak` integration test.
fn heap_peak_peak_bytes() -> (u64, u64, u64) {
    let workload = ScaledArenaTick {
        seed: 1,
        ticks: 600,
        half_extent_m: 20.0,
        fighters: 8,
    };
    let first = workload.run_report();
    let second = workload.run_report();
    debug_assert_eq!(first.ticks, 600, "headless match runs 600 ticks");
    debug_assert_eq!(first.hash, second.hash, "8-fighter hash repeats");
    debug_assert_eq!(
        first.alloc_bytes, second.alloc_bytes,
        "8-fighter alloc repeats"
    );
    let peak = peak_of(&[first.alloc_bytes, second.alloc_bytes]);
    (first.alloc_bytes, second.alloc_bytes, peak)
}

/// Gate verdict text for `peak` against `baseline_text`.
///
/// Parses via [`parse_heap_peak_baseline`] (garbage reads `None` and fails
/// closed), checks the 10 percent rule via [`allows`], formats the report
/// via [`heap_peak_csv`]. Same inputs give the same text twice.
/// Caller: `main --heap-peak` prints it; the binary tests pin PASS/FAIL.
fn heap_peak_gate_text(peak: u64, baseline_text: &str) -> String {
    let baseline = parse_heap_peak_baseline(baseline_text);
    let pass = baseline.map(|base| allows(peak, base)).unwrap_or(false);
    let csv = heap_peak_csv(ScaledArenaTick::NAME_8, peak);
    let verdict = if pass { "PASS" } else { "FAIL" };
    let base_text = baseline
        .map(|base| base.to_string())
        .unwrap_or_else(|| "UNKNOWN".to_owned());
    format!("{csv}{verdict} peak={peak} baseline={base_text}\n")
}

fn capture_arg() -> Option<u32> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--capture" {
            let frames = match args.next() {
                None => CAPTURE_FRAMES_DEFAULT,
                Some(text) => match text.parse::<u32>() {
                    Ok(frames) => frames,
                    Err(_) => {
                        eprintln!("--capture needs frames 1..={CAPTURE_FRAMES_MAX}, got {text:?}");
                        std::process::exit(2);
                    }
                },
            };
            if frames == 0 || frames > CAPTURE_FRAMES_MAX {
                eprintln!("--capture needs frames 1..={CAPTURE_FRAMES_MAX}, got {frames}");
                std::process::exit(2);
            }
            return Some(frames);
        }
    }
    None
}

fn main() {
    if std::env::args().any(|arg| arg == "--vram-plan") {
        println!("{}", vram_plan_text());
        return;
    }
    if std::env::args().any(|arg| arg == "--heap-peak") {
        let (first, second, peak) = heap_peak_peak_bytes();
        debug_assert_eq!(first, second, "peak inputs repeat");
        let baseline_path = format!("{}/baselines/heap_peak.txt", env!("CARGO_MANIFEST_DIR"));
        let baseline_text = std::fs::read_to_string(&baseline_path).unwrap_or_default();
        let text = heap_peak_gate_text(peak, &baseline_text);
        print!("{text}");
        let baseline = parse_heap_peak_baseline(&baseline_text);
        let pass = baseline.map(|base| allows(peak, base)).unwrap_or(false);
        if !pass {
            std::process::exit(1);
        }
        return;
    }
    if let Some(frames) = capture_arg() {
        profiler::set_enabled(true);
        match profiler::capture(frames) {
            Ok(report) => print!("{}", report.summary()),
            Err(error) => {
                eprintln!("capture failed: {error}");
                std::process::exit(1);
            }
        }
        return;
    }
    println!("{CSV_HEADER}");
    // sim-1k + mixed share the dynamic world; render-10k uses a static one.
    for (name, agents, sprites, dynamic) in [
        ("sim-1k", 1000, 0, true),
        ("render-10k", 0, 10000, false),
        ("mixed-1k-10k", 1000, 10000, true),
    ] {
        let (mut world, mut space) = seed_world(agents + sprites, 2040, dynamic);
        let (mut schedule, mut arena) = arena_tick_state(agents + sprites);
        let mut renderer = NullRenderer::new();
        let mut dts: Vec<f64> = Vec::with_capacity(600);
        let hashes = run_ticks(600, 2040, |t: Tick, _r: &mut Rng| {
            let s = Instant::now();
            if let Err(error) = schedule.run_arena_tick(&mut world, &mut arena, 1.0 / 60.0) {
                eprintln!("{name}: harness tick outgrew its presized scratch: {error}");
                std::process::exit(1);
            }
            space.step(1.0 / 60.0);
            renderer.begin_frame(t.0);
            let h = world.state_hash() ^ space.state_hash().wrapping_mul(FNV64_PRIME);
            // Sampled profile seam: only sampled ticks check the gate, so the
            // disabled cost is one relaxed load plus a branch per tick and the
            // harness hash never sees the probe (its hash is discarded).
            if profiler::gate_on(t.0.is_multiple_of(8)) {
                std::hint::black_box(profiler::profiled_physics_step_if(true).0);
            }
            dts.push(s.elapsed().as_secs_f64() * 1000.0);
            h
        });
        let (mean, p50, p99, max) = stats(dts);
        println!(
            "{}",
            csv_row(
                name,
                agents,
                sprites,
                (mean, p50, p99, max),
                hashes.last().copied().unwrap_or_default()
            )
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick_hash(world: &mut World, space: &mut Space) -> u64 {
        world.state_hash() ^ space.state_hash().wrapping_mul(FNV64_PRIME)
    }

    #[test]
    fn harness_ticks_through_run_arena_tick_deterministically() {
        // The stress harness ticks through the arena path. Two seeded runs
        // agree every tick, the checkpoint equals the direct hash, and
        // movement actually moves the combined hash.
        let run = || {
            let (mut world, mut space) = seed_world(8, 2040, true);
            let (mut schedule, mut arena) = arena_tick_state(8);
            let mut hashes = Vec::with_capacity(8);
            for _ in 0..8 {
                let checkpoint = schedule
                    .run_arena_tick(&mut world, &mut arena, 1.0 / 60.0)
                    .expect("presized harness scratch fits 8 bodies");
                space.step(1.0 / 60.0);
                assert_eq!(checkpoint, world.state_hash());
                hashes.push(tick_hash(&mut world, &mut space));
            }
            hashes
        };
        let (first, second) = (run(), run());
        assert_eq!(first, second);
        assert!(
            first.windows(2).any(|pair| pair[0] != pair[1]),
            "dynamic harness never moved its hash"
        );
    }

    #[test]
    fn short_scratch_refuses_harness_tick() {
        let (mut world, mut space) = seed_world(8, 2040, true);
        let (before_world, before_space) = (world.state_hash(), space.state_hash());
        let mut tiny = PhaseArena::new();
        let mut refusing: Schedule<World> = Schedule::new();
        refusing.add_system(Phase::Update, "integrate", |world, ctx| {
            world.integrate(ctx.dt);
        });
        assert!(matches!(
            refusing.run_arena_tick(&mut world, &mut tiny, 1.0 / 60.0),
            Err(engine_ecs::ArenaError::Overflow { .. })
        ));
        space.step(1.0 / 60.0);
        assert_eq!(world.state_hash(), before_world);
        assert_ne!(space.state_hash(), before_space);
    }

    #[test]
    fn harness_csv_header_names_cpu_workload_rate_and_matches_row_width() {
        assert!(CSV_HEADER.contains("cpu_workload_rate"));
        assert!(!CSV_HEADER.contains("fps_est"));
        assert_eq!(CSV_HEADER.split(',').count(), 11);
        let row = csv_row(
            "sim-1k",
            1000,
            0,
            (0.125, 0.1048, 0.2811, 0.4514),
            0xe2f9c8b4aad1a292,
        );
        assert_eq!(
            row,
            "sim-1k,2040,600,1000,0,0.1250,0.1048,0.2811,0.4514,8000.0,e2f9c8b4aad1a292"
        );
        assert_eq!(row.split(',').count(), CSV_HEADER.split(',').count());
    }

    #[test]
    fn vram_plan_text_names_accept_numbers() {
        let text = vram_plan_text();
        assert!(text.contains("reservation=1073741824"), "{text}");
        assert!(text.contains("budget=5100273664"), "{text}");
        assert!(text.contains("usage=1073741824"), "{text}");
    }

    #[test]
    fn vram_plan_text_matches_lib_display_x2() {
        let info = VramInfo {
            budget_bytes: 6 * vram::GIB,
            current_bytes: 2 * vram::GIB,
            requested_bytes: vram::GIB,
        };
        let direct = format!("{}", vram::plan(info));
        assert_eq!(vram_plan_text(), direct, "harness reads the lib line");
        assert_eq!(vram_plan_text(), vram_plan_text(), "same reading repeats");
    }

    #[test]
    fn heap_peak_harness_repeats_peak_x2() {
        let (first, second, peak) = heap_peak_peak_bytes();
        assert_eq!(first, second, "8-fighter alloc repeats");
        assert_eq!(peak, first, "peak is the max sample");
        let (again_first, _, again_peak) = heap_peak_peak_bytes();
        assert_eq!(peak, again_peak, "same match gives same peak twice");
        assert_eq!(first, again_first, "same match gives same alloc twice");
    }

    #[test]
    fn heap_peak_gate_text_passes_baseline_x2() {
        let (_, _, peak) = heap_peak_peak_bytes();
        let baseline_path = format!("{}/baselines/heap_peak.txt", env!("CARGO_MANIFEST_DIR"));
        let baseline_text = std::fs::read_to_string(&baseline_path).expect("baseline file exists");
        let first = heap_peak_gate_text(peak, &baseline_text);
        let second = heap_peak_gate_text(peak, &baseline_text);
        assert_eq!(first, second, "same peak and baseline repeat");
        assert!(first.contains("arena_tick_8"), "{first}");
        assert!(first.contains("PASS"), "{first}");
        assert!(first.contains(&format!("peak={peak}")), "{first}");
    }

    #[test]
    fn heap_peak_gate_text_trips_on_64mib_extra() {
        let baseline_text = std::fs::read_to_string(format!(
            "{}/baselines/heap_peak.txt",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("baseline file exists");
        let baseline = parse_heap_peak_baseline(&baseline_text).expect("baseline holds one u64");
        assert!(
            heap_peak_gate_text(baseline, &baseline_text).contains("PASS"),
            "baseline peak passes its own gate"
        );
        let tripped = baseline + (64 << 20);
        let text = heap_peak_gate_text(tripped, &baseline_text);
        assert!(text.contains("FAIL"), "{text}");
        assert!(
            !allows(tripped, baseline),
            "64 MiB over baseline trips the 10 percent rule"
        );
    }

    #[test]
    fn heap_peak_gate_text_fails_closed_on_garbage_baseline() {
        let (_, _, peak) = heap_peak_peak_bytes();
        for garbage in ["", "nope\n", "  \n"] {
            let text = heap_peak_gate_text(peak, garbage);
            assert!(
                text.contains("FAIL"),
                "{garbage:?} must fail closed: {text}"
            );
            assert!(text.contains("UNKNOWN"), "{text}");
        }
        assert_eq!(parse_heap_peak_baseline(""), None);
        assert_eq!(parse_heap_peak_baseline("nope\n"), None);
    }
}
