//! System-allocator baseline rows: identical hash/alloc gates as the
//! recorded baselines, plus the wall-ms harness a candidate row reuses.
//! Default suite pins hash/alloc exactly; wall samples print only under the
//! ignored `--perf` proof (release, single-threaded):
//! `cargo test --release -p engine-perf --test it allocator_baseline:: -- --ignored --test-threads=1`.
//! Wall ms joins the baseline file only from a quiesced box (load under 10%,
//! High-Performance power plan, three consecutive runs within ±10%);
//! every other timed row stays observed-only.

use engine_perf::allocator::{box_conditions, csv_row, system_rows, AllocatorKind, CSV_HEADER};
use engine_perf::baselines::baseline;
use engine_perf::budget::{check_report, BudgetError};
use engine_perf::workloads::ArenaTick;
use std::time::Instant;

#[test]
fn system_rows_match_baselines_exactly() {
    println!("{CSV_HEADER}");
    for report in system_rows() {
        let recorded = baseline(report.workload).expect("system row names its baseline");
        check_report(&report, &recorded).expect("system row matches its baseline");
        let row = csv_row(&report, AllocatorKind::System, None).expect("row prints");
        assert!(row.starts_with("system,"), "{row}");
        println!("{row}");
    }
}

#[test]
fn seeded_row_trips_the_gate_through_the_harness() {
    const SEED_TWEAK: u64 = 0x9E37_79B9;
    let report = ArenaTick::canonical()
        .with_seed(1 ^ SEED_TWEAK)
        .run_report();
    let recorded = baseline(report.workload).expect("arena row recorded");
    assert_eq!(report.ticks, recorded.ticks, "tweak keeps the tick count");
    assert_ne!(report.hash, recorded.hash, "tweak moves the hash");
    let error = check_report(&report, &recorded).expect_err("harness gate must trip");
    assert!(
        matches!(error, BudgetError::HashMismatch { .. }),
        "tweak trips HashMismatch, got {error}"
    );
}

fn percentile(sorted: &mut [f64], p: f64) -> f64 {
    sorted.sort_by(|a, b| a.total_cmp(b));
    let end = sorted.len().saturating_sub(1);
    let index = ((p / 100.0) * end as f64).round() as usize;
    sorted[index.min(end)]
}

#[test]
#[ignore = "timing: release-mode system wall-ms rows, run with --release -- --ignored --test-threads=1"]
fn system_wall_ms_rows() {
    const WARMUP: usize = 2;
    const SAMPLES: usize = 21;
    println!("{}", box_conditions());
    println!("{CSV_HEADER}");
    for report in system_rows() {
        let run: Box<dyn Fn() -> u64> = match report.workload {
            "physics_step" => {
                Box::new(|| engine_perf::workloads::PhysicsStep::canonical().run_hash())
            }
            "ai_decide" => Box::new(|| engine_perf::workloads::AiDecide::canonical().run_hash()),
            "destruction_step" => {
                Box::new(|| engine_perf::workloads::DestructionStep::canonical().run_hash())
            }
            _ => Box::new(|| {
                engine_perf::workloads::ScaledArenaTick::canonical_for(16)
                    .expect("16 is a listed matrix count")
                    .run_hash()
            }),
        };
        // Warmup first: cold caches never enter the samples.
        for _ in 0..WARMUP {
            let _ = run();
        }
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let start = Instant::now();
            let hash = run();
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(
                hash, report.hash,
                "{} sample keeps its hash",
                report.workload
            );
        }
        let p50 = percentile(&mut samples, 50.0);
        let p95 = percentile(&mut samples, 95.0);
        let p99 = percentile(&mut samples, 99.0);
        let min = samples.first().copied().unwrap_or(f64::NAN);
        let max = samples.last().copied().unwrap_or(f64::NAN);
        for (name, value) in [("p50", p50), ("p95", p95), ("p99", p99)] {
            assert!(
                value.is_finite() && value >= 0.0,
                "{} {name} sample is usable: {value}",
                report.workload
            );
        }
        println!(
            "# stats: workload={} n={SAMPLES} p50={p50:.3}ms p95={p95:.3}ms p99={p99:.3}ms min={min:.3}ms max={max:.3}ms",
            report.workload
        );
        let row = csv_row(&report, AllocatorKind::System, Some(p50)).expect("timed row prints");
        println!("{row}");
    }
}
