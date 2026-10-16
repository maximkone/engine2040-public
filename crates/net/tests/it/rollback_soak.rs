// Donor: gschup/ggrs, MIT, https://github.com/gschup/ggrs (src/input_queue.rs).
// Fresh port in our style against the engine-net rollback API; no vendor bytes.
// Soak: frame-delay input queue (DelayBuffer) plus P2pSession repeat-last
// prediction with first-incorrect-frame resim; fixed-seed 2-player run,
// hash equality across two runs, mispredict count printed.

use engine_core::Fnv64;
use engine_net::rollback::{P2pSession, SimTick};
use engine_net::DelayBuffer;

/// Fixed soak seed: same script on every run.
const SEED: u64 = 2040;
/// Soak length in frames (fits the rollback window).
const FRAMES: usize = 120;
/// Frame delay on the local input queue.
const DELAY: usize = 2;
/// Drop every 5th remote input to force repeat-last prediction.
const LOSS_EVERY: u32 = 5;

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

/// Fixed-seed input script: LCG over 4 input values per player.
fn script(seed: u64, n: usize) -> (Vec<u32>, Vec<u32>) {
    let mut state = seed | 1;
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((state >> 33) % 4) as u32
    };
    let mut local = Vec::with_capacity(n);
    let mut remote = Vec::with_capacity(n);
    for _ in 0..n {
        local.push(next());
        remote.push(next());
    }
    (local, remote)
}

fn fold_stream(stream: &[u64]) -> u64 {
    let mut fold = Fnv64::new();
    for h in stream {
        fold.write_u64(*h);
    }
    fold.finish()
}

/// One soak run: delay local inputs, predict missing remote, confirm late.
/// Returns (tape fold, tip hash, rollbacks, mispredicts, first bad frame).
fn run_soak(seed: u64) -> (u64, u64, u64, u64, Option<u32>) {
    let (local, remote) = script(seed, FRAMES);
    let mut queue = DelayBuffer::new(DELAY);
    let mut session = P2pSession::new(ToyDuel::default());
    let mut delayed = Vec::with_capacity(FRAMES);
    let mut mispredicts = 0u64;
    let mut first_bad = None;
    for f in 0..FRAMES {
        queue.push_local(local[f]);
        let l = queue.readable();
        delayed.push(l);
        let frame = f as u32;
        let avail = if frame % LOSS_EVERY == LOSS_EVERY - 1 {
            None
        } else {
            Some(remote[f])
        };
        session.advance_frame(l, avail);
        if f >= 2 {
            let late = (f - 2) as u32;
            if late % LOSS_EVERY == LOSS_EVERY - 1 {
                let distance = session
                    .confirm_frame(late, remote[f - 2])
                    .expect("late input inside window confirms");
                if distance > 0 {
                    mispredicts += 1;
                    first_bad.get_or_insert(late);
                }
            }
        }
    }
    for (f, item) in remote.iter().enumerate().take(FRAMES) {
        if session.was_predicted(f as u32) == Some(true) {
            let distance = session
                .confirm_frame(f as u32, *item)
                .expect("drain confirm inside window");
            if distance > 0 {
                mispredicts += 1;
                first_bad.get_or_insert(f as u32);
            }
        }
    }
    let stream = session.stream();
    let fold = fold_stream(&stream);
    // Tape tip must equal the true-input replay over delayed locals.
    let mut truth = ToyDuel::default();
    for (l, r) in delayed.iter().zip(remote.iter()) {
        truth.advance([*l, *r]);
    }
    assert_eq!(session.sim().state_hash(), truth.state_hash());
    (
        fold,
        session.sim().state_hash(),
        session.rollback_count(),
        mispredicts,
        first_bad,
    )
}

#[test]
fn rollback_soak_converges() {
    let first = run_soak(SEED);
    let second = run_soak(SEED);
    assert_eq!(first.0, second.0, "same seed must hash twice");
    assert!(first.2 > 0, "seeded loss must force resim");
    let first_bad = match first.4 {
        Some(f) => i64::from(f),
        None => -1,
    };
    println!(
        "passed rollback soak {:#016x} tip {:#016x} rollbacks {} mispredicts {} first_incorrect {}",
        first.0, first.1, first.2, first.3, first_bad
    );
}
