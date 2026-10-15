//! G-20 backroll 500-tick diff test: rollback tape must equal truth.
//!
//! Pattern: the backroll-rs (GGPO-school) discipline reimplemented in
//! `engine_net::rollback` — `save_game_state` / `load_game_state` /
//! `advance_frame` requests over `SimTick::advance`, repeat-last prediction
//! for missing remote inputs, ordered per-player `InputQueue` slots, and
//! rollback plus resimulation on mispredict (`confirm_frame`). No `backroll`
//! crate is wired in: adding it would touch `crates/net/Cargo.toml` (outside
//! this file) and pull an async transport, so the differential runs against
//! our own code twice — one lossy rollback run plus one direct reference run
//! over the same 500-tick input script, then a hash compare. Units: frames
//! (`u32`), compact inputs (`u32`), state hashes (`u64`). Integer-only toy
//! sim (`wrapping_add`/`wrapping_mul`, FNV-1a); no wall clock, no float.

use engine_core::Fnv64;
use engine_net::rollback::{P2pSession, RollbackError, SimTick, MAX_P2P_ROLLBACK, PLAYERS};

/// Tick count for the diff: past the 129-slot window, so eviction runs.
const TICKS: usize = 500;
/// Drop every Nth remote input (prediction), then confirm it 2 frames late.
const LOSS_EVERY: u32 = 5;

/// Integer-only toy duel: two wrapping counters, FNV hash. No float.
#[derive(Clone, Default, PartialEq, Eq)]
struct TickSim {
    a: u64,
    b: u64,
}

impl SimTick for TickSim {
    fn advance(&mut self, inputs: [u32; PLAYERS]) {
        self.a = self
            .a
            .wrapping_add(u64::from(inputs[0]).wrapping_mul(0x9E37_79B9_7F4A_7C15))
            .wrapping_add(1);
        self.b = self
            .b
            .wrapping_add(u64::from(inputs[1]).wrapping_mul(0xBF58_476D_1CE4_E5B9))
            .wrapping_add(2);
    }

    fn state_hash(&self) -> u64 {
        let mut fold = Fnv64::new();
        fold.write_u64(self.a);
        fold.write_u64(self.b);
        fold.finish()
    }
}

/// Same seeded input script for every run: local and remote differ per frame
/// so a repeated (predicted) remote input usually mispredicts.
fn script(n: usize) -> (Vec<u32>, Vec<u32>) {
    let mut local = Vec::with_capacity(n);
    let mut remote = Vec::with_capacity(n);
    for f in 0..n {
        local.push((f as u32).wrapping_mul(7).wrapping_add(1) % 5);
        remote.push((f as u32).wrapping_mul(11).wrapping_add(2) % 5);
    }
    (local, remote)
}

/// Rollback run with seeded loss: drop every `LOSS_EVERY`th remote input
/// (repeat-last prediction), confirm it 2 frames late (inside the window),
/// then drain still-predicted frames back to truth (`TooFar` tolerated past
/// eviction, matching the in-tree `run_tape` helper).
fn run_rollback(local: &[u32], remote: &[u32]) -> P2pSession<TickSim> {
    let mut session = P2pSession::new(TickSim::default());
    for (f, (&l, &r)) in local.iter().zip(remote.iter()).enumerate() {
        let avail = if (f as u32) % LOSS_EVERY == LOSS_EVERY - 1 {
            None
        } else {
            Some(r)
        };
        session.advance_frame(l, avail);
        if f >= 2 && (f as u32 - 2) % LOSS_EVERY == LOSS_EVERY - 1 {
            let late = (f - 2) as u32;
            session
                .confirm_frame(late, remote[f - 2])
                .expect("late input inside window must confirm");
        }
    }
    for (f, &r) in remote.iter().enumerate() {
        if session.was_predicted(f as u32) == Some(true) {
            match session.confirm_frame(f as u32, r) {
                Ok(_) => {}
                Err(RollbackError::TooFar { .. }) => {}
                Err(other) => panic!("unexpected confirm error: {other}"),
            }
        }
    }
    session
}

/// Reference run: the same script stepped straight through the sim with true
/// inputs, one hash per frame. The rollback tape must diff empty against it.
fn run_reference(local: &[u32], remote: &[u32]) -> Vec<u64> {
    let mut sim = TickSim::default();
    let mut tape = Vec::with_capacity(local.len());
    for (&l, &r) in local.iter().zip(remote.iter()) {
        sim.advance([l, r]);
        tape.push(sim.state_hash());
    }
    tape
}

#[test]
fn tick500_rollback_matches_reference() {
    let (local, remote) = script(TICKS);
    let session = run_rollback(&local, &remote);
    let truth = run_reference(&local, &remote);

    assert_eq!(session.frames_played(), TICKS as u32);
    assert_eq!(
        session.stream(),
        truth,
        "500-tick rollback tape must diff empty vs truth"
    );
    assert!(session.rollback_count() > 0, "seeded loss must resim");
    assert!(session.max_rollback() <= MAX_P2P_ROLLBACK);
    for frame in [0, TICKS as u32 / 2, TICKS as u32 - 1] {
        let hash = truth[frame as usize];
        session
            .check_desync(frame, hash)
            .expect("peer hash green on converged tape");
    }

    // A different remote script must diverge: guards a vacuous equality.
    let mut altered = remote.clone();
    altered[257] = altered[257].wrapping_add(1);
    let divergent = run_rollback(&local, &altered);
    assert_ne!(session.stream(), divergent.stream());
}

#[test]
fn tick500_rollback_deterministic_double_run() {
    let (local, remote) = script(TICKS);
    let first = run_rollback(&local, &remote);
    let second = run_rollback(&local, &remote);
    assert_eq!(
        first.stream(),
        second.stream(),
        "same script must hash twice"
    );
    assert_eq!(
        first.sim().state_hash(),
        second.sim().state_hash(),
        "final states must agree"
    );
}
