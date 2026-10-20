//! PF18 alloc-compare pin: `perf --alloc-compare` header plus one row per
//! canonical workload, x2 identical.
//!
//! Consumer: `perf --alloc-compare` (`crates/perf/src/main.rs`
//! `alloc_compare_text` via `poll_present_flags`: header, box line, four rows
//! for the allocator this build installs, no wall delta). Units: ticks, hash,
//! bytes, count; the ms column stays `-` in the default suite. Determinism:
//! hash/alloc rows repeat exactly; wall ms never enters here (timed rows live
//! under `--perf`, see `allocator_baseline.rs`). One process has one global
//! allocator, so the candidate row is a second build (`--features mimalloc`)
//! of the same harness and the comparison is two runs. Errors: test asserts
//! only, no production path.

use engine_perf::allocator::{
    active_kind, compare_ms, csv_row, AllocatorError, AllocatorKind, CSV_HEADER, SPEEDUP_PCT,
};
use std::process::Command;

/// One `--alloc-compare` run straight from the built binary, so the pin
/// proves the seam (`alloc_compare_text`), never a copy of it.
fn alloc_compare_stdout() -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_perf"))
        .arg("--alloc-compare")
        .output()
        .expect("perf --alloc-compare runs");
    assert!(
        output.status.success(),
        "perf --alloc-compare exits 0: {output:?}"
    );
    String::from_utf8(output.stdout).expect("--alloc-compare prints UTF-8")
}

#[test]
fn header_plus_four_active_rows_repeat_x2() {
    let first = alloc_compare_stdout();
    let second = alloc_compare_stdout();
    assert_eq!(first, second, "same build gives the same body twice");
    let mut lines = first.lines();
    assert_eq!(
        lines.next().expect("body holds the header"),
        CSV_HEADER,
        "first line names the shared columns"
    );
    let box_line = lines.next().expect("body holds the box line");
    assert!(box_line.starts_with("# box:"), "{box_line}");
    for token in ["allocator=", "scheme=", "cpu=", "threads=1", "profile="] {
        assert!(box_line.contains(token), "{box_line} names {token}");
    }
    let kind = active_kind();
    assert!(
        box_line.contains(&format!("allocator={kind}")),
        "the box line names the backend this build installs: {box_line}"
    );
    let rows: Vec<&str> = lines.collect();
    assert_eq!(rows.len(), 4, "one row per canonical workload: {rows:?}");
    let mut workloads = Vec::with_capacity(4);
    for row in &rows {
        assert_eq!(row.split(',').count(), 7, "columns never drift: {row}");
        assert!(
            row.ends_with(",-"),
            "default rows carry no wall sample: {row}"
        );
        assert!(
            row.starts_with(&format!("{kind},")),
            "the row names the backend that produced it: {row}"
        );
        let fields: Vec<&str> = row.split(',').collect();
        workloads.push(fields[1].to_owned());
    }
    workloads.sort_unstable();
    workloads.dedup();
    assert_eq!(workloads.len(), 4, "four distinct workloads pinned");
}

#[test]
fn delta_math_repeats_x2_and_bad_samples_refuse() {
    let first = compare_ms(100.0, 90.0).expect("clean samples compare");
    let second = compare_ms(100.0, 90.0).expect("clean samples compare");
    assert_eq!(first, second, "same samples give the same delta twice");
    assert_eq!(first.delta_pct, 10.0, "100 vs 90 reads 10%");
    assert!(first.passes(), "exactly 10% clears SPEEDUP_PCT");
    let same = compare_ms(100.0, 100.0).expect("equal samples compare");
    assert_eq!(same.delta_pct, 0.0, "identical backends read 0%");
    assert!(!same.passes(), "0% keeps the system row");
    assert_eq!(SPEEDUP_PCT, 10.0, "the swap bar stays 10%");
    let probe = engine_perf::workloads::PhysicsStep::canonical().run_report();
    assert!(
        matches!(
            csv_row(&probe, AllocatorKind::System, Some(f64::NAN)),
            Err(AllocatorError::NonFiniteMs { side: "row", .. })
        ),
        "NaN row sample names its side"
    );
    assert!(
        matches!(
            compare_ms(f64::NAN, 1.0),
            Err(AllocatorError::NonFiniteMs { side: "system", .. })
        ),
        "NaN system sample names its side"
    );
    assert_eq!(
        compare_ms(0.0, 0.0),
        Err(AllocatorError::NonPositiveSystem { value_ms: 0.0 })
    );
}
