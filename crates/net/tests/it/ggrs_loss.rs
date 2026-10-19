//! ggrs loss tape over the netcode session: every 5th remote lost then late-confirmed.
//!
//! Owner: `engine-net`; consumer: 2P `Session` predict plus resim path.
//! Units: frames (`u32`), compact inputs (`u32`), hashes (`u64`).
//! Determinism: integer-only toy sim, fixed script, no wall clock.
//! Errors: late confirms inside the window must succeed; evicted ones refuse.

use engine_core::Fnv64;
use engine_net::netcode::{Session, Sim, HISTORY_CAP, MAX_ROLLBACK};

#[derive(Clone, Default, PartialEq, Eq)]
struct Toy {
    a: i32,
    b: i32,
}

impl Sim for Toy {
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

fn seq(n: usize, k: u32) -> Vec<u32> {
    (0..n).map(|f| (f as u32 * k + 1) % 4).collect()
}

fn run_once() -> (Vec<u64>, u64, u64, u64, usize, usize) {
    let n = 64;
    let (local, remote) = (seq(n, 1), seq(n, 3));
    let mut s = Session::new(Toy::default());
    for f in 0..n {
        let avail = if f % 5 == 4 { None } else { Some(remote[f]) };
        s.advance(local[f], avail);
        if f >= 2 && (f - 2) % 5 == 4 {
            s.confirm((f - 2) as u32, remote[f - 2])
                .expect("late input inside window");
        }
    }
    for (f, &r) in remote.iter().enumerate() {
        if s.was_predicted(f as u32) == Some(true) {
            match s.confirm(f as u32, r) {
                Ok(_) => {}
                Err(engine_net::netcode::NetcodeError::TooFar { .. }) => {}
                Err(e) => panic!("unexpected confirm err: {e}"),
            }
        }
    }
    assert!(
        s.history_len() <= HISTORY_CAP + s.pending_len(),
        "history {} > CAP+pending {}",
        s.history_len(),
        HISTORY_CAP + s.pending_len()
    );
    assert!(s.max_rollback() <= MAX_ROLLBACK);
    assert!(s.remote_queue().len() <= 128);
    let tip_frame = s.frame();
    let shed = s.remote_queue_mut().discard_confirmed_frames(tip_frame);
    assert!(s.remote_queue().len() <= 128);
    match s.remote_queue().first_frame() {
        None => {}
        Some(f) => assert!(
            !s.remote_queue().is_confirmed(f),
            "tail must be first unconfirmed"
        ),
    }
    let _ = shed;
    let stream = s.stream();
    let tip = *stream.last().expect("tip");
    let mut fold_state = Fnv64::new();
    for h in &stream {
        fold_state.write_u64(*h);
    }
    let fold = fold_state.finish();
    let mut digest_state = Fnv64::new();
    digest_state.write_u64(fold);
    digest_state.write_u64(tip);
    digest_state.write_u64(s.rollback_count());
    let digest = digest_state.finish();
    let mut refs = Toy::default();
    for f in 0..n {
        refs.advance([local[f], remote[f]]);
    }
    assert_eq!(tip, refs.state_hash(), "tip equals true sim");
    (stream, fold, tip, digest, s.max_rollback(), s.history_len())
}

#[test]
fn ggrs_loss_64_every_5th_converges_bounded() {
    let (s1, fold1, tip1, dig1, rb1, len1) = run_once();
    let (s2, fold2, tip2, dig2, rb2, len2) = run_once();
    assert_eq!(s1, s2, "tape must replay identically x2");
    assert_eq!(
        (fold1, tip1, dig1),
        (fold2, tip2, dig2),
        "fold/tip/digest x2"
    );
    assert_eq!((rb1, len1), (rb2, len2));
}
