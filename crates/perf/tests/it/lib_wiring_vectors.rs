//! Lib wiring vectors: the perf crate root as its callers see it.
//!
//! Consumer: CI, the perf report and the profiler reach every perf number
//! through the crate root (`engine_perf::fight_cost`, `::smoke`, ...).
//! Units: microseconds, milliseconds, bytes, verdicts. Determinism: seeded
//! samples and integer math only, no wall clock, no GPU, no network.
//! Errors are typed per module, asserted through the root path.
//!
//! Why this file exists: `crates/perf/src/lib.rs` changed 9 times in
//! 30 days yet held zero `#[test]` and no `tests/it` module named it, so a
//! dropped `pub mod` would compile the crate but break every caller at
//! their `use` line. Each vector below names the root path, so removing or
//! gating a module fails here first.

use engine_perf::{
    alloc, allocator, arena, baselines, budget, feel, fight_cost, latency, profiler, smoke, soak,
    vram, workloads,
};
use engine_perf::{baselines::Baseline, workloads::WorkloadReport};

/// Every `pub mod` in the crate root stays reachable with its headline
/// numbers intact. Removing one module line fails this file at its `use`.
#[test]
fn root_reaches_every_module_with_headline_numbers() {
    assert_eq!(workloads::ArenaTick::NAME, "arena_tick");
    assert_eq!(workloads::PhysicsStep::NAME, "physics_step");
    assert_eq!(workloads::AiDecide::NAME, "ai_decide");
    assert_eq!(workloads::DestructionStep::NAME, "destruction_step");
    assert_eq!(workloads::SpineFullTick::NAME, "spine_full_tick");
    assert_eq!(workloads::RenderCull::NAME, "render_cull");
    assert_eq!(arena::FrameTransients::NAME, "frame_transients");
    assert_eq!(arena::CULL_KEYS, 1_024);
    assert_eq!(arena::FRAME_BYTES, 7_232);
    assert_eq!(profiler::SYSTEM_SPANS.len(), 4);
    assert_eq!(
        profiler::system_span(profiler::SPAN_ARENA_TICK),
        Ok(profiler::SPAN_ARENA_TICK)
    );
    assert!(profiler::system_span("perf.nope").is_err());
    assert_eq!(soak::SOAK_SEED, 41);
    assert_eq!(soak::SOAK_HOUR_FRAMES, 216_000);
    assert_eq!(smoke::SMOKE_SEED, 41);
    assert_eq!(smoke::SMOKE_FRAMES, 120);
    assert_eq!(fight_cost::FIGHT_P50_BUDGET_US, 33_300);
    assert_eq!(fight_cost::FIGHT_P99_BUDGET_US, 50_000);
    assert_eq!(fight_cost::FIGHT_FIGHTERS, 8);
    assert_eq!(feel::KILL_RISE_MS, 10.0);
    assert_eq!(feel::RUBRIC_PASS_PERMILLE, 900);
    assert_eq!(feel::WIN_LO_PERMILLE, 400);
    assert_eq!(feel::WIN_HI_PERMILLE, 600);
    assert_eq!(vram::GIB, 1_024 * 1_024 * 1_024);
    assert_eq!(alloc::HEAP_PEAK_CSV_HEADER, "workload,peak_bytes");
    assert_eq!(
        allocator::CSV_HEADER,
        "allocator,workload,ticks,hash,alloc_bytes,alloc_count,ms"
    );
    // Float bars read at runtime: a changed bar must move these reads.
    let tick_hz = std::hint::black_box(latency::TICK_HZ);
    assert!(tick_hz > 59.9 && tick_hz < 60.1, "tick rate {tick_hz}");
    let p95_bar = std::hint::black_box(latency::PRESENT_P95_BUDGET_MS);
    assert!(p95_bar > 49.9 && p95_bar < 50.1, "P2 bar {p95_bar}");
    let drift_tol = std::hint::black_box(soak::SOAK_DRIFT_TOLERANCE_MS);
    assert!(
        drift_tol > 4.9 && drift_tol < 5.1,
        "drift tolerance {drift_tol}"
    );
    let swap_bar = std::hint::black_box(allocator::SPEEDUP_PCT);
    assert!(swap_bar > 9.9 && swap_bar < 10.1, "swap bar {swap_bar}");
}

