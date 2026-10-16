//! Paired-rerun harness proof: two headless captures share one hash and the
//! receipt repeats modulo wall-clock.
//!
//! Consumer: the perf report prints the paired receipt beside the single-run
//! frame report. Units: ticks, milliseconds, hash hex. Determinism: hashes
//! repeat exactly; only wall-clock milliseconds vary. Errors: ProfileError
//! names every refusal (disabled capture, divergent pair).

use engine_perf::profiler::{self, ProfileError, MARK_EVERY, TICK_FRAME_TICKS};
use engine_perf::workloads::ArenaTick;
use std::sync::Mutex;

static SERIAL: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().expect("paired test mutex poisons never")
}

/// Fast leg: two small headless runs share one hash and the receipt names
/// both sides plus the machine line; the render repeats byte-for-byte.
#[test]
fn tick_frame_paired_small_runs_share_one_hash() {
    let _guard = lock();
    profiler::set_enabled(true);
    let a = profiler::capture_tick_frame(16).expect("first 16-tick run captures");
    let b = profiler::capture_tick_frame(16).expect("second 16-tick run captures");
    profiler::set_enabled(false);
    assert_eq!(a.ticks, 16);
    assert_eq!(b.ticks, 16);
    assert_eq!(a.hash, b.hash, "paired runs share one hash");
    assert_eq!(
        a.hash,
        ArenaTick::canonical().run_hash(),
        "paired hash equals the off-gate hash"
    );
    let first = profiler::paired_tick_frame_summary(&a, &b).expect("equal hashes receipt");
    assert!(
        first.contains("hash_A == hash_B"),
        "receipt names both hashes"
    );
    assert!(first.contains("tick A p50=") && first.contains("| B p50="));
    assert!(first.contains("frame A p50="));
    assert!(first.contains("machine=") && first.contains("profile="));
    // Pure render repeats byte-for-byte; only live wall-clock varies.
    let second = profiler::paired_tick_frame_summary(&a, &b).expect("rerun repeats");
    assert_eq!(first, second, "receipt repeats modulo wall-clock");
    println!("{first}");
}

/// Kill rules through the real consumer: a flipped hash kills the receipt
/// and a disabled profiler refuses both capture shapes.
#[test]
fn tick_frame_paired_receipt_kills_divergence() {
    let _guard = lock();
    profiler::set_enabled(true);
    let a = profiler::capture_tick_frame(16).expect("16-tick run captures");
    profiler::set_enabled(false);
    let mut split = a.clone();
    split.hash = a.hash.wrapping_add(1);
    assert_eq!(
        profiler::paired_tick_frame_summary(&a, &split),
        Err(ProfileError::DivergedPair {
            hash_a: a.hash,
            hash_b: split.hash,
        }),
        "any hash mismatch kills the receipt"
    );
    profiler::set_enabled(false);
    assert_eq!(
        profiler::capture_tick_frame(16),
        Err(ProfileError::Disabled),
        "off profiler refuses the run"
    );
    assert_eq!(
        profiler::capture_paired_tick_frame(),
        Err(ProfileError::Disabled),
        "off profiler refuses the paired run"
    );
}

/// Full proof at the canonical count: paired 600-tick runs share one hash
/// and print the quantile table with the machine line.
#[test]
#[ignore = "determinism receipt: paired 600-tick capture plus quantile table; run under --perf"]
fn tick_frame_paired_600_runs_share_one_hash() {
    let _guard = lock();
    profiler::set_enabled(true);
    let (a, b) = profiler::capture_paired_tick_frame().expect("paired 600-tick runs capture");
    profiler::set_enabled(false);
    assert_eq!(a.ticks, TICK_FRAME_TICKS);
    assert_eq!(b.ticks, TICK_FRAME_TICKS);
    assert_eq!(a.hash, b.hash, "paired 600-tick runs share one hash");
    assert_eq!(
        a.hash,
        ArenaTick::canonical().run_hash(),
        "paired hash equals the off-gate hash"
    );
    assert_eq!(a.marks.len(), 2 * (TICK_FRAME_TICKS / MARK_EVERY) as usize);
    let receipt = profiler::paired_tick_frame_summary(&a, &b).expect("equal hashes receipt");
    assert!(receipt.contains("hash_A == hash_B"));
    assert!(receipt.contains("tick A p50=") && receipt.contains("frame A p50="));
    assert!(receipt.contains("machine=") && receipt.contains("profile="));
    println!("{receipt}");
}
