//! Gate-failure proofs: a seeded hash regression trips `HashMismatch` and a
//! seeded allocation trips `AllocBytes`, so a silent gate could not pass.

use engine_perf::alloc::measure;
use engine_perf::baselines::{arena_baseline, baseline};
use engine_perf::budget::{check_report, BudgetError};
use engine_perf::workloads::{AiDecide, ArenaTick, DestructionStep, PhysicsStep, WorkloadReport};

const SEED_TWEAK: u64 = 0x9E37_79B9;

#[test]
fn seeded_hash_regression_trips_the_gate() {
    let perturbed = [
        (
            ArenaTick::NAME,
            ArenaTick::canonical()
                .with_seed(1 ^ SEED_TWEAK)
                .run_report(),
        ),
        (
            PhysicsStep::NAME,
            PhysicsStep::canonical()
                .with_seed(4_242 ^ SEED_TWEAK)
                .run_report(),
        ),
        (
            AiDecide::NAME,
            AiDecide::canonical()
                .with_seed(9_001 ^ SEED_TWEAK)
                .run_report(),
        ),
        (
            DestructionStep::NAME,
            DestructionStep::canonical()
                .with_seed(5_150 ^ SEED_TWEAK)
                .run_report(),
        ),
    ];
    for (name, report) in perturbed {
        // Arena rows compare against the compiled track's baseline.
        let recorded = if name == ArenaTick::NAME {
            arena_baseline()
        } else {
            baseline(name).expect("baseline recorded")
        };
        assert_eq!(report.ticks, recorded.ticks, "{name} keeps its tick count");
        assert_ne!(
            report.hash, recorded.hash,
            "{name} hash moves with the seed"
        );
        let error = check_report(&report, &recorded).expect_err("{name} gate must trip");
        assert!(
            matches!(
                error,
                BudgetError::HashMismatch {
                    workload,
                    expected,
                    actual,
                } if workload == name && expected == recorded.hash && actual == report.hash
            ),
            "{name} trips HashMismatch, got {error}"
        );
    }
}

#[test]
fn seeded_allocation_trips_the_gate() {
    let recorded = arena_baseline();
    let workload = ArenaTick::canonical();
    let (hash, observed) = measure(|| {
        let hash = workload.run_hash();
        // black_box keeps the optimizer from eliding the sink in release.
        std::hint::black_box(vec![0u8; 64]);
        hash
    });
    assert_eq!(
        hash, recorded.hash,
        "simulation bytes untouched by the injection"
    );
    let report = WorkloadReport {
        workload: ArenaTick::NAME,
        ticks: workload.ticks,
        hash,
        alloc_bytes: observed.bytes,
        alloc_count: observed.count,
    };
    assert!(
        report.alloc_bytes > recorded.alloc_bytes,
        "injection really allocates: {} vs budget {}",
        report.alloc_bytes,
        recorded.alloc_bytes
    );
    let error = check_report(&report, &recorded).expect_err("alloc gate must trip");
    assert!(
        matches!(
            error,
            BudgetError::AllocBytes { workload, .. } if workload == ArenaTick::NAME
        ),
        "injection trips AllocBytes, got {error}"
    );
}
