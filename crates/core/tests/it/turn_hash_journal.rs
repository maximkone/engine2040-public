//! Turn-hash journal: same seed twice gives same per-turn hashes.
//! Owner: engine-core. Golden unticked: this file pins no golden value,
//! so the legacy golden stays frozen. Units: ticks and hashes are u64.
//! Determinism: same seed plus same steps gives the same journal.

use engine_core::{run_ticks, Rng, TickState};

/// Journal length for the x2 check: long enough to move, short enough to stay fast.
const TICKS: u64 = 64;
/// Fixed seed for the x2 check; a second seed below proves the journal moves.
const SEED: u64 = 2040;

/// One journal: the running hash after each turn, driven tick by tick.
fn journal(seed: u64, ticks: u64) -> Vec<u64> {
    let mut rng = Rng::new(seed);
    let mut state = TickState::new(seed);
    let mut out = Vec::with_capacity(usize::try_from(ticks).expect("tick count fits"));
    for _ in 0..ticks {
        out.push(state.step(rng.next_u64()));
    }
    out
}

#[test]
fn same_seed_x2_per_turn_hashes_equal() {
    let first = journal(SEED, TICKS);
    let second = journal(SEED, TICKS);
    assert_eq!(
        first.len(),
        TICKS as usize,
        "journal holds one hash per turn"
    );
    assert_eq!(
        second.len(),
        TICKS as usize,
        "rerun holds one hash per turn"
    );
    assert_eq!(first, second, "same seed must journal same per-turn hashes");
    assert_ne!(
        first.first(),
        first.last(),
        "journal must move across turns"
    );
}

#[test]
fn journal_matches_run_ticks_driver() {
    // One home: the manual TickState loop journals what run_ticks journals.
    let manual = journal(SEED, TICKS);
    let driven = run_ticks(TICKS, SEED, |_, rng| rng.next_u64());
    assert_eq!(manual, driven, "TickState loop and run_ticks must agree");
}

#[test]
fn different_seed_journals_differ() {
    let base = journal(SEED, TICKS);
    let other = journal(7, TICKS);
    assert_ne!(base, other, "different seeds must journal different hashes");
}
