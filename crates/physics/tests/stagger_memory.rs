//! Integration leg: stagger memory beside the balance core (the arena hit
//! pipeline feeds each landed blow to both [`BalanceState`] and
//! [`StaggerMemory`]: the blow's own level plus the pain it leaves behind).
//! Units: Q8 impulse units, ticks at 60 Hz, millimetres of limp, heap-call
//! counts. Determinism: one hit sequence hashes one way; this file asserts
//! the flinch-to-ragdoll escalation, the limp budget and the refusal, never
//! the servo.
//!
//! This binary installs the counting allocator so the probe below observes
//! real heap use; without it the maxima would read zero.

use engine_core::alloc_probe;
use engine_physics::balance::{BalanceParams, BalanceState, MAX_IMPULSE_Q8};
use engine_physics::stagger_memory::{StaggerLevel, StaggerMemory, LIMP_BUDGET_TICKS};

#[cfg(test)]
#[global_allocator]
static TEST_ALLOC: alloc_probe::CountingAlloc = alloc_probe::CountingAlloc;

/// Severity rank for the escalation proof: flinch reads 0 through ragdoll
/// reads 2, so a sorted rank row means a monotone escalation with no dice.
fn severity(level: StaggerLevel) -> u8 {
    match level {
        StaggerLevel::Flinch => 0,
        StaggerLevel::Stagger => 1,
        StaggerLevel::Ragdoll => 2,
    }
}

/// Drive the acceptance run: ten light blows (192 Q8, under the 256 stagger
/// threshold) spaced 12 ticks apart, 120 ticks of fight in all. Fails on a
/// plausible defect: a missing escalation step, a ragdoll that never
/// arrives, or twins that hash apart.
fn acceptance_run(memory: &mut StaggerMemory) -> [StaggerLevel; 10] {
    let mut levels = [StaggerLevel::Flinch; 10];
    for slot in levels.iter_mut() {
        *slot = memory.record_hit(192, 0).expect("bounded hit");
        for _ in 0..12 {
            memory.step();
        }
    }
    levels
}

#[test]
fn memory_escalates_ten_hits_in_two_seconds_flinch_stagger_ragdoll() {
    let mut memory = StaggerMemory::new();
    let levels = acceptance_run(&mut memory);
    assert_eq!(levels[0], StaggerLevel::Flinch);
    assert_eq!(levels[levels.len() - 1], StaggerLevel::Ragdoll);
    let ranks: [u8; 10] = levels.map(severity);
    let mut ordered = ranks;
    ordered.sort_unstable();
    assert_eq!(ranks, ordered, "escalation never steps down: {levels:?}");
    assert!(
        ranks.contains(&0) && ranks.contains(&1) && ranks.contains(&2),
        "ten hits must visit flinch, stagger and ragdoll in order: {levels:?}"
    );
    let first_stagger = ranks
        .iter()
        .position(|rank| *rank == 1)
        .expect("stagger hit");
    let first_ragdoll = ranks
        .iter()
        .position(|rank| *rank == 2)
        .expect("ragdoll hit");
    assert!(
        first_stagger < first_ragdoll,
        "stagger must precede ragdoll: {levels:?}"
    );
    // Twins driven through the same run hash twice; one more blow moves it.
    let mut twin = StaggerMemory::new();
    let twin_levels = acceptance_run(&mut twin);
    assert_eq!(levels, twin_levels);
    assert_eq!(memory, twin);
    assert_eq!(
        memory.state_hash(),
        twin.state_hash(),
        "same hit sequence hashes twice"
    );
    twin.record_hit(192, 0).expect("extra hit");
    assert_ne!(
        memory.state_hash(),
        twin.state_hash(),
        "hit order must reach the hash"
    );
}

