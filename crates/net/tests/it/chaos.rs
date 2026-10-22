//! Rollback chaos harness: seeded loss, delay and duplication over the P2P tape.
//!
//! Purpose: prove the rollback session converges to the true-input replay
//! under network chaos and refuses out-of-window or forged frames loudly.
//! Owner: net lane. Units: frames (`u32`), compact inputs (`u32`), hashes
//! (`u64`). Determinism: integer-only toy sim plus a seeded `mix64` stream;
//! same seed always yields the same tape. Errors: `RollbackError` variants
//! are asserted, never unwrapped. License: MIT (workspace). CHAOS
//! (idea-only, no vendored bytes): the GGRS-style save/load/advance plus
//! confirm discipline reimplemented over `P2pSession`; the loss/delay/dup
//! matrix shape follows the Rollback-Core chaos-harness school as an idea
//! only per CODELAW 9.

use engine_core::hash::GOLDEN_GAMMA;
use engine_core::mix64;
use engine_core::Fnv64;
use engine_net::rollback::{
    InputQueue, P2pSession, RollbackError, SimTick, INPUT_QUEUE_LEN, MAX_P2P_ROLLBACK, PLAYERS,
};

/// One seeded step over held state through the shared owner.
fn next_rand(state: &mut u64) -> u64 {
    *state = state.wrapping_add(GOLDEN_GAMMA);
    mix64(*state)
}

/// Integer-only toy duel: two wrapping counters, FNV hash. No float.
#[derive(Clone, Default, PartialEq, Eq)]
struct ToyDuel {
    a: i32,
    b: i32,
}

impl SimTick for ToyDuel {
    fn advance(&mut self, inputs: [u32; PLAYERS]) {
        self.a = self.a.wrapping_add(inputs[0] as i32 - 1);
        self.b = self.b.wrapping_add(inputs[1] as i32 - 1);
    }

    fn state_hash(&self) -> u64 {
        let mut fold = Fnv64::new();
        fold.write_i32(self.a);
        fold.write_i32(self.b);
        fold.finish()
    }
}

/// Script inputs: distinct local/remote streams so mispredicts resimulate.
fn script_inputs(n: usize) -> (Vec<u32>, Vec<u32>) {
    let mut local = Vec::with_capacity(n);
    let mut remote = Vec::with_capacity(n);
    for f in 0..n {
        local.push((f as u32 * 7 + 1) % 5);
        remote.push((f as u32 * 11 + 2) % 5);
    }
    (local, remote)
}

/// True-input replay: every remote input present, no prediction.
fn truth_stream(local: &[u32], remote: &[u32]) -> Vec<u64> {
    let mut session = P2pSession::new(ToyDuel::default());
    for (&l, &r) in local.iter().zip(remote.iter()) {
        session.advance_frame(l, Some(r));
    }
    session.stream()
}

/// Chaos run: seeded per-frame loss (predict), 2-frame delayed confirms,
/// and duplicate confirms. Returns the session plus late-confirm count.
fn run_chaos(seed: u64, frames: usize) -> (P2pSession<ToyDuel>, u32) {
    let (local, remote) = script_inputs(frames);
    let mut rng = if seed == 0 { 1 } else { seed };
    let mut session = P2pSession::new(ToyDuel::default());
    let mut late = 0u32;
    for (f, (&l, &_r)) in local.iter().zip(remote.iter()).enumerate() {
        rng = next_rand(&mut rng);
        let drop = rng % 4 == 0;
        if drop {
            session.advance_frame(l, None);
        } else {
            session.advance_frame(l, Some(remote[f]));
        }
        // Delayed confirm: late truth for the frame two ticks back, plus a
        // duplicate confirm to prove idempotence under duplication.
        if f >= 2 {
            rng = next_rand(&mut rng);
            if rng % 3 == 0 {
                let back = (f - 2) as u32;
                if session.was_predicted(back) == Some(true) {
                    session
                        .confirm_frame(back, remote[f - 2])
                        .expect("delayed truth inside window confirms");
                    late += 1;
                    let before = session.stream();
                    assert_eq!(
                        session.confirm_frame(back, remote[f - 2]),
                        Ok(0),
                        "duplicate confirms stay idempotent"
                    );
                    assert_eq!(session.stream(), before, "dup confirm changes nothing");
                }
            }
        }
    }
    // Drain: confirm every still-predicted frame back to truth.
    for (f, &r) in remote.iter().enumerate() {
        if session.was_predicted(f as u32) == Some(true) {
            match session.confirm_frame(f as u32, r) {
                Ok(_) => {}
                Err(RollbackError::TooFar { .. }) => {}
                Err(other) => panic!("unexpected confirm error: {other}"),
            }
        }
    }
    (session, late)
}

