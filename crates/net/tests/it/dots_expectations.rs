//! Buyer-expectation pins over the landed columns and session.
//!
//! Owner: net lane. Units: bytes, ticks, hashes. Determinism: integer ops
//! in stable order, no wall clock, no model calls; replay reads the record.
//! Errors: test asserts only, no new error type.

use engine_arena::{run, ArenaConfig};
use engine_core::Fnv64;
use engine_input::FighterTape;
use engine_net::netcode::{Session, Sim, HISTORY_CAP, MAX_ROLLBACK};
use engine_net::resim::ResimWindow;
use engine_net::scale::{full_wire_len, within_cap, MAX_DATAGRAM_BYTES};
use engine_net::session::{encode_snapshot, MAX_PACKET_BYTES};
use engine_net::snapshot::{sample_snapshot, save_snapshot};

/// Integer-only toy duel for the session: two wrapping counters, FNV hash.
#[derive(Clone, Default, PartialEq, Eq)]
struct ToyDuel {
    a: i32,
    b: i32,
}

impl Sim for ToyDuel {
    fn step(&mut self, inputs: [u32; 2]) {
        let l = i32::try_from(inputs[0]).expect("script fits i32");
        let r = i32::try_from(inputs[1]).expect("script fits i32");
        self.a = self.a.wrapping_add(l.wrapping_sub(1));
        self.b = self.b.wrapping_add(r.wrapping_sub(1));
    }

    fn hash(&self) -> u64 {
        let mut fold = Fnv64::new();
        fold.write_i32(self.a);
        fold.write_i32(self.b);
        fold.finish()
    }
}

/// Duel fixture shape: seed 2040, 600 ticks, 2 fighters.
fn duel_config() -> ArenaConfig {
    ArenaConfig {
        seed: 2040,
        ticks: 600,
        fighters: 2,
        half_extent_m: 20.0,
        table_version: 1,
    }
}

fn script_pair(frame: u32) -> (u32, u32) {
    let local = frame.wrapping_mul(7).wrapping_add(1) % 5;
    let remote = frame.wrapping_mul(11).wrapping_add(2) % 5;
    (local, remote)
}

#[test]
fn tick_window_stays_dense_and_inside_cap_over_600() {
    const N: u32 = 600;
    let mut session = Session::new(ToyDuel::default());
    let mut window = ResimWindow::with_rtt(9);
    let mut remote_tape = Vec::with_capacity(600);
    for frame in 0..N {
        let (local, remote) = script_pair(frame);
        remote_tape.push(remote);
        let avail = if frame % 10 == 9 { None } else { Some(remote) };
        session.advance(local, avail);
        if frame >= 3 && (frame - 3) % 10 == 9 {
            let got = session
                .confirm(
                    frame - 3,
                    remote_tape[usize::try_from(frame - 3).expect("tape fits")],
                )
                .expect("late input inside window");
            window.observe(u32::try_from(got).expect("depth fits u32"));
        }
    }
    for (index, remote) in remote_tape.iter().enumerate() {
        let frame = u32::try_from(index).expect("tape fits u32");
        if session.was_predicted(frame) == Some(true) {
            match session.confirm(frame, *remote) {
                Ok(got) => window.observe(u32::try_from(got).expect("depth fits u32")),
                Err(engine_net::netcode::NetcodeError::TooFar { .. }) => {}
                Err(other) => panic!("unexpected confirm: {other}"),
            }
        }
    }
    assert_eq!(session.frames_played(), N);
    assert_eq!(session.frame(), N);
    assert!(session.len() <= MAX_ROLLBACK + 1);
    assert!(session.history_len() <= HISTORY_CAP + session.pending_len());
    assert!(session.max_rollback() <= MAX_ROLLBACK);
    assert!(session.predicted_count() > 0);
    assert!(session.rollback_count() > 0);
    assert!(window.all_inside());
    assert_eq!(session.stream().len(), session.history_len());
}

#[test]
fn per_tick_snapshot_fits_1200_on_duel_fixture() {
    let full = sample_snapshot(12);
    assert_eq!(full.fighters.len(), 2);
    let bytes = save_snapshot(&full).expect("duel snapshot saves");
    assert!(bytes.len() <= MAX_PACKET_BYTES);
    assert!(bytes.len() <= MAX_DATAGRAM_BYTES);
    assert_eq!(bytes.len(), full_wire_len(2));
    assert_eq!(full_wire_len(2), 47);
    within_cap(2, bytes.len()).expect("duel snapshot fits the cap");
    let wire = encode_snapshot(&full).expect("duel snapshot encodes");
    assert_eq!(wire.len(), bytes.len());
    let back = save_snapshot(&full).expect("resave");
    assert_eq!(back, bytes);
}

#[test]
fn recorded_tape_replays_without_model_x2() {
    let config = duel_config();
    let tape = FighterTape(Vec::new());
    let first = run(&config, &tape);
    let second = run(&config, &tape);
    assert_eq!(first.len(), 600);
    assert_eq!(first, second);
    assert!(!first.is_empty());
    let tip = first.last().expect("600 ticks fold a tip");
    let tip2 = second.last().expect("600 ticks fold a tip");
    assert_eq!(tip.hash, tip2.hash);
    let mut fold = Fnv64::new();
    for checkpoint in &first {
        fold.write_u64(checkpoint.hash);
    }
    let digest = fold.finish();
    let mut fold2 = Fnv64::new();
    for checkpoint in &second {
        fold2.write_u64(checkpoint.hash);
    }
    assert_eq!(digest, fold2.finish());
    assert_ne!(digest, 0);
}
