// Donor: cBournhonesque/lightyear `prediction_window.rs`, Apache-2.0.
// Licence: Apache-2.0 (pattern only, no vendor bytes copied).
// Link: https://github.com/cBournhonesque/lightyear
//
//! Prediction-window wait gate over the rollback tape: cap rollback resims.
//!
//! The client may run ahead of the last server-confirmed tick by at most
//! `max_predicted_ticks` frames. At the edge the client waits instead of
//! predicting, so a late true input can only resimulate a bounded window.
//! Names fit `engine_net::rollback` (`P2pSession`, `SimTick`, frames as
//! `u32`); integer-only toy sim, no wall clock, no float.

use engine_core::Fnv64;
use engine_net::rollback::{P2pSession, SimTick, PLAYERS};

/// Wait-gate decision for one tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gate {
    /// Prediction budget remains: author the next frame.
    Advance,
    /// Window is full: wait for a confirmation before predicting further.
    Wait,
}

/// Lightyear-style prediction window (pattern only): at most
/// `max_predicted_ticks` unconfirmed frames ahead of `confirmed_tick`.
#[derive(Clone, Copy, Debug)]
struct PredictionWindow {
    max_predicted_ticks: u32,
}

impl PredictionWindow {
    /// New gate with the given unconfirmed-frame budget.
    fn new(max_predicted_ticks: u32) -> Self {
        Self {
            max_predicted_ticks,
        }
    }

    /// Unconfirmed depth: how far `current_tick` runs ahead of
    /// `confirmed_tick` (saturates at zero when confirmation leads).
    fn depth(current_tick: u32, confirmed_tick: u32) -> u32 {
        current_tick.saturating_sub(confirmed_tick)
    }

    /// Wait once the unconfirmed depth reaches the budget, else advance.
    fn decide(&self, current_tick: u32, confirmed_tick: u32) -> Gate {
        if Self::depth(current_tick, confirmed_tick) >= self.max_predicted_ticks {
            Gate::Wait
        } else {
            Gate::Advance
        }
    }
}

/// Integer-only toy duel behind the tape: two wrapping counters, FNV hash.
#[derive(Clone, Default, PartialEq, Eq)]
struct ToyDuel {
    a: i32,
    b: i32,
}

impl SimTick for ToyDuel {
    fn advance(&mut self, inputs: [u32; PLAYERS]) {
        let local = i32::try_from(inputs[0] % 5).unwrap_or(0);
        let remote = i32::try_from(inputs[1] % 5).unwrap_or(0);
        self.a = self.a.wrapping_add(local - 1);
        self.b = self.b.wrapping_add(remote - 1);
    }

    fn state_hash(&self) -> u64 {
        let mut fold = Fnv64::new();
        fold.write_i32(self.a);
        fold.write_i32(self.b);
        fold.finish()
    }
}

/// Deterministic input scripts (same complete input gives the same tape).
fn script(n: usize) -> (Vec<u32>, Vec<u32>) {
    let mut local = Vec::with_capacity(n);
    let mut remote = Vec::with_capacity(n);
    for f in 0..n {
        let frame = u32::try_from(f).expect("tape fits u32");
        local.push((frame.wrapping_mul(7).wrapping_add(1)) % 5);
        remote.push((frame.wrapping_mul(11).wrapping_add(2)) % 5);
    }
    (local, remote)
}

/// Gated run: advance while the window allows, otherwise confirm the oldest
/// unconfirmed frame (server ack) before predicting further. Returns
/// (tape, rollbacks, max_rollback, confirmed_tip).
fn run_gated(local: &[u32], remote: &[u32], max_predicted: u32) -> (Vec<u64>, u64, usize, u32) {
    let gate = PredictionWindow::new(max_predicted);
    let mut session = P2pSession::new(ToyDuel::default());
    let mut confirmed: u32 = 0;
    let mut next: usize = 0;
    while next < local.len() || confirmed < u32::try_from(remote.len()).expect("tape fits u32") {
        let current = session.frame();
        if next < local.len() && gate.decide(current, confirmed) == Gate::Advance {
            // Remote always late here: pure repeat-last prediction.
            session.advance_frame(local[next], None);
            next += 1;
        } else if confirmed < u32::try_from(remote.len()).expect("tape fits u32") {
            let idx = usize::try_from(confirmed).expect("tape fits usize");
            session
                .confirm_frame(confirmed, remote[idx])
                .expect("gated confirm stays in window");
            confirmed += 1;
        } else {
            break;
        }
    }
    let tape = session.stream();
    let resims = session.rollback_count();
    let deepest = session.max_rollback();
    (tape, resims, deepest, confirmed)
}