/// Same seed gives the same tape, counters and loss outcome twice.
#[test]
fn chaos_same_seed_same_tape() {
    let (first, late_first) = run_chaos(0xCA05_C0ED_1234_5678, 64);
    let (second, late_second) = run_chaos(0xCA05_C0ED_1234_5678, 64);
    assert_eq!(first.stream(), second.stream(), "same seed, same tape");
    assert_eq!(
        (first.rollback_count(), first.max_rollback()),
        (second.rollback_count(), second.max_rollback()),
        "same seed, same resim counts"
    );
    assert_eq!(late_first, late_second, "same seed, same delay count");
    assert!(first.rollback_count() > 0, "seeded loss must resim");
    assert!(
        first.max_rollback() <= MAX_P2P_ROLLBACK,
        "window bound holds"
    );
    // Neighbour seeds drive a different loss and delay schedule, which guards
    // the same-seed equality against a vacuous pass. The tape itself is
    // seed-independent by design: the drain confirms every predicted frame back
    // to truth (chaos_converges_to_truth), so the schedule is the observable.
    let (other, late_other) = run_chaos(0xCA05_C0ED_1234_5679, 64);
    assert_ne!(
        (first.predicted_count(), late_first),
        (other.predicted_count(), late_other),
        "neighbour seeds diverge"
    );
    assert_eq!(
        first.stream(),
        other.stream(),
        "every seed converges to the same truth"
    );
}

/// Chaos converges to the true-input replay with green peer hashes.
#[test]
fn chaos_converges_to_truth() {
    let frames = 64;
    let (local, remote) = script_inputs(frames);
    let (session, _) = run_chaos(0xD1CE_1234_5678_9ABC, frames);
    assert_eq!(session.frames_played(), frames as u32);
    assert!(
        session.len() <= MAX_P2P_ROLLBACK + 1,
        "window stays bounded"
    );
    assert_eq!(session.queue_len(0), Some(INPUT_QUEUE_LEN.min(frames)));
    assert_eq!(session.queue_len(1), Some(INPUT_QUEUE_LEN.min(frames)));
    let mut truth = ToyDuel::default();
    for (&l, &r) in local.iter().zip(remote.iter()) {
        truth.advance([l, r]);
    }
    assert_eq!(
        session.sim().state_hash(),
        truth.state_hash(),
        "chaos converges to truth"
    );
    assert_eq!(
        session.stream(),
        truth_stream(&local, &remote),
        "replay diff empty after chaos"
    );
    for f in 0..frames as u32 {
        let hash = session.hash_at(f).expect("authored frame hashed");
        session.check_desync(f, hash).expect("peer hash green");
    }
}

/// Out-of-window and forged frames refuse loudly and change nothing.
#[test]
fn chaos_refusals_hold() {
    let frames = MAX_P2P_ROLLBACK + 2;
    let (local, remote) = script_inputs(frames);
    let mut session = P2pSession::new(ToyDuel::default());
    for (&l, &r) in local.iter().zip(remote.iter()) {
        session.advance_frame(l, Some(r));
    }
    let oldest = session.oldest_frame();
    let before = session.stream();
    assert_eq!(
        session.confirm_frame(0, remote[0].wrapping_add(1)),
        Err(RollbackError::TooFar { frame: 0, oldest }),
        "evicted frames refuse"
    );
    assert_eq!(session.stream(), before, "refused confirm changes nothing");
    let far = session.frame() + MAX_P2P_ROLLBACK as u32 + 1;
    assert_eq!(
        session.confirm_frame(far, 1),
        Err(RollbackError::FutureFrame {
            frame: far,
            current: session.frame(),
        }),
        "far-future confirms refuse"
    );
    let good = session.hash_at(oldest).expect("oldest frame hashed");
    let err = session
        .check_desync(oldest, good.wrapping_add(1))
        .expect_err("forged peer hash must refuse");
    let RollbackError::Desync {
        frame,
        local,
        peer,
        report,
    } = err
    else {
        panic!("a forged peer hash must refuse with Desync");
    };
    assert_eq!(
        (frame, local, peer),
        (oldest, good, good.wrapping_add(1)),
        "desync context stays exact"
    );
    assert_eq!(report.tick, oldest, "the report names the divergent tick");
    assert!(
        report.inputs_agree(),
        "every frame here was authored with the true remote input: the wire is exonerated"
    );
    assert_eq!(session.stream(), before, "failed check changes nothing");
    assert_eq!(
        session.check_desync(u32::MAX, 0),
        Err(RollbackError::UnknownFrame(u32::MAX)),
        "unknown frames refuse"
    );
}

/// Queues hold 128 slots: the per-player bound never grows under chaos.
#[test]
fn chaos_queues_stay_bounded() {
    let (session, _) = run_chaos(0xBEEF_0042_1234_5678, MAX_P2P_ROLLBACK + 16);
    assert_eq!(session.len(), MAX_P2P_ROLLBACK + 1, "window caps at 129");
    assert_eq!(session.queue_len(0), Some(INPUT_QUEUE_LEN));
    assert_eq!(session.queue_len(1), Some(INPUT_QUEUE_LEN));
    let queue = session.queue(1).expect("remote queue present");
    assert!(queue.len() <= INPUT_QUEUE_LEN, "queue bound holds");
    assert!(
        session.queue(2).is_none(),
        "out-of-range players answer none"
    );
    let _fresh = InputQueue::new();
}
