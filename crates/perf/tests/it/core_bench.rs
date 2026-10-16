//! Determinism gate for the surviving canonical workloads.
//!
//! Consumer: CI keeps the retired kernel-harness number shape
//! (same seed twice gives same hash and same allocation footprint) pinned
//! against the surviving `workloads` owners, so a shared-state regression
//! fails here first. Units: hashes, bytes, counts. Determinism: seeded
//! runs only, no wall clock, no GPU. Errors: none (asserts, never panics
//! on caller input).

use engine_perf::alloc::measure;
use engine_perf::workloads::{ArenaTick, PhysicsStep};

/// The retired kernel harness is gone; the same repeat-twice shape now
/// pins the surviving canonical runs, so a thread-local counting or
/// shared-state regression trips here before the baseline gate.
#[test]
fn surviving_workloads_deterministic_x2() {
    let first_arena = ArenaTick::canonical().run_hash();
    let second_arena = ArenaTick::canonical().run_hash();
    assert_eq!(first_arena, second_arena, "arena_tick repeats");

    let (hash_a, alloc_a) = measure(|| ArenaTick::canonical().run_hash());
    let (hash_b, alloc_b) = measure(|| ArenaTick::canonical().run_hash());
    assert_eq!(hash_a, hash_b, "arena_tick hash repeats");
    assert_eq!(
        (alloc_a.bytes, alloc_a.count),
        (alloc_b.bytes, alloc_b.count),
        "arena_tick allocation footprint repeats"
    );

    let first_physics = PhysicsStep::canonical().run_hash();
    let second_physics = PhysicsStep::canonical().run_hash();
    assert_eq!(first_physics, second_physics, "physics_step repeats");

    let moved = ArenaTick::canonical().with_seed(1 ^ 0x9E37_79B9).run_hash();
    assert_ne!(moved, first_arena, "reseeded run moves the hash");
}