/// The fight-cost gate reads the same through the root: a quiet ladder
/// passes with known percentiles, blown bars trip naming the bar.
#[test]
fn fight_cost_gate_passes_and_trips_via_root() {
    let ladder = vec![
        28_000, 29_000, 30_000, 31_000, 32_000, 33_000, 34_000, 35_000, 36_000, 49_000,
    ];
    let (p50, p99) = fight_cost::check_gate(&ladder).expect("quiet ladder fits the bars");
    assert_eq!((p50, p99), (33_000, 49_000));
    assert_eq!(
        fight_cost::check_gate(&[]),
        Err(fight_cost::FightCostError::TooFewSamples(0))
    );
    let slow_p50 = vec![fight_cost::FIGHT_P50_BUDGET_US + 1; 4];
    let p50_err = fight_cost::check_gate(&slow_p50).expect_err("p50 over bar trips");
    assert_eq!(
        p50_err,
        fight_cost::FightCostError::P50Blown {
            measured_us: fight_cost::FIGHT_P50_BUDGET_US + 1,
            bar_us: fight_cost::FIGHT_P50_BUDGET_US,
        }
    );
    assert!(p50_err.to_string().contains("p50"), "{p50_err}");
    let frame = fight_cost::FightFrame {
        sim_us: 2_000,
        present_us: 8_000,
        fighters_us: 20_000,
    };
    assert_eq!(frame.total_us(), 30_000);
    assert_eq!(
        fight_cost::per_fighter_us(frame.total_us(), fight_cost::FIGHT_FIGHTERS),
        3_750
    );
    assert_eq!(fight_cost::per_fighter_us(100, 0), 0);
}

/// The packaged smoke run passes deterministically through the root, and
/// slow or empty frames fail the way the gate caller sees them.
#[test]
fn packaged_smoke_passes_and_trips_via_root() {
    let first = smoke::run().expect("packaged samples clear the gate");
    let second = smoke::run().expect("packaged smoke repeats");
    assert!(first.pass, "packaged smoke is the pass path");
    assert_eq!(first, second, "same packaged run repeats exactly");
    assert_eq!(first.report.events, smoke::SMOKE_FRAMES as u32);
    assert!(first.report.present_p95_ms < latency::PRESENT_P95_BUDGET_MS);
    let summary = first.summary();
    assert!(summary.contains("perf_smoke"), "{summary}");
    assert!(summary.contains("frames=120"), "{summary}");
    assert!(summary.contains("verdict=pass"), "{summary}");
    let slow = vec![60.0; smoke::SMOKE_FRAMES];
    let failed = smoke::check(&slow).expect("slow frames still rank");
    assert!(!failed.pass, "60 ms frames fail the 50 ms bar");
    assert!(
        failed.summary().contains("verdict=FAIL"),
        "{}",
        failed.summary()
    );
    assert_eq!(smoke::check(&[]), Err(latency::LatencyError::EmptyInput));
}

/// The short soak holds steady through the root: healthy stream passes,
/// drift stays inside tolerance, and a too-short run names itself.
#[test]
fn short_soak_holds_steady_via_root() {
    let report = soak::run(&soak::SoakConfig::short(600)).expect("seeded soak measures");
    assert!(report.pass, "healthy stream passes the soak");
    assert_eq!(report.report.events, 600);
    assert!(report.drift_ms() <= soak::SOAK_DRIFT_TOLERANCE_MS);
    let summary = report.summary();
    assert!(summary.contains("perf_soak"), "{summary}");
    assert!(summary.contains("verdict=pass"), "{summary}");
    assert_eq!(
        soak::check(&[]),
        Err(soak::SoakError::TooShort { frames: 0 })
    );
    assert!(
        soak::samples(&soak::SoakConfig::short(8)) == soak::samples(&soak::SoakConfig::short(8))
    );
}

/// The budget gate through the root: a matching report passes, and each
/// blown budget names the workload plus the budget it blew.
#[test]
fn budget_gate_names_blown_budget_via_root() {
    fn report() -> WorkloadReport {
        WorkloadReport {
            workload: "probe",
            ticks: 600,
            hash: 0x1234,
            alloc_bytes: 10,
            alloc_count: 2,
        }
    }
    fn recorded() -> Baseline {
        Baseline {
            workload: "probe",
            ticks: 600,
            hash: 0x1234,
            alloc_bytes: 10,
            alloc_count: 2,
        }
    }
    budget::check_report(&report(), &recorded()).expect("matching report passes");
    budget::check_alloc(&report(), &recorded()).expect("matching alloc passes");
    let cases = [
        (
            WorkloadReport {
                ticks: 599,
                ..report()
            },
            "tick budget",
        ),
        (
            WorkloadReport {
                hash: 0x5678,
                ..report()
            },
            "hash budget",
        ),
        (
            WorkloadReport {
                alloc_bytes: 11,
                ..report()
            },
            "alloc-bytes budget",
        ),
        (
            WorkloadReport {
                alloc_count: 3,
                ..report()
            },
            "alloc-count budget",
        ),
    ];
    for (bad, label) in cases {
        let error = budget::check_report(&bad, &recorded()).expect_err("budget must trip");
        assert!(error.to_string().contains(label), "{error}");
        assert!(error.to_string().contains("probe"), "{error}");
    }
    let timed = budget::check_time_ms("probe", 1.0, 2.0).expect_err("time over budget trips");
    assert!(timed.to_string().contains("time budget"), "{timed}");
    budget::check_time_ms("probe", 2.0, 1.0).expect("headroom passes");
}

