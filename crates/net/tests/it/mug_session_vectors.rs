//! MUG 2P resim fixture vectors: 128-frame integer-only rollback, hash-equal.
//!
//! Idea-only port of the chasemcd/cogrid + mug P2P rollback pattern
//! (Apache-2.0, chasemcd/cogrid and chasemcd/mug): two peers author one
//! predicted tape, late true inputs resimulate the window, per-frame hashes
//! stay equal. Fresh implementation over `engine_net::rollback::P2pSession`;
//! no vendor bytes copied. Units: frames (`u32`), compact inputs (`u32`),
//! state hashes (`u64`). Determinism: integer-only sim (`wrapping_add`,
//! FNV-1a via `engine_core::Fnv64`); stable frame order; no float, no wall
//! clock, no `HashMap` iteration in hashed state.

use engine_core::Fnv64;
use engine_net::rollback::{P2pSession, SimTick, PLAYERS};

/// Fixture length: full 128-frame rollback window.
const FRAMES: usize = 128;

/// Integer-only 2-player duel: two wrapping counters plus a tick counter.
/// No float, no maps; the hash folds integers only.
#[derive(Clone, Default, PartialEq, Eq)]
struct MugDuel {
    a: i32,
    b: i32,
    tick: u32,
}

impl SimTick for MugDuel {
    fn advance(&mut self, inputs: [u32; PLAYERS]) {
        let l = i32::try_from(inputs[0]).expect("script input fits i32");
        let r = i32::try_from(inputs[1]).expect("script input fits i32");
        let low2 = i32::try_from(self.tick & 3).expect("2-bit mask fits i32");
        let low1 = i32::try_from(self.tick & 1).expect("1-bit mask fits i32");
        self.a = self.a.wrapping_add(l - 2).wrapping_add(low2);
        self.b = self.b.wrapping_add(r - 2).wrapping_sub(low1);
        self.tick = self.tick.wrapping_add(1);
    }

    fn state_hash(&self) -> u64 {
        // Integer fields only: no float bits, no map iteration order.
        let mut fold = Fnv64::new();
        fold.write_i32(self.a);
        fold.write_i32(self.b);
        fold.write_u32(self.tick);
        fold.finish()
    }
}

/// Deterministic input script: local and remote differ per frame so
/// repeat-last prediction usually mispredicts and forces resim.
fn script() -> (Vec<u32>, Vec<u32>) {
    let mut local = Vec::with_capacity(FRAMES);
    let mut remote = Vec::with_capacity(FRAMES);
    for f in 0..FRAMES {
        let frame = u32::try_from(f).expect("128 frames fit u32");
        local.push((frame.wrapping_mul(7).wrapping_add(1)) % 5);
        remote.push((frame.wrapping_mul(11).wrapping_add(3)) % 5);
    }
    (local, remote)
}

/// Straight run with true inputs: one hash per frame plus the tip.
fn run_straight(local: &[u32], remote: &[u32]) -> (Vec<u64>, u64) {
    let mut sim = MugDuel::default();
    let mut tape = Vec::with_capacity(local.len());
    for (&l, &r) in local.iter().zip(remote.iter()) {
        sim.advance([l, r]);
        tape.push(sim.state_hash());
    }
    (tape, sim.state_hash())
}

/// Rollback run: predict every remote input, then confirm all 128 frames.
fn run_rollback(local: &[u32], remote: &[u32]) -> P2pSession<MugDuel> {
    let mut session = P2pSession::new(MugDuel::default());
    for &l in local {
        session.advance_frame(l, None);
    }
    for (f, &r) in remote.iter().enumerate() {
        let frame = u32::try_from(f).expect("128 frames fit u32");
        session
            .confirm_frame(frame, r)
            .expect("128 frames fit the window, confirm must succeed");
    }
    session
}

#[test]
fn mug_128_resim_hash_equal() {
    let frames = u32::try_from(FRAMES).expect("128 frames fit u32");
    let (local, remote) = script();
    let (truth, tip) = run_straight(&local, &remote);
    let session = run_rollback(&local, &remote);

    assert_eq!(session.frames_played(), frames);
    assert_eq!(
        session.stream(),
        truth,
        "128-frame resim tape must equal the straight run"
    );
    assert_eq!(session.sim().state_hash(), tip);
    assert!(session.rollback_count() > 0, "mispredicts must force resim");
    assert!(
        session.max_rollback() <= FRAMES,
        "rollback depth stays in the 128 window"
    );
    let last = u32::try_from(FRAMES - 1).expect("128 frames fit u32");
    session
        .check_desync(last, tip)
        .expect("peer hash green on converged tip");
}

#[test]
fn mug_128_desync_probe_counted() {
    let frames = u32::try_from(FRAMES).expect("128 frames fit u32");
    let (local, remote) = script();
    let session = run_rollback(&local, &remote);
    let stream = session.stream();
    assert_eq!(stream.len(), FRAMES, "tape holds 128 hashes");

    // All 128 green probes pass: desyncs/128 starts at 0/128.
    let mut green = 0_u32;
    for (f, &h) in stream.iter().enumerate() {
        let frame = u32::try_from(f).expect("128 frames fit u32");
        session
            .check_desync(frame, h)
            .expect("true peer hash must pass");
        green += 1;
    }
    assert_eq!(green, frames, "128 green probes counted");
    assert_eq!(session.last_mismatch(), None, "no mismatch on truth");

    // One flipped bit on frame 64 must refuse loudly: desyncs/128 is 1/128.
    let probe = 64_u32;
    let bad = stream[64].wrapping_add(1);
    let err = session
        .check_desync(probe, bad)
        .expect_err("flipped peer hash must desync");
    assert!(
        !format!("{err}").is_empty(),
        "desync error must carry frame/hash context"
    );
    assert_eq!(session.last_mismatch(), Some(probe));
}
