// Donor: gschup/ggrs PR #98, MIT, https://github.com/gschup/ggrs/pull/98.
// Fresh reimplementation against the engine-net rollback API; no vendor bytes.
// Idea only: a bounded drainable mismatch-event queue fed by per-tick
// checksum comparison (`P2pSession::check_desync`).

use std::collections::VecDeque;

use engine_core::Fnv64;
use engine_net::rollback::{P2pSession, RollbackError, SimTick};

/// Cap for the dispute queue: at most this many undrained events held.
const DISPUTE_CAP: usize = 100;
/// Frame where the test injects the desync.
const DESYNC_FRAME: u32 = 7;
/// Frames authored per run (holds frame 7 well inside the window).
const FRAMES: usize = 16;

/// One surfaced checksum dispute: local tape hash vs peer-claimed hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChecksumEvent {
    /// Divergent frame.
    frame: u32,
    /// Local tape hash.
    local: u64,
    /// Peer-claimed hash.
    peer: u64,
}

/// Bounded drainable mismatch-event queue (cap [`DISPUTE_CAP`]).
///
/// Push is lossy-on-full: past-cap pushes drop the newest event and count one
/// miss, so a burst can never grow memory and the consumer can still see the
/// drop via [`Self::missed`]. Drain takes every held event in frame order.
#[derive(Debug, Default)]
struct DisputeQueue {
    events: VecDeque<ChecksumEvent>,
    missed: u64,
}

impl DisputeQueue {
    /// Empty queue holding at most [`DISPUTE_CAP`] events.
    fn new() -> Self {
        Self {
            events: VecDeque::with_capacity(DISPUTE_CAP),
            missed: 0,
        }
    }

    /// Record one dispute; drops and counts a miss when full.
    fn push(&mut self, event: ChecksumEvent) {
        if self.events.len() >= DISPUTE_CAP {
            self.missed += 1;
            return;
        }
        self.events.push_back(event);
    }

    /// Feed one `check_desync` refusal into the queue.
    /// Matches push nothing; desyncs push one event; unknown frames panic
    /// as bugs (the test authors every probed frame first).
    fn observe<S: SimTick>(&mut self, session: &P2pSession<S>, frame: u32, peer: u64) {
        match session.check_desync(frame, peer) {
            Ok(()) => {}
            Err(RollbackError::Desync { frame, local, peer }) => {
                self.push(ChecksumEvent { frame, local, peer });
            }
            Err(other) => panic!("unexpected checksum probe refusal: {other}"),
        }
    }

    /// Drain every held event in order, leaving the queue empty.
    fn drain(&mut self) -> Vec<ChecksumEvent> {
        self.events.drain(..).collect()
    }

    /// Dropped pushes so far (queue-full misses).
    fn missed(&self) -> u64 {
        self.missed
    }
}

/// Integer-only toy duel: two wrapping counters, FNV hash. No float.
#[derive(Clone, Default, PartialEq, Eq)]
struct ToyDuel {
    a: i32,
    b: i32,
}

impl SimTick for ToyDuel {
    fn advance(&mut self, inputs: [u32; 2]) {
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

/// Fixed input script: deterministic per-frame pair, no RNG.
fn script(n: usize) -> (Vec<u32>, Vec<u32>) {
    let mut local = Vec::with_capacity(n);
    let mut remote = Vec::with_capacity(n);
    for f in 0..n {
        local.push((f as u32 * 7 + 1) % 5);
        remote.push((f as u32 * 11 + 2) % 5);
    }
    (local, remote)
}

/// One run: author `FRAMES` true-input frames, inject a single bad peer hash
/// at frame 7, drain the queue. Returns (drained events, missed count).
fn run_dispute() -> (Vec<ChecksumEvent>, u64) {
    let (local, remote) = script(FRAMES);
    let mut session = P2pSession::new(ToyDuel::default());
    for (&l, &r) in local.iter().zip(remote.iter()) {
        session.advance_frame(l, Some(r));
    }
    let good = session.checksum_at(DESYNC_FRAME).expect("frame 7 authored");
    let mut queue = DisputeQueue::new();
    // Green probe first: must surface nothing.
    queue.observe(&session, DESYNC_FRAME, good);
    assert!(
        queue.drain().is_empty(),
        "matching checksum must surface no event"
    );
    // Single injected desync at frame 7.
    let bad = good.wrapping_add(1);
    queue.observe(&session, DESYNC_FRAME, bad);
    let events = queue.drain();
    (events, queue.missed())
}

#[test]
fn injected_desync_at_frame_7_surfaces_one_event() {
    let (events, missed) = run_dispute();
    assert_eq!(events.len(), 1, "one desync must surface one event");
    assert_eq!(missed, 0, "single event must miss nothing");
    let event = events[0];
    assert_eq!(event.frame, DESYNC_FRAME);
    // Cross-check the hashes against the live tape (oracle, not a const).
    let (local, remote) = script(FRAMES);
    let mut oracle = P2pSession::new(ToyDuel::default());
    for (&l, &r) in local.iter().zip(remote.iter()) {
        oracle.advance_frame(l, Some(r));
    }
    let good = oracle.hash_at(DESYNC_FRAME).expect("oracle frame 7");
    assert_eq!(event.local, good);
    assert_eq!(event.peer, good.wrapping_add(1));
    // Draining again yields nothing: the queue is drainable.
    let mut queue = DisputeQueue::new();
    queue.observe(&oracle, DESYNC_FRAME, good.wrapping_add(1));
    assert_eq!(queue.drain().len(), 1);
    assert!(queue.drain().is_empty(), "second drain must be empty");
}

#[test]
fn dispute_queue_identical_x2() {
    let first = run_dispute();
    let second = run_dispute();
    assert_eq!(first.0, second.0, "same desync must match twice");
    assert_eq!(first.1, second.1, "missed count must match twice");
    assert_eq!(first.0.len(), 1);
    assert_eq!(first.1, 0);
}

#[test]
fn dispute_queue_caps_at_100_and_counts_misses() {
    let mut queue = DisputeQueue::new();
    for i in 0..(DISPUTE_CAP as u64 + 5) {
        queue.push(ChecksumEvent {
            frame: i as u32,
            local: i,
            peer: i.wrapping_add(1),
        });
    }
    let drained = queue.drain();
    assert_eq!(drained.len(), DISPUTE_CAP, "queue must hold at most 100");
    assert_eq!(queue.missed(), 5, "overflow must count misses");
    assert_eq!(drained[0].frame, 0);
    assert_eq!(drained[DISPUTE_CAP - 1].frame, (DISPUTE_CAP - 1) as u32);
}
