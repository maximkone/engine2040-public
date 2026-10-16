//! Integration leg: the public shove-response matrix answers the five
//! standard strengths from outside the crate (the arena hit pipeline calls
//! the same surface: [`response_matrix`] for the table, [`BalanceState`]
//! for the drive). Units: Q8 impulse units, ticks at 60 Hz, heap-call
//! counts. Determinism: one strength sequence hashes one way; this file
//! asserts the order, the budget and the refusal, never the servo.
//!
//! This binary installs the counting allocator so the probe below observes
//! real heap use; without it the maxima would read zero.

use engine_core::alloc_probe;
use engine_physics::balance::{
    response_matrix, BalanceParams, BalanceState, ResponseRow, MAX_IMPULSE_Q8,
    RECOVERY_BUDGET_TICKS, RESPONSE_Q8,
};

#[cfg(test)]
#[global_allocator]
static TEST_ALLOC: alloc_probe::CountingAlloc = alloc_probe::CountingAlloc;

/// Drive one matrix row on a fresh state to a legal stance. Fails the test
/// on a plausible defect: a wrong verdict, a missed row tick or a state
/// that never goes legal inside the budget.
fn drive_row(row: ResponseRow) {
    let mut state = BalanceState::new(BalanceParams::default());
    let dealt = state
        .apply_impact(row.strength_q8)
        .expect("matrix strengths sit inside the ceiling");
    assert_eq!(
        dealt, row.verdict,
        "the machine deals what the matrix promises"
    );
    let mut stepped: u32 = 0;
    while !state.stance_legal() {
        state.step();
        stepped = stepped.saturating_add(1);
        assert!(
            stepped <= RECOVERY_BUDGET_TICKS,
            "strength {} still illegal past the budget",
            row.strength_q8
        );
    }
    assert_eq!(
        stepped, row.recovery_ticks,
        "strength {} must close on its row tick",
        row.strength_q8
    );
    // The driver takes the same path: one apply plus the same steps.
    let mut driven = BalanceState::new(BalanceParams::default());
    let took = driven
        .respond(row.strength_q8)
        .expect("matrix strengths sit inside the ceiling");
    assert_eq!(took, row.recovery_ticks);
    assert!(driven.stance_legal());
    assert_eq!(driven, state);
}

#[test]
fn matrix_orders_five_strengths_and_recovers_within_budget() {
    use engine_physics::balance::GaitVerdict::{Fallen, Staggered, Steady, Stumbling};

    let matrix = response_matrix(BalanceParams::default());
    assert_eq!(
        matrix.map(|row| row.strength_q8),
        RESPONSE_Q8,
        "the matrix answers the shared five-strength table"
    );
    assert_eq!(
        matrix.map(|row| row.verdict),
        [Steady, Staggered, Stumbling, Stumbling, Fallen],
        "default tuning orders 1/5 steady, 2/5 stagger, 3-4/5 stumble, 5/5 fall"
    );
    for row in matrix {
        drive_row(row);
    }
    // Twins driven through the whole matrix hash twice; one more fall moves
    // the hash.
    let params = BalanceParams::default();
    let mut a = BalanceState::new(params);
    let mut b = BalanceState::new(params);
    for strength_q8 in RESPONSE_Q8 {
        a.respond(strength_q8)
            .expect("matrix strengths sit inside the ceiling");
        b.respond(strength_q8)
            .expect("matrix strengths sit inside the ceiling");
    }
    assert_eq!(a, b);
    assert_eq!(
        a.state_hash(),
        b.state_hash(),
        "same matrix run hashes twice"
    );
    b.respond(640).expect("second fall");
    assert_ne!(
        a.state_hash(),
        b.state_hash(),
        "event order must reach the hash"
    );
}

#[test]
fn matrix_refuses_over_ceiling_unchanged() {
    use engine_physics::balance::BalanceError;

    let mut state = BalanceState::new(BalanceParams::default());
    let before = state.state_hash();
    assert_eq!(
        state.respond(MAX_IMPULSE_Q8 + 1).unwrap_err(),
        BalanceError::BadImpulse(MAX_IMPULSE_Q8 + 1)
    );
    assert_eq!(state.shove_count(), 0);
    assert_eq!(state.fall_count(), 0);
    assert!(state.stance_legal());
    assert_eq!(state.state_hash(), before);
}

#[test]
fn matrix_path_alloc_free() {
    let params = BalanceParams::default();
    let ((), allocs) = alloc_probe::measure(|| {
        let matrix = response_matrix(params);
        let mut state = BalanceState::new(params);
        for row in matrix {
            state
                .respond(row.strength_q8)
                .expect("matrix strengths sit inside the ceiling");
        }
        let _ = state.state_hash();
    });
    assert_eq!(
        allocs.calls, 0,
        "matrix path must not allocate, got {allocs:?}"
    );
}
