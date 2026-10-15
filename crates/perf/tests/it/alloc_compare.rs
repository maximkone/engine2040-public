//! PF18 alloc-compare pin: `perf --alloc-compare` header plus 8 rows x2 identical.
//!
//! Consumer: `perf --alloc-compare` (`crates/perf/src/main.rs`
//! `alloc_compare_text` via `poll_present_flags`: header, box line, 8
//! interleaved rows, no wall delta). Units: ticks, hash, bytes, count; the ms
//! column stays `-` in the default suite. Determinism: hash/alloc rows repeat
//! exactly; wall ms never enters here (timed rows live under `--perf`, see
//! `allocator_baseline.rs`). Errors: test asserts only, no production path.

use engine_perf::allocator::{
    compare_ms, csv_row, AllocatorError, AllocatorKind, CSV_HEADER, SPEEDUP_PCT,
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
fn header_plus_8_rows_repeat_x2() {
    let first = alloc_compare_stdout();
    let second = alloc_compare_stdout();
    assert_eq!(first, second, "same backend gives the same body twice");
    let mut lines = first.lines();
    assert_eq!(
        lines.next().expect("body holds the header"),
        CSV_HEADER,
        "first line names the shared columns"
    );
    let box_line = lines.next().expect("body holds the box line");
    assert!(box_line.starts_with("# box:"), "{box_line}");
    for token in ["scheme=", "cpu=", "threads=1", "profile="] {
        assert!(box_line.contains(token), "{box_line} names {token}");
    }
    let rows: Vec<&str> = lines.collect();
    assert_eq!(
        rows.len(),
        8,
        "four workloads times two allocators: {rows:?}"
    );
    let mut workloads = Vec::with_capacity(4);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row.split(',').count(), 7, "columns never drift: {row}");
        assert!(
            row.ends_with(",-"),
            "default rows carry no wall sample: {row}"
        );
        let expected = if index.is_multiple_of(2) {
            "system,"
        } else {
            "mimalloc,"
        };
        assert!(row.starts_with(expected), "{row}");
        let fields: Vec<&str> = row.split(',').collect();
        if index.is_multiple_of(2) {
            workloads.push(fields[1].to_owned());
        } else {
            let system = &rows[index - 1];
            let left: Vec<&str> = system.split(',').collect();
            assert_eq!(left[1], fields[1], "candidate keeps the workload name");
            assert_eq!(left[2], fields[2], "candidate keeps the tick count");
            assert_eq!(left[3], fields[3], "candidate keeps the system hash");
            assert_eq!(left[4], fields[4], "candidate keeps the system bytes");
            assert_eq!(left[5], fields[5], "candidate keeps the system count");
        }
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
