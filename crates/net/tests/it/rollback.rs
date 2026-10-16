//! 8-tick rollback-resim hash test: resim equals the straight run.
//
//! Pattern: `engine_net::rollback::P2pSession` repeat-last prediction over
//! `SimTick::advance`, then late `confirm_frame` resimulates the 8-tick
//! window. The resimulated tape must equal the straight true-input run.
//! Golden unticked: no hardcoded digest yet, equality is the gate. Units:
//! frames (`u32`), compact inputs (`u32`), state hashes (`u64`).
//! Integer-only toy sim (`wrapping_add`, FNV-1a); no wall clock, no float.

use engine_core::Fnv64;
use engine_net::rollback::{P2pSession, SimTick, PLAYERS};

/// Tick count for the resim check: fits the rollback window.
const TICKS: usize = 8;

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

/// Same seeded input script for every run: local and remote differ per frame
/// so a repeated (predicted) remote input usually mispredicts.
fn script() -> (Vec<u32>, Vec<u32>) {
    let mut local = Vec::with_capacity(TICKS);
    let mut remote = Vec::with_capacity(TICKS);
    for f in 0..TICKS {
        let frame = u32::try_from(f).expect("tape fits u32");
        local.push((frame.wrapping_mul(7).wrapping_add(1)) % 5);
        remote.push((frame.wrapping_mul(11).wrapping_add(2)) % 5);
    }
    (local, remote)
}

/// Straight run: the same script stepped directly through the sim with true
/// inputs, one hash per frame.
fn run_straight(local: &[u32], remote: &[u32]) -> (Vec<u64>, u64) {
    let mut sim = ToyDuel::default();
    let mut tape = Vec::with_capacity(local.len());
    for (&l, &r) in local.iter().zip(remote.iter()) {
        sim.advance([l, r]);
        tape.push(sim.state_hash());
    }
    (tape, sim.state_hash())
}

/// Rollback run: predict every remote input (repeat-last), then confirm all
/// 8 frames back to truth so the window resimulates.
fn run_rollback(local: &[u32], remote: &[u32]) -> P2pSession<ToyDuel> {
    let mut session = P2pSession::new(ToyDuel::default());
    for &l in local {
        session.advance_frame(l, None);
    }
    for (f, &r) in remote.iter().enumerate() {
        let frame = u32::try_from(f).expect("tape fits u32");
        session
            .confirm_frame(frame, r)
            .expect("8 ticks fit the window, confirm must succeed");
    }
    session
}

#[test]
fn tick8_resim_equals_straight_run() {
    let (local, remote) = script();
    let (truth, tip) = run_straight(&local, &remote);
    let session = run_rollback(&local, &remote);

    assert_eq!(session.frames_played(), TICKS as u32);
    assert_eq!(
        session.stream(),
        truth,
        "8-tick resim tape must equal the straight run"
    );
    assert_eq!(session.sim().state_hash(), tip);
    assert!(session.rollback_count() > 0, "mispredicts must force resim");
    let last = u32::try_from(TICKS - 1).expect("tape fits u32");
    session
        .check_desync(last, tip)
        .expect("peer hash green on converged tip");
    // Golden unticked: no hardcoded digest assertion yet.
}