#[test]
fn memory_leg_hit_leaves_limp_that_decays_inside_budget() {
    let mut memory = StaggerMemory::new();
    // One heavy blow to the leg (bone 1); the same blow never touches bone 2.
    assert_eq!(
        memory.record_hit(512, 1).expect("leg hit"),
        StaggerLevel::Stagger
    );
    let limp = memory.limp_mm(1).expect("leg limp");
    assert!(limp > 0, "a leg hit must leave a limp offset");
    assert_eq!(memory.limp_mm(2).expect("calm bone"), 0);
    // The spring recovers first: step until the level falls back to flinch,
    // and the limp must still be standing after it.
    while memory.level() != StaggerLevel::Flinch {
        memory.step();
    }
    assert!(
        memory.limp_mm(1).expect("leg limp") > 0,
        "injury persists after the spring recovers"
    );
    // Then the limp itself clears inside its own budget.
    let mut stepped: u32 = 0;
    while memory.limp_mm(1).expect("leg limp") > 0 {
        memory.step();
        stepped = stepped.saturating_add(1);
        assert!(
            stepped <= LIMP_BUDGET_TICKS,
            "limp still standing past its budget"
        );
    }
    assert!(stepped > 0, "the limp must take ticks to clear");
}

#[test]
fn memory_fatigue_lowers_servo_strength_and_recovers_slowly() {
    let mut memory = StaggerMemory::new();
    assert_eq!(memory.servo_strength_q8(), 256);
    acceptance_run(&mut memory);
    let spent = memory.servo_strength_q8();
    assert!(
        spent < 200,
        "ten hits must tire the servo: strength {spent}"
    );
    // Slow recovery: still tired after the two-second exchange itself.
    assert!(
        memory.servo_strength_q8() < 256,
        "fatigue must decay slowly, not clear with pain"
    );
    for _ in 0..1200 {
        memory.step();
    }
    assert_eq!(memory.servo_strength_q8(), 256);
}

#[test]
fn memory_wires_into_balance_hit_or_neither_moves() {
    use engine_physics::balance::BalanceError;

    let mut state = BalanceState::new(BalanceParams::default());
    let mut memory = StaggerMemory::new();
    // A light blow holds the stance steady yet still leaves pain behind.
    let (verdict, level) = state
        .apply_memory_hit(&mut memory, 192, 0)
        .expect("light wired hit");
    assert_eq!(verdict, engine_physics::balance::GaitVerdict::Steady);
    assert_eq!(level, StaggerLevel::Flinch);
    assert_eq!(state.shove_count(), 1);
    assert_eq!(memory.hit_count(), 1);
    // An over-ceiling blow is refused with both systems unchanged.
    let state_before = state.state_hash();
    let memory_before = memory.state_hash();
    assert_eq!(
        state
            .apply_memory_hit(&mut memory, MAX_IMPULSE_Q8 + 1, 0)
            .unwrap_err(),
        BalanceError::BadImpulse(MAX_IMPULSE_Q8 + 1)
    );
    assert_eq!(state.shove_count(), 1);
    assert_eq!(memory.hit_count(), 1);
    assert_eq!(state.state_hash(), state_before);
    assert_eq!(memory.state_hash(), memory_before);
    // A bad bone is refused with both systems unchanged.
    assert_eq!(
        state.apply_memory_hit(&mut memory, 192, 99).unwrap_err(),
        BalanceError::BadBone {
            bone: 99,
            bones: engine_physics::stagger_memory::MAX_MEMORY_BONES,
        }
    );
    assert_eq!(state.shove_count(), 1);
    assert_eq!(memory.hit_count(), 1);
    assert_eq!(state.state_hash(), state_before);
    assert_eq!(memory.state_hash(), memory_before);
}

#[test]
fn memory_path_alloc_free() {
    let ((), allocs) = alloc_probe::measure(|| {
        let mut memory = StaggerMemory::new();
        acceptance_run(&mut memory);
        let _ = memory.state_hash();
        let _ = memory.servo_strength_q8();
        let _ = memory.limp_mm(1);
    });
    assert_eq!(
        allocs.calls, 0,
        "memory path must not allocate, got {allocs:?}"
    );
}
