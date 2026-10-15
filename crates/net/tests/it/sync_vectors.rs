//! Sync-guard vectors for the netcode history window.
//!
//! Own-code behavior vectors drawn from `crates/net/src/netcode.rs:518-579`
//! (bursty-loss tape: history <= CAP+pending, tip == true sim,
//! max_rollback <= 8). Asserts here are window-fold/tip-digest vectors, not
//! request-sequence vectors.
//!
//! Note: ggrs-fork lead comparison is unverified; no donor bytes here.

use engine_core::Fnv64;
use engine_net::netcode::{NetcodeError, Session, Sim, HISTORY_CAP, MAX_ROLLBACK};

/// Deterministic toy sim (own code): accumulates both inputs, FNV-folds state.
#[derive(Clone, Default, PartialEq, Eq)]
struct Toy {
    a: i32,
    b: i32,
}

impl Sim for Toy {
    fn step(&mut self, inputs: [u32; 2]) {
        let la = i32::try_from(inputs[0]).unwrap_or(3);
        let ra = i32::try_from(inputs[1]).unwrap_or(3);
        self.a = self.a.wrapping_add(la - 1);
        self.b = self.b.wrapping_add(ra - 1);
    }

    fn hash(&self) -> u64 {
        let mut fold = Fnv64::new();
        fold.write_i32(self.a);
        fold.write_i32(self.b);
        fold.finish()
    }
}

fn seq(count: usize, k: u32) -> Vec<u32> {
    let mut out = Vec::with_capacity(count);
    for f in 0..count {
        let frame = u32::try_from(f).expect("tape fits u32");
        out.push((frame.wrapping_mul(k).wrapping_add(1)) % 4);
    }
    out
}

fn fold_stream(stream: &[u64]) -> u64 {
    let mut fold = Fnv64::new();
    for h in stream {
        fold.write_u64(*h);
    }
    fold.finish()
}

fn tip_digest(fold: u64, tip: u64, rollbacks: u64) -> u64 {
    let mut d = Fnv64::new();
    d.write_u64(fold);
    d.write_u64(tip);
    d.write_u64(rollbacks);
    d.finish()
}

/// Window cases (tape length, drop stride): 3 cases.
const CASES: [(usize, usize); 3] = [(64, 5), (32, 4), (24, 6)];

struct TapeOutcome {
    stream: Vec<u64>,
    fold: u64,
    tip: u64,
    digest: u64,
    max_rollback: usize,
    history_len: usize,
    pending_len: usize,
    rollbacks: u64,
}

fn run_tape(frames: usize, stride: usize) -> TapeOutcome {
    let (local, remote) = (seq(frames, 1), seq(frames, 3));
    let mut s = Session::new(Toy::default());
    for f in 0..frames {
        let avail = if f % stride == stride - 1 {
            None
        } else {
            Some(remote[f])
        };
        s.advance(local[f], avail);
        // Late confirm two frames behind, once its drop slot arrives.
        if f >= 2 && (f - 2) % stride == stride - 1 {
            let frame = u32::try_from(f - 2).expect("tape fits u32");
            s.confirm(frame, remote[f - 2])
                .expect("late input inside window");
        }
    }
    for (f, &r) in remote.iter().enumerate() {
        if s.was_predicted(u32::try_from(f).expect("tape fits u32")) == Some(true) {
            match s.confirm(u32::try_from(f).expect("tape fits u32"), r) {
                Ok(_) => {}
                Err(NetcodeError::TooFar { .. }) => {}
                Err(e) => panic!("unexpected confirm err: {e}"),
            }
        }
    }
    let stream = s.stream();
    let tip = *stream.last().expect("tape plays at least one frame");
    let fold = fold_stream(&stream);
    let digest = tip_digest(fold, tip, s.rollback_count());
    TapeOutcome {
        stream,
        fold,
        tip,
        digest,
        max_rollback: s.max_rollback(),
        history_len: s.history_len(),
        pending_len: s.pending_len(),
        rollbacks: s.rollback_count(),
    }
}

fn true_tip(frames: usize) -> u64 {
    let (local, remote) = (seq(frames, 1), seq(frames, 3));
    let mut refs = Toy::default();
    for f in 0..frames {
        refs.step([local[f], remote[f]]);
    }
    refs.hash()
}

#[test]
fn window_fold_deterministic_x2() {
    // window cases n = 3: each tape replays identically with same fold.
    let mut n = 0_usize;
    for (frames, stride) in CASES {
        let first = run_tape(frames, stride);
        let second = run_tape(frames, stride);
        assert_eq!(first.stream, second.stream, "tape must replay identically");
        assert_eq!(
            (first.fold, first.tip, first.digest),
            (second.fold, second.tip, second.digest),
            "window-fold/tip-digest stable x2"
        );
        assert_eq!(
            (first.max_rollback, first.history_len),
            (second.max_rollback, second.history_len),
            "window shape stable x2"
        );
        n += 1;
    }
    assert_eq!(n, 3, "window cases n");
}

#[test]
fn tip_matches_true_sim() {
    // window cases n = 3: predicted tip converges to the true-input sim.
    let mut n = 0_usize;
    for (frames, stride) in CASES {
        let out = run_tape(frames, stride);
        assert_eq!(out.tip, true_tip(frames), "tip equals true sim");
        n += 1;
    }
    assert_eq!(n, 3, "window cases n");
}

#[test]
fn history_bounded_cap_plus_pending() {
    // window cases n = 3: history <= CAP + pending, max_rollback <= 8.
    let mut n = 0_usize;
    for (frames, stride) in CASES {
        let out = run_tape(frames, stride);
        assert!(
            out.history_len <= HISTORY_CAP + out.pending_len,
            "history {} > CAP+pending {}",
            out.history_len,
            HISTORY_CAP + out.pending_len
        );
        assert!(
            out.max_rollback <= MAX_ROLLBACK,
            "max_rollback {} > cap {MAX_ROLLBACK}",
            out.max_rollback
        );
        assert!(
            out.max_rollback <= 8,
            "max_rollback {} > 8",
            out.max_rollback
        );
        assert_eq!(out.stream.len(), out.history_len, "stream mirrors history");
        n += 1;
    }
    assert_eq!(n, 3, "window cases n");
}

#[test]
fn tip_digest_covers_fold_tip_and_rollbacks() {
    // Recomputing the digest from parts must match; flipping one part flips it.
    let out = run_tape(64, 5);
    assert_eq!(
        out.digest,
        tip_digest(out.fold, out.tip, out.rollbacks),
        "digest covers fold+tip+rollbacks"
    );
    let flipped = tip_digest(out.fold ^ 1, out.tip, out.rollbacks);
    assert_ne!(out.digest, flipped, "fold bit must move the digest");
}
