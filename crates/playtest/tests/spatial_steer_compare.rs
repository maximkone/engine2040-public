//! E42b comparison run: steered-vs-random tapes to the planted divert.
//!
//! Prints `planted-bug-found-faster` with both tape counts, then asserts
//! the steered order surfaces the planted bug in fewer tapes.

use engine_playtest::spatial_shapes::{compare, comparison_line};

#[test]
fn steered_finds_planted_bug_faster_than_random() {
    let report = compare(2040).expect("steered-vs-random comparison runs");
    let line = comparison_line(&report);
    println!("{line}");
    assert!(
        report.steered_tapes < report.random_tapes,
        "steered must surface the planted divert first: {line}"
    );
}
