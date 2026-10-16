//! Thread-independence proof: 50 concurrent canonical runs of every workload
//! hash identically, and the arena run's allocation totals match too. Each
//! run owns all its state locally; nothing is shared between threads.

use engine_perf::workloads::{AiDecide, ArenaTick, DestructionStep, PhysicsStep};

/// Affine seed tweak that never aliases the legacy low-bit forcing: several
/// high bits flip, so the stream provably differs.
const SEED_TWEAK: u64 = 0x9E37_79B9;

#[test]
fn fifty_concurrent_canonical_runs_are_identical() {
    let arena = ArenaTick::canonical().run_hash();
    let physics = PhysicsStep::canonical().run_hash();
    let ai = AiDecide::canonical().run_hash();
    let destruction = DestructionStep::canonical().run_hash();
    std::thread::scope(|s| {
        let mut handles = Vec::with_capacity(50);
        for _ in 0..50 {
            handles.push(s.spawn(|| {
                (
                    ArenaTick::canonical().run_hash(),
                    PhysicsStep::canonical().run_hash(),
                    AiDecide::canonical().run_hash(),
                    DestructionStep::canonical().run_hash(),
                )
            }));
        }
        for handle in handles {
            let (a, p, i, d) = handle.join().expect("worker joins");
            assert_eq!(a, arena, "arena_tick identical on all threads");
            assert_eq!(p, physics, "physics_step identical on all threads");
            assert_eq!(i, ai, "ai_decide identical on all threads");
            assert_eq!(d, destruction, "destruction_step identical on all threads");
        }
    });
}

#[test]
fn fifty_concurrent_arena_runs_allocate_identically() {
    let expected = ArenaTick::canonical().run_report();
    std::thread::scope(|s| {
        let mut handles = Vec::with_capacity(50);
        for _ in 0..50 {
            handles.push(s.spawn(|| ArenaTick::canonical().run_report()));
        }
        for handle in handles {
            let report = handle.join().expect("worker joins");
            assert_eq!(report, expected, "arena budgets hold under parallelism");
        }
    });
}

#[test]
fn perturbed_seeds_change_every_hash() {
    assert_ne!(
        ArenaTick::canonical().run_hash(),
        ArenaTick::canonical().with_seed(1 ^ SEED_TWEAK).run_hash(),
        "arena_tick observes its seed"
    );
    assert_ne!(
        PhysicsStep::canonical().run_hash(),
        PhysicsStep::canonical()
            .with_seed(4_242 ^ SEED_TWEAK)
            .run_hash(),
        "physics_step observes its seed"
    );
    assert_ne!(
        AiDecide::canonical().run_hash(),
        AiDecide::canonical()
            .with_seed(9_001 ^ SEED_TWEAK)
            .run_hash(),
        "ai_decide observes its seed"
    );
    assert_ne!(
        DestructionStep::canonical().run_hash(),
        DestructionStep::canonical()
            .with_seed(5_150 ^ SEED_TWEAK)
            .run_hash(),
        "destruction_step observes its seed"
    );
}