#[test]
fn advances_in_window() {
    let gate = PredictionWindow::new(4);
    assert_eq!(gate.decide(0, 0), Gate::Advance);
    assert_eq!(gate.decide(3, 0), Gate::Advance);
    assert_eq!(gate.decide(5, 2), Gate::Advance);
}

#[test]
fn waits_at_edge() {
    let gate = PredictionWindow::new(4);
    // Depth == budget waits; deeper still waits; zero budget always waits.
    assert_eq!(gate.decide(4, 0), Gate::Wait);
    assert_eq!(gate.decide(9, 2), Gate::Wait);
    assert_eq!(PredictionWindow::new(0).decide(0, 0), Gate::Wait);
}

#[test]
fn depth_exact() {
    assert_eq!(PredictionWindow::depth(0, 0), 0);
    assert_eq!(PredictionWindow::depth(7, 3), 4);
    // Confirmation leading the tip saturates instead of wrapping.
    assert_eq!(PredictionWindow::depth(2, 9), 0);
    assert_eq!(PredictionWindow::depth(u32::MAX, 0), u32::MAX);
}

#[test]
fn confirms_drain_the_queue() {
    let (local, remote) = script(8);
    let gate = PredictionWindow::new(4);
    let mut session = P2pSession::new(ToyDuel::default());
    let mut confirmed: u32 = 0;
    for &l in &local {
        assert_eq!(
            gate.decide(session.frame(), confirmed),
            Gate::Advance,
            "depth stays under budget while acks keep up"
        );
        session.advance_frame(l, None);
        // Ack the oldest unconfirmed frame once 4 are unconfirmed.
        if PredictionWindow::depth(session.frame(), confirmed) >= 4 {
            let idx = usize::try_from(confirmed).expect("tape fits usize");
            session
                .confirm_frame(confirmed, remote[idx])
                .expect("ack stays in window");
            confirmed += 1;
        }
    }
    for (f, &r) in remote.iter().enumerate() {
        let frame = u32::try_from(f).expect("tape fits u32");
        if session.was_predicted(frame) == Some(true) {
            session
                .confirm_frame(frame, r)
                .expect("drain confirms stay in window");
        }
    }
    let dropped = session.discard_confirmed_frames(8);
    assert_eq!(dropped, 8, "all 8 confirmed frames must drain");
    assert_eq!(session.oldest_frame(), 8);
    assert_eq!(
        session.queue(1).and_then(|q| q.first_frame()),
        None,
        "remote queue drains with the window"
    );
    assert!(
        session.max_rollback() <= 4,
        "wait gate caps resims at the window"
    );
    println!(
        "pred_window resim count={} deepest={} dropped={}",
        session.rollback_count(),
        session.max_rollback(),
        dropped
    );
}

#[test]
fn determinism_x2() {
    let (local, remote) = script(16);
    let (tape_a, resims_a, deep_a, confirmed_a) = run_gated(&local, &remote, 4);
    let (tape_b, resims_b, deep_b, confirmed_b) = run_gated(&local, &remote, 4);
    assert_eq!(tape_a, tape_b, "same inputs gate to the same tape");
    assert_eq!(
        (resims_a, deep_a, confirmed_a),
        (resims_b, deep_b, confirmed_b)
    );
    assert!(resims_a > 0, "late remote inputs must force resims");
    assert!(deep_a <= 4, "wait gate caps rollback depth, got {deep_a}");
    assert_eq!(confirmed_a, 16);
    println!("pred_window resim count={resims_a} deepest={deep_a}");
}
