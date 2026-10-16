//! Peak-plus-gate proofs for the heap budget.
//! Owner: `engine_perf::alloc` owns the pure math; this file only checks it.
//! Units: bytes in, pass/fail out. Determinism: fixed slices replay exactly.
//! Errors: test asserts only, no production path.

use engine_perf::alloc::{
    allows, heap_peak_csv, parse_heap_peak_baseline, peak_of, PeakSnapshot, HEAP_PEAK_CSV_HEADER,
};

#[test]
fn alloc_peak_same_x2() {
    let first = [4_u64, 9, 3, 17, 11];
    let second = [4_u64, 9, 3, 17, 11];
    let peak_first = PeakSnapshot {
        peak_bytes: peak_of(&first),
    };
    let peak_second = PeakSnapshot {
        peak_bytes: peak_of(&second),
    };
    assert_eq!(peak_first.peak_bytes, 17, "peak is the max sample");
    assert_eq!(peak_first, peak_second, "same slice replays the same peak");
    let baseline_bytes: u64 = 18;
    let gate_first = allows(peak_first.peak_bytes, baseline_bytes);
    let gate_second = allows(peak_second.peak_bytes, baseline_bytes);
    assert!(gate_first, "peak 17 fits within 10 percent above 18");
    assert_eq!(
        gate_first, gate_second,
        "same slice replays the same gate answer"
    );
}

#[test]
fn alloc_peak_64mib_trips_gate() {
    const MIB: u64 = 1 << 20;
    const BASELINE_PEAK: u64 = 200 * MIB;
    const EXTRA: u64 = 64 * MIB;
    assert!(
        allows(BASELINE_PEAK, BASELINE_PEAK),
        "baseline peak passes its own gate"
    );
    let tripped = BASELINE_PEAK + EXTRA;
    assert!(
        !allows(tripped, BASELINE_PEAK),
        "baseline plus 64 MiB fails the 10 percent rule"
    );
}

#[test]
fn alloc_peak_zero_baseline() {
    assert!(allows(0, 0), "zero peak fits a zero baseline");
    assert!(!allows(1, 0), "any positive peak fails a zero baseline");
    assert!(!allows(64 << 20, 0), "64 MiB fails a zero baseline");
}

#[test]
fn heap_peak_8fighter_match_same_x2_and_gate() {
    // Consumer: the 600-tick 8-fighter headless match. Peak is the max
    // full-run allocation bytes over two runs; the gate is the 10 percent
    // rule against the recorded baseline file.
    let workload = engine_perf::workloads::ScaledArenaTick::canonical_for(8).expect("8 listed");
    let first = workload.run_report();
    let second = workload.run_report();
    assert_eq!(first.ticks, 600, "headless match runs 600 ticks");
    assert_eq!(
        first.hash, second.hash,
        "8-fighter match hashes identically twice"
    );
    assert_eq!(
        first.alloc_bytes, second.alloc_bytes,
        "8-fighter match allocates identically twice"
    );
    let peak = peak_of(&[first.alloc_bytes, second.alloc_bytes]);
    assert_eq!(peak, first.alloc_bytes, "peak is the max sample");
    let csv = heap_peak_csv("arena_tick_8", peak);
    assert!(
        csv.starts_with(HEAP_PEAK_CSV_HEADER),
        "csv names the header: {csv}"
    );
    assert!(
        csv.contains(&format!("arena_tick_8,{peak}")),
        "csv names the workload and peak: {csv}"
    );
    let report_dir = format!("{}/../../target/reports", env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(&report_dir).expect("report folder exists under target");
    let report_path = format!("{report_dir}/heap-peak.csv");
    std::fs::write(&report_path, &csv).expect("heap-peak.csv writes");
    assert_eq!(
        std::fs::read_to_string(&report_path).expect("heap-peak.csv reads back"),
        csv,
        "the report file holds the same text twice"
    );
    let baseline_path = format!("{}/baselines/heap_peak.txt", env!("CARGO_MANIFEST_DIR"));
    let raw = std::fs::read_to_string(&baseline_path)
        .expect("baseline file exists after the first quiet run");
    let baseline = parse_heap_peak_baseline(&raw).expect("baseline file holds one u64");
    assert!(
        allows(peak, baseline),
        "peak {peak} fits within 10 percent above baseline {baseline}"
    );
    assert!(
        !allows(baseline + (64 << 20), baseline),
        "baseline plus 64 MiB trips the 10 percent rule"
    );
}

use engine_perf::alloc::{measure, peak_assert, subsystem_peak_line};

#[test]
fn alloc_peak_assert_names_both_sides_x2() {
    const BASE: u64 = 58198;
    const OVER: u64 = BASE + (64 << 20);
    assert!(
        peak_assert(BASE, BASE).is_ok(),
        "baseline passes its own gate"
    );
    for _ in 0..2 {
        let err = peak_assert(OVER, BASE).expect_err("64 MiB over trips");
        assert_eq!(err, peak_assert(OVER, BASE).expect_err("same error twice"));
        let text = err.to_string();
        assert!(text.contains(&OVER.to_string()), "names peak: {text}");
        assert!(text.contains(&BASE.to_string()), "names baseline: {text}");
        assert!(text.contains("FAIL"), "names verdict: {text}");
        assert!(!allows(OVER, BASE), "bool gate agrees with assert");
    }
    let line = subsystem_peak_line("heap", BASE, BASE);
    assert_eq!(
        line,
        subsystem_peak_line("heap", BASE, BASE),
        "same line twice"
    );
    assert!(line.contains("PASS"), "{line}");
    assert!(
        subsystem_peak_line("heap", OVER, BASE).contains("FAIL"),
        "over line fails"
    );
}

#[test]
fn alloc_steady_window_reads_zero_and_cooks_x2() {
    let ((), probe) = measure(|| {
        std::hint::black_box(vec![0u8; 64]);
    });
    assert_eq!((probe.count, probe.bytes), (1, 64), "probe is not blind");
    for _ in 0..2 {
        let ((), empty) = measure(|| {});
        assert_eq!(
            (empty.bytes, empty.count),
            (0, 0),
            "empty window reads zero"
        );
        let peak = peak_of(&[58198, 58198]);
        assert_eq!(peak, 58198, "peak is max sample");
        let csv = heap_peak_csv("arena_tick_8", peak);
        assert!(csv.contains("arena_tick_8,58198"), "{csv}");
    }
}