/// The VRAM plan reads the same through the root: reservation, 95% budget,
/// usage, pressure edges and the one-line harness display.
#[test]
fn vram_plan_and_pressure_via_root() {
    let info = vram::VramInfo {
        budget_bytes: 6 * vram::GIB,
        current_bytes: 2 * vram::GIB,
        requested_bytes: vram::GIB,
    };
    let first = vram::plan(info);
    let second = vram::plan(info);
    assert_eq!(first, second, "same reading plans identically");
    assert_eq!(first.reservation_bytes, vram::GIB);
    assert_eq!(first.budget_bytes, 5_100_273_664);
    assert_eq!(first.usage_bytes, vram::GIB);
    let pressure = vram::pressure_pct(first.usage_bytes, first.budget_bytes);
    assert!(pressure < 100, "accept pressure {pressure} stays under 100");
    let over = vram::pressure_pct(first.budget_bytes.saturating_mul(2), first.budget_bytes);
    assert!(over > 100, "over-budget pressure {over} reads over 100");
    assert_eq!(vram::pressure_pct(0, 0), 0);
    assert_eq!(vram::pressure_pct(1, 0), 101);
    let text = format!("{first}");
    assert!(text.contains("reservation=1073741824"), "{text}");
    assert!(text.contains("budget=5100273664"), "{text}");
    assert!(text.contains("pressure="), "{text}");
}

/// The shared latency ranker through the root: healthy legs clear the P2
/// bar, slow legs trip it, and malformed legs fail naming the defect.
#[test]
fn latency_ranker_clears_and_trips_via_root() {
    let sim = [1.0, 2.0, 3.0];
    let present = [2.0, 3.0, 4.0];
    let report = latency::summarize_live(&sim, &present).expect("healthy legs rank");
    latency::check_p2_bar(&report).expect("healthy legs clear the bar");
    let slow_sim = [0.0; 4];
    let slow_present = [60.0; 4];
    let slow = latency::summarize_live(&slow_sim, &slow_present).expect("slow legs still rank");
    let blown = latency::check_p2_bar(&slow).expect_err("60 ms p95 trips the 50 ms bar");
    match blown {
        latency::LatencyError::BudgetBlown {
            actual_ms,
            budget_ms,
        } => {
            assert!(
                actual_ms > budget_ms,
                "measured {actual_ms} over bar {budget_ms}"
            );
        }
        other => panic!("wrong refusal via root: {other:?}"),
    }
    assert_eq!(
        latency::summarize_live(&[], &[]),
        Err(latency::LatencyError::EmptyInput)
    );
    assert_eq!(
        latency::summarize_live(&[1.0], &[1.0, 2.0]),
        Err(latency::LatencyError::MismatchedSamples { sim: 1, present: 2 })
    );
}

/// The remaining roots behave through the crate root: allocation peaks and
/// CSV rows, the allocator swap bar, the recorded baseline rows and the
/// frame arena.
#[test]
fn remaining_roots_behave_via_root() {
    assert_eq!(alloc::peak_of(&[3, 9, 4]), 9);
    assert_eq!(alloc::peak_of(&[]), 0);
    assert!(alloc::allows(110, 100), "10% headroom admits");
    assert!(!alloc::allows(111, 100), "past 10% refuses");
    assert!(alloc::allows(0, 0), "zero on zero admits");
    assert!(!alloc::allows(1, 0), "any peak on zero refuses");
    assert_eq!(alloc::heap_peak_csv_row("probe", 7), "probe,7");
    let swap = allocator::compare_ms(100.0, 85.0).expect("finite samples compare");
    assert!(swap.passes(), "15% faster clears the swap bar");
    assert!(swap.delta_pct > 10.0, "delta reads {}", swap.delta_pct);
    let slow_swap = allocator::compare_ms(100.0, 95.0).expect("finite samples compare");
    assert!(!slow_swap.passes(), "5% faster holds the swap");
    assert!(
        allocator::compare_ms(f64::NAN, 1.0).is_err(),
        "NaN system refuses"
    );
    let physics = baselines::baseline("physics_step").expect("physics row recorded");
    assert_eq!(physics.ticks, 600);
    assert!(
        baselines::baseline("no.such.workload").is_none(),
        "unknown row misses"
    );
    assert!(baselines::time_budget_ms("arena_tick").is_some_and(|ms| ms > 49.0 && ms < 51.0));
    assert!(baselines::time_budget_ms("no.such.workload").is_none());
    let mut scratch = arena::FrameArena::with_capacity_bytes(64);
    assert!(scratch.push_bytes(&[1u8, 2, 3, 4]).is_ok());
    assert_eq!(scratch.used_bytes(), 4);
    scratch.reset();
    assert_eq!(scratch.used_bytes(), 0);
    assert!(
        matches!(
            arena::FrameArena::new().push_bytes(&[1u8]),
            Err(arena::FrameError::Overflow { .. })
        ),
        "empty arena overflows naming the request"
    );
}
